//! The map saved for other tools: a picture (SVG, PNG), a diagram to keep
//! editing (draw.io, which also saves to Visio), text for a wiki or a
//! README (Mermaid, Graphviz DOT) and an inventory for a spreadsheet (CSV).
//! Each holds what's on screen: the cards shown, collapsed sites as one
//! card, and dragged cards where they were left.

mod csv;
mod dot;
mod drawio;
mod mermaid;
mod png;
mod scene;
mod svg;

pub use csv::vendor;
pub use scene::Look;

use super::layout::{self, CARD, Layout, Line};
use super::{Link, Map, Node, Zone};
use eframe::egui::{Pos2, Rect};

/// The map as drawn now.
pub struct Drawn<'a> {
    pub map: &'a Map,
    pub lay: &'a Layout,
    /// Where each card is (None: not shown), dragged ones where they were left.
    pub pos: &'a [Option<Pos2>],
    /// Dragged by hand: joined straight to its uplink, out of any trunk.
    pub moved: &'a [bool],
    /// The legend is shown on the canvas: the pictures (SVG, PNG) put it
    /// under the map.
    pub legend: bool,
    /// What a saved map says it is when it isn't what was found (a plan,
    /// a training map): said in the summary line, as above the canvas.
    pub caption: Option<&'a str>,
}

impl Drawn<'_> {
    /// Every card shown, with its top-left corner.
    fn cards(&self) -> impl Iterator<Item = (usize, &Node, Pos2)> {
        self.map
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| Some((i, n, self.at(i)?)))
    }

    fn at(&self, i: usize) -> Option<Pos2> {
        self.pos.get(i).copied().flatten()
    }

    /// Where a shown card is drawn: a full card, or a client's tile.
    fn rect(&self, i: usize) -> Option<Rect> {
        Some(layout::card_rect(&self.map.nodes[i], self.at(i)?))
    }

    /// The lines as the canvas draws them.
    fn lines(&self) -> Vec<Line> {
        let all = layout::lines(self.map, self.lay, &|i| self.at(i), &|i| {
            self.moved.get(i).copied().unwrap_or(false)
        });
        // The links the canvas's Links view shows, as it shows them.
        let view = self.lay.links;
        if view == layout::Links::All {
            return all;
        }
        let below = layout::has_below(self.map);
        all.into_iter()
            .filter(|l| view.keeps(self.map, l, &|k| below[k]))
            .collect()
    }

    /// A shown card's uplink, when that's shown too.
    fn up(&self, i: usize) -> Option<(usize, Link, Option<&str>)> {
        let (p, link, port) = self.map.nodes[i].up.as_ref()?;
        self.at(*p)?;
        Some((*p, *link, port.as_deref()))
    }

    /// The box a shown card sits in.
    fn group(&self, i: usize) -> Option<usize> {
        self.lay.group.get(i).copied().flatten()
    }

    /// What's drawn: every card and box.
    fn bounds(&self) -> Rect {
        let mut b = Rect::NOTHING;
        for (_, n, p) in self.cards() {
            b = b.union(layout::card_rect(n, p));
        }
        for g in &self.lay.boxes {
            b = b.union(g.rect);
        }
        if b.is_positive() {
            b
        } else {
            Rect::from_min_size(Pos2::ZERO, CARD)
        }
    }

    /// The map's own title (a /mapnetwork map's), else "Network map".
    fn title(&self) -> &str {
        match self.map.title.trim() {
            "" => "Network map",
            t => t,
        }
    }

    /// The line under the title: the map's summary and the version.
    fn summary(&self) -> String {
        format!("{} · is-alive {}", self.map.summary, crate::VERSION)
    }

    /// How many cards a box holds, as on screen: a collapsed site says how
    /// many are inside it.
    fn box_count(&self, g: &layout::Group) -> String {
        match self.map.closed.get(&g.title).filter(|_| g.tree) {
            Some(inside) => format!("{} inside", inside.len()),
            None => g.count.to_string(),
        }
    }

    /// A box's title line: "HQ · 12".
    fn box_label(&self, g: &layout::Group) -> String {
        format!("{} · {}", g.title, self.box_count(g))
    }

    /// The findings that name card `i`, worst first.
    fn findings_of(&self, i: usize) -> Vec<String> {
        self.map
            .findings
            .iter()
            .filter(|f| f.nodes.contains(&i))
            .map(|f| format!("{}: {}", f.sev.label(), f.text))
            .collect()
    }
}

/// What the Export menu offers.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Format {
    Svg,
    /// At 1× or 2×, on the theme's background or on white.
    Png {
        scale: u8,
        white: bool,
    },
    DrawIo,
    Mermaid,
    Dot,
    Csv,
}

