//! Which switch port this computer is plugged into: what the switch says
//! about itself in its LLDP or CDP announcements, sent every 30 to 60
//! seconds to a multicast address on the wire. Read from frames heard on
//! the wired adapter (src/net/listen.rs hears them), or from what lldpd or
//! systemd-networkd already heard. Nothing is sent.
//!
//! The frames come from whatever is at the other end of the cable, so the
//! parsers take any bytes without panicking: every length is checked
//! against what's left, TLVs are counted and capped, and text is cleaned
//! and cut short. Never recursive: LLDP and CDP are flat lists of TLVs.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// LLDP's multicast destination (nearest bridge) and its EtherType.
pub const LLDP_MAC: [u8; 6] = [0x01, 0x80, 0xc2, 0x00, 0x00, 0x0e];
pub const LLDP_TYPE: u16 = 0x88cc;
/// CDP's multicast destination, and its SNAP protocol ID under Cisco's OUI.
pub const CDP_MAC: [u8; 6] = [0x01, 0x00, 0x0c, 0xcc, 0xcc, 0xcc];
const CDP_PID: u16 = 0x2000;

/// TLVs read from one frame at most: a real one has 10 to 30.
const MAX_TLVS: usize = 128;
/// VLAN names kept from one frame.
const MAX_VLANS: usize = 32;
/// Characters kept of a description (sysDescr runs to several lines).
const MAX_DESC: usize = 200;

/// What the switch said about itself and the port.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Neighbor {
    /// "LLDP" or "CDP".
    pub proto: &'static str,
    /// Its name: LLDP's system name, CDP's device ID.
    pub switch: Option<String>,
    /// LLDP's chassis ID (often the switch's MAC).
    pub chassis: Option<String>,
    /// The port this computer is on, as the switch names it.
    pub port: Option<String>,
    /// The port's description (often what's written on the patch panel).
    pub port_desc: Option<String>,
    /// LLDP's system description, CDP's platform.
    pub model: Option<String>,
    pub mgmt: Option<IpAddr>,
    /// The port's untagged VLAN: LLDP's port VLAN ID, CDP's native VLAN.
    pub vlan: Option<u16>,
    /// The voice VLAN: LLDP-MED's network policy, CDP's VoIP VLAN.
    pub voice_vlan: Option<u16>,
    /// VLANs the switch named on the port (802.1 VLAN name), id and name.
    pub vlan_names: Vec<(u16, String)>,
    /// PoE, as words ("class 4, 25.5 W allocated").
    pub poe: Option<String>,
}

// ------------------------------------------------------------- frames

/// One Ethernet frame, as captured: an LLDP or CDP announcement, or None
/// for anything else (or anything malformed).
pub fn parse_frame(f: &[u8]) -> Option<Neighbor> {
    if f.len() < 14 {
        return None;
    }
    let dst = &f[..6];
    let mut at = 12;
    let mut kind = u16::from_be_bytes([f[at], f[at + 1]]);
    // One 802.1Q tag, as some switches send LLDP on a tagged port.
    if kind == 0x8100 {
        at += 4;
        kind = u16::from_be_bytes([*f.get(at)?, *f.get(at + 1)?]);
    }
    let body = f.get(at + 2..)?;
    if dst == LLDP_MAC && kind == LLDP_TYPE {
        return parse_lldp(body);
    }
    // CDP: an 802.3 length (not a type), then LLC/SNAP.
    if dst == CDP_MAC && kind <= 1500 {
        let len = usize::from(kind).min(body.len());
        let body = &body[..len];
        if body.get(..3)? == [0xaa, 0xaa, 0x03]
            && body.get(3..6)? == [0x00, 0x00, 0x0c]
            && u16::from_be_bytes([*body.get(6)?, *body.get(7)?]) == CDP_PID
        {
            return parse_cdp(&body[8..]);
        }
    }
    None
}

/// An LLDPDU: TLVs of 7 bits type, 9 bits length.
pub fn parse_lldp(b: &[u8]) -> Option<Neighbor> {
    let mut n = Neighbor {
        proto: "LLDP",
        ..Default::default()
    };
    let mut at = 0;
    let mut seen = 0;
    let mut chassis = false;
    while at + 2 <= b.len() && seen < MAX_TLVS {
        seen += 1;
        let h = u16::from_be_bytes([b[at], b[at + 1]]);
        let (t, len) = (h >> 9, usize::from(h & 0x1ff));
        at += 2;
        let Some(v) = b.get(at..at + len) else { break };
        at += len;
        match t {
            0 => break,
            1 if !v.is_empty() => {
                chassis = true;
                n.chassis = id_text(v[0], &v[1..], true);
            }
            2 if !v.is_empty() => n.port = id_text(v[0], &v[1..], false),
            4 => n.port_desc = text(v, 255),
            5 => n.switch = text(v, 255),
            6 => n.model = text(v, MAX_DESC),
            8 => {
                if let Some(ip) = mgmt_addr(v) {
                    // IPv4 over IPv6: what a tech types into a browser.
                    if n.mgmt.is_none_or(|m| m.is_ipv6() && ip.is_ipv4()) {
                        n.mgmt = Some(ip);
                    }
                }
            }
            127 if v.len() >= 4 => org(&mut n, [v[0], v[1], v[2]], v[3], &v[4..]),
            _ => {}
        }
    }
    // Chassis and port ID are what make an LLDPDU one.
    (chassis && n.port.is_some()).then_some(n)
}

