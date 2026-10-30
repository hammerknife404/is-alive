//! MAC addresses: this machine's neighbor (ARP) table and the IEEE vendor
//! registry. Reading the table needs no admin rights; it only holds hosts on
//! the local segment, so anything past a router has no MAC here.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::OnceLock;

pub type Mac = [u8; 6];

pub fn fmt(m: &Mac) -> String {
    m.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Locally administered: a phone or laptop's private per-network address, or
/// a virtual NIC. Not in the registry, so there's no vendor to name.
pub fn local_admin(m: &Mac) -> bool {
    m[0] & 0x02 != 0
}

/// Unicast and not all zeros (an incomplete entry).
fn usable(m: &Mac) -> bool {
    m[0] & 0x01 == 0 && m.iter().any(|&b| b != 0)
}

/// IEEE MA-L, MA-M and MA-S registries, rebuilt by scripts/update-oui.py.
static OUI_GZ: &[u8] = include_bytes!("../../assets/oui.txt.gz");

/// Sorted (prefix, vendor) tables for 9-, 7- and 6-hex-digit prefixes;
/// longest match wins.
type Registry = [(usize, Vec<(u64, String)>); 3];

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| {
        let mut text = String::new();
        use std::io::Read;
        let _ = flate2::read::GzDecoder::new(OUI_GZ).read_to_string(&mut text);
        let mut r: Registry = [(9, Vec::new()), (7, Vec::new()), (6, Vec::new())];
        for line in text.lines() {
            let Some((p, v)) = line.split_once('\t') else {
                continue;
            };
            let Ok(key) = u64::from_str_radix(p, 16) else {
                continue;
            };
            if let Some(t) = r.iter_mut().find(|t| t.0 == p.len()) {
                t.1.push((key, v.to_string()));
            }
        }
        for t in &mut r {
            t.1.sort_unstable_by_key(|e| e.0);
        }
        r
    })
}

pub fn vendor(m: &Mac) -> Option<&'static str> {
    let v = m.iter().fold(0u64, |a, &b| a << 8 | b as u64);
    registry().iter().find_map(|(digits, t)| {
        let key = v >> (48 - digits * 4);
        t.binary_search_by_key(&key, |e| e.0)
            .ok()
            .map(|i| t[i].1.as_str())
    })
}

/// Vendor, or why there isn't one.
pub fn vendor_label(m: &Mac) -> String {
    match vendor(m) {
        Some(v) => v.to_string(),
        None if local_admin(m) => "(private/virtual MAC)".into(),
        None => String::new(),
    }
}

/// IPv4 → MAC for every resolved entry in this machine's neighbor table.
pub fn neighbors() -> HashMap<Ipv4Addr, Mac> {
    imp::neighbors()
        .into_iter()
        .filter(|(_, m)| usable(m))
        .collect()
}

/// Entries the OS has just confirmed (the device answered an ARP request in
/// the last few tens of seconds), not leftovers from devices that have
/// gone: `REACHABLE` in Linux's `ip neigh`, Reachable on Windows. Empty
/// where the state isn't available (no `ip` command, macOS).
pub fn confirmed() -> HashMap<Ipv4Addr, Mac> {
    imp::confirmed()
        .into_iter()
        .filter(|(_, m)| usable(m))
        .collect()
}

/// `ip -4 neigh show`: "10.0.0.5 dev eth0 lladdr aa:bb:cc:dd:ee:ff REACHABLE".
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_ip_neigh(text: &str) -> Vec<(Ipv4Addr, Mac)> {
    text.lines()
        .filter(|l| l.split_whitespace().last() == Some("REACHABLE"))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let ip = f.first()?.parse().ok()?;
            let at = f.iter().position(|w| *w == "lladdr")?;
            Some((ip, parse_mac(f.get(at + 1)?)?))
        })
        .collect()
}

/// "aa:bb:cc:dd:ee:ff", "aa-bb-…" or macOS's unpadded "0:1b:…".
pub fn parse_mac(s: &str) -> Option<Mac> {
    let parts: Vec<&str> = s.split([':', '-']).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut m = [0u8; 6];
    for (b, p) in m.iter_mut().zip(parts) {
        if p.is_empty() || p.len() > 2 {
            return None;
        }
        *b = u8::from_str_radix(p, 16).ok()?;
    }
    Some(m)
}

/// /proc/net/arp: "IP  HW type  Flags  HW address  Mask  Device";
/// flag 0x2 marks a completed entry.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_proc_arp(text: &str) -> Vec<(Ipv4Addr, Mac)> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let flags = u32::from_str_radix(f.get(2)?.trim_start_matches("0x"), 16).ok()?;
            (flags & 0x2 != 0).then_some(())?;
            Some((f.first()?.parse().ok()?, parse_mac(f.get(3)?)?))
        })
        .collect()
}

/// `arp -an`: "? (192.168.1.1) at 0:1b:2c:3d:4e:5f on en0 ifscope [ethernet]".
#[cfg_attr(any(target_os = "linux", windows), allow(dead_code))]
fn parse_arp_an(text: &str) -> Vec<(Ipv4Addr, Mac)> {
    text.lines()
        .filter_map(|l| {
            let ip = l.split_once('(')?.1.split_once(')')?.0.parse().ok()?;
            let mac = l.split_once(" at ")?.1.split_whitespace().next()?;
            Some((ip, parse_mac(mac)?))
        })
        .collect()
}

#[cfg(target_os = "linux")]
mod imp {
    pub fn neighbors() -> Vec<(std::net::Ipv4Addr, super::Mac)> {
        std::fs::read_to_string("/proc/net/arp")
            .map(|t| super::parse_proc_arp(&t))
            .unwrap_or_default()
    }

