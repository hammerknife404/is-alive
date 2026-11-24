//! A Wi-Fi snapshot, when the way out is over Wi-Fi: network name (SSID),
//! the access point this computer is associated with (BSSID), band and
//! channel, and signal. Read from the system once: it can't show roaming
//! or how the signal changes. Windows: wlanapi (not `netsh`, whose text is
//! translated). Linux: `iw dev <if> link`.

use super::checks::{Item, Status};
use super::netconf::Adapter;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub ssid: Option<String>,
    /// The access point's radio MAC, as `aa:bb:cc:dd:ee:ff`.
    pub bssid: Option<String>,
    pub freq_mhz: Option<u32>,
    /// When the frequency isn't known (Windows without location access).
    pub channel: Option<u32>,
    pub signal_dbm: Option<i32>,
    /// Windows' 0–100 signal quality, where no dBm is given.
    pub quality: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wifi {
    Connected(Snapshot),
    NotConnected,
    /// Windows withheld the SSID and BSSID: location access is off for
    /// desktop apps (Windows 11 24H2 on). What it still gave.
    #[cfg_attr(not(windows), allow(dead_code))]
    Denied(Snapshot),
    Unknown(String),
}

/// Band and channel of a centre frequency in MHz.
pub fn band_channel(mhz: u32) -> Option<(&'static str, u32)> {
    match mhz {
        2484 => Some(("2.4 GHz", 14)),
        2412..=2472 => Some(("2.4 GHz", (mhz - 2407) / 5)),
        5150..=5895 => Some(("5 GHz", (mhz - 5000) / 5)),
        5925..=7125 => Some(("6 GHz", (mhz - 5950) / 5)),
        _ => None,
    }
}

/// How good a signal is, for a tech standing by the laptop.
fn signal_word(dbm: i32) -> (&'static str, Status) {
    if dbm >= -67 {
        ("good", Status::Ok)
    } else if dbm >= -75 {
        (
            "fair: fine for browsing, marginal for calls and video",
            Status::Warn,
        )
    } else {
        ("weak: expect drops and slow transfers here", Status::Warn)
    }
}

/// "5 GHz channel 36, signal -52 dBm (good)", as much as is known.
fn radio(s: &Snapshot) -> (Vec<String>, Status) {
    let mut parts = Vec::new();
    match (s.freq_mhz.and_then(band_channel), s.channel) {
        (Some((band, ch)), _) => parts.push(format!("{band} channel {ch}")),
        (None, Some(ch)) => parts.push(format!("channel {ch}")),
        _ => {}
    }
    let mut status = Status::Ok;
    if let Some(dbm) = s.signal_dbm {
        let (word, st) = signal_word(dbm);
        parts.push(format!("signal {dbm} dBm ({word})"));
        status = st;
    } else if let Some(q) = s.quality {
        // Windows' quality runs linearly from -100 dBm (0) to -50 (100).
        let dbm = q.min(100) as i32 / 2 - 100;
        let (word, st) = signal_word(dbm);
        parts.push(format!("signal quality {q}% (about {dbm} dBm, {word})"));
        status = st;
    }
    (parts, status)
}

