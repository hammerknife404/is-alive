//! SMTP on each MX: banner, STARTTLS, the certificate it presents, and
//! whether it relays for strangers. No mail is sent: the session stops at
//! RCPT and quits.

use super::super::sanitize;
use super::super::tls;
use super::dnsq::Client;
use super::{Check, MxInfo, addrs};
use rustls::StreamOwned;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

const CONNECT: Duration = Duration::from_secs(6);
/// Some servers delay the greeting on purpose (tarpitting); allow for it.
const READ: Duration = Duration::from_secs(15);
const MAX_HOSTS: usize = 3;

/// SMTP replies over any stream, plain or TLS.
struct Session<S> {
    s: S,
    buf: Vec<u8>,
}

impl<S: Read + Write> Session<S> {
    fn line(&mut self) -> io::Result<String> {
        loop {
            if let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
                let l: Vec<u8> = self.buf.drain(..=i).collect();
                return Ok(sanitize(String::from_utf8_lossy(&l).trim_end()));
            }
            if self.buf.len() > 64 * 1024 {
                return Err(io::Error::other("reply line too long"));
            }
            let mut chunk = [0u8; 4096];
            let n = self.s.read(&mut chunk)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed",
                ));
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// A full (possibly multi-line "250-...") reply: code and text lines.
    fn reply(&mut self) -> io::Result<(u16, Vec<String>)> {
        let mut lines = Vec::new();
        loop {
            let l = self.line()?;
            let code = l.get(..3).and_then(|c| c.parse().ok());
            let more = l.as_bytes().get(3) == Some(&b'-');
            lines.push(l.get(4..).unwrap_or("").to_string());
            match code {
                Some(c) if !more => return Ok((c, lines)),
                Some(_) => {}
                None => return Err(io::Error::other(format!("not an SMTP reply: {l}"))),
            }
        }
    }

    fn cmd(&mut self, c: &str) -> io::Result<(u16, Vec<String>)> {
        self.s.write_all(format!("{c}\r\n").as_bytes())?;
        self.s.flush()?;
        self.reply()
    }
}

/// What one MX host did.
#[derive(Debug, Default)]
struct Probe {
    banner: Option<String>,
    starttls: bool,
    tls: Option<String>,
    /// Certificate verification error, if any.
    cert_error: Option<String>,
    /// Leaf certificate: subject CN and notAfter.
    cert: Option<(String, Option<chrono::DateTime<chrono::Utc>>)>,
    /// None: couldn't test (MAIL FROM refused).
    open_relay: Option<bool>,
    /// The relay test broke off (connection dropped or stalled).
    relay_error: Option<String>,
}

fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
    let s = TcpStream::connect_timeout(&addr, CONNECT)?;
    s.set_read_timeout(Some(READ))?;
    s.set_write_timeout(Some(READ))?;
    Ok(s)
}

/// EHLO, MAIL FROM and RCPT TO a third-party domain, then QUIT. A server
/// that accepts the RCPT from an unauthenticated stranger is an open relay.
fn relay_test<S: Read + Write>(s: &mut Session<S>, helo: &str) -> io::Result<Option<bool>> {
    s.cmd(&format!("EHLO {helo}"))?;
    let (mail, _) = s.cmd("MAIL FROM:<is-alive-relay-test@is-alive.invalid>")?;
    if mail != 250 {
        let _ = s.cmd("QUIT");
        return Ok(None);
    }
    let (rcpt, _) = s.cmd("RCPT TO:<is-alive-relay-test@gmail.com>")?;
    let _ = s.cmd("QUIT");
    Ok(Some(matches!(rcpt, 250 | 251)))
}

/// Connect, then talk SMTP. Only a connect timeout keeps the TimedOut kind:
/// a server that answered the connection but then stalled is not a blocked
/// port 25.
fn probe(host: &str, addr: SocketAddr) -> io::Result<Probe> {
    let tcp = connect(addr)?;
    talk(host, tcp).map_err(|e| {
        if matches!(
            e.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ) {
            io::Error::other(format!(
                "connected, but no reply within {}s",
                READ.as_secs()
            ))
        } else {
            e
        }
    })
}

