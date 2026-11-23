//! The map as a /mapnetwork page (`.dc.html`): the skill's own template
//! with this map's data in its SITE DATA block. It opens in any browser,
//! Claude's /mapnetwork updates it on the next visit, and is-alive opens it
//! back on the Map. Every card goes in, dragged or not (the page lays the
//! map out itself), with its uplink, findings and the technician's note;
//! clients only when the map draws them, and never past the page's limit.

use super::Drawn;
use crate::map::{Link, Node, Zone};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Most cards a /mapnetwork page holds (its validator's hard cap).
pub const CARDS_MAX: usize = 120;

/// Most characters in a card's top line (the skill's authoring cap).
const KIND_MAX: usize = 18;

/// The zone a role goes in on the page, by key.
fn zone_key(z: Zone) -> &'static str {
    match z {
        Zone::Internet => "internet",
        Zone::Gateway => "gateway",
        Zone::Switch => "switch",
        Zone::Wireless => "wireless",
        Zone::Gear => "gear",
        Zone::Server => "server",
        Zone::Device => "device",
        Zone::Client => "client",
        Zone::Unknown => "unknown",
    }
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// "Contoso Health — HQ" → "contoso-health-hq": a key the page can use.
/// Letters outside ASCII can't go in it, so a name with any ("Москва",
/// "Zürich", a name in kanji) ends in a hash of the whole name: the same
/// name gets the same key every visit, and two such names don't both
/// become "x".
fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut lost = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            lost |= c.is_alphanumeric();
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
        }
    }
    let out = out.trim_end_matches('-');
    match (out.is_empty(), lost) {
        (true, false) => "x".into(),
        (true, true) => format!("u{:08x}", fnv(s)),
        (false, true) => format!("{out}-u{:08x}", fnv(s)),
        (false, false) => out.to_string(),
    }
}

/// FNV-1a: fixed, so a key is the same in every build and on every run.
fn fnv(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// `base`, or `base-2`, `base-3`… the first not yet `taken`.
fn unique(base: String, taken: &mut HashSet<String>) -> String {
    let mut key = base.clone();
    let mut k = 2;
    while !taken.insert(key.clone()) {
        key = format!("{base}-{k}");
        k += 1;
    }
    key
}

/// What the page's limit left out: (infrastructure cards, clients).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Left {
    pub infra: usize,
    pub clients: usize,
}

impl Left {
    pub fn total(self) -> usize {
        self.infra + self.clients
    }

    /// "12 cards left out to stay under 120 (8 infrastructure, 4
    /// clients)"; empty when none were.
    pub fn line(self) -> String {
        let n = self.total();
        if n == 0 {
            return String::new();
        }
        let what = match (self.infra, self.clients) {
            (0, c) => format!("{c} client{}", if c == 1 { "" } else { "s" }),
            (i, 0) => format!("{i} infrastructure card{}", if i == 1 { "" } else { "s" }),
            (i, c) => format!(
                "{n} cards ({i} infrastructure, {c} client{})",
                if c == 1 { "" } else { "s" }
            ),
        };
        format!("{what} left out to stay under {CARDS_MAX} cards")
    }
}

/// The cards that go on the page: every card but the +N more stand-ins,
/// and clients only while the map draws them. Past the page's limit the
/// clients go first, then the deepest infrastructure, so every card kept
/// still has the uplink it hangs from; `Left` says what went.
fn kept(d: &Drawn) -> (Vec<usize>, Left) {
    let shown = |i: usize| d.pos.get(i).copied().flatten().is_some();
    let all: Vec<usize> = (0..d.map.nodes.len())
        .filter(|&i| {
            let n = &d.map.nodes[i];
            !crate::map::is_more(n) && (!n.client || shown(i))
        })
        .collect();
    if all.len() <= CARDS_MAX {
        return (all, Left::default());
    }
    let depth = d.map.depths();
    let mut infra: Vec<usize> = all
        .iter()
        .copied()
        .filter(|&i| !d.map.nodes[i].client)
        .collect();
    // Top down: a card's uplink is always shallower, so it's kept first.
    infra.sort_by_key(|&i| (depth[i], i));
    let room = CARDS_MAX.saturating_sub(infra.len());
    let mut keep: HashSet<usize> = infra.iter().copied().take(CARDS_MAX).collect();
    keep.extend(
        all.iter()
            .copied()
            .filter(|&i| d.map.nodes[i].client)
            .take(room),
    );
    let out: Vec<usize> = all.iter().copied().filter(|i| keep.contains(i)).collect();
    let gone = |client: bool| {
        all.iter()
            .filter(|&&i| d.map.nodes[i].client == client && !keep.contains(&i))
            .count()
    };
    (
        out,
        Left {
            infra: gone(false),
            clients: gone(true),
        },
    )
}

