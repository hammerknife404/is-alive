pub mod checks;
pub mod clock;
pub mod dnsq;
pub mod dnssd;
pub mod domain;
pub mod hostinfo;
pub mod icmp;
pub mod join;
pub mod joined;
pub mod mac;
pub mod names;
pub mod ports;
pub mod speed;
pub mod subnet;
pub mod targets;
pub mod tls;
pub mod web;
pub mod wol;

use socket2::{Domain, Protocol, Socket, Type};
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// A system tool by absolute path from the usual system folders, never a
/// PATH lookup: a folder earlier in PATH can't slip in a look-alike. None if
/// it isn't installed. Unix only; Windows uses its APIs instead.
#[cfg(not(windows))]
pub fn system_tool(name: &str) -> Option<std::path::PathBuf> {
    ["/usr/sbin", "/sbin", "/usr/bin", "/bin"]
        .iter()
        .map(|d| std::path::Path::new(d).join(name))
        .find(|p| p.is_file())
}

/// Run `cmd` for its output, for at most `limit`: past it the program is
/// stopped and what it printed so far comes back (with a TimedOut error if
/// it printed nothing). Both pipes are read as it runs, so a program that
/// prints a lot can't fill one and stall.
pub fn output_within(
    cmd: &mut std::process::Command,
    limit: std::time::Duration,
) -> io::Result<std::process::Output> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let drain = |p: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = p {
                // At most 4 MB: no status tool prints more.
                let _ = p.by_ref().take(4 << 20).read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break Some(s);
        }
        if start.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    match status {
        Some(status) => Ok(std::process::Output {
            status,
            stdout,
            stderr,
        }),
        None => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("still running after {} s: stopped", limit.as_secs()),
        )),
    }
}

/// Resolve a host name or IP literal. Prefers IPv4 unless `prefer_v6`.
pub fn resolve(host: &str, prefer_v6: bool) -> io::Result<IpAddr> {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    // A zone (fe80::1%eth0) names the interface, and the tests send to a
    // bare address: the system would refuse with "Invalid argument". Said
    // plainly instead, as is a multicast address, which no test answers.
    if let Some((addr, zone)) = host.split_once('%')
        && addr.parse::<std::net::Ipv6Addr>().is_ok()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{addr}%{zone} is a link-local address with a zone, which this can't send to: use the device's IPv4 address or a global IPv6 one"
            ),
        ));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip.is_multicast() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{ip} is a multicast address: test one device's address"),
            ));
        }
        return Ok(ip);
    }
    let addrs: Vec<IpAddr> = (host, 0).to_socket_addrs()?.map(|a| a.ip()).collect();
    addrs
        .iter()
        .find(|a| a.is_ipv6() == prefer_v6)
        .or(addrs.first())
        .copied()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no addresses found"))
}

pub fn lookup_all(host: &str) -> io::Result<Vec<IpAddr>> {
    let mut v: Vec<IpAddr> = dns_lookup::lookup_host(host.trim())?.collect();
    v.sort_by_key(|a| (a.is_ipv6(), *a));
    v.dedup();
    Ok(v)
}

/// PTR lookup. The answer comes from whoever controls the reverse zone,
/// so control and bidi-override characters are stripped before display.
pub fn reverse(ip: IpAddr) -> Option<String> {
    dns_lookup::lookup_addr(&ip)
        .ok()
        .map(|n| sanitize(&n))
        .filter(|n| !n.is_empty() && n != &ip.to_string())
}

pub(crate) fn sanitize(s: &str) -> String {
    clean(s, 255)
}