fn talk(host: &str, tcp: TcpStream) -> io::Result<Probe> {
    // An address literal is always a valid EHLO name.
    let helo = match tcp.local_addr()?.ip() {
        IpAddr::V4(ip) => format!("[{ip}]"),
        IpAddr::V6(ip) => format!("[IPv6:{ip}]"),
    };
    let mut s = Session {
        s: tcp,
        buf: Vec::new(),
    };
    let mut p = Probe::default();
    let (code, banner) = s.reply()?;
    p.banner = Some(format!("{code} {}", banner.join(" / ")));
    if code != 220 {
        return Ok(p);
    }
    let (code, caps) = s.cmd(&format!("EHLO {helo}"))?;
    p.starttls = code == 250 && caps.iter().any(|c| c.eq_ignore_ascii_case("STARTTLS"));
    if !p.starttls {
        p.relay(relay_test(&mut s, &helo));
        return Ok(p);
    }
    let (code, _) = s.cmd("STARTTLS")?;
    if code != 220 || !s.buf.is_empty() {
        return Err(io::Error::other(format!("STARTTLS refused ({code})")));
    }

    let (mut conn, verifier) = tls::client(host)?;
    let mut tcp = s.s;
    tls::handshake(&mut conn, &mut tcp)?;
    let (version, cipher) = tls::negotiated(&conn);
    p.tls = Some(format!("{version}, {cipher}"));
    p.cert_error = verifier.verdict().map(|e| e.to_string());
    p.cert = conn
        .peer_certificates()
        .and_then(|c| c.first())
        .map(|leaf| match tls::parse(leaf.as_ref()) {
            Some(c) => (c.subject, c.not_after),
            None => ("(unparsed)".into(), None),
        });
    let mut s = Session {
        s: StreamOwned::new(conn, tcp),
        buf: Vec::new(),
    };
    p.relay(relay_test(&mut s, &helo));
    Ok(p)
}

impl Probe {
    /// Keep what was learned so far when the relay test breaks off.
    fn relay(&mut self, r: io::Result<Option<bool>>) {
        match r {
            Ok(v) => self.open_relay = v,
            Err(e) => self.relay_error = Some(e.to_string()),
        }
    }
}

