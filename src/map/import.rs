//! A /mapnetwork map redrawn as this app's map: its devices become cards
//! with roles read from their kind, its links become the tree (each card
//! hangs off the first link that reaches it from the internet; any other
//! link is listed in its details), and its findings become the band. A
//! sweep of the same network marks which of its devices answer, and what
//! the map left out (/mapnetwork maps clients only on request) joins it as
//! clients: the sweep's other hosts, and the client tables of the loaded
//! UniFi backups of that site, each under the card it most likely plugs
//! into.

use super::{
    B, Input, Link, Map, Node, Sev, Unifi, Zone, caps, limit, merge_unifi, parse_unifi, sweep_card,
    unifi_client_card,
};
use crate::mapnetwork::Doc;
use crate::net::mac::{self, Mac};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::Ipv4Addr;

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("").trim()
}

fn strings(v: &Value, k: &str) -> Vec<String> {
    match v.get(k) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| match x {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Role from a card's words (its kind, its zone's key or name, then its
/// label: the caller tries them in that order). Network words come before
/// voice and client words: a "voice edge" zone is the carrier's router, not
/// a phone. Switch words come before EDGE and ROUTING: "EDGE SWITCH" is a
/// switch. A voice zone's card is voice gear unless it says it's a phone:
/// /mapnetwork draws infrastructure, and clients only on request.
fn role(text: &str) -> Option<Zone> {
    let up = text.to_uppercase();
    let words: HashSet<&str> = up
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let any = |ws: &[&str]| ws.iter().any(|w| words.contains(w));
    let phrase = |ps: &[&str]| ps.iter().any(|p| up.contains(p));
    if any(&["INTERNET", "WAN", "ISP", "CARRIER"]) {
        Some(Zone::Internet)
    } else if any(&[
        "FIREWALL",
        "FIREWALLS",
        "GATEWAY",
        "ROUTER",
        "ROUTERS",
        "SONICWALL",
        "FORTIGATE",
        "UDM",
        "UCG",
        "USG",
        "UXG",
    ]) {
        Some(Zone::Gateway)
    } else if any(&["SWITCH", "SWITCHES", "USW", "IDF", "MDF", "DISTRIBUTION"]) {
        // Before EDGE and ROUTING: an "EDGE SWITCH", a "USW EDGE" or a
        // "ROUTING SWITCH" is a switch.
        Some(Zone::Switch)
    } else if any(&["EDGE", "ROUTING"]) {
        // A "voice edge" is the carrier's router, before VOICE below.
        Some(Zone::Gateway)
    } else if any(&[
        "AP", "APS", "WIFI", "WIRELESS", "UAP", "MESH", "BRIDGE", "WLAN", "SSID",
    ]) || phrase(&["ACCESS POINT", "WI-FI"])
    {
        Some(Zone::Wireless)
    } else if any(&[
        "SERVER",
        "SERVERS",
        "NAS",
        "SAN",
        "STORAGE",
        "HYPERVISOR",
        "ESXI",
        "DC",
        "RODC",
        "SQL",
        "BDR",
        "BACKUP",
        "WSUS",
        "IMAGING",
        "VM",
        "PBX",
    ]) || phrase(&["DOMAIN CONTROLLER", "HYPER-V", "PHONE SYSTEM"])
    {
        Some(Zone::Server)
    } else if any(&[
        "PRINTER",
        "PRINTERS",
        "COPIER",
        "COPIERS",
        "MFP",
        "MFD",
        "PLOTTER",
        "SCANNER",
        "CAMERA",
        "CAMERAS",
        "NVR",
        "DVR",
        "UPS",
        "PDU",
        "IOT",
        "OT",
        "PLC",
        "HMI",
        "SENSOR",
        "DOOR",
        "BADGE",
        "INTERCOM",
        "HVAC",
        "THERMOSTAT",
    ]) || phrase(&["ACCESS CONTROL"])
    {
        Some(Zone::Device)
    } else if any(&["PHONE", "PHONES", "HANDSET", "HANDSETS"]) {
        Some(Zone::Client)
    } else if any(&["VOIP", "VOICE", "SIP", "ATA"]) {
        Some(Zone::Gear)
    } else if any(&[
        "WORKSTATION",
        "WORKSTATIONS",
        "LAPTOP",
        "DESKTOP",
        "PC",
        "TABLET",
        "CLIENT",
        "CLIENTS",
        "GUEST",
        "TRUSTED",
    ]) || phrase(&["ADDITIONAL DEVICES"])
    {
        Some(Zone::Client)
    } else if any(&[
        "CONTROLLER",
        "KVM",
        "IPMI",
        "MODEM",
        "ONT",
        "NETWORK",
        "INFRA",
    ]) {
        Some(Zone::Gear)
    } else if any(&["UNKNOWN", "UNIDENTIFIED", "EXTERNAL", "UNRESOLVED"]) {
        Some(Zone::Unknown)
    } else {
        None
    }
}

/// The first address in `text`; a subnet (`10.0.0.0/24`) isn't one.
fn first_ip(text: &str) -> Option<Ipv4Addr> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '/'))
        .filter(|w| !w.contains('/'))
        .find_map(|w| w.trim_matches('.').parse().ok())
}

fn first_mac<'a>(texts: impl IntoIterator<Item = &'a str>) -> Option<Mac> {
    texts.into_iter().find_map(|t| {
        t.split(|c: char| !(c.is_ascii_hexdigit() || c == ':' || c == '-'))
            .filter(|w| w.len() == 17)
            .find_map(mac::parse_mac)
    })
}

/// Words that make a line about another device, or a maybe: "Candidate
/// identity: Dell 00:18:8b:… unconfirmed" names someone else's MAC.
const NOT_ITS_OWN: &[&str] = &[
    "candidate",
    "unconfirmed",
    "possibl",
    "maybe",
    "likely",
    "probabl",
    "suspect",
    "might",
    "could be",
    "behind",
    "upstream",
    "downstream",
    "neighbo",
    "seen on",
    "via ",
    "peer",
    "standby",
    "previous",
    "old mac",
    "before rma",
    "replaced",
];

/// A MAC anywhere in `t`: "aa:bb:cc:dd:ee:ff", "aa-bb-…" or Cisco's
/// dotted "aabb.ccdd.eeff".
fn mac_in(t: &str) -> Option<Mac> {
    first_mac([t]).or_else(|| {
        t.split(|c: char| !(c.is_ascii_hexdigit() || c == '.'))
            .map(|w| w.trim_matches('.'))
            .filter(|w| w.len() == 14)
            .find_map(|w| {
                let g: Vec<&str> = w.split('.').collect();
                if g.len() != 3 || g.iter().any(|x| x.len() != 4) {
                    return None;
                }
                let hex = g.concat();
                let mut m = [0u8; 6];
                for (k, b) in m.iter_mut().enumerate() {
                    *b = u8::from_str_radix(&hex[2 * k..2 * k + 2], 16).ok()?;
                }
                Some(m)
            })
    })
}

/// A card's own MAC from its text, for maps that don't write a `mac`
/// field: a part of the sub line, a chip or a detail that is the MAC
/// outright ("MAC aa:bb:…", "MAC address: aa-bb-…", "MAC-address: …",
/// "MAC address (LAN): …", "LAN MAC: …", or the address alone, Cisco's
/// dotted "aabb.ccdd.eeff" too), followed by nothing or a tag ("· ARP
/// only", "(from the switch)"). Never one inside a sentence ("Uplink: Core
/// port 7 (aa:bb:…)", "aa:bb:… belongs to the ISP router"), or on a line
/// that is a maybe or about another unit ("Candidate …", "HA peer", "Old
/// MAC before RMA").
fn own_mac(sub: &str, lines: &[&str]) -> Option<Mac> {
    use regex::Regex;
    use std::sync::LazyLock;
    static LEAD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^(?:(?:lan|wan|mgmt|management|wired|wireless|wi-?fi)\s+)?mac(?:[\s-]*addr(?:ess)?)?(?:\s*\([^)]{0,20}\))?\s*[:=]?\s*",
        )
        .unwrap()
    });
    let plain = |t: &str| {
        let low = t.to_lowercase();
        !NOT_ITS_OWN.iter().any(|w| low.contains(w))
    };
    let explicit = |t: &str| {
        let t = t.trim();
        let rest = LEAD.find(t).map_or(t, |m| &t[m.end()..]);
        let (first, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let tail = tail.trim();
        // Nothing after it, or a tag: "· ARP only", "(from the switch)".
        if !(tail.is_empty() || tail.starts_with(['·', '|', '(', '[', '-', '–'])) {
            return None;
        }
        mac_in(first.trim_matches(|c: char| matches!(c, ',' | ';' | '(' | ')' | '[' | ']')))
    };
    sub.split(['·', '|'])
        .chain(lines.iter().copied())
        .filter(|t| plain(t))
        .find_map(explicit)
}

/// What a /mapnetwork edge says about the card it leads to: how it's
/// linked, the parent's port when the label names one, and lines for the
/// card's details (what the label said otherwise, its own port, why the
/// link is inferred).
struct EdgeRead {
    link: Link,
    port: Option<String>,
    notes: Vec<String>,
}

/// A link's label read as a port: "Port 21", "port #21", "p21",
/// "Gi1/0/21", "1/1/1", "eth0", "X1", "LAN2", "LAN 2", "SFP+ 1", and a
/// label that is only a port ("24", "A1"). A count ("21 wired"), a range
/// ("port 1-24"), a VLAN ("VLAN 10 gw"), a name ("AT&T", "Building IX
/// 2"), a size ("LAG x 4", "LAG x4"), a speed ("10/100", "LAN 100 Mbps")
/// or a date ("3/4/25") isn't one: a short prefix (x, ix, em, po…) counts
/// only joined to its number.
fn port_in(text: &str) -> Option<String> {
    use regex::Regex;
    use std::sync::LazyLock;
    static PORT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(?:port|p)\s*#?\s*(\d{1,3}(?:/\d{1,3}){0,2})\b(\s*[-–]\s*\d)?").unwrap()
    });
    // Long names may have a space before the number; short ones may not.
    static IFACE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)\b(?:(?:gigabitethernet|tengigabitethernet|fastethernet|ethernet|port-channel|ether|eth|sfp\+?|lan|wan|mgmt)\s?-?|(?:gi|ge|fa|te|xe|et|x|po|igb|em|ix)-?)\d{1,3}(?:/\d{1,3}){0,2}(?:\.\d{1,4})?(\s*[-–]\s*\d+)?(?:\b|$)",
        )
        .unwrap()
    });
    // A stack/slot/port alone or in a label: "1/1/1", "1/0/24", an ICX's
    // "1/3/1". A date ("3/4/25": a slot past 2 and a year) isn't one.
    static SLOT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:^|\s)((\d{1,2})/(\d{1,2})/(\d{1,3}))(?:\s|$)").unwrap());
    // The whole label is the port: "24", "A1", an HPE stack's "1/24".
    static ALONE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^(?:\d{1,3}(?:/\d{1,2})?|[a-z]\d{1,2})$").unwrap());
    // A speed: "10/100", "100/1000".
    static SPEED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:10|100|1000)/(?:100|1000)(?:/1000)?$").unwrap());
    // A speed or size right after it: "LAN 100 Mbps" is a link speed;
    // "X1 GbE" names the port's kind, not a size.
    static UNIT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^\s*(?:[kmg]bps|[kmg]b/s|[kmg]bit|mb|gb|mhz|ghz|%)(?:\b|$)").unwrap()
    });
    let t = text.trim();
    if let Some(c) = PORT.captures(t) {
        // "port 1-24" is a range of ports, not one.
        return c.get(2).is_none().then(|| format!("Port {}", &c[1]));
    }
    if let Some(c) = IFACE.captures(t) {
        let m = c.get(0)?;
        let before = t[..m.start()].trim_end().to_ascii_lowercase();
        let last = before.rsplit(' ').next().unwrap_or("");
        // "LAG x4" is a size: an "x" right after an aggregate's word or a
        // number. "Trunk X2", "Fiber X1", "HA X3" are ports.
        let times = m.as_str().to_ascii_lowercase().starts_with('x')
            && (matches!(last, "lag" | "lacp" | "mlag" | "bond")
                || last.ends_with(|c: char| c.is_ascii_digit()));
        // "Gi1/0/1-24" is a range too.
        return (c.get(1).is_none() && !times && !UNIT.is_match(&t[m.end()..]))
            .then(|| m.as_str().trim().to_string());
    }
    if let Some(c) = SLOT.captures(t) {
        let n = |k: usize| c[k].parse::<u16>().unwrap_or(0);
        let date = n(3) > 2 && n(4) >= 13;
        return (!date).then(|| c[1].to_string());
    }
    if ALONE.is_match(t) && !SPEED.is_match(t) {
        return Some(match t.parse::<u16>() {
            Ok(n) => format!("Port {n}"),
            Err(_) => t.to_string(),
        });
    }
    None
}

/// Words that say a link is a tunnel, not a cable.
const TUNNEL: &[&str] = &["ipsec", "vpn", "tunnel", "wireguard", "site-to-site", "s2s"];

/// Words that describe the link itself, kept as its label: "LAG", "PoE".
const LINK_WORDS: &[&str] = &[
    "lag", "lacp", "mlag", "bond", "poe", "fiber", "fibre", "sfp", "trunk", "stack", "ha",
];

fn read_edge(e: &Value, from_root: bool) -> EdgeRead {
    let label = s(e, "label");
    let low = label.to_lowercase();
    let words: Vec<&str> = low
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|w| !w.is_empty())
        .collect();
    let dashed = e.get("dashed").and_then(Value::as_bool) == Some(true);
    let cls = s(e, "cls");
    let mut notes = Vec::new();
    // "X2 → Port 24": the parent's port first, then the card's own.
    // "port 1 - 24" is a range, not two ends; "X2 - 24" is two ends.
    static RANGE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)\b(?:ports?|p)\s*#?\s*\d{1,3}\s+[-–—]\s+\d").unwrap()
    });
    let seps: &[&str] = if RANGE.is_match(label) {
        &["→", "<->", "->", "↔", " to "]
    } else {
        &["→", "<->", "->", "↔", " to ", " – ", " — ", " - "]
    };
    let split = seps.iter().find_map(|sep| label.split_once(sep));
    let (port, own) = match split {
        Some((a, b)) => (port_in(a), port_in(b)),
        // "Port 24 uplink": the card's own uplink port, not the parent's.
        None if words.contains(&"uplink") => (None, port_in(label)),
        None => (port_in(label), None),
    };
    if let Some(p) = &own {
        notes.push(format!("Its own port: {p}"));
    }
    let tunnel = TUNNEL
        .iter()
        .any(|t| words.iter().any(|w| w.starts_with(t)));
    let link_word = LINK_WORDS.iter().any(|t| words.contains(t));
    // The label stays the link's when it's a port, a tunnel, a kind of
    // link, or the WAN's (the ISP); otherwise it goes in the details.
    let keep = port.is_some() || tunnel || link_word || from_root;
    let shown = if port.is_some() && split.is_none() && own.is_none() {
        Some(label.to_string())
    } else if port.is_some() {
        port.clone()
    } else if keep && !label.is_empty() {
        Some(label.to_string())
    } else {
        None
    };
    if !label.is_empty() && !keep {
        notes.push(format!("Link on the map: {label}"));
    }
    let link = match cls {
        "backbone" => Link::Backbone,
        "wifi" => Link::Wireless,
        _ if tunnel => {
            notes.push(format!(
                "Linked by a tunnel on the map ({label}), not a cable"
            ));
            Link::Wired
        }
        _ if !dashed => Link::Wired,
        _ if low.contains("reciprocal") => {
            notes.push(crate::inferred::UPLINK_RECIPROCAL.into());
            Link::Assumed
        }
        _ if low.contains("last seen") || low.contains("not seen") || low.contains("stale") => {
            notes.push(crate::inferred::UPLINK_LAST_SEEN.into());
            Link::Assumed
        }
        _ => {
            notes.push(crate::inferred::UPLINK_MAP.into());
            Link::Assumed
        }
    };
    EdgeRead {
        link,
        port: shown,
        notes,
    }
}

