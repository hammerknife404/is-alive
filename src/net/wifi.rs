//! A Wi-Fi snapshot, when the way out is over Wi-Fi: network name (SSID),
//! the access point this computer is associated with (BSSID), band,
//! channel and its width, signal and link rates. Read from the system
//! once: it can't show roaming or how the signal changes. Windows: wlanapi
//! (not `netsh`, whose text is translated). Linux: `iw dev <if> link` and
//! `iw dev <if> info`.
//!
//! And the access points nearby (SSID, BSSID, channel, signal), which show
//! co-channel interference. Full scan only reads the list the system
//! already has; an active scan (probe requests, the radio hopping
//! channels, the connection pausing) runs only from Scan Wi-Fi, never
//! alongside Full scan's probes. Windows: WlanGetNetworkBssList (WlanScan
//! first for a scan). Linux: `nmcli dev wifi list --rescan no` (`yes` for a
//! scan).

use super::checks::{Item, Status};
#[cfg(not(windows))]
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
    /// Channel width in MHz: 20, 40, 80, 160 or 320. On Windows, the width
    /// the access point says it operates at.
    pub width_mhz: Option<u32>,
    /// Link rates in Mbit/s: what this computer sends (Tx) and receives
    /// (Rx) at.
    pub tx_mbps: Option<f64>,
    pub rx_mbps: Option<f64>,
}

/// One access point the system has heard.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bss {
    /// None for a hidden network.
    pub ssid: Option<String>,
    /// Its radio MAC, as `aa:bb:cc:dd:ee:ff`.
    pub bssid: String,
    pub freq_mhz: Option<u32>,
    pub channel: Option<u32>,
    pub signal_dbm: Option<i32>,
    /// NetworkManager's 0–100 signal strength, where no dBm is given.
    pub signal_pct: Option<u32>,
}

