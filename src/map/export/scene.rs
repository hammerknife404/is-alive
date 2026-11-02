//! The map as a picture: boxes, lines, cards, the legend and the findings
//! under them, as shapes and text runs that SVG and PNG both draw from, so
//! the two always match.

use super::Drawn;
use crate::map::layout::{self, CARD, Line};
use crate::map::legend::{self, Entry, Swatch};
use crate::map::{Link, Sev, Zone};
use eframe::egui::{Color32, Pos2, Rect, Vec2, Visuals, pos2, vec2};

pub type Rgb = [u8; 3];

/// The page: the theme's own colours, as the Map tab draws them, or white
/// whatever the theme.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    bg: Rgb,
    card: Rgb,
    line: Rgb,
    text: Rgb,
    weak: Rgb,
    dark: bool,
    white: bool,
}

impl Look {
    /// The page under the theme `vis` draws, or a white one.
    pub fn new(vis: &Visuals, white: bool) -> Self {
        // Miami's panels let the backdrop through; the page is solid.
        let bg = solid(vis.panel_fill);
        // A card as on the canvas, without its role's tint.
        let card = if vis.dark_mode && vis.panel_fill.a() == 255 {
            bg.lerp_to_gamma(Color32::WHITE, 0.06)
        } else {
            solid(vis.window_fill)
        };
        let on = |c: Color32| rgb(card.blend(c));
        Look {
            bg: rgb(bg),
            card: rgb(card),
            line: on(vis.widgets.noninteractive.bg_stroke.color),
            text: on(vis.strong_text_color()),
            weak: on(vis.weak_text_color()),
            dark: vis.dark_mode,
            white,
        }
    }
}

fn rgb(c: Color32) -> Rgb {
    [c.r(), c.g(), c.b()]
}

/// The colour without its see-through.
fn solid(c: Color32) -> Color32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    Color32::from_rgb(r, g, b)
}

pub struct Stroke {
    pub rgb: Rgb,
    pub alpha: f32,
    pub width: f32,
    /// Dash and gap lengths.
    pub dash: Option<[f32; 2]>,
}

/// A run of text in one colour and weight (400 regular, 600 or 700 bold).
pub struct Run {
    pub text: String,
    pub rgb: Rgb,
    pub weight: u16,
}

pub enum Item {
    Rect {
        r: Rect,
        round: f32,
        fill: Option<(Rgb, f32)>,
        stroke: Option<Stroke>,
    },
    Line {
        pts: Vec<Pos2>,
        stroke: Stroke,
    },
    Dot {
        c: Pos2,
        r: f32,
        rgb: Rgb,
    },
    /// `at` is the baseline's start, or its middle when `middle`.
    Text {
        at: Pos2,
        size: f32,
        mono: bool,
        middle: bool,
        runs: Vec<Run>,
    },
    /// One card's shapes (an SVG group).
    Card(Vec<Item>),
    /// The legend's shapes (an SVG group of class "legend").
    Legend(Vec<Item>),
}

pub struct Scene {
    pub size: Vec2,
    pub items: Vec<Item>,
}

/// The page's colours.
struct Ink {
    bg: Rgb,
    card: Rgb,
    line: Rgb,
    text: Rgb,
    weak: Rgb,
    gone: Rgb,
    /// Roles in the light palette: white under a dark theme's neons.
    light_roles: bool,
}

impl Ink {
    fn new(look: Look) -> Self {
        if look.white {
            return Ink {
                bg: [0xff; 3],
                card: [0xff; 3],
                line: [0xd2, 0xd6, 0xdc],
                text: [0x1d, 0x21, 0x28],
                weak: [0x69, 0x70, 0x7c],
                gone: [0xc8, 0x1e, 0x1e],
                light_roles: !crate::theme::current().is_light(),
            };
        }
        Ink {
            bg: look.bg,
            card: look.card,
            line: look.line,
            text: look.text,
            weak: look.weak,
            gone: if look.dark {
                [0xff, 0x40, 0x40]
            } else {
                [0xc8, 0x1e, 0x1e]
            },
            light_roles: false,
        }
    }