/// The Wi-Fi check, from a snapshot of `adapter`.
pub fn judge(w: &Wifi, adapter: &str) -> Item {
    let check = "Wi-Fi";
    let on = if adapter.is_empty() {
        "On Wi-Fi".to_string()
    } else {
        format!("On Wi-Fi ({adapter})")
    };
    match w {
        Wifi::Connected(s) => {
            let mut parts = Vec::new();
            if let Some(n) = &s.ssid {
                parts.push(format!("network \"{n}\""));
            }
            if let Some(b) = &s.bssid {
                parts.push(format!("access point {b}"));
            }
            let (r, status) = radio(s);
            parts.extend(r);
            Item {
                check,
                status,
                detail: format!(
                    "{on}: {}. A snapshot: it can't show roaming or how the signal changes.",
                    parts.join(", ")
                ),
            }
        }
        Wifi::Denied(s) => {
            let (r, _) = radio(s);
            let got = if r.is_empty() {
                String::new()
            } else {
                format!(" It did give {}.", r.join(", "))
            };
            Item {
                check,
                status: Status::Unknown,
                detail: format!(
                    "{on}, but Windows withheld the network's name and access point: since \
                     Windows 11 24H2 that needs location access. Turn on Settings › Privacy & \
                     security › Location › Let desktop apps access your location, then run Full \
                     scan again.{got}"
                ),
            }
        }
        Wifi::NotConnected => Item {
            check,
            status: Status::Unknown,
            detail: format!("{on}, but the adapter says it isn't connected to a network."),
        },
        Wifi::Unknown(e) => Item {
            check,
            status: Status::Unknown,
            detail: format!("{on}, but its details couldn't be read: {e}."),
        },
    }
}

/// `iw dev <if> link`.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_iw_link(text: &str) -> Wifi {
    let mut s = Snapshot::default();
    let mut connected = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("Not connected") {
            return Wifi::NotConnected;
        }
        if let Some(rest) = l.strip_prefix("Connected to ") {
            connected = true;
            s.bssid = rest
                .split_whitespace()
                .next()
                .and_then(super::mac::parse_mac)
                .map(|m| super::mac::fmt(&m));
        } else if let Some(v) = l.strip_prefix("SSID:") {
            let v = super::sanitize(v.trim());
            s.ssid = Some(v).filter(|v| !v.is_empty());
        } else if let Some(v) = l.strip_prefix("freq:") {
            // "5180" or "5180.0".
            s.freq_mhz = v.trim().split('.').next().and_then(|f| f.parse().ok());
        } else if let Some(v) = l.strip_prefix("signal:") {
            s.signal_dbm = v.split_whitespace().next().and_then(|d| d.parse().ok());
        }
    }
    if connected {
        Wifi::Connected(s)
    } else {
        Wifi::Unknown("iw gave no link".into())
    }
}

#[cfg(target_os = "linux")]
pub fn snapshot(adapter: &Adapter) -> Wifi {
    let Some(prog) = super::system_tool("iw") else {
        return Wifi::Unknown("iw isn't installed".into());
    };
    let mut cmd = std::process::Command::new(prog);
    cmd.env("LC_ALL", "C").args(["dev", &adapter.name, "link"]);
    match super::output_within(&mut cmd, std::time::Duration::from_secs(3)) {
        Ok(o) => parse_iw_link(&String::from_utf8_lossy(&o.stdout)),
        Err(e) => Wifi::Unknown(format!("iw: {e}")),
    }
}