/// What the page shows under a card: its details without the lines
/// is-alive added when it read a page (they'd pile up visit after visit),
/// then its open ports and the technician's note. A port list or note the
/// card has now replaces the line an earlier page wrote for it; with
/// neither, that line stays (a note from another tech's page isn't lost).
fn details(n: &Node) -> Vec<String> {
    use crate::map::{OPEN_PORTS, TECH_NOTE};
    let note = n.note.trim();
    let mut v: Vec<String> = n
        .details
        .iter()
        .filter(|l| !l.starts_with("Site: ") && !l.starts_with("Zone on the map: "))
        .filter(|l| n.open.is_empty() || !l.starts_with(OPEN_PORTS))
        .filter(|l| note.is_empty() || !l.starts_with(TECH_NOTE))
        .cloned()
        .collect();
    if !n.open.is_empty() {
        v.push(format!("{OPEN_PORTS}{}", crate::map::port_names(&n.open)));
    }
    if !note.is_empty() {
        v.push(format!("{TECH_NOTE}{note}"));
    }
    v
}

/// The page's SITE DATA block for the map as it is now.
pub fn data(d: &Drawn) -> String {
    let m = d.map;
    let (keep, left) = kept(d);
    let kept_set: HashSet<usize> = keep.iter().copied().collect();

    // Ids the page can use, unique.
    let mut ids: HashMap<usize, String> = HashMap::new();
    let mut taken: HashSet<String> = HashSet::new();
    for &i in &keep {
        ids.insert(i, unique(slug(&m.nodes[i].id), &mut taken));
    }

    // Sites, by the order first seen; one key each, even for names that
    // slug alike ("Site A", "site-a").
    let mut sites: Vec<(String, String)> = Vec::new();
    let mut site_keys: HashSet<String> = HashSet::new();
    for &i in &keep {
        if let Some(s) = &m.nodes[i].site
            && !sites.iter().any(|(_, n)| n == s)
        {
            sites.push((unique(slug(s), &mut site_keys), s.clone()));
        }
    }
    let site_key: HashMap<&str, &str> = sites
        .iter()
        .map(|(k, n)| (n.as_str(), k.as_str()))
        .collect();
    // On a map with sites every card needs one, or the page puts it in an
    // "Unassigned" box: a card without takes its uplink's, the top the first.
    let site_of = |i: usize| -> Option<&str> {
        let first = sites.first().map(|(_, n)| n.as_str())?;
        let mut at = Some(i);
        while let Some(k) = at {
            if let Some(s) = m.nodes[k].site.as_deref() {
                return Some(s);
            }
            at = m.nodes[k].parent();
        }
        Some(first)
    };

    // Zones: a card read from a /mapnetwork page keeps that page's zone
    // (is-alive noted it as "Zone on the map: Servers (VLAN 10)"); the
    // rest go by role, in the legend's order.
    let page_zone = |n: &Node| -> Option<(String, String)> {
        let line = n
            .details
            .iter()
            .find_map(|l| l.strip_prefix("Zone on the map: "))?;
        Some(match line.rsplit_once(" (") {
            Some((name, vlan)) if vlan.ends_with(')') => {
                (name.to_string(), vlan.trim_end_matches(')').to_string())
            }
            _ => (line.to_string(), String::new()),
        })
    };
    let mut zone_of: HashMap<usize, String> = HashMap::new();
    let mut zones: Vec<(String, Value)> = Vec::new();
    let mut add_zone = |key: String, name: &str, vlan: &str, z: Zone| {
        if !zones.iter().any(|(k, _)| *k == key) {
            zones.push((
                key,
                json!({
                    "name": name,
                    "vlan": vlan,
                    "dark": hex(z.rgb(true)),
                    "light": hex(z.rgb(false)),
                }),
            ));
        }
    };
    // A page's zones by name, each its own key: two names that slug
    // alike don't fold into one box under the first one's name.
    let mut zone_keys: HashSet<String> = HashSet::new();
    let mut page_keys: HashMap<String, String> = HashMap::new();
    for &i in &keep {
        let n = &m.nodes[i];
        if let Some((name, vlan)) = page_zone(n) {
            let key = page_keys
                .entry(name.clone())
                .or_insert_with(|| unique(format!("z-{}", slug(&name)), &mut zone_keys))
                .clone();
            add_zone(key.clone(), &name, &vlan, n.zone);
            zone_of.insert(i, key);
        }
    }
    for z in Zone::ALL {
        for &i in &keep {
            if !zone_of.contains_key(&i) && m.nodes[i].zone == z {
                add_zone(zone_key(z).to_string(), z.name(), "", z);
                zone_of.insert(i, zone_key(z).to_string());
            }
        }
    }

    let nodes: Vec<Value> = keep
        .iter()
        .map(|&i| {
            let n = &m.nodes[i];
            let mut o = serde_json::Map::new();
            o.insert("id".into(), json!(ids[&i]));
            if let Some(k) = site_of(i).and_then(|s| site_key.get(s)) {
                o.insert("site".into(), json!(k));
            }
            o.insert("zone".into(), json!(zone_of[&i]));
            let kind: String = n.kind.chars().take(KIND_MAX).collect();
            o.insert("kind".into(), json!(kind));
            o.insert("label".into(), json!(n.label));
            o.insert("sub".into(), json!(n.sub));
            if let Some(ip) = n.ip {
                o.insert("ip".into(), json!(ip.to_string()));
            }
            if let Some(mac) = &n.mac {
                o.insert("mac".into(), json!(crate::net::mac::fmt(mac)));
            }
            o.insert("details".into(), json!(details(n)));
            o.insert("flags".into(), json!(n.flags));
            // A site's name is the page's own box: not a chip as well. A
            // card a person drew on a plan says so, and opens back so.
            let mut chips: Vec<&str> = n
                .chips
                .iter()
                .map(String::as_str)
                .filter(|c| n.site.as_deref() != Some(*c) && *c != DRAWN)
                .collect();
            if n.drawn {
                chips.push(DRAWN);
            }
            o.insert("chips".into(), json!(chips));
            Value::Object(o)
        })
        .collect();

    let mut edges: Vec<Value> = Vec::new();
    for &i in &keep {
        let Some((p, link, port)) = &m.nodes[i].up else {
            continue;
        };
        if !kept_set.contains(p) {
            continue;
        }
        let mut o = serde_json::Map::new();
        o.insert("id".into(), json!(format!("e{}", edges.len() + 1)));
        o.insert("from".into(), json!(ids[p]));
        o.insert("to".into(), json!(ids[&i]));
        let (cls, dashed) = match link {
            Link::Backbone => ("backbone", false),
            Link::Wired => ("wired", false),
            Link::Wireless => ("wifi", true),
            // Not in the data: dashed, as /mapnetwork marks a link it inferred.
            Link::Assumed => ("wired", true),
        };
        o.insert("cls".into(), json!(cls));
        // A link a person drew isn't one anything showed: dashed, like
        // one is-alive inferred.
        if dashed || m.nodes[i].drawn_up {
            o.insert("dashed".into(), json!(true));
        }
        if let Some(port) = port.as_deref().filter(|p| !p.is_empty()) {
            o.insert("label".into(), json!(port));
        }
        edges.push(Value::Object(o));
    }

    let actions: Vec<Value> = m
        .findings
        .iter()
        .enumerate()
        .map(|(k, f)| {
            let on: Vec<&String> = f.nodes.iter().filter_map(|i| ids.get(i)).collect();
            json!({
                "id": format!("f{}", k + 1),
                "sev": f.sev.label(),
                "text": f.text,
                "nodes": on,
            })
        })
        .collect();

    // The summary starts with the title the page already shows.
    let summary = m
        .summary
        .strip_prefix(d.title())
        .map_or(m.summary.as_str(), |s| s.trim_start_matches(" · "));
    let mut subtitle = format!("{summary} · from is-alive {}", crate::VERSION);
    // A plan or a training map says so first, as in the other exports.
    if let Some(c) = d.caption {
        subtitle = format!("{c} · {subtitle}");
    }
    if left.total() > 0 {
        subtitle += &format!(" · {}", left.line());
    }
    let meta = json!({ "title": d.title(), "subtitle": subtitle });

    let mut s = String::new();
    s += "    // Exported from is-alive's Map. /mapnetwork can update this map in\n";
    s += "    // place; is-alive opens it back on the Map.\n";
    s += &format!("    this.META = {};\n\n", lit(&meta));
    s += "    this.SITES = {\n";
    for (k, (key, name)) in sites.iter().enumerate() {
        let comma = if k + 1 < sites.len() { "," } else { "" };
        s += &format!(
            "      {}: {}{comma}\n",
            lit(&json!(key)),
            lit(&json!({ "name": name }))
        );
    }
    s += "    };\n\n";
    s += "    this.ZONES = {\n";
    for (k, (key, z)) in zones.iter().enumerate() {
        let comma = if k + 1 < zones.len() { "," } else { "" };
        s += &format!("      {}: {}{comma}\n", lit(&json!(key)), lit(z));
    }
    s += "    };\n\n";
    s += &list("NODES", &nodes);
    s += &list("EDGES", &edges);
    s += &list("ACTIONS", &actions);
    s += "    this.BANDS = [];\n";
    s
}

