//! One small HTTP or HTTPS GET of a device's own page, to read what it
//! calls itself: the page's `<title>`, else its `Server` header. Read-only
//! and without logging in. A management page's self-signed certificate is
//! fine here: nothing but the request is sent.

use super::tls;
use rustls::StreamOwned;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Most of a page read: a title sits near the top.
const READ_MAX: usize = 32 * 1024;
/// Longest title kept.
const TITLE_MAX: usize = 80;

/// What a GET of one page answered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub status: u16,
    pub title: Option<String>,
    pub server: Option<String>,
    pub location: Option<String>,
    /// The `Date` header, as sent.
    pub date: Option<String>,
}

impl Page {
    /// What the device calls itself: its title, else its server's name.
    pub fn label(&self) -> Option<String> {
        self.title.clone().or_else(|| self.server.clone())
    }
}

/// How long one GET may take in all, in `timeout`s: the handshake and the
/// read share it. A device that dribbles a byte at a time can't hold a
/// sweep's worker (and its Stop) past this.
const WHOLE: u32 = 3;

/// GET `path` from `addr`, asking for `host`, over TLS when `https`. All of
/// it, from the handshake to the last byte read, within `WHOLE` times
/// `timeout` of connecting.
pub fn get(
    addr: SocketAddr,
    host: &str,
    https: bool,
    path: &str,
    timeout: Duration,
) -> io::Result<Page> {
    fetch(addr, host, https, path, timeout).0
}

/// As `get`, with the certificate the device showed in that same
/// handshake (over TLS, once the handshake finished), even when what
/// followed wasn't a page.
pub fn fetch(
    addr: SocketAddr,
    host: &str,
    https: bool,
    path: &str,
    timeout: Duration,
) -> (io::Result<Page>, Option<tls::Cert>) {
    let mut cert = None;
    let page = fetch_into(addr, host, https, path, timeout, &mut cert);
    (page, cert)
}

fn fetch_into(
    addr: SocketAddr,
    host: &str,
    https: bool,
    path: &str,
    timeout: Duration,
    cert: &mut Option<tls::Cert>,
) -> io::Result<Page> {
    let tcp = TcpStream::connect_timeout(&addr, timeout)?;
    let mut tcp = super::Timed::new(tcp, timeout, timeout * WHOLE);
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: is-alive\r\nAccept: text/html\r\n\
         Connection: close\r\n\r\n"
    );
    let mut buf = Vec::new();
    if https {
        // The page's title is wanted, not a judgement of its certificate:
        // what it says is read, as the device's own claim, and nothing more.
        let mut conn = tls::client_unjudged(host)?;
        tls::handshake(&mut conn, &mut tcp)?;
        *cert = tls::leaf_of(&conn);
        let mut s = StreamOwned::new(conn, tcp);
        s.write_all(req.as_bytes())?;
        read_some(&mut s, &mut buf);
    } else {
        tcp.write_all(req.as_bytes())?;
        read_some(&mut tcp, &mut buf);
    }
    parse(&buf).ok_or_else(|| io::Error::other("not an HTTP answer"))
}

/// Read until the end, `READ_MAX`, the page's `</title>` (all that's
/// wanted from it), a timeout or an error, keeping what came.
fn read_some(r: &mut impl Read, buf: &mut Vec<u8>) {
    let mut chunk = [0u8; 4096];
    while buf.len() < READ_MAX {
        match r.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        // Looked for in what just came and the few bytes before it.
        let from = buf.len().saturating_sub(chunk.len() + 8);
        if buf[from..]
            .windows(8)
            .any(|w| w.eq_ignore_ascii_case(b"</title>"))
        {
            break;
        }
    }
}

/// Read an HTTP response's status, a few headers and the page's title.
pub fn parse(raw: &[u8]) -> Option<Page> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines
        .next()?
        .strip_prefix("HTTP/")?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    let mut page = Page {
        status,
        ..Page::default()
    };
    for l in lines {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        let v = Some(super::sanitize(v.trim())).filter(|v| !v.is_empty());
        match k.trim().to_ascii_lowercase().as_str() {
            "server" => page.server = v,
            "location" => page.location = v,
            "date" => page.date = v,
            _ => {}
        }
    }
    page.title = title(body).filter(|t| !placeholder(t));
    Some(page)
}