/// As `sanitize`, for a page of text (a WHOIS record): its line breaks and
/// tabs kept, every other control and bidi character dropped.
pub(crate) fn sanitize_text(s: &str) -> String {
    s.chars()
        .filter(|&c| {
            c == '\n'
                || c == '\t'
                || !(c.is_control()
                    || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        })
        .collect()
}

/// As `sanitize`, for long TXT data (SPF, DKIM keys).
fn sanitize_long(s: &str) -> String {
    clean(s, 4096)
}

fn clean(s: &str, max: usize) -> String {
    s.chars()
        .filter(|&c| {
            !c.is_control()
                && !matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        })
        .take(max)
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub enum TcpState {
    Open,
    Closed,
    Filtered,
    Error(String),
}

/// TCP connect probe. Returns state and the connect time.
pub fn tcp_probe(addr: SocketAddr, timeout: Duration) -> (TcpState, f64) {
    let start = Instant::now();
    let r = connect(addr, timeout);
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    let state = match r {
        Ok(_) => TcpState::Open,
        Err(e) => match e.kind() {
            io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset => TcpState::Closed,
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => TcpState::Filtered,
            _ => TcpState::Error(e.to_string()),
        },
    };
    (state, ms)
}

/// A port check: `tcp_probe`, but on Windows a filtered answer gets one
/// more try. Windows sends no SYN retransmit here (see `no_syn_retransmit`),
/// so one lost packet would otherwise read an open port as filtered. Other
/// systems already resend within the timeout. Not for TCP ping, whose
/// timings are single attempts.
pub fn tcp_check(addr: SocketAddr, timeout: Duration) -> (TcpState, f64) {
    let first = tcp_probe(addr, timeout);
    if cfg!(windows) && first.0 == TcpState::Filtered {
        return tcp_probe(addr, timeout);
    }
    first
}

fn connect(addr: SocketAddr, timeout: Duration) -> io::Result<()> {
    let sock = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    #[cfg(windows)]
    no_syn_retransmit(&sock);
    sock.connect_timeout(&addr.into(), timeout)
}

/// Windows resends the SYN after an RST (~2 s by default), so a refused
/// port looks like a timeout. Disable SYN retransmission so RST fails fast.
/// Unsupported before Windows 10 RS3; failure here just keeps the default.
#[cfg(windows)]
fn no_syn_retransmit(sock: &Socket) {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{
        SIO_TCP_INITIAL_RTO, TCP_INITIAL_RTO_NO_SYN_RETRANSMISSIONS, TCP_INITIAL_RTO_PARAMETERS,
        WSAIoctl,
    };
    let params = TCP_INITIAL_RTO_PARAMETERS {
        Rtt: u16::MAX, // TCP_INITIAL_RTO_UNSPECIFIED_RTT: keep the default RTT
        MaxSynRetransmissions: TCP_INITIAL_RTO_NO_SYN_RETRANSMISSIONS as u8,
    };
    let mut returned = 0u32;
    unsafe {
        WSAIoctl(
            sock.as_raw_socket() as _,
            SIO_TCP_INITIAL_RTO,
            (&params as *const TCP_INITIAL_RTO_PARAMETERS).cast(),
            std::mem::size_of::<TCP_INITIAL_RTO_PARAMETERS>() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
            None,
        );
    }
}

/// Spaces out probe starts across all workers: at most `per_sec` per second,
/// evenly spaced (no bursts). 0 means unlimited.
pub struct Pacer {
    interval: Option<Duration>,
    next: Mutex<Instant>,
}

impl Pacer {
    pub fn new(per_sec: u32) -> Self {
        Self {
            interval: (per_sec > 0).then(|| Duration::from_secs_f64(1.0 / per_sec as f64)),
            next: Mutex::new(Instant::now()),
        }
    }

    /// Block until this caller's slot. Returns false if cancelled while waiting.
    pub fn wait(&self, cancel: &AtomicBool) -> bool {
        let Some(interval) = self.interval else {
            return !cancel.load(Ordering::Relaxed);
        };
        let slot = {
            let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
            let slot = (*next).max(Instant::now());
            *next = slot + interval;
            slot
        };
        loop {
            if cancel.load(Ordering::Relaxed) {
                return false;
            }
            let left = slot.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            std::thread::sleep(left.min(Duration::from_millis(50)));
        }
    }
}

/// Run `f` over `items` on up to `workers` threads, at most `per_sec` starts
/// per second (0 = unlimited), stopping early on cancel.
pub fn par_for<T: Sync>(
    items: &[T],
    workers: usize,
    per_sec: u32,
    cancel: &AtomicBool,
    f: impl Fn(&T) + Sync,
) {
    let next = AtomicUsize::new(0);
    let pacer = Pacer::new(per_sec);
    let workers = workers.clamp(1, items.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                while pacer.wait(cancel) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    f(item);
                }
            });
        }
    });
}