/// The access points nearby, as the system lists them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Nearby {
    pub aps: Vec<Bss>,
    /// Seconds since the newest entry was heard, when the system says.
    pub age_secs: Option<u64>,
    /// Fresh from a scan is-alive asked for (Scan Wi-Fi), not the list the
    /// system already had.
    pub scanned: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Neighbours {
    Listed(Nearby),
    /// Windows withheld the list: location access is off (24H2 on).
    #[cfg_attr(not(windows), allow(dead_code))]
    Denied,
    Unknown(String),
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
        // 6 GHz: channel 2 sits alone at 5935; the rest count up from 5950.
        5935 => Some(("6 GHz", 2)),
        5955..=7115 => Some(("6 GHz", (mhz - 5950) / 5)),
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

/// A rate in Mbit/s: whole where it is, else one decimal.
fn rate(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// "5 GHz channel 36" or "channel 36", from what's known.
fn where_on(freq_mhz: Option<u32>, channel: Option<u32>) -> Option<String> {
    match (freq_mhz.and_then(band_channel), channel) {
        (Some((band, ch)), _) => Some(format!("{band} channel {ch}")),
        (None, Some(ch)) => Some(format!("channel {ch}")),
        _ => None,
    }
}

/// Band (when known) and channel, to compare two radios by.
fn channel_key(freq_mhz: Option<u32>, channel: Option<u32>) -> Option<(Option<&'static str>, u32)> {
    match (freq_mhz.and_then(band_channel), channel) {
        (Some((band, ch)), _) => Some((Some(band), ch)),
        (None, Some(ch)) => Some((None, ch)),
        _ => None,
    }
}

/// "5 GHz channel 36 (80 MHz wide), signal -52 dBm (good), link Tx 866.7 /
/// Rx 780 Mbit/s", as much as is known.
fn radio(s: &Snapshot) -> (Vec<String>, Status) {
    let mut parts = Vec::new();
    if let Some(w) = where_on(s.freq_mhz, s.channel) {
        match s.width_mhz {
            Some(mhz) => parts.push(format!("{w} ({mhz} MHz wide)")),
            None => parts.push(w),
        }
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
    match (s.tx_mbps, s.rx_mbps) {
        (Some(t), Some(r)) => parts.push(format!("link Tx {} / Rx {} Mbit/s", rate(t), rate(r))),
        (Some(t), None) => parts.push(format!("link Tx {} Mbit/s", rate(t))),
        (None, Some(r)) => parts.push(format!("link Rx {} Mbit/s", rate(r))),
        (None, None) => {}
    }
    (parts, status)
}

/// How many access points a Nearby Wi-Fi line names before "and N more".
const NAMED: usize = 12;

/// `"Example Guest" 02:00:5e:10:00:02 5 GHz channel 36 -61 dBm`.
fn ap_text(b: &Bss) -> String {
    let name = b
        .ssid
        .as_deref()
        .map_or("(hidden)".to_string(), |n| format!("\"{n}\""));
    let mut t = format!("{name} {}", b.bssid);
    if let Some(w) = where_on(b.freq_mhz, b.channel) {
        t += &format!(" {w}");
    }
    if let Some(d) = b.signal_dbm {
        t += &format!(" {d} dBm");
    } else if let Some(p) = b.signal_pct {
        t += &format!(" {p}%");
    }
    t
}

/// Strongest first: dBm where given, else the percentage.
fn by_signal(aps: &mut [&Bss]) {
    aps.sort_by_key(|b| {
        std::cmp::Reverse(
            b.signal_dbm
                .or(b.signal_pct.map(|p| p as i32 / 2 - 100))
                .unwrap_or(i32::MIN),
        )
    });
}

/// The access points nearby, with the ones sharing this connection's
/// channel first: those are its co-channel interference (on 2.4 GHz,
/// channels under 5 apart overlap too). Reported, not judged: how much
/// they get in the way depends on how busy they are, which one list can't
/// show.
pub fn judge_nearby(n: &Neighbours, here: Option<&Snapshot>) -> Item {
    let check = "Nearby Wi-Fi";
    let system = if cfg!(windows) {
        "Windows"
    } else {
        "NetworkManager"
    };
    let n = match n {
        Neighbours::Listed(n) => n,
        Neighbours::Denied => {
            return Item {
                check,
                status: Status::Unknown,
                detail: "Windows withheld the list of nearby networks: since Windows 11 24H2 \
                         that needs location access too (Settings › Privacy & security › \
                         Location › Let desktop apps access your location)."
                    .into(),
            };
        }
        Neighbours::Unknown(e) => {
            return Item {
                check,
                status: Status::Unknown,
                detail: format!("The list of nearby networks couldn't be read: {e}."),
            };
        }
    };
    let from = if n.scanned {
        "a scan just found".to_string()
    } else {
        format!("in the list {system} already had (is-alive didn't scan)")
    };
    let age = match n.age_secs {
        Some(s) if !n.scanned => format!(", the newest heard {s} s ago"),
        None if !n.scanned => "; its age is unknown".to_string(),
        _ => String::new(),
    };
    if n.aps.is_empty() {
        return Item {
            check,
            status: Status::Unknown,
            detail: format!(
                "No access points {from}{age}. Scan Wi-Fi, in this run's result, looks now."
            ),
        };
    }
    let mine = here.and_then(|s| s.bssid.as_deref());
    let key = here.and_then(|s| channel_key(s.freq_mhz, s.channel));
    let mut others: Vec<&Bss> = n
        .aps
        .iter()
        .filter(|b| Some(b.bssid.as_str()) != mine)
        .collect();
    by_signal(&mut others);
    let same = |b: &Bss| match (key, channel_key(b.freq_mhz, b.channel)) {
        (Some((band, ch)), Some((bb, bc))) => {
            ch == bc && (band.is_none() || bb.is_none() || band == bb)
        }
        _ => false,
    };
    // 2.4 GHz channels are 5 MHz apart and 20 wide: under 5 apart overlap.
    let overlaps = |b: &Bss| match (key, channel_key(b.freq_mhz, b.channel)) {
        (Some((Some("2.4 GHz"), ch)), Some((Some("2.4 GHz"), bc))) => {
            ch != bc && ch.abs_diff(bc) < 5
        }
        _ => false,
    };
    let count = n.aps.len();
    let mut parts = vec![format!(
        "{count} access point{} {from}{age}",
        if count == 1 { "" } else { "s" }
    )];
    let mut named = 0;
    let list = |aps: Vec<&&Bss>, named: &mut usize| -> String {
        let shown: Vec<String> = aps
            .iter()
            .take(NAMED.saturating_sub(*named))
            .map(|b| ap_text(b))
            .collect();
        *named += shown.len();
        let more = aps.len() - shown.len();
        let mut t = shown.join("; ");
        if more > 0 {
            if !t.is_empty() {
                t += "; ";
            }
            t += &format!("and {more} more");
        }
        t
    };
    if let Some((band, ch)) = key {
        let on = band.map_or(format!("channel {ch}"), |b| format!("{b} channel {ch}"));
        let co: Vec<&&Bss> = others.iter().filter(|b| same(b)).collect();
        if co.is_empty() {
            parts.push(format!("none other on this connection's channel ({on})"));
        } else {
            parts.push(format!(
                "{} other{} on this connection's channel ({on}), its co-channel interference: {}",
                co.len(),
                if co.len() == 1 { "" } else { "s" },
                list(co, &mut named)
            ));
        }
        let near: Vec<&&Bss> = others.iter().filter(|b| overlaps(b)).collect();
        if !near.is_empty() {
            parts.push(format!(
                "{} on overlapping 2.4 GHz channels: {}",
                near.len(),
                list(near, &mut named)
            ));
        }
        let rest: Vec<&&Bss> = others.iter().filter(|b| !same(b) && !overlaps(b)).collect();
        if !rest.is_empty() {
            parts.push(format!("elsewhere: {}", list(rest, &mut named)));
        }
    } else {
        let all: Vec<&&Bss> = others.iter().collect();
        parts.push(format!("strongest first: {}", list(all, &mut named)));
    }
    Item {
        check,
        status: Status::Ok,
        detail: format!("{}.", parts.join(". ")),
    }
}

/// A Wi-Fi scan's table: one row per access point, strongest first.
pub fn rows(n: &Nearby, here: Option<&Snapshot>) -> Vec<Vec<String>> {
    let mine = here.and_then(|s| s.bssid.as_deref());
    let mut aps: Vec<&Bss> = n.aps.iter().collect();
    by_signal(&mut aps);
    aps.iter()
        .map(|b| {
            let bc = b.freq_mhz.and_then(band_channel);
            vec![
                b.ssid.clone().unwrap_or_default(),
                b.bssid.clone(),
                bc.map(|(band, _)| band.to_string()).unwrap_or_default(),
                crate::log::cell(bc.map(|(_, c)| c).or(b.channel)),
                crate::log::cell(b.signal_dbm),
                crate::log::cell(b.signal_pct),
                if Some(b.bssid.as_str()) == mine {
                    "yes".into()
                } else {
                    String::new()
                },
            ]
        })
        .collect()
}

/// The channel width an access point operates at, from its beacon's
/// information elements: HT (40), VHT (80, 160), and on 6 GHz HE and EHT
/// (up to 320). The widest one says.
#[cfg(any(windows, test))]
pub(crate) fn width_from_ies(ies: &[u8]) -> Option<u32> {
    // The 6 GHz part of an HE Operation element.
    fn he(d: &[u8]) -> Option<u32> {
        let p = u32::from(*d.first()?) | u32::from(*d.get(1)?) << 8 | u32::from(*d.get(2)?) << 16;
        if p & (1 << 17) == 0 {
            return None;
        }
        // Parameters, BSS colour, basic MCS set; then the optional parts.
        let mut at = 6;
        if p & (1 << 14) != 0 {
            at += 3;
        }
        if p & (1 << 15) != 0 {
            at += 1;
        }
        Some(match d.get(at + 1)? & 0x03 {
            0 => 20,
            1 => 40,
            2 => 80,
            _ => 160,
        })
    }
    fn eht(d: &[u8]) -> Option<u32> {
        if d.first()? & 0x01 == 0 {
            return None;
        }
        match d.get(5)? & 0x07 {
            0 => Some(20),
            1 => Some(40),
            2 => Some(80),
            3 => Some(160),
            4 => Some(320),
            _ => None,
        }
    }
    let mut best = None;
    let mut i = 0;
    while let (Some(&id), Some(&len)) = (ies.get(i), ies.get(i + 1)) {
        let Some(d) = ies.get(i + 2..i + 2 + len as usize) else {
            break;
        };
        let w = match id {
            // HT Operation: a secondary channel, and any width allowed.
            61 if d.len() >= 2 => Some(if d[1] & 0x03 != 0 && d[1] & 0x04 != 0 {
                40
            } else {
                20
            }),
            // VHT Operation: 1 is 80, or 160 when the second centre is set.
            192 if d.len() >= 3 => match d[0] {
                1 if d[2] != 0 && d[1].abs_diff(d[2]) >= 8 => Some(160),
                1 => Some(80),
                2 | 3 => Some(160),
                _ => None,
            },
            255 => match d.first() {
                Some(36) => he(&d[1..]),
                Some(106) => eht(&d[1..]),
                _ => None,
            },
            _ => None,
        };
        best = best.max(w);
        i += 2 + len as usize;
    }
    best
}

/// Values in `nmcli -t` output: split on colons, with `\:` and `\\`
/// unescaped.
#[cfg(any(target_os = "linux", test))]
fn terse_fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let (Some(n), Some(s)) = (chars.next(), out.last_mut()) {
                    s.push(n);
                }
            }
            ':' => out.push(String::new()),
            c => {
                if let Some(s) = out.last_mut() {
                    s.push(c);
                }
            }
        }
    }
    out
}

