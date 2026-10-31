//! The map as a standalone SVG: same layout, colours and cards, a title
//! line and the findings underneath, for a ticket or a site doc.

use super::layout::{self, CARD};
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

pub fn render(
    map: &Map,
    pos: &[Option<Pos2>],
    stacked: &[bool],
    boxes: &[layout::Group],
    dark: bool,
) -> String {
    let (bg, card, line, text, weak) = if dark {
        ("#16181c", "#202329", "#363b44", "#e8eaee", "#8b919c")
    } else {
        ("#f6f7f9", "#ffffff", "#d2d6dc", "#1d2128", "#69707c")
    };
    let mut b = Rect::NOTHING;
    for p in pos.iter().flatten() {
        b = b.union(Rect::from_min_size(*p, CARD));
    }
    for g in boxes {
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
    for g in boxes {
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
        let _ = writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" font-size="11" font-weight="600" fill="{colour}">{} · {}</text>"#,
            p.x + layout::BOX_PAD,
            p.y + layout::BOX_TITLE / 2.0 + 4.0,
            esc(&g.title.to_uppercase()),
            g.count
        );
    }

    for (i, n) in map.nodes.iter().enumerate() {
        let (Some((up, link, label)), Some(c)) = (&n.up, pos[i]) else {
            continue;
        };
        let Some(p) = pos[*up] else { continue };
        let pts: Vec<Pos2> = layout::route(p, c, stacked[i])
            .into_iter()
            .map(at)
            .collect();
        let d: String = pts
            .iter()
            .enumerate()
            .map(|(k, p)| format!("{}{:.1} {:.1}", if k == 0 { "M" } else { " L" }, p.x, p.y))
            .collect();
        let (colour, width, dash) = match link {
            Link::Backbone => (role(Zone::Gateway), 3.0, ""),
            Link::Wired => (role(n.zone), 1.6, ""),
            Link::Wireless => (role(n.zone), 1.6, r#" stroke-dasharray="7 5""#),
            Link::Assumed => (weak.to_string(), 1.4, r#" stroke-dasharray="2.5 4""#),
        };
        let _ = writeln!(
            s,
            r#"<path d="{d}" fill="none" stroke="{colour}" stroke-opacity="0.75" stroke-width="{width}"{dash}/>"#
        );
        if let Some(l) = label {
            let e = *pts.last().expect("route has points");
            let (x, anchor, l) = if stacked[i] {
                (
                    e.x - layout::INDENT / 4.0,
                    "middle",
                    l.trim_start_matches("port "),
                )
            } else {
                (e.x + 4.0, "start", l.as_str())
            };
            let _ = writeln!(
                s,
                r#"<text x="{x:.1}" y="{:.1}" font-size="10" fill="{weak}" text-anchor="{anchor}">{}</text>"#,
                e.y - 3.0,
                esc(l)
            );
        }
    }

    for (i, n) in map.nodes.iter().enumerate() {
        let Some(p) = pos[i].map(at) else { continue };
        let accent = if n.gone {
            (if dark { "#ff4040" } else { "#c81e1e" }).to_string()
        } else {
            role(n.zone)
        };
        let (x, y) = (p.x, p.y);
        // A pinned card missing from the latest data: dashed, in red.
        let (edge, dash) = if n.gone {
            (accent.as_str(), r#" stroke-dasharray="6 4""#)
        } else {
            (line, "")
        };
        let _ = writeln!(
            s,
            r#"<g><rect x="{x:.1}" y="{y:.1}" width="{}" height="{}" rx="7" fill="{card}" stroke="{edge}"{dash}/>"#,
            CARD.x, CARD.y
        );
        let _ = writeln!(
            s,
            r#"<rect x="{x:.1}" y="{y:.1}" width="4" height="{}" rx="2" fill="{accent}"/>"#,
            CARD.y
        );
        let _ = writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" font-size="9.5" font-family="Consolas, Menlo, monospace" fill="{weak}">{}</text>"#,
            x + 12.0,
            y + 17.0,
            esc(&n.kind)
        );
        for (k, l) in wrap(&n.label, 26, 2).iter().enumerate() {
            let _ = writeln!(
                s,
                r#"<text x="{:.1}" y="{:.1}" font-size="13" font-weight="600" fill="{text}">{}</text>"#,
                x + 12.0,
                y + 34.0 + k as f32 * 16.0,
                esc(l)
            );
        }
        if !n.sub.is_empty() {
            let rows = wrap(&n.label, 26, 2).len().max(1);
            let _ = writeln!(
                s,
                r#"<text x="{:.1}" y="{:.1}" font-size="10.5" font-family="Consolas, Menlo, monospace" fill="{accent}">{}</text>"#,
                x + 12.0,
                y + 34.0 + rows as f32 * 16.0,
                esc(&n.sub)
            );
        }
        let mut cx = x + 12.0;
        for c in &n.chips {
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
        let s = render(&m, &l.pos, &l.stacked, &l.boxes, true);
        assert!(s.starts_with("<svg") && s.trim_end().ends_with("</svg>"));
        assert!(s.contains("a&lt;b&gt;&amp;c"));
        assert!(!s.contains("a<b>"));
        assert!(s.contains("Telnet"));
        assert_eq!(s.matches("<g>").count(), s.matches("</g>").count());
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