impl Format {
    /// The menu's entries, PNG standing for its sizes.
    pub const MENU: [Format; 6] = [
        Format::Svg,
        Format::Png {
            scale: 1,
            white: false,
        },
        Format::DrawIo,
        Format::Mermaid,
        Format::Dot,
        Format::Csv,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Format::Svg => "SVG image",
            Format::Png { .. } => "PNG image",
            Format::DrawIo => "draw.io diagram",
            Format::Mermaid => "Mermaid flowchart",
            Format::Dot => "Graphviz DOT",
            Format::Csv => "CSV inventory",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Format::Svg => {
                "A picture that stays sharp at any size, for a ticket or a site document"
            }
            Format::Png { .. } => {
                "A picture for chat, email or a slide: at 1× or 2× (sharper), on the theme's \
                 background or on white"
            }
            Format::DrawIo => {
                "Opens in diagrams.net (draw.io) to keep editing: cards, links and site boxes \
                 as shapes, IP and MAC as data. diagrams.net also saves it as Visio (.vsdx)"
            }
            Format::Mermaid => {
                "A flowchart in text for Markdown: GitHub, GitLab, Azure DevOps, Notion and \
                 Obsidian draw it. Sites become subgraphs"
            }
            Format::Dot => {
                "For Graphviz: dot lays it out afresh, neato -n2 keeps the map's own layout"
            }
            Format::Csv => {
                "One row per card (name, role, IP, MAC, vendor, site, uplink, port, flags, \
                 findings) for a spreadsheet"
            }
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Svg => "svg",
            Format::Png { .. } => "png",
            Format::DrawIo => "drawio",
            Format::Mermaid => "mmd",
            Format::Dot => "dot",
            Format::Csv => "csv",
        }
    }

    /// The text formats; PNG is drawn by `Picture::png`.
    pub fn text(self, d: &Drawn, look: Look) -> Option<String> {
        Some(match self {
            Format::Svg => svg::write(&scene::draw(d, look)),
            Format::Png { .. } => return None,
            Format::DrawIo => drawio::write(d),
            Format::Mermaid => mermaid::write(d),
            Format::Dot => dot::write(d),
            Format::Csv => csv::write(d),
        })
    }

    /// A caution to show as `text` is saved: a Mermaid chart too big for
    /// the viewers the hint names.
    pub fn warning(self, text: &str) -> Option<&'static str> {
        (self == Format::Mermaid && mermaid::too_big(text)).then_some(
            "This map is too big for most Mermaid viewers (GitHub's among them): \
             draw.io or Graphviz DOT handle it, or set Clients to Off",
        )
    }
}

/// The map drawn, ready to turn into pixels (off the UI thread: a big map
/// at 2× takes a moment).
pub struct Picture(scene::Scene);

impl Picture {
    pub fn new(d: &Drawn, look: Look) -> Self {
        Picture(scene::draw(d, look))
    }

    /// As a PNG at `scale` (made smaller if the image would be huge).
    pub fn png(&self, scale: f32) -> Result<Vec<u8>, String> {
        png::write(&self.0, scale)
    }
}

/// "Acme — Network" → "acme-network-map.svg"; no title, "is-alive-map.svg".
pub fn file_name(title: &str, ext: &str) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(60).collect();
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        format!("is-alive-map.{ext}")
    } else if slug.ends_with("map") {
        format!("{slug}.{ext}")
    } else {
        format!("{slug}-map.{ext}")
    }
}