pub fn build(doc: &Doc, input: &Input) -> Map {
    let sites: HashMap<&str, &str> = doc
        .sites
        .iter()
        .map(|(k, n)| (k.as_str(), n.as_str()))
        .collect();
    let multi_site = doc.sites.len() > 1;

    // Ids in file order; a repeated id keeps its first card, as the engine does.
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut src: Vec<&Value> = Vec::new();
    for v in &doc.nodes {
        let id = s(v, "id");
        if !id.is_empty() && !index.contains_key(id) {
            index.insert(id, src.len());
            src.push(v);
        }
    }
    let edges: Vec<(usize, usize, &Value)> = doc
        .edges
        .iter()
        .filter_map(|e| {
            let (f, t) = (*index.get(s(e, "from"))?, *index.get(s(e, "to"))?);
            (f != t).then_some((f, t, e))
        })
        .collect();
    let mut incoming = vec![false; src.len()];
    for &(_, t, _) in &edges {
        incoming[t] = true;
    }

    let zone_of = |v: &Value| {
        let z = s(v, "zone");
        let zinfo = doc.zones.get(z);
        // The kind, then the zone, and the label last: a "DESKTOP-4QX"
        // in the OT zone is filed with the OT gear around it.
        let zname = zinfo.and_then(|i| role(s(i, "name")));
        // /mapnetwork's `access` zone is its access points, whatever it's
        // named ("Access", "Access layer"), unless the name says switches.
        let zrole = if z.eq_ignore_ascii_case("access") {
            Some(
                zname
                    .filter(|r| *r == Zone::Switch)
                    .unwrap_or(Zone::Wireless),
            )
        } else {
            role(z).or(zname)
        };
        role(s(v, "kind"))
            .or(zrole)
            .or_else(|| role(s(v, "label")))
            .unwrap_or(Zone::Unknown)
    };
    // The map's own internet card, when it has one at the top, is the root.
    let root_src = (0..src.len()).find(|&i| !incoming[i] && zone_of(src[i]) == Zone::Internet);

    let mut b = B::default();
    // Card index in `b` for each source node; the root is card 0.
    let mut card = vec![usize::MAX; src.len()];
    if root_src.is_none() {
        b.add(Node {
            id: "mapnetwork:internet".into(),
            zone: Zone::Internet,
            kind: "INTERNET".into(),
            label: "Internet".into(),
            sub: input.wan.map(|w| format!("WAN {w}")).unwrap_or_default(),
            ..Default::default()
        });
    }
    let order: Vec<usize> = root_src
        .into_iter()
        .chain((0..src.len()).filter(|&i| Some(i) != root_src))
        .collect();
    for (rank, &i) in order.iter().enumerate() {
        let v = src[i];
        let zone = if Some(i) == root_src {
            Zone::Internet
        } else {
            match zone_of(v) {
                // Only the root draws as the internet; another WAN card is
                // the edge of the network.
                Zone::Internet => Zone::Gateway,
                z => z,
            }
        };
        let sub = s(v, "sub").to_string();
        let mut details = strings(v, "details");
        let chips = strings(v, "chips");
        // "SONICWALL INFERRED" or an `inferred` chip: the map worked its
        // role out. One mark for it, as everywhere: the word leaves the kind.
        let raw_kind = s(v, "kind");
        let kind_inferred = raw_kind
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|w| w.eq_ignore_ascii_case(crate::inferred::WORD));
        let role_inferred = kind_inferred
            || chips
                .iter()
                .any(|c| c.eq_ignore_ascii_case(crate::inferred::WORD));
        let kind = if kind_inferred {
            let k: Vec<&str> = raw_kind
                .split_whitespace()
                .filter(|w| {
                    !w.trim_matches(|c: char| !c.is_ascii_alphanumeric())
                        .eq_ignore_ascii_case(crate::inferred::WORD)
                })
                .collect();
            k.join(" ")
                .trim_matches(|c: char| c == '(' || c == ')' || c == '·' || c == '-' || c == ' ')
                .to_string()
        } else {
            raw_kind.to_string()
        };
        let mut n = Node {
            id: s(v, "id").to_string(),
            zone,
            kind: caps(&kind),
            label: s(v, "label").to_string(),
            // The node's own ip/mac fields first; older maps only write
            // them into the text.
            ip: s(v, "ip").trim().parse().ok().or_else(|| first_ip(&sub)),
            mac: mac::parse_mac(s(v, "mac").trim()).or_else(|| {
                let lines: Vec<&str> = chips
                    .iter()
                    .chain(details.iter())
                    .map(String::as_str)
                    .collect();
                own_mac(&sub, &lines)
            }),
            sub,
            flags: strings(v, "flags"),
            client: zone == Zone::Client,
            ..Default::default()
        };
        if n.label.is_empty() {
            n.label = n.id.clone();
        }
        n.rank = (u8::from(n.flags.is_empty()), rank as u64);
        let site = sites.get(s(v, "site")).copied();
        n.site = site.map(str::to_string);
        if multi_site && let Some(site) = site {
            n.chip(site);
        }
        // is-alive's own page marks a card a person drew on a plan.
        let drawn = crate::map::export::DC_HTML_DRAWN;
        n.drawn = chips.iter().any(|c| c == drawn);
        for c in chips.into_iter().filter(|c| c != drawn) {
            if c.eq_ignore_ascii_case(crate::inferred::WORD) {
                n.chip(crate::inferred::WORD);
            } else {
                n.chip(c);
            }
        }
        if role_inferred {
            n.chip(crate::inferred::WORD);
            if !details.iter().any(|d| d.starts_with("Role inferred")) {
                details.push(crate::inferred::ROLE_MAP.into());
            }
        }
        let vlans = strings(v, "vlans");
        match vlans.len() {
            0 => {}
            1 => n.chip(format!("VLAN {}", vlans[0])),
            _ => n.chip(format!("VLANs {}", vlans.join("/"))),
        }
        if let Some(site) = site {
            details.push(format!("Site: {site}"));
        }
        if let Some(z) = doc.zones.get(s(v, "zone")) {
            let (name, vlan) = (s(z, "name"), s(z, "vlan"));
            if !name.is_empty() {
                details.push(match vlan {
                    "" => format!("Zone on the map: {name}"),
                    v => format!("Zone on the map: {name} ({v})"),
                });
            }
        }
        n.details = details;
        n.seen("mapnetwork");
        card[i] = b.add(n);
    }

    // The tree: breadth-first from the internet, so each card hangs off the
    // shortest way up. Cards no link reaches hang off the internet as assumed.
    let mut down: Vec<Vec<usize>> = vec![Vec::new(); src.len()];
    for (k, &(f, _, _)) in edges.iter().enumerate() {
        down[f].push(k);
    }
    let mut via: Vec<Option<usize>> = vec![None; src.len()];
    let mut reached = vec![false; src.len()];
    let mut queue = VecDeque::new();
    if let Some(r) = root_src {
        reached[r] = true;
        queue.push_back(r);
    }
    // Unreached cards with no link into them go first: they're tops of
    // their own trees.
    let mut rest = (0..src.len())
        .filter(|&i| !incoming[i])
        .chain((0..src.len()).filter(|&i| incoming[i]));
    loop {
        while let Some(u) = queue.pop_front() {
            for &k in &down[u] {
                let t = edges[k].1;
                if !reached[t] {
                    reached[t] = true;
                    via[t] = Some(k);
                    queue.push_back(t);
                }
            }
        }
        let Some(i) = rest.by_ref().find(|&i| !reached[i]) else {
            break;
        };
        reached[i] = true;
        queue.push_back(i);
    }
    // Cards the map links to nothing go under one "Not linked" card: only
    // a gateway may sit straight on the internet.
    let mut unlinked = Vec::new();
    for i in 0..src.len() {
        if Some(i) == root_src {
            continue;
        }
        let c = card[i];
        let up = match via[i] {
            Some(k) => {
                let (f, _, e) = edges[k];
                let r = read_edge(e, Some(f) == root_src);
                b.nodes[c].details.extend(r.notes);
                (card[f], r.link, r.port)
            }
            None if b.nodes[c].zone == Zone::Gateway && root_src.is_none() => {
                (0, Link::Backbone, None)
            }
            None if b.nodes[c].zone == Zone::Gateway => {
                b.nodes[c].details.push(
                    "Uplink inferred: the map links nothing to it, so it sits on the internet \
                     as the gateway it is"
                        .into(),
                );
                (0, Link::Assumed, None)
            }
            None => {
                unlinked.push(c);
                continue;
            }
        };
        b.nodes[c].up = Some(up);
    }
    if !unlinked.is_empty() {
        // Under the gateway when there's one on the internet, it's
        // confirmed (its role read, its link to the internet drawn solid)
        // and every unlinked card is at its site; else at the root. An
        // inferred gateway, or one site's, doesn't own other sites' strays.
        let mut gws = (0..b.nodes.len())
            .filter(|&i| b.nodes[i].zone == Zone::Gateway && b.nodes[i].parent() == Some(0));
        let home = match (gws.next(), gws.next()) {
            (Some(g), None)
                if !b.nodes[g].role_inferred()
                    && b.nodes[g].up.as_ref().is_some_and(|u| u.1 != Link::Assumed)
                    && unlinked.iter().all(|&c| b.nodes[c].site == b.nodes[g].site) =>
            {
                g
            }
            _ => 0,
        };
        let nl = b.add(Node {
            id: "mapnetwork:not-linked".into(),
            zone: Zone::Unknown,
            kind: "NOT LINKED".into(),
            label: "Not linked".into(),
            sub: format!("{} card(s), placement unknown", unlinked.len()),
            details: vec![
                "Cards the /mapnetwork map links to nothing: where they plug in isn't known, \
                 so they wait here"
                    .into(),
            ],
            up: Some((home, Link::Assumed, None)),
            ..Default::default()
        });
        b.nodes[nl]
            .details
            .push(crate::inferred::UPLINK_NONE.into());
        for c in unlinked {
            b.nodes[c].details.push(crate::inferred::UPLINK_NONE.into());
            b.nodes[c].up = Some((nl, Link::Assumed, None));
        }
    }
    // The scanner's public address stays on the map's own internet card.
    if let (Some(r), Some(w)) = (root_src, input.wan) {
        let n = &mut b.nodes[card[r]];
        let line = format!("WAN {w}");
        if n.sub.is_empty() {
            n.sub = line;
        } else if !n.sub.contains(&w.to_string()) {
            n.details
                .push(format!("Public address this scan left by: {w}"));
        }
    }
    let label_of = |e: &Value| Some(s(e, "label").to_string()).filter(|l| !l.is_empty());
    let used: HashSet<usize> = via.iter().flatten().copied().collect();
    // A few per card: a crafted page can hold 50,000 links between the
    // same two cards, each line a copy of a label.
    const ALSO_MAX: usize = 20;
    let mut also: HashMap<usize, usize> = HashMap::new();
    for (k, &(f, t, e)) in edges.iter().enumerate() {
        if used.contains(&k) {
            continue;
        }
        let tail = label_of(e).map(|l| format!(" ({l})")).unwrap_or_default();
        for (on, other, dir) in [(card[t], card[f], "from"), (card[f], card[t], "to")] {
            let n = also.entry(on).or_default();
            *n += 1;
            if *n <= ALSO_MAX {
                let line = format!("Also linked {dir} {}{tail}", b.nodes[other].label);
                b.nodes[on].details.push(line);
            }
        }
    }
    for (c, n) in also {
        if n > ALSO_MAX {
            b.nodes[c]
                .details
                .push(format!("Also linked {} more times", n - ALSO_MAX));
        }
    }

    // The map's own "+7 more" cards, one card for each device they list.
    // The cards made from their lines: the only cards a controller's
    // client may join by address alone.
    let mut only_counted = 0usize;
    let mut listed_cards: HashSet<usize> = HashSet::new();
    for &c in &card {
        if !is_overflow(&b.nodes[c]) {
            continue;
        }
        let before = b.nodes.len();
        only_counted = only_counted.saturating_add(expand_overflow(&mut b, c, src.len() as u64));
        listed_cards.extend(before..b.nodes.len());
        if !is_overflow(&b.nodes[c]) {
            listed_cards.insert(c);
        }
    }

    for a in &doc.actions {
        let sev = match s(a, "sev") {
            "high" | "critical" => Sev::High,
            "low" | "info" => Sev::Low,
            _ => Sev::Med,
        };
        let text = s(a, "text");
        if text.is_empty() {
            continue;
        }
        let ids = strings(a, "nodes")
            .into_iter()
            .filter(|id| index.contains_key(id.as_str()))
            .collect();
        b.findings.push((sev, text.to_string(), ids));
    }

    // What each card's zone says about it ("VLAN 20 · 192.168.2.x"), to
    // place the hosts the map left out.
    let mut zone_text = vec![String::new(); b.nodes.len()];
    for (i, v) in src.iter().enumerate() {
        if let Some(z) = doc.zones.get(s(v, "zone")) {
            zone_text[card[i]] = format!("{} {}", s(z, "name"), s(z, "vlan"));
        }
    }
    let cards = b.nodes.len();
    // A UniFi backup of the same site knows clients the map left out, and
    // the switch or AP each was last seen on. Each backup joins on its own:
    // one of another site stays out.
    let mut unifi_at = Vec::new();
    let mut joined = Vec::new();
    let mut joined_names = Vec::new();
    for f in &input.unifi {
        let u = parse_unifi(f.json);
        let at = join_unifi(&mut b, &u, cards);
        if !at.is_empty() {
            unifi_at.extend(at);
            joined.push(u);
            joined_names.push(f.name);
        }
    }
    let backups = (joined.len(), input.unifi.len());
    let unifi = merge_unifi(joined, &joined_names);
    // A sweep joins only when it found hardware the map knows by MAC: the
    // same network, not a look-alike 192.168.1.x.
    let sweep_at: Vec<usize> = input
        .sweep
        .iter()
        .filter_map(|h| b.by_mac.get(&h.mac?).copied())
        .filter(|&i| i < cards)
        .collect();
    let local = !sweep_at.is_empty();
    // Each source's hosts go by the subnets of the site(s) it was found at:
    // two sites can both use 192.168.1.x.
    let hints = Hints {
        sweep: subnet_uplinks(&b, &zone_text, &sites_of(&b, &sweep_at)),
        unifi: subnet_uplinks(&b, &zone_text, &sites_of(&b, &unifi_at)),
    };
    let mut off_map = 0;
    let mut deferred = Vec::new();
    if local {
        let in_unifi: HashSet<Mac> = unifi
            .iter()
            .flat_map(|u| u.clients.iter().map(|c| c.mac))
            .collect();
        for h in &input.sweep {
            match b.find(h.mac, Some(h.ip)) {
                Some(i) => b.merge_seen(i, h),
                // UniFi's card for it, below, knows where it plugs in.
                None if h.mac.is_some_and(|m| in_unifi.contains(&m)) => {
                    off_map += 1;
                    deferred.push(h);
                }
                None => {
                    off_map += 1;
                    let i = b.add(sweep_card(h, Some(h.ip) == input.this));
                    b.merge_seen(i, h);
                }
            }
        }
    }
    let mut unifi_added = 0;
    // The firewall's own interface MACs, from a config export loaded too
    // (it isn't drawn, but it says which MACs are the firewall's).
    let fw_macs: HashSet<Mac> = input
        .sonicwall
        .iter()
        .filter_map(|f| super::parse_sonicwall(f.json))
        .flat_map(|s| s.ifaces.into_iter().filter_map(|f| f.mac))
        .map(|m| mac::universal(&m))
        .collect();
    if let Some(u) = &unifi {
        // Of the controller's clients at one address, the one seen last:
        // with DHCP churn the others are stale.
        let mut newest: HashMap<Ipv4Addr, (i64, Mac)> = HashMap::new();
        for c in &u.clients {
            if let Some(ip) = c.ip {
                let e = newest.entry(ip).or_insert((c.last_seen, c.mac));
                if c.last_seen > e.0 {
                    *e = (c.last_seen, c.mac);
                }
            }
        }
        let newest: HashMap<Ipv4Addr, Mac> =
            newest.into_iter().map(|(ip, (_, m))| (ip, m)).collect();
        for c in &u.clients {
            if let Some(i) = b.find(Some(c.mac), None) {
                b.nodes[i].seen("UniFi");
                continue;
            }
            // The same address on the gateway, from a MAC that is one of
            // its own interfaces, or on a "+N more" line with no MAC and
            // no other name: that card, not a second. Anything else at a
            // card's address (a guest laptop on the firewall's, a stale
            // phone on a printer's) is its own card: on infrastructure's
            // address, the address clash is a finding.
            if let Some(i) = c.ip.and_then(|ip| b.by_ip.get(&ip).copied()) {
                let n = &b.nodes[i];
                let gateway = n.zone == Zone::Gateway && !n.client;
                if gateway && own_interface(n, c, &fw_macs) {
                    let newest_here = c.ip.and_then(|ip| newest.get(&ip)) == Some(&c.mac);
                    let n = &mut b.nodes[i];
                    n.seen("UniFi");
                    // Its MAC only from the client seen there last: an old
                    // box's record (before a swap) mustn't name it.
                    if n.mac.is_none() && newest_here {
                        n.mac = Some(c.mac);
                        b.by_mac.entry(c.mac).or_insert(i);
                    } else if n.mac != Some(c.mac) {
                        n.detail(format!(
                            "The controller's client table has it at the same address with \
                             MAC {}, one of its own interfaces",
                            mac::fmt(&c.mac)
                        ));
                    }
                    continue;
                }
                // A line with no name ("Host 192.0.2.77") takes any; a
                // named one only the same name, not a client with none.
                let unnamed = n.ip.is_some_and(|ip| n.label == format!("Host {ip}"));
                let names_agree = unnamed
                    || c.name
                        .as_deref()
                        .is_some_and(|name| n.label.eq_ignore_ascii_case(name));
                if listed_cards.contains(&i)
                    && n.mac.is_none()
                    && names_agree
                    && c.ip.and_then(|ip| newest.get(&ip)) == Some(&c.mac)
                {
                    let n = &mut b.nodes[i];
                    n.seen("UniFi");
                    n.mac = Some(c.mac);
                    b.by_mac.entry(c.mac).or_insert(i);
                    if let Some(name) = &c.name
                        && n.label.starts_with("Host ")
                    {
                        n.label = name.clone();
                    }
                    if let Some(v) = &c.oui
                        && !n.details.iter().any(|d| d.starts_with("Vendor"))
                    {
                        n.details.push(format!("Vendor {v} (UniFi)"));
                    }
                    if c.guest {
                        n.chip("guest");
                    }
                    continue;
                }
            }
            let (parent, link, port) = b.unifi_uplink(c);
            let mut n = unifi_client_card(c);
            n.up = parent.map(|p| (p, link, port));
            let i = b.add(n);
            unifi_added += 1;
            if let Some(h) = deferred.iter().find(|h| h.mac == Some(c.mac)) {
                b.merge_seen(i, h);
                // It answered just now.
                b.nodes[i].rank.1 = 0;
            }
        }
    }
    let unplaced = place_added(&mut b, cards, &hints);

    let mut summary = [doc.title.as_str(), doc.subtitle.as_str()]
        .iter()
        .filter(|t| !t.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ");
    summary += if summary.is_empty() {
        "from /mapnetwork"
    } else {
        " · from /mapnetwork"
    };
    if local {
        summary += &format!(" · sweep merged, {off_map} host(s) not on the map added as clients");
    } else if !input.sweep.is_empty() {
        summary += " · sweep not merged (no device on the map shares a MAC with it)";
    }
    summary += &match backups {
        (0, 0) => String::new(),
        (1, 1) => format!(" · UniFi backup merged, {unifi_added} client(s) added"),
        (0, 1) => " · UniFi backup not merged (none of its devices is on the map)".into(),
        (0, _) => " · UniFi backups not merged (none of their devices is on the map)".into(),
        (j, n) if j == n => {
            format!(" · {n} UniFi backups merged, {unifi_added} client(s) added")
        }
        (j, n) => format!(
            " · {j} of {n} UniFi backups merged, {unifi_added} client(s) added (the others' \
             devices aren't on the map)"
        ),
    };
    if unplaced > 0 {
        summary += &format!(" · {unplaced} not placed");
    }
    if only_counted > 0 {
        summary += &format!(" · {only_counted} client(s) the map counts but doesn't list");
    }
    b.address_findings();
    let mut map = limit(b, input);
    summary += &map.hidden_note();
    map.summary = summary;
    map.cap_depth_noted();
    map.sweep_apart = !local && !input.sweep.is_empty();
    map.title = doc.title.trim().to_string();
    // A map without SITES is one place: the map's own title names it. With
    // SITES, a card the map put at no site stays outside every site's box.
    if doc.sites.is_empty() {
        let title = doc.title.trim();
        map.fill_sites(if title.is_empty() {
            "This network"
        } else {
            title
        });
    }
    map
}