/// `this.NAME = [ … ];`, one item to a line.
fn list(name: &str, items: &[Value]) -> String {
    let mut s = format!("    this.{name} = [\n");
    for (k, v) in items.iter().enumerate() {
        let comma = if k + 1 < items.len() { "," } else { "" };
        s += &format!("      {}{comma}\n", lit(v));
    }
    s += "    ];\n\n";
    s
}

/// A value as a JavaScript literal: JSON, with `<` escaped so no text can
/// close the page's script, and `this.` and `SITE DATA` so no name can
/// pass for the data's own shape when it's read back (a device calling
/// itself `this.ACTIONS=[]`). Each is a whole JSON escape inside a string,
/// read back as the same text.
fn lit(v: &Value) -> String {
    v.to_string()
        .replace('<', "\\u003c")
        .replace("this.", "this\\u002e")
        .replace("SITE DATA", "SITE\\u0020DATA")
}

/// The chip on a card a person drew (a plan's), read back as such.
pub const DRAWN: &str = "drawn";

/// The whole page: the /mapnetwork template with this map's data.
pub fn write(d: &Drawn) -> String {
    String::from_utf8_lossy(&crate::skills::in_template(&data(d))).into_owned()
}

/// What the page would leave out, for a note as it's saved.
pub fn left_out(d: &Drawn) -> Left {
    kept(d).1
}

