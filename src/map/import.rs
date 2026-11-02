//! A /mapnetwork map redrawn as this app's map: its devices become cards
//! with roles read from their kind, its links become the tree (each card
//! hangs off the first link that reaches it from the internet; any other
//! link is listed in its details), and its findings become the band. A
//! sweep of the same network marks which of its devices answer, and what
//! the map left out (/mapnetwork maps clients only on request) joins it as
//! clients: the sweep's other hosts, and a loaded UniFi backup's client
//! table, each under the card it most likely plugs into.

use super::{
    B, Input, Link, Map, Node, Sev, Unifi, Zone, caps, limit, parse_unifi, sweep_card,
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
        } else if s(e, "cls") == "wifi" {
            Link::Wireless
        } else if e.get("dashed").and_then(Value::as_bool) == Some(true) {
            // /mapnetwork dashes a link it inferred (a sweep can't see
            // uplinks): assumed, as this app's own sweep draws it.
            Link::Assumed
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

    // The map's own "+7 more" cards, one card for each device they list.
    for &c in &card {
        expand_overflow(&mut b, c, src.len() as u64);
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
    // the switch or AP each was last seen on.
    let mut unifi_at = Vec::new();
    let unifi = input.unifi.map(parse_unifi).filter(|u| {
        unifi_at = join_unifi(&mut b, u, cards);
        !unifi_at.is_empty()
    });
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
    if let Some(u) = &unifi {
        for c in &u.clients {
            if let Some(i) = b.find(Some(c.mac), None) {
                b.nodes[i].seen("UniFi");
                continue;
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
    if unifi.is_some() {
        summary += &format!(" · UniFi backup merged, {unifi_added} client(s) added");
    } else if input.unifi.is_some() {
        summary += " · UniFi backup not merged (none of its devices is on the map)";
    }
    if unplaced > 0 {
        summary += &format!(" · {unplaced} not placed");
    }
    let mut map = limit(b, input);
    summary += &map.hidden_note();
    map.summary = summary;
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

/// Whether a /mapnetwork card is the map's own "+7 more": the clients
/// past its 10 per uplink, one line of details each.
pub(super) fn is_overflow(n: &Node) -> bool {
    n.kind.contains("ADDITIONAL DEVICES")
}

/// Turn the overflow card `c` into a client card for each device it lists
/// by address or MAC ("laptop-jsmith · 192.168.1.42"), under the same
/// uplink, so the reveal steps through them like any other clients; its
/// other lines (notes) go on each. The first device takes the card's
/// place. Without such lines it stays one card, revealed last. `after`:
/// past the rank of every card from the map.
fn expand_overflow(b: &mut B, c: usize, after: u64) {
    let n = &b.nodes[c];
    if !is_overflow(n) {
        return;
    }
    let (hosts, notes): (Vec<&String>, Vec<&String>) = n
        .details
        .iter()
        .partition(|d| first_ip(d).is_some() || first_mac([d.as_str()]).is_some());
    let label = n.label.clone();
    let chips: Vec<String> = n
        .chips
        .iter()
        .filter(|c| *c != "collapsed" && !c.ends_with(" devices"))
        .cloned()
        .collect();
    let devices: Vec<Node> = hosts
        .iter()
        .enumerate()
        .map(|(k, line)| {
            let ip = first_ip(line);
            let mac = first_mac([line.as_str()]);
            let head = line.split('·').next().unwrap_or("").trim();
            let name = Some(head)
                .filter(|h| !h.is_empty() && first_ip(h).is_none() && first_mac([*h]).is_none());
            let mut d = Node {
                id: format!(
                    "{}:{}",
                    n.id,
                    match (ip, mac) {
                        (Some(ip), _) => ip.to_string(),
                        (None, Some(m)) => mac::fmt(&m),
                        _ => k.to_string(),
                    }
                ),
                zone: Zone::Client,
                kind: "CLIENT".into(),
                label: match (name, ip, mac) {
                    (Some(h), ..) => h.to_string(),
                    (None, Some(ip), _) => format!("Host {ip}"),
                    (None, None, Some(m)) => mac::fmt(&m),
                    _ => line.to_string(),
                },
                sub: ip.map(|i| i.to_string()).unwrap_or_default(),
                ip,
                mac,
                chips: chips.clone(),
                client: true,
                up: n.up.clone(),
                site: n.site.clone(),
                seen_by: n.seen_by.clone(),
                // After the map's own clients, in the map's order.
                rank: (2, after + k as u64),
                ..Default::default()
            };
            d.details.push(line.to_string());
            d.details.extend(notes.iter().map(|s| s.to_string()));
            d.details
                .push(format!("In \"{label}\" on the /mapnetwork map"));
            d
        })
        .collect();
    let mut devices = devices.into_iter();
    let Some(first) = devices.next() else {
        return;
    };
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
    votes
        .into_iter()
        .filter_map(|(net, v)| {
            let best = v
                .into_iter()
                .max_by_key(|&(i, n)| (n, std::cmp::Reverse(i)))?;
            Some((net, best.0))
        })
        .collect()
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
                    n.details.push(
                        "Uplink guessed from its subnet: other cards there hang off it".into(),
                    );
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
            unifi: Some(&unifi),
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
                unifi: Some(&unifi),
                clients: Clients::All,
                ..Default::default()
            },
        );
        assert!(m.nodes.iter().all(|n| n.label != "02:00:00:00:00:42"));
        assert!(m.summary.contains("UniFi backup not merged"));
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
        assert_eq!(pc.up.as_ref().unwrap().2.as_deref(), Some("inferred"));
        assert_eq!(by(&m, "ap").link(), Some(Link::Wired));
        assert_eq!(by(&m, "ph").link(), Some(Link::Wireless));
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
                } else {
                    assert!(m.nodes.len() > 30, "{name}");
                }
                for n in &m.nodes {
                    if let Some(up) = &n.up {
                        assert!(up.0 < m.nodes.len(), "{name}: {} hangs off nothing", n.id);
                    }
                }
                for mode in Mode::ALL {
                    let l = layout_in(&m, &|_| true, mode);
                    assert!(l.pos.iter().all(|p| p.is_some()), "{name} {mode:?}");
                }
            }
        }
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