/// A gateway card's brand or product word, and the maker words its own
/// interfaces' MACs carry: a "FortiGate 60F" is Fortinet's, a "Firebox"
/// WatchGuard's, a "Starlink router" SpaceX's.
const GATEWAY_BRANDS: &[(&str, &[&str])] = &[
    ("fortigate", &["fortinet"]),
    ("fortiwifi", &["fortinet"]),
    ("fortinet", &["fortinet"]),
    ("meraki", &["meraki"]),
    ("pfsense", &["netgate"]),
    ("netgate", &["netgate"]),
    ("opnsense", &["deciso"]),
    ("firebox", &["watchguard"]),
    ("watchguard", &["watchguard"]),
    ("sonicwall", &["sonicwall"]),
    ("unifi", &["ubiquiti"]),
    ("ubiquiti", &["ubiquiti"]),
    ("udm", &["ubiquiti"]),
    ("ucg", &["ubiquiti"]),
    ("usg", &["ubiquiti"]),
    ("uxg", &["ubiquiti"]),
    ("edgerouter", &["ubiquiti"]),
    ("mikrotik", &["mikrotik", "routerboard"]),
    ("routerboard", &["mikrotik", "routerboard"]),
    ("palo alto", &["palo alto"]),
    ("sophos", &["sophos"]),
    ("juniper", &["juniper"]),
    ("srx", &["juniper"]),
    ("asa", &["cisco"]),
    ("cisco", &["cisco"]),
    ("netgear", &["netgear"]),
    ("orbi", &["netgear"]),
    ("tp-link", &["tp-link"]),
    ("omada", &["tp-link"]),
    ("starlink", &["spacex", "space exploration"]),
    ("draytek", &["draytek"]),
    ("vigor", &["draytek"]),
    ("zyxel", &["zyxel"]),
    ("peplink", &["peplink", "pismo"]),
    ("pepwave", &["peplink", "pismo"]),
    ("cradlepoint", &["cradlepoint"]),
    ("aruba", &["aruba", "hewlett packard"]),
    ("barracuda", &["barracuda"]),
    ("check point", &["check point"]),
    ("eero", &["eero"]),
    ("asus", &["asustek"]),
];

/// Whether the controller's client `c` is plausibly one of gateway `n`'s
/// own interfaces: the same maker's prefix as the card's MAC (a
/// SonicWall's LAN answers on its own MAC with the locally-administered
/// bit set), or a MAC a config export lists. On a card with no MAC, the
/// maker the card's brand names (a "FortiGate 60F" card and a Fortinet
/// MAC; whole words, so "link" isn't "Starlink"); a card that names no
/// brand takes a network-gear maker's wired, non-guest client, unless its
/// name says it's an endpoint (a Cisco desk phone "SEP…", an AP).
fn own_interface(n: &Node, c: &super::UClient, fw_macs: &HashSet<Mac>) -> bool {
    let u = mac::universal(&c.mac);
    if fw_macs.contains(&u) {
        return true;
    }
    if let Some(m) = n.mac {
        return mac::universal(&m)[..3] == u[..3];
    }
    // The registry, then the controller's own lookup, then the guess a
    // locally administered MAC's cleared prefix gives.
    let maker = mac::vendor(&c.mac)
        .map(str::to_string)
        .or_else(|| c.oui.clone())
        .or_else(|| mac::universal_vendor(&c.mac).map(str::to_string));
    let Some(maker) = maker else {
        return false;
    };
    let text = format!("{} {} {}", n.kind, n.label, n.sub);
    let named: Vec<&[&str]> = GATEWAY_BRANDS
        .iter()
        .filter(|(brand, _)| super::word_in(&text, brand))
        .map(|(_, makers)| *makers)
        .collect();
    if !named.is_empty() {
        return named
            .iter()
            .flat_map(|m| m.iter())
            .any(|w| super::word_in(&maker, w));
    }
    let endpoint = c.name.as_deref().is_some_and(|name| {
        super::cisco_phone_name(Some(name))
            || matches!(
                role(name),
                Some(Zone::Client | Zone::Wireless | Zone::Device | Zone::Server)
            )
    });
    super::gear_brand(Some(&maker)).is_some()
        && super::phone_brand(Some(&maker)).is_none()
        && c.wired
        && !c.guest
        && !endpoint
}

/// Whether a /mapnetwork card is the map's own "+7 more": the clients
/// past its 10 per uplink, one line of details each.
pub(super) fn is_overflow(n: &Node) -> bool {
    n.kind.contains("ADDITIONAL DEVICES")
}

/// One device a "+N more" card lists: "client-0badf00d · Hikvision ·
/// guest", "laptop-a · 192.168.1.40", "MAC 38:c6:… · ARP only".
struct Listed {
    line: String,
    name: Option<String>,
    ip: Option<Ipv4Addr>,
    mac: Option<Mac>,
    vendor: Option<String>,
    guest: bool,
}

/// Words in a listed line that say how it's connected, not who made it.
const LISTED_TAGS: &[&str] = &[
    "wired",
    "wireless",
    "wifi",
    "wi-fi",
    "arp only",
    "offline",
    "online",
    "blocked",
    "private mac",
    "randomized",
    "random mac",
    "dhcp",
    "static",
    "unknown",
    "none",
];

/// Heads of a "+N more" line that name a kind of client, not a device:
/// "Wireless · 8", "Clients · 40".
const SUMMARY_HEADS: &[&str] = &[
    "clients", "devices", "hosts", "wired", "wireless", "guests", "total", "iot", "phones",
];

/// "8", "12 devices", "40 clients": a count.
fn is_count(f: &str) -> bool {
    let mut w = f.split_whitespace();
    w.next().is_some_and(|n| n.parse::<u32>().is_ok())
        && w.next().is_none_or(|k| {
            let k = k.to_lowercase();
            [
                "device", "client", "host", "guest", "item", "more", "total", "wired", "wireless",
            ]
            .iter()
            .any(|p| k.starts_with(p))
        })
        && w.next().is_none()
}

/// Whether a listed line's field reads as a maker ("Hangzhou Hikvision",
/// "Hikvision", "Dell Inc.", "Epson"): a maker as people write one (see
/// `written_maker`), and it isn't a VLAN, a band, a speed, an OS or a "key: value"
/// ("VLAN-20", "5GHz", "1Gbps", "Win11", "Front desk").
fn vendor_like(f: &str) -> bool {
    static NOT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)\b(?:vlan|ssid|port|ghz|mhz|[kmg]bps|dbm|win\d*|windows|macos|ios|ipados|android|linux|firmware|version|uptime|printers?|cameras?|phones?|laptops?|desktops?|tablets?|tv|servers?|kiosks?)\b|\d(?:\.\d+)?\s*(?:ghz|mhz|[kmg]bps|[kmg]b/s)\b|[:=/%@]",
        )
        .unwrap()
    });
    let words: Vec<&str> = f.split_whitespace().collect();
    !words.is_empty()
        && words.len() <= 5
        && f.len() <= 48
        && !NOT.is_match(f)
        && super::written_maker(f)
}

/// "…and 121 more" (or "+121 more", "and 121 others"): devices the card
/// only counts.
fn unlisted_count(line: &str) -> Option<usize> {
    static MORE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^\s*(?:…|\.\.\.|\+)?\s*(?:and\s+)?(\d+)\s+(?:more|others?)\b")
            .unwrap()
    });
    MORE.captures(line).and_then(|c| c[1].parse().ok())
}

/// A listed device from a line of a "+N more" card's details, or None
/// for a note ("Private MACs are per-network addresses").
fn listed(line: &str) -> Option<Listed> {
    let fields: Vec<&str> = line
        .split(['·', '|'])
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .collect();
    let head = *fields.first()?;
    let ip = first_ip(line);
    let mac = first_mac([line]);
    let one_word = |t: &str| !t.contains(char::is_whitespace) && t.len() <= 63;
    let tag = |t: &str| {
        let low = t.to_lowercase();
        low == "guest"
            || LISTED_TAGS.contains(&low.as_str())
            || SUMMARY_HEADS.contains(&low.as_str())
    };
    // "Wireless · 8", "Guest · 12 devices": a count of a kind, not a device.
    let summary =
        ip.is_none() && mac.is_none() && (tag(head) || fields.iter().any(|f| is_count(f)));
    let device = !summary
        && (ip.is_some()
            || mac.is_some()
            || (fields.len() > 1 && one_word(head))
            || (fields.len() == 1 && one_word(head) && !head.ends_with(':')));
    if !device {
        return None;
    }
    let names_it = |f: &str| first_ip(f).is_some() || first_mac([f]).is_some();
    let name = Some(head).filter(|h| !names_it(h)).map(str::to_string);
    let mut vendor = None;
    let mut guest = false;
    for f in fields.iter().skip(usize::from(name.is_some())) {
        let low = f.to_lowercase();
        if names_it(f) {
            continue;
        }
        if low == "guest" || low.starts_with("guest ") {
            guest = true;
        } else if LISTED_TAGS
            .iter()
            .any(|t| low == *t || low.starts_with(&format!("{t} ")))
        {
        } else if vendor.is_none() && vendor_like(f) {
            vendor = crate::map::export::clean_vendor(f);
        }
    }
    Some(Listed {
        line: line.to_string(),
        name,
        ip,
        mac,
        vendor,
        guest,
    })
}

/// Where a "+N more" card's devices are, to find a line's device again:
/// by MAC; by address, when the names agree (or one is missing); by
/// name, when the address and MAC don't disagree.
#[derive(Default)]
struct Known {
    mac: HashMap<Mac, usize>,
    /// The first device at each address.
    ip: HashMap<Ipv4Addr, usize>,
    ip_named: HashMap<(Ipv4Addr, String), usize>,
    /// A device at the address with no name yet; dropped once it has one,
    /// so a second name there is a second device.
    ip_unnamed: HashMap<Ipv4Addr, usize>,
    name: HashMap<String, usize>,
}

impl Known {
    fn find(&self, hosts: &[Listed], h: &Listed) -> Option<usize> {
        let agree = |a: Option<Mac>, b: Option<Mac>| a.is_none() || b.is_none() || a == b;
        if let Some(&k) = h.mac.and_then(|m| self.mac.get(&m)) {
            return Some(k);
        }
        let name = h.name.as_deref().map(str::to_lowercase);
        if let Some(ip) = h.ip {
            let at = match &name {
                None => self.ip.get(&ip),
                Some(n) => self
                    .ip_named
                    .get(&(ip, n.clone()))
                    .or_else(|| self.ip_unnamed.get(&ip)),
            };
            if let Some(&k) = at.filter(|&&k| agree(hosts[k].mac, h.mac)) {
                return Some(k);
            }
        }
        let &k = self.name.get(name.as_ref()?)?;
        let o = &hosts[k];
        ((o.ip.is_none() || h.ip.is_none() || o.ip == h.ip) && agree(o.mac, h.mac)).then_some(k)
    }

    /// Index device `k` by what it has now.
    fn index(&mut self, hosts: &[Listed], k: usize) {
        let o = &hosts[k];
        if let Some(m) = o.mac {
            self.mac.entry(m).or_insert(k);
        }
        let name = o.name.as_deref().map(str::to_lowercase);
        if let Some(ip) = o.ip {
            self.ip.entry(ip).or_insert(k);
            match &name {
                Some(n) => {
                    self.ip_named.entry((ip, n.clone())).or_insert(k);
                    if self.ip_unnamed.get(&ip) == Some(&k) {
                        self.ip_unnamed.remove(&ip);
                    }
                }
                None => {
                    self.ip_unnamed.entry(ip).or_insert(k);
                }
            }
        }
        if let Some(n) = name {
            self.name.entry(n).or_insert(k);
        }
    }
}

/// A second line about the same device fills in what the first left out.
fn merge_listed(o: &mut Listed, h: Listed) {
    o.vendor = o.vendor.take().or(h.vendor);
    o.guest |= h.guest;
    o.mac = o.mac.or(h.mac);
    o.ip = o.ip.or(h.ip);
    if o.name.is_none() {
        o.name = h.name;
    }
}

/// Whether a line of a card's details is about its link (said where the
/// card hangs), not a note to copy onto each device it lists.
fn about_its_link(line: &str) -> bool {
    [
        "Link on the map:",
        "Its own port:",
        "Uplink ",
        "Placement unknown",
        "Linked by a tunnel",
    ]
    .iter()
    .any(|p| line.starts_with(p))
}

/// Turn the overflow card `c` into a client card for each device it lists
/// ("laptop-jsmith · 192.168.1.42", "client-0badf00d · Hikvision ·
/// guest", a bare name), under the same uplink (not on its port: that was
/// the card's), so the reveal steps through them like any other clients.
/// Its notes go on each, with the vendor and guest tag its line names. A
/// device listed twice, or already on the map by address or MAC, is one
/// card. The first device takes the card's place, unless the card also
/// says "…and N more": then it stays, as that count, saying the map
/// doesn't list them. Without device lines it stays one card, revealed
/// last. `after`: past the rank of every card from the map. Returns how
/// many the card only counts.
///
/// A /mapnetwork card lists a handful. A crafted page could list hundreds
/// of thousands, each card copying every note line: so at most
/// `OVERFLOW_HOSTS` cards and `OVERFLOW_NOTES` notes each, and never past
/// the map's own `NODES_MAX` in all.
fn expand_overflow(b: &mut B, c: usize, after: u64) -> usize {
    const OVERFLOW_HOSTS: usize = 1_000;
    const OVERFLOW_NOTES: usize = 20;
    let n = &b.nodes[c];
    if !is_overflow(n) {
        return 0;
    }
    // Cards it may add: the first device takes the card's own place, so
    // it costs nothing, unless the card stays as "…and N more".
    let keep_card = n
        .details
        .iter()
        .any(|d| unlisted_count(d).is_some_and(|k| k > 0));
    let room = crate::mapnetwork::NODES_MAX
        .saturating_sub(b.nodes.len())
        .saturating_add(usize::from(!keep_card))
        .min(OVERFLOW_HOSTS);
    let mut unlisted = 0usize;
    // The devices drawn, and those past the cap (only counted, but once
    // each), each found again by `Known`: maps, not a scan per line.
    let (mut hosts, mut known) = (Vec::<Listed>::new(), Known::default());
    let (mut extra, mut known_extra) = (Vec::<Listed>::new(), Known::default());
    let mut notes: Vec<String> = Vec::new();
    let mut notes_left_out = 0usize;
    let mut elsewhere: Vec<(usize, String)> = Vec::new();
    for d in &n.details {
        if let Some(k) = unlisted_count(d) {
            unlisted = unlisted.saturating_add(k);
            continue;
        }
        let Some(h) = listed(d) else {
            if about_its_link(d) {
            } else if notes.len() < OVERFLOW_NOTES {
                notes.push(d.clone());
            } else {
                notes_left_out += 1;
            }
            continue;
        };
        if let Some(k) = known.find(&hosts, &h) {
            merge_listed(&mut hosts[k], h);
            known.index(&hosts, k);
            continue;
        }
        if let Some(j) = b.find(h.mac, h.ip).filter(|&j| {
            // The same device: its MAC, or its address with no other
            // name. A differently named device at a card's address is an
            // address clash, its own card.
            let o = &b.nodes[j];
            j != c
                && (h.mac.is_some_and(|m| o.mac == Some(m))
                    || (h.mac.is_none()
                        && h.name
                            .as_deref()
                            .is_none_or(|n| n.eq_ignore_ascii_case(&o.label))))
        }) {
            if elsewhere.len() < OVERFLOW_HOSTS {
                elsewhere.push((j, h.line));
            }
            continue;
        }
        let (list, idx) = if hosts.len() < room {
            (&mut hosts, &mut known)
        } else {
            (&mut extra, &mut known_extra)
        };
        match idx.find(list, &h) {
            Some(k) => {
                merge_listed(&mut list[k], h);
                idx.index(list, k);
            }
            None => {
                list.push(h);
                idx.index(list, list.len() - 1);
            }
        }
    }
    let left_out = extra.len();
    let label = n.label.clone();
    for (j, line) in elsewhere {
        b.nodes[j].detail(format!(
            "Also listed in \"{label}\" on the /mapnetwork map: {line}"
        ));
    }
    let n = &b.nodes[c];
    let chips: Vec<String> = n
        .chips
        .iter()
        .filter(|c| *c != "collapsed" && !c.ends_with(" devices"))
        .take(8)
        .cloned()
        .collect();
    let up = n.up.as_ref().map(|u| (u.0, u.1, None));
    // Two devices can share an address (a clash) or a name: their ids
    // can't.
    let mut ids: HashSet<String> = HashSet::new();
    let mut devices: Vec<Node> = hosts
        .iter()
        .enumerate()
        .map(|(k, h)| {
            let mut id = format!(
                "{}:{}",
                n.id,
                match (h.ip, h.mac, &h.name) {
                    (Some(ip), ..) => ip.to_string(),
                    (None, Some(m), _) => mac::fmt(&m),
                    (None, None, Some(name)) => name.to_lowercase(),
                    _ => k.to_string(),
                }
            );
            if !ids.insert(id.clone()) {
                id = format!("{id}:{k}");
                ids.insert(id.clone());
            }
            let mut d = Node {
                id,
                zone: Zone::Client,
                kind: "CLIENT".into(),
                label: match (&h.name, h.ip, h.mac) {
                    (Some(name), ..) => name.clone(),
                    (None, Some(ip), _) => format!("Host {ip}"),
                    (None, None, Some(m)) => mac::fmt(&m),
                    _ => h.line.clone(),
                },
                sub: h.ip.map(|i| i.to_string()).unwrap_or_default(),
                ip: h.ip,
                mac: h.mac,
                chips: chips.clone(),
                client: true,
                up: up.clone(),
                site: n.site.clone(),
                seen_by: n.seen_by.clone(),
                // After the map's own clients, in the map's order.
                rank: (2, after + k as u64),
                ..Default::default()
            };
            d.details.push(h.line.clone());
            if let Some(v) = &h.vendor {
                d.details.push(format!("Vendor {v}"));
            }
            if h.guest {
                d.chip("guest");
            }
            d.details.extend(notes.iter().cloned());
            d.details
                .push(format!("In \"{label}\" on the /mapnetwork map"));
            d
        })
        .collect();
    let mut lines = Vec::new();
    if left_out > 0 {
        lines.push(format!(
            "{left_out} more listed in \"{label}\" aren't drawn"
        ));
    }
    if notes_left_out > 0 {
        lines.push(format!(
            "{notes_left_out} more lines of notes in \"{label}\" aren't copied"
        ));
    }
    if keep_card {
        let listed_n = devices.len();
        let card = &mut b.nodes[c];
        card.label = format!("+{unlisted} not listed");
        card.details
            .retain(|d| unlisted_count(d).is_none() && listed(d).is_none());
        card.details.insert(
            0,
            format!(
                "\"{label}\" on the /mapnetwork map counts {unlisted} more device(s) it doesn't \
                 list: is-alive can't draw what isn't named ({listed_n} it does list are \
                 their own cards)"
            ),
        );
        card.details.extend(lines);
        for d in devices {
            b.add(d);
        }
        return unlisted;
    }
    if devices.is_empty() {
        return 0;
    }
    let mut first = devices.remove(0);
    first.details.extend(lines);
    b.nodes[c] = first;
    if let Some(m) = b.nodes[c].mac {
        b.by_mac.entry(m).or_insert(c);
    }
    if let Some(ip) = b.nodes[c].ip {
        b.by_ip.entry(ip).or_insert(c);
    }
    for d in devices {
        b.add(d);
    }
    0
}

