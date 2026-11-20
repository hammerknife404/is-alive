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
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "x".into() } else { out }
}

/// The cards that go on the page: every card but the +N more stand-ins,
/// and clients only while the map draws them. Past the page's limit the
/// clients go first; the second value says how many were left out.
fn kept(d: &Drawn) -> (Vec<usize>, usize) {
    let shown = |i: usize| d.pos.get(i).copied().flatten().is_some();
    let all: Vec<usize> = (0..d.map.nodes.len())
        .filter(|&i| {
            let n = &d.map.nodes[i];
            !crate::map::is_more(n) && (!n.client || shown(i))
        })
        .collect();
    if all.len() <= CARDS_MAX {
        return (all, 0);
    }
    let infra: Vec<usize> = all
        .iter()
        .copied()
        .filter(|&i| !d.map.nodes[i].client)
        .collect();
    let room = CARDS_MAX.saturating_sub(infra.len());
    let mut keep: HashSet<usize> = infra.iter().copied().take(CARDS_MAX).collect();
    keep.extend(
        all.iter()
            .copied()
            .filter(|&i| d.map.nodes[i].client)
            .take(room),
    );
    let out: Vec<usize> = all.iter().copied().filter(|i| keep.contains(i)).collect();
    let left = all.len() - out.len();
    (out, left)
}

/// What the page shows under a card: its details without the lines
/// is-alive added when it read a page (they'd pile up visit after visit),
/// then its open ports and the technician's note.
fn details(n: &Node) -> Vec<String> {
    let mut v: Vec<String> = n
        .details
        .iter()
        .filter(|l| !l.starts_with("Site: ") && !l.starts_with("Zone on the map: "))
        .cloned()
        .collect();
    if !n.open.is_empty() {
        let ports: Vec<String> = n.open.iter().map(u16::to_string).collect();
        v.push(format!("Open ports: {}", ports.join(", ")));
    }
    if !n.note.trim().is_empty() {
        v.push(format!("Tech note: {}", n.note.trim()));
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
        let base = slug(&m.nodes[i].id);
        let mut id = base.clone();
        let mut k = 2;
        while !taken.insert(id.clone()) {
            id = format!("{base}-{k}");
            k += 1;
        }
        ids.insert(i, id);
    }

    // Sites, by the order first seen.
    let mut sites: Vec<(String, String)> = Vec::new();
    for &i in &keep {
        if let Some(s) = &m.nodes[i].site
            && !sites.iter().any(|(_, n)| n == s)
        {
            sites.push((slug(s), s.clone()));
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
    for &i in &keep {
        let n = &m.nodes[i];
        if let Some((name, vlan)) = page_zone(n) {
            let key = format!("z-{}", slug(&name));
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
            // A site's name is the page's own box: not a chip as well.
            let chips: Vec<&String> = n
                .chips
                .iter()
                .filter(|c| n.site.as_deref() != Some(c.as_str()))
                .collect();
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
        if dashed {
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
    if left > 0 {
        subtitle += &format!(" · {left} client(s) left out to stay under {CARDS_MAX} cards");
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
/// close the page's script.
fn lit(v: &Value) -> String {
    v.to_string().replace('<', "\\u003c")
}

/// The whole page: the /mapnetwork template with this map's data.
pub fn write(d: &Drawn) -> String {
    String::from_utf8_lossy(&crate::skills::in_template(&data(d))).into_owned()
}

/// How many cards the page would leave out, for a note as it's saved.
pub fn left_out(d: &Drawn) -> usize {
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

    /// Ids the page can use, each once.
    #[test]
    fn ids_are_plain_and_unique() {
        assert_eq!(slug("mapnetwork:internet"), "mapnetwork-internet");
        assert_eq!(slug("ip:10.0.0.1"), "ip-10-0-0-1");
        assert_eq!(slug("::"), "x");
    }
}