/// LLDP's chassis or port ID by its subtype.
fn id_text(sub: u8, v: &[u8], chassis: bool) -> Option<String> {
    // Chassis 4 and port 3 are a MAC; chassis 5 and port 4 an address.
    let (mac, addr) = if chassis { (4, 5) } else { (3, 4) };
    if sub == mac && v.len() == 6 {
        let m: [u8; 6] = v.try_into().ok()?;
        return Some(super::mac::fmt(&m));
    }
    if sub == addr {
        return net_addr(v).map(|a| a.to_string());
    }
    text(v, 255).or_else(|| hex(v))
}

/// An address family byte (1 IPv4, 2 IPv6) and the address.
fn net_addr(v: &[u8]) -> Option<IpAddr> {
    match (v.first()?, v.len()) {
        (1, 5) => Some(IpAddr::V4(Ipv4Addr::new(v[1], v[2], v[3], v[4]))),
        (2, 17) => {
            let a: [u8; 16] = v[1..].try_into().ok()?;
            Some(IpAddr::V6(Ipv6Addr::from(a)))
        }
        _ => None,
    }
}

/// LLDP's management address TLV: a length (counting the family byte),
/// the family, the address; the interface numbering after it is skipped.
fn mgmt_addr(v: &[u8]) -> Option<IpAddr> {
    let len = usize::from(*v.first()?);
    net_addr(v.get(1..1 + len)?)
}

/// Organizationally specific TLVs: 802.1, 802.3 and LLDP-MED.
fn org(n: &mut Neighbor, oui: [u8; 3], sub: u8, v: &[u8]) {
    match (oui, sub) {
        // 802.1: port VLAN ID.
        ([0x00, 0x80, 0xc2], 1) if v.len() >= 2 => {
            let id = u16::from_be_bytes([v[0], v[1]]);
            if (1..4095).contains(&id) {
                n.vlan = Some(id);
            }
        }
        // 802.1: VLAN name.
        ([0x00, 0x80, 0xc2], 3) if v.len() >= 3 => {
            let id = u16::from_be_bytes([v[0], v[1]]);
            let len = usize::from(v[2]);
            if let Some(name) = v.get(3..3 + len).and_then(|s| text(s, 64))
                && n.vlan_names.len() < MAX_VLANS
                && !n.vlan_names.iter().any(|(i, _)| *i == id)
            {
                n.vlan_names.push((id, name));
            }
        }
        // 802.3: power via MDI.
        ([0x00, 0x12, 0x0f], 2) if v.len() >= 3 => n.poe = Some(poe_8023(v)),
        // LLDP-MED: network policy, voice (1) when the VLAN is known.
        ([0x00, 0x12, 0xbb], 2) if v.len() >= 4 => {
            let bits = u32::from_be_bytes([0, v[1], v[2], v[3]]);
            let unknown = bits & 0x80_0000 != 0;
            let id = ((bits >> 9) & 0xfff) as u16;
            if v[0] == 1 && !unknown && (1..4095).contains(&id) {
                n.voice_vlan = Some(id);
            }
        }
        _ => {}
    }
}

/// 802.3's power via MDI: support flags, pair, class, and (802.3at) the
/// power the switch has allocated, in tenths of a watt.
fn poe_8023(v: &[u8]) -> String {
    let mut s = Vec::new();
    // MDI power support: bit 0 says the sender is a PSE (gives power).
    if v[0] & 0x01 != 0 {
        s.push("switch supplies power".to_string());
    }
    // Bit 1: power is supported here; bit 2: it's switched on.
    if v[0] & 0x02 != 0 {
        s.push(if v[0] & 0x04 != 0 { "on" } else { "off" }.to_string());
    }
    // Class 1 to 5 stands for PoE class 0 to 4.
    if (1..=5).contains(&v[2]) {
        s.push(format!("class {}", v[2] - 1));
    }
    if v.len() >= 8 {
        let alloc = u16::from_be_bytes([v[6], v[7]]);
        if alloc > 0 {
            s.push(format!("{:.1} W allocated", f32::from(alloc) / 10.0));
        }
    }
    if s.is_empty() {
        "advertised".into()
    } else {
        s.join(", ")
    }
}