#[cfg(test)]
mod tests {
    use super::super::tests::{HOSTILE, with_acme, with_hostile};
    use super::*;
    use crate::map::layout::Mode;

    /// Acme saved as a page and opened again: the same cards, uplinks,
    /// sites and findings.
    #[test]
    fn a_map_saved_as_dc_html_opens_back_the_same() {
        with_acme(Mode::Sites, |d| {
            let page = write(d);
            assert!(
                page.contains("<html") || page.contains("<!DOCTYPE"),
                "the template's page"
            );
            let doc = crate::mapnetwork::parse(page.as_bytes()).expect("reads back");
            let back = crate::map::build(&crate::map::Input {
                mapnetwork: Some(&doc),
                ..Default::default()
            });
            let names = |m: &crate::map::Map| {
                let mut v: Vec<(String, Option<String>)> = m
                    .nodes
                    .iter()
                    .filter(|n| !crate::map::is_more(n) && !n.client)
                    .map(|n| {
                        (
                            n.label.clone(),
                            n.parent().map(|p| m.nodes[p].label.clone()),
                        )
                    })
                    .collect();
                v.sort();
                v
            };
            assert_eq!(names(&back), names(d.map));
            // Every card keeps its site (one without takes its uplink's).
            for n in d.map.nodes.iter().filter(|n| n.site.is_some()) {
                let b = back.nodes.iter().find(|b| b.label == n.label).unwrap();
                assert_eq!(b.site, n.site, "{}", n.label);
            }
            assert_eq!(back.findings.len(), d.map.findings.len());
            for (a, b) in back.findings.iter().zip(&d.map.findings) {
                assert_eq!(
                    (a.sev, &a.text, a.nodes.len()),
                    (b.sev, &b.text, b.nodes.len())
                );
            }
            // The page's own zones (segments, VLANs) come back out.
            let servers = doc.zones.values().find(|z| z["name"] == "Servers").unwrap();
            assert_eq!(servers["vlan"], "VLAN 10 · 192.168.1.x");
            // Its lines don't pile up visit after visit.
            let again = crate::mapnetwork::parse(page.as_bytes()).unwrap();
            assert!(
                !again
                    .nodes
                    .iter()
                    .flat_map(|n| n["details"].as_array().into_iter().flatten())
                    .any(|l| l.as_str().is_some_and(|l| l.starts_with("Site: ")))
            );
        });
    }

