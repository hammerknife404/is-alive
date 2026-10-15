//! This machine's LAN address, gateway and public (WAN) address.

use std::net::{Ipv4Addr, UdpSocket};
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct HostInfo {
    /// Address and prefix length on the default-route interface.
    pub lan: Option<(Ipv4Addr, u32)>,
    pub gateway: Option<Ipv4Addr>,
    pub wan: Option<Ipv4Addr>,
}

impl HostInfo {
    /// LAN and gateway only; no network traffic.
    pub fn local() -> Self {
        Self {
            lan: lan(),
            gateway: gateway(),
            wan: None,
        }
    }

    /// Everything, including one DNS query for the public address.
    pub fn fetch() -> Self {
        Self {
            wan: wan(),
            ..Self::local()
        }
    }
}

/// IPv4 address and prefix length of the interface that holds the default route.
pub fn lan() -> Option<(Ipv4Addr, u32)> {
    // Connecting a UDP socket only does a route lookup; nothing is sent.
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    let std::net::IpAddr::V4(ip) = sock.local_addr().ok()?.ip() else {
        return None;
    };
    let mask = if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .find_map(|i| match i.addr {
            if_addrs::IfAddr::V4(a) if a.ip == ip => Some(a.netmask),
            _ => None,
        })?;
    Some((ip, u32::from(mask).leading_ones()))
}

#[cfg(target_os = "linux")]
pub fn gateway() -> Option<Ipv4Addr> {
    parse_proc_route(&std::fs::read_to_string("/proc/net/route").ok()?)
}

/// Default-route gateway from /proc/net/route (hex, host byte order).
#[cfg(any(target_os = "linux", test))]
fn parse_proc_route(table: &str) -> Option<Ipv4Addr> {
    table.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 3 || f[1] != "00000000" {
            return None;
        }
        let gw = u32::from_str_radix(f[2], 16).ok()?;
        (gw != 0).then(|| Ipv4Addr::from(gw.to_le_bytes()))
    })
}

#[cfg(windows)]
pub fn gateway() -> Option<Ipv4Addr> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetBestRoute, MIB_IPFORWARDROW};
    let mut row: MIB_IPFORWARDROW = unsafe { std::mem::zeroed() };
    let dest = u32::from_ne_bytes([1, 1, 1, 1]);
    if unsafe { GetBestRoute(dest, 0, &mut row) } != 0 {
        return None;
    }
    let gw = Ipv4Addr::from(row.dwForwardNextHop.to_ne_bytes());
    (!gw.is_unspecified()).then_some(gw)
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn gateway() -> Option<Ipv4Addr> {
    None
}

/// Public IPv4 via OpenDNS: `myip.opendns.com` resolves to the asker's address.
pub fn wan() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let id = std::process::id() as u16;
    sock.send_to(&dns_query(id, "myip.opendns.com"), "208.67.222.222:53")
        .ok()?;
    let mut buf = [0u8; 512];
    let n = sock.recv(&mut buf).ok()?;
    parse_a_answer(&buf[..n], id)
}

fn dns_query(id: u16, name: &str) -> Vec<u8> {
    let mut q = id.to_be_bytes().to_vec();
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]); // RD, 1 question
    for label in name.split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 1, 0, 1]); // root, type A, class IN
    q
}

/// First A record in a DNS response with matching id.
fn parse_a_answer(r: &[u8], id: u16) -> Option<Ipv4Addr> {
    if r.len() < 12 || r[..2] != id.to_be_bytes() || r[3] & 0x0f != 0 {
        return None;
    }
    let qd = u16::from_be_bytes([r[4], r[5]]);
    let an = u16::from_be_bytes([r[6], r[7]]);
    let mut i = 12;
    let skip_name = |mut i: usize| -> Option<usize> {
        loop {
            let len = *r.get(i)? as usize;
            if len & 0xc0 == 0xc0 {
                return Some(i + 2);
            }
            i += 1 + len;
            if len == 0 {
                return Some(i);
            }
        }
    };
    for _ in 0..qd {
        i = skip_name(i)? + 4;
    }
    for _ in 0..an {
        i = skip_name(i)?;
        let h = r.get(i..i + 10)?;
        let (ty, len) = (
            u16::from_be_bytes([h[0], h[1]]),
            u16::from_be_bytes([h[8], h[9]]),
        );
        i += 10;
        let data = r.get(i..i + len as usize)?;
        if ty == 1 && len == 4 {
            return Some(Ipv4Addr::new(data[0], data[1], data[2], data[3]));
        }
        i += len as usize;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_gateway() {
        let t = "Iface\tDestination\tGateway \tFlags\n\
                 eth0\t0000A8C0\t00000000\t0001\n\
                 eth0\t00000000\t0100A8C0\t0003\n";
        assert_eq!(parse_proc_route(t), Some(Ipv4Addr::new(192, 168, 0, 1)));
    }

    #[test]
    fn parses_a_answer() {
        let mut r = dns_query(7, "myip.opendns.com");
        r[2] = 0x81; // response
        r[7] = 1; // one answer
        r.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 0, 0, 4, 203, 0, 113, 5]);
        assert_eq!(parse_a_answer(&r, 7), Some(Ipv4Addr::new(203, 0, 113, 5)));
        assert_eq!(parse_a_answer(&r, 8), None);
    }
}
