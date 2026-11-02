//! How a lit path is coloured: the colour each stretch of a lit line takes
//! from the cards it leads to, the packets flowing down it, and colours
//! kept legible on the canvas. The map view draws with these; its legend
//! and the exporters call the same rules, so a lit path looks alike in all.

use super::Link;
use super::layout::{self, Line, Piece, Step};
use eframe::egui::{Color32, Pos2};
use std::collections::{BTreeMap, HashSet};

/// What the rules need to know about the cards being drawn.
pub(crate) struct Ink<'a> {
    /// The colour card `k`'s link takes when lit.
    pub card: &'a dyn Fn(usize) -> Color32,
    /// The colour of a stretch from uplink `up` that leads to cards of
    /// several colours: the uplink's own.
    pub uplink: &'a dyn Fn(usize) -> Color32,
    /// The same for a stretch of one trunk alone, from `up` into box `g`:
    /// the box's colour.
    pub boxed: &'a dyn Fn(usize, usize) -> Color32,
    /// Card `k`'s link.
    pub link: &'a dyn Fn(usize) -> Link,
    /// How strongly card `k`'s link is drawn, 0 to 1 (1 if nothing fades).
    pub alpha: &'a dyn Fn(usize) -> f32,
}

/// How a lit piece of line looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Look {
    pub colour: Color32,
    /// Its cards' colours differ: the uplink's (or box's) colour, its halo
    /// quieter.
    pub mixed: bool,
    /// The strongest of its cards' links: one solid link makes it solid.
    pub link: Link,
    /// The least faded of its cards'.
    pub alpha: f32,
    /// Part of a trunk into a box, drawn a little wider.
    pub trunk: bool,
}

/// How each of the `pieces` of the lit `lines` (see `layout::pieces`)
/// looks: the colour its cards share, else that of the one trunk it's on
/// (its box's) or of their uplink.
pub(crate) fn piece_looks(lines: &[&Line], pieces: &[Piece], ink: &Ink) -> Vec<Look> {
    pieces
        .iter()
        .map(|pc| {
            let first = lines[pc.lines[0]];
            let colours = distinct(pc.feeds.iter().map(|&k| (ink.card)(k)));
            let fallback = match (pc.lines.len(), first.into) {
                (1, Some(g)) => (ink.boxed)(first.up, g),
                _ => (ink.uplink)(first.up),
            };
            let (colour, mixed) = path_colour(&colours, fallback);
            Look {
                colour,
                mixed,
                link: strongest(pc.feeds.iter().map(|&k| (ink.link)(k))),
                alpha: pc.feeds.iter().map(|&k| (ink.alpha)(k)).fold(0.0, f32::max),
                trunk: pc.lines.iter().any(|&l| lines[l].into.is_some()),
            }
        })
        .collect()
}

/// A stretch of lit line drawn as one polyline: pieces that look alike,
/// one after another along a line, so its corners join.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    pub pts: Vec<Pos2>,
    /// How far along its line it starts, so a line in pieces dashes as one.
    pub s0: f32,
    pub look: Look,
    /// The most lines along any of its pieces.
    fed: usize,
}

/// The lit `lines` as runs to draw, in order: each piece once, walking
/// the lines longest first (so a column's trunk is one line down to its
/// last lit card and the others add their own stubs), the shared
/// stretches first so the stubs leaving them sit on top.
pub(crate) fn runs(
    lines: &[&Line],
    pieces: &[Piece],
    steps: &[Vec<Step>],
    looks: &[Look],
) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let mut drawn = vec![false; pieces.len()];
    for li in deepest_first(lines) {
        let mut run: Option<Run> = None;
        for st in &steps[li] {
            if std::mem::replace(&mut drawn[st.piece], true) {
                out.extend(run.take());
                continue;
            }
            let (look, fed) = (looks[st.piece], pieces[st.piece].lines.len());
            match &mut run {
                Some(r) if r.look == look && r.pts.last() == Some(&st.a) => {
                    r.pts.push(st.b);
                    r.fed = r.fed.max(fed);
                }
                _ => {
                    out.extend(run.take());
                    run = Some(Run {
                        pts: vec![st.a, st.b],
                        s0: st.s,
                        look,
                        fed,
                    });
                }
            }
        }
        out.extend(run);
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.fed));
    out
}

