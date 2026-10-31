//! A /mapnetwork map redrawn as this app's map: its devices become cards
//! with roles read from their kind, its links become the tree (each card
//! hangs off the first link that reaches it from the internet; any other
//! link is listed in its details), and its findings become the band. A
//! sweep of the same network marks which of its devices answer.

use super::{B, Input, Link, Map, Node, Sev, Zone, caps, limit};
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

/// Role from a card's words: its kind first, then its label, then the
/// zone it was filed under in the map.
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
    } else if any(&["UNKNOWN", "UNIDENTIFIED", "EXTERNAL", "UNRESOLVED"]) {
        Some(Zone::Unknown)
    } else if any(&[
        "FIREWALL",
        "FIREWALLS",
        "GATEWAY",
        "ROUTER",
        "SONICWALL",
        "FORTIGATE",
        "UDM",
        "UCG",
        "USG",
        "UXG",
    ]) {
        Some(Zone::Gateway)
    } else if any(&["SWITCH", "SWITCHES", "USW", "IDF", "MDF", "DISTRIBUTION"]) {
        Some(Zone::Switch)
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
        "HOST",
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
        "CAMERA",
        "CAMERAS",
        "NVR",
        "DVR",
        "UPS",
        "PDU",
        "IOT",
        "SENSOR",
        "DOOR",
        "BADGE",
        "HVAC",
        "THERMOSTAT",
    ]) {
        Some(Zone::Device)
    } else if any(&[
        "WORKSTATION",
        "WORKSTATIONS",
        "LAPTOP",
        "DESKTOP",
        "PC",
        "PHONE",
        "PHONES",
        "TABLET",
        "CLIENT",
        "CLIENTS",
        "GUEST",
        "VOIP",
        "VOICE",
        "TRUSTED",
    ]) || phrase(&["ADDITIONAL DEVICES"])
    {
        Some(Zone::Client)
    } else if any(&["EDGE", "ROUTING"]) {
        Some(Zone::Gateway)
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
        role(s(v, "kind"))
            .or_else(|| role(s(v, "label")))
            .or_else(|| role(z))
            .or_else(|| zinfo.and_then(|i| role(s(i, "name"))))
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
        let mut n = Node {
            id: s(v, "id").to_string(),
            zone,
            kind: caps(s(v, "kind")),
            label: s(v, "label").to_string(),
            ip: first_ip(&sub),
            mac: first_mac(
                std::iter::once(sub.as_str())
                    .chain(chips.iter().map(String::as_str))
                    .chain(details.iter().map(String::as_str)),
            ),
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
        for c in chips {
            n.chip(c);
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
    let link_of = |e: &Value| {
        if s(e, "cls") == "backbone" {
            Link::Backbone
        } else if s(e, "cls") == "wifi" || e.get("dashed").and_then(Value::as_bool) == Some(true) {
            Link::Wireless
        } else {
            Link::Wired
        }
    };
    let label_of = |e: &Value| Some(s(e, "label").to_string()).filter(|l| !l.is_empty());
    for i in 0..src.len() {
        if Some(i) == root_src {
            continue;
        }
        let up = match via[i] {
            Some(k) => {
                let (f, _, e) = edges[k];
                (card[f], link_of(e), label_of(e))
            }
            // Only a firewall can sit straight on the internet.
            None if b.nodes[card[i]].zone == Zone::Gateway && root_src.is_none() => {
                (0, Link::Backbone, None)
            }
            None => (0, Link::Assumed, None),
        };
        b.nodes[card[i]].up = Some(up);
    }
    let used: HashSet<usize> = via.iter().flatten().copied().collect();
    for (k, &(f, t, e)) in edges.iter().enumerate() {
        if used.contains(&k) {
            continue;
        }
        let tail = label_of(e).map(|l| format!(" ({l})")).unwrap_or_default();
        let (fl, tl) = (
            b.nodes[card[f]].label.clone(),
            b.nodes[card[t]].label.clone(),
        );
        b.nodes[card[t]]
            .details
            .push(format!("Also linked from {fl}{tail}"));
        b.nodes[card[f]]
            .details
            .push(format!("Also linked to {tl}{tail}"));
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

    // A sweep joins only when it found hardware the map knows by MAC: the
    // same network, not a look-alike 192.168.1.x.
    let local = input
        .sweep
        .iter()
        .any(|h| h.mac.is_some_and(|m| b.by_mac.contains_key(&m)));
    let mut off_map = 0;
    if local {
        for h in &input.sweep {
            match b.find(h.mac, Some(h.ip)) {
                Some(i) => b.merge_seen(i, h),
                None => off_map += 1,
            }
        }
    }

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
        summary += &format!(" · sweep merged, {off_map} host(s) not on the map");
    } else if !input.sweep.is_empty() {
        summary += " · sweep not merged (no device on the map shares a MAC with it)";
    }
    let mut map = limit(b, input);
    if map.hidden > 0 && !input.clients && !summary.contains("clients not mapped") {
        summary += " · clients not mapped";
    }
    map.summary = summary;
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Seen;

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
                clients: true,
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
        assert!(off.summary.ends_with("clients not mapped"));
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
        };
        let look_alike = Input {
            sweep: vec![host([10, 0, 0, 2], Some([2, 0, 0, 0, 0, 9]))],
            ..Default::default()
        };
        let m = build(&doc, &look_alike);
        assert_eq!(by(&m, "sw").seen_by, ["mapnetwork"]);
        assert!(m.summary.contains("sweep not merged"));

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
        assert!(m.summary.contains("1 host(s) not on the map"));
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
}
