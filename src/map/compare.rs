//! Compare the map now with one saved before, of the same network: which
//! devices are new since, which are gone, and what changed on the ones in
//! both (address, open ports, name). For a technician back at a site:
//! a device nobody knows about, a printer missing, Remote Desktop newly
//! open.
//!
//! A device is the same device when its MAC is: addresses move with DHCP,
//! the hardware address doesn't. One with no MAC on either side (known
//! only by IP, past a router) is matched by IP, and failing that by its
//! card's id (a /mapnetwork card keeps its id when its address changes).
//! Two cards at one IP with different MACs are two devices: one gone, one
//! new.
//!
//! What can't be told isn't claimed:
//! - Ports are compared only when both maps had their ports checked
//!   (a sweep's Check mgmt ports, or a source that lists them); otherwise
//!   "closed" would only mean "not looked at".
//! - Clients come and go, and a saved map holds only the ones it showed.
//!   One saved with every client (Clients: All) has its new clients
//!   flagged; otherwise only new infrastructure is, and a client is still
//!   reported gone or changed.

use super::{Map, Node, Zone};
use crate::net::mac::{self, Mac};
use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;

/// What changed on a device found on both maps.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Ip {
        was: Ipv4Addr,
        now: Ipv4Addr,
    },
    /// Ports open now that weren't then.
    Opened(Vec<u16>),
    /// Ports open then that aren't now.
    Closed(Vec<u16>),
    Name {
        was: String,
        now: String,
    },
}

