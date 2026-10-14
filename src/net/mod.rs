pub mod icmp;
pub mod ports;
pub mod targets;

use socket2::{Domain, Protocol, Socket, Type};
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Resolve a host name or IP literal. Prefers IPv4 unless `prefer_v6`.
pub fn resolve(host: &str, prefer_v6: bool) -> io::Result<IpAddr> {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
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

pub fn reverse(ip: IpAddr) -> Option<String> {
    dns_lookup::lookup_addr(&ip)
        .ok()
        .filter(|n| n != &ip.to_string())
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

/// Run `f` over `items` on up to `workers` threads, stopping early on cancel.
pub fn par_for<T: Sync>(items: &[T], workers: usize, cancel: &AtomicBool, f: impl Fn(&T) + Sync) {
    let next = AtomicUsize::new(0);
    let workers = workers.clamp(1, items.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                while !cancel.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    f(item);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn resolves_literals() {
        assert_eq!(
            resolve(" 10.0.0.1 ", false).unwrap(),
            "10.0.0.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            resolve("[::1]", false).unwrap(),
            "::1".parse::<IpAddr>().unwrap()
        );
    }
}