pub(super) fn smtp(c: &Client, mx: &MxInfo) -> Check {
    Check::run(|k| {
        if mx.hosts.is_empty() {
            k.info("no MX hosts to connect to");
            return Ok(());
        }
        let hosts: Vec<&String> = mx.hosts.iter().take(MAX_HOSTS).collect();
        let results: Vec<(&String, Result<Probe, String>)> = std::thread::scope(|s| {
            let hs: Vec<_> = hosts
                .iter()
                .map(|&h| {
                    s.spawn(move || {
                        let r = (|| {
                            let ips = addrs(c, h).map_err(|e| e.to_string())?.0;
                            let ip = ips
                                .iter()
                                .find(|i| i.is_ipv4())
                                .or(ips.first())
                                .ok_or("does not resolve")?;
                            probe(h, SocketAddr::new(*ip, 25)).map_err(|e| {
                                if e.kind() == io::ErrorKind::TimedOut
                                    || e.kind() == io::ErrorKind::WouldBlock
                                {
                                    "timed out".to_string()
                                } else {
                                    e.to_string()
                                }
                            })
                        })();
                        (h, r)
                    })
                })
                .collect();
            hs.into_iter().filter_map(|h| h.join().ok()).collect()
        });

        // Every host timing out is almost always outbound port 25 being
        // blocked here, not every MX being down.
        if results
            .iter()
            .all(|(_, r)| matches!(r, Err(e) if e == "timed out"))
        {
            k.info("port 25 timed out on every MX: this network (or ISP) probably blocks outbound SMTP. Run from a network that allows it");
            return Ok(());
        }
        let mut good = 0;
        let mut relay_untested = 0;
        for (h, r) in &results {
            let p = match r {
                Ok(p) => p,
                Err(e) => {
                    k.rec(format!("{h}  ✖ {e}"));
                    k.fail(format!("{h}: {e}"));
                    continue;
                }
            };
            if let Some(b) = &p.banner {
                k.rec(format!("{h}  {b}"));
            }
            if !p.banner.as_deref().is_some_and(|b| b.starts_with("220")) {
                k.fail(format!("{h}: no 220 greeting, so senders can't deliver"));
                continue;
            }
            let mut ok = true;
            if !p.starttls {
                ok = false;
                k.warn(format!(
                    "{h}: no STARTTLS, so mail to you travels unencrypted"
                ));
            }
            if let Some(t) = &p.tls {
                k.rec(format!("{h}  TLS {t}"));
            }
            if let Some((cn, exp)) = &p.cert {
                let days = exp.map(|e| (e - chrono::Utc::now()).num_days());
                k.rec(format!(
                    "{h}  certificate {cn}, expires {}",
                    exp.map_or("?".into(), |e| e.format("%Y-%m-%d").to_string())
                ));
                if let Some(d) = days.filter(|&d| (0..tls::SOON_DAYS).contains(&d)) {
                    ok = false;
                    k.warn(format!("{h}: certificate expires in {d} day(s)"));
                }
            }
            if let Some(e) = &p.cert_error {
                ok = false;
                k.warn(format!(
                    "{h}: certificate doesn't validate ({e}); most senders still deliver, but MTA-STS senders refuse"
                ));
            }
            match p.open_relay {
                _ if p.relay_error.is_some() => k.info(format!(
                    "{h}: the relay test broke off ({}), so relaying couldn't be tested",
                    p.relay_error.as_deref().unwrap_or_default()
                )),
                Some(true) => {
                    ok = false;
                    k.fail(format!(
                        "{h} is an OPEN RELAY: it accepted mail for gmail.com from an unauthenticated stranger"
                    ));
                }
                Some(false) => {}
                None => k.info(format!(
                    "{h}: refused the test sender, so relaying couldn't be tested"
                )),
            }
            if ok {
                good += 1;
                if p.open_relay != Some(false) {
                    relay_untested += 1;
                }
            }
        }
        if good > 0 {
            // Only claim "not an open relay" when every passing host was tested.
            let relay = if relay_untested == 0 {
                ", not an open relay"
            } else {
                ""
            };
            k.pass(format!(
                "{good}/{} MX host(s): greeting, STARTTLS with a valid certificate{relay}",
                results.len()
            ));
        }
        if mx.hosts.len() > MAX_HOSTS {
            k.info(format!(
                "checked the first {MAX_HOSTS} of {} MX hosts",
                mx.hosts.len()
            ));
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A scripted SMTP server: answers each command with the next reply.
    fn server(replies: &'static [&'static str]) -> SocketAddr {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut r = std::io::BufReader::new(s.try_clone().unwrap());
            s.write_all(replies[0].as_bytes()).unwrap();
            for reply in &replies[1..] {
                let mut line = String::new();
                if std::io::BufRead::read_line(&mut r, &mut line).unwrap_or(0) == 0 {
                    return;
                }
                s.write_all(reply.as_bytes()).unwrap();
            }
        });
        addr
    }

    #[test]
    fn plain_server_without_starttls() {
        let addr = server(&[
            "220 mx.test ESMTP\r\n",
            "250-mx.test\r\n250 SIZE 1000\r\n",
            "250-mx.test\r\n250 SIZE 1000\r\n",
            "250 ok\r\n",
            "550 relay denied\r\n",
            "221 bye\r\n",
        ]);
        let p = probe("mx.test", addr).unwrap();
        assert_eq!(p.banner.as_deref(), Some("220 mx.test ESMTP"));
        assert!(!p.starttls);
        assert_eq!(p.open_relay, Some(false));
    }

    #[test]
    fn detects_open_relay() {
        let addr = server(&[
            "220 mx.test\r\n",
            "250 mx.test\r\n",
            "250 mx.test\r\n",
            "250 ok\r\n",
            "250 accepted\r\n",
            "221 bye\r\n",
        ]);
        assert_eq!(probe("mx.test", addr).unwrap().open_relay, Some(true));
    }
}