/// The /24 of an address.
fn net24(ip: Ipv4Addr) -> [u8; 3] {
    let o = ip.octets();
    [o[0], o[1], o[2]]
}

/// The /24s a line of text names: "192.168.2.x", or "10.0.0.0/24" (or a
/// smaller block). Wider ones say too little to place a host by.
fn nets_in(text: &str) -> Vec<[u8; 3]> {
    let mut out = Vec::new();
    for w in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '/')) {
        let w = w.to_ascii_lowercase();
        let net = if let Some(head) = w.strip_suffix(".x") {
            let o: Vec<u8> = head.split('.').filter_map(|p| p.parse().ok()).collect();
            (o.len() == 3 && head.split('.').count() == 3).then(|| [o[0], o[1], o[2]])
        } else if let Some((ip, bits)) = w.split_once('/') {
            match (ip.parse::<Ipv4Addr>(), bits.parse::<u8>()) {
                (Ok(ip), Ok(24..=32)) => Some(net24(ip)),
                _ => None,
            }
        } else {
            None
        };
        if let Some(n) = net.filter(|n| !out.contains(n)) {
            out.push(n);
        }
    }
    out
}

fn is_infra(z: Zone) -> bool {
    matches!(
        z,
        Zone::Gateway | Zone::Switch | Zone::Wireless | Zone::Gear
    )
}

/// Where a /24's hosts most likely plug in, for the sweep's hosts and for
/// the UniFi backup's clients.
struct Hints {
    sweep: HashMap<[u8; 3], usize>,
    unifi: HashMap<[u8; 3], usize>,
}

/// The sites of the cards `at`.
fn sites_of(b: &B, at: &[usize]) -> HashSet<Option<String>> {
    at.iter().map(|&i| b.nodes[i].site.clone()).collect()
}

/// For each /24 the map shows, the card a host from it most likely hangs
/// off: the gateway, switch or AP the map's own cards in that subnet hang
/// off (by their address, or their zone's "VLAN 20 · 192.168.2.x"), else
/// one in the subnet itself; on a tie, the one higher up the file. Only
/// the map's own cards at `sites` count; all of them when that names no
/// site.
fn subnet_uplinks(
    b: &B,
    zone_text: &[String],
    sites: &HashSet<Option<String>>,
) -> HashMap<[u8; 3], usize> {
    let any = sites.is_empty() || sites.contains(&None);
    let mut votes: HashMap<[u8; 3], HashMap<usize, usize>> = HashMap::new();
    for (i, n) in b.nodes.iter().enumerate().take(zone_text.len()) {
        if !any && !sites.contains(&n.site) {
            continue;
        }
        let mut nets = nets_in(zone_text.get(i).map_or("", String::as_str));
        if let Some(ip) = n.ip
            && !nets.contains(&net24(ip))
        {
            nets.push(net24(ip));
        }
        for net in nets {
            let v = votes.entry(net).or_default();
            if is_infra(n.zone) {
                *v.entry(i).or_default() += 1;
            }
            // A firewall's own uplink is the WAN side, not its subnet's.
            if n.zone != Zone::Gateway
                && let Some(p) = n.parent()
                && is_infra(b.nodes[p].zone)
            {
                *v.entry(p).or_default() += 2;
            }
        }
    }
    // A card the map itself marks inferred (a scan-only switch) never
    // outvotes one it confirms, however many hosts hang off it.
    votes
        .into_iter()
        .filter_map(|(net, v)| {
            let best = v
                .into_iter()
                .max_by_key(|&(i, n)| (!speculative(&b.nodes[i]), n, std::cmp::Reverse(i)))?;
            Some((net, best.0))
        })
        .collect()
}

/// A card the map drew as a maybe: its role inferred, seen only by a scan,
/// or placed nowhere.
fn speculative(n: &Node) -> bool {
    n.role_inferred()
        || n.zone == Zone::Unknown
        || n.chips.iter().any(|c| {
            let c = c.to_lowercase();
            c == "scan-only" || c == "unconfirmed" || c == "external"
        })
}

/// Link each UniFi device to its card on the map (by MAC, else by name,
/// else by address), so its clients find their switch or AP. Returns the
/// cards it found by MAC or name: none when the backup is of another site.
/// `cards`: how many cards came from the map.
fn join_unifi(b: &mut B, u: &Unifi, cards: usize) -> Vec<usize> {
    let mut links = Vec::new();
    let mut same = Vec::new();
    for d in &u.devices {
        let by_name = || (0..cards).find(|&i| b.nodes[i].label.eq_ignore_ascii_case(&d.name));
        match b.by_mac.get(&d.mac).copied().or_else(by_name) {
            Some(i) => {
                same.push(i);
                links.push((d.mac, i));
            }
            None => {
                if let Some(&i) = d.ip.and_then(|ip| b.by_ip.get(&ip)) {
                    links.push((d.mac, i));
                }
            }
        }
    }
    if !same.is_empty() {
        for (m, i) in links {
            b.by_mac.entry(m).or_insert(i);
        }
    }
    same
}

