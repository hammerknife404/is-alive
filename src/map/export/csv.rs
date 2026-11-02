//! The map as an inventory: one row per card for a spreadsheet, with
//! cells Excel would run as a formula defused (names come off the network).

use super::Drawn;
use crate::log::{CSV_START, csv_row};
use crate::map::{Link, Node};

const HEAD: [&str; 13] = [
    "name", "role", "kind", "ip", "mac", "vendor", "site", "uplink", "port", "link", "address",
    "flags", "findings",
];

pub fn write(d: &Drawn) -> String {
    let mut s = String::from(CSV_START);
    s += &csv_row(HEAD);
    for (i, n, _) in d.cards() {
        let (uplink, port, link) = match &n.up {
            Some((p, l, port)) => (
                d.map.nodes[*p].label.clone(),
                port.clone().unwrap_or_default(),
                link_name(*l).to_string(),
            ),
            None => Default::default(),
        };
        let row = [
            n.label.clone(),
            n.zone.name().to_string(),
            n.kind.clone(),
            n.ip.map(|ip| ip.to_string()).unwrap_or_default(),
            n.mac.as_ref().map(crate::net::mac::fmt).unwrap_or_default(),
            vendor(n).unwrap_or_default(),
            n.site.clone().unwrap_or_default(),
            uplink,
            port,
            link,
            n.sub.clone(),
            n.flags.join("; "),
            d.findings_of(i).join("; "),
        ];
        s += &csv_row(row.iter().map(String::as_str));
    }
    s
}

fn link_name(l: Link) -> &'static str {
    match l {
        Link::Backbone => "backbone",
        Link::Wired => "wired",
        Link::Wireless => "wireless",
        Link::Assumed => "assumed",
    }
}

/// The vendor a card's details name ("Vendor Ubiquiti (UniFi)"), else
/// its MAC prefix's.
pub fn vendor(n: &Node) -> Option<String> {
    n.details
        .iter()
        .find_map(|d| d.strip_prefix("Vendor "))
        .filter(|v| !v.starts_with("unknown"))
        .map(str::to_string)
        .or_else(|| {
            n.mac
                .as_ref()
                .and_then(crate::net::mac::vendor)
                .map(str::to_string)
        })
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_acme;
    use super::*;
    use crate::map::Map;
    use crate::map::layout::{self, Mode};

    /// Split on commas outside quotes, one record per CRLF line.
    fn rows(s: &str) -> Vec<Vec<String>> {
        let s = s.strip_prefix(CSV_START).expect("BOM");
        let mut out = vec![];
        let (mut row, mut cell, mut quoted) = (vec![], String::new(), false);
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '"' if quoted && it.peek() == Some(&'"') => {
                    cell.push('"');
                    it.next();
                }
                '"' => quoted = !quoted,
                ',' if !quoted => row.push(std::mem::take(&mut cell)),
                '\r' if !quoted => {}
                '\n' if !quoted => {
                    row.push(std::mem::take(&mut cell));
                    out.push(std::mem::take(&mut row));
                }
                c => cell.push(c),
            }
        }
        out
    }

    #[test]
    fn acme_has_a_row_per_card() {
        with_acme(Mode::Sites, |d| {
            let r = rows(&write(d));
            assert_eq!(r[0], HEAD);
            assert_eq!(r.len() - 1, d.cards().count());
            assert!(r.iter().all(|row| row.len() == HEAD.len()));
            // Every card names its site and every card but the top its uplink.
            assert!(r[1..].iter().filter(|row| !row[7].is_empty()).count() >= r.len() - 3);
        });
    }

    #[test]
    fn formulas_from_the_network_are_defused() {
        let m = Map {
            nodes: vec![
                Node {
                    label: "=HYPERLINK(\"http://x\",\"y\")".into(),
                    kind: "+cmd".into(),
                    ..Default::default()
                },
                Node {
                    label: "@SUM(A1)".into(),
                    sub: "-2+3".into(),
                    up: Some((0, Link::Wired, Some("=1+1".into()))),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let lay = layout::layout_in(&m, &|_| true, Mode::Cluster);
        let s = write(&Drawn {
            map: &m,
            lay: &lay,
            pos: &lay.pos,
            moved: &[false, false],
            legend: false,
            caption: None,
        });
        let r = rows(&s);
        assert_eq!(r.len(), 3);
        for row in &r[1..] {
            for cell in row {
                assert!(!cell.starts_with(['=', '+', '-', '@']), "{cell}");
            }
        }
        assert_eq!(r[1][0], "'=HYPERLINK(\"http://x\",\"y\")");
        assert_eq!(r[2][8], "'=1+1");
    }
}