/// A title that says nothing about the device: what a page shows while
/// its script loads, a bare login or redirect page. The Server header
/// says more, so it's dropped.
fn placeholder(title: &str) -> bool {
    let t: String = title
        .trim()
        .trim_end_matches(['.', '…', '!', ' '])
        .to_ascii_lowercase();
    const SAYS_NOTHING: &[&str] = &[
        "",
        "loading",
        "please wait",
        "please wait while loading",
        "wait",
        "redirect",
        "redirecting",
        "redirection",
        "redirect page",
        "moved",
        "302 found",
        "301 moved permanently",
        "found",
        "login",
        "log in",
        "logon",
        "sign in",
        "signin",
        "user login",
        "login page",
        "web login",
        "authentication",
        "authorization required",
        "home",
        "index",
        "index.html",
        "welcome",
        "untitled",
        "untitled document",
        "document",
        "web",
        "web ui",
        "webui",
        "web management",
        "management",
        "main page",
        "default",
        "default page",
        "javascript required",
        "no title",
        "null",
        "undefined",
    ];
    SAYS_NOTHING.contains(&t.as_str()) || t.chars().all(|c| !c.is_alphanumeric())
}

/// A page's `<title>`, entities decoded, on one line, cut to `TITLE_MAX`.
pub fn title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    let raw = &html[start..end];
    let decoded = raw
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&nbsp;", " ");
    let one_line = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    let t: String = super::sanitize(&one_line).chars().take(TITLE_MAX).collect();
    Some(t).filter(|t| !t.is_empty())
}

/// Where a redirect from `addr` leads on the same device: (https, port,
/// path). None for another host, or nothing to follow.
pub fn same_device(location: &str, addr: SocketAddr, https: bool) -> Option<(bool, u16, String)> {
    if location.starts_with('/') {
        return Some((https, addr.port(), location.to_string()));
    }
    let (scheme, rest) = location.split_once("://")?;
    let tls = match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" => false,
        _ => return None,
    };
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (hostport, if tls { 443 } else { 80 }),
    };
    (host == addr.ip().to_string()).then(|| (tls, port, path.to_string()))
}

/// What a device's web page and its certificate say about it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Named {
    /// Its page's title, else its Server header.
    pub label: Option<String>,
    /// `label` is a real title, not a Server header standing in for a
    /// placeholder or missing one.
    pub titled: bool,
    /// The certificate it showed on its HTTPS management port.
    pub cert: Option<tls::Cert>,
}

/// The HTTPS management ports, in the order they're tried.
const HTTPS_PORTS: [u16; 3] = [443, 8443, 4433];

/// What a device's web page calls it: the first of its open web ports
/// that answers with a title (or a server name), following one redirect
/// on the device itself. HTTPS first. The certificate comes from the
/// first of those GETs that went over TLS; where none did but an HTTPS
/// port is open, one handshake reads it and closes.
pub fn identify(addr: std::net::IpAddr, open: &[u16], timeout: Duration) -> Named {
    let mut out = Named::default();
    let mut over_tls = false;
    let mut cert = None;
    let host = addr.to_string();
    let mut get = |at: SocketAddr, https: bool, path: &str| {
        over_tls |= https;
        let (page, c) = fetch(at, &host, https, path, timeout);
        if cert.is_none() {
            cert = c;
        }
        page
    };
    if let Some(page) = first_page(addr, open, &mut get) {
        out.titled = page.title.is_some();
        out.label = page.label();
    }
    out.cert = cert;
    if out.cert.is_none()
        && !over_tls
        && let Some(&p) = HTTPS_PORTS.iter().find(|p| open.contains(p))
    {
        out.cert = tls::leaf(SocketAddr::new(addr, p), timeout).ok().flatten();
    }
    out
}