/// The cards from `first` on are hosts the map left out: hang each under
/// its UniFi uplink, else the card its subnet hangs off at the site its
/// source was found at, else a "Not placed" card, and put it at its
/// uplink's site. Returns how many went under Not placed. Each stays a
/// client or not as its own source says: a switch or NAS the sweep found
/// is drawn whatever the Clients level.
fn place_added(b: &mut B, first: usize, hints: &Hints) -> usize {
    let mut unplaced = Vec::new();
    for i in first..b.nodes.len() {
        let n = &mut b.nodes[i];
        let unifi = n.seen_by.contains(&"UniFi");
        n.chip("not on map");
        n.details.insert(
            0,
            format!(
                "Not on the /mapnetwork map: added from {}",
                if unifi {
                    "the UniFi backup"
                } else {
                    "the sweep"
                }
            ),
        );
        if n.up.is_none() {
            let hints = if unifi { &hints.unifi } else { &hints.sweep };
            match n.ip.and_then(|ip| hints.get(&net24(ip))) {
                Some(&p) => {
                    n.up = Some((p, Link::Assumed, None));
                    n.details.push(crate::inferred::UPLINK_SUBNET.into());
                }
                None => {
                    unplaced.push(i);
                    continue;
                }
            }
        }
        if let Some(p) = b.nodes[i].parent() {
            b.nodes[i].site = b.nodes[p].site.clone();
        }
    }
    if unplaced.is_empty() {
        return 0;
    }
    let np = b.add(Node {
        id: "mapnetwork:not-placed".into(),
        zone: Zone::Unknown,
        kind: "NOT PLACED".into(),
        label: "Not placed".into(),
        sub: format!("{} host(s), uplink unknown", unplaced.len()),
        details: vec![
            "Hosts the sweep or the UniFi backup found that this map doesn't show, with \
             nothing to say where they plug in (no UniFi uplink, and no card on the map in \
             their subnet)"
                .into(),
        ],
        up: Some((0, Link::Assumed, None)),
        ..Default::default()
    });
    for &i in &unplaced {
        b.nodes[i].up = Some((np, Link::Assumed, None));
    }
    unplaced.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{Clients, Seen};

    const PAGE: &str = r#"
    /* ===================== SITE DATA ===================== */
    this.META = { title:'Acme', subtitle:'HQ' };
    this.SITES = { hq:{ name:'Head Office' }, br:{ name:'Branch' } };
    this.ZONES = { core:{ name:'Edge / routing', vlan:'WAN' }, trusted:{ name:'Workstations' } };
    this.NODES = [
      { id:'internet', zone:'core', kind:'WAN', label:'Internet / WAN' },
      { id:'fw', site:'hq', zone:'core', kind:'FIREWALL · HA PAIR', label:'NSA 3700', sub:'10.0.0.1',
        chips:['HA'], flags:['WAN2 untested'], vlans:[10,20], details:['c0:ea:e4:00:00:01'] },
      { id:'fw2', site:'hq', zone:'core', kind:'FIREWALL · STANDBY', label:'NSA standby', sub:'10.0.0.254' },
      { id:'sw', site:'hq', zone:'core', kind:'CORE SWITCH', label:'USW-24', sub:'10.0.0.2' },
      { id:'ap', site:'hq', zone:'core', kind:'WIFI AP', label:'AP Lobby', sub:'10.0.0.10' },
      { id:'ws', site:'hq', zone:'trusted', kind:'WORKSTATION', label:'WS-1', sub:'10.0.1.5 (WiFi)' },
      { id:'lonely', site:'br', zone:'mystery', kind:'THING', label:'Odd box' },
    ];
    this.EDGES = [
      { id:'w1', from:'internet', to:'fw', cls:'backbone', label:'AT&T' },
      { id:'w2', from:'internet', to:'fw', cls:'backbone', dashed:true },
      { id:'ha', from:'fw', to:'fw2', cls:'wired', label:'HA sync' },
      { id:'m1', from:'fw', to:'sw', cls:'backbone', label:'LAG' },
      { id:'m2', from:'sw', to:'ap', cls:'wired', label:'PoE' },
      { id:'m3', from:'ap', to:'ws', cls:'wifi', dashed:true },
      { id:'x', from:'fw2', to:'sw', cls:'wired', label:'standby' },
      { id:'bad', from:'nope', to:'sw' },
    ];
    this.ACTIONS = [ { sev:'high', text:'Test failover', nodes:['fw','gone'] }, { sev:'low', text:'Tidy', nodes:[] } ];
    /* =================== END SITE DATA =================== */"#;

    fn by<'a>(m: &'a Map, id: &str) -> &'a Node {
        m.nodes.iter().find(|n| n.id == id).unwrap()
    }

    #[test]
    fn redraws_a_mapnetwork_map() {
        let doc = crate::mapnetwork::parse(PAGE.as_bytes()).unwrap();
        let m = build(
            &doc,
            &Input {
                clients: Clients::Some,
                ..Default::default()
            },
        );
        assert_eq!(m.nodes[0].id, "internet");
        assert_eq!(m.nodes[0].zone, Zone::Internet);
        let fw = by(&m, "fw");
        assert_eq!(fw.zone, Zone::Gateway);
        assert_eq!(fw.kind, "FIREWALL · HA PAIR");
        assert_eq!(
            fw.up.as_ref().map(|u| (u.0, u.1)),
            Some((0, Link::Backbone))
        );
        assert_eq!(fw.chips, ["Head Office", "HA", "VLANs 10/20"]);
        assert_eq!(fw.ip, Some(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(first_ip("VLAN 10 · 192.168.10.0/24"), None);
        assert_eq!(
            first_ip("10.103.10.51 (WiFi)"),
            Some(Ipv4Addr::new(10, 103, 10, 51))
        );
        assert!(fw.mac.is_some());
        assert!(
            fw.details
                .iter()
                .any(|d| d == "Zone on the map: Edge / routing (WAN)")
        );
        let sw = by(&m, "sw");
        assert_eq!(m.nodes[sw.parent().unwrap()].id, "fw");
        assert_eq!(sw.up.as_ref().unwrap().2.as_deref(), Some("LAG"));
        assert!(
            sw.details
                .iter()
                .any(|d| d == "Also linked from NSA standby (standby)")
        );
        assert_eq!(by(&m, "ap").zone, Zone::Wireless);
        let ws = by(&m, "ws");
        assert!(ws.client);
        assert_eq!(ws.link(), Some(Link::Wireless));
        let lonely = by(&m, "lonely");
        assert_eq!(
            (lonely.zone, lonely.link()),
            (Zone::Unknown, Some(Link::Assumed))
        );
        assert_eq!(m.findings.len(), 2);
        assert_eq!(m.findings[0].sev, Sev::High);
        assert_eq!(m.findings[0].nodes.len(), 1);
        assert!(m.summary.starts_with("Acme · HQ · from /mapnetwork"));

        // Clients off: the workstation folds into a count, as everywhere else.
        let off = build(&doc, &Input::default());
        assert!(off.nodes.iter().all(|n| n.id != "ws"));
        assert!(off.summary.ends_with("not drawn"), "{}", off.summary);
    }

    /// A client on a self-assigned address, or on the address of the
    /// gateway or a switch, is a finding on an imported map too: client
    /// cards and the lines of its "+N more" card alike, clients off or on.
    #[test]
    fn self_assigned_and_clashing_clients_on_an_imported_map() {
        let page = r#"
        /* ===================== SITE DATA ===================== */
        this.META = { title:'Example Site' };
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'fw', kind:'FIREWALL', label:'Edge FW', sub:'192.0.2.1' },
          { id:'sw', kind:'SWITCH', label:'Core SW', sub:'192.0.2.2' },
          { id:'ws', kind:'WORKSTATION', label:'front-desk', sub:'192.0.2.2' },
          { id:'more', kind:'ADDITIONAL DEVICES', label:'+2 more', sub:'via Core SW',
            details:['laptop-b · 192.0.2.1', 'copier-3 · 169.254.10.20'] },
        ];
        this.EDGES = [
          { from:'inet', to:'fw' }, { from:'fw', to:'sw' }, { from:'sw', to:'ws' },
          { from:'sw', to:'more' },
        ];
        /* =================== END SITE DATA =================== */"#;
        let doc = crate::mapnetwork::parse(page.as_bytes()).unwrap();
        for clients in [Clients::All, Clients::Off] {
            let m = build(
                &doc,
                &Input {
                    clients,
                    ..Default::default()
                },
            );
            let texts: Vec<&str> = m.findings.iter().map(|f| f.text.as_str()).collect();
            let gw = m
                .findings
                .iter()
                .find(|f| f.text.contains("Address clash on 192.0.2.1"))
                .expect("laptop-b on the firewall's address");
            assert_eq!(gw.sev, Sev::High, "the gateway's address: worst first");
            assert!(gw.text.contains("laptop-b") && gw.text.contains("Edge FW"));
            let sw = m
                .findings
                .iter()
                .find(|f| f.text.contains("Address clash on 192.0.2.2"))
                .expect("front-desk on the switch's address");
            assert_eq!(sw.sev, Sev::Med);
            assert!(sw.text.contains("front-desk") && sw.text.contains("Core SW"));
            let dhcp = m
                .findings
                .iter()
                .find(|f| f.text.contains("self-assigned"))
                .expect("copier-3 on 169.254.x.x");
            assert!(dhcp.text.starts_with("1 device(s)"), "{texts:?}");
            assert!(dhcp.text.contains("copier-3") && dhcp.text.contains(crate::map::NO_DHCP));
            assert_eq!(m.findings.len(), 3, "{texts:?}");
            // The firewall's card is flagged, drawn at every clients setting.
            assert!(
                by(&m, "fw")
                    .flags
                    .iter()
                    .any(|f| f == "Address clash on 192.0.2.1")
            );
        }
    }

    /// A node's own ip and mac fields win over what its text says; a
    /// malformed one falls back to the text.
    #[test]
    fn a_nodes_ip_and_mac_fields_come_first() {
        let page = r#"
        /* ===================== SITE DATA ===================== */
        this.META = { title:'T' };
        this.ZONES = { core:{ name:'Core' } };
        this.NODES = [
          { id:'internet', zone:'core', kind:'WAN', label:'Internet' },
          { id:'sw', zone:'core', kind:'SWITCH', label:'USW', sub:'Core switch',
            ip:'10.0.0.2', mac:'F4:E2:C6:00:00:02' },
          { id:'ap', zone:'core', kind:'WIFI AP', label:'AP', sub:'10.0.0.10',
            ip:'10.0.0.11', details:['f4:e2:c6:00:00:0a'] },
          { id:'bad', zone:'core', kind:'SWITCH', label:'Old', sub:'10.0.0.3',
            ip:'not an ip', mac:'nope', chips:['f4:e2:c6:00:00:03'] },
        ];
        this.EDGES = [
          { id:'a', from:'internet', to:'sw' }, { id:'b', from:'sw', to:'ap' },
          { id:'c', from:'sw', to:'bad' },
        ];
        this.ACTIONS = [];
        /* =================== END SITE DATA =================== */"#;
        let doc = crate::mapnetwork::parse(page.as_bytes()).unwrap();
        let m = build(&doc, &Input::default());
        let mac = |s: &str| mac::parse_mac(s);
        let sw = by(&m, "sw");
        assert_eq!(
            sw.ip,
            Some(Ipv4Addr::new(10, 0, 0, 2)),
            "nothing in the text"
        );
        assert_eq!(sw.mac, mac("f4:e2:c6:00:00:02"));
        let ap = by(&m, "ap");
        assert_eq!(
            ap.ip,
            Some(Ipv4Addr::new(10, 0, 0, 11)),
            "the field, not sub"
        );
        assert_eq!(ap.mac, mac("f4:e2:c6:00:00:0a"));
        let bad = by(&m, "bad");
        assert_eq!(bad.ip, Some(Ipv4Addr::new(10, 0, 0, 3)));
        assert_eq!(bad.mac, mac("f4:e2:c6:00:00:03"));
    }

    #[test]
    fn sweep_joins_only_on_a_shared_mac() {
        let doc = crate::mapnetwork::parse(PAGE.as_bytes()).unwrap();
        let host = |ip: [u8; 4], mac: Option<Mac>| Seen {
            ip: ip.into(),
            mac,
            name: None,
            rtt: Some(1.0),
            open: Vec::new(),
            checked: true,
            also: vec![],
            web: None,
            multicast: None,
            dnssd: None,
        };
        let look_alike = Input {
            sweep: vec![host([10, 0, 0, 2], Some([2, 0, 0, 0, 0, 9]))],
            ..Default::default()
        };
        let m = build(&doc, &look_alike);
        assert_eq!(by(&m, "sw").seen_by, ["mapnetwork"]);
        assert!(m.summary.contains("sweep not merged") && m.sweep_apart);

        let here = Input {
            sweep: vec![
                host([10, 0, 0, 1], Some([0xc0, 0xea, 0xe4, 0, 0, 1])),
                host([10, 0, 0, 2], None),
                host([10, 0, 0, 99], None),
            ],
            ..Default::default()
        };
        let m = build(&doc, &here);
        assert_eq!(by(&m, "fw").seen_by, ["mapnetwork", "sweep"]);
        assert_eq!(by(&m, "sw").seen_by, ["mapnetwork", "sweep"]);
        assert!(m.summary.contains("1 host(s) not on the map") && !m.sweep_apart);
    }

    #[test]
    fn hosts_the_map_left_out_join_under_their_likely_uplink() {
        let doc = crate::mapnetwork::parse(PAGE.as_bytes()).unwrap();
        let host = |ip: [u8; 4], mac: Option<Mac>| Seen {
            ip: ip.into(),
            mac,
            name: None,
            rtt: Some(1.0),
            open: Vec::new(),
            checked: true,
            also: vec![],
            web: None,
            multicast: None,
            dnssd: None,
        };
        let laptop = [2, 0, 0, 0, 0, 0x99];
        let sweep = vec![
            host([10, 0, 0, 1], Some([0xc0, 0xea, 0xe4, 0, 0, 1])),
            host([10, 0, 0, 99], Some(laptop)),
            host([10, 0, 1, 77], None),
            host([172, 16, 0, 5], None),
        ];
        // The backup's switch is the map's USW-24 by name; its clients
        // were anonymized by Config Export.
        let unifi = serde_json::json!([
            {"__cmd": "select", "collection": "device"},
            {"name": "USW-24", "type": "usw", "mac": "00:00:00:00:00:02", "ip": "10.0.0.2"},
            {"__cmd": "select", "collection": "user"},
            {"mac": "02:00:00:00:00:99", "hostname": "client-0badf00d",
             "last_uplink_mac": "00:00:00:00:00:02", "last_uplink_remote_port": 7, "is_wired": true},
            {"mac": "02:00:00:00:00:42", "last_ip": "10.0.0.150"},
        ])
        .to_string();
        let input = Input {
            sweep,
            unifi: crate::map::one(&unifi),
            clients: Clients::All,
            ..Default::default()
        };
        let m = build(&doc, &input);
        let parent = |n: &Node| m.nodes[n.parent().unwrap()].id.clone();
        let named = |l: &str| m.nodes.iter().find(|n| n.label == l).unwrap();
        // The sweep's laptop is the backup's anonymized client: one card,
        // its token for a name, on its switch port.
        let laptop = named("client-0badf00d");
        assert!(laptop.client);
        assert_eq!(laptop.seen_by, ["UniFi", "sweep"]);
        assert_eq!(parent(laptop), "sw");
        assert_eq!(laptop.up.as_ref().unwrap().1, Link::Wired);
        assert_eq!(laptop.up.as_ref().unwrap().2.as_deref(), Some("port 7"));
        // By subnet: where the map's own cards in it hang.
        let wifi = named("Host 10.0.1.77");
        assert_eq!(parent(wifi), "ap");
        assert_eq!(wifi.link(), Some(Link::Assumed));
        assert_eq!(parent(named("02:00:00:00:00:42")), "fw");
        // Nothing to go by: Not placed.
        let stray = named("Host 172.16.0.5");
        assert_eq!(parent(stray), "mapnetwork:not-placed");
        assert!(stray.chips.contains(&"not on map".to_string()));
        assert!(
            m.summary
                .contains("3 host(s) not on the map added as clients")
        );
        assert!(m.summary.contains("UniFi backup merged, 2 client(s) added"));
        assert!(m.summary.contains("1 not placed"), "{}", m.summary);
        // Clients off: none drawn, each uplink with a count to step open.
        let off = build(
            &doc,
            &Input {
                clients: Clients::Off,
                ..input
            },
        );
        assert!(off.nodes.iter().all(|n| !n.client));
        assert_eq!(off.under.get("fw"), Some(&(0, 1)));
        assert_eq!(off.under.get("mapnetwork:not-placed"), Some(&(0, 1)));
    }

    /// Last visit's map read back, this visit's sweep merged: what the
    /// sweep says now about an address replaces what the page said, so
    /// the lines don't pile up visit after visit.
    #[test]
    fn last_visits_ping_and_ports_give_way_to_this_sweep() {
        let doc = crate::mapnetwork::parse(
            r#"
    this.NODES = [
      { id:'inet', kind:'WAN', label:'Internet' },
      { id:'sw', kind:'SWITCH', label:'SW', sub:'192.168.1.2', mac:'02:00:00:00:00:02',
        details:['Answered ping in 0.36 ms', 'Open ports: 23 Telnet, 80 HTTP',
                 'Also at 192.168.1.9: Answered ping in 2.0 ms', 'Tech note: spare PSU'] },
    ];
    this.EDGES = [{ from:'inet', to:'sw' }];"#
                .as_bytes(),
        )
        .unwrap();
        let swept = |open: Vec<u16>, checked: bool| {
            let input = Input {
                sweep: vec![Seen {
                    ip: [192, 168, 1, 2].into(),
                    mac: mac::parse_mac("02:00:00:00:00:02"),
                    name: None,
                    rtt: Some(0.41),
                    open,
                    checked,
                    also: vec![],
                    web: None,
                    multicast: None,
                    dnssd: None,
                }],
                clients: Clients::All,
                ..Default::default()
            };
            let m = build(&doc, &input);
            m.nodes
                .iter()
                .find(|n| n.label == "SW")
                .unwrap()
                .details
                .clone()
        };
        let lines = |d: &[String], start: &str| -> Vec<String> {
            d.iter().filter(|l| l.starts_with(start)).cloned().collect()
        };
        let d = swept(vec![80], true);
        assert_eq!(
            lines(&d, "Answered ping in "),
            ["Answered ping in 0.41 ms"],
            "{d:?}"
        );
        let ports = lines(&d, "Open ports: ");
        assert_eq!(ports.len(), 1, "{d:?}");
        assert!(ports[0].starts_with("Open ports: 80") && !ports[0].contains("23"));
        // Another address's line, and the note, are left as they were.
        assert!(d.contains(&"Also at 192.168.1.9: Answered ping in 2.0 ms".to_string()));
        assert!(d.contains(&"Tech note: spare PSU".to_string()));
        // Checked and nothing open now (Telnet turned off): the old list goes.
        assert!(lines(&swept(vec![], true), "Open ports: ").is_empty());
        // Not checked this time: last visit's list is all there is.
        assert_eq!(
            lines(&swept(vec![], false), "Open ports: "),
            ["Open ports: 23 Telnet, 80 HTTP"]
        );
    }

    /// Two sites on the same 192.168.1.x, each with its firewall; the
    /// branch's switch has the map's own "+3 more", HQ's one that lists
    /// nobody.
    const TWO: &str = r#"
    this.SITES = { hq:{ name:'HQ' }, br:{ name:'Branch' } };
    this.NODES = [
      { id:'inet', kind:'WAN', label:'Internet' },
      { id:'fwh', site:'hq', kind:'FIREWALL', label:'FW HQ', sub:'192.168.1.1', details:['MAC 00:00:00:00:00:0a'] },
      { id:'swh', site:'hq', kind:'SWITCH', label:'SW HQ', sub:'192.168.1.2' },
      { id:'aph', site:'hq', kind:'WIFI AP', label:'AP HQ', sub:'192.168.1.3' },
      { id:'pc', site:'hq', kind:'WORKSTATION', label:'PC HQ', sub:'192.168.1.20' },
      { id:'crowd', site:'hq', kind:'ADDITIONAL DEVICES', label:'+30 more', sub:'via SW HQ',
        details:['About 30 wifi clients, not listed'] },
      { id:'fwb', site:'br', kind:'FIREWALL', label:'FW Branch', sub:'192.168.1.1', details:['MAC 00:00:00:00:00:0b'] },
      { id:'swb', site:'br', kind:'SWITCH', label:'SW Branch', sub:'192.168.1.4' },
      { id:'swb_overflow', site:'br', kind:'ADDITIONAL DEVICES', label:'+3 more', sub:'via SW Branch',
        chips:['collapsed','3 devices','VLAN 10'],
        details:['laptop-a · 192.168.1.40', '192.168.1.41 · private MAC 4e:06:2d:b7:33:bc',
                 'MAC 38:c6:ce:9d:a0:d0 · ARP only', 'Private MACs are per-network addresses'] },
    ];
    this.EDGES = [
      { from:'inet', to:'fwh' }, { from:'inet', to:'fwb' }, { from:'fwh', to:'swh' },
      { from:'swh', to:'aph' }, { from:'swh', to:'pc' }, { from:'swh', to:'crowd' },
      { from:'fwb', to:'swb' }, { from:'swb', to:'swb_overflow' },
    ];"#;

    #[test]
    fn a_sweep_at_one_site_places_hosts_there() {
        let doc = crate::mapnetwork::parse(TWO.as_bytes()).unwrap();
        let host = |last: u8, mac: Option<Mac>, open: Vec<u16>| Seen {
            ip: [192, 168, 1, last].into(),
            mac,
            name: None,
            rtt: Some(1.0),
            open,
            checked: true,
            also: vec![],
            web: None,
            multicast: None,
            dnssd: None,
        };
        // Swept at the branch: its firewall answers by MAC.
        let input = Input {
            sweep: vec![
                host(1, Some([0, 0, 0, 0, 0, 0x0b]), vec![]),
                host(50, None, vec![]),
                host(60, None, vec![23, 5000]),
            ],
            clients: Clients::All,
            ..Default::default()
        };
        let m = build(&doc, &input);
        let named = |l: &str| m.nodes.iter().find(|n| n.label == l).unwrap();
        let parent = |n: &Node| m.nodes[n.parent().unwrap()].id.as_str();
        let new = named("Host 192.168.1.50");
        assert_eq!(parent(new), "swb", "under the branch's gear, not HQ's");
        // In its uplink's site box, in Sites.
        assert_eq!(new.site.as_deref(), Some("Branch"));
        // Network gear the sweep found stays a full card, drawn at Off too.
        let gear = m.nodes.iter().find(|n| n.open.contains(&23)).unwrap();
        assert_eq!((gear.zone, gear.client), (Zone::Gear, false));
        assert!(gear.chips.contains(&"not on map".to_string()));
        let off = build(
            &doc,
            &Input {
                clients: Clients::Off,
                ..input
            },
        );
        assert!(off.nodes.iter().any(|n| n.open.contains(&23)));
        // A +N more card sits in its uplink's site.
        let step: HashMap<String, usize> = [("swb".to_string(), 1)].into();
        let m = build(
            &doc,
            &Input {
                clients: Clients::Off,
                reveal: Some(&step),
                ..Default::default()
            },
        );
        let more = m.nodes.iter().find(|n| n.id == "more:swb").unwrap();
        assert_eq!(more.site.as_deref(), Some("Branch"));
    }

    /// A crafted page whose "+N more" card lists far more devices than any
    /// map, and as many notes: each line used to become a card holding
    /// every note (5,000 × 5,000 strings here). Capped, and said so.
    #[test]
    fn a_huge_more_card_is_capped() {
        let hosts: Vec<String> = (0..5_000)
            .map(|i| format!("'h{i} · 10.0.{}.{}'", i >> 8, i & 255))
            .collect();
        // Three notes of a megabyte first: each card copies its notes.
        let notes: Vec<String> = (0..3)
            .map(|_| format!("'{}'", "n".repeat(1 << 20)))
            .chain((0..5_000).map(|i| format!("'note {i}'")))
            .collect();
        let page = format!(
            "this.NODES = [ {{ id:'sw', kind:'SWITCH', label:'SW', sub:'10.9.0.2' }},\n\
             {{ id:'more', kind:'ADDITIONAL DEVICES', label:'+5000 more', details:[{}, {}] }} ];\n\
             this.EDGES = [ {{ from:'sw', to:'more' }} ];",
            hosts.join(","),
            notes.join(",")
        );
        let doc = crate::mapnetwork::parse(page.as_bytes()).unwrap();
        let m = build(
            &doc,
            &Input {
                clients: Clients::All,
                ..Default::default()
            },
        );
        let cards = m.nodes.iter().filter(|n| n.client).count();
        assert!((1..=1_000).contains(&cards), "{cards} cards");
        assert!(m.nodes.iter().all(|n| n.details.len() <= 25));
        assert!(m.nodes.iter().any(|n| {
            n.details
                .iter()
                .any(|d| d == "4000 more listed in \"+5000 more\" aren't drawn")
        }));
        assert!(m.nodes.iter().any(|n| {
            n.details
                .iter()
                .any(|d| d == "4983 more lines of notes in \"+5000 more\" aren't copied")
        }));
        // Bytes, not just counts: the megabyte notes are cut when read.
        let bytes: usize = m
            .nodes
            .iter()
            .flat_map(|n| &n.details)
            .map(String::len)
            .sum();
        assert!(bytes < 8 << 20, "{bytes} bytes of details");
    }

    /// Thousands of links between the same two cards: a few lines each,
    /// then a count, not a copy of a label per link.
    #[test]
    fn many_links_between_two_cards() {
        let edges = vec!["{ from:'a', to:'b' }"; 5_000].join(",");
        let page = format!(
            "this.NODES = [ {{ id:'a', kind:'SWITCH', label:'A' }}, \
             {{ id:'b', kind:'SWITCH', label:'B' }} ];\nthis.EDGES = [ {edges} ];"
        );
        let doc = crate::mapnetwork::parse(page.as_bytes()).unwrap();
        let m = build(&doc, &Input::default());
        for n in m.nodes.iter().filter(|n| n.label == "A" || n.label == "B") {
            let also = n
                .details
                .iter()
                .filter(|d| d.starts_with("Also linked"))
                .count();
            assert!(also <= 21, "{also} lines on {}", n.label);
        }
        assert!(
            m.nodes
                .iter()
                .any(|n| n.details.iter().any(|d| d == "Also linked 4979 more times"))
        );
    }

    #[test]
    fn the_maps_own_more_card_becomes_its_devices() {
        let doc = crate::mapnetwork::parse(TWO.as_bytes()).unwrap();
        let m = build(
            &doc,
            &Input {
                clients: Clients::All,
                ..Default::default()
            },
        );
        let under = |id: &str| -> Vec<&Node> {
            m.nodes
                .iter()
                .filter(|n| n.parent().is_some_and(|p| m.nodes[p].id == id))
                .collect()
        };
        // One client card per device it lists, by address or MAC; the
        // note goes on each, the "collapsed" chips don't.
        let devices = under("swb");
        let labels: Vec<&str> = devices.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(
            labels,
            ["laptop-a", "Host 192.168.1.41", "38:c6:ce:9d:a0:d0"]
        );
        let laptop = devices[0];
        assert!(laptop.client && laptop.zone == Zone::Client);
        assert_eq!(laptop.ip, Some([192, 168, 1, 40].into()));
        assert_eq!(laptop.site.as_deref(), Some("Branch"));
        assert_eq!(laptop.chips, ["Branch", "VLAN 10"]);
        assert!(laptop.details.iter().any(|d| d.starts_with("Private MACs")));
        assert!(devices[1].mac.is_some());
        assert_eq!(m.under.get("swb"), Some(&(3, 3)));
        // One that lists nobody stays one card, revealed after the real
        // clients.
        let m = build(
            &doc,
            &Input {
                clients: Clients::Off,
                reveal: Some(&[("swh".to_string(), 1)].into()),
                ..Default::default()
            },
        );
        assert!(m.nodes.iter().any(|n| n.label == "PC HQ"));
        assert!(m.nodes.iter().all(|n| n.label != "+30 more"));
        assert_eq!(by(&m, "more:swh").label, "+1 more");
    }

    #[test]
    fn a_backup_of_another_site_does_not_join() {
        let doc = crate::mapnetwork::parse(PAGE.as_bytes()).unwrap();
        let unifi = serde_json::json!([
            {"__cmd": "select", "collection": "device"},
            {"name": "Elsewhere", "type": "usw", "mac": "00:00:00:00:00:77", "ip": "10.0.0.2"},
            {"__cmd": "select", "collection": "user"},
            {"mac": "02:00:00:00:00:42", "last_uplink_mac": "00:00:00:00:00:77"},
        ])
        .to_string();
        let m = build(
            &doc,
            &Input {
                unifi: crate::map::one(&unifi),
                clients: Clients::All,
                ..Default::default()
            },
        );
        assert!(m.nodes.iter().all(|n| n.label != "02:00:00:00:00:42"));
        assert!(m.summary.contains("UniFi backup not merged"));
    }

    /// Two backups open: the one of this map's site joins, the other's
    /// clients stay out.
    #[test]
    fn of_two_backups_only_this_sites_joins() {
        let doc = crate::mapnetwork::parse(PAGE.as_bytes()).unwrap();
        let here = serde_json::json!([
            {"__cmd": "select", "collection": "device"},
            {"name": "USW-24", "type": "usw", "mac": "00:00:00:00:00:02", "ip": "10.0.0.2"},
            {"__cmd": "select", "collection": "user"},
            {"mac": "02:00:00:00:00:99", "hostname": "front-desk",
             "last_uplink_mac": "00:00:00:00:00:02", "is_wired": true},
        ])
        .to_string();
        let elsewhere = serde_json::json!([
            {"__cmd": "select", "collection": "device"},
            {"name": "Elsewhere", "type": "usw", "mac": "00:00:00:00:00:77", "ip": "192.0.2.2"},
            {"__cmd": "select", "collection": "user"},
            {"mac": "02:00:00:00:00:42", "hostname": "other-desk",
             "last_uplink_mac": "00:00:00:00:00:77"},
        ])
        .to_string();
        let files = |a: &str, b: &str| {
            build(
                &doc,
                &Input {
                    unifi: vec![
                        crate::map::Loaded {
                            name: "here.unf",
                            json: a,
                        },
                        crate::map::Loaded {
                            name: "elsewhere.unf",
                            json: b,
                        },
                    ],
                    clients: Clients::All,
                    ..Default::default()
                },
            )
        };
        let m = files(&here, &elsewhere);
        assert!(m.nodes.iter().any(|n| n.label == "front-desk"));
        assert!(m.nodes.iter().all(|n| n.label != "other-desk"));
        assert!(
            m.summary.contains(
                "1 of 2 UniFi backups merged, 1 client(s) added (the others' devices aren't on \
                 the map)"
            ),
            "{}",
            m.summary
        );
        let m = files(&elsewhere, &elsewhere);
        assert!(
            m.summary.contains("UniFi backups not merged"),
            "{}",
            m.summary
        );
    }

    #[test]
    fn subnets_named_in_zone_text() {
        assert_eq!(nets_in("VLAN 20 · 192.168.2.x"), [[192, 168, 2]]);
        assert_eq!(
            nets_in("10.0.0.0/24 and 10.1.0.0/16, 10.0.0.9/32"),
            [[10, 0, 0]]
        );
        assert!(nets_in("192.168.10/11.x · WAN").is_empty());
    }

    #[test]
    fn synthesizes_the_internet_without_one() {
        let doc = crate::mapnetwork::parse(
            b"this.NODES = [ {id:'gw', kind:'GATEWAY', label:'UDM'}, {id:'s', kind:'SWITCH', label:'S'} ];\
              this.EDGES = [ {from:'gw', to:'s'}, {from:'s', to:'gw'} ];",
        )
        .unwrap();
        let m = build(&doc, &Input::default());
        assert_eq!(m.nodes[0].zone, Zone::Internet);
        assert_eq!(by(&m, "gw").link(), Some(Link::Backbone));
        assert_eq!(m.nodes[by(&m, "s").parent().unwrap()].id, "gw");
    }

    #[test]
    fn a_dashed_wired_link_is_assumed_not_wireless() {
        // How /mapnetwork draws an uplink a sweep could only infer.
        let doc = crate::mapnetwork::parse(
            b"this.NODES = [ {id:'gw', kind:'GATEWAY', label:'GW'}, {id:'pc', kind:'PC', label:'PC'},\
              {id:'ap', kind:'AP', label:'AP'}, {id:'ph', kind:'PHONE', label:'Phone'} ];\
              this.EDGES = [ {from:'gw', to:'pc', cls:'wired', dashed:true, label:'inferred'},\
              {from:'gw', to:'ap', cls:'wired'}, {from:'ap', to:'ph', cls:'wifi', dashed:true} ];",
        )
        .unwrap();
        let m = build(
            &doc,
            &Input {
                clients: Clients::Some,
                ..Default::default()
            },
        );
        let pc = by(&m, "pc");
        assert_eq!(pc.link(), Some(Link::Assumed));
        // "inferred" isn't a port: it says why, in the card's details.
        assert_eq!(pc.up.as_ref().unwrap().2, None);
        assert!(pc.details.iter().any(|d| d == "Link on the map: inferred"));
        assert_eq!(
            crate::inferred::uplink_why(&pc.details),
            crate::inferred::UPLINK_MAP
        );
        assert_eq!(by(&m, "ap").link(), Some(Link::Wired));
        assert_eq!(by(&m, "ph").link(), Some(Link::Wireless));
    }
}