    /// A card read from an earlier page: its current ports and note
    /// replace the lines that page wrote, once each; with neither, the old
    /// lines stay.
    #[test]
    fn a_cards_ports_and_note_are_written_once() {
        let mut n = Node {
            details: vec![
                "Open ports: 22 SSH".into(),
                "Tech note: old".into(),
                "Also at 192.0.2.9: Open ports: 80 HTTP".into(),
            ],
            open: vec![22, 443],
            note: "new".into(),
            ..Default::default()
        };
        let d = details(&n);
        let count = |p: &str| d.iter().filter(|l| l.starts_with(p)).count();
        assert_eq!(count("Open ports: "), 1, "{d:?}");
        assert_eq!(count("Tech note: "), 1, "{d:?}");
        assert!(d.contains(&"Tech note: new".to_string()));
        assert!(
            d.iter()
                .any(|l| l.starts_with("Open ports: 22") && l.contains("443"))
        );
        assert!(d.contains(&"Also at 192.0.2.9: Open ports: 80 HTTP".to_string()));
        n.open.clear();
        n.note.clear();
        assert_eq!(details(&n), n.details);
    }

    /// A hostile note can't close the page's script, and comes back whole.
    #[test]
    fn a_hostile_note_stays_text_in_dc_html() {
        with_hostile(|d| {
            let block = data(d);
            assert!(!block.contains("</script"));
            let page = write(d);
            let doc = crate::mapnetwork::parse(page.as_bytes()).unwrap();
            let b = doc.nodes.iter().find(|n| n["label"] == "B").unwrap();
            let lines: Vec<&str> = b["details"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|l| l.as_str())
                .collect();
            assert!(
                lines
                    .iter()
                    .any(|l| *l == format!("Tech note: {}", HOSTILE.trim())),
                "{lines:?}"
            );
        });
    }

    fn drawn_of<R>(m: &crate::map::Map, caption: Option<&str>, f: impl FnOnce(&Drawn) -> R) -> R {
        let lay = crate::map::layout::layout_in(m, &|_| true, Mode::Cluster);
        let moved = vec![false; m.nodes.len()];
        f(&Drawn {
            map: m,
            lay: &lay,
            pos: &lay.pos,
            moved: &moved,
            legend: false,
            caption,
            exploded: None,
        })
    }

