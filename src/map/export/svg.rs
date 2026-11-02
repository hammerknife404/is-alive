//! The map as a standalone SVG: same layout, colours and cards, a title
//! line and the findings underneath, for a ticket or a site doc.

use super::scene::{Item, Run, Scene, Stroke};
use super::{hex, xml};
use std::fmt::Write;

const SANS: &str = "Segoe UI, Helvetica, Arial, sans-serif";
const MONO: &str = "Consolas, Menlo, monospace";

pub fn write(scene: &Scene) -> String {
    let (w, h) = (scene.size.x, scene.size.y);
    let mut s = String::new();
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}" font-family="{SANS}">"#
    );
    for item in &scene.items {
        item_to(&mut s, item);
    }
    s += "</svg>\n";
    s
}

/// ` fill-opacity="0.5"`, or nothing when opaque.
fn opacity(name: &str, a: f32) -> String {
    if a < 1.0 {
        format!(r#" {name}-opacity="{a}""#)
    } else {
        String::new()
    }
}

fn stroke(st: &Stroke) -> String {
    let mut s = format!(r#" stroke="{}""#, hex(st.rgb));
    s += &opacity("stroke", st.alpha);
    if st.width != 1.0 {
        s += &format!(r#" stroke-width="{}""#, st.width);
    }
    if let Some([a, b]) = st.dash {
        s += &format!(r#" stroke-dasharray="{a} {b}""#);
    }
    s
}

fn weight(w: u16) -> String {
    if w > 400 {
        format!(r#" font-weight="{w}""#)
    } else {
        String::new()
    }
}

fn item_to(s: &mut String, item: &Item) {
    match item {
        Item::Rect {
            r,
            round,
            fill,
            stroke: st,
        } => {
            let _ = write!(
                s,
                r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}""#,
                r.min.x,
                r.min.y,
                r.width(),
                r.height()
            );
            if *round > 0.0 {
                let _ = write!(s, r#" rx="{round}""#);
            }
            match fill {
                Some((c, a)) => *s += &format!(r#" fill="{}"{}"#, hex(*c), opacity("fill", *a)),
                None => *s += r#" fill="none""#,
            }
            if let Some(st) = st {
                *s += &stroke(st);
            }
            *s += "/>\n";
        }
        Item::Line { pts, stroke: st } => {
            let d: String = pts
                .iter()
                .enumerate()
                .map(|(k, p)| format!("{}{:.1} {:.1}", if k == 0 { "M" } else { " L" }, p.x, p.y))
                .collect();
            let _ = writeln!(s, r#"<path d="{d}" fill="none"{}/>"#, stroke(st));
        }
        Item::Dot { c, r, rgb } => {
            let _ = writeln!(
                s,
                r#"<circle cx="{:.1}" cy="{:.1}" r="{r}" fill="{}"/>"#,
                c.x,
                c.y,
                hex(*rgb)
            );
        }
        Item::Text {
            at,
            size,
            mono,
            middle,
            runs,
        } => {
            let _ = write!(
                s,
                r#"<text x="{:.1}" y="{:.1}" font-size="{size}""#,
                at.x, at.y
            );
            if *mono {
                let _ = write!(s, r#" font-family="{MONO}""#);
            }
            if *middle {
                *s += r#" text-anchor="middle""#;
            }
            // One run: its colour and weight on the text itself.
            if let [
                Run {
                    text,
                    rgb,
                    weight: w,
                },
            ] = runs.as_slice()
            {
                let _ = writeln!(
                    s,
                    r#" fill="{}"{}>{}</text>"#,
                    hex(*rgb),
                    weight(*w),
                    xml(text)
                );
                return;
            }
            *s += ">";
            for r in runs {
                let _ = write!(
                    s,
                    r#"<tspan fill="{}"{}>{}</tspan>"#,
                    hex(r.rgb),
                    weight(r.weight),
                    xml(&r.text)
                );
            }
            *s += "</text>\n";
        }
        Item::Card(items) => {
            *s += "<g>\n";
            for i in items {
                item_to(s, i);
            }
            *s += "</g>\n";
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Drawn, Look, scene};
    use super::*;
    use crate::map::layout::{self, Mode};
    use crate::map::{Input, Link, Map, Seen, Zone, build};

    fn render(m: &Map, lay: &layout::Layout, moved: &[bool]) -> String {
        write(&scene::draw(
            &Drawn {
                map: m,
                lay,
                pos: &lay.pos,
                moved,
            },
            Look::new(&eframe::egui::Visuals::dark(), false),
        ))
    }

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
        let l = layout::layout_in(&m, &|_| true, Mode::Cluster);
        let s = render(&m, &l, &vec![false; m.nodes.len()]);
        assert!(s.starts_with("<svg") && s.trim_end().ends_with("</svg>"));
        assert!(s.contains("a&lt;b&gt;&amp;c"));
        assert!(!s.contains("a<b>"));
        assert!(s.contains("Telnet"));
        assert_eq!(s.matches("<g>").count(), s.matches("</g>").count());
        super::super::tests::parse_xml(&s);
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
        let l = layout::layout_in(&m, &|_| true, Mode::Zones);
        let lines = layout::lines(&m, &l, &|i| l.pos[i], &|_| false);
        let trunk = lines
            .iter()
            .find(|x| x.kids.len() > 1)
            .expect("hosts share a trunk into their box");
        let s = render(&m, &l, &vec![false; m.nodes.len()]);
        assert!(s.contains(&format!(">×{}<", trunk.kids.len())), "{s}");
        // Dragged, a card leaves the trunk and gets its own line.
        let mut moved = vec![false; m.nodes.len()];
        moved[trunk.kids[0]] = true;
        let dragged = render(&m, &l, &moved);
        assert!(dragged.contains(&format!(">×{}<", trunk.kids.len() - 1)));
    }

    #[test]
    fn acme_svg_parses_with_a_group_per_card() {
        super::super::tests::with_acme(Mode::Sites, |d| {
            let s = write(&scene::draw(
                d,
                Look::new(&eframe::egui::Visuals::dark(), false),
            ));
            super::super::tests::parse_xml(&s);
            assert_eq!(s.matches("<g>").count(), d.cards().count());
            assert!(s.contains("Acme Distribution LLC — Multi-Site"));
        });
    }
}