/// A connection that gives up at a deadline: each read or write waits at
/// most `each`, and never past `deadline`. A per-read timeout alone lets a
/// device that dribbles a byte at a time hold a worker (and a Stop) for
/// hours; this bounds the whole exchange.
pub(crate) struct Timed {
    tcp: std::net::TcpStream,
    deadline: Instant,
    each: Duration,
}

impl Timed {
    /// `tcp`, done within `whole` from now.
    pub fn new(tcp: std::net::TcpStream, each: Duration, whole: Duration) -> Self {
        Self {
            tcp,
            deadline: Instant::now() + whole,
            each,
        }
    }

    fn arm(&self) -> io::Result<()> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "took too long"));
        }
        let wait = Some(left.min(self.each));
        self.tcp.set_read_timeout(wait)?;
        self.tcp.set_write_timeout(wait)
    }
}

impl io::Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.arm()?;
        self.tcp.read(buf)
    }
}

impl io::Write for Timed {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.arm()?;
        self.tcp.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.tcp.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program that doesn't finish is stopped at the limit; one that
    /// does gives what it printed.
    #[cfg(unix)]
    #[test]
    fn a_slow_program_is_stopped_at_the_limit() {
        use std::time::{Duration, Instant};
        let t = Instant::now();
        let r = output_within(
            std::process::Command::new("sleep").arg("10"),
            Duration::from_millis(300),
        );
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_secs(3));
        let out = output_within(
            std::process::Command::new("echo").arg("joined"),
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "joined\n");
    }
    use std::net::TcpListener;

    #[test]
    fn tcp_probe_open_and_closed() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let open = l.local_addr().unwrap();
        assert_eq!(tcp_probe(open, Duration::from_secs(1)).0, TcpState::Open);
        drop(l);
        assert_eq!(tcp_probe(open, Duration::from_secs(1)).0, TcpState::Closed);
    }

    #[test]
    fn pacer_caps_rate() {
        let cancel = AtomicBool::new(false);
        let items: Vec<u32> = (0..21).collect();
        let t0 = Instant::now();
        // 21 starts at 100/s = 20 gaps of 10 ms, despite 8 workers.
        par_for(&items, 8, 100, &cancel, |_| {});
        let took = t0.elapsed();
        assert!(took >= Duration::from_millis(195), "{took:?}");
        assert!(took < Duration::from_millis(600), "{took:?}");
    }

    #[test]
    fn pacer_stops_on_cancel() {
        let cancel = AtomicBool::new(true);
        assert!(!Pacer::new(1).wait(&cancel));
        assert!(!Pacer::new(0).wait(&cancel));
    }

    #[test]
    fn sanitizes_ptr_names() {
        assert_eq!(
            sanitize("host.example\u{202E}moc.lapyap"),
            "host.examplemoc.lapyap"
        );
        assert_eq!(sanitize("a\r\nb\x1b[31m"), "ab[31m");
        assert_eq!(sanitize(&"x".repeat(300)).len(), 255);
    }

    #[test]
    fn resolves_literals() {
        assert_eq!(
            resolve(" 10.0.0.1 ", false).unwrap(),
            "10.0.0.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            resolve("[::1]", false).unwrap(),
            "::1".parse::<IpAddr>().unwrap()
        );
        // Said plainly, not "Invalid argument" from the socket later.
        let zoned = resolve("fe80::1%eth0", false).unwrap_err().to_string();
        assert!(zoned.contains("link-local address with a zone"), "{zoned}");
        assert!(
            resolve("ff02::1", false)
                .unwrap_err()
                .to_string()
                .contains("multicast")
        );
        assert!(resolve("224.0.0.251", false).is_err());
    }
}