    fn card(id: &str, label: &str, up: Option<usize>, zone: Zone) -> Node {
        Node {
            id: id.into(),
            label: label.into(),
            zone,
            up: up.map(|p| (p, Link::Wired, None)),
            ..Default::default()
        }
    }

    /// A device's name, a site's or the title can't pass for the data's
    /// own shape when the page is read back: every card, link and finding
    /// comes back, and each name as it was.
    #[test]
    fn names_cant_pass_for_the_data() {
        let names = [
            "this.ACTIONS=[]",
            "this.EDGES = [] // x",
            "x */ this.NODES = [{id:'z'}] /*",
            "END SITE DATA",
            "=== SITE DATA ====",
            "it's \"quoted\" `tick`",
        ];
        let mut nodes = vec![card("internet", "Internet", None, Zone::Internet)];
        for (k, n) in names.iter().enumerate() {
            nodes.push(card(&format!("n{k}"), n, Some(0), Zone::Switch));
        }
        let m = crate::map::Map {
            title: "this.META = {title:'Owned'}".into(),
            findings: vec![crate::map::Finding {
                sev: crate::map::Sev::High,
                text: "Real finding".into(),
                nodes: vec![1],
            }],
            nodes,
            ..Default::default()
        };
        let page = drawn_of(&m, None, write);
        let doc = crate::mapnetwork::parse(page.as_bytes()).expect("reads back");
        assert_eq!(doc.title, "this.META = {title:'Owned'}");
        assert_eq!(doc.nodes.len(), names.len() + 1);
        assert_eq!(doc.edges.len(), names.len());
        assert_eq!(doc.actions.len(), 1);
        assert_eq!(doc.actions[0]["text"], "Real finding");
        for n in names {
            assert!(doc.nodes.iter().any(|d| d["label"] == n), "{n}");
        }
    }

    /// Past the page's limit, infrastructure goes deepest first, so every
    /// card kept keeps the uplink it hangs from, and the note says what
    /// went (not that clients did).
    #[test]
    fn past_the_limit_every_card_kept_keeps_its_uplink() {
        let mut nodes = vec![
            card("internet", "Internet", None, Zone::Internet),
            card("gw", "Gateway", Some(0), Zone::Gateway),
        ];
        for s in 0..10 {
            nodes.push(card(
                &format!("sw{s}"),
                &format!("Switch {s}"),
                Some(1),
                Zone::Switch,
            ));
            let sw = nodes.len() - 1;
            for a in 0..14 {
                nodes.push(card(
                    &format!("ap{s}-{a}"),
                    &format!("AP {s}-{a}"),
                    Some(sw),
                    Zone::Wireless,
                ));
            }
        }
        assert_eq!(nodes.len(), 152);
        let m = crate::map::Map {
            nodes,
            ..Default::default()
        };
        let (keep, left) = drawn_of(&m, None, kept);
        assert_eq!(keep.len(), CARDS_MAX);
        assert_eq!(
            left,
            Left {
                infra: 32,
                clients: 0
            }
        );
        let kept: HashSet<usize> = keep.iter().copied().collect();
        for &i in &keep {
            if let Some(p) = m.nodes[i].parent() {
                assert!(kept.contains(&p), "{} lost its uplink", m.nodes[i].label);
            }
        }
        let page = drawn_of(&m, None, write);
        let doc = crate::mapnetwork::parse(page.as_bytes()).unwrap();
        assert_eq!(
            doc.edges.len(),
            CARDS_MAX - 1,
            "a link for every card but the top"
        );
        assert!(
            doc.subtitle
                .contains("32 infrastructure cards left out to stay under 120 cards")
        );
        assert!(!doc.subtitle.contains("client"));
    }

