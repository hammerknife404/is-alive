//! How this computer's network adapter is set up: which adapter carries
//! the route to an address (and whether it's Wi-Fi or a VPN), and what it
//! was given: DHCP server, DNS servers and DNS suffix. Read from the
//! system, nothing sent. Windows: GetAdaptersAddresses. Linux:
//! NetworkManager, else systemd-networkd's lease file, else unknown.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};

/// The adapter that carries the route to an address.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Adapter {
    /// Its name: `wlan0`, or Windows' friendly name ("Wi-Fi").
    pub name: String,
    /// Its driver's description on Windows ("Intel(R) Wi-Fi 6 AX201").
    pub desc: String,
    pub vpn: bool,
    pub wifi: bool,
    /// Its link MTU.
    pub mtu: Option<u32>,
    /// Windows: the adapter's GUID ("{…}"), how wlanapi names it.
    pub guid: String,
}

/// Where the DHCP lease came from.
#[derive(Clone, Debug, PartialEq)]
pub enum Dhcp {
    Server(Ipv4Addr),
    /// The address is set by hand (or the system says DHCP is off).
    Static,
    Unknown,
}

/// What this computer's adapter was given.
#[derive(Clone, Debug, PartialEq)]
pub struct NetConf {
    pub dhcp: Dhcp,
    /// In the order the system uses them; empty when unknown.
    pub dns: Vec<IpAddr>,
    /// The connection's DNS suffix (search domain), when it has one.
    pub suffix: Option<String>,
}

impl NetConf {
    pub fn unknown() -> Self {
        Self {
            dhcp: Dhcp::Unknown,
            dns: Vec::new(),
            suffix: None,
        }
    }

    pub fn dhcp_text(&self) -> String {
        match self.dhcp {
            Dhcp::Server(ip) => ip.to_string(),
            Dhcp::Static => "none (static address)".into(),
            Dhcp::Unknown => "unknown".into(),
        }
    }

    pub fn dns_text(&self) -> String {
        if self.dns.is_empty() {
            "unknown".into()
        } else {
            let v: Vec<String> = self.dns.iter().map(IpAddr::to_string).collect();
            v.join(", ")
        }
    }

    /// "none" only when the rest was read: an unknown setup's suffix is
    /// unknown too.
    pub fn suffix_text(&self) -> String {
        match (&self.suffix, &self.dhcp, self.dns.is_empty()) {
            (Some(s), ..) => s.clone(),
            (None, Dhcp::Unknown, true) => "unknown".into(),
            (None, ..) => "none".into(),
        }
    }
}

/// The local address the system would send from to reach `dest`: a UDP
/// connect only looks the route up, nothing is sent.
pub fn source_for(dest: Ipv4Addr) -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect((dest, 9)).ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(ip) => Some(ip),
        IpAddr::V6(_) => None,
    }
}

/// Names a VPN client gives its adapter, as Windows describes it or Linux
/// names it.
const VPN_WORDS: &[&str] = &[
    "vpn",
    "wireguard",
    "wintun",
    "tap-windows",
    "tap-",
    "openvpn",
    "anyconnect",
    "globalprotect",
    "pangp",
    "fortinet",
    "forticlient",
    "netextender",
    "sonicwall",
    "juniper",
    "pulse secure",
    "zscaler",
    "tailscale",
    "zerotier",
    "nordlynx",
    "cloudflare warp",
];

/// Linux interface names VPNs use.
const VPN_PREFIXES: &[&str] = &[
    "tun",
    "tap",
    "wg",
    "ppp",
    "ipsec",
    "vti",
    "gpd",
    "cscotun",
    "utun",
    "tailscale",
    "zt",
    "nordlynx",
];

/// Whether an adapter looks like a VPN's, from its name and description.
pub fn vpn_like(name: &str, desc: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let d = desc.to_ascii_lowercase();
    VPN_PREFIXES.iter().any(|p| n.starts_with(p))
        || VPN_WORDS.iter().any(|w| d.contains(w) || n.contains(w))
}

// ------------------------------------------------------------- Linux

#[cfg(target_os = "linux")]
fn iface_of(ip: Ipv4Addr) -> Option<String> {
    if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .find(|i| matches!(&i.addr, if_addrs::IfAddr::V4(a) if a.ip == ip))
        .map(|i| i.name)
}