impl Change {
    pub fn text(&self) -> String {
        let ports = |p: &[u16]| {
            p.iter()
                .map(|p| {
                    let s = crate::net::ports::service_name(*p);
                    if s.is_empty() {
                        p.to_string()
                    } else {
                        format!("{p} ({s})")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        match self {
            Change::Ip { was, now } => format!("IP {was} → {now}"),
            Change::Opened(p) => format!("Newly open: {}", ports(p)),
            Change::Closed(p) => format!("No longer open: {}", ports(p)),
            Change::Name { was, now } => format!("Name {was} → {now}"),
        }
    }
}

/// A device on the map now, listed as new or changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Here {
    /// Its card's id on the map now.
    pub id: String,
    pub label: String,
    pub kind: String,
    pub ip: Option<Ipv4Addr>,
    pub mac: Option<Mac>,
    pub client: bool,
}

/// A device on the saved map that isn't on the map now.
#[derive(Clone, Debug, PartialEq)]
pub struct Gone {
    pub label: String,
    pub kind: String,
    pub ip: Option<Ipv4Addr>,
    pub mac: Option<Mac>,
    pub client: bool,
}

/// The two maps compared.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diff {
    pub new: Vec<Here>,
    pub changed: Vec<(Here, Vec<Change>)>,
    pub gone: Vec<Gone>,
    /// Devices on both, unchanged.
    pub same: usize,
    /// Both maps had ports checked, so open ports were compared.
    pub ports: bool,
    /// New clients were looked for (the saved map had every client).
    pub clients: bool,
}

impl Diff {
    /// What changed on card `id`, if it's on the list: None for a card
    /// that's new or unchanged.
    pub fn changes(&self, id: &str) -> Option<&[Change]> {
        self.changed
            .iter()
            .find(|(h, _)| h.id == id)
            .map(|(_, c)| c.as_slice())
    }

    pub fn is_new(&self, id: &str) -> bool {
        self.new.iter().any(|h| h.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.new.is_empty() && self.changed.is_empty() && self.gone.is_empty()
    }

    /// "2 new · 1 changed · 3 gone", or "No changes".
    pub fn counts(&self) -> String {
        if self.is_empty() {
            return "No changes".into();
        }
        let mut v = Vec::new();
        for (n, w) in [
            (self.new.len(), "new"),
            (self.changed.len(), "changed"),
            (self.gone.len(), "gone"),
        ] {
            if n > 0 {
                v.push(format!("{n} {w}"));
            }
        }
        v.join(" · ")
    }

    /// The whole comparison as text, for Copy: against saved map `name`.
    pub fn report(&self, name: &str, saved_on: &str) -> String {
        let mut s = format!(
            "Changes since \"{name}\" (saved {saved_on}): {}\n",
            self.counts()
        );
        let who = |label: &str, kind: &str, ip: Option<Ipv4Addr>, m: Option<Mac>| {
            let mut parts = vec![label.to_string()];
            if !kind.is_empty() {
                parts.push(kind.to_lowercase());
            }
            if let Some(ip) = ip.filter(|ip| ip.to_string() != label) {
                parts.push(ip.to_string());
            }
            if let Some(m) = m {
                parts.push(mac::fmt(&m));
            }
            parts.join(", ")
        };
        if !self.new.is_empty() {
            s += &format!("\nNew ({}):\n", self.new.len());
            for h in &self.new {
                s += &format!("  {}\n", who(&h.label, &h.kind, h.ip, h.mac));
            }
        }
        if !self.changed.is_empty() {
            s += &format!("\nChanged ({}):\n", self.changed.len());
            for (h, cs) in &self.changed {
                let what: Vec<String> = cs.iter().map(Change::text).collect();
                s += &format!(
                    "  {}: {}\n",
                    who(&h.label, &h.kind, h.ip, h.mac),
                    what.join("; ")
                );
            }
        }
        if !self.gone.is_empty() {
            s += &format!("\nGone ({}):\n", self.gone.len());
            for g in &self.gone {
                s += &format!("  {}\n", who(&g.label, &g.kind, g.ip, g.mac));
            }
        }
        let mut notes = Vec::new();
        if !self.ports {
            notes.push("open ports weren't compared (not checked on both maps)");
        }
        if !self.clients {
            notes.push("new clients weren't looked for (the saved map didn't show every client)");
        }
        if !notes.is_empty() {
            s += &format!("\nNote: {}.\n", notes.join("; "));
        }
        s
    }
}

/// A card that stands for a device: not the internet, not a "+N more"
/// stand-in, not one kept only because it's pinned, and not one drawn by
/// hand.
fn device(n: &Node) -> bool {
    n.zone != Zone::Internet
        && !n.id.starts_with("more:")
        && n.kind != "MORE CLIENTS"
        && !n.gone
        && !n.drawn
}

/// A MAC worth matching on: a real, single device's.
fn real(m: &Mac) -> bool {
    m[0] & 0x01 == 0 && m.iter().any(|&b| b != 0)
}

fn here(n: &Node) -> Here {
    Here {
        id: n.id.clone(),
        label: n.label.clone(),
        kind: n.kind.clone(),
        ip: n.ip,
        mac: n.mac,
        client: n.client,
    }
}

/// Compare `now` with the map saved before, `was`. `was_all`: the saved
/// map showed every client (Clients: All), so a client not on it is new.
pub fn diff(was: &Map, now: &Map, was_all: bool) -> Diff {
    let old: Vec<&Node> = was.nodes.iter().filter(|n| device(n)).collect();
    let cur: Vec<&Node> = now.nodes.iter().filter(|n| device(n)).collect();
    let mut by_mac: HashMap<Mac, Vec<usize>> = HashMap::new();
    let mut by_ip: HashMap<Ipv4Addr, Vec<usize>> = HashMap::new();
    let mut by_id: HashMap<&str, usize> = HashMap::new();
    for (k, n) in old.iter().enumerate() {
        if let Some(m) = n.mac.filter(real) {
            by_mac.entry(m).or_default().push(k);
        }
        if let Some(ip) = n.ip {
            by_ip.entry(ip).or_default().push(k);
        }
        by_id.entry(n.id.as_str()).or_insert(k);
    }
    let mut used = vec![false; old.len()];
    let mut pair: Vec<Option<usize>> = vec![None; cur.len()];
    let take = |cands: Option<&Vec<usize>>, ok: &dyn Fn(usize) -> bool, used: &mut Vec<bool>| {
        let k = cands?.iter().copied().find(|&k| !used[k] && ok(k))?;
        used[k] = true;
        Some(k)
    };
    // By MAC first: the same hardware, wherever DHCP put it.
    for (i, n) in cur.iter().enumerate() {
        if let Some(m) = n.mac.filter(real) {
            pair[i] = take(by_mac.get(&m), &|_| true, &mut used);
        }
    }
    // Then by IP, unless both sides have a MAC (then they'd have matched:
    // different MACs at one address are different devices).
    for (i, n) in cur.iter().enumerate() {
        if pair[i].is_some() {
            continue;
        }
        let mine = n.mac.filter(real);
        if let Some(ip) = n.ip {
            let ok = |k: usize| mine.is_none() || old[k].mac.filter(real).is_none();
            pair[i] = take(by_ip.get(&ip), &ok, &mut used);
        }
    }
    // Last, with no MAC on either side, by the card's id: a /mapnetwork
    // card keeps its own id when its address changes. (A sweep's ids are
    // its IP or MAC, so this adds nothing there.)
    for (i, n) in cur.iter().enumerate() {
        if pair[i].is_none() && n.mac.filter(real).is_none() {
            let ok = |k: usize| old[k].mac.filter(real).is_none();
            pair[i] = by_id
                .get(n.id.as_str())
                .copied()
                .filter(|&k| !used[k] && ok(k));
            if let Some(k) = pair[i] {
                used[k] = true;
            }
        }
    }

    let checked = |m: &[&Node]| m.iter().any(|n| !n.open.is_empty());
    let ports = checked(&old) && checked(&cur);
    let mut d = Diff {
        ports,
        clients: was_all,
        ..Diff::default()
    };
    for (i, n) in cur.iter().enumerate() {
        let Some(k) = pair[i] else {
            if !n.client || was_all {
                d.new.push(here(n));
            }
            continue;
        };
        let o = old[k];
        let mut cs = Vec::new();
        if let (Some(a), Some(b)) = (o.ip, n.ip)
            && a != b
        {
            cs.push(Change::Ip { was: a, now: b });
        }
        if ports {
            let then: HashSet<u16> = o.open.iter().copied().collect();
            let now: HashSet<u16> = n.open.iter().copied().collect();
            let mut opened: Vec<u16> = now.difference(&then).copied().collect();
            let mut closed: Vec<u16> = then.difference(&now).copied().collect();
            opened.sort_unstable();
            closed.sort_unstable();
            if !opened.is_empty() {
                cs.push(Change::Opened(opened));
            }
            if !closed.is_empty() {
                cs.push(Change::Closed(closed));
            }
        }
        // A name, not an address standing in for one.
        let named = |s: &str| !s.is_empty() && s.parse::<Ipv4Addr>().is_err();
        if named(&o.label) && named(&n.label) && o.label != n.label {
            cs.push(Change::Name {
                was: o.label.clone(),
                now: n.label.clone(),
            });
        }
        if cs.is_empty() {
            d.same += 1;
        } else {
            d.changed.push((here(n), cs));
        }
    }
    for (k, o) in old.iter().enumerate() {
        if !used[k] {
            d.gone.push(Gone {
                label: o.label.clone(),
                kind: o.kind.clone(),
                ip: o.ip,
                mac: o.mac,
                client: o.client,
            });
        }
    }
    // Infrastructure first, then by address.
    let key = |client: bool, ip: Option<Ipv4Addr>, label: &str| {
        (
            client,
            ip.map(u32::from).unwrap_or(u32::MAX),
            label.to_string(),
        )
    };
    d.new.sort_by_key(|h| key(h.client, h.ip, &h.label));
    d.changed
        .sort_by_key(|(h, _)| key(h.client, h.ip, &h.label));
    d.gone.sort_by_key(|g| key(g.client, g.ip, &g.label));
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, label: &str, ip: Option<&str>, mac: Option<Mac>, open: &[u16]) -> Node {
        Node {
            id: id.into(),
            zone: Zone::Server,
            kind: "SERVER".into(),
            label: label.into(),
            ip: ip.map(|s| s.parse().unwrap()),
            mac,
            open: open.to_vec(),
            ..Node::default()
        }
    }

    fn map(nodes: Vec<Node>) -> Map {
        let mut internet = Node {
            id: "internet".into(),
            zone: Zone::Internet,
            label: "Internet".into(),
            ..Node::default()
        };
        internet.kind = "WAN".into();
        let mut all = vec![internet];
        all.extend(nodes);
        Map {
            nodes: all,
            ..Map::default()
        }
    }

    const A: Mac = [0x00, 0x11, 0x32, 0, 0, 1];
    const B: Mac = [0x00, 0x11, 0x32, 0, 0, 2];
    const C: Mac = [0x00, 0x11, 0x32, 0, 0, 3];

    #[test]
    fn nothing_changed_is_no_changes() {
        let m = || {
            map(vec![node(
                "mac:a",
                "NAS",
                Some("10.0.0.5"),
                Some(A),
                &[443],
            )])
        };
        let d = diff(&m(), &m(), true);
        assert!(d.is_empty());
        assert_eq!(d.same, 1);
        assert_eq!(d.counts(), "No changes");
    }

    #[test]
    fn a_device_is_followed_by_its_mac_when_dhcp_moves_it() {
        let was = map(vec![node("mac:a", "NAS", Some("10.0.0.5"), Some(A), &[])]);
        let now = map(vec![node("mac:a", "NAS", Some("10.0.0.9"), Some(A), &[])]);
        let d = diff(&was, &now, true);
        assert!(d.new.is_empty() && d.gone.is_empty());
        assert_eq!(
            d.changed[0].1,
            [Change::Ip {
                was: "10.0.0.5".parse().unwrap(),
                now: "10.0.0.9".parse().unwrap()
            }]
        );
        assert_eq!(d.changed[0].1[0].text(), "IP 10.0.0.5 → 10.0.0.9");
    }

    #[test]
    fn another_device_on_a_reused_ip_is_one_gone_and_one_new() {
        let was = map(vec![node("ip", "Printer", Some("10.0.0.30"), Some(A), &[])]);
        let now = map(vec![node("ip", "Laptop", Some("10.0.0.30"), Some(B), &[])]);
        let d = diff(&was, &now, true);
        assert_eq!(d.new.len(), 1);
        assert_eq!(d.new[0].label, "Laptop");
        assert_eq!(d.gone.len(), 1);
        assert_eq!(d.gone[0].label, "Printer");
        assert_eq!(d.counts(), "1 new · 1 gone");
    }

    #[test]
    fn without_macs_the_ip_matches() {
        let was = map(vec![node(
            "ip:10.1.0.1",
            "Branch fw",
            Some("10.1.0.1"),
            None,
            &[],
        )]);
        let now = map(vec![node(
            "ip:10.1.0.1",
            "Branch fw",
            Some("10.1.0.1"),
            Some(C),
            &[],
        )]);
        assert!(diff(&was, &now, true).is_empty());
    }

    #[test]
    fn ports_are_compared_only_when_both_maps_checked_them() {
        let was = map(vec![
            node("a", "Server", Some("10.0.0.5"), Some(A), &[22, 443]),
            node("b", "Switch", Some("10.0.0.2"), Some(B), &[80]),
        ]);
        let now = map(vec![
            node("a", "Server", Some("10.0.0.5"), Some(A), &[443, 3389]),
            node("b", "Switch", Some("10.0.0.2"), Some(B), &[80]),
        ]);
        let d = diff(&was, &now, true);
        assert!(d.ports);
        assert_eq!(
            d.changes("a").unwrap(),
            [Change::Opened(vec![3389]), Change::Closed(vec![22])]
        );
        assert_eq!(d.changes("a").unwrap()[0].text(), "Newly open: 3389 (rdp)");
        assert!(d.changes("b").is_none());
        // A map with no ports checked: "closed" would only mean "not looked".
        let bare = map(vec![
            node("a", "Server", Some("10.0.0.5"), Some(A), &[]),
            node("b", "Switch", Some("10.0.0.2"), Some(B), &[]),
        ]);
        let d = diff(&was, &bare, true);
        assert!(!d.ports && d.is_empty());
        assert!(d.report("x", "y").contains("open ports weren't compared"));
    }

    #[test]
    fn a_rename_is_a_change_but_an_address_for_a_name_isnt() {
        let was = map(vec![
            node("a", "OLD-PC", Some("10.0.0.5"), Some(A), &[]),
            node("b", "10.0.0.6", Some("10.0.0.6"), Some(B), &[]),
        ]);
        let now = map(vec![
            node("a", "NEW-PC", Some("10.0.0.5"), Some(A), &[]),
            node("b", "nas.local", Some("10.0.0.6"), Some(B), &[]),
        ]);
        let d = diff(&was, &now, true);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].1[0].text(), "Name OLD-PC → NEW-PC");
    }