    /// A role's colour in the theme in use, as on screen.
    fn role(&self, z: Zone) -> Rgb {
        if self.light_roles {
            return z.rgb(false);
        }
        let c = z.colour_in(crate::theme::current());
        [c.r(), c.g(), c.b()]
    }
}

const AMBER: Rgb = [0xe8, 0xb8, 0x48];

fn run(text: impl Into<String>, rgb: Rgb, weight: u16) -> Vec<Run> {
    vec![Run {
        text: text.into(),
        rgb,
        weight,
    }]
}

fn text(at: Pos2, size: f32, mono: bool, runs: Vec<Run>) -> Item {
    Item::Text {
        at,
        size,
        mono,
        middle: false,
        runs,
    }
}

/// Split into at most `rows` lines of about `width` characters, with an
/// ellipsis when it doesn't fit.
pub fn wrap(s: &str, width: usize, rows: usize) -> Vec<String> {
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

/// The map as on screen, with a title line above and the findings below.
pub fn draw(d: &Drawn, look: Look) -> Scene {
    let ink = Ink::new(look);
    let map = d.map;
    let b = d.bounds();
    let (m, head) = (32.0, 64.0);
    let foot = 28.0 + 20.0 * map.findings.len().min(12) as f32;
    let drawn = d.lines();
    // In Sites the box already names the site.
    let sites = d.lay.boxes.iter().any(|g| g.tree);
    // The legend, when it's shown on the canvas, goes under the map.
    let entries = if d.legend {
        legend::entries(map, &d.lay.boxes, &drawn, sites, 0, false)
    } else {
        Vec::new()
    };
    let leg = (!entries.is_empty()).then(|| legend_plan(&entries, b.width()));
    let leg_h = leg.as_ref().map_or(0.0, |l| l.height + 16.0);
    let off = vec2(m - b.min.x, m + head - b.min.y);
    let size = vec2(
        b.width().max(leg.as_ref().map_or(0.0, |l| l.width)) + 2.0 * m,
        b.height() + 2.0 * m + head + leg_h + foot,
    );
    let at = |p: Pos2| p + off;
    let mut items = vec![Item::Rect {
        r: Rect::from_min_size(Pos2::ZERO, size),
        round: 0.0,
        fill: Some((ink.bg, 1.0)),
        stroke: None,
    }];
    items.push(text(
        pos2(m, m + 6.0),
        20.0,
        false,
        run(d.title(), ink.text, 600),
    ));
    items.push(text(
        pos2(m, m + 26.0),
        12.0,
        false,
        run(
            format!("{} · is-alive {}", map.summary, crate::VERSION),
            ink.weak,
            400,
        ),
    ));
    boxes(d, &ink, &at, &mut items);
    lines(d, &drawn, &ink, &at, &mut items);
    for (_, n, p) in d.cards() {
        items.push(Item::Card(card(n, at(p), sites, &ink)));
    }
    // The legend under the map, clear of everything on it.
    if let Some(plan) = &leg {
        let top = pos2(m, m + head + b.height() + 16.0);
        items.push(Item::Legend(draw_legend(&entries, plan, top, &ink)));
    }
    findings(d, &ink, m, size.y - foot + 8.0, &mut items);
    Scene { size, items }
}

/// Zones and Sites boxes, under the links and cards.
fn boxes(d: &Drawn, ink: &Ink, at: &dyn Fn(Pos2) -> Pos2, items: &mut Vec<Item>) {
    for g in &d.lay.boxes {
        let r = Rect::from_min_size(at(g.rect.min), g.rect.size());
        let colour = g.zone.map_or(ink.weak, |z| ink.role(z));
        items.push(box_rect(r, 10.0, colour));
        items.push(text(
            pos2(
                r.min.x + layout::BOX_PAD,
                r.min.y + layout::BOX_TITLE / 2.0 + 4.0,
            ),
            11.0,
            false,
            run(
                format!("{} · {}", g.title.to_uppercase(), d.box_count(g)),
                colour,
                600,
            ),
        ));
    }
}

/// Links and trunks, styled by the link they carry.
fn lines(d: &Drawn, drawn: &[Line], ink: &Ink, at: &dyn Fn(Pos2) -> Pos2, items: &mut Vec<Item>) {
    let map = d.map;
    for line in drawn.iter().filter(|l| !l.quiet) {
        let pts: Vec<Pos2> = line.pts.iter().map(|p| at(*p)).collect();
        // A trunk carries its strongest link, in its box's colour.
        let n = line
            .kids
            .iter()
            .map(|&k| &map.nodes[k])
            .max_by_key(|m| m.link().map_or(0, layout::strength))
            .expect("a line carries a link");
        let (rgb, width, dash) = link_style(n.link().unwrap_or(Link::Wired), ink.role(n.zone), ink);
        let into = line.into.and_then(|g| d.lay.boxes.get(g));
        let rgb = into.and_then(|g| g.zone).map_or(rgb, |z| ink.role(z));
        let width = if into.is_some() { width + 0.6 } else { width };
        let end = *pts.last().expect("route has points");
        items.push(link_line(pts, rgb, width, dash));
        if into.is_some() && line.kids.len() > 1 {
            items.push(text(
                pos2(end.x + 5.0, end.y - 4.0),
                10.0,
                false,
                run(format!("×{}", line.kids.len()), ink.weak, 400),
            ));
        }
    }
}

/// A card: its role's bar, kind, name, address, chips and a flag mark.
fn card(n: &crate::map::Node, p: Pos2, sites: bool, ink: &Ink) -> Vec<Item> {
    let accent = if n.gone { ink.gone } else { ink.role(n.zone) };
    let (x, y) = (p.x, p.y);
    // A pinned card missing from the latest data: dashed, in red.
    let mut v = card_body(Rect::from_min_size(p, CARD), 7.0, accent, n.gone, ink);
    v.push(text(
        pos2(x + 12.0, y + 17.0),
        9.5,
        true,
        run(&n.kind, ink.weak, 400),
    ));
    let label = wrap(&n.label, 26, 2);
    for (k, l) in label.iter().enumerate() {
        v.push(text(
            pos2(x + 12.0, y + 34.0 + k as f32 * 16.0),
            13.0,
            false,
            run(l, ink.text, 600),
        ));
    }
    if !n.sub.is_empty() {
        let rows = label.len().max(1);
        v.push(text(
            pos2(x + 12.0, y + 34.0 + rows as f32 * 16.0),
            10.5,
            true,
            run(&n.sub, accent, 400),
        ));
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
        v.push(chip_pill(
            Rect::from_min_size(pos2(cx, y + CARD.y - 22.0), vec2(cw, 15.0)),
            accent,
        ));
        v.push(text(
            pos2(cx + 5.0, y + CARD.y - 11.0),
            9.5,
            false,
            run(c, accent, 400),
        ));
        cx += cw + 4.0;
    }
    if !n.flags.is_empty() {
        v.extend(warn_badge(pos2(x + CARD.x - 12.0, y + 12.0)));
    }
    v
}

// The pieces the map is drawn with, shared with the legend so its swatches
// are the picture's own drawing.

/// How a link is drawn: (colour, width, dashes). `accent` is its card's
/// colour.
fn link_style(link: Link, accent: Rgb, ink: &Ink) -> (Rgb, f32, Option<[f32; 2]>) {
    match link {
        Link::Backbone => (ink.role(Zone::Gateway), 3.0, None),
        Link::Wired => (accent, 1.6, None),
        Link::Wireless => (accent, 1.6, Some([7.0, 5.0])),
        Link::Assumed => (ink.weak, 1.4, Some([2.5, 4.0])),
    }
}

/// A line along `pts` in a link's style.
fn link_line(pts: Vec<Pos2>, rgb: Rgb, width: f32, dash: Option<[f32; 2]>) -> Item {
    Item::Line {
        pts,
        stroke: Stroke {
            rgb,
            alpha: 0.75,
            width,
            dash,
        },
    }
}

/// A card's body: fill, edge (dashed in `accent` when `gone`) and colour bar.
fn card_body(r: Rect, round: f32, accent: Rgb, gone: bool, ink: &Ink) -> Vec<Item> {
    vec![
        Item::Rect {
            r,
            round,
            fill: Some((ink.card, 1.0)),
            stroke: Some(Stroke {
                rgb: if gone { accent } else { ink.line },
                alpha: 1.0,
                width: 1.0,
                dash: gone.then_some([6.0, 4.0]),
            }),
        },
        Item::Rect {
            r: Rect::from_min_size(r.min, vec2(4.0, r.height())),
            round: 2.0,
            fill: Some((accent, 1.0)),
            stroke: None,
        },
    ]
}

/// A chip's pill (its text goes over it).
fn chip_pill(r: Rect, accent: Rgb) -> Item {
    Item::Rect {
        r,
        round: r.height() / 2.0,
        fill: Some((accent, 0.16)),
        stroke: None,
    }
}

/// The warning badge centred at `c`.
fn warn_badge(c: Pos2) -> [Item; 2] {
    [
        Item::Dot {
            c,
            r: 7.0,
            rgb: AMBER,
        },
        Item::Text {
            at: pos2(c.x, c.y + 4.0),
            size: 11.0,
            mono: false,
            middle: true,
            runs: run("!", [0; 3], 700),
        },
    ]
}

/// A site's or a role's box.
fn box_rect(r: Rect, round: f32, colour: Rgb) -> Item {
    Item::Rect {
        r,
        round,
        fill: Some((colour, 0.07)),
        stroke: Some(Stroke {
            rgb: colour,
            alpha: 0.5,
            width: 1.0,
            dash: None,
        }),
    }
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

/// A legend colour as the picture draws the map.
fn legend_ink(i: legend::Ink, ink: &Ink) -> Rgb {
    match i {
        legend::Ink::Role(z) => ink.role(z),
        legend::Ink::Gone => ink.gone,
        legend::Ink::Weak => ink.weak,
        legend::Ink::Warn => AMBER,
    }
}

/// The legend with its top-left at `top`, each swatch drawn as the picture
/// draws the map.
fn draw_legend(entries: &[Entry], plan: &LegendPlan, top: Pos2, ink: &Ink) -> Vec<Item> {
    let mut v = vec![text(
        pos2(top.x, top.y + 12.0),
        13.0,
        false,
        run("Legend", ink.text, 600),
    )];
    for slot in &plan.slots {
        let rx = top.x + slot.col as f32 * plan.col;
        let mid = top.y + 24.0 + slot.row as f32 * LEG_ROW + LEG_ROW / 2.0;
        let Some(k) = slot.entry else {
            let title = slot.head.map_or("", |h| h.title());
            v.push(text(
                pos2(rx, mid + 3.5),
                9.5,
                false,
                run(title, ink.weak, 600),
            ));
            continue;
        };
        let e = &entries[k];
        let c = legend_ink(e.ink, ink);
        let sw = Rect::from_min_size(pos2(rx, mid - 6.5), vec2(LEG_SWATCH, 13.0));
        match e.swatch {
            Swatch::Card | Swatch::Gone | Swatch::Folded => {
                v.extend(card_body(sw, 3.0, c, e.swatch == Swatch::Gone, ink));
                if e.swatch == Swatch::Folded {
                    let chip = Rect::from_min_max(
                        pos2(sw.min.x + 6.0, sw.max.y - 6.0),
                        pos2(sw.max.x - 5.0, sw.max.y - 2.0),
                    );
                    v.push(chip_pill(chip, c));
                }
            }
            Swatch::Warn => v.extend(warn_badge(pos2(sw.center().x, mid))),
            Swatch::Pin => v.push(Item::Text {
                at: pos2(sw.center().x, mid + 4.0),
                size: 11.0,
                mono: false,
                middle: true,
                runs: run("📌", c, 400),
            }),
            Swatch::Chip => {
                v.push(chip_pill(sw, c));
                v.push(Item::Text {
                    at: pos2(sw.center().x, mid + 3.0),
                    size: 9.0,
                    mono: false,
                    middle: true,
                    runs: run("port", c, 400),
                });
            }
            Swatch::Line(link) => {
                // The entry's colour is the one its lines have (a box's,
                // for lines into one); the width and dashes are the kind's.
                let (_, width, dash) = link_style(link, c, ink);
                v.push(link_line(
                    vec![pos2(sw.min.x, mid), pos2(sw.max.x, mid)],
                    c,
                    width,
                    dash,
                ));
            }
            Swatch::Trunk(link) => {
                let (_, width, dash) = link_style(link, c, ink);
                let end = sw.max.x - 12.0;
                v.push(link_line(
                    vec![pos2(sw.min.x, mid), pos2(end, mid)],
                    c,
                    width + 0.6,
                    dash,
                ));
                v.push(text(
                    pos2(end + 1.0, mid + 3.5),
                    10.0,
                    false,
                    run("×N", ink.weak, 400),
                ));
            }
            Swatch::Box => v.push(box_rect(sw, 3.0, c)),
            // Only on the canvas: a picture has no selection.
            Swatch::Lit => {}
        }
        let mut runs = run(&e.label, ink.text, 400);
        if let Some(n) = e.count {
            runs.push(Run {
                text: format!("  {n}"),
                rgb: ink.weak,
                weight: 400,
            });
        }
        v.push(text(pos2(sw.max.x + 8.0, mid + 4.0), 11.5, false, runs));
    }
    v
}

/// The first dozen findings, worst first.
fn findings(d: &Drawn, ink: &Ink, m: f32, mut y: f32, items: &mut Vec<Item>) {
    if d.map.findings.is_empty() {
        return;
    }
    items.push(text(
        pos2(m, y),
        13.0,
        false,
        run("Findings", ink.text, 600),
    ));
    for f in d.map.findings.iter().take(12) {
        y += 20.0;
        let c = match f.sev {
            Sev::High => [0xeb, 0x5a, 0x5a],
            Sev::Med => [0xe6, 0xb4, 0x3c],
            Sev::Low => ink.weak,
        };
        items.push(text(
            pos2(m, y),
            11.5,
            false,
            vec![
                Run {
                    text: f.sev.label().to_uppercase(),
                    rgb: c,
                    weight: 600,
                },
                Run {
                    text: format!(" {}", f.text),
                    rgb: ink.text,
                    weight: 400,
                },
            ],
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn white_is_white_whatever_the_theme() {
        let ink = Ink::new(Look::new(&Visuals::dark(), true));
        assert_eq!(ink.bg, [0xff; 3]);
        assert_eq!(ink.text, [0x1d, 0x21, 0x28]);
    }

    /// Blueprint's drafting blue, not a stock dark grey; cards and text
    /// from the same theme.
    #[test]
    fn the_theme_background_is_the_themes_own() {
        let mut vis = Visuals::dark();
        vis.panel_fill = Color32::from_rgb(0x0c, 0x24, 0x43);
        vis.override_text_color = Some(Color32::from_rgb(0xdf, 0xea, 0xfa));
        vis.weak_text_color = Some(Color32::from_rgb(0x8f, 0xb0, 0xd6));
        let ink = Ink::new(Look::new(&vis, false));
        assert_eq!(ink.bg, [0x0c, 0x24, 0x43]);
        assert_eq!(ink.weak, [0x8f, 0xb0, 0xd6]);
        assert!(ink.card[2] > ink.card[0], "a blue card: {:?}", ink.card);
        // A see-through panel (Miami) still gives a solid page.
        vis.panel_fill = Color32::from_rgba_unmultiplied(22, 14, 44, 184);
        assert_eq!(Ink::new(Look::new(&vis, false)).bg, [22, 14, 44]);
        // A light theme's page is light, its text dark.
        let ink = Ink::new(Look::new(&Visuals::light(), false));
        assert!(ink.bg[0] > 200 && ink.text[0] < 80);
    }
}