/// `nmcli -t -f SSID,BSSID,CHAN,FREQ,SIGNAL dev wifi list`.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_nmcli_wifi(text: &str) -> Vec<Bss> {
    let mut aps: Vec<Bss> = Vec::new();
    for line in text.lines() {
        let f = terse_fields(line);
        let [ssid, bssid, chan, freq, signal] = f.as_slice() else {
            continue;
        };
        let Some(mac) = super::mac::parse_mac(bssid.trim()) else {
            continue;
        };
        let bssid = super::mac::fmt(&mac);
        if aps.iter().any(|a| a.bssid == bssid) {
            continue;
        }
        let ssid = super::sanitize(ssid.trim());
        aps.push(Bss {
            ssid: Some(ssid).filter(|s| !s.is_empty() && s != "--"),
            bssid,
            channel: chan.trim().parse().ok(),
            // "5180 MHz".
            freq_mhz: freq.split_whitespace().next().and_then(|v| v.parse().ok()),
            signal_dbm: None,
            signal_pct: signal.trim().parse::<u32>().ok().map(|p| p.min(100)),
        });
    }
    aps
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
        } else if let Some(v) = l.strip_prefix("tx bitrate:") {
            s.tx_mbps = v.split_whitespace().next().and_then(|d| d.parse().ok());
            s.width_mhz = s.width_mhz.or(bitrate_width(v));
        } else if let Some(v) = l.strip_prefix("rx bitrate:") {
            s.rx_mbps = v.split_whitespace().next().and_then(|d| d.parse().ok());
            s.width_mhz = s.width_mhz.or(bitrate_width(v));
        }
    }
    if connected {
        Wifi::Connected(s)
    } else {
        Wifi::Unknown("iw gave no link".into())
    }
}