    #[test]
    fn new_clients_count_only_when_the_saved_map_had_every_client() {
        let mut phone = node("mac:c", "Phone", Some("10.0.0.80"), Some(C), &[]);
        phone.client = true;
        let mut ap = node("mac:b", "AP", Some("10.0.0.3"), Some(B), &[]);
        ap.zone = Zone::Wireless;
        let was = map(vec![]);
        let now = map(vec![ap.clone(), phone.clone()]);
        // Saved with Clients: Some: only the AP is new.
        let d = diff(&was, &now, false);
        assert_eq!(d.new.len(), 1);
        assert_eq!(d.new[0].label, "AP");
        assert!(
            d.report("x", "y")
                .contains("new clients weren't looked for")
        );
        // Saved with every client: the phone is new too, after the AP.
        let d = diff(&was, &now, true);
        let names: Vec<&str> = d.new.iter().map(|h| h.label.as_str()).collect();
        assert_eq!(names, ["AP", "Phone"]);
        // A client gone is gone either way.
        let d = diff(&map(vec![phone]), &map(vec![]), false);
        assert_eq!(d.gone.len(), 1);
    }

    #[test]
    fn stand_ins_and_pinned_leftovers_are_not_devices() {
        let mut more = node("more:sw", "+12 more", None, None, &[]);
        more.kind = "MORE CLIENTS".into();
        let mut kept = node("mac:b", "Old AP", Some("10.0.0.3"), Some(B), &[]);
        kept.gone = true;
        let d = diff(&map(vec![]), &map(vec![more, kept]), true);
        assert!(d.is_empty());
    }