    /// /proc/net/arp has no state, so ask iproute2.
    pub fn confirmed() -> Vec<(std::net::Ipv4Addr, super::Mac)> {
        std::process::Command::new("ip")
            .args(["-4", "neigh", "show"])
            .output()
            .map(|o| super::parse_ip_neigh(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default()
    }
}

#[cfg(windows)]
mod imp {
    use std::net::Ipv4Addr;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        FreeMibTable, GetIpNetTable2, MIB_IPNET_TABLE2,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, NlnsProbe, NlnsReachable};

    pub fn neighbors() -> Vec<(Ipv4Addr, super::Mac)> {
        table(NlnsProbe)
    }

    pub fn confirmed() -> Vec<(Ipv4Addr, super::Mac)> {
        table(NlnsReachable)
    }

    /// Entries whose state is at least `min` (Probe and up carry an address;
    /// Reachable means an ARP reply arrived recently).
    fn table(min: i32) -> Vec<(Ipv4Addr, super::Mac)> {
        let mut out = Vec::new();
        let mut table: *mut MIB_IPNET_TABLE2 = std::ptr::null_mut();
        unsafe {
            if GetIpNetTable2(AF_INET, &mut table) != 0 || table.is_null() {
                return out;
            }
            let rows =
                std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
            for r in rows {
                // Unreachable and Incomplete entries carry no address yet.
                if r.PhysicalAddressLength != 6 || r.State < min {
                    continue;
                }
                let ip = Ipv4Addr::from(r.Address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes());
                let mut m = [0u8; 6];
                m.copy_from_slice(&r.PhysicalAddress[..6]);
                out.push((ip, m));
            }
            FreeMibTable(table.cast());
        }
        out
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    pub fn confirmed() -> Vec<(std::net::Ipv4Addr, super::Mac)> {
        Vec::new()
    }

    pub fn neighbors() -> Vec<(std::net::Ipv4Addr, super::Mac)> {
        std::process::Command::new("arp")
            .arg("-an")
            .output()
            .map(|o| super::parse_arp_an(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_lookup_longest_prefix() {
        let ubnt = [0x24, 0x5a, 0x4c, 1, 2, 3];
        assert_eq!(vendor(&ubnt), Some("Ubiquiti"));
        assert_eq!(vendor(&[0x00, 0x17, 0x9a, 0, 0, 1]), Some("D-Link"));
        // Every MA-M/MA-S prefix must win over a shorter one that covers it.
        let r = registry();
        let (digits, t) = (&r[1].0, &r[1].1);
        let (key, name) = &t[t.len() / 2];
        let mac_bits = key << (48 - digits * 4);
        let m: Mac = std::array::from_fn(|i| (mac_bits >> (40 - 8 * i)) as u8);
        assert_eq!(vendor(&m), Some(name.as_str()));
        assert!(r.iter().all(|(_, t)| t.len() > 1000));
    }

    #[test]
    fn local_admin_macs() {
        let m = [0x02 | 0xa0, 1, 2, 3, 4, 5];
        assert!(local_admin(&m));
        assert_eq!(vendor_label(&m), "(private/virtual MAC)");
        assert!(!local_admin(&[0x24, 0x5a, 0x4c, 1, 2, 3]));
    }

    #[test]
    fn parses_proc_net_arp() {
        let t = "IP address       HW type     Flags       HW address            Mask     Device\n\
                 192.168.1.1      0x1         0x2         24:5a:4c:aa:bb:cc     *        eth0\n\
                 192.168.1.9      0x1         0x0         00:00:00:00:00:00     *        eth0\n";
        let v = parse_proc_arp(t);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].0, Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(fmt(&v[0].1), "24:5a:4c:aa:bb:cc");
    }

    #[test]
    fn parses_ip_neigh_reachable_only() {
        let t = "10.0.0.1 dev wlp2s0 lladdr 9c:c9:eb:e6:c7:ec router REACHABLE\n\
                 10.0.0.57 dev wlp2s0 lladdr 3c:2a:f4:01:02:03 REACHABLE\n\
                 10.0.0.60 dev wlp2s0 lladdr 3c:2a:f4:01:02:04 STALE\n\
                 10.0.0.61 dev wlp2s0 FAILED\n\
                 10.0.0.62 dev wlp2s0 lladdr 3c:2a:f4:01:02:05 DELAY\n";
        let v = parse_ip_neigh(t);
        assert_eq!(
            v.iter().map(|e| e.0.to_string()).collect::<Vec<_>>(),
            ["10.0.0.1", "10.0.0.57"]
        );
        assert_eq!(fmt(&v[1].1), "3c:2a:f4:01:02:03");
    }

    #[test]
    fn parses_arp_an() {
        let t = "? (192.168.1.1) at 0:1b:2c:3d:4e:5f on en0 ifscope [ethernet]\n\
                 ? (192.168.1.7) at (incomplete) on en0 ifscope [ethernet]\n";
        let v = parse_arp_an(t);
        assert_eq!(
            v,
            [(
                Ipv4Addr::new(192, 168, 1, 1),
                [0, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f]
            )]
        );
    }

    #[test]
    fn filters_unusable() {
        assert!(!usable(&[0; 6]));
        assert!(!usable(&[0xff; 6]));
        assert!(!usable(&[0x01, 0x00, 0x5e, 0, 0, 1]));
        assert!(usable(&[0x24, 0x5a, 0x4c, 1, 2, 3]));
    }
}
