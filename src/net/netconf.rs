//! How this computer's network adapter is set up: which adapter carries
//! the route to an address (and whether it's Wi-Fi or a VPN), and what it
//! was given: DHCP server and lease times, DNS servers and DNS suffix.
//! Read from the system, nothing sent. Windows: GetAdaptersAddresses (and
//! GetAdaptersInfo for the lease times, which it alone keeps). Linux:
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

/// A wired (Ethernet) adapter: where the switch's LLDP and CDP can be
/// heard. Wi-Fi never carries them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wired {
    /// Its name: `eth0`, or Windows' friendly name ("Ethernet 2").
    pub name: String,
    /// What a capture opens: the Linux interface name, or Windows' GUID.
    pub dev: String,
    /// Linux's interface index (0 on Windows).
    pub index: u32,
    pub mac: Option<[u8; 6]>,
    /// It has a link (a cable to something switched on).
    pub up: bool,
    /// It holds this computer's LAN address.
    pub lan: bool,
}

/// An adapter with a default route: where it sends what isn't local.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// Its name: `wlan0`, or Windows' friendly name ("Wi-Fi").
    pub name: String,
    pub wifi: bool,
    pub vpn: bool,
    pub gateway: Ipv4Addr,
}

/// Every adapter that's up with a default gateway, from the routing table
/// (Linux) or the adapters' own list (Windows): read, nothing sent.
#[cfg(target_os = "linux")]
pub fn gateways() -> Vec<Route> {
    let Ok(table) = std::fs::read_to_string("/proc/net/route") else {
        return Vec::new();
    };
    parse_default_routes(&table)
        .into_iter()
        .map(|(name, gateway)| {
            let sys = std::path::Path::new("/sys/class/net").join(&name);
            let kind: u32 = std::fs::read_to_string(sys.join("type"))
                .ok()
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(1);
            Route {
                wifi: sys.join("wireless").exists() || sys.join("phy80211").exists(),
                vpn: vpn_like(&name, "")
                    || sys.join("tun_flags").exists()
                    || matches!(kind, 65534 | 512 | 768 | 769 | 776 | 778),
                name: super::sanitize(&name),
                gateway,
            }
        })
        .collect()
}

/// /proc/net/route's default routes through a gateway: (interface,
/// gateway), each interface once, in the table's order.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_default_routes(table: &str) -> Vec<(String, Ipv4Addr)> {
    let mut out: Vec<(String, Ipv4Addr)> = Vec::new();
    for l in table.lines().skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        // Iface Destination Gateway Flags RefCnt Use Metric Mask …
        if f.len() < 8 || f[1] != "00000000" || f[7] != "00000000" {
            continue;
        }
        let flags = u32::from_str_radix(f[3], 16).unwrap_or(0);
        let Ok(gw) = u32::from_str_radix(f[2], 16) else {
            continue;
        };
        // RTF_UP and RTF_GATEWAY.
        if flags & 0x3 != 0x3 || gw == 0 || out.iter().any(|r| r.0 == f[0]) {
            continue;
        }
        out.push((f[0].to_string(), Ipv4Addr::from(gw.to_le_bytes())));
    }
    out
}

/// The wired adapter to listen on: the one with the LAN address and a
/// link, else the first with a link, else (to say "no link") the first.
pub fn pick_wired(all: &[Wired]) -> Option<&Wired> {
    all.iter()
        .find(|w| w.up && w.lan)
        .or_else(|| all.iter().find(|w| w.up))
        .or_else(|| all.first())
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
    /// When the DHCP lease was obtained (or last renewed) and when it
    /// runs out, as Unix seconds. Reported, not judged.
    pub lease_obtained: Option<i64>,
    pub lease_expires: Option<i64>,
}

impl NetConf {
    pub fn unknown() -> Self {
        Self {
            dhcp: Dhcp::Unknown,
            dns: Vec::new(),
            suffix: None,
            lease_obtained: None,
            lease_expires: None,
        }
    }

