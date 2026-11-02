//! The map as a draw.io diagram (diagrams.net): a shape per card with its
//! IP, MAC and the rest as data, site and role boxes as containers holding
//! their cards, and links with the canvas's own bends, so it opens looking
//! like the map and can be edited from there. diagrams.net also saves it
//! as Visio (.vsdx), PDF and the rest.

use super::{Drawn, ink, xml};
use crate::map::layout::{self, Line};
use crate::map::{Link, Node, Zone};
use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};
use std::fmt::Write;

const TEXT: &str = "#1d2128";
const WEAK: &str = "#69707c";
const GONE: &str = "#c81e1e";
const FLAG: &str = "#a06a00";

/// Escape for HTML inside a label.
fn html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn write(d: &Drawn) -> String {
    let b = d.bounds();
    // Room above for the title, as in the picture.
    let off = vec2(40.0 - b.min.x, 110.0 - b.min.y);
    let (w, h) = (b.width() + 80.0, b.height() + 150.0);
    let mut s = String::new();
    let _ = writeln!(
        s,
        r#"<mxfile host="is-alive" agent="is-alive {}" type="device">"#,
        crate::VERSION
    );
    let _ = writeln!(
        s,
        r#"<diagram id="is-alive-map" name="{}">"#,
        xml(d.title())
    );
    let _ = writeln!(
        s,
        r#"<mxGraphModel grid="1" gridSize="10" guides="1" tooltips="1" connect="1" arrows="1" fold="1" page="1" pageScale="1" pageWidth="{w:.0}" pageHeight="{h:.0}" math="0" shadow="0">"#
    );
    s += "<root>\n<mxCell id=\"0\"/>\n<mxCell id=\"1\" parent=\"0\"/>\n";
    let text = |s: &mut String, id: &str, value: &str, style: &str, r: Rect| {
        let _ = writeln!(
            s,
            r#"<mxCell id="{id}" value="{}" style="{style}" vertex="1" parent="1">{}</mxCell>"#,
            xml(value),
            geometry(r)
        );
    };
    text(
        &mut s,
        "title",
        d.title(),
        &format!(
            "text;html=0;fontSize=20;fontStyle=1;align=left;verticalAlign=top;fontColor={TEXT};"
        ),
        Rect::from_min_size(pos2(40.0, 24.0), vec2(w - 80.0, 30.0)),
    );
    text(
        &mut s,
        "summary",
        &match d.caption {
            // A plan or a training map says so first.
            Some(c) => format!("{c} · {}", d.summary()),
            None => d.summary(),
        },
        &format!(
            "text;html=0;fontSize=11;align=left;verticalAlign=top;fontColor={WEAK};whiteSpace=wrap;"
        ),
        Rect::from_min_size(pos2(40.0, 56.0), vec2(w - 80.0, 30.0)),
    );
    for (k, g) in d.lay.boxes.iter().enumerate() {
        let c = g.zone.map_or(WEAK.to_string(), ink);
        let _ = writeln!(
            s,
            r#"<mxCell id="box{k}" value="{}" style="rounded=1;arcSize=3;absoluteArcSize=1;html=0;container=1;collapsible=0;recursiveResize=0;fillColor={c};fillOpacity=7;strokeColor={c};strokeOpacity=50;fontColor={c};fontStyle=1;fontSize=11;align=left;verticalAlign=top;spacingLeft=12;spacingTop=4;" vertex="1" parent="1">{}</mxCell>"#,
            xml(&format!("{} · {}", g.title.to_uppercase(), d.box_count(g))),
            geometry(g.rect.translate(off))
        );
    }
    let sites = d.lay.boxes.iter().any(|g| g.tree);
    for (i, n, p) in d.cards() {
        // In its box when it sits inside it (a card dragged out doesn't).
        let card = layout::card_rect(n, p);
        let (parent, rel) = match d
            .group(i)
            .filter(|&g| d.lay.boxes[g].rect.contains_rect(card))
        {
            Some(g) => (
                format!("box{g}"),
                card.translate(-d.lay.boxes[g].rect.min.to_vec2()),
            ),
            None => ("1".to_string(), card.translate(off)),
        };
        let _ = writeln!(
            s,
            r#"<UserObject id="n{i}" label="{}" placeholders="0"{}><mxCell style="{}" vertex="1" parent="{parent}">{}</mxCell></UserObject>"#,
            xml(&label(n, sites)),
            data(d, i, n),
            style(n),
            geometry(rel)
        );
    }
    for (k, line) in d.lines().iter().filter(|l| !l.quiet).enumerate() {
        edge(&mut s, d, k, line, off);
    }
    if !d.map.findings.is_empty() {
        let mut v = String::from("<b>Findings</b>");
        for f in &d.map.findings {
            let _ = write!(
                v,
                "<br><b>{}</b> {}",
                f.sev.label().to_uppercase(),
                html(&f.text)
            );
        }
        let rows = d.map.findings.len() as f32 + 1.0;
        text(
            &mut s,
            "findings",
            &v,
            &format!(
                "text;html=1;fontSize=11;align=left;verticalAlign=top;whiteSpace=wrap;fontColor={TEXT};"
            ),
            Rect::from_min_size(
                pos2(40.0, b.max.y + off.y + 30.0),
                vec2(w - 80.0, 16.0 * rows),
            ),
        );
    }
    s += "</root>\n</mxGraphModel>\n</diagram>\n</mxfile>\n";
    s
}