/// Bugs from two site visits, each on a fictional map: documentation
/// addresses, made-up names, locally administered MACs.
#[cfg(test)]
mod visits {
    use super::*;
    use crate::map::{Clients, Seen};

    fn by<'a>(m: &'a Map, id: &str) -> &'a Node {
        m.nodes.iter().find(|n| n.id == id).expect(id)
    }

    fn all(page: &str) -> Map {
        build(
            &crate::mapnetwork::parse(page.as_bytes()).unwrap(),
            &Input {
                clients: Clients::All,
                ..Default::default()
            },
        )
    }

    fn host(ip: [u8; 4], mac: Option<Mac>) -> Seen {
        Seen {
            ip: ip.into(),
            mac,
            name: None,
            rtt: Some(1.0),
            open: Vec::new(),
            checked: true,
            also: vec![],
            web: None,
            multicast: None,
            dnssd: None,
        }
    }

    /// 1, 2: every client a "+N more" card lists becomes a card, with
    /// the vendor and guest tag its line names, once each; "…and N more"
    /// stays a count that says so; none takes the parent's port.
    #[test]
    fn a_more_card_keeps_every_listed_client() {
        let m = all(r#"
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'sw', kind:'SWITCH', label:'Core', sub:'192.0.2.2' },
          { id:'sw_overflow', kind:'ADDITIONAL DEVICES', label:'+9 more', sub:'via Core',
            details:['client-0a1b2c3d · Hangzhou Hikvision · guest',
                     'client-0a1b2c3d · Hangzhou Hikvision',
                     'front-desk · 192.0.2.40', 'front-desk-2 · 192.0.2.40 · Dell',
                     'laptop-b · 192.0.2.41 · VLAN 20 · Win11', 'Wireless · 8',
                     'Guest · 12 devices',
                     'kiosk-7', 'Seen in the controller over the last week',
                     '…and 4 more'] },
        ];
        this.EDGES = [ { from:'inet', to:'sw', cls:'backbone' },
                       { from:'sw', to:'sw_overflow', cls:'wired', label:'21 wired' } ];"#);
        let under: Vec<&Node> = m
            .nodes
            .iter()
            .filter(|n| n.parent().is_some_and(|p| m.nodes[p].id == "sw"))
            .collect();
        let labels: Vec<&str> = under.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "+4 not listed",
                "client-0a1b2c3d",
                "front-desk",
                "front-desk-2",
                "laptop-b",
                "kiosk-7"
            ],
            "one card per client (two names at one address are two), the count kept, \
             summary lines not devices"
        );
        let named = |l: &str| *under.iter().find(|n| n.label == l).unwrap();
        // Each line's vendor stays its own; a VLAN or an OS isn't one.
        assert_eq!(crate::map::export::vendor(named("front-desk")), None);
        assert_eq!(
            crate::map::export::vendor(named("front-desk-2")).as_deref(),
            Some("Dell")
        );
        assert_eq!(crate::map::export::vendor(named("laptop-b")), None);
        assert_ne!(named("front-desk").id, named("front-desk-2").id);
        let cam = under[1];
        assert!(cam.chips.contains(&"guest".to_string()));
        assert_eq!(
            crate::map::export::vendor(cam).as_deref(),
            Some("Hangzhou Hikvision")
        );
        // "21 wired" is a count, not a port: no client hangs off port 21.
        for n in &under {
            assert_eq!(n.up.as_ref().unwrap().2, None, "{}", n.label);
        }
        assert!(under[0].details[0].contains("counts 4 more device(s) it doesn't list"));
        assert!(
            m.summary
                .contains("4 client(s) the map counts but doesn't list")
        );
        assert!(
            named("kiosk-7")
                .details
                .iter()
                .any(|d| d.starts_with("Seen in the controller"))
        );
    }

    #[test]
    fn only_a_port_is_read_as_a_port() {
        for (label, port) in [
            ("Port 21", Some("Port 21")),
            ("p21", Some("Port 21")),
            ("port #7", Some("Port 7")),
            ("Gi1/0/21", Some("Gi1/0/21")),
            ("X1", Some("X1")),
            ("eth0", Some("eth0")),
            ("21 wired", None),
            ("VLAN 10 gw", None),
            ("AT&T", None),
            ("LAG", None),
            ("1/1/1", Some("1/1/1")),
            ("uplink 1/1/48", Some("1/1/48")),
            ("A1", Some("A1")),
            ("24", Some("Port 24")),
            ("LAN 2", Some("LAN 2")),
            ("SFP+ 1", Some("SFP+ 1")),
            ("ix0", Some("ix0")),
            ("Po1", Some("Po1")),
            ("port-channel2", Some("port-channel2")),
            ("Building IX 2", None),
            ("LAG x 4", None),
            ("port 1-24", None),
            ("Gi1/0/1-24", None),
            ("Group 2", None),
            ("10/100", None),
            ("LAG x4", None),
            ("3/4/25", None),
            ("LAN 100 Mbps", None),
            ("1/0/24", Some("1/0/24")),
            ("1/24", Some("1/24")),
            ("2/48", Some("2/48")),
            ("1/3/1", Some("1/3/1")),
            ("1/4/2", Some("1/4/2")),
            ("Fiber X1", Some("X1")),
            ("Trunk X2", Some("X2")),
            ("HA X3", Some("X3")),
            ("SFP X1", Some("X1")),
            ("Gi1/0/24 GbE", Some("Gi1/0/24")),
            ("X1 GbE", Some("X1")),
            ("100/1000", None),
        ] {
            assert_eq!(port_in(label).as_deref(), port, "{label}");
        }
    }

    /// 4, 5, 15, 16: a switch nothing links to waits under "Not linked",
    /// not on the internet; each dashed link says why in its own words;
    /// "X2 → Port 24" is the parent's port then the card's own; a backbone
    /// from the firewall's LAN keeps its port; the scanner's public
    /// address stays on the map's own internet card.
    #[test]
    fn links_say_what_is_known() {
        let doc = crate::mapnetwork::parse(
            r#"
        this.NODES = [
          { id:'inet', kind:'INTERNET', label:'Internet', sub:'Carrier fibre' },
          { id:'fw', kind:'FIREWALL', label:'Edge FW', sub:'192.0.2.1' },
          { id:'sw', kind:'CORE SWITCH', label:'Core', sub:'192.0.2.2' },
          { id:'ap', kind:'AP', label:'AP', sub:'192.0.2.3' },
          { id:'old', kind:'SWITCH', label:'Old switch', sub:'192.0.2.4' },
          { id:'rfw', kind:'FIREWALL', label:'Branch FW', sub:'198.51.100.1' },
          { id:'idf', kind:'SWITCH', label:'IDF', sub:'192.0.2.5' },
          { id:'lost', kind:'SWITCH', label:'Mystery switch', sub:'192.0.2.9' },
          { id:'pc', kind:'WORKSTATION', label:'Behind mystery', sub:'192.0.2.60' },
        ];
        this.EDGES = [
          { from:'inet', to:'fw', cls:'backbone', label:'ISP A' },
          { from:'fw', to:'sw', cls:'backbone', label:'X2 → Port 24' },
          { from:'sw', to:'ap', cls:'wired', dashed:true, label:'Port 7' },
          { from:'sw', to:'old', cls:'wired', dashed:true, label:'Port 9 · reciprocal' },
          { from:'fw', to:'rfw', cls:'wired', dashed:true, label:'IPsec to branch' },
          { from:'sw', to:'idf', cls:'wired', dashed:true, label:'VLAN 10 gw' },
          { from:'lost', to:'pc', cls:'wired' },
        ];"#
            .as_bytes(),
        )
        .unwrap();
        let m = build(
            &doc,
            &Input {
                clients: Clients::All,
                wan: Some([203, 0, 113, 7].into()),
                ..Default::default()
            },
        );
        let up = |id: &str| by(&m, id).up.clone().unwrap();
        let why = |id: &str| crate::inferred::uplink_why(&by(&m, id).details).to_string();
        // 4: placement unknown, under the firewall, with its subtree.
        let lost = by(&m, "lost");
        assert_eq!(m.nodes[lost.parent().unwrap()].id, "mapnetwork:not-linked");
        assert_eq!(m.nodes[up("mapnetwork:not-linked").0].id, "fw");
        assert_eq!(why("lost"), crate::inferred::UPLINK_NONE);
        assert_eq!(m.nodes[by(&m, "pc").parent().unwrap()].id, "lost");
        // 15: the parent's port, and the card's own in its details.
        let sw = up("sw");
        assert_eq!((sw.1, sw.2.as_deref()), (Link::Backbone, Some("X2")));
        assert!(
            by(&m, "sw")
                .details
                .contains(&"Its own port: Port 24".to_string())
        );
        // 5: dashed with a port: still inferred (is-alive's own export
        // dashes a hand-drawn link with its port), the port kept.
        assert_eq!(
            (up("ap").1, up("ap").2.as_deref()),
            (Link::Assumed, Some("Port 7"))
        );
        assert_eq!(why("ap"), crate::inferred::UPLINK_MAP);
        assert_eq!(up("old").1, Link::Assumed);
        assert_eq!(why("old"), crate::inferred::UPLINK_RECIPROCAL);
        // A tunnel isn't "it answered here".
        assert_eq!(up("rfw").1, Link::Wired);
        assert!(by(&m, "rfw").details.iter().any(|d| d.contains("tunnel")));
        // "VLAN 10 gw" isn't a port; the map's dashed link says why.
        assert_eq!(up("idf").2, None);
        assert_eq!(why("idf"), crate::inferred::UPLINK_MAP);
        // 16: the public address on the map's own internet card.
        assert!(
            by(&m, "inet")
                .details
                .iter()
                .any(|d| d.contains("203.0.113.7"))
        );
    }

    /// 5, 6, 11: the role from the kind, then the zone, the label last;
    /// network words before voice words; no "inferred" twice.
    #[test]
    fn roles_from_kind_then_zone() {
        let m = all(r#"
        this.ZONES = { voip:{ name:'Voice' }, vedge:{ name:'Voice edge' },
                       vdev:{ name:'Voice & devices' }, ext:{ name:'Unidentified / external' },
                       ot:{ name:'OT devices' } };
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'gw', kind:'SONICWALL INFERRED', label:'Edge', sub:'192.0.2.1' },
          { id:'vr', zone:'vedge', kind:'TA900', label:'Carrier voice' },
          { id:'copier', zone:'vdev', kind:'COPIER', label:'Copier 2F' },
          { id:'ata', zone:'voip', kind:'GRANDSTREAM', label:'Fax ATA' },
          { id:'phone', zone:'voip', kind:'DESK PHONE', label:'Reception' },
          { id:'carrier', zone:'ext', kind:'CARRIER ROUTER', label:'Carrier CPE' },
          { id:'ot1', zone:'ot', kind:'IPC-427', label:'DESKTOP-4QX7' },
          { id:'ot2', zone:'ot', kind:'IPC-427', label:'Line 2 box' },
          { id:'door', zone:'ot', kind:'ACCESS CONTROL', label:'Front door' },
        ];
        this.EDGES = [ { from:'inet', to:'gw' }, { from:'gw', to:'vr' }, { from:'gw', to:'copier' },
          { from:'gw', to:'ata' }, { from:'gw', to:'phone' }, { from:'gw', to:'carrier' },
          { from:'gw', to:'ot1' }, { from:'gw', to:'ot2' }, { from:'gw', to:'door' } ];"#);
        for (id, zone) in [
            ("gw", Zone::Gateway),
            ("vr", Zone::Gateway),
            ("copier", Zone::Device),
            ("ata", Zone::Gear),
            ("phone", Zone::Client),
            ("carrier", Zone::Gateway),
            ("ot1", Zone::Device),
            ("ot2", Zone::Device),
            ("door", Zone::Device),
        ] {
            assert_eq!(by(&m, id).zone, zone, "{id}");
        }
        // Drawn whatever the Clients level: not clients.
        assert!(!by(&m, "copier").client && !by(&m, "vr").client);
        let gw = by(&m, "gw");
        assert_eq!(gw.kind, "SONICWALL");
        assert!(gw.role_inferred());
        assert_eq!(gw.inferred_why(), crate::inferred::ROLE_MAP);
        assert_eq!(Zone::Device.name(), "Printers, cameras, IoT");
    }

    /// 12, 13: a candidate's MAC isn't the card's, so the sweep doesn't
    /// merge that device into it; a speculative switch doesn't outvote the
    /// confirmed core for where the subnet's hosts hang.
    #[test]
    fn maybes_stay_maybes() {
        let doc = crate::mapnetwork::parse(
            r#"
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'fw', kind:'FIREWALL', label:'FW', sub:'192.0.2.1', mac:'02:00:00:00:00:01' },
          { id:'core', kind:'SWITCH', label:'Core', sub:'192.0.2.2' },
          { id:'guess', kind:'SWITCH', label:'Likely switch', chips:['inferred','scan-only'],
            details:['Candidate identity: Dell 02:00:00:00:00:44 unconfirmed'] },
          { id:'a', kind:'PRINTER', label:'P1', sub:'192.0.2.21' },
          { id:'b', kind:'PRINTER', label:'P2', sub:'192.0.2.22' },
          { id:'c', kind:'PRINTER', label:'P3', sub:'192.0.2.23' },
        ];
        this.EDGES = [ { from:'inet', to:'fw' }, { from:'fw', to:'core' }, { from:'core', to:'guess' },
          { from:'guess', to:'a', dashed:true }, { from:'guess', to:'b', dashed:true },
          { from:'guess', to:'c', dashed:true } ];"#
                .as_bytes(),
        )
        .unwrap();
        let dell = [2, 0, 0, 0, 0, 0x44];
        let m = build(
            &doc,
            &Input {
                sweep: vec![
                    host([192, 0, 2, 1], Some([2, 0, 0, 0, 0, 1])),
                    host([192, 0, 2, 44], Some(dell)),
                    host([192, 0, 2, 99], None),
                ],
                clients: Clients::All,
                ..Default::default()
            },
        );
        let guess = by(&m, "guess");
        assert_eq!(guess.mac, None);
        assert!(!guess.seen_by.contains(&"sweep"));
        assert!(
            m.nodes
                .iter()
                .any(|n| n.mac == Some(dell) && n.id != "guess")
        );
        let stray = m
            .nodes
            .iter()
            .find(|n| n.ip == Some([192, 0, 2, 99].into()))
            .unwrap();
        assert_ne!(m.nodes[stray.parent().unwrap()].id, "guess");
        assert!(!own_mac("", &["MAC 02:00:00:00:00:44 · possibly the NAS"]).is_some());
        assert!(own_mac("", &["MAC 02:00:00:00:00:45"]).is_some());
    }

    /// 17, 1: the controller's client at the firewall's address, with
    /// another MAC, is the firewall (no second card); a "+N more" line's
    /// address the controller knows is one card, with its MAC.
    #[test]
    fn the_controller_doesnt_double_a_card() {
        let doc = crate::mapnetwork::parse(
            r#"
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'fw', kind:'FIREWALL', label:'FW', sub:'192.0.2.1', mac:'02:00:00:00:00:01' },
          { id:'sw', kind:'SWITCH', label:'USW-24', sub:'192.0.2.2', mac:'02:00:00:00:00:02' },
          { id:'sw_overflow', kind:'ADDITIONAL DEVICES', label:'+1 more', details:['192.0.2.77'] },
        ];
        this.EDGES = [ { from:'inet', to:'fw' }, { from:'fw', to:'sw' }, { from:'sw', to:'sw_overflow' } ];"#
                .as_bytes(),
        )
        .unwrap();
        let unifi = serde_json::json!([
            {"__cmd": "select", "collection": "device"},
            {"name": "USW-24", "type": "usw", "mac": "02:00:00:00:00:02", "ip": "192.0.2.2"},
            {"__cmd": "select", "collection": "user"},
            {"mac": "02:00:00:00:00:a1", "last_ip": "192.0.2.1", "hostname": "fw-lan"},
            {"mac": "02:00:00:00:00:77", "last_ip": "192.0.2.77", "hostname": "client-77777777",
             "oui": "Fictional Devices"},
        ])
        .to_string();
        let m = build(
            &doc,
            &Input {
                unifi: crate::map::one(&unifi),
                clients: Clients::All,
                ..Default::default()
            },
        );
        assert_eq!(
            m.nodes
                .iter()
                .filter(|n| n.ip == Some([192, 0, 2, 1].into()))
                .count(),
            1
        );
        assert!(
            by(&m, "fw")
                .details
                .iter()
                .any(|d| d.contains("02:00:00:00:00:a1"))
        );
        let at77: Vec<&Node> = m
            .nodes
            .iter()
            .filter(|n| n.ip == Some([192, 0, 2, 77].into()))
            .collect();
        assert_eq!(at77.len(), 1);
        assert_eq!(at77[0].label, "client-77777777");
        assert!(at77[0].mac.is_some());
    }

    fn with_unifi(page: &str, users: serde_json::Value) -> Map {
        // The switch every page has: the backup joins by it.
        let mut rows = vec![
            serde_json::json!({"__cmd": "select", "collection": "device"}),
            serde_json::json!({"name": "Core", "type": "usw", "mac": "02:00:00:00:00:02",
                               "ip": "192.0.2.2"}),
            serde_json::json!({"__cmd": "select", "collection": "user"}),
        ];
        rows.extend(users.as_array().unwrap().iter().cloned());
        let unifi = serde_json::Value::Array(rows).to_string();
        build(
            &crate::mapnetwork::parse(page.as_bytes()).unwrap(),
            &Input {
                unifi: crate::map::one(&unifi),
                clients: Clients::All,
                ..Default::default()
            },
        )
    }

    /// A guest laptop on the firewall's address, or a stale phone on a
    /// printer's, is its own card and the clash is a finding: the
    /// firewall takes no vendor, guest tag or MAC from it.
    #[test]
    fn a_stranger_at_a_cards_address_is_a_clash() {
        let page = r#"
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'fw', kind:'FIREWALL', label:'FW', sub:'192.0.2.1', mac:'02:00:5e:00:00:01' },
          { id:'sw', kind:'SWITCH', label:'Core', sub:'192.0.2.2', mac:'02:00:00:00:00:02' },
          { id:'prn', kind:'PRINTER', label:'Front printer', sub:'192.0.2.30' },
        ];
        this.EDGES = [ { from:'inet', to:'fw' }, { from:'fw', to:'sw' }, { from:'sw', to:'prn' } ];"#;
        let m = with_unifi(
            page,
            serde_json::json!([
                {"mac": "06:11:22:00:00:0a", "last_ip": "192.0.2.1", "hostname": "guest-laptop",
                 "oui": "Fictional Laptops", "is_guest": true},
                {"mac": "06:11:22:00:00:0b", "last_ip": "192.0.2.30", "hostname": "old-phone",
                 "oui": "Fictional Phones"},
            ]),
        );
        let fw = by(&m, "fw");
        assert!(!fw.chips.contains(&"guest".to_string()));
        assert!(!fw.details.iter().any(|d| d.contains("Fictional")));
        assert_eq!(fw.mac, Some([2, 0, 0x5e, 0, 0, 1]));
        // High: the gateway's address.
        assert!(
            m.findings
                .iter()
                .any(|f| f.sev == Sev::High && f.text.starts_with("Address clash on 192.0.2.1")),
            "{:?}",
            m.findings.iter().map(|f| &f.text).collect::<Vec<_>>()
        );
        assert!(
            m.nodes
                .iter()
                .any(|n| n.label == "guest-laptop" && n.client)
        );
        let prn = by(&m, "prn");
        assert_eq!(prn.mac, None);
        assert!(m.nodes.iter().any(|n| n.label == "old-phone" && n.client));
    }

    /// The firewall's own other interface joins it: by its maker's
    /// prefix, or because a config export lists it.
    #[test]
    fn the_firewalls_own_interface_joins_it() {
        let page = r#"
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'fw', kind:'FIREWALL', label:'FW', sub:'192.0.2.1', mac:'02:12:34:00:00:01' },
          { id:'sw', kind:'SWITCH', label:'Core', sub:'192.0.2.2', mac:'02:00:00:00:00:02' },
        ];
        this.EDGES = [ { from:'inet', to:'fw' }, { from:'fw', to:'sw' } ];"#;
        // Another of its interfaces: the same prefix (fictional, locally
        // administered).
        let m = with_unifi(
            page,
            serde_json::json!([{"mac": "02:12:34:00:00:02", "last_ip": "192.0.2.1"}]),
        );
        assert_eq!(
            m.nodes
                .iter()
                .filter(|n| n.ip == Some([192, 0, 2, 1].into()))
                .count(),
            1
        );
        assert!(
            m.findings
                .iter()
                .all(|f| !f.text.starts_with("Address clash"))
        );
        // Another maker's MAC, listed as one of its interfaces in an export.
        let doc = crate::mapnetwork::parse(page.as_bytes()).unwrap();
        let unifi = serde_json::json!([
            {"__cmd": "select", "collection": "device"},
            {"name": "Core", "type": "usw", "mac": "02:00:00:00:00:02", "ip": "192.0.2.2"},
            {"__cmd": "select", "collection": "user"},
            {"mac": "02:aa:bb:00:00:03", "last_ip": "192.0.2.1"},
        ])
        .to_string();
        let export = serde_json::json!({"interfaces": [
            {"name": "X0", "zone": "LAN", "ip": "192.0.2.1", "mac": "02:aa:bb:00:00:03"}
        ]})
        .to_string();
        let without = build(
            &doc,
            &Input {
                unifi: crate::map::one(&unifi),
                clients: Clients::All,
                ..Default::default()
            },
        );
        assert!(
            without
                .findings
                .iter()
                .any(|f| f.text.starts_with("Address clash"))
        );
        let with = build(
            &doc,
            &Input {
                unifi: crate::map::one(&unifi),
                sonicwall: crate::map::one(&export),
                clients: Clients::All,
                ..Default::default()
            },
        );
        assert!(
            with.findings
                .iter()
                .all(|f| !f.text.starts_with("Address clash")),
            "{:?}",
            with.findings.iter().map(|f| &f.text).collect::<Vec<_>>()
        );
    }

    /// With DHCP churn, a "+N more" line's address goes to the client
    /// seen there last, and only when the names agree.
    #[test]
    fn a_listed_line_takes_the_newest_client_at_its_address() {
        let page = r#"
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'sw', kind:'SWITCH', label:'Core', sub:'192.0.2.2', mac:'02:00:00:00:00:02' },
          { id:'sw_overflow', kind:'ADDITIONAL DEVICES', label:'+2 more',
            details:['192.0.2.77', 'scanner-1 · 192.0.2.78'] },
        ];
        this.EDGES = [ { from:'inet', to:'sw' }, { from:'sw', to:'sw_overflow' } ];"#;
        let m = with_unifi(
            page,
            serde_json::json!([
                {"mac": "06:00:00:00:00:01", "last_ip": "192.0.2.77", "hostname": "stale-one",
                 "last_seen": 100},
                {"mac": "06:00:00:00:00:02", "last_ip": "192.0.2.77", "hostname": "today",
                 "last_seen": 900},
                {"mac": "06:00:00:00:00:03", "last_ip": "192.0.2.78", "hostname": "someone-else",
                 "last_seen": 900},
            ]),
        );
        let at = |ip: [u8; 4]| -> Vec<&Node> {
            m.nodes.iter().filter(|n| n.ip == Some(ip.into())).collect()
        };
        let line = at([192, 0, 2, 77])
            .into_iter()
            .find(|n| n.mac == Some([6, 0, 0, 0, 0, 2]))
            .expect("the newest client joins the line");
        assert_eq!(line.label, "today");
        let scanner = at([192, 0, 2, 78])
            .into_iter()
            .find(|n| n.label == "scanner-1")
            .unwrap();
        assert_eq!(scanner.mac, None, "another name isn't the same device");
    }

    /// "Not linked" goes under the gateway only when it's confirmed and at
    /// the same site; else at the root.
    #[test]
    fn not_linked_hangs_under_a_confirmed_gateway_of_its_site_only() {
        let page = |gw: &str, site: &str| {
            format!(
                r#"
        this.SITES = {{ a:'Site A', b:'Site B' }};
        this.NODES = [
          {{ id:'inet', kind:'WAN', label:'Internet' }},
          {{ id:'gw', site:'a', kind:'{gw}', label:'Edge', sub:'192.0.2.1' }},
          {{ id:'lost', site:'{site}', kind:'SWITCH', label:'Lost', sub:'192.0.2.9' }},
        ];
        this.EDGES = [ {{ from:'inet', to:'gw', cls:'backbone' }} ];"#
            )
        };
        for (gw, site, home) in [
            ("FIREWALL", "a", "gw"),
            ("FIREWALL INFERRED", "a", "inet"),
            ("FIREWALL", "b", "inet"),
        ] {
            let m = all(&page(gw, site));
            let nl = by(&m, "mapnetwork:not-linked");
            assert_eq!(m.nodes[nl.parent().unwrap()].id, home, "{gw} {site}");
        }
    }

    /// A crafted "+N more" card of 30,000 lines, all one address: capped,
    /// counted, and linear (no line compared with every other).
    #[test]
    fn a_huge_listed_card_is_capped_before_it_is_compared() {
        let lines: Vec<String> = (0..30_000)
            .map(|k| format!("'host-{k} · 192.0.2.50'"))
            .chain((0..30_000).map(|k| format!("'other-{k}'")))
            // Each past the cap again: counted once.
            .chain((0..30_000).map(|k| format!("'other-{k}'")))
            .collect();
        let page = format!(
            r#"
        this.NODES = [
          {{ id:'inet', kind:'WAN', label:'Internet' }},
          {{ id:'sw', kind:'SWITCH', label:'Core', sub:'192.0.2.2' }},
          {{ id:'sw_overflow', kind:'ADDITIONAL DEVICES', label:'+lots', details:[{}] }},
        ];
        this.EDGES = [ {{ from:'inet', to:'sw' }}, {{ from:'sw', to:'sw_overflow' }} ];"#,
            lines.join(",")
        );
        let doc = crate::mapnetwork::parse(page.as_bytes()).unwrap();
        let mut b = B::default();
        for v in &doc.nodes {
            b.add(Node {
                id: s(v, "id").into(),
                kind: caps(s(v, "kind")),
                label: s(v, "label").into(),
                details: strings(v, "details"),
                ..Default::default()
            });
        }
        assert_eq!(expand_overflow(&mut b, 2, 3), 0);
        let listed = b
            .nodes
            .iter()
            .filter(|n| n.id.starts_with("sw_overflow"))
            .count();
        assert_eq!(listed, 1_000);
        assert!(
            b.nodes[2]
                .details
                .iter()
                .any(|d| d.starts_with("59000 more listed"))
        );
        let ids: HashSet<&str> = b.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids.len(), b.nodes.len(), "ids are unique");
    }

    /// Switch words win over EDGE and ROUTING. "Access" alone is no
    /// role word: /mapnetwork's `access` zone is "Access points", and
    /// door, badge, guest or remote access aren't switches.
    #[test]
    fn switch_words_win() {
        for (text, zone) in [
            ("EDGE SWITCH", Some(Zone::Switch)),
            ("USW EDGE", Some(Zone::Switch)),
            ("ROUTING SWITCH", Some(Zone::Switch)),
            ("access", None),
            ("Access points", Some(Zone::Wireless)),
            ("Access switches", Some(Zone::Switch)),
            ("DOOR ACCESS", Some(Zone::Device)),
            ("BADGE ACCESS", Some(Zone::Device)),
            ("GUEST ACCESS", Some(Zone::Client)),
            ("REMOTE ACCESS SERVER", Some(Zone::Server)),
            ("ACCESS POINT", Some(Zone::Wireless)),
            ("ACCESS CONTROL", Some(Zone::Device)),
            ("Voice edge", Some(Zone::Gateway)),
            ("EDGE ROUTER", Some(Zone::Gateway)),
        ] {
            assert_eq!(role(text), zone, "{text}");
        }
    }

    /// A MAC named outright in a detail is the card's, in any common
    /// form; a maybe never is.
    #[test]
    fn explicit_macs_in_details() {
        let m = [2, 0, 0, 0, 0, 0x46];
        for line in [
            "MAC address: 02-00-00-00-00-46",
            "MAC: 02:00:00:00:00:46 (from the switch)",
            "mac=02:00:00:00:00:46",
            "0200.0000.0046",
            "02:00:00:00:00:46 · ARP only",
            "MAC-address: 02:00:00:00:00:46",
            "MAC address (LAN): 02:00:00:00:00:46",
            "LAN MAC: 02:00:00:00:00:46",
        ] {
            assert_eq!(own_mac("", &[line]), Some(m), "{line}");
        }
        // Another unit's MAC, named in a sentence or a hedged line.
        for line in [
            "Uplink: Core port 7 (02:00:00:00:00:46)",
            "Standby unit: 02:00:00:00:00:46",
            "MAC of the HA peer: 02:00:00:00:00:46",
            "Old MAC before RMA: 02:00:00:00:00:46",
            "Connected to core-sw 0200.0000.0046",
            "Default gateway MAC 02:00:00:00:00:46",
            "Mgmt interface 0200.0000.0046 on VLAN 99",
            "02:00:00:00:00:46 belongs to the ISP router",
            "MAC 02:00:00:00:00:46 of the ISP router",
        ] {
            assert_eq!(own_mac("", &[line]), None, "{line}");
        }
        assert_eq!(
            own_mac("192.0.2.4 · Uplink: Core port 7 (02:00:00:00:00:46)", &[]),
            None
        );
        assert_eq!(
            own_mac("", &["Candidate identity: 0200.0000.0046 unconfirmed"]),
            None
        );
        assert_eq!(own_mac("192.0.2.4 · 0200.0000.0046", &[]), Some(m));
    }
    /// /mapnetwork's `access` zone is its access points.
    #[test]
    fn the_access_zone_is_access_points() {
        let m = all(r#"
        this.ZONES = { access:{ name:'Access points', vlan:'wireless' } };
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'ap', zone:'access', kind:'U6-LR', label:'Lobby' },
        ];
        this.EDGES = [ { from:'inet', to:'ap' } ];"#);
        assert_eq!(by(&m, "ap").zone, Zone::Wireless);
        // Named otherwise, or not at all: still access points, unless the
        // name says switches.
        for (name, zone) in [
            ("", Zone::Wireless),
            ("Access", Zone::Wireless),
            ("Access layer", Zone::Wireless),
            ("Access switches", Zone::Switch),
        ] {
            let m = all(&format!(
                r#"
        this.ZONES = {{ access:{{ name:'{name}' }} }};
        this.NODES = [
          {{ id:'inet', kind:'WAN', label:'Internet' }},
          {{ id:'d', zone:'access', kind:'U6-LR', label:'Lobby' }},
        ];
        this.EDGES = [ {{ from:'inet', to:'d' }} ];"#
            ));
            assert_eq!(by(&m, "d").zone, zone, "{name}");
        }
    }

    /// A MAC-less third-party firewall: a gear maker's wired client at its
    /// address is its interface (no clash); a maker's name must be a whole
    /// word of the card's ("link" isn't "Starlink"), and a streaming stick
    /// on the Wi-Fi isn't the router's interface.
    #[test]
    fn a_mac_less_gateway_and_its_interfaces() {
        let page = |kind: &str, label: &str| {
            format!(
                r#"
        this.NODES = [
          {{ id:'inet', kind:'WAN', label:'Internet' }},
          {{ id:'fw', kind:'{kind}', label:'{label}', sub:'192.0.2.1' }},
          {{ id:'sw', kind:'SWITCH', label:'Core', sub:'192.0.2.2', mac:'02:00:00:00:00:02' }},
        ];
        this.EDGES = [ {{ from:'inet', to:'fw' }}, {{ from:'fw', to:'sw' }} ];"#
            )
        };
        let clash = |m: &Map| {
            m.findings
                .iter()
                .any(|f| f.text.starts_with("Address clash"))
        };
        let forti = with_unifi(
            &page("FIREWALL", "FortiGate 60F"),
            serde_json::json!([{"mac": "06:00:00:00:00:a1", "last_ip": "192.0.2.1",
                                "oui": "Fortinet, Inc.", "is_wired": true}]),
        );
        assert!(!clash(&forti));
        assert_eq!(by(&forti, "fw").mac, Some([6, 0, 0, 0, 0, 0xa1]));
        let star = with_unifi(
            &page("ROUTER", "Starlink router"),
            serde_json::json!([{"mac": "06:00:00:00:00:a2", "last_ip": "192.0.2.1",
                                "hostname": "tv-stick", "oui": "TP-Link Systems Inc"}]),
        );
        // Its own card (filed as a gear maker's), not merged into the router.
        assert_eq!(
            star.nodes
                .iter()
                .filter(|n| n.ip == Some([192, 0, 2, 1].into()))
                .count(),
            2
        );
        assert_eq!(by(&star, "fw").mac, None);
    }

    /// Within one card: a name given to an unnamed address doesn't let a
    /// second name in; a bare name and the same name with an address are
    /// one device.
    #[test]
    fn listed_lines_merge_only_when_they_agree() {
        let page = r#"
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'sw', kind:'SWITCH', label:'Core', sub:'192.0.2.2' },
          { id:'sw_overflow', kind:'ADDITIONAL DEVICES', label:'+5 more',
            details:['192.0.2.40', 'front-desk · 192.0.2.40', 'front-desk-2 · 192.0.2.40 · Dell',
                     'kiosk-7', 'kiosk-7 · 192.0.2.77'] },
        ];
        this.EDGES = [ { from:'inet', to:'sw' }, { from:'sw', to:'sw_overflow' } ];"#;
        let m = all(page);
        let named = |l: &str| -> Vec<&Node> { m.nodes.iter().filter(|n| n.label == l).collect() };
        assert_eq!(named("front-desk").len(), 1);
        assert_eq!(crate::map::export::vendor(named("front-desk")[0]), None);
        assert_eq!(
            crate::map::export::vendor(named("front-desk-2")[0]).as_deref(),
            Some("Dell")
        );
        let kiosk = named("kiosk-7");
        assert_eq!(kiosk.len(), 1, "one kiosk");
        assert_eq!(kiosk[0].ip, Some([192, 0, 2, 77].into()));
    }

    /// Only a "+N more" line joins a controller client by address, and a
    /// named card never takes a client with no name.
    #[test]
    fn only_listed_lines_take_a_clients_mac() {
        let page = r#"
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'sw', kind:'SWITCH', label:'Core', sub:'192.0.2.2', mac:'02:00:00:00:00:02' },
          { id:'pc', kind:'WORKSTATION', label:'Reception PC', sub:'192.0.2.60' },
          { id:'sw_overflow', kind:'ADDITIONAL DEVICES', label:'+1 more',
            details:['scanner-1 · 192.0.2.61'] },
        ];
        this.EDGES = [ { from:'inet', to:'sw' }, { from:'sw', to:'pc' },
                       { from:'sw', to:'sw_overflow' } ];"#;
        let m = with_unifi(
            page,
            serde_json::json!([
                {"mac": "06:00:00:00:00:b1", "last_ip": "192.0.2.60", "is_guest": true,
                 "oui": "Fictional Phones"},
                {"mac": "06:00:00:00:00:b2", "last_ip": "192.0.2.61"},
            ]),
        );
        let pc = by(&m, "pc");
        assert_eq!(pc.mac, None);
        assert!(!pc.chips.contains(&"guest".to_string()));
        let scanner = m.nodes.iter().find(|n| n.label == "scanner-1").unwrap();
        assert_eq!(scanner.mac, None, "no hostname doesn't agree with a name");
    }
    fn gw_page(kind: &str, label: &str) -> String {
        format!(
            r#"
        this.NODES = [
          {{ id:'inet', kind:'WAN', label:'Internet' }},
          {{ id:'fw', kind:'{kind}', label:'{label}', sub:'192.0.2.1' }},
          {{ id:'sw', kind:'SWITCH', label:'Core', sub:'192.0.2.2', mac:'02:00:00:00:00:02' }},
        ];
        this.EDGES = [ {{ from:'inet', to:'fw' }}, {{ from:'fw', to:'sw' }} ];"#
        )
    }

    fn high_clash(m: &Map) -> bool {
        m.findings
            .iter()
            .any(|f| f.sev == Sev::High && f.text.starts_with("Address clash on 192.0.2.1"))
    }

    /// A box swapped out: the old Netgear's record and the FortiGate's at
    /// one address, the old one first. The card takes the newest client's
    /// MAC, the one its brand names, whatever the file order.
    #[test]
    fn a_mac_less_gateway_takes_its_own_newest_interface() {
        let m = with_unifi(
            &gw_page("FIREWALL", "FortiGate 60F"),
            serde_json::json!([
                {"mac": "06:00:00:00:00:c1", "last_ip": "192.0.2.1", "oui": "Netgear",
                 "is_wired": true, "last_seen": 100},
                {"mac": "06:00:00:00:00:c2", "last_ip": "192.0.2.1", "oui": "Fortinet, Inc.",
                 "is_wired": true, "last_seen": 900},
            ]),
        );
        assert_eq!(by(&m, "fw").mac, Some([6, 0, 0, 0, 0, 0xc2]));
    }

    /// A Cisco desk phone at a SonicWall's address isn't its interface:
    /// the card names its brand, and Cisco isn't it. Named "SEP" and its
    /// MAC, it's a phone, so the clash is a High finding.
    #[test]
    fn a_desk_phone_at_the_firewalls_address_stays_a_clash() {
        let m = with_unifi(
            &gw_page("FIREWALL", "SonicWall TZ370"),
            serde_json::json!([
                {"mac": "06:00:00:00:00:d1", "last_ip": "192.0.2.1", "hostname": "SEP0600000000D1",
                 "oui": "Cisco Systems, Inc", "is_wired": true},
            ]),
        );
        assert_eq!(by(&m, "fw").mac, None);
        assert!(high_clash(&m));
        // A card that names no brand: an endpoint's name keeps it out too.
        let m = with_unifi(
            &gw_page("FIREWALL", "Edge"),
            serde_json::json!([
                {"mac": "06:00:00:00:00:d2", "last_ip": "192.0.2.1", "hostname": "SEP0600000000D2",
                 "oui": "Cisco Systems, Inc", "is_wired": true},
            ]),
        );
        assert_eq!(by(&m, "fw").mac, None);
    }

    /// A device named "Host 5" is named: it doesn't take a stranger's MAC.
    #[test]
    fn a_host_named_host_is_named() {
        let page = r#"
        this.NODES = [
          { id:'inet', kind:'WAN', label:'Internet' },
          { id:'sw', kind:'SWITCH', label:'Core', sub:'192.0.2.2', mac:'02:00:00:00:00:02' },
          { id:'sw_overflow', kind:'ADDITIONAL DEVICES', label:'+2 more',
            details:['Host 5 · 192.0.2.65', '192.0.2.66'] },
        ];
        this.EDGES = [ { from:'inet', to:'sw' }, { from:'sw', to:'sw_overflow' } ];"#;
        let m = with_unifi(
            page,
            serde_json::json!([
                {"mac": "06:00:00:00:00:e1", "last_ip": "192.0.2.65", "hostname": "guest-laptop",
                 "is_guest": true},
                {"mac": "06:00:00:00:00:e2", "last_ip": "192.0.2.66", "hostname": "printer-2"},
            ]),
        );
        let named = m.nodes.iter().find(|n| n.label == "Host 5").unwrap();
        assert_eq!(named.mac, None);
        // A line with no name at all still takes the client there.
        assert!(m.nodes.iter().any(|n| n.label == "printer-2"
            && n.mac == Some([6, 0, 0, 0, 0, 0xe2])
            && n.details.iter().any(|d| d.contains("/mapnetwork map"))));
    }

    /// The vendor on a listed line is a maker as people write one.
    #[test]
    fn listed_vendors_are_makers() {
        let v = |line: &str| listed(line).and_then(|l| l.vendor);
        // The skill's own example line.
        assert_eq!(
            v("client-0a1b2c3d · Hikvision · guest").as_deref(),
            Some("Hikvision")
        );
        for maker in [
            "Epson",
            "Meraki",
            "Aruba",
            "Dahua",
            "Supermicro",
            "Gigabyte",
            "Dell",
        ] {
            assert_eq!(v(&format!("dev-1 · {maker}")).as_deref(), Some(maker));
        }
        for not in [
            "Living room",
            "Main office",
            "Home office",
            "Second floor",
            "Shop floor",
            "Office",
            "The lobby",
            "Test",
            "Galaxy S23",
            "Pixel 8",
            "Echo Dot",
            "5GHz",
            "VLAN-20",
        ] {
            assert_eq!(v(&format!("dev-1 · {not}")), None, "{not}");
        }
    }

    /// "X2 - 24" is the parent's port and the card's own; "port 1 - 24" a
    /// range.
    #[test]
    fn dashes_between_ports() {
        for (label, port, own) in [
            ("X2 - 24", Some("X2"), true),
            ("X2 – 24", Some("X2"), true),
            ("port 1 - 24", None, false),
        ] {
            let r = read_edge(&serde_json::json!({ "label": label }), false);
            assert_eq!(r.port.as_deref(), port, "{label}");
            assert_eq!(
                r.notes.contains(&"Its own port: Port 24".to_string()),
                own,
                "{label}"
            );
        }
    }
}