/// "80MHz" in an iw bitrate line ("866.7 MBit/s VHT-MCS 9 80MHz short GI").
#[cfg(any(target_os = "linux", test))]
fn bitrate_width(v: &str) -> Option<u32> {
    v.split_whitespace()
        .find_map(|w| w.strip_suffix("MHz")?.parse().ok())
}

/// The width in `iw dev <if> info`: "channel 36 (5180 MHz), width: 80 MHz,
/// center1: 5210 MHz".
#[cfg(any(target_os = "linux", test))]
pub(crate) fn parse_iw_info_width(text: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        let (_, w) = l.trim().strip_prefix("channel ")?.split_once("width:")?;
        w.split_whitespace().next()?.parse().ok()
    })
}

#[cfg(target_os = "linux")]
fn iw(adapter: &Adapter, what: &str) -> Result<String, String> {
    let prog = super::system_tool("iw").ok_or("iw isn't installed")?;
    let mut cmd = std::process::Command::new(prog);
    cmd.env("LC_ALL", "C").args(["dev", &adapter.name, what]);
    super::output_within(&mut cmd, std::time::Duration::from_secs(3))
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .map_err(|e| format!("iw: {e}"))
}

#[cfg(target_os = "linux")]
pub fn snapshot(adapter: &Adapter) -> Wifi {
    let mut w = match iw(adapter, "link") {
        Ok(t) => parse_iw_link(&t),
        Err(e) => return Wifi::Unknown(e),
    };
    // The channel's own width, over what the bitrate lines carry.
    if let Wifi::Connected(s) = &mut w
        && let Some(width) = iw(adapter, "info")
            .ok()
            .as_deref()
            .and_then(parse_iw_info_width)
    {
        s.width_mhz = Some(width);
    }
    w
}

/// The access points nearby: NetworkManager's list as it stands, or
/// (`scan`) after it scans, which makes the radio send probe requests and
/// hop channels.
#[cfg(target_os = "linux")]
pub fn nearby(adapter: &Adapter, scan: bool) -> Neighbours {
    let Some(prog) = super::system_tool("nmcli") else {
        return Neighbours::Unknown("nmcli isn't installed (NetworkManager keeps the list)".into());
    };
    let mut cmd = std::process::Command::new(prog);
    cmd.env("LC_ALL", "C").args([
        "-t",
        "-f",
        "SSID,BSSID,CHAN,FREQ,SIGNAL",
        "device",
        "wifi",
        "list",
        "--rescan",
        if scan { "yes" } else { "no" },
        "ifname",
        &adapter.name,
    ]);
    let wait = std::time::Duration::from_secs(if scan { 20 } else { 3 });
    match super::output_within(&mut cmd, wait) {
        Ok(o) if o.status.success() => Neighbours::Listed(Nearby {
            aps: parse_nmcli_wifi(&String::from_utf8_lossy(&o.stdout)),
            age_secs: None,
            scanned: scan,
        }),
        Ok(o) => {
            let e = super::sanitize(String::from_utf8_lossy(&o.stderr).trim());
            let e = e.trim_start_matches("Error: ").trim_end_matches('.');
            Neighbours::Unknown(format!("nmcli: {e}"))
        }
        Err(e) => Neighbours::Unknown(format!("nmcli: {e}")),
    }
}