fn geometry(r: Rect) -> String {
    format!(
        r#"<mxGeometry x="{:.0}" y="{:.0}" width="{:.0}" height="{:.0}" as="geometry"/>"#,
        r.min.x,
        r.min.y,
        r.width(),
        r.height()
    )
}

/// The card's face: kind, name, address, chips and flags, as on screen.
fn label(n: &Node, sites: bool) -> String {
    let accent = accent(n);
    let mut v = format!(
        r#"<div style="font-size:9px;font-family:monospace;color:{WEAK}">{}</div><b>{}</b>"#,
        html(&n.kind),
        html(&n.label)
    );
    if !n.sub.is_empty() {
        let _ = write!(
            v,
            r#"<div style="font-size:10px;font-family:monospace;color:{accent}">{}</div>"#,
            html(&n.sub)
        );
    }
    let chips: Vec<&str> = crate::map::drawn_chip(n)
        .into_iter()
        .chain(
            n.up.as_ref()
                .and_then(|u| u.2.as_deref())
                .into_iter()
                .chain(n.chips.iter().map(String::as_str))
                .filter(|c| !(sites && n.site.as_deref() == Some(*c))),
        )
        .collect();
    if !chips.is_empty() {
        let _ = write!(
            v,
            r#"<div style="font-size:9px;color:{accent}">{}</div>"#,
            html(&chips.join(" · "))
        );
    }
    if !n.flags.is_empty() {
        let _ = write!(
            v,
            r#"<div style="font-size:9px;color:{FLAG}">⚠ {}</div>"#,
            html(&n.flags.join("; "))
        );
    }
    v
}

fn accent(n: &Node) -> String {
    if n.gone {
        GONE.to_string()
    } else {
        ink(n.zone)
    }
}

/// A card's shape: rounded, outlined in its role's colour; dashed red
/// when it's pinned but gone from the data.
fn style(n: &Node) -> String {
    let mut s = format!(
        "rounded=1;arcSize=8;absoluteArcSize=1;whiteSpace=wrap;html=1;align=left;verticalAlign=top;spacingLeft=10;spacingTop=2;fillColor=#ffffff;strokeColor={};strokeWidth=1.5;fontColor={TEXT};fontSize=12;",
        accent(n)
    );
    if n.gone {
        s += "dashed=1;";
    }
    s
}