    /// A lease time in local time ("2026-11-24 08:02"); "none" with a
    /// static address, else "unknown".
    pub fn lease_text(&self, at: Option<i64>) -> String {
        match (at.and_then(local_time), &self.dhcp) {
            (Some(t), _) => t,
            (None, Dhcp::Static) => "none".into(),
            (None, _) => "unknown".into(),
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

/// Unix seconds as local time, to the minute; None for 0 (Windows' "no
/// lease") or a time chrono can't hold.
fn local_time(secs: i64) -> Option<String> {
    use chrono::TimeZone;
    if secs <= 0 {
        return None;
    }
    let t = chrono::Local.timestamp_opt(secs, 0).single()?;
    Some(t.format("%Y-%m-%d %H:%M").to_string())
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

/// Words in a virtual Ethernet adapter's name or description (Windows):
/// a hypervisor's switch or host-only network, a container's, a capture
/// driver's loopback. Only used when Windows won't say whether an adapter
/// is hardware.
const VIRTUAL_WORDS: &[&str] = &[
    "vethernet",
    "hyper-v",
    "virtual ethernet",
    "vmware",
    "vmnet",
    "virtualbox",
    "host-only",
    "docker",
    "wsl",
    "loopback",
    "parallels",
];

/// Whether an Ethernet adapter looks virtual, from its name and description.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn virtual_like(name: &str, desc: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let d = desc.to_ascii_lowercase();
    VIRTUAL_WORDS.iter().any(|w| n.contains(w) || d.contains(w))
}

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

/// The physical Ethernet adapters: ARPHRD_ETHER, backed by a device (not a
/// bridge, veth or tunnel), not Wi-Fi.
#[cfg(target_os = "linux")]
pub fn wired(lan: Option<Ipv4Addr>) -> Vec<Wired> {
    let lan_if = lan.and_then(iface_of);
    let Ok(dir) = std::fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    let read = |p: std::path::PathBuf| {
        std::fs::read_to_string(p)
            .ok()
            .map(|s| s.trim().to_string())
    };
    let mut out: Vec<Wired> = dir
        .flatten()
        .take(256)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let sys = e.path();
            let ether = read(sys.join("type")).as_deref() == Some("1");
            let wifi = sys.join("wireless").exists() || sys.join("phy80211").exists();
            if !ether || wifi || !sys.join("device").exists() {
                return None;
            }
            Some(Wired {
                index: read(sys.join("ifindex"))
                    .and_then(|i| i.parse().ok())
                    .unwrap_or(0),
                mac: read(sys.join("address"))
                    .and_then(|m| super::mac::parse_mac(&m))
                    .filter(|m| *m != [0; 6]),
                // Reading carrier fails while the adapter is down.
                up: read(sys.join("carrier")).as_deref() == Some("1"),
                lan: lan_if.as_deref() == Some(name.as_str()),
                dev: name.clone(),
                name: super::sanitize(&name),
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
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
    let Some(path) = std::fs::read_to_string(format!("/sys/class/net/{name}/ifindex"))
        .ok()
        .map(|i| format!("/run/systemd/netif/leases/{}", i.trim()))
    else {
        return NetConf::unknown();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return NetConf::unknown();
    };
    parse_networkd_lease(&text)
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
    let mut lease_time: Option<i64> = None;
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
                    // Unix seconds, and the lease's length.
                    "expiry" => conf.lease_expires = val.parse().ok(),
                    "dhcp_lease_time" => lease_time = val.parse().ok(),
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
    conf.lease_obtained = conf
        .lease_expires
        .zip(lease_time)
        .and_then(|(e, l)| e.checked_sub(l));
    Some(conf)
}

/// A systemd-networkd lease file (`/run/systemd/netif/leases/<ifindex>`):
/// KEY=value lines. Its lease times stay unknown: the file holds only the
/// lease's length, and networkd rewrites it on other link changes too, so
/// the file's own time isn't when the lease was obtained.
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
        GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST,
        GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH, IP_ADAPTER_DHCP_ENABLED,
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
        let mut f = Some(f);
        let mut out = None;
        each_adapter(|ad| {
            if out.is_none() && holds(ad, ip) {
                out = f.take().map(|f| f(ad));
            }
        });
        out
    }

    /// Whether the adapter has `ip`.
    fn holds(ad: &IP_ADAPTER_ADDRESSES_LH, ip: Ipv4Addr) -> bool {
        let mut u = ad.FirstUnicastAddress;
        while !u.is_null() {
            let ua = unsafe { &*u };
            if unsafe { addr(&ua.Address) } == Some(IpAddr::V4(ip)) {
                return true;
            }
            u = ua.Next;
        }
        false
    }

    /// The physical Ethernet adapters (IF_TYPE_ETHERNET_CSMACD), with
    /// whether each has a link (IfOperStatusUp).
    pub fn wired(lan: Option<Ipv4Addr>) -> Vec<super::Wired> {
        let mut out = Vec::new();
        each_adapter(|ad| {
            if ad.IfType != 6 || out.len() >= 64 {
                return;
            }
            let name = unsafe { wide(ad.FriendlyName) };
            let desc = unsafe { wide(ad.Description) };
            // Hyper-V's and WSL's vEthernet, VMware's VMnet and VirtualBox's
            // host-only adapters are Ethernet too, and always up: listening
            // there hears nothing and blames the switch. The system's own
            // word first (a hardware interface), the name when it won't say.
            let hardware =
                hardware_interface(ad).unwrap_or_else(|| !super::virtual_like(&name, &desc));
            if !hardware || super::vpn_like(&name, &desc) {
                return;
            }
            let mac = (ad.PhysicalAddressLength == 6)
                .then(|| {
                    let mut m = [0u8; 6];
                    m.copy_from_slice(&ad.PhysicalAddress[..6]);
                    m
                })
                .filter(|m| *m != [0; 6]);
            out.push(super::Wired {
                name: crate::net::sanitize(&name),
                dev: unsafe { narrow(ad.AdapterName) },
                index: 0,
                mac,
                up: ad.OperStatus == 1,
                lan: lan.is_some_and(|ip| holds(ad, ip)),
            });
        });
        out
    }

    /// Every adapter that's up with an IPv4 default gateway.
    pub fn gateways() -> Vec<super::Route> {
        let mut out: Vec<super::Route> = Vec::new();
        each_adapter(|ad| {
            if ad.OperStatus != 1 || out.len() >= 64 {
                return;
            }
            let mut g = ad.FirstGatewayAddress;
            let mut gateway = None;
            while !g.is_null() && gateway.is_none() {
                let ga = unsafe { &*g };
                if let Some(IpAddr::V4(a)) = unsafe { addr(&ga.Address) }
                    && !a.is_unspecified()
                {
                    gateway = Some(a);
                }
                g = ga.Next;
            }
            let Some(gateway) = gateway else { return };
            let name = unsafe { wide(ad.FriendlyName) };
            let desc = unsafe { wide(ad.Description) };
            out.push(super::Route {
                wifi: ad.IfType == WIFI,
                vpn: VIRTUAL_TYPES.contains(&ad.IfType) || super::vpn_like(&name, &desc),
                name: crate::net::sanitize(&name),
                gateway,
            });
        });
        out
    }

    /// Whether Windows calls the adapter a hardware interface (MIB_IF_ROW2's
    /// HardwareInterface, bit 0 of its flags); None if it can't be read.
    fn hardware_interface(ad: &IP_ADAPTER_ADDRESSES_LH) -> Option<bool> {
        use windows_sys::Win32::NetworkManagement::IpHelper::{GetIfEntry2, MIB_IF_ROW2};
        let mut row: MIB_IF_ROW2 = unsafe { std::mem::zeroed() };
        row.InterfaceLuid = ad.Luid;
        (unsafe { GetIfEntry2(&mut row) } == 0)
            .then_some(row.InterfaceAndOperStatusFlags._bitfield & 0x01 != 0)
    }

    /// Call `f` on every adapter.
    fn each_adapter(mut f: impl FnMut(&IP_ADAPTER_ADDRESSES_LH)) {
        // Gateways too, for `gateways`: the rest don't read them.
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_INCLUDE_GATEWAYS;
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
                _ => return,
            }
        }
        let mut a = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while !a.is_null() {
            let ad = unsafe { &*a };
            f(ad);
            a = ad.Next;
        }
    }

    /// When the adapter named `guid` got its DHCP lease and when it runs
    /// out: GetAdaptersAddresses doesn't keep these, GetAdaptersInfo does
    /// (Unix seconds, 0 for none).
    fn lease_times(guid: &str) -> (Option<i64>, Option<i64>) {
        use windows_sys::Win32::NetworkManagement::IpHelper::{GetAdaptersInfo, IP_ADAPTER_INFO};
        let mut len: u32 = 0;
        unsafe { GetAdaptersInfo(std::ptr::null_mut(), &mut len) };
        if len == 0 || guid.is_empty() {
            return (None, None);
        }
        // u64s keep the buffer aligned for IP_ADAPTER_INFO.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let first = buf.as_mut_ptr().cast::<IP_ADAPTER_INFO>();
        if unsafe { GetAdaptersInfo(first, &mut len) } != 0 {
            return (None, None);
        }
        let mut a = first.cast_const();
        while !a.is_null() {
            let info = unsafe { &*a };
            let raw = &info.AdapterName;
            let n = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            // CHARs: i8 on Windows.
            if raw[..n].iter().map(|&b| b as u8).eq(guid.bytes()) {
                let t = |v: i64| (v > 0).then_some(v);
                return (t(info.LeaseObtained), t(info.LeaseExpires));
            }
            a = info.Next;
        }
        (None, None)
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
            let (lease_obtained, lease_expires) = if matches!(dhcp, Dhcp::Server(_)) {
                lease_times(&unsafe { narrow(ad.AdapterName) })
            } else {
                (None, None)
            };
            NetConf {
                dhcp,
                dns,
                suffix: Some(crate::net::sanitize(suffix.trim())).filter(|s| !s.is_empty()),
                lease_obtained,
                lease_expires,
            }
        })
        .unwrap_or_else(NetConf::unknown)
    }
}

