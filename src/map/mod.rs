//! Network map: one picture of what the app knows about a site. The sweep,
//! a UniFi backup and a SonicWall export merge into a tree rooted at the
//! internet, in the spirit of /mapnetwork: roles as colours, infrastructure
//! first, clients only on request, and findings drawn from evidence. A link
//! the data doesn't show (a sweep can't see switch ports) is drawn as
//! assumed, never as fact.

pub mod import;
pub mod layout;
pub mod runs;
pub mod svg;
pub mod view;

use crate::net::mac::{self, Mac};
use crate::theme::Theme;
use eframe::egui::Color32;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::Ipv4Addr;

/// Most cards on one map, as /mapnetwork: past this it stops being a map.
pub const BUDGET: usize = 120;
/// Clients shown under one switch or AP before the rest fold into a card.
const PER_UPLINK: usize = 10;

/// Role, which is the map's only colour channel.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Default)]
pub enum Zone {
    Internet,
    Gateway,
    Switch,
    Wireless,
    /// Network gear the sweep found by vendor or management port.
    Gear,
    Server,
    /// Printers and cameras.
    Device,
    Client,
    #[default]
    Unknown,
}

impl Zone {
    pub const ALL: [Zone; 9] = [
        Zone::Internet,
        Zone::Gateway,
        Zone::Switch,
        Zone::Wireless,
        Zone::Gear,
        Zone::Server,
        Zone::Device,
        Zone::Client,
        Zone::Unknown,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Zone::Internet => "Internet",
            Zone::Gateway => "Gateway / firewall",
            Zone::Switch => "Switches",
            Zone::Wireless => "Access points",
            Zone::Gear => "Network gear (inferred)",
            Zone::Server => "Servers / NAS",
            Zone::Device => "Printers / cameras",
            Zone::Client => "Clients",
            Zone::Unknown => "Unidentified",
        }
    }

    /// (dark, light) accent: /mapnetwork's palette, plus teal and slate.
    pub fn rgb(self, dark: bool) -> [u8; 3] {
        let (d, l) = match self {
            Zone::Internet => ([150, 160, 178], [84, 96, 116]),
            Zone::Gateway => ([0x5b, 0x9d, 0xff], [0x25, 0x63, 0xc9]),
            Zone::Switch => ([0xa9, 0x8b, 0xff], [0x6d, 0x44, 0xd9]),
            Zone::Wireless => ([0x5f, 0xcf, 0x8a], [0x1c, 0x9b, 0x54]),
            Zone::Gear => ([0x4f, 0xc3, 0xd9], [0x1f, 0x8a, 0xa0]),
            Zone::Server => ([0xe0, 0xb3, 0x4d], [0xa9, 0x79, 0x0c]),
            Zone::Device => ([0xf0, 0x94, 0x4a], [0xcf, 0x5d, 0x18]),
            Zone::Client => ([0x8f, 0xa8, 0xc8], [0x47, 0x65, 0x8c]),
            Zone::Unknown => ([0x8a, 0x93, 0xa3], [0x5d, 0x68, 0x78]),
        };
        if dark { d } else { l }
    }

    /// The five roles a theme menu shows as swatches, in /mapnetwork's
    /// zone order (core, server, trusted, iot, guest).
    pub const SWATCH: [Zone; 5] = [
        Zone::Gateway,
        Zone::Switch,
        Zone::Wireless,
        Zone::Device,
        Zone::Server,
    ];

    /// Its colour in a theme: /mapnetwork's zone colours for that theme
    /// (Miami's own neons), with gear, clients and the rest to match.
    pub fn colour_in(self, t: Theme) -> Color32 {
        // Internet, Gateway, Switch, Wireless, Gear, Server, Device, Client, Unknown.
        let hex: [u32; 9] = match t {
            Theme::Dark | Theme::Light => {
                let [r, g, b] = self.rgb(!t.is_light());
                return Color32::from_rgb(r, g, b);
            }
            Theme::Miami => [
                0xbeaae6, 0xff48aa, 0xb278ff, 0x28e2dc, 0x5ab4ff, 0xff9646, 0xffdc5a, 0x9aa0f0,
                0x9682b4,
            ],
            Theme::Slate => [
                0x8a94a8, 0x7aa7ff, 0xb8a2ff, 0x72d49c, 0x6fd3e0, 0xe6c36a, 0xf2a566, 0x9fb0cc,
                0x7f8899,
            ],
            Theme::Phosphor => [
                0x6f9c7c, 0x4fd1ff, 0xc792ff, 0x5cf28a, 0x3ff0d0, 0xf2e05c, 0xffb454, 0x8fd4a2,
                0x5f8c6c,
            ],
            Theme::Blueprint => [
                0x8fb0d6, 0x8cc4ff, 0xc7b3ff, 0x8ee6b0, 0x7fe0ea, 0xffe08a, 0xffbd7a, 0xc3d8f0,
                0x7c9dc4,
            ],
            Theme::Contrast => [
                0xd0d0d0, 0x6cb6ff, 0xd2a8ff, 0x56f08a, 0x4fe0f0, 0xffe066, 0xffa657, 0xffffff,
                0xb0b0b0,
            ],
            Theme::Rgb => [
                0xa58cd6, 0x00e5ff, 0xb14dff, 0x39ff14, 0xff00c8, 0xfff200, 0xff8a00, 0x7a9cff,
                0x8a6fc0,
            ],
            Theme::Paper => [
                0x6a5f4c, 0x2f5fb3, 0x7046c2, 0x2b7d4b, 0x1f7f8a, 0x94700f, 0xb8561c, 0x5a6f8c,
                0x8a7f6c,
            ],
        };
        let i = Zone::ALL.iter().position(|z| *z == self).unwrap_or(0);
        let h = hex[i];
        Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }
}

/// How a card hangs off its parent.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Link {
    /// Internet to the gateway.
    Backbone,
    Wired,
    Wireless,
    /// Not in the data: placed under the gateway because it answered there.
    Assumed,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Sev {
    High,
    Med,
    Low,
}

impl Sev {
    pub fn label(self) -> &'static str {
        match self {
            Sev::High => "high",
            Sev::Med => "med",
            Sev::Low => "low",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    /// Stable across rebuilds, so positions and the selection survive.
    pub id: String,
    pub zone: Zone,
    /// Small caps line at the top of the card, at most 18 characters.
    pub kind: String,
    pub label: String,
    pub sub: String,
    pub chips: Vec<String>,
    pub details: Vec<String>,
    pub flags: Vec<String>,
    pub ip: Option<Ipv4Addr>,
    pub mac: Option<Mac>,
    /// An end device: drawn only when clients are on.
    pub client: bool,
    pub seen_by: Vec<&'static str>,
    pub open: Vec<u16>,
    /// Parent, link and a label for the link (the parent's port). None only
    /// for the internet.
    pub up: Option<(usize, Link, Option<String>)>,
    /// Ranks clients for the per-uplink cap: lower shows first.
    rank: (u8, u64),
    /// The site it's at, when the source has sites (/mapnetwork maps).
    pub site: Option<String>,
    /// Pinned, then missing from the latest data: kept on the map, marked,
    /// for the user to review.
    pub gone: bool,
}

impl Node {
    pub fn parent(&self) -> Option<usize> {
        self.up.as_ref().map(|u| u.0)
    }

    pub fn link(&self) -> Option<Link> {
        self.up.as_ref().map(|u| u.1)
    }

    fn seen(&mut self, by: &'static str) {
        if !self.seen_by.contains(&by) {
            self.seen_by.push(by);
        }
    }