/// One colour's packets down one lit line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Packets {
    /// The line (an index into those given to `packet_plan`).
    pub line: usize,
    pub colour: Color32,
    /// Its slot in the cycle of the lines from its uplink, and how many
    /// slots the cycle has: packets `slots` gaps apart, `slot` gaps on
    /// from the cycle's start.
    pub slot: usize,
    pub slots: usize,
    /// How far along the line they start: before that, the same packets
    /// are already running down a line it shares the way with.
    pub from: f32,
    /// The strongest link, and the least faded, of the cards they're for.
    pub link: Link,
    pub alpha: f32,
}

/// The packets down the lit `lines`, a colour each for the cards a line
/// leads to. The lines from one uplink share a cycle, its slots taken in
/// turn by their cards' colours in the order they branch off going down
/// (see `packet_slots`); each colour's packets run down its deepest line,
/// and down the others only from where they leave the lines before, so on
/// a shared stretch the colours take turns and none hides another.
pub(crate) fn packet_plan(
    lines: &[&Line],
    pieces: &[Piece],
    steps: &[Vec<Step>],
    ink: &Ink,
) -> Vec<Packets> {
    let mut by_up: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for li in deepest_first(lines) {
        by_up.entry(lines[li].up).or_default().push(li);
    }
    // A line's cards by colour, in order.
    let colours = |li: usize| {
        let mut out: Vec<(Color32, Vec<usize>)> = Vec::new();
        for &k in &lines[li].kids {
            let c = (ink.card)(k);
            match out.iter_mut().find(|x| x.0 == c) {
                Some(x) => x.1.push(k),
                None => out.push((c, vec![k])),
            }
        }
        out
    };
    let mut out = Vec::new();
    for (up, group) in by_up {
        // Where each line's way becomes its own.
        let parts = |li: usize| {
            let own = steps[li].iter().find(|s| pieces[s.piece].lines == [li]);
            own.map_or(arc(&lines[li].pts), |s| s.s)
        };
        let mut branches: Vec<(f32, usize)> = group.iter().map(|&li| (parts(li), li)).collect();
        branches.sort_by(|a, b| a.0.total_cmp(&b.0));
        let order = distinct(
            branches
                .iter()
                .flat_map(|&(_, li)| colours(li).into_iter().map(|c| c.0)),
        );
        let slots = packet_slots(&order, (ink.uplink)(up));
        let mut covered: Vec<HashSet<usize>> = vec![HashSet::new(); slots.len()];
        for li in group {
            for (c, kids) in colours(li) {
                let slot = order
                    .iter()
                    .position(|&x| x == c)
                    .unwrap_or(0)
                    .min(slots.len() - 1);
                let Some(from) = steps[li].iter().find(|s| !covered[slot].contains(&s.piece))
                else {
                    continue;
                };
                out.push(Packets {
                    line: li,
                    colour: slots[slot],
                    slot,
                    slots: slots.len(),
                    from: from.s,
                    link: strongest(kids.iter().map(|&k| (ink.link)(k))),
                    alpha: kids.iter().map(|&k| (ink.alpha)(k)).fold(0.0, f32::max),
                });
                covered[slot].extend(steps[li].iter().map(|s| s.piece));
            }
        }
    }
    out
}

/// The lit lines worked out for drawing: their runs and packets, kept from
/// frame to frame while the lines and their cards' looks stay the same
/// (the map only moves while a card is dragged), as working them out is
/// quadratic in the cards along one column.
#[derive(Default)]
pub(crate) struct Lit {
    /// The lit lines, each trunk carrying only its lit cards.
    pub lines: Vec<Line>,
    /// Their cards' and uplinks' colours, links and fades, as last worked out.
    seen: Vec<(Color32, u8, u32)>,
    pub runs: Vec<Run>,
    pub packets: Vec<Packets>,
}

impl Lit {
    /// Work out `lines` for drawing, unless they and their looks are as
    /// they were last time.
    pub(crate) fn update(&mut self, lines: Vec<Line>, ink: &Ink) {
        let mut seen = Vec::new();
        for l in &lines {
            seen.push(((ink.uplink)(l.up), 0, 0));
            if let Some(g) = l.into {
                seen.push(((ink.boxed)(l.up, g), 0, 0));
            }
            seen.extend(l.kids.iter().map(|&k| {
                let link = layout::strength((ink.link)(k));
                ((ink.card)(k), link, (ink.alpha)(k).to_bits())
            }));
        }
        if lines == self.lines && seen == self.seen {
            return;
        }
        let refs: Vec<&Line> = lines.iter().collect();
        let (pieces, steps) = layout::pieces(&refs);
        let looks = piece_looks(&refs, &pieces, ink);
        self.runs = runs(&refs, &pieces, &steps, &looks);
        self.packets = packet_plan(&refs, &pieces, &steps, ink);
        self.lines = lines;
        self.seen = seen;
    }
}