/// What diagrams.net shows under Edit Data, and searches.
fn data(d: &Drawn, i: usize, n: &Node) -> String {
    let mut fields = vec![
        ("role", n.zone.name().to_string()),
        ("kind", n.kind.clone()),
    ];
    if let Some(ip) = n.ip {
        fields.push(("ip", ip.to_string()));
    }
    if let Some(m) = &n.mac {
        fields.push(("mac", crate::net::mac::fmt(m)));
    }
    if let Some(v) = super::csv::vendor(n) {
        fields.push(("vendor", v));
    }
    if let Some(site) = &n.site {
        fields.push(("site", site.clone()));
    }
    if !n.flags.is_empty() {
        fields.push(("flags", n.flags.join("; ")));
    }
    let f = d.findings_of(i);
    if !f.is_empty() {
        fields.push(("findings", f.join("; ")));
    }
    fields
        .iter()
        .map(|(k, v)| format!(r#" {k}="{}""#, xml(v)))
        .collect()
}

/// Where `p` sits on `r`'s outline, as draw.io's 0–1 fractions.
fn anchor(r: Rect, p: Pos2) -> (f32, f32) {
    let f = |v: f32, lo: f32, len: f32| ((v - lo) / len).clamp(0.0, 1.0);
    (f(p.x, r.min.x, r.width()), f(p.y, r.min.y, r.height()))
}

/// A link or trunk: from its uplink to the card (or the box a trunk runs
/// into), through the canvas's own bends.
fn edge(s: &mut String, d: &Drawn, k: usize, line: &Line, off: Vec2) {
    let (Some(first), Some(last)) = (line.pts.first(), line.pts.last()) else {
        return;
    };
    let Some(up) = d.rect(line.up) else { return };
    let map = d.map;
    let n = line
        .kids
        .iter()
        .map(|&k| &map.nodes[k])
        .max_by_key(|m| m.link().map_or(0, crate::map::layout::strength))
        .expect("a line carries a link");
    let into = line.into.and_then(|g| d.lay.boxes.get(g).map(|b| (g, b)));
    let (target, to) = match into {
        Some((g, b)) => (format!("box{g}"), b.rect),
        None => {
            let kid = line.kids[0];
            let Some(r) = d.rect(kid) else { return };
            (format!("n{kid}"), r)
        }
    };
    let link = n.link().unwrap_or(Link::Wired);
    let (colour, width, dash) = match link {
        Link::Backbone => (ink(Zone::Gateway), 3.0, ""),
        Link::Wired => (ink(n.zone), 1.6, ""),
        Link::Wireless => (ink(n.zone), 1.6, "dashed=1;dashPattern=7 5;"),
        Link::Assumed => (WEAK.to_string(), 1.4, "dashed=1;dashPattern=2 4;"),
    };
    let colour = into.and_then(|(_, b)| b.zone).map_or(colour, ink);
    let width = if into.is_some() { width + 0.6 } else { width };
    let square = line
        .pts
        .windows(2)
        .all(|p| (p[0].x - p[1].x).abs() < 0.5 || (p[0].y - p[1].y).abs() < 0.5);
    let (ex, ey) = anchor(up, *first);
    let (nx, ny) = anchor(to, *last);
    let label = match into {
        Some(_) if line.kids.len() > 1 => format!("×{}", line.kids.len()),
        _ => String::new(),
    };
    let _ = write!(
        s,
        r#"<mxCell id="e{k}" value="{label}" style="{}endArrow=none;html=1;rounded=0;strokeColor={colour};strokeWidth={width};{dash}exitX={ex:.3};exitY={ey:.3};exitDx=0;exitDy=0;exitPerimeter=0;entryX={nx:.3};entryY={ny:.3};entryDx=0;entryDy=0;entryPerimeter=0;fontSize=10;fontColor={WEAK};labelBackgroundColor=#ffffff;" edge="1" parent="1" source="n{}" target="{target}">"#,
        if square {
            "edgeStyle=orthogonalEdgeStyle;"
        } else {
            ""
        },
        line.up
    );
    s.push_str(r#"<mxGeometry x="0.8" relative="1" as="geometry">"#);
    let bends = &line.pts[1..line.pts.len().saturating_sub(1).max(1)];
    if !bends.is_empty() {
        s.push_str(r#"<Array as="points">"#);
        for p in bends {
            let p = *p + off;
            let _ = write!(s, r#"<mxPoint x="{:.0}" y="{:.0}"/>"#, p.x, p.y);
        }
        s.push_str("</Array>");
    }
    s.push_str("</mxGeometry></mxCell>\n");
}

#[cfg(test)]
mod tests {
    use super::super::tests::{parse_xml, with_acme};
    use super::*;
    use crate::map::layout::Mode;
    use std::collections::HashSet;

    #[test]
    fn acme_opens_as_a_diagram_with_every_card_box_and_link() {
        for mode in [Mode::Sites, Mode::Zones, Mode::Cluster] {
            with_acme(mode, |d| {
                let s = write(d);
                let doc = parse_xml(&s);
                let root = doc.root_element();
                assert_eq!(root.tag_name().name(), "mxfile");
                let cards = doc
                    .descendants()
                    .filter(|e| e.has_tag_name("UserObject"))
                    .count();
                assert_eq!(cards, d.cards().count());
                let boxes = doc
                    .descendants()
                    .filter(|e| {
                        e.attribute("style")
                            .is_some_and(|s| s.contains("container=1"))
                    })
                    .count();
                assert_eq!(boxes, d.lay.boxes.len());
                let edges: Vec<_> = doc
                    .descendants()
                    .filter(|e| e.attribute("edge") == Some("1"))
                    .collect();
                assert_eq!(edges.len(), d.lines().iter().filter(|l| !l.quiet).count());
                // Ids unique; every parent, source and target is one of them.
                let ids: Vec<&str> = doc
                    .descendants()
                    .filter_map(|e| e.attribute("id"))
                    .collect();
                let set: HashSet<&str> = ids.iter().copied().collect();
                assert_eq!(set.len(), ids.len());
                for e in doc.descendants() {
                    for a in ["parent", "source", "target"] {
                        if let Some(v) = e.attribute(a) {
                            assert!(set.contains(v), "{a}={v}");
                        }
                    }
                }
                // Data for Edit Data.
                assert!(doc.descendants().any(|e| e.attribute("ip").is_some()));
            });
        }
    }

    #[test]
    fn labels_are_escaped_twice_html_inside_xml() {
        let n = Node {
            label: "<img src=x onerror=alert(1)> & co".into(),
            kind: "A\"B".into(),
            ..Default::default()
        };
        let l = label(&n, false);
        assert!(l.contains("&lt;img src=x onerror=alert(1)&gt; &amp; co"));
        let attr = xml(&l);
        assert!(!attr.contains('<') && !attr.contains('"'));
    }
}