#[cfg(windows)]
pub fn snapshot(adapter: &Adapter) -> Wifi {
    win::snapshot(adapter)
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn snapshot(_: &Adapter) -> Wifi {
    Wifi::Unknown("not supported on this system".into())
}

#[cfg(windows)]
mod win {
    use super::{Snapshot, Wifi};
    use crate::net::netconf::Adapter;
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::NetworkManagement::WiFi::{
        WLAN_BSS_LIST, WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST,
        WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanGetNetworkBssList, WlanOpenHandle,
        WlanQueryInterface, dot11_BSS_type_infrastructure, wlan_interface_state_connected,
        wlan_intf_opcode_channel_number, wlan_intf_opcode_current_connection,
        wlan_intf_opcode_rssi,
    };
    use windows_sys::core::GUID;

    const ACCESS_DENIED: u32 = 5;

    fn guid_text(g: &GUID) -> String {
        let d = g.data4;
        format!(
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            g.data1, g.data2, g.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
        )
    }

    /// One WlanQueryInterface: Ok(data, freed by the caller) or the error.
    unsafe fn query(h: HANDLE, g: &GUID, op: i32) -> Result<*mut c_void, u32> {
        let mut size = 0u32;
        let mut data: *mut c_void = null_mut();
        let r = unsafe { WlanQueryInterface(h, g, op, null(), &mut size, &mut data, null_mut()) };
        if r != 0 || data.is_null() {
            return Err(r);
        }
        Ok(data)
    }

    unsafe fn read_u32(h: HANDLE, g: &GUID, op: i32) -> Option<u32> {
        unsafe {
            let p = query(h, g, op).ok()?;
            let v = std::ptr::read_unaligned(p.cast::<u32>());
            WlanFreeMemory(p);
            Some(v)
        }
    }

    pub fn snapshot(adapter: &Adapter) -> Wifi {
        let mut ver = 0u32;
        let mut h: HANDLE = null_mut();
        if unsafe { WlanOpenHandle(2, null(), &mut ver, &mut h) } != 0 {
            return Wifi::Unknown("the WLAN service didn't answer".into());
        }
        let w = unsafe { snapshot_with(h, adapter) };
        unsafe { WlanCloseHandle(h, null()) };
        w
    }

    unsafe fn snapshot_with(h: HANDLE, adapter: &Adapter) -> Wifi {
        unsafe {
            let mut list: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
            if WlanEnumInterfaces(h, null(), &mut list) != 0 || list.is_null() {
                return Wifi::Unknown("no Wi-Fi adapter listed".into());
            }
            let n = (*list).dwNumberOfItems as usize;
            let first = std::ptr::addr_of!((*list).InterfaceInfo).cast::<WLAN_INTERFACE_INFO>();
            let infos = std::slice::from_raw_parts(first, n);
            let pick = infos
                .iter()
                .find(|i| guid_text(&i.InterfaceGuid).eq_ignore_ascii_case(&adapter.guid))
                .or(if n == 1 { infos.first() } else { None })
                .map(|i| i.InterfaceGuid);
            WlanFreeMemory(list.cast());
            let Some(g) = pick else {
                return Wifi::Unknown("its Wi-Fi interface wasn't found".into());
            };

            let mut s = Snapshot {
                channel: read_u32(h, &g, wlan_intf_opcode_channel_number),
                signal_dbm: read_u32(h, &g, wlan_intf_opcode_rssi).map(|v| v as i32),
                ..Default::default()
            };
            let conn = match query(h, &g, wlan_intf_opcode_current_connection) {
                Ok(p) => {
                    let c = std::ptr::read_unaligned(p.cast::<WLAN_CONNECTION_ATTRIBUTES>());
                    WlanFreeMemory(p);
                    c
                }
                Err(ACCESS_DENIED) => return Wifi::Denied(s),
                Err(e) => return Wifi::Unknown(format!("WlanQueryInterface error {e}")),
            };
            if conn.isState != wlan_interface_state_connected {
                return Wifi::NotConnected;
            }
            let a = &conn.wlanAssociationAttributes;
            let len = (a.dot11Ssid.uSSIDLength as usize).min(32);
            let ssid = crate::net::sanitize(&String::from_utf8_lossy(&a.dot11Ssid.ucSSID[..len]));
            s.ssid = Some(ssid).filter(|v| !v.is_empty());
            let bssid = a.dot11Bssid;
            s.bssid = (bssid != [0; 6]).then(|| crate::net::mac::fmt(&bssid));
            s.quality = Some(a.wlanSignalQuality);
            // The associated access point's own entry: its frequency and RSSI.
            let mut bss: *mut WLAN_BSS_LIST = null_mut();
            let secure = conn.wlanSecurityAttributes.bSecurityEnabled;
            if WlanGetNetworkBssList(
                h,
                &g,
                &a.dot11Ssid,
                dot11_BSS_type_infrastructure,
                secure,
                null(),
                &mut bss,
            ) == 0
                && !bss.is_null()
            {
                let n = (*bss).dwNumberOfItems as usize;
                let first = std::ptr::addr_of!((*bss).wlanBssEntries)
                    .cast::<windows_sys::Win32::NetworkManagement::WiFi::WLAN_BSS_ENTRY>();
                if let Some(e) = std::slice::from_raw_parts(first, n)
                    .iter()
                    .find(|e| e.dot11Bssid == bssid)
                {
                    s.freq_mhz = Some(e.ulChCenterFrequency / 1000);
                    s.signal_dbm = Some(e.lRssi);
                }
                WlanFreeMemory(bss.cast());
            }
            Wifi::Connected(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iw_link_reads_as_a_snapshot() {
        let out = "Connected to 02:00:5e:10:00:01 (on wlan0)\n\
                   \tSSID: Example Guest\n\
                   \tfreq: 5180.0\n\
                   \tRX: 1234 bytes (10 packets)\n\
                   \tsignal: -52 dBm\n\
                   \trx bitrate: 866.7 MBit/s VHT-MCS 9 80MHz short GI VHT-NSS 2\n";
        let Wifi::Connected(s) = parse_iw_link(out) else {
            panic!("connected")
        };
        assert_eq!(s.ssid.as_deref(), Some("Example Guest"));
        assert_eq!(s.bssid.as_deref(), Some("02:00:5e:10:00:01"));
        assert_eq!(s.freq_mhz, Some(5180));
        assert_eq!(s.signal_dbm, Some(-52));
        let r = judge(&Wifi::Connected(s), "wlan0");
        assert_eq!(r.status, Status::Ok);
        assert!(
            r.detail.contains("5 GHz channel 36") && r.detail.contains("-52 dBm (good)"),
            "{}",
            r.detail
        );
        // A snapshot, and it says so: no claim about roaming.
        assert!(r.detail.contains("can't show roaming"));
        assert_eq!(parse_iw_link("Not connected.\n"), Wifi::NotConnected);
    }

    #[test]
    fn bands_and_channels_from_the_frequency() {
        assert_eq!(band_channel(2412), Some(("2.4 GHz", 1)));
        assert_eq!(band_channel(2437), Some(("2.4 GHz", 6)));
        assert_eq!(band_channel(2484), Some(("2.4 GHz", 14)));
        assert_eq!(band_channel(5745), Some(("5 GHz", 149)));
        assert_eq!(band_channel(5955), Some(("6 GHz", 1)));
        assert_eq!(band_channel(900), None);
    }

    #[test]
    fn weak_signal_warns_and_location_off_says_how_to_fix_it() {
        let weak = Snapshot {
            signal_dbm: Some(-79),
            freq_mhz: Some(2437),
            ..Default::default()
        };
        let r = judge(&Wifi::Connected(weak), "Wi-Fi");
        assert_eq!(r.status, Status::Warn);
        assert!(r.detail.contains("weak") && r.detail.contains("2.4 GHz channel 6"));
        // Windows' quality, where no dBm came: 60% is about -70 dBm.
        let q = Snapshot {
            quality: Some(60),
            ..Default::default()
        };
        let r = judge(&Wifi::Connected(q), "Wi-Fi");
        assert_eq!(r.status, Status::Warn);
        assert!(r.detail.contains("about -70 dBm"), "{}", r.detail);
        let denied = Snapshot {
            channel: Some(36),
            ..Default::default()
        };
        let r = judge(&Wifi::Denied(denied), "Wi-Fi");
        assert_eq!(r.status, Status::Unknown);
        assert!(r.detail.contains("Let desktop apps access your location"));
        assert!(r.detail.contains("channel 36"));
    }

    #[test]
    fn hostile_iw_output_never_panics() {
        crate::hostile::each_text(
            "Connected to 02:00:5e:10:00:01 (on wlan0)\n\tSSID: x\n\tfreq: 2412\n\tsignal: -40 dBm\n",
            |t| {
                let _ = parse_iw_link(t);
            },
        );
    }
}