/// The lines' indices, longest first.
fn deepest_first(lines: &[&Line]) -> Vec<usize> {
    let lens: Vec<f32> = lines.iter().map(|l| arc(&l.pts)).collect();
    let mut deep: Vec<usize> = (0..lines.len()).collect();
    deep.sort_by(|&a, &b| lens[b].total_cmp(&lens[a]));
    deep
}

/// The most solid of some links.
fn strongest(links: impl Iterator<Item = Link>) -> Link {
    links
        .max_by_key(|&l| layout::strength(l))
        .unwrap_or(Link::Wired)
}

/// Packets of a colour with less chroma than this are hollow rings: the
/// internet's and unknown cards' greys, the clients' grey-blue, a white
/// client. A greyish dot on a coloured line is hard to tell from a faded
/// one; every theme's role colours are well above it.
pub(crate) const GREY: f32 = 0.08;

/// The colour a lit stretch of line takes from the colours of the cards it
/// leads to: theirs when they share one; else its uplink's, as a blend of
/// them would look like a card that isn't on the map. True when mixed.
pub(crate) fn path_colour(cards: &[Color32], uplink: Color32) -> (Color32, bool) {
    match cards.split_first() {
        Some((&c, rest)) if rest.iter().all(|&x| x == c) => (c, false),
        Some(_) => (uplink, true),
        None => (uplink, false),
    }
}

/// The colour of each slot in the packets' cycle along the lines from one
/// uplink, for its cards' colours in the order they branch off: the first
/// four their own, any more sharing a fifth in the uplink's colour.
pub(crate) fn packet_slots(colours: &[Color32], uplink: Color32) -> Vec<Color32> {
    let mut slots: Vec<Color32> = colours.iter().copied().take(4).collect();
    if colours.len() > 4 || slots.is_empty() {
        slots.push(uplink);
    }
    slots
}