/// A role's colour on a white page (draw.io, DOT, Mermaid), as `#rrggbb`.
fn ink(z: Zone) -> String {
    hex(z.rgb(false))
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Escape for XML text and attribute values. Characters XML can't hold
/// (controls, U+FFFE and U+FFFF) are dropped, a line break kept as one.
fn xml(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o += "&amp;",
            '<' => o += "&lt;",
            '>' => o += "&gt;",
            '"' => o += "&quot;",
            '\'' => o += "&apos;",
            '\n' => o += "&#10;",
            c if c.is_control() || matches!(c, '\u{FFFE}' | '\u{FFFF}') => {}
            c => o.push(c),
        }
    }
    o
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::map::layout::Mode;

    /// The built-in Acme example as the Map tab first shows it.
    pub fn acme() -> Map {
        let bytes = crate::skills::example(&crate::skills::example_path(0)).expect("example");
        let doc = crate::mapnetwork::parse(bytes).expect("parses");
        crate::map::build(&crate::map::Input {
            mapnetwork: Some(&doc),
            ..Default::default()
        })
    }

    /// A note as hostile as typing allows (a paste from anywhere): a
    /// formula, markup, quotes, line breaks, a backslash escape and a
    /// character no file should carry.
    pub const HOSTILE: &str = "=cmd|' /C calc'!A0\n<script>alert(\"x\")</script> & 'q'\u{FFFE}\\N";

    /// Two cards, the second with the hostile note on it.
    pub fn with_hostile<R>(f: impl FnOnce(&Drawn) -> R) -> R {
        let m = Map {
            nodes: vec![
                Node {
                    id: "a".into(),
                    label: "A".into(),
                    ..Default::default()
                },
                Node {
                    id: "b".into(),
                    label: "B".into(),
                    up: Some((0, Link::Wired, None)),
                    note: HOSTILE.into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let lay = layout::layout_in(&m, &|_| true, Mode::Cluster);
        f(&Drawn {
            map: &m,
            lay: &lay,
            pos: &lay.pos,
            moved: &[false, false],
            legend: true,
            caption: None,
        })
    }

    /// A hostile note stays text: SVG and draw.io still parse, with it
    /// whole (but for what XML can't hold) on hover.
    #[test]
    fn a_hostile_note_stays_text_in_svg_and_drawio() {
        with_hostile(|d| {
            let look = Look::new(&eframe::egui::Visuals::light(), true);
            let svg = Format::Svg.text(d, look).unwrap();
            let doc = parse_xml(&svg);
            let title = doc
                .descendants()
                .find(|n| n.has_tag_name("title"))
                .and_then(|n| n.text())
                .unwrap();
            assert_eq!(title, format!("B\n{}", HOSTILE.replace('\u{FFFE}', "")));
            assert!(!doc.descendants().any(|n| n.has_tag_name("script")));
            // The legend says what the mark is.
            assert!(svg.contains("Has a note"));
            let drawio = Format::DrawIo.text(d, look).unwrap();
            let doc = parse_xml(&drawio);
            let card = doc
                .descendants()
                .find(|n| n.attribute("note").is_some())
                .unwrap();
            assert_eq!(
                card.attribute("note"),
                Some(HOSTILE.replace('\u{FFFE}', "").as_str())
            );
            let tip = card.attribute("tooltip").unwrap();
            assert!(tip.contains("&lt;script&gt;") && !tip.contains("<script"));
        });
    }
    pub fn parse_xml(s: &str) -> roxmltree::Document<'_> {
        roxmltree::Document::parse(s).unwrap_or_else(|e| panic!("not XML: {e}"))
    }

    /// Runs `f` on Acme drawn in `mode`, with nothing dragged.
    pub fn with_acme<R>(mode: Mode, f: impl FnOnce(&Drawn) -> R) -> R {
        let m = acme();
        let lay = layout::layout_in(&m, &|_| true, mode);
        let moved = vec![false; m.nodes.len()];
        f(&Drawn {
            map: &m,
            lay: &lay,
            pos: &lay.pos,
            moved: &moved,
            legend: false,
            caption: None,
        })
    }

    #[test]
    fn file_names_follow_the_title() {
        assert_eq!(file_name("Acme — Network", "svg"), "acme-network-map.svg");
        assert_eq!(file_name("HQ map", "png"), "hq-map.png");
        assert_eq!(file_name("", "csv"), "is-alive-map.csv");
        assert_eq!(file_name("../..", "dot"), "is-alive-map.dot");
        assert_eq!(file_name("Café/Bar", "mmd"), "café-bar-map.mmd");
    }

    #[test]
    fn acme_has_a_title_and_cards() {
        with_acme(Mode::Sites, |d| {
            assert_eq!(d.title(), "Acme Distribution LLC — Multi-Site");
            assert!(d.cards().count() > 5);
        });
    }

    /// A plan or a training map's picture and diagram say what it is, so
    /// neither is taken for what was found; a map as found says nothing.
    #[test]
    fn a_plan_or_training_map_says_so_in_its_picture_and_diagram() {
        let look = Look::new(&eframe::egui::Visuals::dark(), false);
        let said = "Training map — made up for practice, not discovered";
        with_acme(Mode::Sites, |d| {
            for f in [Format::Svg, Format::DrawIo] {
                let text = f.text(d, look).expect("text");
                assert!(!text.contains("not discovered"), "{f:?}");
                let d = Drawn {
                    caption: Some(said),
                    ..*d
                };
                let text = f.text(&d, look).expect("text");
                assert!(text.contains(said), "{f:?}");
                assert!(text.contains(&d.summary()), "{f:?}");
                parse_xml(&text);
            }
        });
        // However narrow the map, the picture is wide enough for it.
        let m = crate::map::Map {
            nodes: vec![crate::map::Node {
                id: "internet".into(),
                zone: Zone::Internet,
                ..Default::default()
            }],
            ..Default::default()
        };
        let lay = layout::layout_in(&m, &|_| true, Mode::Cluster);
        let d = Drawn {
            map: &m,
            lay: &lay,
            pos: &lay.pos,
            moved: &[false],
            legend: false,
            caption: Some(said),
        };
        let w = scene::draw(&d, look).size.x;
        assert!(w >= 7.0 * said.chars().count() as f32 + 64.0, "{w}");
    }

    /// A card dragged out of its column: the pictures and diagrams take
    /// the canvas's routing, so its link goes round the cards in the way
    /// (client tiles at their own size) rather than through them.
    #[test]
    fn a_dragged_cards_link_misses_the_other_cards_in_exports_too() {
        let m = acme();
        let lay = layout::layout_in(&m, &|_| true, Mode::Cluster);
        let shown: Vec<usize> = (0..m.nodes.len())
            .filter(|&k| lay.pos[k].is_some())
            .collect();
        let bottom = shown
            .iter()
            .map(|&k| layout::card_rect(&m.nodes[k], lay.pos[k].unwrap()).bottom())
            .fold(f32::MIN, f32::max);
        let look = Look::new(&eframe::egui::Visuals::dark(), false);
        // Each stacked card in turn, dropped under the whole map at its
        // own x, and beside its column.
        let stacked: Vec<usize> = shown
            .iter()
            .copied()
            .filter(|&k| lay.stacked[k] && m.nodes[k].parent().is_some())
            .take(4)
            .collect();
        assert!(!stacked.is_empty());
        for k in stacked {
            let home = lay.pos[k].unwrap();
            for to in [
                eframe::egui::pos2(home.x, bottom + 120.0),
                home + eframe::egui::vec2(CARD.x + 40.0, 0.0),
            ] {
                let mut pos = lay.pos.clone();
                pos[k] = Some(to);
                let mut moved = vec![false; m.nodes.len()];
                moved[k] = true;
                let d = Drawn {
                    map: &m,
                    lay: &lay,
                    pos: &pos,
                    moved: &moved,
                    legend: false,
                    caption: None,
                };
                let own = d
                    .lines()
                    .into_iter()
                    .find(|l| l.kids == [k])
                    .expect("its link");
                let r = d.rect(k).unwrap();
                let end = *own.pts.last().unwrap();
                assert!(r.expand(0.01).contains(end), "{k}: {:?}", own.pts);
                // Only where the drop itself sits on a card can't it be clear.
                let on_card = shown
                    .iter()
                    .any(|&o| o != k && d.rect(o).unwrap().intersects(r));
                if !on_card {
                    let up = m.nodes[k].parent().unwrap();
                    for &o in shown.iter().filter(|&&o| o != k && o != up) {
                        let c = d.rect(o).unwrap().shrink(1.0);
                        let hit = own
                            .pts
                            .windows(2)
                            .any(|s| c.intersects(Rect::from_two_pos(s[0], s[1])));
                        assert!(!hit, "{k} to {to:?} runs through {o}: {:?}", own.pts);
                    }
                }
                for f in [Format::Svg, Format::DrawIo] {
                    parse_xml(&f.text(&d, look).expect("text"));
                }
            }
        }
    }

    #[test]
    fn xml_escapes_and_drops_controls() {
        assert_eq!(xml("a<b>&\"'\u{7}\nc"), "a&lt;b&gt;&amp;&quot;&apos;&#10;c");
    }

    /// A name XML can't hold, as a hostile hostname might carry: SVG and
    /// draw.io still open.
    #[test]
    fn svg_and_drawio_stay_xml_with_noncharacters() {
        assert_eq!(xml("a\u{FFFE}b\u{FFFF}c\u{1F5A5}"), "abc\u{1F5A5}");
        let m = crate::map::build(&crate::map::Input {
            sweep: vec![crate::map::Seen {
                ip: "10.0.0.9".parse().unwrap(),
                mac: None,
                name: Some("host\u{FFFE}x\u{FFFF}".into()),
                rtt: Some(1.0),
                open: vec![],
                checked: true,
            }],
            gateway: Some("10.0.0.1".parse().unwrap()),
            range: Some("10.0.0.0/24".into()),
            clients: crate::map::Clients::All,
            ..Default::default()
        });
        let lay = layout::layout_in(&m, &|_| true, Mode::Cluster);
        let moved = vec![false; m.nodes.len()];
        let d = Drawn {
            map: &m,
            lay: &lay,
            pos: &lay.pos,
            moved: &moved,
            legend: false,
            caption: None,
        };
        let look = Look::new(&eframe::egui::Visuals::dark(), false);
        for f in [Format::Svg, Format::DrawIo] {
            let text = f.text(&d, look).expect("text");
            assert!(text.contains("hostx"), "{f:?}");
            parse_xml(&text);
        }
    }
}