/// The adapter carrying the route to `dest`.
#[cfg(target_os = "linux")]
pub fn adapter_to(dest: Ipv4Addr) -> Option<Adapter> {
    let name = iface_of(source_for(dest)?)?;
    let sys = std::path::Path::new("/sys/class/net").join(&name);
    let mtu = std::fs::read_to_string(sys.join("mtu"))
        .ok()
        .and_then(|m| m.trim().parse().ok());
    // A tun device (most VPN clients) has tun_flags; ARPHRD_NONE (65534)
    // and PPP (512) are point-to-point links with no MAC.
    let kind: u32 = std::fs::read_to_string(sys.join("type"))
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(1);
    let vpn = vpn_like(&name, "")
        || sys.join("tun_flags").exists()
        || matches!(kind, 65534 | 512 | 768 | 769 | 776 | 778);
    Some(Adapter {
        wifi: sys.join("wireless").exists() || sys.join("phy80211").exists(),
        vpn,
        mtu,
        name: super::sanitize(&name),
        ..Default::default()
    })
}

/// DHCP server, DNS and suffix of the adapter holding `ip`.
#[cfg(target_os = "linux")]
pub fn read(ip: Ipv4Addr) -> NetConf {
    let Some(name) = iface_of(ip) else {
        return NetConf::unknown();
    };
    if let Some(c) = nmcli(&name) {
        return c;
    }
    let lease = std::fs::read_to_string(format!("/sys/class/net/{name}/ifindex"))
        .ok()
        .and_then(|i| {
            std::fs::read_to_string(format!("/run/systemd/netif/leases/{}", i.trim())).ok()
        });
    lease
        .as_deref()
        .map(parse_networkd_lease)
        .unwrap_or_else(NetConf::unknown)
}

#[cfg(target_os = "linux")]
fn nmcli(iface: &str) -> Option<NetConf> {
    let prog = super::system_tool("nmcli")?;
    let mut cmd = std::process::Command::new(prog);
    cmd.env("LC_ALL", "C").args([
        "-t",
        "-f",
        "GENERAL.STATE,IP4.DNS,IP4.DOMAIN,DHCP4",
        "device",
        "show",
        iface,
    ]);
    let out = super::output_within(&mut cmd, std::time::Duration::from_secs(3)).ok()?;
    if !out.status.success() {
        return None;
    }
    parse_nmcli(&String::from_utf8_lossy(&out.stdout))
}

/// `nmcli -t -f GENERAL.STATE,IP4.DNS,IP4.DOMAIN,DHCP4 device show <if>`.
/// None when NetworkManager doesn't manage the device.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_nmcli(text: &str) -> Option<NetConf> {
    let mut conf = NetConf::unknown();
    let mut managed = false;
    let mut dhcp_seen = false;
    let mut dhcp_domain = None;
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        // Terse mode escapes colons in values.
        let v = v.replace("\\:", ":");
        let v = v.trim();
        let key = k.split('[').next().unwrap_or(k);
        match key {
            "GENERAL.STATE" => {
                managed = !v.contains("unmanaged") && !v.is_empty();
            }
            "IP4.DNS" => {
                if let Ok(ip) = v.parse() {
                    conf.dns.push(ip);
                }
            }
            "IP4.DOMAIN" => {
                if conf.suffix.is_none() && !v.is_empty() {
                    conf.suffix = Some(super::sanitize(v));
                }
            }
            "DHCP4.OPTION" => {
                dhcp_seen = true;
                let Some((opt, val)) = v.split_once('=') else {
                    continue;
                };
                let val = val.trim();
                match opt.trim() {
                    "dhcp_server_identifier" => {
                        if let Ok(ip) = val.parse() {
                            conf.dhcp = Dhcp::Server(ip);
                        }
                    }
                    "domain_name" if !val.is_empty() => {
                        dhcp_domain = Some(super::sanitize(val));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if !managed {
        return None;
    }
    if conf.dhcp == Dhcp::Unknown && !dhcp_seen && !conf.dns.is_empty() {
        // Configured, and no lease: set by hand.
        conf.dhcp = Dhcp::Static;
    }
    if conf.suffix.is_none() {
        conf.suffix = dhcp_domain;
    }
    Some(conf)
}

/// A systemd-networkd lease file (`/run/systemd/netif/leases/<ifindex>`):
/// KEY=value lines.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_networkd_lease(text: &str) -> NetConf {
    let mut conf = NetConf::unknown();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "SERVER_ADDRESS" => {
                if let Ok(ip) = v.parse() {
                    conf.dhcp = Dhcp::Server(ip);
                }
            }
            "DNS" => {
                conf.dns = v
                    .split_whitespace()
                    .filter_map(|a| a.parse().ok())
                    .collect()
            }
            "DOMAINNAME" if !v.is_empty() => conf.suffix = Some(super::sanitize(v)),
            _ => {}
        }
    }
    conf
}