/// A CDP packet after its SNAP header: version, TTL, checksum, then TLVs
/// of 2 bytes type, 2 bytes length (counting those 4).
pub fn parse_cdp(b: &[u8]) -> Option<Neighbor> {
    if b.len() < 4 || !(1..=2).contains(&b[0]) {
        return None;
    }
    let mut n = Neighbor {
        proto: "CDP",
        ..Default::default()
    };
    let mut at = 4;
    let mut seen = 0;
    while at + 4 <= b.len() && seen < MAX_TLVS {
        seen += 1;
        let t = u16::from_be_bytes([b[at], b[at + 1]]);
        let len = usize::from(u16::from_be_bytes([b[at + 2], b[at + 3]]));
        if len < 4 {
            break;
        }
        let Some(v) = b.get(at + 4..at + len) else {
            break;
        };
        at += len;
        match t {
            0x0001 => n.switch = text(v, 255),
            0x0002 | 0x0016 => {
                if let Some(ip) = cdp_addrs(v)
                    && (t == 0x0016 || n.mgmt.is_none())
                {
                    n.mgmt = Some(ip);
                }
            }
            0x0003 => n.port = text(v, 255),
            0x0006 => n.model = text(v, MAX_DESC),
            0x000a if v.len() >= 2 => {
                let id = u16::from_be_bytes([v[0], v[1]]);
                if (1..4095).contains(&id) {
                    n.vlan = Some(id);
                }
            }
            0x000e if v.len() >= 3 => {
                let id = u16::from_be_bytes([v[1], v[2]]);
                if (1..4095).contains(&id) {
                    n.voice_vlan = Some(id);
                }
            }
            0x0010 if v.len() >= 2 => {
                let mw = u16::from_be_bytes([v[0], v[1]]);
                n.poe = Some(format!("{:.1} W", f32::from(mw) / 1000.0));
            }
            _ => {}
        }
    }
    (n.switch.is_some() || n.port.is_some()).then_some(n)
}

/// CDP's address list: a count, then (protocol type, length, protocol,
/// address length, address). The first IPv4 address, else IPv6.
fn cdp_addrs(v: &[u8]) -> Option<IpAddr> {
    let count = u32::from_be_bytes(v.get(..4)?.try_into().ok()?);
    let mut at = 4;
    let mut v6 = None;
    for _ in 0..count.min(16) {
        let pt = *v.get(at)?;
        let pl = usize::from(*v.get(at + 1)?);
        let proto = v.get(at + 2..at + 2 + pl)?;
        at += 2 + pl;
        let al = usize::from(u16::from_be_bytes([*v.get(at)?, *v.get(at + 1)?]));
        let a = v.get(at + 2..at + 2 + al)?;
        at += 2 + al;
        match (pt, proto, al) {
            // NLPID, IP.
            (1, [0xcc], 4) => return Some(IpAddr::V4(Ipv4Addr::new(a[0], a[1], a[2], a[3]))),
            // 802.2, IPv6's EtherType at the end.
            (2, [.., 0x86, 0xdd], 16) => {
                let b: [u8; 16] = a.try_into().ok()?;
                v6.get_or_insert(IpAddr::V6(Ipv6Addr::from(b)));
            }
            _ => {}
        }
    }
    v6
}

/// Printable UTF-8 as one cleaned line, at most `max` characters; None if
/// empty or binary.
fn text(b: &[u8], max: usize) -> Option<String> {
    let b = &b[..b.len() - b.iter().rev().take_while(|&&c| c == 0).count()];
    let s = std::str::from_utf8(b).ok()?;
    if s.chars()
        .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
    {
        return None;
    }
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let one: String = super::sanitize(&one).chars().take(max).collect();
    (!one.is_empty()).then_some(one)
}

