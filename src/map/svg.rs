//! The map as a standalone SVG: same layout, colours and cards, a title
//! line and the findings underneath, for a ticket or a site doc.

use super::layout::{self, CARD, Layout};
use super::{Link, Map, Zone};
use eframe::egui::{Pos2, Rect, vec2};
use std::fmt::Write;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// A role's colour in the theme in use, as on screen.
fn role(z: Zone) -> String {
    let c = z.colour_in(crate::theme::current());
    hex([c.r(), c.g(), c.b()])
}

/// Split into at most `rows` lines of about `width` characters, with an
/// ellipsis when it doesn't fit.
fn wrap(s: &str, width: usize, rows: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur += word;
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.len() > rows {
        lines.truncate(rows);
        let last = lines.last_mut().expect("rows > 0");
        last.push('…');
    }
    for l in &mut lines {
        if l.chars().count() > width + 1 {
            *l = l.chars().take(width).collect::<String>() + "…";
        }
    }
    lines
}

/// The map as drawn: `pos` is where each card is (None: not shown),
/// `moved` whether it was dragged by hand. Links, trunks, boxes and chips
/// follow the canvas.
pub fn render(map: &Map, pos: &[Option<Pos2>], lay: &Layout, moved: &[bool], dark: bool) -> String {
    let boxes = &lay.boxes;
    let (bg, card, line, text, weak) = if dark {
        ("#16181c", "#202329", "#363b44", "#e8eaee", "#8b919c")
    } else {
        ("#f6f7f9", "#ffffff", "#d2d6dc", "#1d2128", "#69707c")
    };
    let mut b = Rect::NOTHING;
    for (n, p) in map.nodes.iter().zip(pos) {
        if let Some(p) = p {
            b = b.union(layout::card_rect(n, *p));
        }
    }
    for g in boxes.iter() {
        b = b.union(g.rect);
    }
    if !b.is_positive() {
        b = Rect::from_min_size(Pos2::ZERO, CARD);
    }
    let (m, head) = (32.0, 64.0);
    let foot = 28.0 + 20.0 * map.findings.len().min(12) as f32;
    let off = vec2(m - b.min.x, m + head - b.min.y);
    let (w, h) = (b.width() + 2.0 * m, b.height() + 2.0 * m + head + foot);
    let at = |p: Pos2| p + off;
    let mut s = String::new();
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}" font-family="Segoe UI, Helvetica, Arial, sans-serif">"#
    );
    let _ = writeln!(s, r#"<rect width="100%" height="100%" fill="{bg}"/>"#);
    let _ = writeln!(
        s,
        r#"<text x="{m}" y="{}" font-size="20" font-weight="600" fill="{text}">Network map</text>"#,
        m + 6.0
    );
    let _ = writeln!(
        s,
        r#"<text x="{m}" y="{}" font-size="12" fill="{weak}">{} · is-alive {}</text>"#,
        m + 26.0,
        esc(&map.summary),
        crate::VERSION
    );

    // Zones and Sites boxes, under the links and cards.
    for g in boxes.iter() {
        let r = g.rect;
        let p = at(r.min);
        let colour = g.zone.map_or_else(|| weak.to_string(), role);
        let _ = writeln!(
            s,
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="10" fill="{colour}" fill-opacity="0.07" stroke="{colour}" stroke-opacity="0.5"/>"#,
            p.x,
            p.y,
            r.width(),
            r.height()
        );
        // A collapsed site says how many cards it holds, as on screen.
        let count = match map.closed.get(&g.title).filter(|_| g.tree) {
            Some(inside) => format!("{} inside", inside.len()),
            None => g.count.to_string(),
        };
        let _ = writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" font-size="11" font-weight="600" fill="{colour}">{} · {count}</text>"#,
            p.x + layout::BOX_PAD,
            p.y + layout::BOX_TITLE / 2.0 + 4.0,
            esc(&g.title.to_uppercase()),
        );
    }

    let lines = layout::lines(map, lay, &|i| pos.get(i).copied().flatten(), &|i| {
        moved.get(i).copied().unwrap_or(false)
    });
    for line in lines.iter().filter(|l| !l.quiet) {
        let pts: Vec<Pos2> = line.pts.iter().map(|p| at(*p)).collect();
        let d: String = pts
            .iter()
            .enumerate()
            .map(|(k, p)| format!("{}{:.1} {:.1}", if k == 0 { "M" } else { " L" }, p.x, p.y))
            .collect();
        // A trunk carries its strongest link, in its box's colour.
        let n = line
            .kids
            .iter()
            .map(|&k| &map.nodes[k])
            .max_by_key(|m| m.link().map_or(0, layout::strength))
            .expect("a line carries a link");
        let link = n.link().unwrap_or(Link::Wired);
        let (colour, width, dash) = match link {
            Link::Backbone => (role(Zone::Gateway), 3.0, ""),
            Link::Wired => (role(n.zone), 1.6, ""),
            Link::Wireless => (role(n.zone), 1.6, r#" stroke-dasharray="7 5""#),
            Link::Assumed => (weak.to_string(), 1.4, r#" stroke-dasharray="2.5 4""#),
        };
        let into = line.into.and_then(|g| boxes.get(g));
        let colour = into.and_then(|g| g.zone).map_or(colour, role);
        let width = if into.is_some() { width + 0.6 } else { width };
        let _ = writeln!(
            s,
            r#"<path d="{d}" fill="none" stroke="{colour}" stroke-opacity="0.75" stroke-width="{width}"{dash}/>"#
        );
        if into.is_some() && line.kids.len() > 1 {
            let e = *pts.last().expect("route has points");
            let _ = writeln!(
                s,
                r#"<text x="{:.1}" y="{:.1}" font-size="10" fill="{weak}">×{}</text>"#,
                e.x + 5.0,
                e.y - 4.0,
                line.kids.len()
            );
        }
    }

    // In Sites the box already names the site.
    let sites = boxes.iter().any(|g| g.tree);
    for (i, n) in map.nodes.iter().enumerate() {
        let Some(p) = pos[i].map(at) else { continue };
        let accent = if n.gone {
            (if dark { "#ff4040" } else { "#c81e1e" }).to_string()
        } else {
            role(n.zone)
        };
        let (x, y) = (p.x, p.y);
        // A client is a tile: its name and address, nothing else.
        let size = layout::card_size(n);
        let tile = size.y < CARD.y;
        // A pinned card missing from the latest data: dashed, in red.
        let (edge, dash) = if n.gone {
            (accent.as_str(), r#" stroke-dasharray="6 4""#)
        } else {
            (line, "")
        };
        let _ = writeln!(
            s,
            r#"<g><rect x="{x:.1}" y="{y:.1}" width="{}" height="{}" rx="7" fill="{card}" stroke="{edge}"{dash}/>"#,
            size.x, size.y
        );
        let _ = writeln!(
            s,
            r#"<rect x="{x:.1}" y="{y:.1}" width="4" height="{}" rx="2" fill="{accent}"/>"#,
            size.y
        );
        if !tile {
            let _ = writeln!(
                s,
                r#"<text x="{:.1}" y="{:.1}" font-size="9.5" font-family="Consolas, Menlo, monospace" fill="{weak}">{}</text>"#,
                x + 12.0,
                y + 17.0,
                esc(&n.kind)
            );
        }
        let (top, rows) = if tile { (y + 20.0, 1) } else { (y + 34.0, 2) };
        let label = wrap(&n.label, 26, rows);
        for (k, l) in label.iter().enumerate() {
            let _ = writeln!(
                s,
                r#"<text x="{:.1}" y="{:.1}" font-size="13" font-weight="600" fill="{text}">{}</text>"#,
                x + 12.0,
                top + k as f32 * 16.0,
                esc(l)
            );
        }
        if !n.sub.is_empty() {
            let _ = writeln!(
                s,
                r#"<text x="{:.1}" y="{:.1}" font-size="10.5" font-family="Consolas, Menlo, monospace" fill="{accent}">{}</text>"#,
                x + 12.0,
                top + label.len().max(1) as f32 * 16.0,
                esc(&n.sub)
            );
        }
        // The uplink's port or note rides on the card, as on screen.
        let chips =
            n.up.as_ref()
                .and_then(|u| u.2.as_deref())
                .into_iter()
                .chain(n.chips.iter().map(String::as_str))
                .filter(|c| !(sites && n.site.as_deref() == Some(*c)) && !tile);
        let mut cx = x + 12.0;
        for c in chips {
            let cw = 10.0 + 5.6 * c.chars().count() as f32;
            if cx + cw > x + CARD.x - 8.0 {
                break;
            }
            let _ = writeln!(
                s,
                r#"<rect x="{cx:.1}" y="{:.1}" width="{cw:.1}" height="15" rx="7.5" fill="{accent}" fill-opacity="0.16"/><text x="{:.1}" y="{:.1}" font-size="9.5" fill="{accent}">{}</text>"#,
                y + CARD.y - 22.0,
                cx + 5.0,
                y + CARD.y - 11.0,
                esc(c)
            );
            cx += cw + 4.0;
        }
        if !n.flags.is_empty() {
            let _ = writeln!(
                s,
                r##"<circle cx="{:.1}" cy="{:.1}" r="7" fill="#e8b848"/><text x="{:.1}" y="{:.1}" font-size="11" font-weight="700" text-anchor="middle" fill="#000">!</text>"##,
                x + CARD.x - 12.0,
                y + 12.0,
                x + CARD.x - 12.0,
                y + 16.0
            );
        }
        s += "</g>\n";
    }

    let mut y = h - foot + 8.0;
    if !map.findings.is_empty() {
        let _ = writeln!(
            s,
            r#"<text x="{m}" y="{y:.1}" font-size="13" font-weight="600" fill="{text}">Findings</text>"#
        );
        for f in map.findings.iter().take(12) {
            y += 20.0;
            let c = match f.sev {
                super::Sev::High => "#eb5a5a",
                super::Sev::Med => "#e6b43c",
                super::Sev::Low => weak,
            };
            let _ = writeln!(
                s,
                r#"<text x="{m}" y="{y:.1}" font-size="11.5" fill="{text}"><tspan fill="{c}" font-weight="600">{}</tspan>  {}</text>"#,
                f.sev.label().to_uppercase(),
                esc(&f.text)
            );
        }
    }
    s += "</svg>\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{Input, Seen, build};

    #[test]
    fn svg_is_well_formed_and_escaped() {
        let m = build(&Input {
            sweep: vec![Seen {
                ip: "10.0.0.9".parse().unwrap(),
                mac: None,
                name: Some("a<b>&c".into()),
                rtt: Some(1.0),
                open: vec![23],
            }],
            gateway: Some("10.0.0.1".parse().unwrap()),
            range: Some("10.0.0.0/24".into()),
            ..Default::default()
        });
        let l = crate::map::layout::layout_in(&m, &|_| true, crate::map::layout::Mode::Cluster);
        let s = render(&m, &l.pos, &l, &vec![false; m.nodes.len()], true);
        assert!(s.starts_with("<svg") && s.trim_end().ends_with("</svg>"));
        assert!(s.contains("a&lt;b&gt;&amp;c"));
        assert!(!s.contains("a<b>"));
        assert!(s.contains("Telnet"));
        assert_eq!(s.matches("<g>").count(), s.matches("</g>").count());
    }

    #[test]
    fn zones_export_trunks_and_dragged_cards_as_on_screen() {
        // internet → gateway → 3 clients, each role in its own box.
        let zones = [
            Zone::Internet,
            Zone::Gateway,
            Zone::Client,
            Zone::Client,
            Zone::Client,
        ];
        let m = Map {
            nodes: zones
                .iter()
                .enumerate()
                .map(|(i, &zone)| crate::map::Node {
                    id: format!("n{i}"),
                    label: format!("n{i}"),
                    zone,
                    up: (i > 0).then(|| (i.min(2) - 1, Link::Wired, None)),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let l = crate::map::layout::layout_in(&m, &|_| true, crate::map::layout::Mode::Zones);
        let lines = layout::lines(&m, &l, &|i| l.pos[i], &|_| false);
        let trunk = lines
            .iter()
            .find(|x| x.kids.len() > 1)
            .expect("hosts share a trunk into their box");
        let s = render(&m, &l.pos, &l, &vec![false; m.nodes.len()], true);
        assert!(s.contains(&format!(">×{}<", trunk.kids.len())), "{s}");
        // Dragged, a card leaves the trunk and gets its own line.
        let mut moved = vec![false; m.nodes.len()];
        moved[trunk.kids[0]] = true;
        let dragged = render(&m, &l.pos, &l, &moved, true);
        assert!(dragged.contains(&format!(">×{}<", trunk.kids.len() - 1)));
    }

    #[test]
    fn clients_export_as_tiles() {
        let host = |ip: &str, name: &str| Seen {
            ip: ip.parse().unwrap(),
            mac: None,
            name: Some(name.into()),
            rtt: Some(1.0),
            open: vec![],
        };
        let m = build(&Input {
            sweep: vec![host("10.0.0.9", "laptop"), host("10.0.0.10", "desktop")],
            gateway: Some("10.0.0.1".parse().unwrap()),
            clients: crate::map::Clients::All,
            ..Default::default()
        });
        let l = crate::map::layout::layout_in(&m, &|_| true, crate::map::layout::Mode::Cluster);
        let s = render(&m, &l.pos, &l, &vec![false; m.nodes.len()], false);
        let tile = format!(r#"height="{}""#, layout::TILE.y);
        // A tile each, with its name and address but no kind line.
        assert_eq!(
            s.matches(&format!(r#"width="{}" {tile}"#, layout::TILE.x))
                .count(),
            2
        );
        assert!(s.contains(">laptop<") && s.contains(">10.0.0.9<"));
        assert!(!s.contains(">HOST<"));
    }

    #[test]
    fn wrap_clamps() {
        assert_eq!(wrap("Core Closet - Top", 26, 2), ["Core Closet - Top"]);
        let w = wrap(
            "Student Worship - AP 2 - 1st Floor and more words here",
            20,
            2,
        );
        assert_eq!(w.len(), 2);
        assert!(w[1].ends_with('…'));
    }
}