#[cfg(test)]
mod examples {
    use super::*;
    use crate::map::layout::{Mode, layout_in};
    use crate::map::{Clients, Seen};

    /// The built-in examples redraw in every Clients setting and layout,
    /// with every card's uplink pointing at a card that is there.
    #[test]
    fn examples_build_in_every_client_mode_and_layout() {
        for (name, f) in crate::skills::EXAMPLES {
            let blank = f == crate::skills::BLANK;
            let doc = crate::mapnetwork::parse(crate::skills::example_file(f).unwrap()).unwrap();
            for clients in Clients::ALL {
                let input = Input {
                    clients,
                    ..Default::default()
                };
                let m = build(&doc, &input);
                if blank {
                    // The Internet and the gateway under it, at one site.
                    assert_eq!(m.nodes.len(), 2, "{name}");
                    assert_eq!(m.nodes[0].zone, Zone::Internet);
                    assert_eq!(m.nodes[1].zone, Zone::Gateway);
                    assert_eq!(
                        m.nodes[1].up.as_ref().map(|u| (u.0, u.1)),
                        Some((0, Link::Backbone))
                    );
                    assert_eq!(m.nodes[1].site.as_deref(), Some("Site"));
                    assert!(m.findings.is_empty() && m.hidden == 0);
                } else if f == crate::skills::HOME {
                    // A house: small, every card under the Internet.
                    assert!((8..=15).contains(&m.nodes.len()), "{name}");
                } else {
                    assert!(m.nodes.len() > 30, "{name}");
                }
                for n in &m.nodes {
                    if let Some(up) = &n.up {
                        assert!(up.0 < m.nodes.len(), "{name}: {} hangs off nothing", n.id);
                    }
                }
                // Contoso's only gateway on the internet is inferred, at
                // its own site: the other clinics' strays aren't under it.
                if f.contains("Contoso") {
                    let nl = m
                        .nodes
                        .iter()
                        .find(|n| n.id == "mapnetwork:not-linked")
                        .expect("Contoso has cards nothing links to");
                    let up = nl.parent().unwrap();
                    assert_eq!(m.nodes[up].zone, Zone::Internet, "{name}");
                }
                for mode in Mode::ALL {
                    let l = layout_in(&m, &|_| true, mode);
                    assert!(l.pos.iter().all(|p| p.is_some()), "{name} {mode:?}");
                }
            }
        }
    }