// ------------------------------------------------------------- Windows

#[cfg(windows)]
mod win {
    use super::{Adapter, Dhcp, NetConf};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses,
        IP_ADAPTER_ADDRESSES_LH, IP_ADAPTER_DHCP_ENABLED,
    };
    use windows_sys::Win32::Networking::WinSock::{
        AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6, SOCKET_ADDRESS,
    };

    /// IF_TYPE_IEEE80211; IF_TYPE_PPP and IF_TYPE_TUNNEL (Windows' own VPN
    /// connections). Not IF_TYPE_PROP_VIRTUAL: Hyper-V's switches are that.
    const WIFI: u32 = 71;
    const VIRTUAL_TYPES: [u32; 2] = [23, 131];

    unsafe fn addr(sa: &SOCKET_ADDRESS) -> Option<IpAddr> {
        unsafe {
            if sa.lpSockaddr.is_null() {
                return None;
            }
            match (*sa.lpSockaddr).sa_family {
                AF_INET => {
                    let s = std::ptr::read_unaligned(sa.lpSockaddr.cast::<SOCKADDR_IN>());
                    Some(IpAddr::V4(Ipv4Addr::from(
                        s.sin_addr.S_un.S_addr.to_ne_bytes(),
                    )))
                }
                AF_INET6 => {
                    let s = std::ptr::read_unaligned(sa.lpSockaddr.cast::<SOCKADDR_IN6>());
                    Some(IpAddr::V6(Ipv6Addr::from(s.sin6_addr.u.Byte)))
                }
                _ => None,
            }
        }
    }

    unsafe fn wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        unsafe {
            let mut n = 0;
            while *p.add(n) != 0 && n < 1024 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
        }
    }

    unsafe fn narrow(p: *const u8) -> String {
        if p.is_null() {
            return String::new();
        }
        unsafe {
            std::ffi::CStr::from_ptr(p.cast())
                .to_string_lossy()
                .into_owned()
        }
    }

    /// Call `f` on the adapter holding `ip`, if there is one.
    fn with_adapter<T>(ip: Ipv4Addr, f: impl FnOnce(&IP_ADAPTER_ADDRESSES_LH) -> T) -> Option<T> {
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
        // 15 KB is Microsoft's advice for the first try; grow if told to.
        let mut len: u32 = 15_000;
        let mut buf: Vec<u64>;
        let mut tries = 0;
        loop {
            buf = vec![0u64; (len as usize).div_ceil(8)];
            let r = unsafe {
                GetAdaptersAddresses(
                    AF_UNSPEC as u32,
                    flags,
                    std::ptr::null(),
                    buf.as_mut_ptr().cast(),
                    &mut len,
                )
            };
            match r {
                0 => break,
                // ERROR_BUFFER_OVERFLOW
                111 if tries < 3 => tries += 1,
                _ => return None,
            }
        }
        let mut a = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while !a.is_null() {
            let ad = unsafe { &*a };
            let mut u = ad.FirstUnicastAddress;
            while !u.is_null() {
                let ua = unsafe { &*u };
                if unsafe { addr(&ua.Address) } == Some(IpAddr::V4(ip)) {
                    return Some(f(ad));
                }
                u = ua.Next;
            }
            a = ad.Next;
        }
        None
    }

    pub fn adapter_to(dest: Ipv4Addr) -> Option<Adapter> {
        let ip = super::source_for(dest)?;
        with_adapter(ip, |ad| {
            let name = unsafe { wide(ad.FriendlyName) };
            let desc = unsafe { wide(ad.Description) };
            Adapter {
                vpn: VIRTUAL_TYPES.contains(&ad.IfType) || super::vpn_like(&name, &desc),
                wifi: ad.IfType == WIFI,
                mtu: Some(ad.Mtu),
                guid: unsafe { narrow(ad.AdapterName) },
                name: crate::net::sanitize(&name),
                desc: crate::net::sanitize(&desc),
            }
        })
    }

    pub fn read(ip: Ipv4Addr) -> NetConf {
        with_adapter(ip, |ad| {
            let dhcp_on = unsafe { ad.Anonymous2.Flags } & IP_ADAPTER_DHCP_ENABLED != 0;
            let dhcp = match unsafe { addr(&ad.Dhcpv4Server) } {
                // No lease yet reads as 0.0.0.0 or 255.255.255.255.
                Some(IpAddr::V4(s)) if dhcp_on && !s.is_unspecified() && !s.is_broadcast() => {
                    Dhcp::Server(s)
                }
                _ if !dhcp_on => Dhcp::Static,
                _ => Dhcp::Unknown,
            };
            let mut dns = Vec::new();
            let mut d = ad.FirstDnsServerAddress;
            while !d.is_null() {
                let e = unsafe { &*d };
                // Windows' unconfigured IPv6 placeholders (fec0:0:0:ffff::1…).
                match unsafe { addr(&e.Address) } {
                    Some(IpAddr::V6(v6)) if v6.segments()[0] == 0xfec0 => {}
                    Some(a) => dns.push(a),
                    None => {}
                }
                d = e.Next;
            }
            let suffix = unsafe { wide(ad.DnsSuffix) };
            NetConf {
                dhcp,
                dns,
                suffix: Some(crate::net::sanitize(suffix.trim())).filter(|s| !s.is_empty()),
            }
        })
        .unwrap_or_else(NetConf::unknown)
    }
}