#[cfg(windows)]
pub use win::{nearby, snapshot};

#[cfg(not(any(windows, target_os = "linux")))]
pub fn snapshot(_: &Adapter) -> Wifi {
    Wifi::Unknown("not supported on this system".into())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn nearby(_: &Adapter, _: bool) -> Neighbours {
    Neighbours::Unknown("not supported on this system".into())
}

#[cfg(windows)]
mod win {
    use super::{Bss, Nearby, Neighbours, Snapshot, Wifi};
    use crate::net::netconf::Adapter;
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{FILETIME, HANDLE};
    use windows_sys::Win32::NetworkManagement::WiFi::{
        DOT11_SSID, WLAN_BSS_ENTRY, WLAN_BSS_LIST, WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO,
        WLAN_INTERFACE_INFO_LIST, WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory,
        WlanGetNetworkBssList, WlanOpenHandle, WlanQueryInterface, WlanScan, dot11_BSS_type_any,
        dot11_BSS_type_infrastructure, wlan_interface_state_connected,
        wlan_intf_opcode_channel_number, wlan_intf_opcode_current_connection,
        wlan_intf_opcode_rssi,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::core::GUID;

    const ACCESS_DENIED: u32 = 5;
    /// How long a driver has to finish a scan WlanScan asks for.
    const SCAN_WAIT: std::time::Duration = std::time::Duration::from_secs(4);

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

    /// Run `f` with a WLAN handle and the GUID of `adapter`'s interface.
    fn with_interface<T>(
        adapter: &Adapter,
        f: impl FnOnce(HANDLE, &GUID) -> T,
        failed: impl Fn(String) -> T,
    ) -> T {
        let mut ver = 0u32;
        let mut h: HANDLE = null_mut();
        if unsafe { WlanOpenHandle(2, null(), &mut ver, &mut h) } != 0 {
            return failed("the WLAN service didn't answer".into());
        }
        let pick = unsafe {
            let mut list: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
            if WlanEnumInterfaces(h, null(), &mut list) != 0 || list.is_null() {
                None
            } else {
                let n = (*list).dwNumberOfItems as usize;
                let first = std::ptr::addr_of!((*list).InterfaceInfo).cast::<WLAN_INTERFACE_INFO>();
                let infos = std::slice::from_raw_parts(first, n);
                let g = infos
                    .iter()
                    .find(|i| guid_text(&i.InterfaceGuid).eq_ignore_ascii_case(&adapter.guid))
                    .or(if n == 1 { infos.first() } else { None })
                    .map(|i| i.InterfaceGuid);
                WlanFreeMemory(list.cast());
                Some(g)
            }
        };
        let r = match pick {
            None => failed("no Wi-Fi adapter listed".into()),
            Some(None) => failed("its Wi-Fi interface wasn't found".into()),
            Some(Some(g)) => f(h, &g),
        };
        unsafe { WlanCloseHandle(h, null()) };
        r
    }

    /// The BSS list (all networks with `ssid` None), each entry with its
    /// information elements; Err with the error code.
    unsafe fn bss_list(
        h: HANDLE,
        g: &GUID,
        ssid: Option<&DOT11_SSID>,
        secure: i32,
    ) -> Result<Vec<(WLAN_BSS_ENTRY, Vec<u8>)>, u32> {
        unsafe {
            let mut bss: *mut WLAN_BSS_LIST = null_mut();
            let (ssid, kind) = match ssid {
                Some(s) => (s as *const DOT11_SSID, dot11_BSS_type_infrastructure),
                None => (null(), dot11_BSS_type_any),
            };
            let r = WlanGetNetworkBssList(h, g, ssid, kind, secure, null(), &mut bss);
            if r != 0 || bss.is_null() {
                return Err(r);
            }
            let total = (*bss).dwTotalSize as usize;
            let n = (*bss).dwNumberOfItems as usize;
            let base = bss.cast::<u8>();
            let first = std::ptr::addr_of!((*bss).wlanBssEntries).cast::<u8>();
            let head = first.offset_from(base) as usize;
            let size = std::mem::size_of::<WLAN_BSS_ENTRY>();
            let mut out = Vec::new();
            for i in 0..n {
                // Bounded before any pointer is made: a count larger than
                // the buffer stops here.
                let at = match i.checked_mul(size).and_then(|o| o.checked_add(head)) {
                    Some(at) if at.checked_add(size).is_some_and(|end| end <= total) => at,
                    _ => break,
                };
                let p = base.add(at).cast::<WLAN_BSS_ENTRY>();
                let e = std::ptr::read_unaligned(p);
                // The elements sit ulIeOffset bytes from the entry's start,
                // within the list's own buffer.
                let (off, len) = (e.ulIeOffset as usize, e.ulIeSize as usize);
                let ies = if len > 0
                    && at
                        .checked_add(off)
                        .and_then(|v| v.checked_add(len))
                        .is_some_and(|end| end <= total)
                {
                    std::slice::from_raw_parts(p.cast::<u8>().add(off), len).to_vec()
                } else {
                    Vec::new()
                };
                out.push((e, ies));
            }
            WlanFreeMemory(bss.cast());
            Ok(out)
        }
    }

    pub fn snapshot(adapter: &Adapter) -> Wifi {
        with_interface(
            adapter,
            |h, g| unsafe { snapshot_with(h, g) },
            Wifi::Unknown,
        )
    }

    unsafe fn snapshot_with(h: HANDLE, g: &GUID) -> Wifi {
        unsafe {
            let mut s = Snapshot {
                channel: read_u32(h, g, wlan_intf_opcode_channel_number),
                signal_dbm: read_u32(h, g, wlan_intf_opcode_rssi).map(|v| v as i32),
                ..Default::default()
            };
            let conn = match query(h, g, wlan_intf_opcode_current_connection) {
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
            // Kbit/s.
            let mbps = |k: u32| (k > 0).then(|| f64::from(k) / 1000.0);
            s.tx_mbps = mbps(a.ulTxRate);
            s.rx_mbps = mbps(a.ulRxRate);
            // The associated access point's own entry: its frequency, RSSI
            // and, from its information elements, its channel width.
            let secure = conn.wlanSecurityAttributes.bSecurityEnabled;
            if let Ok(list) = bss_list(h, g, Some(&a.dot11Ssid), secure)
                && let Some((e, ies)) = list.iter().find(|(e, _)| e.dot11Bssid == bssid)
            {
                s.freq_mhz = Some(e.ulChCenterFrequency / 1000);
                s.signal_dbm = Some(e.lRssi);
                s.width_mhz = super::width_from_ies(ies);
            }
            Wifi::Connected(s)
        }
    }

    /// FILETIME now, in 100 ns ticks.
    fn now_ticks() -> u64 {
        let mut ft = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        unsafe { GetSystemTimeAsFileTime(&mut ft) };
        (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
    }

    /// The access points Windows has heard: its cached list, or (`scan`)
    /// after WlanScan, which sends probe requests and hops channels.
    pub fn nearby(adapter: &Adapter, scan: bool) -> Neighbours {
        with_interface(
            adapter,
            |h, g| unsafe {
                if scan {
                    match WlanScan(h, g, null(), null(), null()) {
                        0 => std::thread::sleep(SCAN_WAIT),
                        ACCESS_DENIED => return Neighbours::Denied,
                        e => return Neighbours::Unknown(format!("WlanScan error {e}")),
                    }
                }
                let list = match bss_list(h, g, None, 0) {
                    Ok(l) => l,
                    Err(ACCESS_DENIED) => return Neighbours::Denied,
                    Err(e) => {
                        return Neighbours::Unknown(format!("WlanGetNetworkBssList error {e}"));
                    }
                };
                let newest = list.iter().map(|(e, _)| e.ullHostTimestamp).max();
                let now = now_ticks();
                // Ticks to seconds; a time in the future, or over a day
                // old, isn't believed.
                let age_secs = newest
                    .filter(|&t| t > 0 && t <= now)
                    .map(|t| (now - t) / 10_000_000)
                    .filter(|&s| s < 86_400);
                let mut aps: Vec<Bss> = Vec::new();
                for (e, _) in &list {
                    let bssid = crate::net::mac::fmt(&e.dot11Bssid);
                    if e.dot11Bssid == [0; 6] || aps.iter().any(|a| a.bssid == bssid) {
                        continue;
                    }
                    let len = (e.dot11Ssid.uSSIDLength as usize).min(32);
                    let ssid =
                        crate::net::sanitize(&String::from_utf8_lossy(&e.dot11Ssid.ucSSID[..len]));
                    let freq = e.ulChCenterFrequency / 1000;
                    aps.push(Bss {
                        ssid: Some(ssid).filter(|v| !v.is_empty()),
                        bssid,
                        freq_mhz: Some(freq),
                        channel: super::band_channel(freq).map(|(_, c)| c),
                        signal_dbm: Some(e.lRssi),
                        signal_pct: None,
                    });
                }
                Neighbours::Listed(Nearby {
                    aps,
                    age_secs,
                    scanned: scan,
                })
            },
            Neighbours::Unknown,
        )
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
                   \trx bitrate: 866.7 MBit/s VHT-MCS 9 80MHz short GI VHT-NSS 2\n\
                   \ttx bitrate: 780.0 MBit/s VHT-MCS 8 80MHz short GI VHT-NSS 2\n";
        let Wifi::Connected(s) = parse_iw_link(out) else {
            panic!("connected")
        };
        assert_eq!(s.ssid.as_deref(), Some("Example Guest"));
        assert_eq!(s.bssid.as_deref(), Some("02:00:5e:10:00:01"));
        assert_eq!(s.freq_mhz, Some(5180));
        assert_eq!(s.signal_dbm, Some(-52));
        // Rates from the bitrate lines; the width too, until `iw info` says.
        assert_eq!((s.tx_mbps, s.rx_mbps), (Some(780.0), Some(866.7)));
        assert_eq!(s.width_mhz, Some(80));
        let r = judge(&Wifi::Connected(s.clone()), "wlan0");
        assert!(
            r.detail.contains("5 GHz channel 36 (80 MHz wide)")
                && r.detail.contains("link Tx 780 / Rx 866.7 Mbit/s"),
            "{}",
            r.detail
        );
        let info = "Interface wlan0\n\tifindex 3\n\ttype managed\n\
                    \tchannel 36 (5180 MHz), width: 160 MHz, center1: 5250 MHz\n";
        assert_eq!(parse_iw_info_width(info), Some(160));
        assert_eq!(parse_iw_info_width("Interface wlan0\n"), None);
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
        assert_eq!(band_channel(5935), Some(("6 GHz", 2)));
        assert_eq!(band_channel(7115), Some(("6 GHz", 233)));
        // Every frequency a driver could report: no panic, no wrap.
        for mhz in 0..=8000 {
            if let Some((_, ch)) = band_channel(mhz) {
                assert!((1..=233).contains(&ch), "{mhz} MHz → {ch}");
            }
        }
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

    fn ap(ssid: &str, last: u8, freq: u32, dbm: i32) -> Bss {
        Bss {
            ssid: Some(ssid.into()).filter(|s: &String| !s.is_empty()),
            bssid: format!("02:00:5e:10:00:{last:02x}"),
            freq_mhz: Some(freq),
            channel: band_channel(freq).map(|(_, c)| c),
            signal_dbm: Some(dbm),
            signal_pct: None,
        }
    }

    #[test]
    fn neighbours_on_this_channel_come_first() {
        let here = Snapshot {
            bssid: Some("02:00:5e:10:00:01".into()),
            freq_mhz: Some(5180),
            ..Default::default()
        };
        let near = Neighbours::Listed(Nearby {
            aps: vec![
                ap("Example Guest", 1, 5180, -50),
                ap("Neighbour", 2, 5180, -71),
                ap("", 3, 5180, -60),
                ap("Example Staff", 4, 5745, -55),
                ap("Far", 5, 2437, -80),
            ],
            age_secs: Some(8),
            scanned: false,
        });
        let r = judge_nearby(&near, Some(&here));
        // Reported, not judged.
        assert_eq!(r.status, Status::Ok);
        let d = &r.detail;
        assert!(d.starts_with("5 access points in the list"), "{d}");
        assert!(
            d.contains("is-alive didn't scan") && d.contains("8 s ago"),
            "{d}"
        );
        // Its own access point isn't its own interference; the strongest
        // co-channel one is named first, a hidden one says so.
        assert!(
            d.contains(
                "2 others on this connection's channel (5 GHz channel 36), its co-channel \
                 interference: (hidden) 02:00:5e:10:00:03 5 GHz channel 36 -60 dBm; \
                 \"Neighbour\""
            ),
            "{d}"
        );
        assert!(d.contains("elsewhere: \"Example Staff\""), "{d}");
        assert!(!d.contains("overlapping"), "{d}");
        // 2.4 GHz: channels under 5 apart overlap.
        let here = Snapshot {
            freq_mhz: Some(2412),
            ..Default::default()
        };
        let near = Neighbours::Listed(Nearby {
            aps: vec![
                ap("A", 1, 2412, -60),
                ap("B", 2, 2422, -65),
                ap("C", 3, 2437, -70),
            ],
            age_secs: None,
            scanned: true,
        });
        let d = judge_nearby(&near, Some(&here)).detail;
        assert!(d.contains("3 access points a scan just found."), "{d}");
        assert!(d.contains("1 other on this connection's channel"), "{d}");
        assert!(
            d.contains("1 on overlapping 2.4 GHz channels: \"B\""),
            "{d}"
        );
        assert!(d.contains("elsewhere: \"C\""), "{d}");
        // A long list is cut, saying how many more.
        let many = Neighbours::Listed(Nearby {
            aps: (0..30).map(|i| ap("X", i, 5745, -40 - i as i32)).collect(),
            ..Default::default()
        });
        let d = judge_nearby(&many, None).detail;
        assert!(d.contains("its age is unknown"), "{d}");
        assert!(d.contains("and 18 more"), "{d}");
        assert_eq!(
            judge_nearby(&Neighbours::Denied, None).status,
            Status::Unknown
        );
        assert!(
            judge_nearby(&Neighbours::Denied, None)
                .detail
                .contains("Let desktop apps access your location")
        );
        // The rows: strongest first, the connected one marked.
        let Neighbours::Listed(n) = near else {
            unreachable!()
        };
        let here = Snapshot {
            bssid: Some("02:00:5e:10:00:02".into()),
            ..Default::default()
        };
        let rows = rows(&n, Some(&here));
        assert_eq!(rows[0][0], "A");
        assert_eq!(
            rows[1][1..],
            ["02:00:5e:10:00:02", "2.4 GHz", "3", "-65", "", "yes"]
        );
    }

    #[test]
    fn networkmanager_lists_the_access_points_it_has() {
        let out = "Example Guest:02\\:00\\:5E\\:10\\:00\\:01:36:5180 MHz:72\n\
                   a\\:b:02\\:00\\:5E\\:10\\:00\\:02:6:2437 MHz:40\n\
                   :02\\:00\\:5E\\:10\\:00\\:03:149:5745 MHz:101\n\
                   Example Guest:02\\:00\\:5E\\:10\\:00\\:01:36:5180 MHz:72\n\
                   junk\n";
        let aps = parse_nmcli_wifi(out);
        assert_eq!(aps.len(), 3, "{aps:?}");
        assert_eq!(aps[0].bssid, "02:00:5e:10:00:01");
        assert_eq!(aps[0].freq_mhz, Some(5180));
        assert_eq!(aps[0].signal_pct, Some(72));
        // An escaped colon stays in the name; a hidden one has none.
        assert_eq!(aps[1].ssid.as_deref(), Some("a:b"));
        assert_eq!(aps[2].ssid, None);
        assert_eq!(aps[2].signal_pct, Some(100));
    }

    #[test]
    fn the_width_from_an_access_points_elements() {
        // HT Operation: primary 36, secondary above, any width.
        let ht40 = [
            61, 22, 36, 0x05, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(width_from_ies(&ht40), Some(40));
        let ht20 = [61, 2, 36, 0x00];
        assert_eq!(width_from_ies(&ht20), Some(20));
        // VHT Operation 80, then 160 by the second centre segment.
        let vht80 = [&ht40[..], &[192, 5, 1, 42, 0, 0, 0]].concat();
        assert_eq!(width_from_ies(&vht80), Some(80));
        let vht160 = [&ht40[..], &[192, 5, 1, 42, 50, 0, 0]].concat();
        assert_eq!(width_from_ies(&vht160), Some(160));
        // HE Operation with 6 GHz info (bit 17), control width 3: 160.
        let he = [255, 12, 36, 0, 0, 0x02, 0, 0, 0, 1, 0x03, 7, 15, 0];
        assert_eq!(width_from_ies(&he), Some(160));
        // EHT Operation: information present, width 4: 320.
        let eht = [255, 9, 106, 0x01, 0, 0, 0, 0, 0x04, 31, 63];
        assert_eq!(width_from_ies(&[&he[..], &eht[..]].concat()), Some(320));
        // Nothing that says, or cut short.
        assert_eq!(width_from_ies(&[0, 4, b'a', b'b', b'c', b'd']), None);
        assert_eq!(width_from_ies(&[192, 5, 1]), None);
    }

    #[test]
    fn hostile_wifi_lists_never_panic() {
        crate::hostile::each_text("x:02\\:00\\:5E\\:10\\:00\\:01:36:5180 MHz:72\n", |t| {
            let _ = parse_nmcli_wifi(t);
        });
        crate::hostile::each_text("\tchannel 36 (5180 MHz), width: 80 MHz\n", |t| {
            let _ = parse_iw_info_width(t);
        });
        crate::hostile::each(
            &[61, 2, 36, 5, 192, 5, 1, 42, 50, 0, 0, 255, 3, 106, 1, 0],
            |b| {
                let _ = width_from_ies(b);
            },
        );
    }
}
