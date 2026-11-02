//! The map as a standalone SVG: same layout, colours and cards, a title
//! line, the legend and the findings underneath, for a ticket or a site doc.

use super::layout::{self, CARD, Layout};
use super::legend::{self, Entry, Ink, Swatch};
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

/// The export's colours: (background, card, card edge, text, weak text).
fn palette(dark: bool) -> [&'static str; 5] {
    if dark {
        ["#16181c", "#202329", "#363b44", "#e8eaee", "#8b919c"]
    } else {
        ["#f6f7f9", "#ffffff", "#d2d6dc", "#1d2128", "#69707c"]
    }
}

/// The red of a pinned card missing from the latest data.
fn gone(dark: bool) -> &'static str {
    if dark { "#ff4040" } else { "#c81e1e" }
}

/// The warning badge's amber.
const WARN: &str = "#e8b848";

/// How a link is drawn: (colour, width, dash attribute). `accent` is its
/// card's colour, `weak` the weak text colour.
fn link_style(link: Link, accent: &str, weak: &str) -> (String, f32, &'static str) {
    match link {
        Link::Backbone => (role(Zone::Gateway), 3.0, ""),
        Link::Wired => (accent.to_string(), 1.6, ""),
        Link::Wireless => (accent.to_string(), 1.6, r#" stroke-dasharray="7 5""#),
        Link::Assumed => (weak.to_string(), 1.4, r#" stroke-dasharray="2.5 4""#),
    }
}

/// A line along `d` (path data) in a link's style.
fn link_path(s: &mut String, d: &str, colour: &str, width: f32, dash: &str) {
    let _ = writeln!(
        s,
        r#"<path d="{d}" fill="none" stroke="{colour}" stroke-opacity="0.75" stroke-width="{width}"{dash}/>"#
    );
}

/// A card's body: fill, edge (dashed red when `gone`) and colour bar.
#[allow(clippy::too_many_arguments)]
fn card_body(s: &mut String, r: Rect, rx: f32, fill: &str, edge: &str, accent: &str, gone: bool) {
    let (edge, dash) = if gone {
        (accent, r#" stroke-dasharray="6 4""#)
    } else {
        (edge, "")
    };
    let _ = writeln!(
        s,
        r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="{rx}" fill="{fill}" stroke="{edge}"{dash}/>"#,
        r.min.x,
        r.min.y,
        r.width(),
        r.height()
    );
    let _ = writeln!(
        s,
        r#"<rect x="{:.1}" y="{:.1}" width="4" height="{:.1}" rx="2" fill="{accent}"/>"#,
        r.min.x,
        r.min.y,
        r.height()
    );
}

/// A chip's pill (its text over it).
fn chip_pill(s: &mut String, r: Rect, accent: &str) {
    let _ = writeln!(
        s,
        r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="{:.1}" fill="{accent}" fill-opacity="0.16"/>"#,
        r.min.x,
        r.min.y,
        r.width(),
        r.height(),
        r.height() / 2.0
    );
}

/// The warning badge centred at (`x`, `y`).
fn warn_badge(s: &mut String, x: f32, y: f32) {
    let _ = writeln!(
        s,
        r##"<circle cx="{x:.1}" cy="{y:.1}" r="7" fill="{WARN}"/><text x="{x:.1}" y="{:.1}" font-size="11" font-weight="700" text-anchor="middle" fill="#000">!</text>"##,
        y + 4.0
    );
}

/// A site's or a role's box.
fn box_rect(s: &mut String, r: Rect, rx: f32, colour: &str) {
    let _ = writeln!(
        s,
        r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="{rx}" fill="{colour}" fill-opacity="0.07" stroke="{colour}" stroke-opacity="0.5"/>"#,
        r.min.x,
        r.min.y,
        r.width(),
        r.height()
    );
}

/// The legend's rows and columns, laid out for a picture `w` wide.
struct LegendPlan {
    slots: Vec<legend::Slot>,
    /// A column's width, with the gap after it.
    col: f32,
    width: f32,
    height: f32,
}

const LEG_ROW: f32 = 18.0;
const LEG_SWATCH: f32 = 30.0;
/// Roughly how wide a character of the legend's 11.5 px text is.
const LEG_CHAR: f32 = 6.2;
const LEG_GAP: f32 = 20.0;

fn legend_plan(entries: &[Entry], w: f32) -> LegendPlan {
    let col = entries
        .iter()
        .map(|e| {
            let chars = e.label.chars().count() + e.count.map_or(0, |n| n.to_string().len() + 2);
            LEG_SWATCH + 8.0 + chars as f32 * LEG_CHAR
        })
        .fold(140.0f32, f32::max);
    // As many columns as the picture is wide for, as few rows as that lets
    // with the longest section in one column.
    let fit = (((w + LEG_GAP) / (col + LEG_GAP)).floor() as usize).max(1);
    let total = entries.len() + 4;
    let longest = entries
        .iter()
        .map(|e| entries.iter().filter(|x| x.section == e.section).count() + 1)
        .max()
        .unwrap_or(1);
    let mut rows = total.div_ceil(fit).max(longest);
    let slots = loop {
        let slots = legend::slots(entries, rows);
        if slots.iter().all(|s| s.col < fit) || rows >= total {
            break slots;
        }
        rows += 1;
    };
    let cols = slots.iter().map(|s| s.col + 1).max().unwrap_or(1);
    let used = slots.iter().map(|s| s.row + 1).max().unwrap_or(1);
    LegendPlan {
        slots,
        col: col + LEG_GAP,
        width: cols as f32 * (col + LEG_GAP) - LEG_GAP,
        height: 24.0 + used as f32 * LEG_ROW,
    }
}

/// A legend colour as the export draws the map.
fn ink(i: Ink, dark: bool, weak: &str) -> String {
    match i {
        Ink::Role(z) => role(z),
        Ink::Gone => gone(dark).to_string(),
        Ink::Weak => weak.to_string(),
        Ink::Warn => WARN.to_string(),
    }
}

/// The legend with its top-left at (`x`, `y`), each swatch drawn as the
/// export draws the map.
fn draw_legend(s: &mut String, entries: &[Entry], plan: &LegendPlan, x: f32, y: f32, dark: bool) {
    let [_, card, line, text, weak] = palette(dark);
    let _ = writeln!(
        s,
        r#"<g class="legend"><text x="{x}" y="{:.1}" font-size="13" font-weight="600" fill="{text}">Legend</text>"#,
        y + 12.0
    );
    for slot in &plan.slots {
        let rx = x + slot.col as f32 * plan.col;
        let mid = y + 24.0 + slot.row as f32 * LEG_ROW + LEG_ROW / 2.0;
        let Some(k) = slot.entry else {
            let title = slot.head.map_or("", |h| h.title());
            let _ = writeln!(
                s,
                r#"<text x="{rx:.1}" y="{:.1}" font-size="9.5" font-weight="600" fill="{weak}">{title}</text>"#,
                mid + 3.5
            );
            continue;
        };
        let e = &entries[k];
        let c = ink(e.ink, dark, weak);
        let sw = Rect::from_min_size(Pos2::new(rx, mid - 6.5), vec2(LEG_SWATCH, 13.0));
        match e.swatch {
            Swatch::Card | Swatch::Gone | Swatch::Folded => {
                card_body(s, sw, 3.0, card, line, &c, e.swatch == Swatch::Gone);
                if e.swatch == Swatch::Folded {
                    let chip = Rect::from_min_max(
                        Pos2::new(sw.min.x + 6.0, sw.max.y - 6.0),
                        Pos2::new(sw.max.x - 5.0, sw.max.y - 2.0),
                    );
                    chip_pill(s, chip, &c);
                }
            }
            Swatch::Warn => warn_badge(s, sw.center().x, mid),
            Swatch::Pin => {
                let _ = writeln!(
                    s,
                    r#"<text x="{:.1}" y="{:.1}" font-size="11" text-anchor="middle">📌</text>"#,
                    sw.center().x,
                    mid + 4.0
                );
            }
            Swatch::Chip => {
                chip_pill(s, sw, &c);
                let _ = writeln!(
                    s,
                    r#"<text x="{:.1}" y="{:.1}" font-size="9" text-anchor="middle" fill="{c}">port</text>"#,
                    sw.center().x,
                    mid + 3.0
                );
            }
            Swatch::Line(link) => {
                // The entry's colour is the one its lines have (a box's,
                // for lines into one); the width and dashes are the kind's.
                let (_, width, dash) = link_style(link, &c, weak);
                let d = format!("M{:.1} {mid:.1} L{:.1} {mid:.1}", sw.min.x, sw.max.x);
                link_path(s, &d, &c, width, dash);
            }
            Swatch::Trunk(link) => {
                let (_, width, dash) = link_style(link, &c, weak);
                let end = sw.max.x - 12.0;
                let d = format!("M{:.1} {mid:.1} L{end:.1} {mid:.1}", sw.min.x);
                link_path(s, &d, &c, width + 0.6, dash);
                let _ = writeln!(
                    s,
                    r#"<text x="{:.1}" y="{:.1}" font-size="10" fill="{weak}">×N</text>"#,
                    end + 1.0,
                    mid + 3.5
                );
            }
            Swatch::Box => box_rect(s, sw, 3.0, &c),
            // Only on the canvas: an export has no selection.
            Swatch::Lit => {}
        }
        let count = e.count.map_or_else(String::new, |n| {
            format!(r#"  <tspan fill="{weak}">{n}</tspan>"#)
        });
        let _ = writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" font-size="11.5" fill="{text}">{}{count}</text>"#,
            sw.max.x + 8.0,
            mid + 4.0,
            esc(&e.label)
        );
    }
    s.push_str("</g>\n");
}

/// The map as drawn: `pos` is where each card is (None: not shown),
/// `moved` whether it was dragged by hand. Links, trunks, boxes and chips
/// follow the canvas; with `with_legend`, the legend goes under the map.
pub fn render(
    map: &Map,
    pos: &[Option<Pos2>],
    lay: &Layout,
    moved: &[bool],
    dark: bool,
    with_legend: bool,
) -> String {
    let boxes = &lay.boxes;
    let [bg, card, line, text, weak] = palette(dark);
    let mut b = Rect::NOTHING;
    for p in pos.iter().flatten() {
        b = b.union(Rect::from_min_size(*p, CARD));
    }
    for g in boxes.iter() {
        b = b.union(g.rect);
    }
    if !b.is_positive() {
        b = Rect::from_min_size(Pos2::ZERO, CARD);
    }
    let lines = layout::lines(map, lay, &|i| pos.get(i).copied().flatten(), &|i| {
        moved.get(i).copied().unwrap_or(false)
    });
    let (m, head) = (32.0, 64.0);
    let foot = 28.0 + 20.0 * map.findings.len().min(12) as f32;
    // In Sites the box already names the site.
    let sites = boxes.iter().any(|g| g.tree);
    let entries = if with_legend {
        legend::entries(map, boxes, &lines, sites, 0, false)
    } else {
        Vec::new()
    };
    let leg = (!entries.is_empty()).then(|| legend_plan(&entries, b.width()));
    let leg_h = leg.as_ref().map_or(0.0, |l| l.height + 16.0);
    let off = vec2(m - b.min.x, m + head - b.min.y);
    let w = b.width().max(leg.as_ref().map_or(0.0, |l| l.width)) + 2.0 * m;
    let h = b.height() + 2.0 * m + head + leg_h + foot;
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
        box_rect(&mut s, Rect::from_min_size(p, r.size()), 10.0, &colour);
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
        let (colour, width, dash) = link_style(link, &role(n.zone), weak);
        let into = line.into.and_then(|g| boxes.get(g));
        let colour = into.and_then(|g| g.zone).map_or(colour, role);
        let width = if into.is_some() { width + 0.6 } else { width };
        link_path(&mut s, &d, &colour, width, dash);
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

    for (i, n) in map.nodes.iter().enumerate() {
        let Some(p) = pos[i].map(at) else { continue };
        let accent = if n.gone {
            gone(dark).to_string()
        } else {
            role(n.zone)
        };
        let (x, y) = (p.x, p.y);
        s += "<g>";
        // A pinned card missing from the latest data: dashed, in red.
        card_body(
            &mut s,
            Rect::from_min_size(p, CARD),
            7.0,
            card,
            line,
            &accent,
            n.gone,
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
        // The uplink's port or note rides on the card, as on screen.
        let chips =
            n.up.as_ref()
                .and_then(|u| u.2.as_deref())
                .into_iter()
                .chain(n.chips.iter().map(String::as_str))
                .filter(|c| !(sites && n.site.as_deref() == Some(*c)));
        let mut cx = x + 12.0;
        for c in chips {
            let cw = 10.0 + 5.6 * c.chars().count() as f32;
            if cx + cw > x + CARD.x - 8.0 {
                break;
            }
            chip_pill(
                &mut s,
                Rect::from_min_size(Pos2::new(cx, y + CARD.y - 22.0), vec2(cw, 15.0)),
                &accent,
            );
            let _ = writeln!(
                s,
                r#"<text x="{:.1}" y="{:.1}" font-size="9.5" fill="{accent}">{}</text>"#,
                cx + 5.0,
                y + CARD.y - 11.0,
                esc(c)
            );
            cx += cw + 4.0;
        }
        if !n.flags.is_empty() {
            warn_badge(&mut s, x + CARD.x - 12.0, y + 12.0);
        }
        s += "</g>\n";
    }

    // The legend under the map, clear of everything on it.
    if let Some(plan) = &leg {
        draw_legend(
            &mut s,
            &entries,
            plan,
            m,
            m + head + b.height() + 16.0,
            dark,
        );
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
        let s = render(&m, &l.pos, &l, &vec![false; m.nodes.len()], true, false);
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
        let s = render(&m, &l.pos, &l, &vec![false; m.nodes.len()], true, false);
        assert!(s.contains(&format!(">×{}<", trunk.kids.len())), "{s}");
        // Dragged, a card leaves the trunk and gets its own line.
        let mut moved = vec![false; m.nodes.len()];
        moved[trunk.kids[0]] = true;
        let dragged = render(&m, &l.pos, &l, &moved, true, false);
        assert!(dragged.contains(&format!(">×{}<", trunk.kids.len() - 1)));
    }

    #[test]
    fn the_legend_goes_under_the_map_when_asked_for() {
        let f = crate::skills::EXAMPLES[0].1;
        let doc = crate::mapnetwork::parse(crate::skills::file(f).unwrap()).unwrap();
        let m = crate::map::import::build(&doc, &Input::default());
        for mode in crate::map::layout::Mode::ALL {
            let l = crate::map::layout::layout_in(&m, &|_| true, mode);
            let moved = vec![false; m.nodes.len()];
            let off = render(&m, &l.pos, &l, &moved, false, false);
            let on = render(&m, &l.pos, &l, &moved, false, true);
            assert!(!off.contains("class=\"legend\"") && !off.contains(">Legend<"));
            assert!(
                on.contains("class=\"legend\"") && on.contains(">Legend<"),
                "{mode:?}"
            );
            assert_eq!(on.matches("<g").count(), on.matches("</g>").count());
            // The same entries the canvas lists (bar pins and the lit path).
            let lines = layout::lines(&m, &l, &|i| l.pos[i], &|_| false);
            let sites = mode == crate::map::layout::Mode::Sites;
            for e in legend::entries(&m, &l.boxes, &lines, sites, 0, false) {
                assert!(on.contains(&esc(&e.label)), "{mode:?}: {}", e.label);
            }
            // Below the map: the picture grows, and nothing above moves.
            let height = |s: &str| -> f32 {
                let h = s.split("height=\"").nth(1).unwrap();
                h[..h.find('"').unwrap()].parse().unwrap()
            };
            assert!(height(&on) > height(&off) + 40.0);
            let cards = |s: &str| -> Vec<String> {
                s.lines()
                    .filter(|l| l.starts_with("<g><rect"))
                    .map(String::from)
                    .collect()
            };
            assert!(!cards(&off).is_empty());
            assert_eq!(cards(&on), cards(&off), "{mode:?}: the cards stay put");
        }
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