    #[test]
    fn a_card_without_a_mac_keeps_its_id_when_its_ip_changes() {
        let was = map(vec![node(
            "srv2",
            "Hypervisor",
            Some("10.10.5.11"),
            None,
            &[],
        )]);
        let now = map(vec![node(
            "srv2",
            "Hypervisor",
            Some("10.10.5.12"),
            None,
            &[],
        )]);
        let d = diff(&was, &now, true);
        assert!(d.new.is_empty() && d.gone.is_empty());
        assert_eq!(d.changed[0].1[0].text(), "IP 10.10.5.11 → 10.10.5.12");
        // An IP-made id at another address is another device.
        let was = map(vec![node("ip:10.0.0.5", "x", Some("10.0.0.5"), None, &[])]);
        let now = map(vec![node("ip:10.0.0.6", "x", Some("10.0.0.6"), None, &[])]);
        assert_eq!(diff(&was, &now, true).counts(), "1 new · 1 gone");
    }

    #[test]
    fn cards_with_neither_ip_nor_mac_match_by_id() {
        let was = map(vec![node("sw-closet", "Closet switch", None, None, &[])]);
        let now = map(vec![node("sw-closet", "Closet switch", None, None, &[])]);
        assert!(diff(&was, &now, true).is_empty());
    }

    #[test]
    fn the_report_lists_each_group() {
        let was = map(vec![
            node("a", "NAS", Some("10.0.0.5"), Some(A), &[443]),
            node("b", "Printer", Some("10.0.0.30"), Some(B), &[9100]),
        ]);
        let now = map(vec![
            node("a", "NAS", Some("10.0.0.5"), Some(A), &[443, 23]),
            node("c", "DESKTOP-7", Some("10.0.0.50"), Some(C), &[445]),
        ]);
        let r = diff(&was, &now, true).report("HQ visit", "2026-10-01");
        assert!(r.starts_with(
            "Changes since \"HQ visit\" (saved 2026-10-01): 1 new · 1 changed · 1 gone"
        ));
        assert!(r.contains("New (1):\n  DESKTOP-7, server, 10.0.0.50, 00:11:32:00:00:03"));
        assert!(r.contains(
            "Changed (1):\n  NAS, server, 10.0.0.5, 00:11:32:00:00:01: Newly open: 23 (telnet)"
        ));
        assert!(r.contains("Gone (1):\n  Printer, server, 10.0.0.30, 00:11:32:00:00:02"));
        assert!(!r.contains("Note:"));
    }
}