#[cfg(windows)]
pub use win::{adapter_to, read};

#[cfg(not(any(windows, target_os = "linux")))]
pub fn adapter_to(_: Ipv4Addr) -> Option<Adapter> {
    None
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn read(_: Ipv4Addr) -> NetConf {
    NetConf::unknown()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networkmanager_says_where_the_lease_came_from() {
        let out = "GENERAL.STATE:100 (connected)\n\
                   IP4.DNS[1]:192.0.2.10\n\
                   IP4.DNS[2]:192.0.2.11\n\
                   IP4.DOMAIN[1]:corp.example.com\n\
                   DHCP4.OPTION[1]:broadcast_address = 192.0.2.255\n\
                   DHCP4.OPTION[2]:dhcp_server_identifier = 192.0.2.10\n\
                   DHCP4.OPTION[3]:domain_name = corp.example.com\n";
        let c = parse_nmcli(out).unwrap();
        assert_eq!(c.dhcp, Dhcp::Server("192.0.2.10".parse().unwrap()));
        assert_eq!(c.dns_text(), "192.0.2.10, 192.0.2.11");
        assert_eq!(c.suffix_text(), "corp.example.com");
        // A server other than the gateway is normal (a domain controller):
        // nothing is judged from it.
        // Set by hand: DNS, no lease.
        let c = parse_nmcli("GENERAL.STATE:100 (connected)\nIP4.DNS[1]:1.1.1.1\n").unwrap();
        assert_eq!(c.dhcp, Dhcp::Static);
        assert_eq!(c.suffix_text(), "none");
        // Not NetworkManager's: the lease file is asked instead.
        assert_eq!(parse_nmcli("GENERAL.STATE:10 (unmanaged)\n"), None);
        assert_eq!(parse_nmcli(""), None);
    }

    #[test]
    fn a_networkd_lease_names_the_server() {
        let lease = "# This is private data. Do not parse.\nADDRESS=192.0.2.50\n\
                     ROUTER=192.0.2.1\nSERVER_ADDRESS=192.0.2.2\nDNS=192.0.2.2 192.0.2.3\n\
                     DOMAINNAME=office.example.net\n";
        let c = parse_networkd_lease(lease);
        assert_eq!(c.dhcp_text(), "192.0.2.2");
        assert_eq!(c.dns.len(), 2);
        assert_eq!(c.suffix.as_deref(), Some("office.example.net"));
        let none = NetConf::unknown();
        assert_eq!(
            (none.dhcp_text(), none.dns_text(), none.suffix_text()),
            ("unknown".into(), "unknown".into(), "unknown".into())
        );
    }

    #[test]
    fn hostile_netconf_text_never_panics() {
        crate::hostile::each_text(
            "GENERAL.STATE:100 (connected)\nIP4.DNS[1]:192.0.2.1\n\
             DHCP4.OPTION[2]:dhcp_server_identifier = 192.0.2.1\n",
            |t| {
                let _ = parse_nmcli(t);
            },
        );
        crate::hostile::each_text("SERVER_ADDRESS=192.0.2.1\nDNS=192.0.2.1\n", |t| {
            let _ = parse_networkd_lease(t);
        });
    }

    #[test]
    fn vpn_adapters_are_known_by_name() {
        assert!(vpn_like("wg0", ""));
        assert!(vpn_like("tun0", ""));
        assert!(vpn_like(
            "Ethernet 3",
            "PANGP Virtual Ethernet Adapter Secure"
        ));
        assert!(vpn_like("Local Area Connection", "WireGuard Tunnel"));
        assert!(!vpn_like("eth0", ""));
        assert!(!vpn_like("Wi-Fi", "Intel(R) Wi-Fi 6 AX201 160MHz"));
        assert!(!vpn_like("enp3s0", "Realtek PCIe GbE Family Controller"));
    }
}
