//! The map for Graphviz: `dot` lays it out afresh, `neato -n2` keeps the
//! map's own layout (each card's position is in it). Boxes become
//! clusters, links are styled as on the canvas.

use super::{Drawn, ink};
use crate::map::layout::CARD;
use crate::map::{Link, Zone};
use std::fmt::Write;

/// A DOT string's inside: quotes and backslashes escaped (so `\N` and
/// friends stay text), line breaks as `\n`.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o += "\\\"",
            '\\' => o += "\\\\",
            '\n' => o += "\\n",
            c if c.is_control() => {}
            c => o.push(c),
        }
    }
    o
}

pub fn write(d: &Drawn) -> String {
    let b = d.bounds();
    let mut s = String::new();
    let _ = writeln!(
        s,
        "// {} · is-alive {}\n// dot -Tsvg lays it out afresh; neato -n2 -Tsvg keeps the map's layout.",
        esc(d.title()).replace('\n', " "),
        crate::VERSION
    );
    let _ = writeln!(s, "graph \"is-alive map\" {{");
    let _ = writeln!(
        s,
        "  graph [label=\"{}\", labelloc=t, fontname=\"Helvetica\", fontsize=18, splines=ortho, nodesep=0.3, ranksep=0.6];",
        esc(d.title())
    );
    let _ = writeln!(
        s,
        "  node [shape=box, style=\"rounded,filled\", fillcolor=\"#ffffff\", fontname=\"Helvetica\", fontsize=10, fontcolor=\"#1d2128\", penwidth=1.5, width={:.2}, height={:.2}];",
        CARD.x / 72.0,
        CARD.y / 72.0
    );
    s += "  edge [penwidth=1.4];\n";
    let node = |s: &mut String, i: usize, indent: &str| {
        let n = &d.map.nodes[i];
        let mut label = format!("{}\n{}", n.kind, n.label);
        if !n.sub.is_empty() {
            label = format!("{label}\n{}", n.sub);
        }
        if !n.flags.is_empty() {
            label = format!("{label}\n! {}", n.flags.join(", "));
        }
        // Points, y up, at the card's middle: what neato -n2 reads.
        let p = d.at(i).unwrap_or_default();
        let (x, y) = (p.x + CARD.x / 2.0 - b.min.x, b.max.y - p.y - CARD.y / 2.0);
        let colour = if n.gone {
            "#c81e1e".into()
        } else {
            ink(n.zone)
        };
        let style = if n.gone {
            ", style=\"rounded,filled,dashed\""
        } else {
            ""
        };
        let _ = writeln!(
            s,
            "{indent}n{i} [label=\"{}\", color=\"{colour}\", pos=\"{x:.0},{y:.0}!\"{style}];",
            esc(&label)
        );
    };
    let cards: Vec<usize> = d.cards().map(|(i, _, _)| i).collect();
    for (k, g) in d.lay.boxes.iter().enumerate() {
        let c = g.zone.map_or("#69707c".into(), ink);
        let _ = writeln!(
            s,
            "  subgraph cluster_{k} {{\n    label=\"{}\"; labeljust=l; style=\"rounded\"; color=\"{c}\"; fontcolor=\"{c}\"; fontname=\"Helvetica\"; fontsize=11;",
            esc(&d.box_label(g))
        );
        for &i in cards.iter().filter(|&&i| d.group(i) == Some(k)) {
            node(&mut s, i, "    ");
        }
        s += "  }\n";
    }
    for &i in cards.iter().filter(|&&i| d.group(i).is_none()) {
        node(&mut s, i, "  ");
    }
    for &i in &cards {
        let Some((up, link, port)) = d.up(i) else {
            continue;
        };
        let n = &d.map.nodes[i];
        let mut attrs = match link {
            Link::Backbone => format!("color=\"{}\", penwidth=3", ink(Zone::Gateway)),
            Link::Wired => format!("color=\"{}\"", ink(n.zone)),
            Link::Wireless => format!("color=\"{}\", style=dashed", ink(n.zone)),
            Link::Assumed => "color=\"#8a93a3\", style=dotted".to_string(),
        };
        if let Some(p) = port {
            let _ = write!(
                attrs,
                ", xlabel=\"{}\", fontsize=9, fontname=\"Helvetica\"",
                esc(p)
            );
        }
        let _ = writeln!(s, "  n{up} -- n{i} [{attrs}];");
    }
    s += "}\n";
    s
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_acme;
    use super::*;
    use crate::map::layout::Mode;
    use std::collections::HashSet;

    #[test]
    fn strings_stay_strings() {
        assert_eq!(esc("a\"b\\N\nc"), "a\\\"b\\\\N\\nc");
    }

    /// Braces and quotes balance outside strings; unescaped quotes pair up.
    fn balanced(s: &str) -> bool {
        let (mut depth, mut quoted, mut esc) = (0i32, false, false);
        for line in s.lines().filter(|l| !l.trim_start().starts_with("//")) {
            for c in line.chars() {
                match c {
                    _ if esc => esc = false,
                    '\\' if quoted => esc = true,
                    '"' => quoted = !quoted,
                    '{' if !quoted => depth += 1,
                    '}' if !quoted => depth -= 1,
                    _ => {}
                }
                if depth < 0 {
                    return false;
                }
            }
            if quoted {
                return false;
            }
        }
        depth == 0
    }

    #[test]
    fn acme_is_a_sane_graph() {
        for mode in [Mode::Sites, Mode::Zones] {
            with_acme(mode, |d| {
                let s = write(d);
                assert!(balanced(&s), "{s}");
                let ids: Vec<&str> = s
                    .lines()
                    .map(str::trim)
                    .filter(|l| l.starts_with('n') && l.contains(" [label="))
                    .map(|l| l.split(' ').next().unwrap())
                    .collect();
                let set: HashSet<&str> = ids.iter().copied().collect();
                assert_eq!(set.len(), ids.len());
                assert_eq!(ids.len(), d.cards().count());
                assert_eq!(s.matches("subgraph cluster_").count(), d.lay.boxes.len());
                for l in s.lines().filter(|l| l.contains(" -- ")) {
                    let w: Vec<&str> = l.split_whitespace().collect();
                    assert!(set.contains(w[0]) && set.contains(w[2]), "{l}");
                }
            });
        }
        assert!(!balanced("graph { \"a\\\" }"));
    }
}