    fn chip(&mut self, c: impl Into<String>) {
        let c = c.into();
        if !self.chips.contains(&c) {
            self.chips.push(c);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub sev: Sev,
    pub text: String,
    pub nodes: Vec<usize>,
}

#[derive(Default)]
pub struct Map {
    pub nodes: Vec<Node>,
    pub findings: Vec<Finding>,
    /// What the map was built from, for under the title.
    pub summary: String,
    /// Clients known but not drawn.
    pub hidden: usize,
    /// Ids of the cards left out to keep the map readable (clients off, or
    /// past the per-uplink cap): still in the data, just not drawn.
    pub folded: HashSet<String>,
}

impl Map {
    pub fn children(&self) -> Vec<Vec<usize>> {
        let mut c = vec![Vec::new(); self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            if let Some(p) = n.parent() {
                c[p].push(i);
            }
        }
        c
    }

    /// Every site named on a card, in the order first seen.
    pub fn sites(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for s in self.nodes.iter().filter_map(|n| n.site.as_ref()) {
            if !v.contains(s) {
                v.push(s.clone());
            }
        }
        v
    }

    /// Path from the internet down to `i`.
    pub fn path(&self, mut i: usize) -> Vec<usize> {
        let mut p = vec![i];
        while let Some(u) = self.nodes.get(i).and_then(Node::parent) {
            if p.contains(&u) {
                break;
            }
            p.push(u);
            i = u;
        }
        p.reverse();
        p
    }
}

/// A host from the sweep.
#[derive(Clone, Debug)]
pub struct Seen {
    pub ip: Ipv4Addr,
    pub mac: Option<Mac>,
    pub name: Option<String>,
    /// None: answered ARP, not ping.
    pub rtt: Option<f64>,
    pub open: Vec<u16>,
}

#[derive(Default)]
pub struct Input<'a> {
    pub sweep: Vec<Seen>,
    /// The swept range, once a sweep has finished.
    pub range: Option<String>,
    pub ports_checked: bool,
    pub gateway: Option<Ipv4Addr>,
    pub this: Option<Ipv4Addr>,
    pub wan: Option<Ipv4Addr>,
    pub unifi: Option<&'a str>,
    pub sonicwall: Option<&'a str>,
    /// A /mapnetwork map: when loaded, it is the map.
    pub mapnetwork: Option<&'a crate::mapnetwork::Doc>,
    pub clients: bool,
    /// Every client gets a card: no per-uplink cap, no budget.
    pub all_clients: bool,
    /// Ids of uplinks whose clients are all shown ("Show all" on a
    /// +N more card), whatever the cap.
    pub open: Option<&'a HashSet<String>>,
}

impl Input<'_> {
    pub fn is_empty(&self) -> bool {
        self.sweep.is_empty()
            && self.unifi.is_none()
            && self.sonicwall.is_none()
            && self.mapnetwork.is_none()
    }
}

// ---------------------------------------------------------------- sources

struct UDevice {
    mac: Mac,
    name: String,
    kind: String,
    model: String,
    ip: Option<Ipv4Addr>,
    version: Option<String>,
    serial: Option<String>,
    eol: bool,
    disabled: bool,
    ports: usize,
    /// (uplink MAC, port on the uplink, wireless).
    up: Option<(Mac, Option<i64>, bool)>,
}

struct UClient {
    mac: Mac,
    name: Option<String>,
    oui: Option<String>,
    ip: Option<Ipv4Addr>,
    wired: bool,
    guest: bool,
    blocked: bool,
    up: Option<(Mac, Option<i64>)>,
    last_seen: i64,
}

#[derive(Default)]
struct Unifi {
    devices: Vec<UDevice>,
    clients: Vec<UClient>,
    /// "VLAN 10 Voice" lines.
    networks: Vec<String>,
    sites: Vec<String>,
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn ipv4(v: &Value, k: &str) -> Option<Ipv4Addr> {
    text(v, k)?.parse().ok()
}

/// mongoexport-style array from Config Export: `{"__cmd":"select"}` markers
/// followed by that collection's documents.
fn parse_unifi(json: &str) -> Unifi {
    let mut u = Unifi::default();
    let Ok(Value::Array(docs)) = serde_json::from_str::<Value>(json) else {
        return u;
    };
    let mut coll = String::new();
    for d in &docs {
        if d.get("__cmd").and_then(Value::as_str) == Some("select") {
            coll = text(d, "collection").unwrap_or_default();
            continue;
        }
        match coll.as_str() {
            "site" => u.sites.extend(text(d, "desc").or_else(|| text(d, "name"))),
            "device" => {
                let Some(m) = text(d, "mac").and_then(|m| mac::parse_mac(&m)) else {
                    continue;
                };
                let kind = text(d, "type").unwrap_or_else(|| "device".into());
                let up = d.get("last_uplink").and_then(|l| {
                    let um = mac::parse_mac(&text(l, "uplink_mac")?)?;
                    Some((
                        um,
                        l.get("uplink_remote_port").and_then(Value::as_i64),
                        text(l, "type").as_deref() == Some("wireless"),
                    ))
                });
                u.devices.push(UDevice {
                    mac: m,
                    // UniFi leaves some names unset (mesh APs): type + MAC tail.
                    name: text(d, "name")
                        .unwrap_or_else(|| format!("{kind}-{}", &mac::fmt(&m)[12..])),
                    model: text(d, "model").unwrap_or_default(),
                    kind,
                    ip: ipv4(d, "ip"),
                    version: text(d, "version"),
                    serial: text(d, "serial"),
                    eol: d.get("model_in_eol").and_then(Value::as_bool) == Some(true),
                    disabled: d.get("disabled").and_then(Value::as_bool) == Some(true),
                    ports: d
                        .get("port_table")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len),
                    up,
                });
            }
            "user" => {
                let Some(m) = text(d, "mac").and_then(|m| mac::parse_mac(&m)) else {
                    continue;
                };
                let up = text(d, "last_uplink_mac")
                    .and_then(|um| mac::parse_mac(&um))
                    .map(|um| (um, d.get("last_uplink_remote_port").and_then(Value::as_i64)));
                u.clients.push(UClient {
                    mac: m,
                    name: text(d, "name").or_else(|| text(d, "hostname")),
                    oui: text(d, "oui"),
                    ip: ipv4(d, "fixed_ip").or_else(|| ipv4(d, "last_ip")),
                    wired: d.get("is_wired").and_then(Value::as_bool) == Some(true),
                    guest: d.get("is_guest").and_then(Value::as_bool) == Some(true),
                    blocked: d.get("blocked").and_then(Value::as_bool) == Some(true),
                    up,
                    last_seen: d.get("last_seen").and_then(Value::as_i64).unwrap_or(0),
                });
            }
            "networkconf" => {
                let name = text(d, "name").unwrap_or_default();
                let line = match (d.get("vlan").and_then(Value::as_i64), text(d, "ip_subnet")) {
                    (Some(v), Some(s)) => format!("VLAN {v} {name} ({s})"),
                    (Some(v), None) => format!("VLAN {v} {name}"),
                    (None, Some(s)) => format!("{name} ({s}, untagged)"),
                    (None, None) => continue,
                };
                u.networks.push(line);
            }
            _ => {}
        }
    }
    u
}

struct Iface {
    name: String,
    zone: String,
    ip: Option<Ipv4Addr>,
    mask: Option<String>,
    mac: Option<Mac>,
    mgmt: Vec<String>,
}

/// A device the firewall heard on one of its ports (LLDP or LLTD).
struct Heard {
    iface: String,
    /// LLTD: a Windows host. LLDP: network gear, which names its port.
    lltd: bool,
    mac: Option<Mac>,
    ip: Option<Ipv4Addr>,
    name: Option<String>,
    description: Option<String>,
    port: Option<String>,
    caps: String,
}

#[derive(Default)]
struct Sonic {
    model: Option<String>,
    firmware: Option<String>,
    serial: Option<String>,
    name: Option<String>,
    ifaces: Vec<Iface>,
    heard: Vec<Heard>,
}

fn parse_sonicwall(json: &str) -> Option<Sonic> {
    let v: Value = serde_json::from_str(json).ok()?;
    let mut s = Sonic::default();
    for n in v
        .get("neighbors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        s.heard.push(Heard {
            iface: text(n, "interface").unwrap_or_default(),
            lltd: text(n, "protocol").as_deref() == Some("LLTD"),
            mac: text(n, "mac").and_then(|m| mac::parse_mac(&m)),
            ip: ipv4(n, "ip"),
            name: text(n, "name"),
            description: text(n, "description"),
            port: text(n, "port"),
            caps: text(n, "capabilities").unwrap_or_default(),
        });
    }
    if let Some(sys) = v.get("system").and_then(Value::as_object) {
        for (k, val) in sys {
            let (k, val) = (k.to_ascii_lowercase(), val.as_str().unwrap_or_default());
            if val.is_empty() {
                continue;
            }
            let slot = if k.contains("serial") {
                &mut s.serial
            } else if k.contains("model") {
                &mut s.model
            } else if k.contains("firmware") || k.contains("romversion") {
                &mut s.firmware
            } else if k.contains("firewallname") || k.contains("prodname") {
                &mut s.name
            } else {
                continue;
            };
            slot.get_or_insert_with(|| val.to_string());
        }
    }
    for i in v.get("interfaces").and_then(Value::as_array)? {
        s.ifaces.push(Iface {
            name: text(i, "name").unwrap_or_default(),
            zone: text(i, "zone").unwrap_or_default(),
            ip: ipv4(i, "ip").filter(|ip| !ip.is_unspecified()),
            mask: text(i, "mask"),
            mac: text(i, "mac").and_then(|m| mac::parse_mac(&m)),
            mgmt: i
                .get("management")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    Some(s)
}

// ---------------------------------------------------------- classification

const GEAR: &[(&str, &str)] = &[
    ("ubiquiti", "UBIQUITI"),
    ("meraki", "MERAKI"),
    ("cisco", "CISCO"),
    ("aruba", "ARUBA"),
    ("hewlett packard enterprise", "HPE"),
    ("juniper", "JUNIPER"),
    ("routerboard", "MIKROTIK"),
    ("mikrotik", "MIKROTIK"),
    ("fortinet", "FORTINET"),
    ("sonicwall", "SONICWALL"),
    ("ruckus", "RUCKUS"),
    ("cambium", "CAMBIUM"),
    ("extreme networks", "EXTREME"),
    ("brocade", "BROCADE"),
    ("zyxel", "ZYXEL"),
    ("draytek", "DRAYTEK"),
    ("watchguard", "WATCHGUARD"),
    ("palo alto", "PALO ALTO"),
    ("sophos", "SOPHOS"),
    ("allied telesis", "ALLIED TELESIS"),
    ("netgear", "NETGEAR"),
    ("tp-link", "TP-LINK"),
    ("d-link", "D-LINK"),
    ("engenius", "ENGENIUS"),
    ("peplink", "PEPLINK"),
    ("cradlepoint", "CRADLEPOINT"),
    ("arista", "ARISTA"),
];
const NAS: &[&str] = &[
    "synology",
    "qnap",
    "asustor",
    "buffalo",
    "western digital",
    "drobo",
    "terramaster",
    "netapp",
];
const PRINTERS: &[&str] = &[
    "brother", "canon", "epson", "lexmark", "xerox", "kyocera", "ricoh", "konica", "zebra",
];
const CAMERAS: &[&str] = &[
    "hikvision",
    "dahua",
    "axis comm",
    "amcrest",
    "reolink",
    "hanwha",
    "uniview",
    "vivotek",
    "avigilon",
    "mobotix",
    "verkada",
];

fn has(vendor: Option<&str>, words: &[&str]) -> bool {
    let v = vendor.unwrap_or_default().to_ascii_lowercase();
    words.iter().any(|w| v.contains(w))
}

fn gear_brand(vendor: Option<&str>) -> Option<&'static str> {
    let v = vendor?.to_ascii_lowercase();
    GEAR.iter().find(|(k, _)| v.contains(k)).map(|g| g.1)
}

/// First word of a registry name, for a card's kind line.
fn short_vendor(v: &str) -> String {
    v.split([' ', ',', '.'])
        .find(|w| !w.is_empty())
        .unwrap_or(v)
        .to_uppercase()
}

/// Upper case, at most 18 characters, cut at a word when one is close.
fn caps(s: &str) -> String {
    let s = s.trim().to_uppercase();
    let Some((i, next)) = s.char_indices().nth(18) else {
        return s;
    };
    let head = &s[..i];
    match head.rfind(' ') {
        Some(sp) if next != ' ' && sp >= 8 => head[..sp].to_string(),
        _ => head.trim_end().to_string(),
    }
}

/// Role of a host the sweep found and nothing else describes.
fn classify(vendor: Option<&str>, private: bool, open: &[u16]) -> (Zone, String, bool) {
    let p = |x| open.contains(&x);
    if let Some(b) = gear_brand(vendor) {
        return (Zone::Gear, caps(&format!("{b} DEVICE")), false);
    }
    if p(23) || p(8291) || p(541) || p(4433) {
        return (Zone::Gear, "NETWORK DEVICE".into(), false);
    }
    if has(vendor, NAS) || p(5000) {
        let k = vendor.map_or("NAS".into(), |v| caps(&format!("{} NAS", short_vendor(v))));
        return (Zone::Server, k, false);
    }
    if has(vendor, PRINTERS) || p(9100) {
        return (Zone::Device, "PRINTER".into(), true);
    }
    if has(vendor, CAMERAS) || p(554) {
        return (Zone::Device, "CAMERA".into(), true);
    }
    let kind = match vendor {
        Some(v) => caps(&short_vendor(v)),
        None if private => "PRIVATE MAC".into(),
        None => "HOST".into(),
    };
    (Zone::Client, kind, true)
}

/// Role of a device the firewall heard, from its LLDP capabilities and
/// description; an LLTD talker is a Windows host.
fn heard_role(h: &Heard) -> (Zone, String) {
    if h.lltd {
        return (Zone::Client, "WINDOWS HOST".into());
    }
    let can = h.caps.to_ascii_lowercase();
    let model = h
        .description
        .as_deref()
        .and_then(|d| d.split([',', ';']).next())
        .map(str::trim)
        .filter(|m| !m.is_empty() && m.len() <= 24);
    let vendor = h.mac.as_ref().and_then(mac::vendor);
    let named = |word: &str| {
        caps(&match model {
            Some(m) => format!("{word} {m}"),
            None => word.to_string(),
        })
    };
    if can.contains("wlan") || can.contains("access point") {
        (Zone::Wireless, named("AP"))
    } else if can.contains("bridge") || can.contains("mac-relay") || can.contains("switch") {
        (Zone::Switch, named("SWITCH"))
    } else if can.contains("telephone") || can.contains("phone") {
        (Zone::Client, named("PHONE"))
    } else if can.contains("router") {
        (Zone::Gear, named("ROUTER"))
    } else {
        let word = vendor.map_or("NETWORK DEVICE".into(), |v| {
            format!("{} DEVICE", short_vendor(v))
        });
        (Zone::Gear, caps(&word))
    }
}

fn unifi_role(kind: &str, model: &str) -> (Zone, String) {
    let (zone, word) = match kind {
        "usw" => (Zone::Switch, "SWITCH"),
        "uap" => (Zone::Wireless, "AP"),
        "udb" => (Zone::Wireless, "BRIDGE"),
        k if is_unifi_gateway(k) => (Zone::Gateway, "GATEWAY"),
        _ => (Zone::Gear, "UNIFI"),
    };
    (zone, caps(&format!("{word} {model}")))
}

fn is_unifi_gateway(kind: &str) -> bool {
    matches!(kind, "ugw" | "udm" | "uxg" | "ucg" | "udr")
}

fn port_names(open: &[u16]) -> String {
    open.iter()
        .map(|p| format!("{p} {}", crate::net::ports::service_name(*p)))
        .collect::<Vec<_>>()
        .join(", ")
}

// ------------------------------------------------------------------- build

#[derive(Default)]
struct B {
    nodes: Vec<Node>,
    by_mac: HashMap<Mac, usize>,
    by_ip: HashMap<Ipv4Addr, usize>,
    /// (severity, text, node ids): resolved to indices once the set is final.
    findings: Vec<(Sev, String, Vec<String>)>,
}

impl B {
    fn add(&mut self, n: Node) -> usize {
        let i = self.nodes.len();
        if let Some(m) = n.mac {
            self.by_mac.entry(m).or_insert(i);
        }
        if let Some(ip) = n.ip {
            self.by_ip.entry(ip).or_insert(i);
        }
        self.nodes.push(n);
        i
    }

    fn find(&self, mac: Option<Mac>, ip: Option<Ipv4Addr>) -> Option<usize> {
        mac.and_then(|m| self.by_mac.get(&m))
            .or_else(|| ip.and_then(|i| self.by_ip.get(&i)))
            .copied()
    }

    fn finding(&mut self, sev: Sev, text: String, nodes: &[usize]) {
        let ids = nodes.iter().map(|&i| self.nodes[i].id.clone()).collect();
        self.findings.push((sev, text, ids));
    }

    /// Fold what the sweep saw into a card.
    fn merge_seen(&mut self, i: usize, s: &Seen) {
        let n = &mut self.nodes[i];
        n.seen("sweep");
        if n.ip.is_none() {
            n.ip = Some(s.ip);
            self.by_ip.entry(s.ip).or_insert(i);
        }
        if n.mac.is_none() {
            n.mac = s.mac;
        }
        if n.sub.is_empty() {
            n.sub = s.ip.to_string();
        }
        n.details.push(match s.rtt {
            Some(r) => format!("Answered ping in {}", crate::widgets::fmt_ms(r)),
            None => "Answered ARP, not ping (firewalled or ICMP off)".into(),
        });
        if s.rtt.is_none() {
            n.chip("ARP only");
        }
        if let Some(name) = &s.name {
            n.details.push(format!("DNS name {name}"));
        }
        if !s.open.is_empty() {
            n.details
                .push(format!("Open ports: {}", port_names(&s.open)));
            for p in &s.open {
                if !n.open.contains(p) {
                    n.open.push(*p);
                }
            }
        }
    }
}

pub fn build(input: &Input) -> Map {
    if let Some(doc) = input.mapnetwork {
        return import::build(doc, input);
    }
    let mut b = B::default();
    let unifi = input.unifi.map(parse_unifi);
    let sonic = input.sonicwall.and_then(parse_sonicwall);
    // A backup is often read far from its site. What this machine sees only
    // joins the map when the sweep found hardware the backup or export
    // knows by MAC: the same network, not a look-alike 192.168.1.x.
    let local =
        (unifi.is_none() && sonic.is_none()) || same_site(input, unifi.as_ref(), sonic.as_ref());
    let had_sweep = !input.sweep.is_empty();
    let away;
    let input = if local {
        input
    } else {
        away = Input {
            unifi: input.unifi,
            sonicwall: input.sonicwall,
            clients: input.clients,
            all_clients: input.all_clients,
            open: input.open,
            ..Default::default()
        };
        &away
    };
    let seen_mac: HashMap<Mac, &Seen> = input
        .sweep
        .iter()
        .filter_map(|s| Some((s.mac?, s)))
        .collect();

    let internet = b.add(Node {
        id: "internet".into(),
        zone: Zone::Internet,
        kind: "INTERNET".into(),
        label: "Internet".into(),
        sub: input.wan.map(|w| format!("WAN {w}")).unwrap_or_default(),
        details: input
            .wan
            .map(|w| format!("Public address {w}, as seen from this machine"))
            .into_iter()
            .collect(),
        ..Default::default()
    });

    // The gateway: the firewall's own config beats a UniFi gateway, which
    // beats the default route of this machine.
    let mut gw = None;
    if let Some(s) = &sonic {
        let lan = s
            .ifaces
            .iter()
            .find(|f| f.ip.is_some() && f.ip == input.gateway)
            .or_else(|| {
                s.ifaces
                    .iter()
                    .find(|f| f.ip.is_some() && f.zone.eq_ignore_ascii_case("lan"))
            });
        let mut n = Node {
            id: "gateway".into(),
            zone: Zone::Gateway,
            kind: caps(&format!("SONICWALL {}", s.model.as_deref().unwrap_or(""))),
            label: s
                .name
                .clone()
                .or_else(|| s.model.clone())
                .unwrap_or_else(|| "SonicWall".into()),
            sub: lan
                .and_then(|f| f.ip)
                .map(|i| i.to_string())
                .unwrap_or_default(),
            ip: lan.and_then(|f| f.ip),
            mac: lan.and_then(|f| f.mac),
            seen_by: vec!["SonicWall"],
            up: Some((internet, Link::Backbone, None)),
            ..Default::default()
        };
        for (k, v) in [
            ("Model", &s.model),
            ("Firmware", &s.firmware),
            ("Serial", &s.serial),
        ] {
            if let Some(v) = v {
                n.details.push(format!("{k} {v}"));
            }
        }
        for f in &s.ifaces {
            let mut l = format!("{} {}", f.name, f.zone);
            if let Some(ip) = f.ip {
                l += &format!(" {ip}");
                if let Some(m) = &f.mask {
                    l += &format!(" / {m}");
                }
            }
            if !f.mgmt.is_empty() {
                l += &format!(" (mgmt: {})", f.mgmt.join(", "));
            }
            n.details.push(l);
        }
        let i = b.add(n);
        for f in &s.ifaces {
            if let Some(m) = f.mac {
                b.by_mac.entry(m).or_insert(i);
            }
            if let Some(ip) = f.ip {
                b.by_ip.entry(ip).or_insert(i);
            }
        }
        for f in &s.ifaces {
            let open: Vec<&str> = f
                .mgmt
                .iter()
                .map(String::as_str)
                .filter(|m| matches!(*m, "http" | "https" | "ssh"))
                .collect();
            if f.zone.to_ascii_lowercase().contains("wan") && !open.is_empty() {
                b.finding(
                    Sev::High,
                    format!(
                        "{} ({}) allows {} management from the internet: limit it to LAN or VPN.",
                        f.name,
                        f.zone,
                        open.join("/").to_uppercase()
                    ),
                    &[i],
                );
                b.nodes[i].flags.push(format!(
                    "{} management on {}",
                    open.join("/").to_uppercase(),
                    f.name
                ));
            }
        }
        gw = Some(i);
    }

    // UniFi devices.
    let mut device_idx: Vec<usize> = Vec::new();
    if let Some(u) = &unifi {
        for d in &u.devices {
            let (zone, kind) = unifi_role(&d.kind, &d.model);
            let mut n = Node {
                id: format!("mac:{}", mac::fmt(&d.mac)),
                zone,
                kind,
                label: d.name.clone(),
                sub: d.ip.map(|i| i.to_string()).unwrap_or_default(),
                ip: d.ip,
                mac: Some(d.mac),
                seen_by: vec!["UniFi"],
                ..Default::default()
            };
            n.details.push(format!("Model {}", d.model));
            n.details.push(format!("MAC {}", mac::fmt(&d.mac)));
            if let Some(v) = &d.version {
                n.details.push(format!("Firmware {v}"));
            }
            if let Some(s) = &d.serial {
                n.details.push(format!("Serial {s}"));
            }
            if d.ports > 0 {
                n.chip(format!("{} ports", d.ports));
            }
            if d.eol {
                n.flags.push("End-of-life model".into());
            }
            if d.disabled {
                n.flags.push("Disabled in the controller".into());
            }
            let gateway_type = is_unifi_gateway(&d.kind);
            let i = b.add(n);
            if gateway_type && gw.is_none() {
                b.nodes[i].up = Some((internet, Link::Backbone, None));
                b.nodes[i].id = "gateway".into();
                gw = Some(i);
            }
            device_idx.push(i);
        }
        if !u.networks.is_empty()
            && let Some(g) = gw
        {
            b.nodes[g].details.push("Networks (UniFi):".into());
            b.nodes[g]
                .details
                .extend(u.networks.iter().map(|n| format!("  {n}")));
        }
    }

    // Still none: this machine's default route, or a placeholder holding
    // the tree when there's data but no gateway in it.
    let gw = match gw {
        Some(g) => g,
        None if input.gateway.is_some() || unifi.is_some() => b.add(Node {
            id: "gateway".into(),
            zone: Zone::Gateway,
            kind: "GATEWAY".into(),
            label: if input.gateway.is_some() {
                "Gateway".into()
            } else {
                "Gateway (not in the data)".into()
            },
            sub: input.gateway.map(|g| g.to_string()).unwrap_or_default(),
            ip: input.gateway,
            details: input
                .gateway
                .map(|g| format!("Default gateway of this machine: {g}"))
                .into_iter()
                .collect(),
            up: Some((internet, Link::Backbone, None)),
            ..Default::default()
        }),
        None => internet,
    };
    if let Some(m) = input
        .gateway
        .and_then(|g| input.sweep.iter().find(|s| s.ip == g).and_then(|s| s.mac))
    {
        b.by_mac.entry(m).or_insert(gw);
        if b.nodes[gw].mac.is_none() {
            b.nodes[gw].mac = Some(m);
        }
    }

    // UniFi uplinks.
    if let Some(u) = &unifi {
        let resolved: Vec<Option<usize>> = u
            .devices
            .iter()
            .map(|d| d.up.and_then(|(m, ..)| b.by_mac.get(&m).copied()))
            .collect();
        let children = |p: usize| resolved.iter().filter(|r| **r == Some(p)).count();
        let mut cut = HashSet::new();
        for a in 0..u.devices.len() {
            let (ia, Some(ib)) = (device_idx[a], resolved[a]) else {
                continue;
            };
            // Each other's uplink: one record is stale, or there's a loop.
            if let Some(bpos) = device_idx.iter().position(|&x| x == ib)
                && resolved[bpos] == Some(ia)
                && ia < ib
            {
                // The one with more below it goes on top and loses its uplink.
                let top = if children(ia) >= children(ib) { ia } else { ib };
                cut.insert(top);
                b.finding(
                    Sev::Med,
                    format!(
                        "UniFi lists {} and {} as each other's uplink: one record is stale or there's a loop. Drawn with {} on top; check the uplink port on both.",
                        b.nodes[ia].label, b.nodes[ib].label, b.nodes[top].label
                    ),
                    &[ia, ib],
                );
            }
        }
        let mut unresolved: BTreeMap<Mac, Vec<usize>> = BTreeMap::new();
        for (a, d) in u.devices.iter().enumerate() {
            let i = device_idx[a];
            if i == gw || cut.contains(&i) {
                continue;
            }
            let Some((um, port, wireless)) = d.up else {
                continue;
            };
            let link = if wireless {
                Link::Wireless
            } else {
                Link::Wired
            };
            let label = port.filter(|p| *p > 0).map(|p| format!("port {p}"));
            match resolved[a] {
                Some(p) => b.nodes[i].up = Some((p, link, label)),
                None => {
                    unresolved.entry(um).or_default().push(i);
                    b.nodes[i].up = Some((usize::MAX, link, label));
                }
            }
        }
        // An uplink MAC nothing here manages: a real device we can't name.
        for (um, kids) in unresolved {
            let s = seen_mac.get(&um);
            let vendor = mac::vendor(&um);
            let mut n = Node {
                id: format!("mac:{}", mac::fmt(&um)),
                zone: Zone::Unknown,
                kind: vendor.map_or("UNKNOWN DEVICE".into(), |v| {
                    caps(&format!("{} DEVICE", short_vendor(v)))
                }),
                label: s
                    .and_then(|s| s.name.clone())
                    .unwrap_or_else(|| "Unidentified upstream device".into()),
                sub: s.map_or_else(|| mac::fmt(&um), |s| s.ip.to_string()),
                ip: s.map(|s| s.ip),
                mac: Some(um),
                up: Some((gw, Link::Assumed, None)),
                ..Default::default()
            };
            n.chip("not managed");
            n.details.push(format!("MAC {}", mac::fmt(&um)));
            n.details.push(match vendor {
                Some(v) => format!("Vendor {v} (MAC prefix; confirm on site)"),
                None => "Vendor unknown (MAC prefix not registered)".into(),
            });
            let names: Vec<_> = kids.iter().map(|&k| b.nodes[k].label.clone()).collect();
            n.details.push(format!("Uplink of {}", names.join(", ")));
            let d = b.add(n);
            for &k in &kids {
                if let Some(up) = &mut b.nodes[k].up {
                    up.0 = d;
                }
            }
            b.finding(
                Sev::Med,
                format!(
                    "{} uplink{} to {}{}, which neither the UniFi backup nor the firewall export describes: identify it on site.",
                    names.join(", "),
                    if kids.len() == 1 { "s" } else { "" },
                    mac::fmt(&um),
                    vendor.map(|v| format!(" ({v})")).unwrap_or_default()
                ),
                &[d],
            );
        }
        // No uplink recorded, or cut from a pair: under the gateway.
        for &i in &device_idx {
            if b.nodes[i].up.is_none() {
                b.nodes[i].up = Some((gw, Link::Assumed, None));
                if !cut.contains(&i) {
                    b.finding(
                        Sev::Low,
                        format!(
                            "No uplink recorded for {}: drawn under the gateway.",
                            b.nodes[i].label
                        ),
                        &[i],
                    );
                }
            }
        }
        // EOL hardware and mixed firmware.
        let eol: Vec<usize> = u
            .devices
            .iter()
            .zip(&device_idx)
            .filter(|(d, _)| d.eol)
            .map(|(_, &i)| i)
            .collect();
        if !eol.is_empty() {
            let list: Vec<_> = eol
                .iter()
                .map(|&i| format!("{} ({})", b.nodes[i].label, b.nodes[i].kind))
                .collect();
            b.finding(
                Sev::Med,
                format!(
                    "End-of-life hardware: {}. Plan replacements; no more firmware fixes.",
                    list.join(", ")
                ),
                &eol,
            );
        }
        let mut fw: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for d in &u.devices {
            if let Some(v) = &d.version {
                fw.entry(&d.model).or_default().insert(v);
            }
        }
        for (model, versions) in fw.iter().filter(|(_, v)| v.len() > 1) {
            let nodes: Vec<usize> = u
                .devices
                .iter()
                .zip(&device_idx)
                .filter(|(d, _)| d.model == *model)
                .map(|(_, &i)| i)
                .collect();
            b.finding(
                Sev::Low,
                format!(
                    "{model} units run {} firmware versions ({}): bring them level.",
                    versions.len(),
                    versions.iter().copied().collect::<Vec<_>>().join(", ")
                ),
                &nodes,
            );
        }
    }

    // What the firewall heard on its ports. LLDP never crosses a switch, so
    // an LLDP neighbour is plugged into that port: a real link, which beats
    // an assumed one. LLTD does cross switches: a Windows host is somewhere
    // on that port's network, so its link stays assumed.
    if let Some(s) = &sonic {
        for h in &s.heard {
            let at = match &h.port {
                Some(p) => format!("{}, its {p}", h.iface),
                None => h.iface.clone(),
            };
            let note = if h.lltd {
                format!(
                    "The firewall hears it on {}'s network (LLTD; the switch path isn't known)",
                    h.iface
                )
            } else {
                format!("Plugged into the firewall's {at} (LLDP)")
            };
            let link = if h.lltd { Link::Assumed } else { Link::Wired };
            match b.find(h.mac, h.ip) {
                Some(i) if i == gw || i == internet => {}
                Some(i) => {
                    let n = &mut b.nodes[i];
                    n.seen("firewall");
                    n.details.push(note);
                    if !h.lltd && n.up.as_ref().is_none_or(|u| u.1 == Link::Assumed) {
                        n.up = Some((gw, Link::Wired, Some(at)));
                        // Its uplink is known now.
                        let id = n.id.clone();
                        b.findings.retain(|(_, text, ids)| {
                            !(text.starts_with("No uplink recorded")
                                && ids.len() == 1
                                && ids[0] == id)
                        });
                    }
                }
                None => {
                    let (zone, kind) = heard_role(h);
                    let mut n = Node {
                        id: match (h.mac, h.ip) {
                            (Some(m), _) => format!("mac:{}", mac::fmt(&m)),
                            (None, Some(ip)) => format!("ip:{ip}"),
                            _ => continue,
                        },
                        zone,
                        kind,
                        label: h
                            .name
                            .clone()
                            .or_else(|| h.ip.map(|ip| ip.to_string()))
                            .or_else(|| h.mac.map(|m| mac::fmt(&m)))
                            .unwrap_or_default(),
                        sub: h.ip.map(|ip| ip.to_string()).unwrap_or_default(),
                        ip: h.ip,
                        mac: h.mac,
                        client: zone == Zone::Client,
                        rank: (1, h.ip.map_or(u64::MAX, |ip| u32::from(ip) as u64)),
                        up: Some((gw, link, (!h.lltd).then_some(at))),
                        ..Default::default()
                    };
                    n.seen("firewall");
                    n.details.push(note);
                    if let Some(d) = &h.description {
                        n.details.push(d.clone());
                    }
                    if let Some(m) = h.mac {
                        n.details.push(format!("MAC {}", mac::fmt(&m)));
                    }
                    b.add(n);
                }
            }
        }
    }

    // Sweep hosts.
    let unifi_client: HashSet<Mac> = unifi
        .as_ref()
        .map(|u| u.clients.iter().map(|c| c.mac).collect())
        .unwrap_or_default();
    let mut deferred = Vec::new();
    for s in &input.sweep {
        let is_gw = Some(s.ip) == input.gateway;
        let at = if is_gw {
            Some(gw)
        } else {
            b.find(s.mac, Some(s.ip))
        };
        if let Some(i) = at.filter(|&i| i != internet) {
            b.merge_seen(i, s);
            continue;
        }
        let this = Some(s.ip) == input.this;
        let vendor = s.mac.as_ref().and_then(mac::vendor);
        let private = s.mac.as_ref().is_some_and(mac::local_admin);
        let (zone, kind, client) = if this {
            (Zone::Client, "THIS MACHINE".into(), false)
        } else {
            classify(vendor, private, &s.open)
        };
        // A client UniFi knows becomes its card below, under its AP or switch.
        if client && s.mac.is_some_and(|m| unifi_client.contains(&m)) {
            deferred.push(s);
            continue;
        }
        let label = match (&s.name, this) {
            (_, true) => "This machine".into(),
            (Some(n), _) => n.split('.').next().unwrap_or(n).to_string(),
            (None, _) => match vendor {
                Some(v) => format!(
                    "{} device",
                    short_vendor(v).to_lowercase().replace_first_upper()
                ),
                None => format!("Host {}", s.ip),
            },
        };
        let mut n = Node {
            id: format!("ip:{}", s.ip),
            zone,
            kind,
            label,
            sub: String::new(),
            mac: s.mac,
            client,
            up: Some((gw, Link::Assumed, None)),
            rank: (if s.name.is_some() { 1 } else { 2 }, u32::from(s.ip) as u64),
            ..Default::default()
        };
        if let Some(m) = &s.mac {
            n.details.push(
                format!("MAC {} {}", mac::fmt(m), mac::vendor_label(m))
                    .trim_end()
                    .to_string(),
            );
        } else if !this {
            n.details
                .push("No MAC: past a router, or this machine".into());
        }
        if zone == Zone::Gear || zone == Zone::Server {
            n.chip("inferred");
            n.details
                .push("Role inferred from vendor and open ports".into());
        }
        let i = b.add(n);
        b.merge_seen(i, s);
    }
    if let Some(t) = input.this
        && !b.by_ip.contains_key(&t)
        && !input.is_empty()
    {
        b.add(Node {
            id: format!("ip:{t}"),
            zone: Zone::Client,
            kind: "THIS MACHINE".into(),
            label: "This machine".into(),
            sub: t.to_string(),
            ip: Some(t),
            up: Some((gw, Link::Assumed, None)),
            details: vec!["Where the sweep ran from".into()],
            ..Default::default()
        });
    }

    // UniFi clients: placed under the AP or switch they were last seen on.
    if let Some(u) = &unifi {
        for c in &u.clients {
            let parent = c.up.and_then(|(m, _)| b.by_mac.get(&m).copied());
            let link = match (parent, c.wired) {
                (None, _) => Link::Assumed,
                (Some(_), true) => Link::Wired,
                (Some(_), false) => Link::Wireless,
            };
            let port =
                c.up.and_then(|(_, p)| p)
                    .filter(|p| *p > 0 && c.wired)
                    .map(|p| format!("port {p}"));
            let up = Some((parent.unwrap_or(gw), link, port));
            if let Some(i) = b.find(Some(c.mac), None) {
                // Already a card (the sweep's): now we know where it plugs in.
                if i != gw && b.nodes[i].link() == Some(Link::Assumed) {
                    b.nodes[i].up = up;
                }
                b.nodes[i].seen("UniFi");
                continue;
            }
            let vendor = c.oui.as_deref();
            // Infrastructure hiding in the client table (an unadopted AP,
            // a NAS) is shown even with clients off.
            let (zone, kind, client) = match classify(vendor, false, &[]) {
                (Zone::Client, _, _) if vendor.is_none() => (Zone::Client, "CLIENT".into(), true),
                r => r,
            };
            let mut n = Node {
                id: format!("mac:{}", mac::fmt(&c.mac)),
                zone,
                kind,
                label: c.name.clone().unwrap_or_else(|| mac::fmt(&c.mac)),
                sub: c.ip.map(|i| i.to_string()).unwrap_or_default(),
                ip: c.ip,
                mac: Some(c.mac),
                client,
                seen_by: vec!["UniFi"],
                up,
                rank: (
                    if c.blocked {
                        0
                    } else if c.name.is_some() {
                        1
                    } else {
                        2
                    },
                    u64::MAX - c.last_seen.max(0) as u64,
                ),
                ..Default::default()
            };
            n.details.push(format!("MAC {}", mac::fmt(&c.mac)));
            if let Some(v) = vendor {
                n.details.push(format!("Vendor {v} (UniFi)"));
            }
            n.details
                .push(if c.wired { "Wired" } else { "Wireless" }.into());
            if c.guest {
                n.chip("guest");
            }
            if !client {
                n.chip("in client table");
            }
            if c.blocked {
                n.flags.push("Blocked in the controller".into());
            }
            let i = b.add(n);
            if let Some(s) = deferred.iter().find(|s| s.mac == Some(c.mac)) {
                b.merge_seen(i, s);
                b.nodes[i].rank.0 = b.nodes[i].rank.0.min(1);
            }
        }
    }

    // Findings the sweep's evidence supports.
    let telnet: Vec<usize> = (0..b.nodes.len())
        .filter(|&i| b.nodes[i].open.contains(&23))
        .collect();
    if !telnet.is_empty() {
        let list: Vec<_> = telnet.iter().map(|&i| describe(&b.nodes[i])).collect();
        b.finding(
            Sev::High,
            format!(
                "Telnet is open on {}: it sends logins in clear text. Turn it off; use SSH or HTTPS.",
                list.join(", ")
            ),
            &telnet,
        );
        for &i in &telnet {
            b.nodes[i].flags.push("Telnet open".into());
        }
    }
    let plain: Vec<usize> = (0..b.nodes.len())
        .filter(|&i| {
            let n = &b.nodes[i];
            matches!(
                n.zone,
                Zone::Gateway | Zone::Switch | Zone::Wireless | Zone::Gear
            ) && n.open.contains(&80)
                && ![443, 8443, 4433].iter().any(|p| n.open.contains(p))
        })
        .collect();
    if !plain.is_empty() {
        let list: Vec<_> = plain.iter().map(|&i| describe(&b.nodes[i])).collect();
        b.finding(
            Sev::Low,
            format!(
                "Management page only over plain HTTP on {}: enable HTTPS.",
                list.join(", ")
            ),
            &plain,
        );
    }
    if input.ports_checked {
        let rdp: Vec<usize> = (0..b.nodes.len())
            .filter(|&i| b.nodes[i].open.contains(&3389))
            .collect();
        if rdp.len() > 1 {
            b.finding(
                Sev::Low,
                format!(
                    "Remote Desktop answers on {} hosts: make sure it isn't forwarded from the internet and needs NLA.",
                    rdp.len()
                ),
                &rdp,
            );
        }
    }
    if let (Some(range), Some(_)) = (&input.range, &unifi) {
        let swept: HashSet<Ipv4Addr> = crate::net::targets::parse_range(range)
            .unwrap_or_default()
            .into_iter()
            .collect();
        let silent: Vec<usize> = device_idx
            .iter()
            .copied()
            .filter(|&i| {
                let n = &b.nodes[i];
                n.ip.is_some_and(|ip| swept.contains(&ip)) && !n.seen_by.contains(&"sweep")
            })
            .collect();
        if !silent.is_empty() {
            let list: Vec<_> = silent.iter().map(|&i| describe(&b.nodes[i])).collect();
            b.finding(
                Sev::Low,
                format!(
                    "In the swept range but no answer to ping or ARP: {}. Offline, or its address changed.",
                    list.join(", ")
                ),
                &silent,
            );
            for &i in &silent {
                b.nodes[i].chip("no answer");
            }
        }
        let stray: Vec<usize> = (0..b.nodes.len())
            .filter(|&i| {
                let n = &b.nodes[i];
                n.seen_by == ["sweep"]
                    && n.mac
                        .as_ref()
                        .and_then(mac::vendor)
                        .is_some_and(|v| v.to_ascii_lowercase().contains("ubiquiti"))
            })
            .collect();
        if !stray.is_empty() {
            let list: Vec<_> = stray.iter().map(|&i| describe(&b.nodes[i])).collect();
            b.finding(
                Sev::Low,
                format!(
                    "Ubiquiti hardware not in this backup: {}. Unadopted, or managed by another controller.",
                    list.join(", ")
                ),
                &stray,
            );
        }
    }

    let mut summary = summary(input, unifi.as_ref(), sonic.as_ref());
    if !local && had_sweep {
        summary += " · sweep not merged: it found no device the data knows (another network?)";
    } else if !local {
        summary += " · sweep here to add what this machine sees";
    }
    let mut map = limit(b, input);
    map.summary = summary;
    if map.hidden > 0 && !input.clients {
        map.summary += " · clients not mapped";
    }
    map
}

fn describe(n: &Node) -> String {
    match n.ip {
        Some(ip) if n.label != ip.to_string() => format!("{} ({ip})", n.label),
        _ => n.label.clone(),
    }
}

trait FirstUpper {
    fn replace_first_upper(&self) -> String;
}

impl FirstUpper for String {
    fn replace_first_upper(&self) -> String {
        let mut c = self.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default()
    }
}

fn same_site(input: &Input, unifi: Option<&Unifi>, sonic: Option<&Sonic>) -> bool {
    let mut macs: HashSet<Mac> = HashSet::new();
    if let Some(u) = unifi {
        macs.extend(u.devices.iter().map(|d| d.mac));
        macs.extend(u.devices.iter().filter_map(|d| d.up.map(|u| u.0)));
        macs.extend(u.clients.iter().map(|c| c.mac));
    }
    if let Some(s) = sonic {
        macs.extend(s.ifaces.iter().filter_map(|f| f.mac));
        macs.extend(s.heard.iter().filter_map(|h| h.mac));
    }
    input
        .sweep
        .iter()
        .any(|h| h.mac.is_some_and(|m| macs.contains(&m)))
}

fn summary(input: &Input, unifi: Option<&Unifi>, sonic: Option<&Sonic>) -> String {
    let mut parts = Vec::new();
    if let Some(u) = unifi {
        let site = u.sites.first().map(|s| format!(" {s}")).unwrap_or_default();
        parts.push(format!("UniFi{site} ({} devices)", u.devices.len()));
    }
    if let Some(s) = sonic {
        parts.push(format!(
            "SonicWall {}",
            s.model.as_deref().unwrap_or("export")
        ));
    }
    match &input.range {
        Some(r) => parts.push(format!("sweep of {r} ({} hosts)", input.sweep.len())),
        None if !input.sweep.is_empty() => {
            parts.push(format!("sweep running ({} so far)", input.sweep.len()))
        }
        None => {}
    }
    parts.join(" · ")
}

/// Clients off: fold them into a count on their parent. On: at most
/// PER_UPLINK under each parent and the rest in one card, tightening until
/// the map fits the budget; uplinks opened with Show all, or every one with
/// all clients, show them all.
fn limit(b: B, input: &Input) -> Map {
    let clients = input.clients || input.all_clients;
    let B {
        mut nodes,
        findings,
        ..
    } = b;
    let n = nodes.len();
    let open: HashSet<usize> = (0..n)
        .filter(|&p| {
            clients && (input.all_clients || input.open.is_some_and(|o| o.contains(&nodes[p].id)))
        })
        .collect();
    let mut by_parent: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, node) in nodes.iter().enumerate() {
        if node.client
            && let Some(p) = node.parent()
        {
            by_parent.entry(p).or_default().push(i);
        }
    }
    for kids in by_parent.values_mut() {
        kids.sort_by_key(|&i| nodes[i].rank);
    }
    let infra = n - by_parent.values().map(Vec::len).sum::<usize>();
    let mut cap = if clients { PER_UPLINK } else { 0 };
    let cap_of = |p: usize, cap: usize| if open.contains(&p) { usize::MAX } else { cap };
    let cost = |cap: usize| {
        infra
            + by_parent
                .iter()
                .map(|(&p, k)| {
                    let c = cap_of(p, cap);
                    k.len().min(c) + usize::from(k.len() > c && clients)
                })
                .sum::<usize>()
    };
    // Shown in full on request, so the budget only tightens the rest.
    while cap > 0 && cost(cap) > BUDGET && !input.all_clients {
        cap = match cap {
            c if c > 5 => 5,
            c if c > 3 => 3,
            c => c - 1,
        };
    }
    let mut keep = vec![true; n];
    let mut folded = HashSet::new();
    let mut extra = Vec::new();
    let mut hidden = 0;
    for (&p, kids) in &by_parent {
        let rest = &kids[kids.len().min(cap_of(p, cap))..];
        if rest.is_empty() {
            continue;
        }
        hidden += rest.len();
        for &i in rest {
            keep[i] = false;
            folded.insert(nodes[i].id.clone());
        }
        if clients && (cost(cap) <= BUDGET || open.contains(&p)) {
            let mut more = Node {
                id: format!("more:{}", nodes[p].id),
                zone: Zone::Client,
                kind: "MORE CLIENTS".into(),
                label: format!("+{} more", rest.len()),
                sub: format!("via {}", nodes[p].label),
                client: true,
                up: Some((p, Link::Assumed, None)),
                ..Default::default()
            };
            more.chips = vec!["collapsed".into(), format!("{} devices", rest.len())];
            more.details = rest
                .iter()
                .take(60)
                .map(|&i| {
                    let c = &nodes[i];
                    [c.label.as_str(), c.sub.as_str()]
                        .iter()
                        .filter(|s| !s.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .collect();
            if rest.len() > 60 {
                more.details.push(format!("…and {} more", rest.len() - 60));
            }
            extra.push(more);
            hidden -= rest.len();
        } else {
            let c = format!("{} clients", kids.len());
            nodes[p].chip(c);
        }
    }
    // Reindex.
    let mut map_to = vec![usize::MAX; n];
    let mut out = Vec::new();
    for (i, node) in nodes.into_iter().enumerate() {
        if keep[i] {
            map_to[i] = out.len();
            out.push(node);
        }
    }
    for node in &mut out {
        if let Some(up) = &mut node.up {
            up.0 = map_to[up.0];
        }
    }
    for mut more in extra {
        if let Some(up) = &mut more.up {
            up.0 = map_to[up.0];
        }
        out.push(more);
    }
    let index: HashMap<&str, usize> = out
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut findings: Vec<Finding> = findings
        .into_iter()
        .map(|(sev, text, ids)| Finding {
            sev,
            text,
            nodes: ids
                .iter()
                .filter_map(|id| index.get(id.as_str()).copied())
                .collect(),
        })
        .collect();
    findings.sort_by_key(|f| f.sev);
    Map {
        nodes: out,
        findings,
        summary: String::new(),
        hidden,
        folded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn unifi(devices: &[Value], users: &[Value]) -> String {
        let mut v = vec![
            json!({"__cmd": "select", "collection": "site"}),
            json!({"desc": "HQ"}),
        ];
        v.push(json!({"__cmd": "select", "collection": "device"}));
        v.extend(devices.iter().cloned());
        v.push(json!({"__cmd": "select", "collection": "networkconf"}));
        v.push(json!({"name": "Voice", "vlan": 10}));
        if !users.is_empty() {
            v.push(json!({"__cmd": "select", "collection": "user"}));
            v.extend(users.iter().cloned());
        }
        Value::Array(v).to_string()
    }

    fn dev(name: &str, kind: &str, mac: &str, ip: &str, up: Option<(&str, i64)>) -> Value {
        let mut d = json!({"name": name, "type": kind, "model": "M", "mac": mac, "ip": ip,
            "version": "1.0"});
        if let Some((m, p)) = up {
            d["last_uplink"] = json!({"uplink_mac": m, "uplink_remote_port": p, "type": "wire"});
        }
        d
    }

    fn by_label<'a>(m: &'a Map, l: &str) -> &'a Node {
        m.nodes.iter().find(|n| n.label == l).expect(l)
    }

    fn parent<'a>(m: &'a Map, l: &str) -> &'a str {
        &m.nodes[by_label(m, l).parent().expect("has parent")].label
    }

    #[test]
    fn unifi_tree_with_reciprocal_pair_and_unknown_uplink() {
        let j = unifi(
            &[
                dev(
                    "Core",
                    "usw",
                    "00:00:00:00:00:01",
                    "10.0.0.2",
                    Some(("00:00:00:00:00:02", 18)),
                ),
                dev(
                    "Annex",
                    "usw",
                    "00:00:00:00:00:02",
                    "10.0.0.3",
                    Some(("00:00:00:00:00:01", 51)),
                ),
                dev(
                    "AP1",
                    "uap",
                    "00:00:00:00:00:03",
                    "10.0.0.4",
                    Some(("00:00:00:00:00:01", 3)),
                ),
                dev(
                    "AP2",
                    "uap",
                    "00:00:00:00:00:04",
                    "10.0.0.5",
                    Some(("00:00:00:00:00:01", 4)),
                ),
                dev(
                    "Shop",
                    "usw",
                    "00:00:00:00:00:05",
                    "10.0.0.6",
                    Some(("e4:38:83:00:00:09", 1)),
                ),
            ],
            &[],
        );
        let m = build(&Input {
            unifi: Some(&j),
            gateway: Some("10.0.0.1".parse().unwrap()),
            ..Default::default()
        });
        assert_eq!(parent(&m, "Annex"), "Core", "more children wins the pair");
        assert_eq!(parent(&m, "Core"), "Gateway (not in the data)");
        assert_eq!(parent(&m, "AP1"), "Core");
        assert_eq!(
            by_label(&m, "AP1").up.as_ref().unwrap().2.as_deref(),
            Some("port 3")
        );
        assert_eq!(parent(&m, "Shop"), "Unidentified upstream device");
        assert_eq!(
            parent(&m, "Unidentified upstream device"),
            "Gateway (not in the data)"
        );
        assert!(
            m.findings
                .iter()
                .any(|f| f.text.contains("each other's uplink"))
        );
        assert!(
            m.findings
                .iter()
                .any(|f| f.text.contains("e4:38:83:00:00:09"))
        );
        // Every card reaches the internet.
        for i in 0..m.nodes.len() {
            assert_eq!(m.path(i)[0], 0, "{}", m.nodes[i].label);
        }
        assert!(m.summary.contains("UniFi HQ (5 devices)"));
    }

    #[test]
    fn sweep_only_merges_gateway_and_infers_roles() {
        let s = |ip: &str, mac: Option<&str>, open: &[u16]| Seen {
            ip: ip.parse().unwrap(),
            mac: mac.and_then(mac::parse_mac),
            name: None,
            rtt: Some(1.0),
            open: open.to_vec(),
        };
        let input = Input {
            sweep: vec![
                s("192.168.1.1", Some("02:00:00:00:00:01"), &[80, 443]),
                s("192.168.1.20", None, &[9100]),
                s("192.168.1.30", None, &[23, 80]),
                s("192.168.1.40", None, &[]),
                s("192.168.1.50", None, &[]),
            ],
            range: Some("192.168.1.0/24".into()),
            ports_checked: true,
            gateway: Some("192.168.1.1".parse().unwrap()),
            this: Some("192.168.1.50".parse().unwrap()),
            ..Default::default()
        };
        let m = build(&input);
        let gw = by_label(&m, "Gateway");
        assert_eq!(gw.seen_by, ["sweep"]);
        // Clients off: the printer and plain host fold into a count; this
        // machine and the telnet box stay.
        assert!(m.nodes.iter().all(|n| !n.client));
        assert!(by_label(&m, "This machine").up.is_some());
        assert!(gw.chips.iter().any(|c| c == "2 clients"), "{:?}", gw.chips);
        assert_eq!(m.hidden, 2);
        let gear = m
            .nodes
            .iter()
            .find(|n| n.open.contains(&23))
            .expect("telnet host");
        assert_eq!((gear.zone, gear.client), (Zone::Gear, false));
        assert_eq!(m.findings[0].sev, Sev::High);
        assert!(m.findings[0].text.contains("Telnet"));

        let m = build(&Input {
            clients: true,
            ..input
        });
        assert!(m.nodes.iter().any(|n| n.kind == "PRINTER"));
        assert_eq!(m.hidden, 0);
    }

    #[test]
    fn clients_cap_and_budget() {
        let mut users = Vec::new();
        for i in 0..300 {
            users.push(
                json!({"mac": format!("00:11:22:33:{:02x}:{:02x}", i / 256, i % 256),
                "hostname": format!("pc{i}"), "last_uplink_mac": "00:00:00:00:00:01",
                "is_wired": true, "last_seen": i}),
            );
        }
        let j = unifi(
            &[dev("Core", "usw", "00:00:00:00:00:01", "10.0.0.2", None)],
            &users,
        );
        let m = build(&Input {
            unifi: Some(&j),
            clients: true,
            ..Default::default()
        });
        let kids = m
            .nodes
            .iter()
            .filter(|n| n.client && n.kind != "MORE CLIENTS")
            .count();
        assert_eq!(kids, PER_UPLINK);
        let more = m.nodes.iter().find(|n| n.kind == "MORE CLIENTS").unwrap();
        assert_eq!(more.label, "+290 more");
        assert_eq!(by_label(&m, "pc299").kind, "CLIENT", "most recent first");
        assert!(m.nodes.len() <= BUDGET);

        // All: a card for every one of the 300, past the budget.
        let count = |m: &Map| {
            m.nodes
                .iter()
                .filter(|n| n.client && n.kind != "MORE CLIENTS")
                .count()
        };
        let all = build(&Input {
            unifi: Some(&j),
            clients: true,
            all_clients: true,
            ..Default::default()
        });
        assert_eq!(count(&all), 300);
        assert!(all.nodes.iter().all(|n| n.kind != "MORE CLIENTS"));
        assert!(all.folded.is_empty());
        // Show all on the +N more card opens just that uplink.
        let core = m.nodes[more.parent().unwrap()].id.clone();
        let open: HashSet<String> = [core].into();
        let opened = build(&Input {
            unifi: Some(&j),
            clients: true,
            open: Some(&open),
            ..Default::default()
        });
        assert_eq!(count(&opened), 300);
        assert!(m.folded.contains(&by_label(&all, "pc0").id));

        let off = build(&Input {
            unifi: Some(&j),
            ..Default::default()
        });
        assert!(
            by_label(&off, "Core")
                .chips
                .contains(&"300 clients".to_string())
        );
    }

    #[test]
    fn what_the_firewall_hears_goes_on_the_map() {
        let sw = json!({
            "system": {"model": "TZ 370"},
            "interfaces": [{"name": "X0", "zone": "LAN", "ip": "10.0.0.1", "mac": "18:b1:69:00:00:01"}],
            "neighbors": [
                {"interface": "X0", "protocol": "LLDP", "mac": "f0:9f:c2:00:00:09", "ip": "10.0.0.9",
                 "name": "Closet Switch", "description": "US-24-250W, 7.1.26", "port": "Port 24",
                 "capabilities": "2-Port-MAC-Relay"},
                {"interface": "X0", "protocol": "LLDP", "mac": "00:00:00:00:00:01", "ip": "10.0.0.2",
                 "name": "Core", "port": "Port 1", "capabilities": "Bridge"},
                {"interface": "X0", "protocol": "LLTD", "mac": "00:11:22:00:00:05", "ip": "10.0.0.165"}
            ]
        })
        .to_string();
        // Core is in the UniFi backup with no uplink recorded.
        let j = unifi(
            &[dev("Core", "usw", "00:00:00:00:00:01", "10.0.0.2", None)],
            &[],
        );
        let m = build(&Input {
            unifi: Some(&j),
            sonicwall: Some(&sw),
            clients: true,
            ..Default::default()
        });
        let gw = m.nodes[1].label.clone();
        let closet = by_label(&m, "Closet Switch");
        assert_eq!(closet.zone, Zone::Switch);
        assert_eq!(closet.kind, "SWITCH US-24-250W");
        assert_eq!(parent(&m, "Closet Switch"), gw);
        assert_eq!(
            closet.up.as_ref().map(|u| (u.1, u.2.clone())),
            Some((Link::Wired, Some("X0, its Port 24".into())))
        );
        // The backup's switch: its assumed uplink becomes the firewall port.
        let core = by_label(&m, "Core");
        assert_eq!(core.link(), Some(Link::Wired));
        assert!(core.seen_by.contains(&"firewall"));
        assert!(
            !m.findings
                .iter()
                .any(|f| f.text.starts_with("No uplink recorded"))
        );
        let host = by_label(&m, "10.0.0.165");
        assert!(host.client);
        assert_eq!(host.kind, "WINDOWS HOST");
        // LLTD crosses switches: where it plugs in isn't known.
        assert_eq!(host.link(), Some(Link::Assumed));
    }

    #[test]
    fn sonicwall_is_the_gateway_and_wan_management_is_flagged() {
        let sw = json!({
            "system": {"model": "TZ 470", "firmwareVersion": "7.0.1"},
            "interfaces": [
                {"name": "X0", "zone": "LAN", "ip": "10.0.0.1", "mask": "255.255.255.0",
                 "mac": "18:b1:69:00:00:01", "management": ["https", "ping"]},
                {"name": "X1", "zone": "WAN", "ip": "203.0.113.5", "mac": "18:b1:69:00:00:02",
                 "management": ["https", "ssh"]}
            ]
        })
        .to_string();
        let j = unifi(
            &[dev(
                "Core",
                "usw",
                "00:00:00:00:00:01",
                "10.0.0.2",
                Some(("18:b1:69:00:00:01", 3)),
            )],
            &[],
        );
        let m = build(&Input {
            unifi: Some(&j),
            sonicwall: Some(&sw),
            gateway: Some("10.0.0.1".parse().unwrap()),
            ..Default::default()
        });
        assert!(
            m.summary.contains("sweep here to add"),
            "no sweep: nothing local merged, and the subtitle says how to add it"
        );
        let gw = &m.nodes[1];
        assert_eq!(gw.kind, "SONICWALL TZ 470");
        assert_eq!(gw.sub, "10.0.0.1");
        assert_eq!(parent(&m, "Core"), gw.label);
        assert_eq!(by_label(&m, "Core").link().unwrap(), Link::Wired);
        assert_eq!(m.findings[0].sev, Sev::High);
        assert!(m.findings[0].text.contains("X1 (WAN) allows HTTPS/SSH"));
    }

    #[test]
    fn local_scan_merges_only_on_a_shared_mac() {
        let j = unifi(
            &[dev("Core", "usw", "00:00:00:00:00:01", "10.0.0.2", None)],
            &[],
        );
        let host = |mac: &str| Seen {
            ip: "192.168.1.7".parse().unwrap(),
            mac: mac::parse_mac(mac),
            name: None,
            rtt: Some(1.0),
            open: vec![],
        };
        let at_office = build(&Input {
            unifi: Some(&j),
            sweep: vec![host("aa:00:00:00:00:07")],
            gateway: Some("192.168.1.1".parse().unwrap()),
            range: Some("192.168.1.0/24".into()),
            ..Default::default()
        });
        assert!(at_office.summary.contains("sweep not merged"));
        let gw = |m: &Map| m.nodes.iter().find(|n| n.id == "gateway").unwrap().clone();
        assert_eq!(gw(&at_office).label, "Gateway (not in the data)");
        assert!(
            at_office
                .nodes
                .iter()
                .all(|n| n.ip != Some("192.168.1.7".parse().unwrap()))
        );
        let on_site = build(&Input {
            unifi: Some(&j),
            sweep: vec![host("00:00:00:00:00:01")],
            gateway: Some("192.168.1.1".parse().unwrap()),
            ..Default::default()
        });
        assert_eq!(gw(&on_site).sub, "192.168.1.1");
        assert!(by_label(&on_site, "Core").seen_by.contains(&"sweep"));
    }

    #[test]
    fn kind_is_short_caps() {
        assert_eq!(caps("switch usl48pb with a long tail"), "SWITCH USL48PB");
        assert_eq!(caps("gateway "), "GATEWAY");
        assert_eq!(short_vendor("Ubiquiti Inc"), "UBIQUITI");
        assert_eq!(classify(Some("Ubiquiti Inc"), false, &[]).0, Zone::Gear);
        assert_eq!(classify(None, false, &[9100]).1, "PRINTER");
        assert_eq!(classify(None, true, &[]).1, "PRIVATE MAC");
    }
}