#[cfg(windows)]
pub use win::{adapter_to, gateways, read, wired};

#[cfg(not(any(windows, target_os = "linux")))]
pub fn adapter_to(_: Ipv4Addr) -> Option<Adapter> {
    None
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn wired(_: Option<Ipv4Addr>) -> Vec<Wired> {
    Vec::new()
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn gateways() -> Vec<Route> {
    Vec::new()
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn read(_: Ipv4Addr) -> NetConf {
    NetConf::unknown()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_routes_are_read_per_interface() {
        let table = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
eth0\t00000000\t010200C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
eth0\t000200C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n\
wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
eth0\t00000000\t020200C0\t0003\t0\t0\t200\t00000000\t0\t0\t0\n";
        assert_eq!(
            parse_default_routes(table),
            vec![
                ("eth0".to_string(), "192.0.2.1".parse().unwrap()),
                ("wlan0".to_string(), "192.168.1.1".parse().unwrap()),
            ]
        );
    }

    #[test]
    fn networkmanager_says_where_the_lease_came_from() {
        let out = "GENERAL.STATE:100 (connected)\n\
                   IP4.DNS[1]:192.0.2.10\n\
                   IP4.DNS[2]:192.0.2.11\n\
                   IP4.DOMAIN[1]:corp.example.com\n\
                   DHCP4.OPTION[1]:broadcast_address = 192.0.2.255\n\
                   DHCP4.OPTION[2]:dhcp_server_identifier = 192.0.2.10\n\
                   DHCP4.OPTION[3]:domain_name = corp.example.com\n\
                   DHCP4.OPTION[4]:dhcp_lease_time = 86400\n\
                   DHCP4.OPTION[5]:expiry = 1795000000\n";
        let c = parse_nmcli(out).unwrap();
        assert_eq!(c.lease_expires, Some(1_795_000_000));
        assert_eq!(c.lease_obtained, Some(1_795_000_000 - 86_400));
        assert_eq!(c.dhcp, Dhcp::Server("192.0.2.10".parse().unwrap()));
        assert_eq!(c.dns_text(), "192.0.2.10, 192.0.2.11");
        assert_eq!(c.suffix_text(), "corp.example.com");
        // A server other than the gateway is normal (a domain controller):
        // nothing is judged from it.
        // Set by hand: DNS, no lease.
        let c = parse_nmcli("GENERAL.STATE:100 (connected)\nIP4.DNS[1]:1.1.1.1\n").unwrap();
        assert_eq!(c.dhcp, Dhcp::Static);
        assert_eq!(c.lease_text(c.lease_obtained), "none");
        assert_eq!(c.suffix_text(), "none");
        // Not NetworkManager's: the lease file is asked instead.
        assert_eq!(parse_nmcli("GENERAL.STATE:10 (unmanaged)\n"), None);
        assert_eq!(parse_nmcli(""), None);
    }

    #[test]
    fn a_networkd_lease_names_the_server() {
        let lease = "# This is private data. Do not parse.\nADDRESS=192.0.2.50\n\
                     ROUTER=192.0.2.1\nSERVER_ADDRESS=192.0.2.2\nDNS=192.0.2.2 192.0.2.3\n\
                     DOMAINNAME=office.example.net\nLIFETIME=3600\n";
        let c = parse_networkd_lease(lease);
        // Only the lease's length is there: when it was obtained isn't.
        assert_eq!(c.lease_text(c.lease_obtained), "unknown");
        assert_eq!(c.lease_text(c.lease_expires), "unknown");
        assert_eq!(c.dhcp_text(), "192.0.2.2");
        assert_eq!(c.dns.len(), 2);
        assert_eq!(c.suffix.as_deref(), Some("office.example.net"));
        let none = NetConf::unknown();
        assert_eq!(
            (none.dhcp_text(), none.dns_text(), none.suffix_text()),
            ("unknown".into(), "unknown".into(), "unknown".into())
        );
        assert_eq!(none.lease_text(none.lease_expires), "unknown");
    }

    #[test]
    fn hostile_netconf_text_never_panics() {
        crate::hostile::each_text(
            "GENERAL.STATE:100 (connected)\nIP4.DNS[1]:192.0.2.1\n\
             DHCP4.OPTION[2]:dhcp_server_identifier = 192.0.2.1\n\
             DHCP4.OPTION[3]:expiry = 1795000000\nDHCP4.OPTION[4]:dhcp_lease_time = 60\n",
            |t| {
                let _ = parse_nmcli(t);
            },
        );
        crate::hostile::each_text(
            "SERVER_ADDRESS=192.0.2.1\nDNS=192.0.2.1\nLIFETIME=60\n",
            |t| {
                let _ = parse_networkd_lease(t);
            },
        );
    }

    #[test]
    fn virtual_ethernet_adapters_are_known_by_name() {
        for (name, desc) in [
            ("vEthernet (WSL)", "Hyper-V Virtual Ethernet Adapter"),
            (
                "vEthernet (Default Switch)",
                "Hyper-V Virtual Ethernet Adapter #2",
            ),
            (
                "VMware Network Adapter VMnet8",
                "VMware Virtual Ethernet Adapter for VMnet8",
            ),
            ("Ethernet 3", "VirtualBox Host-Only Ethernet Adapter"),
            ("Npcap Loopback Adapter", "Npcap Loopback Adapter"),
        ] {
            assert!(virtual_like(name, desc), "{name}");
        }
        for (name, desc) in [
            ("Ethernet", "Intel(R) Ethernet Connection (13) I219-LM"),
            ("Ethernet 2", "Realtek USB GbE Family Controller"),
            ("eth0", ""),
        ] {
            assert!(!virtual_like(name, desc), "{name}");
        }
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