    /// A plan says so on the page, as in the other exports, and a card a
    /// person drew opens back as drawn.
    #[test]
    fn a_plan_says_so_and_its_drawn_cards_come_back_drawn() {
        let mut gw = card("gw", "Gateway", Some(0), Zone::Gateway);
        gw.drawn = true;
        gw.drawn_up = true;
        let m = crate::map::Map {
            nodes: vec![card("internet", "Internet", None, Zone::Internet), gw],
            ..Default::default()
        };
        let plan = "Plan — drawn by hand, not discovered";
        let page = drawn_of(&m, Some(plan), write);
        let doc = crate::mapnetwork::parse(page.as_bytes()).unwrap();
        assert!(doc.subtitle.starts_with(plan), "{}", doc.subtitle);
        assert_eq!(
            doc.edges[0]["dashed"], true,
            "a drawn link isn't one anything showed"
        );
        let back = crate::map::build(&crate::map::Input {
            mapnetwork: Some(&doc),
            ..Default::default()
        });
        let g = back.nodes.iter().find(|n| n.label == "Gateway").unwrap();
        assert!(g.drawn && !g.chips.iter().any(|c| c == DRAWN));
        // As found: no caption, no mark.
        let found = drawn_of(&m, None, write);
        assert!(
            !crate::mapnetwork::parse(found.as_bytes())
                .unwrap()
                .subtitle
                .contains("Plan")
        );
    }

    /// Sites and zones whose names slug alike ("Москва" and "Казань" both
    /// lose every letter) stay apart on the page and when it's read back.
    #[test]
    fn sites_and_zones_named_alike_stay_apart() {
        let mut m = super::super::tests::acme();
        // Every other card in each site and each zone.
        for (k, n) in m.nodes.iter_mut().enumerate() {
            if n.zone == Zone::Internet {
                continue;
            }
            let (site, zone) = [("Москва", "Site A"), ("Казань", "site-a")][k % 2];
            n.site = Some(site.to_string());
            n.details.retain(|l| !l.starts_with("Zone on the map: "));
            n.details.push(format!("Zone on the map: {zone}"));
        }
        let lay = crate::map::layout::layout_in(&m, &|_| true, Mode::Sites);
        let moved = vec![false; m.nodes.len()];
        let d = Drawn {
            map: &m,
            lay: &lay,
            pos: &lay.pos,
            moved: &moved,
            legend: false,
            caption: None,
            exploded: None,
        };
        let doc = crate::mapnetwork::parse(write(&d).as_bytes()).unwrap();
        let mut site_names: Vec<&str> = doc.sites.iter().map(|(_, n)| n.as_str()).collect();
        site_names.sort();
        assert_eq!(site_names, ["Казань", "Москва"]);
        assert_ne!(doc.sites[0].0, doc.sites[1].0);
        let zone_names: Vec<&str> = doc
            .zones
            .values()
            .filter_map(|v| v["name"].as_str())
            .collect();
        assert!(
            zone_names.contains(&"Site A") && zone_names.contains(&"site-a"),
            "{zone_names:?}"
        );
        let back = crate::map::build(&crate::map::Input {
            mapnetwork: Some(&doc),
            ..Default::default()
        });
        for n in m.nodes.iter().filter(|n| n.site.is_some() && !n.client) {
            if let Some(b) = back.nodes.iter().find(|b| b.label == n.label) {
                assert_eq!(b.site, n.site, "{}", n.label);
            }
        }
    }

    /// Ids the page can use, each once.
    #[test]
    fn ids_are_plain_and_unique() {
        assert_eq!(slug("mapnetwork:internet"), "mapnetwork-internet");
        assert_eq!(slug("ip:10.0.0.1"), "ip-10-0-0-1");
        assert_eq!(slug("::"), "x");
        assert_eq!(slug("Contoso Health — HQ"), "contoso-health-hq");
        // Names in other scripts: a key of their own, the same each time.
        let (msk, kzn) = (slug("Москва"), slug("Казань"));
        assert_ne!(msk, kzn);
        assert_eq!(msk, slug("Москва"));
        // Tokyo and Osaka, in kanji.
        let (tokyo, osaka) = ("\u{6771}\u{4eac}", "\u{5927}\u{962a}");
        assert_ne!(slug(tokyo), slug(osaka));
        assert!(slug("Zürich HQ").starts_with("z-rich-hq-u"));
        assert!(
            [msk, kzn, slug(tokyo)]
                .iter()
                .all(|k| k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        );
    }
}
