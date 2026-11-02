//! The map as a Mermaid flowchart, for Markdown (GitHub, GitLab, Azure
//! DevOps, Notion, Obsidian draw it): a node per card in its role's
//! colours, a subgraph per box, links styled as on the canvas.

use super::{Drawn, ink};
use crate::map::{Link, Zone};
use std::fmt::Write;

/// A label safe inside Mermaid's `"…"`: quotes, `#`, `&`, angle brackets
/// and backticks as Mermaid's `#code;` entities, line breaks as spaces.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' | '#' | '&' | '<' | '>' | '`' | ';' | '|' | '\\' => {
                let _ = write!(o, "#{};", c as u32);
            }
            c if c.is_control() => o.push(' '),
            c => o.push(c),
        }
    }
    o
}

/// The class a role's cards get.
fn class(z: Zone) -> &'static str {
    match z {
        Zone::Internet => "internet",
        Zone::Gateway => "gateway",
        Zone::Switch => "switch",
        Zone::Wireless => "ap",
        Zone::Gear => "gear",
        Zone::Server => "server",
        Zone::Device => "device",
        Zone::Client => "client",
        Zone::Unknown => "unknown",
    }
}

pub fn write(d: &Drawn) -> String {
    let mut s = String::new();
    // The title in front matter, a YAML string.
    let title: String = d
        .title()
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let _ = writeln!(s, "---\ntitle: \"{title}\"\n---");
    s += "flowchart TB\n";
    let node = |s: &mut String, i: usize, indent: &str| {
        let n = &d.map.nodes[i];
        let mut label = format!("{}<br/><b>{}</b>", esc(&n.kind), esc(&n.label));
        if !n.sub.is_empty() {
            let _ = write!(label, "<br/>{}", esc(&n.sub));
        }
        if !n.flags.is_empty() {
            let _ = write!(label, "<br/>⚠ {}", esc(&n.flags.join(", ")));
        }
        let _ = writeln!(s, "{indent}n{i}[\"{label}\"]:::{}", class(n.zone));
    };
    let cards: Vec<usize> = d.cards().map(|(i, _, _)| i).collect();
    for (k, g) in d.lay.boxes.iter().enumerate() {
        let _ = writeln!(s, "  subgraph g{k}[\"{}\"]", esc(&d.box_label(g)));
        for &i in cards.iter().filter(|&&i| d.group(i) == Some(k)) {
            node(&mut s, i, "    ");
        }
        s += "  end\n";
    }
    for &i in cards.iter().filter(|&&i| d.group(i).is_none()) {
        node(&mut s, i, "  ");
    }
    // Every link between two cards shown, Zones' in-box ones too: Mermaid
    // lays the chart out itself.
    let mut assumed = Vec::new();
    let mut count = 0;
    for &i in &cards {
        let Some((up, link, port)) = d.up(i) else {
            continue;
        };
        let arrow = match link {
            Link::Backbone => "===",
            Link::Wired => "---",
            Link::Wireless | Link::Assumed => "-.-",
        };
        let label = port.map_or(String::new(), |p| format!("|\"{}\"|", esc(p)));
        let _ = writeln!(s, "  n{up} {arrow}{label} n{i}");
        if link == Link::Assumed {
            assumed.push(count.to_string());
        }
        count += 1;
    }
    for z in Zone::ALL {
        let c = ink(z);
        let _ = writeln!(
            s,
            "  classDef {} fill:#ffffff,stroke:{c},stroke-width:2px,color:#1d2128",
            class(z)
        );
    }
    if !assumed.is_empty() {
        let _ = writeln!(
            s,
            "  linkStyle {} stroke:#8a93a3,stroke-dasharray:2 4",
            assumed.join(",")
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::super::tests::with_acme;
    use super::*;
    use crate::map::layout::Mode;
    use std::collections::HashSet;

    #[test]
    fn labels_cant_break_out_of_their_quotes() {
        let e = esc("a\"]-->x[\"b #1; <script> `c` |d|\n");
        assert!(!e.contains(['"', '<', '>', '`', '|', '\n']));
        assert!(e.contains("#34;") && e.contains("#35;1#59;"));
    }

    #[test]
    fn acme_is_a_sane_flowchart() {
        for mode in [Mode::Sites, Mode::Zones] {
            with_acme(mode, |d| {
                let s = write(d);
                assert!(s.starts_with("---\ntitle: \"Acme"));
                let body: Vec<&str> = s.lines().skip(3).collect();
                assert_eq!(body[0], "flowchart TB");
                // Subgraphs open and close in pairs.
                let open = body
                    .iter()
                    .filter(|l| l.trim_start().starts_with("subgraph "))
                    .count();
                let end = body.iter().filter(|l| l.trim() == "end").count();
                assert_eq!((open, end), (d.lay.boxes.len(), d.lay.boxes.len()));
                // One node per card, each id once; links only between them.
                let ids: Vec<&str> = body
                    .iter()
                    .filter_map(|l| l.trim().split_once("[\"").map(|x| x.0))
                    .filter(|id| id.starts_with('n'))
                    .collect();
                let set: HashSet<&str> = ids.iter().copied().collect();
                assert_eq!(set.len(), ids.len());
                assert_eq!(ids.len(), d.cards().count());
                for l in body
                    .iter()
                    .filter(|l| l.contains("---") || l.contains("-.-") || l.contains("==="))
                {
                    let w: Vec<&str> = l.split_whitespace().collect();
                    assert!(set.contains(w[0]) && set.contains(w.last().unwrap()), "{l}");
                }
                // Quotes balance on every line.
                assert!(body.iter().all(|l| l.matches('"').count() % 2 == 0));
            });
        }
    }
}