/// The page that names the device, by `get` (address, https, path):
/// two ports at most, and one redirect on the device itself.
fn first_page(
    addr: std::net::IpAddr,
    open: &[u16],
    get: &mut impl FnMut(SocketAddr, bool, &str) -> io::Result<Page>,
) -> Option<Page> {
    const ORDER: [(u16, bool); 6] = [
        (443, true),
        (8443, true),
        (4433, true),
        (80, false),
        (8080, false),
        (5000, false),
    ];
    // Two ports at most: a device's page is on its first or second.
    for (port, https) in ORDER.into_iter().filter(|(p, _)| open.contains(p)).take(2) {
        let at = SocketAddr::new(addr, port);
        let Ok(page) = get(at, https, "/") else {
            continue;
        };
        if page.title.is_none()
            && (300..400).contains(&page.status)
            && let Some((tls, p, path)) = page
                .location
                .as_deref()
                .and_then(|l| same_device(l, at, https))
            && let Ok(next) = get(SocketAddr::new(addr, p), tls, &path)
            && next.label().is_some()
        {
            return Some(next);
        }
        if page.label().is_some() {
            return Some(page);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server on this machine: what `talk` does with the connection.
    fn serve(talk: impl FnOnce(TcpStream) + Send + 'static) -> SocketAddr {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((s, _)) = l.accept() {
                talk(s);
            }
        });
        addr
    }

    /// A device that sends a byte every so often, never quite timing a
    /// read out, is given up on at the whole GET's deadline.
    #[test]
    fn a_page_that_dribbles_is_cut_off() {
        let addr = serve(|mut s| {
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nServer: slow\r\n\r\n");
            for _ in 0..200 {
                std::thread::sleep(Duration::from_millis(100));
                if s.write_all(b"x").is_err() {
                    break;
                }
            }
        });
        let t = std::time::Instant::now();
        let page = get(addr, "x", false, "/", Duration::from_millis(300)).unwrap();
        assert_eq!(page.server.as_deref(), Some("slow"));
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "took {:?}",
            t.elapsed()
        );
    }

    /// A placeholder title ("Loading...", "Login") says nothing about the
    /// device: the Server header names it instead. A real title stays.
    #[test]
    fn a_placeholder_title_falls_back_to_the_server() {
        let page = |title: &str| {
            parse(
                format!(
                    "HTTP/1.1 200 OK\r\nServer: Acme-Edge/2.1\r\n\r\n<html><title>{title}</title>"
                )
                .as_bytes(),
            )
            .unwrap()
        };
        for t in [
            "Loading...",
            "Login",
            "Redirecting…",
            "Please wait",
            "  ",
            "...",
            "Untitled Document",
        ] {
            let p = page(t);
            assert_eq!(p.title, None, "{t:?}");
            assert_eq!(p.label().as_deref(), Some("Acme-Edge/2.1"), "{t:?}");
        }
        assert_eq!(
            page("SonicWall - Login").label().as_deref(),
            Some("SonicWall - Login")
        );
        assert_eq!(
            page("Poly VVX 450").label().as_deref(),
            Some("Poly VVX 450")
        );
    }

    /// The title is all that's wanted: once it's in, the read stops rather
    /// than wait for a server that keeps the connection open.
    #[test]
    fn the_read_stops_at_the_title() {
        let addr = serve(|mut s| {
            let _ = s.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><head><TITLE>UniFi OS</TITLE>",
            );
            std::thread::sleep(Duration::from_secs(10));
        });
        let t = std::time::Instant::now();
        let page = get(addr, "x", false, "/", Duration::from_secs(5)).unwrap();
        assert_eq!(page.title.as_deref(), Some("UniFi OS"));
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "took {:?}",
            t.elapsed()
        );
    }

    /// An HTTPS device on loopback with its own self-signed certificate
    /// (fictional names), answering every connection with `reply`. Counts
    /// the connections it took.
    fn serve_tls(
        reply: &'static str,
    ) -> (SocketAddr, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use rcgen::{CertificateParams, DnType, KeyPair};
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        use std::sync::{Arc, atomic::AtomicUsize, atomic::Ordering};
        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::new(vec!["nas.test.example".to_string()]).unwrap();
        p.distinguished_name = rcgen::DistinguishedName::new();
        p.distinguished_name.push(DnType::CommonName, "DS-0000");
        p.distinguished_name
            .push(DnType::OrganizationName, "Synology Inc.");
        let cert = p.self_signed(&key).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert.der().to_vec())],
            PrivateKeyDer::try_from(key.serialize_der()).unwrap(),
        )
        .unwrap();
        let config = Arc::new(config);
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let n = count.clone();
        std::thread::spawn(move || {
            for tcp in l.incoming().flatten() {
                n.fetch_add(1, Ordering::SeqCst);
                let conn = rustls::ServerConnection::new(config.clone()).unwrap();
                let mut s = StreamOwned::new(conn, tcp);
                let mut b = [0u8; 1024];
                if s.read(&mut b).is_ok() {
                    let _ = s.write_all(reply.as_bytes());
                    s.conn.send_close_notify();
                    let _ = s.flush();
                }
            }
        });
        (addr, count)
    }

    /// The certificate comes from the GET's own handshake: one connection,
    /// and a placeholder title leaves the Server header to name it.
    #[test]
    fn an_https_get_brings_its_certificate() {
        use std::sync::atomic::Ordering;
        let (addr, count) =
            serve_tls("HTTP/1.1 200 OK\r\nServer: nginx\r\n\r\n<html><title>Loading...</title>");
        let (page, cert) = fetch(addr, "127.0.0.1", true, "/", Duration::from_secs(5));
        let page = page.unwrap();
        assert_eq!(page.title, None);
        assert_eq!(page.label().as_deref(), Some("nginx"));
        let c = cert.expect("the handshake's certificate");
        assert_eq!(c.dn().as_deref(), Some("CN=DS-0000, O=Synology Inc."));
        assert_eq!(c.names, ["nas.test.example"]);
        assert!(c.self_signed);
        assert_eq!(count.load(Ordering::SeqCst), 1, "no second connection");
        // Plain HTTP: no certificate.
        let plain = serve(|mut s| {
            let mut b = [0u8; 512];
            let _ = s.read(&mut b);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n<title>x</title>");
        });
        assert_eq!(
            fetch(plain, "x", false, "/", Duration::from_secs(2)).1,
            None
        );
    }

    /// What follows the handshake isn't a page: the certificate is still
    /// kept.
    #[test]
    fn a_certificate_is_kept_when_the_page_isnt_one() {
        let (addr, _) = serve_tls("SSH-2.0-nope\r\n");
        let (page, cert) = fetch(addr, "127.0.0.1", true, "/", Duration::from_secs(5));
        assert!(page.is_err());
        assert_eq!(cert.unwrap().cn.as_deref(), Some("DS-0000"));
    }

    /// `first_page` tries two ports at most and follows one redirect on
    /// the device; `identify` keeps the first certificate a GET brought.
    #[test]
    fn the_page_that_names_a_device_is_found_in_order() {
        let ip: std::net::IpAddr = "192.0.2.5".parse().unwrap();
        let mut asked = Vec::new();
        let mut get = |at: SocketAddr, https: bool, path: &str| {
            asked.push((at.port(), https, path.to_string()));
            match (at.port(), path) {
                (443, "/") => parse(b"HTTP/1.1 302 Found\r\nLocation: /ui\r\n\r\n")
                    .ok_or_else(|| io::Error::other("x")),
                (443, "/ui") => parse(b"HTTP/1.1 200 OK\r\nServer: Acme\r\n\r\n")
                    .ok_or_else(|| io::Error::other("x")),
                _ => Err(io::Error::other("closed")),
            }
        };
        let page = first_page(ip, &[80, 443, 8443, 8080], &mut get).unwrap();
        assert_eq!(page.label().as_deref(), Some("Acme"));
        assert_eq!(
            asked,
            [(443, true, "/".to_string()), (443, true, "/ui".to_string())]
        );
    }

    #[test]
    fn hostile_pages_never_panic() {
        let raw = b"HTTP/1.1 301 Moved\r\nServer: x\r\nLocation: https://192.0.2.1:8443/a?b#c\r\n\
                    Content-Type: text/html\r\n\r\n<html><head><TITLE> A &amp; B &#x41;&#65; </TITLE>";
        crate::hostile::each(raw, |r| {
            if let Some(p) = parse(r) {
                let _ = p.label();
            }
        });
    }

    #[test]
    fn a_pages_title_and_headers_are_read() {
        let raw = b"HTTP/1.1 200 OK\r\nServer: lighttpd/1.4\r\nDate: Wed, 11 Nov 2026 06:00:00 GMT\r\n\
                    Content-Type: text/html\r\n\r\n<html><head>\n<TITLE>\n  UniFi &amp; Network\n</TITLE>";
        let p = parse(raw).unwrap();
        assert_eq!(p.status, 200);
        assert_eq!(p.title.as_deref(), Some("UniFi & Network"));
        assert_eq!(p.server.as_deref(), Some("lighttpd/1.4"));
        assert_eq!(p.date.as_deref(), Some("Wed, 11 Nov 2026 06:00:00 GMT"));
        assert_eq!(p.label().as_deref(), Some("UniFi & Network"));
        // No title: the server's name; an empty one isn't a title.
        let p =
            parse(b"HTTP/1.0 200 OK\r\nServer: HP HTTP Server\r\n\r\n<title> </title>").unwrap();
        assert_eq!(p.label().as_deref(), Some("HP HTTP Server"));
        assert!(parse(b"SSH-2.0-OpenSSH_9.6\r\n").is_none());
        // Cut to a line of TITLE_MAX characters, controls taken out.
        let long = format!("<title>{}\u{7}</title>", "x".repeat(200));
        assert_eq!(title(&long).unwrap().chars().count(), TITLE_MAX);
    }

    #[test]
    fn redirects_are_followed_only_on_the_same_device() {
        let at: SocketAddr = "192.0.2.10:80".parse().unwrap();
        assert_eq!(
            same_device("/login.html", at, false),
            Some((false, 80, "/login.html".into()))
        );
        assert_eq!(
            same_device("https://192.0.2.10:8443/manage", at, false),
            Some((true, 8443, "/manage".into()))
        );
        assert_eq!(
            same_device("https://192.0.2.10", at, false),
            Some((true, 443, "/".into()))
        );
        assert_eq!(same_device("https://cloud.example.com/", at, false), None);
        assert_eq!(same_device("ftp://192.0.2.10/", at, false), None);
    }

    /// A real server on this machine: a page with a title, and one that
    /// sends you to another page on the same device.
    #[test]
    fn a_device_is_named_by_its_page() {
        use std::net::TcpListener;
        let serve = |answer: &'static str| {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = l.local_addr().unwrap().port();
            std::thread::spawn(move || {
                for s in l.incoming().take(2).flatten() {
                    let mut s = s;
                    let mut b = [0u8; 1024];
                    let n = s.read(&mut b).unwrap_or(0);
                    let req = String::from_utf8_lossy(&b[..n]);
                    let reply = if req.starts_with("GET /login") {
                        "HTTP/1.1 200 OK\r\n\r\n<title>SonicWall - Authentication</title>"
                    } else {
                        answer
                    };
                    let _ = s.write_all(reply.as_bytes());
                }
            });
            port
        };
        let t = Duration::from_secs(2);
        let ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        let page = serve("HTTP/1.1 200 OK\r\n\r\n<title>Synology DiskStation</title>");
        let got = get(SocketAddr::new(ip, page), "127.0.0.1", false, "/", t).unwrap();
        assert_eq!(got.title.as_deref(), Some("Synology DiskStation"));
        let moved = serve("HTTP/1.1 302 Found\r\nLocation: /login\r\n\r\n");
        let at = SocketAddr::new(ip, moved);
        let first = get(at, "127.0.0.1", false, "/", t).unwrap();
        let (tls, port, path) = same_device(first.location.as_deref().unwrap(), at, false).unwrap();
        let next = get(SocketAddr::new(ip, port), "127.0.0.1", tls, &path, t).unwrap();
        assert_eq!(next.label().as_deref(), Some("SonicWall - Authentication"));
    }
}
