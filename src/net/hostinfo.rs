//! This machine: name, LAN address and MAC, gateway and public (WAN) address.

use std::net::{Ipv4Addr, UdpSocket};
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct HostInfo {
    /// Address and prefix length on the default-route interface.
    pub lan: Option<(Ipv4Addr, u32)>,
    pub gateway: Option<Ipv4Addr>,
    pub wan: Option<Ipv4Addr>,
    /// Computer name: the NetBIOS name on Windows, else the hostname.
    pub name: Option<String>,
    /// Active Directory domain, when the machine is joined to one.
    pub domain: Option<String>,
    /// MAC of the LAN interface.
    pub mac: Option<String>,
}

impl HostInfo {
    /// LAN and gateway only; no network traffic.
    pub fn local() -> Self {
        let lan = lan();
        Self {
            lan,
            gateway: gateway(),
            wan: None,
            name: computer_name(),
            domain: domain(),
            mac: lan.and_then(|(ip, _)| mac_of(ip)),
        }
    }

    /// Aligned lines for the Results log.
    pub fn summary(&self) -> String {
        self.fields()
            .iter()
            .map(|(k, v)| format!("{k:<9}{v}\n"))
            .collect()
    }

    /// A Markdown block for the Notes tab, stamped with `when`.
    pub fn markdown(&self, when: &str) -> String {
        let mut s = format!("**This machine** · {when}\n");
        for (k, v) in self.fields() {
            s += &format!("- {k}: {v}\n");
        }
        s
    }

    /// (label, value) for what's known, in reading order.
    fn fields(&self) -> Vec<(&'static str, String)> {
        let mut f = Vec::new();
        if let Some(n) = &self.name {
            f.push((
                "Computer",
                match &self.domain {
                    Some(d) => format!("{n} ({d})"),
                    None => n.clone(),
                },
            ));
        }
        f.push((
            "LAN",
            self.lan
                .map_or("unknown".into(), |(ip, bits)| format!("{ip}/{bits}")),
        ));
        if let Some(m) = &self.mac {
            f.push(("MAC", m.clone()));
        }
        f.push((
            "Gateway",
            self.gateway.map_or("unknown".into(), |g| g.to_string()),
        ));
        f.push((
            "WAN",
            self.wan.map_or("not looked up".into(), |w| w.to_string()),
        ));
        f
    }

    pub fn table(&self, version: &str) -> crate::log::Table {
        let row = |k: &str, v: String| vec![k.to_string(), v];
        crate::log::Table {
            columns: &["field", "value"],
            rows: vec![
                row("version", version.to_string()),
                row(
                    "lan",
                    crate::log::cell(self.lan.map(|(ip, bits)| format!("{ip}/{bits}"))),
                ),
                row("gateway", crate::log::cell(self.gateway)),
                row("wan", crate::log::cell(self.wan)),
            ],
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

/// NetBIOS name on Windows (COMPUTERNAME), else the hostname.
fn computer_name() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| dns_lookup::get_hostname().ok())
        .map(|n| super::sanitize(n.trim()))
        .filter(|n| !n.is_empty())
}

/// The AD domain Windows was logged on to, if any. A workgroup machine has
/// none (USERDOMAIN is then just the computer name, so it isn't used).
fn domain() -> Option<String> {
    std::env::var("USERDNSDOMAIN")
        .ok()
        .map(|d| super::sanitize(d.trim()))
        .filter(|d| !d.is_empty())
}

/// MAC of the interface holding `ip`.
#[cfg(target_os = "linux")]
fn mac_of(ip: Ipv4Addr) -> Option<String> {
    let name = if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .find(|i| matches!(&i.addr, if_addrs::IfAddr::V4(a) if a.ip == ip))?
        .name;
    let m = std::fs::read_to_string(format!("/sys/class/net/{name}/address")).ok()?;
    let m = super::mac::parse_mac(m.trim())?;
    (m != [0; 6]).then(|| super::mac::fmt(&m))
}

#[cfg(windows)]
fn mac_of(ip: Ipv4Addr) -> Option<String> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetAdaptersInfo, IP_ADAPTER_INFO};
    let mut len: u32 = 0;
    unsafe { GetAdaptersInfo(std::ptr::null_mut(), &mut len) };
    if len == 0 {
        return None;
    }
    // u64s keep the buffer aligned for IP_ADAPTER_INFO.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    let first = buf.as_mut_ptr().cast::<IP_ADAPTER_INFO>();
    if unsafe { GetAdaptersInfo(first, &mut len) } != 0 {
        return None;
    }
    let want = ip.to_string();
    let mut a = first;
    while !a.is_null() {
        let info = unsafe { &*a };
        let mut addr: *const _ = &info.IpAddressList;
        while !addr.is_null() {
            let entry = unsafe { &*addr };
            let raw = &entry.IpAddress.String;
            let n = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            // CHARs: i8 on Windows.
            if raw[..n].iter().map(|&b| b as u8).eq(want.bytes()) && info.AddressLength == 6 {
                let mut m = [0u8; 6];
                m.copy_from_slice(&info.Address[..6]);
                return Some(super::mac::fmt(&m));
            }
            addr = entry.Next;
        }
        a = info.Next;
    }
    None
}

#[cfg(not(any(windows, target_os = "linux")))]
fn mac_of(_: Ipv4Addr) -> Option<String> {
    None
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