    /// The home example: a router behind the ISP's, a mesh node, a switch
    /// nothing can scan (so its links are assumed), and the issues of a
    /// house, worst first.
    #[test]
    fn the_home_example_has_assumed_links_and_the_usual_issues() {
        let doc =
            crate::mapnetwork::parse(crate::skills::example_file(crate::skills::HOME).unwrap())
                .unwrap();
        let m = build(
            &doc,
            &Input {
                clients: Clients::All,
                ..Default::default()
            },
        );
        let at = |id: &str| m.nodes.iter().find(|n| n.id == id).expect(id);
        let link = |id: &str| at(id).up.as_ref().map(|u| u.1);
        for (id, zone) in [
            ("internet", Zone::Internet),
            ("isp", Zone::Gateway),
            ("router", Zone::Gateway),
            ("mesh", Zone::Wireless),
            ("switch", Zone::Switch),
            ("nas", Zone::Server),
            ("printer", Zone::Device),
            ("doorbell", Zone::Device),
            ("tv", Zone::Client),
            ("laptop", Zone::Client),
            ("unknown", Zone::Unknown),
        ] {
            assert_eq!(at(id).zone, zone, "{id}");
        }
        assert_eq!(link("isp"), Some(Link::Backbone));
        assert_eq!(link("router"), Some(Link::Wired));
        // The unmanaged switch and what's behind it: assumed, not seen.
        for id in ["switch", "tv", "console"] {
            assert_eq!(link(id), Some(Link::Assumed), "{id}");
        }
        for id in ["mesh", "printer", "doorbell", "family"] {
            assert_eq!(link(id), Some(Link::Wireless), "{id}");
        }
        // Eleven findings, worst first, each on the cards it's about.
        assert_eq!(m.findings.len(), 11);
        assert!(m.findings[0].text.contains("port 5000"));
        assert!(!at("router").flags.is_empty() && !at("nas").flags.is_empty());
        assert!(at("thermostat").flags.is_empty());
    }

    /// The blank map with a sweep: it lists no MACs, so no sweep shares
    /// one with it and the sweep is kept apart (not taken for this map's),
    /// with its +N steps open or not, in every setting and layout.
    #[test]
    fn the_blank_map_keeps_a_sweep_apart() {
        let doc =
            crate::mapnetwork::parse(crate::skills::example_file(crate::skills::BLANK).unwrap())
                .unwrap();
        let sweep: Vec<Seen> = (1..40)
            .map(|last| Seen {
                ip: [192, 168, 1, last].into(),
                mac: Some([0, 0, 0, 0, 1, last]),
                name: None,
                rtt: Some(1.0),
                open: vec![],
                checked: true,
                also: vec![],
                web: None,
                multicast: None,
                dnssd: None,
            })
            .collect();
        let open: HashMap<String, usize> = [("gateway".to_string(), usize::MAX)].into();
        for clients in Clients::ALL {
            for reveal in [None, Some(&open)] {
                let m = build(
                    &doc,
                    &Input {
                        sweep: sweep.clone(),
                        gateway: Some([192, 168, 1, 1].into()),
                        clients,
                        reveal,
                        ..Default::default()
                    },
                );
                let ids: Vec<&str> = m.nodes.iter().map(|n| n.id.as_str()).collect();
                assert_eq!(ids, ["internet", "gateway"], "{clients:?}");
                assert!(m.summary.contains("sweep not merged"), "{}", m.summary);
                for mode in Mode::ALL {
                    let l = layout_in(&m, &|_| true, mode);
                    assert!(l.pos.iter().all(|p| p.is_some()), "{clients:?} {mode:?}");
                }
            }
        }
    }
}