/// The colours in order, each once.
pub(crate) fn distinct(colours: impl Iterator<Item = Color32>) -> Vec<Color32> {
    let mut out = Vec::new();
    for c in colours {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// A polyline's length.
fn arc(pts: &[Pos2]) -> f32 {
    pts.windows(2).map(|s| s[0].distance(s[1])).sum()
}

/// `c` as a line or packet on the canvas `bg`: as it is, unless it's too
/// near the canvas's lightness to see; then lighter or darker, keeping its
/// hue and as much of its chroma as fits.
pub(crate) fn legible(c: Color32, bg: Color32) -> Color32 {
    const MIN: f32 = 0.25;
    let [l, a, b] = oklab(c);
    let back = oklab(bg)[0];
    if (l - back).abs() >= MIN {
        return c;
    }
    let l = if back > 0.5 { back - MIN } else { back + MIN }.clamp(0.0, 1.0);
    // Less chroma until it's a colour a screen can show.
    let mut k = 1.0;
    for _ in 0..16 {
        if let Some([r, g, b]) = from_oklab([l, a * k, b * k]) {
            return Color32::from_rgba_unmultiplied(r, g, b, c.a());
        }
        k *= 0.8;
    }
    let [r, g, b] = from_oklab([l, 0.0, 0.0]).unwrap_or([128; 3]);
    Color32::from_rgba_unmultiplied(r, g, b, c.a())
}

/// How a packet in colour `c` is drawn on the canvas `bg`: in that colour
/// made `legible`, and as a hollow ring (true) when it's a grey card's.
pub(crate) fn packet_look(c: Color32, bg: Color32) -> (Color32, bool) {
    (legible(c, bg), chroma(c) < GREY)
}

/// How colourful `c` is (OKLab chroma): about 0 for greys, 0.1 to 0.3 for
/// the roles' colours.
pub(crate) fn chroma(c: Color32) -> f32 {
    let [_, a, b] = oklab(c);
    a.hypot(b)
}

/// OKLab (lightness, a, b) of a colour, ignoring its alpha.
fn oklab(c: Color32) -> [f32; 3] {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    let lin = |v: u8| {
        let x = v as f32 / 255.0;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(r), lin(g), lin(b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// An OKLab colour in sRGB, or None when a screen can't show it.
fn from_oklab([l, a, b]: [f32; 3]) -> Option<[u8; 3]> {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let rgb = [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ];
    if rgb.iter().any(|&v| !(-1e-3..=1.0 + 1e-3).contains(&v)) {
        return None;
    }
    Some(rgb.map(|v| {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0).round() as u8
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::layout::{Layout, Mode};
    use crate::map::{Map, Node, Zone};
    use crate::theme::Theme;
    use eframe::egui::pos2;

    fn rgb(h: u32) -> Color32 {
        Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }

    #[test]
    fn a_stretch_led_to_one_colour_takes_it_and_a_mixed_one_its_uplinks() {
        let (gw, teal, grey) = (rgb(0x5b9dff), rgb(0x4fc3d9), rgb(0x8fa8c8));
        assert_eq!(path_colour(&[teal], gw), (teal, false));
        assert_eq!(path_colour(&[teal, teal], gw), (teal, false));
        assert_eq!(path_colour(&[teal, grey], gw), (gw, true));
        // Whatever order the cards come in: never a blend of them.
        assert_eq!(path_colour(&[grey, teal, teal], gw), (gw, true));
        assert_eq!(path_colour(&[], gw), (gw, false));
    }

    /// internet → gateway → a column of the screenshot's cards: two
    /// Hewletts and a Netgear (gear, teal) and this machine (a client).
    fn screenshot() -> (Map, Layout) {
        let mut m = Map::default();
        let zones = [
            Zone::Internet,
            Zone::Gateway,
            Zone::Gear,
            Zone::Gear,
            Zone::Gear,
            Zone::Client,
        ];
        for (i, z) in zones.into_iter().enumerate() {
            m.nodes.push(Node {
                id: i.to_string(),
                label: format!("n{i}"),
                zone: z,
                up: match i {
                    0 => None,
                    1 => Some((0, Link::Backbone, None)),
                    _ => Some((1, Link::Assumed, None)),
                },
                ..Default::default()
            });
        }
        let l = layout::layout_in(&m, &|_| true, Mode::Cluster);
        (m, l)
    }

    /// The rules for `m` in the Dark theme, a box taking `boxed`.
    fn with_ink<T>(m: &Map, boxed: Color32, f: impl FnOnce(&Ink) -> T) -> T {
        let colour = |k: usize| m.nodes[k].zone.colour_in(Theme::Dark);
        let ink = Ink {
            card: &colour,
            uplink: &colour,
            boxed: &|_, _| boxed,
            link: &|k| m.nodes[k].link().unwrap_or(Link::Wired),
            alpha: &|_| 1.0,
        };
        f(&ink)
    }

    #[test]
    fn each_piece_of_a_mixed_column_keeps_the_colour_it_leads_to() {
        let (m, l) = screenshot();
        let colour = |k: usize| m.nodes[k].zone.colour_in(Theme::Dark);
        let gw = colour(1);
        let lines = layout::lines(&m, &l, &|k| l.pos[k], &|_| false);
        // The second card (.254, teal) and this machine (grey) lit.
        let lit: Vec<&Line> = lines
            .iter()
            .filter(|x| x.kids == [3] || x.kids == [5])
            .collect();
        let (pieces, steps) = layout::pieces(&lit);
        let looks = with_ink(&m, gw, |ink| piece_looks(&lit, &pieces, ink));
        let of = |p: usize| (looks[p].colour, looks[p].mixed);
        let teal = colour(3);
        let grey = colour(5);
        // This machine's line: the gateway's colour (mixed) down to where
        // .254's stub leaves, then its own grey, stub and all.
        let mine = &steps[lit.iter().position(|x| x.kids == [5]).unwrap()];
        let seen: Vec<(Color32, bool)> = mine.iter().map(|s| of(s.piece)).collect();
        let split = seen.iter().position(|c| !c.1).unwrap();
        assert!(split > 0);
        assert!(seen[..split].iter().all(|&c| c == (gw, true)));
        assert!(seen[split..].iter().all(|&c| c == (grey, false)));
        // .254's own stub is teal.
        let its = &steps[lit.iter().position(|x| x.kids == [3]).unwrap()];
        assert_eq!(of(its.last().unwrap().piece), (teal, false));
        // Drawn as runs, each piece once: the shared stretch first.
        let runs = runs(&lit, &pieces, &steps, &looks);
        assert_eq!(runs[0].look.colour, gw);
        let ends: usize = runs.iter().map(|r| r.pts.len() - 1).sum();
        assert!(ends >= pieces.len());
        // Lit alone, the whole of .254's line is teal (the 2.png case).
        let alone: Vec<&Line> = lines.iter().filter(|x| x.kids == [3]).collect();
        let (pieces, _) = layout::pieces(&alone);
        let looks = with_ink(&m, gw, |ink| piece_looks(&alone, &pieces, ink));
        assert!(looks.iter().all(|l| (l.colour, l.mixed) == (teal, false)));
    }

    /// Two trunks from one uplink (0) that leave it together and part:
    /// into box 0 with cards 1 (gear) and 2 (a client), and into box 1
    /// with card 3 (a server). The Zones case of the review: the shared
    /// stretch took the colour of whichever was drawn last, and hid the
    /// other's packets under its own.
    #[test]
    fn trunks_that_share_a_way_share_its_colour_and_take_turns_with_packets() {
        let mut m = Map::default();
        for (i, z) in [Zone::Gateway, Zone::Gear, Zone::Client, Zone::Server]
            .into_iter()
            .enumerate()
        {
            m.nodes.push(Node {
                id: i.to_string(),
                zone: z,
                up: (i > 0).then_some((0, Link::Wired, None)),
                ..Default::default()
            });
        }
        let trunk = |kids: Vec<usize>, g: usize, x: f32| Line {
            up: 0,
            kids,
            into: Some(g),
            quiet: false,
            pts: vec![
                pos2(0.0, 0.0),
                pos2(0.0, 20.0),
                pos2(x, 20.0),
                pos2(x, 60.0),
            ],
        };
        let (a, b) = (trunk(vec![1, 2], 0, -80.0), trunk(vec![3], 1, 120.0));
        let lit = [&a, &b];
        let (pieces, steps) = layout::pieces(&lit);
        let colour = |k: usize| m.nodes[k].zone.colour_in(Theme::Dark);
        let boxed = rgb(0x123456);
        let looks = with_ink(&m, boxed, |ink| piece_looks(&lit, &pieces, ink));
        // The drop they share: the uplink's colour (three cards, two
        // colours), wide as a trunk.
        let drop = steps[0][0].piece;
        assert_eq!(steps[1][0].piece, drop);
        assert_eq!(pieces[drop].lines, [0, 1]);
        assert_eq!((looks[drop].colour, looks[drop].mixed), (colour(0), true));
        assert!(looks[drop].trunk);
        // Each trunk's own way: a mixed trunk alone in its box's colour,
        // the other in its card's.
        let own = |li: usize| looks[steps[li].last().unwrap().piece];
        assert_eq!((own(0).colour, own(0).mixed), (boxed, true));
        assert_eq!((own(1).colour, own(1).mixed), (colour(3), false));
        // Packets: one slot per colour across both trunks, so each colour
        // runs down the shared drop once, in a slot of its own.
        let plan = with_ink(&m, boxed, |ink| packet_plan(&lit, &pieces, &steps, ink));
        assert_eq!(plan.len(), 3);
        assert!(plan.iter().all(|p| p.slots == 3 && p.from == 0.0));
        let mut slots: Vec<usize> = plan.iter().map(|p| p.slot).collect();
        slots.sort_unstable();
        assert_eq!(slots, [0, 1, 2]);
        let of = |c: Color32| plan.iter().find(|p| p.colour == c).unwrap();
        assert_eq!(of(colour(3)).line, 1);
        assert_eq!(of(colour(1)).line, 0);
        assert_eq!(of(colour(2)).line, 0);
    }

    /// The lit lines are worked out again whenever what they show changes:
    /// a card moved (a drag, undo or redo, a saved map opened, clients
    /// shown), a card's colour (a theme), its link, or its fade (a role
    /// faded out from the legend or the sidebar); and kept while nothing has.
    #[test]
    fn the_lit_cache_follows_moves_colours_and_fades() {
        let (m, l) = screenshot();
        let lines = layout::lines(&m, &l, &|k| l.pos[k], &|_| false);
        let lit: Vec<Line> = lines.iter().filter(|x| x.kids == [3]).cloned().collect();
        let fade = std::cell::Cell::new(1.0f32);
        let teal = std::cell::Cell::new(m.nodes[3].zone.colour_in(Theme::Dark));
        let colour = |k: usize| {
            if k == 3 {
                teal.get()
            } else {
                m.nodes[k].zone.colour_in(Theme::Dark)
            }
        };
        let alpha = |_: usize| fade.get();
        let ink = Ink {
            card: &colour,
            uplink: &colour,
            boxed: &|_, _| Color32::RED,
            link: &|k| m.nodes[k].link().unwrap_or(Link::Wired),
            alpha: &alpha,
        };
        let mut c = Lit::default();
        c.update(lit.clone(), &ink);
        let first = c.runs.clone();
        assert!(!first.is_empty());
        c.update(lit.clone(), &ink);
        assert_eq!(c.runs, first);
        // Faded out.
        fade.set(0.15);
        c.update(lit.clone(), &ink);
        assert!(c.runs.iter().all(|r| r.look.alpha == 0.15));
        // Another colour.
        teal.set(Color32::from_rgb(200, 40, 120));
        c.update(lit.clone(), &ink);
        assert!(c.runs.iter().any(|r| r.look.colour == teal.get()));
        // Moved: the card dragged, or put back by undo.
        let at = |k: usize| {
            l.pos[k].map(|p| {
                if k == 3 {
                    p + eframe::egui::vec2(300.0, 0.0)
                } else {
                    p
                }
            })
        };
        let moved = layout::lines(&m, &l, &at, &|k| k == 3);
        let lit2: Vec<Line> = moved.into_iter().filter(|x| x.kids == [3]).collect();
        c.update(lit2.clone(), &ink);
        assert_eq!(c.lines, lit2);
        c.update(lit, &ink);
        assert!(
            c.runs
                .iter()
                .all(|r| r.pts.iter().all(|p| p.x < 300.0 + l.pos[3].unwrap().x))
        );
        assert_eq!(
            c.lines[0].pts,
            lines.iter().find(|x| x.kids == [3]).unwrap().pts
        );
    }

    #[test]
    fn packets_take_turns_one_slot_a_colour() {
        let c: Vec<Color32> = (1..=6).map(|k| rgb(k * 0x202020)).collect();
        let up = rgb(0x5b9dff);
        // One colour: one slot, so the packets space as they always have.
        assert_eq!(packet_slots(&c[..1], up), [c[0]]);
        assert_eq!(packet_slots(&c[..4], up), c[..4]);
        // A fifth and beyond share one slot in the uplink's colour.
        assert_eq!(packet_slots(&c, up), [c[0], c[1], c[2], c[3], up]);
        assert_eq!(packet_slots(&[], up), [up]);
    }

    #[test]
    fn a_colour_too_near_the_canvas_is_pushed_off_it() {
        let dark = rgb(0x1b1b1b);
        let light = rgb(0xf6f7f9);
        let gw = rgb(0x5b9dff);
        // Far enough from both: unchanged.
        assert_eq!(legible(gw, dark), gw);
        assert_eq!(legible(rgb(0x2563c9), light), rgb(0x2563c9));
        // A dim line on the dark canvas comes up; a pale one on the light
        // canvas goes down; either way a quarter of OKLab lightness off it.
        for (c, bg) in [
            (rgb(0x3a2a5a), dark),
            (rgb(0xd8f0ff), light),
            (rgb(0xffffff), light),
        ] {
            let out = legible(c, bg);
            let gap = (oklab(out)[0] - oklab(bg)[0]).abs();
            assert!(gap >= 0.24, "{c:?} to {out:?}: {gap}");
            // Same hue, where it has one.
            if chroma(c) > 0.03 {
                let (h0, h1) = (oklab(c), oklab(out));
                let turn = (h0[2].atan2(h0[1]) - h1[2].atan2(h1[1])).abs();
                assert!(turn < 0.1, "{c:?} to {out:?}");
            }
        }
        // Deterministic, and settled: a second pass changes nothing.
        let once = legible(rgb(0x3a2a5a), dark);
        assert_eq!(legible(rgb(0x3a2a5a), dark), once);
        assert_eq!(legible(once, dark), once);
    }

    #[test]
    fn greys_count_as_grey_and_roles_do_not() {
        assert!(chroma(Color32::WHITE) < GREY);
        for t in crate::theme::Theme::ALL {
            for z in [
                Zone::Gateway,
                Zone::Gear,
                Zone::Switch,
                Zone::Wireless,
                Zone::Server,
                Zone::Device,
            ] {
                assert!(chroma(z.colour_in(t)) > GREY, "{z:?} in {t:?}");
            }
            // Clients are grey-blue but in the neon themes.
            use crate::theme::Theme::{Miami, Phosphor, Rgb};
            if !matches!(t, Miami | Phosphor | Rgb) {
                assert!(chroma(Zone::Client.colour_in(t)) < GREY, "clients in {t:?}");
            }
        }
    }
}