/// Binary as hex pairs, 32 bytes at most.
fn hex(b: &[u8]) -> Option<String> {
    (!b.is_empty()).then(|| {
        b.iter()
            .take(32)
            .map(|c| format!("{c:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    })
}

// ------------------------------------------------- what daemons heard

/// `lldpcli -f json show neighbors`: lldpd's own JSON, whose shape varies
/// with how many of a thing there are (an object for one, a list for
/// several; a chassis keyed by its name, or not). The neighbour heard on
/// `iface` (any, when None), first one wins.
// Linux's daemons only.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_lldpcli(json: &str, iface: Option<&str>) -> Option<Neighbor> {
    use serde_json::Value;
    let v: Value = serde_json::from_str(json).ok()?;
    let ifs = v.get("lldp")?.get("interface")?;
    // Each interface as (name, body).
    let objs: Vec<&Value> = match ifs {
        Value::Array(a) => a.iter().take(64).collect(),
        o => vec![o],
    };
    let (_, b) = objs
        .into_iter()
        .filter_map(Value::as_object)
        .flat_map(|m| m.iter().take(64))
        .find(|(name, _)| iface.is_none_or(|i| i == name.as_str()))?;
    let via = b.get("via").and_then(Value::as_str).unwrap_or("LLDP");
    let mut n = Neighbor {
        proto: if via.eq_ignore_ascii_case("cdpv1") || via.eq_ignore_ascii_case("cdpv2") {
            "CDP"
        } else {
            "LLDP"
        },
        ..Default::default()
    };
    if let Some(c) = b.get("chassis").and_then(Value::as_object) {
        // Keyed by the system name, when it has one.
        let (name, c) = match c.get("id") {
            Some(_) => (None, b.get("chassis")),
            None => match c.iter().next() {
                Some((k, inner)) => (Some(k.clone()), Some(inner)),
                None => (None, None),
            },
        };
        n.switch = name.and_then(|s| text(s.as_bytes(), 255));
        if let Some(c) = c {
            n.chassis = c.get("id").and_then(value_text);
            n.model = c
                .get("descr")
                .and_then(value_text)
                .map(|d| cut(&d, MAX_DESC));
            n.mgmt = c.get("mgmt-ip").and_then(|m| match m {
                Value::Array(a) => a.iter().filter_map(value_text).find_map(|s| s.parse().ok()),
                m => value_text(m).and_then(|s| s.parse().ok()),
            });
            if n.switch.is_none() {
                n.switch = c.get("name").and_then(value_text);
            }
        }
    }
    if let Some(p) = b.get("port") {
        n.port = p.get("id").and_then(value_text);
        n.port_desc = p.get("descr").and_then(value_text);
    }
    let vlans: Vec<&Value> = match b.get("vlan") {
        Some(Value::Array(a)) => a.iter().take(MAX_VLANS).collect(),
        Some(o) => vec![o],
        None => Vec::new(),
    };
    for vl in vlans {
        let id = vl
            .get("vlan-id")
            .and_then(value_text)
            .and_then(|s| s.parse::<u16>().ok());
        let Some(id) = id.filter(|i| (1..4095).contains(i)) else {
            continue;
        };
        if vl.get("pvid").and_then(Value::as_bool) == Some(true) {
            n.vlan = Some(id);
        }
        if let Some(name) = vl.get("value").and_then(value_text)
            && !n.vlan_names.iter().any(|(i, _)| *i == id)
        {
            n.vlan_names.push((id, name));
        }
    }
    if let Some(med) = b.get("lldp-med") {
        let pol: Vec<&Value> = match med.get("policy") {
            Some(Value::Array(a)) => a.iter().take(16).collect(),
            Some(o) => vec![o],
            None => Vec::new(),
        };
        for p in pol {
            let voice = p
                .get("apptype")
                .and_then(value_text)
                .is_some_and(|a| a.eq_ignore_ascii_case("voice"));
            let id = p
                .get("vlan")
                .and_then(|v| v.get("vlan-id").or(Some(v)))
                .and_then(value_text)
                .and_then(|s| s.parse::<u16>().ok());
            if voice && let Some(id) = id.filter(|i| (1..4095).contains(i)) {
                n.voice_vlan = Some(id);
            }
        }
    }
    (n.port.is_some() || n.switch.is_some()).then_some(n)
}

/// A lldpd value as text: a string, a number, or `{"value": …}`, or the
/// first of a list of those.
// Linux's daemons only.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn value_text(v: &serde_json::Value) -> Option<String> {
    use serde_json::Value;
    match v {
        Value::String(s) => text(s.as_bytes(), 255),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(o) => o.get("value").and_then(value_text),
        Value::Array(a) => a.first().and_then(value_text),
        _ => None,
    }
}

// Linux's daemons only.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// `networkctl lldp <iface>`: systemd-networkd's table. The columns are
/// found by the header's names, which differ between versions (CHASSIS ID
/// came out, SYSTEM DESCRIPTION went in); the first row for `iface` (any,
/// when None) wins.
// Linux's daemons only.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_networkctl(out: &str, iface: Option<&str>) -> Option<Neighbor> {
    let mut lines = out.lines();
    let head = lines.find(|l| l.trim_start().starts_with("LINK"))?;
    let names = [
        "LINK",
        "CHASSIS ID",
        "SYSTEM NAME",
        "SYSTEM DESCRIPTION",
        "CAPS",
        "PORT ID",
        "PORT DESCRIPTION",
    ];
    let mut cols: Vec<(&str, usize)> = names
        .iter()
        .filter_map(|n| head.find(n).map(|at| (*n, at)))
        .collect();
    cols.sort_by_key(|c| c.1);
    if cols.len() < 3 {
        return None;
    }
    for line in lines.take(256) {
        // The table ends at its first blank line; the legend follows.
        if line.trim().is_empty() {
            break;
        }
        let field = |name: &str| -> Option<String> {
            let i = cols.iter().position(|c| c.0 == name)?;
            let start = cols[i].1;
            let end = cols.get(i + 1).map_or(line.len(), |c| c.1).min(line.len());
            let s = line.get(start.min(end)..end)?.trim();
            (!s.is_empty() && s != "n/a").then(|| super::sanitize(s))
        };
        let link = field("LINK");
        if iface.is_some_and(|i| link.as_deref() != Some(i)) {
            continue;
        }
        let n = Neighbor {
            proto: "LLDP",
            switch: field("SYSTEM NAME"),
            chassis: field("CHASSIS ID"),
            port: field("PORT ID"),
            port_desc: field("PORT DESCRIPTION"),
            model: field("SYSTEM DESCRIPTION"),
            ..Default::default()
        };
        if n.port.is_some() || n.switch.is_some() {
            return Some(n);
        }
    }
    None
}

// ------------------------------------------------------------- words

impl Neighbor {
    /// (field, label, value) for what's known, in reading order: the
    /// fields are Results' and the bundle's, the labels the screen's.
    pub fn fields(&self) -> Vec<(&'static str, &'static str, String)> {
        let mut f = Vec::new();
        let mut put = |k, l, v: Option<String>| {
            if let Some(v) = v {
                f.push((k, l, v));
            }
        };
        put("switch", "Switch", self.switch.clone());
        put("port", "Port", self.port.clone());
        put("port_description", "Port desc.", self.port_desc.clone());
        put("vlan", "VLAN", self.vlan.map(|v| v.to_string()));
        put(
            "voice_vlan",
            "Voice VLAN",
            self.voice_vlan.map(|v| v.to_string()),
        );
        put(
            "vlan_names",
            "VLAN names",
            (!self.vlan_names.is_empty()).then(|| {
                self.vlan_names
                    .iter()
                    .map(|(i, n)| format!("{i} {n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }),
        );
        put("management", "Management", self.mgmt.map(|m| m.to_string()));
        put("poe", "PoE", self.poe.clone());
        put("model", "Model", self.model.clone());
        put("chassis_id", "Chassis ID", self.chassis.clone());
        f
    }

    /// "Gi1/0/12 on sw-core-01".
    pub fn short(&self) -> String {
        match (&self.port, &self.switch) {
            (Some(p), Some(s)) => format!("{p} on {s}"),
            (Some(p), None) => p.clone(),
            (None, Some(s)) => s.clone(),
            (None, None) => "a switch".into(),
        }
    }

    /// The scanner's record in a bundle's inventory.
    pub fn json(&self, via: &str) -> serde_json::Value {
        serde_json::json!({
            "heard_via": via,
            "protocol": self.proto,
            "switch": self.switch,
            "port": self.port,
            "port_description": self.port_desc,
            "vlan": self.vlan,
            "voice_vlan": self.voice_vlan,
            "vlan_names": self.vlan_names.iter().map(|(i, n)| serde_json::json!({"id": i, "name": n})).collect::<Vec<_>>(),
            "management": self.mgmt.map(|m| m.to_string()),
            "poe": self.poe,
            "model": self.model,
            "chassis_id": self.chassis,
        })
    }
}

/// What was heard, and how: what the scanner's record in a bundle holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub n: Neighbor,
    /// "raw socket", "Npcap", "lldpd", "systemd-networkd", or "SNMP" (a
    /// switch's forwarding table, not an announcement).
    pub via: String,
    /// The wired adapter it was heard on (empty for SNMP).
    pub adapter: String,
}

impl Heard {
    /// Results' rows (field, value): the neighbour's, then how.
    pub fn rows(&self) -> Vec<Vec<String>> {
        let mut r: Vec<Vec<String>> = self
            .n
            .fields()
            .into_iter()
            .map(|(k, _, v)| vec![k.to_string(), v])
            .collect();
        r.push(vec!["protocol".into(), self.n.proto.into()]);
        r.push(vec!["heard_via".into(), self.via.clone()]);
        if !self.adapter.is_empty() {
            r.push(vec!["adapter".into(), self.adapter.clone()]);
        }
        r
    }

    /// Back from a Results run's rows; None for a run that heard nothing.
    pub fn from_rows(rows: &[Vec<String>]) -> Option<Heard> {
        let get = |k: &str| {
            rows.iter()
                .find(|r| r.first().is_some_and(|f| f == k))
                .and_then(|r| r.get(1))
                .filter(|v| !v.is_empty())
                .cloned()
        };
        let via = get("heard_via")?;
        let n = Neighbor {
            proto: match get("protocol").as_deref() {
                Some("CDP") => "CDP",
                Some("SNMP") => "SNMP",
                _ => "LLDP",
            },
            switch: get("switch"),
            chassis: get("chassis_id"),
            port: get("port"),
            port_desc: get("port_description"),
            model: get("model"),
            mgmt: get("management").and_then(|m| m.parse().ok()),
            vlan: get("vlan").and_then(|v| v.parse().ok()),
            voice_vlan: get("voice_vlan").and_then(|v| v.parse().ok()),
            vlan_names: get("vlan_names")
                .map(|v| {
                    v.split(", ")
                        .take(MAX_VLANS)
                        .filter_map(|p| {
                            let (i, n) = p.split_once(' ')?;
                            Some((i.parse().ok()?, n.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            poe: get("poe"),
        };
        (n.port.is_some() || n.switch.is_some()).then(|| Heard {
            n,
            via,
            adapter: get("adapter").unwrap_or_default(),
        })
    }

    /// The scanner's record in a bundle's inventory.
    pub fn json(&self) -> serde_json::Value {
        let mut v = self.n.json(&self.via);
        v["adapter"] = serde_json::json!((!self.adapter.is_empty()).then_some(&self.adapter));
        v
    }
}

// ------------------------------------------------------------- tests

/// Frames for tests and the scene tour: fictional names, documentation
/// addresses, a locally administered (made-up) MAC.
#[cfg(test)]
pub mod sample {
    fn tlv(t: u16, v: &[u8]) -> Vec<u8> {
        let mut o = ((t << 9) | v.len() as u16).to_be_bytes().to_vec();
        o.extend_from_slice(v);
        o
    }

    fn eth(dst: [u8; 6], rest: &[u8]) -> Vec<u8> {
        let mut f = dst.to_vec();
        f.extend_from_slice(&[0x02, 0x00, 0x5e, 0x10, 0x00, 0x20]);
        f.extend_from_slice(rest);
        f
    }

    /// An LLDP frame from "sw-example-01", port Gi1/0/12.
    pub fn lldp() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(tlv(1, &[4, 0x02, 0x00, 0x5e, 0x10, 0x00, 0x20]));
        b.extend(tlv(2, b"\x05Gi1/0/12"));
        b.extend(tlv(3, &[0, 120]));
        b.extend(tlv(4, b"Room 2.14 desk B"));
        b.extend(tlv(5, b"sw-example-01"));
        b.extend(tlv(6, b"Example Switch 2400\nVersion 1.2.3"));
        b.extend(tlv(8, &[5, 1, 192, 0, 2, 10, 2, 0, 0, 0, 1, 0]));
        b.extend(tlv(127, &[0x00, 0x80, 0xc2, 1, 0, 20]));
        b.extend(tlv(
            127,
            &[0x00, 0x80, 0xc2, 3, 0, 20, 5, b'S', b't', b'a', b'f', b'f'],
        ));
        b.extend(tlv(
            127,
            &[0x00, 0x80, 0xc2, 3, 0, 30, 5, b'V', b'o', b'i', b'c', b'e'],
        ));
        b.extend(tlv(
            127,
            &[0x00, 0x12, 0x0f, 2, 0x0f, 1, 5, 0x51, 0, 0xff, 0, 0xff],
        ));
        // Voice, tagged, VLAN 30, priority 5, DSCP 46.
        let bits: u32 = (1 << 22) | (30 << 9) | (5 << 6) | 46;
        let b3 = bits.to_be_bytes();
        b.extend(tlv(127, &[0x00, 0x12, 0xbb, 2, 1, b3[1], b3[2], b3[3]]));
        b.extend(tlv(0, &[]));
        let mut rest = super::LLDP_TYPE.to_be_bytes().to_vec();
        rest.extend(b);
        eth(super::LLDP_MAC, &rest)
    }

    /// A CDP frame from "sw-example-02", port GigabitEthernet0/7.
    pub fn cdp() -> Vec<u8> {
        let t = |t: u16, v: &[u8]| {
            let mut o = t.to_be_bytes().to_vec();
            o.extend((v.len() as u16 + 4).to_be_bytes());
            o.extend_from_slice(v);
            o
        };
        let mut c = vec![2, 180, 0, 0];
        c.extend(t(1, b"sw-example-02"));
        c.extend(t(2, &[0, 0, 0, 1, 1, 1, 0xcc, 0, 4, 192, 0, 2, 20]));
        c.extend(t(3, b"GigabitEthernet0/7"));
        c.extend(t(6, b"Example 2960"));
        c.extend(t(0x0a, &[0, 10]));
        c.extend(t(0x0e, &[1, 0, 40]));
        c.extend(t(0x10, &[0x3a, 0x98]));
        let mut snap = vec![0xaa, 0xaa, 0x03, 0, 0, 0x0c, 0x20, 0x00];
        snap.extend(c);
        let mut rest = (snap.len() as u16).to_be_bytes().to_vec();
        rest.extend(snap);
        eth(super::CDP_MAC, &rest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_lldp_frame_names_the_switch_port_and_vlans() {
        let n = parse_frame(&sample::lldp()).expect("an LLDP frame");
        assert_eq!(n.proto, "LLDP");
        assert_eq!(n.switch.as_deref(), Some("sw-example-01"));
        assert_eq!(n.port.as_deref(), Some("Gi1/0/12"));
        assert_eq!(n.port_desc.as_deref(), Some("Room 2.14 desk B"));
        assert_eq!(
            n.model.as_deref(),
            Some("Example Switch 2400 Version 1.2.3")
        );
        assert_eq!(n.chassis.as_deref(), Some("02:00:5e:10:00:20"));
        assert_eq!(n.mgmt, Some("192.0.2.10".parse().unwrap()));
        assert_eq!(n.vlan, Some(20));
        assert_eq!(n.voice_vlan, Some(30));
        assert_eq!(
            n.vlan_names,
            vec![(20, "Staff".into()), (30, "Voice".into())]
        );
        assert_eq!(
            n.poe.as_deref(),
            Some("switch supplies power, on, class 4, 25.5 W allocated")
        );
        assert_eq!(n.short(), "Gi1/0/12 on sw-example-01");
    }

    #[test]
    fn a_cdp_frame_names_the_switch_port_and_vlans() {
        let n = parse_frame(&sample::cdp()).expect("a CDP frame");
        assert_eq!(n.proto, "CDP");
        assert_eq!(n.switch.as_deref(), Some("sw-example-02"));
        assert_eq!(n.port.as_deref(), Some("GigabitEthernet0/7"));
        assert_eq!(n.model.as_deref(), Some("Example 2960"));
        assert_eq!(n.mgmt, Some("192.0.2.20".parse().unwrap()));
        assert_eq!(n.vlan, Some(10));
        assert_eq!(n.voice_vlan, Some(40));
        assert_eq!(n.poe.as_deref(), Some("15.0 W"));
    }

    #[test]
    fn other_frames_are_not_announcements() {
        // The LLDP frame to the wrong address, and with the wrong type.
        let mut f = sample::lldp();
        f[5] = 0x03;
        assert_eq!(parse_frame(&f), None);
        let mut f = sample::lldp();
        f[13] = 0x00;
        assert_eq!(parse_frame(&f), None);
        // Without a chassis ID it isn't an LLDPDU.
        assert_eq!(parse_lldp(&[0x04, 0x02, 0x05, b'x', 0, 0]), None);
    }

    #[test]
    fn a_heard_switch_comes_back_from_its_results_rows() {
        for f in [sample::lldp(), sample::cdp()] {
            let n = parse_frame(&f).unwrap();
            let h = Heard {
                n,
                via: "raw socket".into(),
                adapter: "eth0".into(),
            };
            assert_eq!(Heard::from_rows(&h.rows()), Some(h.clone()));
            assert_eq!(h.json()["port"], h.n.port.clone().unwrap().as_str());
        }
        assert_eq!(
            Heard::from_rows(&[vec!["result".into(), "silent".into()]]),
            None
        );
    }

    #[test]
    fn a_tagged_lldp_frame_is_read() {
        let f = sample::lldp();
        let mut t = f[..12].to_vec();
        t.extend([0x81, 0x00, 0x00, 0x14]);
        t.extend(&f[12..]);
        assert_eq!(parse_frame(&t), parse_frame(&f));
    }

    #[test]
    fn hostile_frames_never_panic() {
        for good in [sample::lldp(), sample::cdp()] {
            crate::hostile::each(&good, |f| {
                if let Some(n) = parse_frame(f) {
                    assert!(n.vlan_names.len() <= MAX_VLANS);
                    let _ = (n.fields(), n.json("frame"));
                }
            });
        }
        crate::hostile::each(&sample::lldp()[14..], |b| {
            let _ = parse_lldp(b);
        });
        crate::hostile::each(&sample::cdp()[24..], |b| {
            let _ = parse_cdp(b);
        });
        // A frame of nothing but empty TLVs stops at the cap.
        let mut f = sample::lldp()[..14].to_vec();
        f.extend(std::iter::repeat_n([0x02, 0x00], 5000).flatten());
        assert_eq!(parse_frame(&f), None);
        // A CDP address list claiming four billion entries.
        let mut v = vec![0xff, 0xff, 0xff, 0xff];
        v.extend([1, 1, 0xcc, 0, 4, 192, 0, 2, 1]);
        assert!(cdp_addrs(&v).is_some());
    }

    #[test]
    fn long_and_odd_text_is_cleaned_and_cut() {
        let mut b = Vec::new();
        b.extend(sample_tlv(1, &[7, b'c']));
        b.extend(sample_tlv(2, &[7, b'p']));
        let mut long = b"\x1b[31m".to_vec();
        long.extend(std::iter::repeat_n(b'x', 500));
        b.extend(sample_tlv(5, &long));
        b.extend(sample_tlv(6, &[b'a'; 500]));
        b.extend(sample_tlv(4, "\u{202e}desk".as_bytes()));
        let n = parse_lldp(&b).expect("an LLDPDU");
        // Escape codes make it binary, not a name.
        assert_eq!(n.switch, None);
        assert_eq!(n.model.as_ref().map(|m| m.chars().count()), Some(MAX_DESC));
        assert_eq!(n.port_desc.as_deref(), Some("desk"));
        // A port ID that's binary reads as hex.
        let mut b = Vec::new();
        b.extend(sample_tlv(1, &[7, b'c']));
        b.extend(sample_tlv(2, &[6, 0x00, 0x01]));
        assert_eq!(parse_lldp(&b).unwrap().port.as_deref(), Some("00:01"));
    }

    fn sample_tlv(t: u16, v: &[u8]) -> Vec<u8> {
        let mut o = ((t << 9) | v.len() as u16).to_be_bytes().to_vec();
        o.extend_from_slice(v);
        o
    }

    const LLDPCLI: &str = r#"{"lldp": {"interface": {"eth0": {
        "via": "LLDP", "rid": "1", "age": "0 day, 00:01:10",
        "chassis": {"sw-example-01": {
            "id": {"type": "mac", "value": "02:00:5e:10:00:20"},
            "descr": "Example Switch 2400", "mgmt-ip": ["192.0.2.10", "2001:db8::10"]}},
        "port": {"id": {"type": "ifname", "value": "Gi1/0/12"}, "descr": "Room 2.14", "ttl": "120"},
        "vlan": [{"vlan-id": "20", "pvid": true, "value": "Staff"}, {"vlan-id": "30", "pvid": false, "value": "Voice"}],
        "lldp-med": {"policy": {"apptype": "Voice", "defined": true, "vlan": {"vlan-id": "30"}}}
    }}}}"#;

    #[test]
    fn lldpcli_json_is_read() {
        let n = parse_lldpcli(LLDPCLI, Some("eth0")).expect("a neighbour");
        assert_eq!(n.switch.as_deref(), Some("sw-example-01"));
        assert_eq!(n.port.as_deref(), Some("Gi1/0/12"));
        assert_eq!(n.port_desc.as_deref(), Some("Room 2.14"));
        assert_eq!(n.mgmt, Some("192.0.2.10".parse().unwrap()));
        assert_eq!(n.vlan, Some(20));
        assert_eq!(n.voice_vlan, Some(30));
        assert_eq!(parse_lldpcli(LLDPCLI, Some("eth1")), None);
        // Several interfaces come as a list.
        let list = r#"{"lldp": {"interface": [
            {"eth1": {"chassis": {"id": {"value": "c1"}, "name": {"value": "sw-other"}}, "port": {"id": {"value": "7"}}}},
            {"eth0": {"via": "CDPv2", "chassis": {"sw-example-02": {"id": {"value": "c2"}}}, "port": {"id": {"value": "Gi0/7"}}}}
        ]}}"#;
        let n = parse_lldpcli(list, Some("eth0")).unwrap();
        assert_eq!((n.proto, n.port.as_deref()), ("CDP", Some("Gi0/7")));
        let n = parse_lldpcli(list, Some("eth1")).unwrap();
        assert_eq!(n.switch.as_deref(), Some("sw-other"));
        // No neighbours yet.
        assert_eq!(parse_lldpcli(r#"{"lldp": {}}"#, None), None);
    }

    #[test]
    fn hostile_lldpcli_json_never_panics() {
        crate::hostile::each(LLDPCLI.as_bytes(), |b| {
            let _ = parse_lldpcli(&String::from_utf8_lossy(b), None);
        });
    }

    const NETWORKCTL: &str = "\
LINK   SYSTEM NAME   SYSTEM DESCRIPTION  CAPS        PORT ID   PORT DESCRIPTION
eth0   sw-example-01 Example Switch 2400 ..b........ Gi1/0/12  Room 2.14

Capability Flags:
o - Other; p - Repeater;  b - Bridge; w - WLAN Access Point; r - Router;

1 neighbors listed.
";

    #[test]
    fn networkctl_lldp_is_read() {
        let n = parse_networkctl(NETWORKCTL, Some("eth0")).expect("a neighbour");
        assert_eq!(n.switch.as_deref(), Some("sw-example-01"));
        assert_eq!(n.port.as_deref(), Some("Gi1/0/12"));
        assert_eq!(n.port_desc.as_deref(), Some("Room 2.14"));
        assert_eq!(n.model.as_deref(), Some("Example Switch 2400"));
        assert_eq!(parse_networkctl(NETWORKCTL, Some("eth1")), None);
        assert_eq!(
            parse_networkctl(
                "LINK CHASSIS ID SYSTEM NAME CAPS PORT ID PORT DESCRIPTION\n\n0 neighbors listed.\n",
                None
            ),
            None
        );
    }

    #[test]
    fn hostile_networkctl_text_never_panics() {
        crate::hostile::each(NETWORKCTL.as_bytes(), |b| {
            let _ = parse_networkctl(&String::from_utf8_lossy(b), None);
        });
        // Multibyte characters where the columns cut.
        let _ = parse_networkctl(
            "LINK   SYSTEM NAME   PORT ID\néééééééééééééééééééééééé\n",
            None,
        );
    }
}
