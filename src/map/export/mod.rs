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

    /// The lines as the canvas draws them.
    fn lines(&self) -> Vec<Line> {
        layout::lines(self.map, self.lay, &|i| self.at(i), &|i| {
            self.moved.get(i).copied().unwrap_or(false)
        })
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
        for (_, _, p) in self.cards() {
            b = b.union(Rect::from_min_size(p, CARD));
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

/// Escape for XML text and attribute values. Control characters XML can't
/// hold are dropped, a line break kept as one.
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
            c if c.is_control() => {}
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

    /// Parses as XML, or fails the test with where it doesn't.
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

    #[test]
    fn xml_escapes_and_drops_controls() {
        assert_eq!(xml("a<b>&\"'\u{7}\nc"), "a&lt;b&gt;&amp;&quot;&apos;&#10;c");
    }
}
