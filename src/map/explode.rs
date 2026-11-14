//! The exploded view: the laid-out map pulled apart like an exploded
//! drawing. Each role is a plate (internet at the top, clients at the
//! bottom); the plates tilt back, turn about the upright axis and lift
//! apart, and a trace line runs from each uplink straight down to the
//! plate its cards sit on, then across that plate to each of them.
//!
//! Only places move: a card stays upright, at its own size, so its text
//! reads at any angle. The projection has no perspective (lines that are
//! parallel on the map stay parallel), and at no explode it is the flat
//! map exactly, so the view is the same map opened up, not another one.
//! Split pulls the map's sub-assemblies (each branch off the card where
//! the tree first forks) apart sideways as well.
//!
//! Nothing here draws: `build` takes where the flat layout put each card
//! and gives back where it goes, the order to draw in, the plates and the
//! lines. The Map tab draws those as it draws any layout.

use super::Zone;
use super::layout::{CARD, GAP_X, Line, STACK_GAP};
use eframe::egui::{Pos2, Rect, Vec2, vec2};
use std::collections::{BTreeMap, HashSet};

/// How flat the plates lie fully exploded: a plate's depth is drawn at
/// this much of its width.
pub const TILT: f32 = 0.5;
/// The turn the view opens at, and the step of a turn left or right: the
/// corners of an isometric drawing, and the views square on between them.
pub const SNAP: f32 = std::f32::consts::FRAC_PI_4;
/// A plate's margin round its cards, on the flat map.
const PLATE_PAD: f32 = 22.0;
/// The room between two plates fully exploded, for the lines between them.
const PLATE_GAP: f32 = 64.0;
/// The room between two sub-assemblies fully split.
const SPLIT_GAP: f32 = 180.0;
/// The most the map is spread to keep turned cards off each other.
const SPREAD_MAX: f32 = 2.0;
/// In 3D, how far one plate floats above the next fully exploded: this
/// much of the map's width across, within these limits, so a big site's
/// plates stand as clear of each other as a small one's.
const TIER_GAP: f32 = 0.2;
const TIER_GAP_MIN: f32 = 300.0;
const TIER_GAP_MAX: f32 = 1600.0;
/// In 3D, how low and how high the view looks down from, in radians
/// above the plates: nearly edge on, to nearly straight down.
pub const PITCH_MIN: f32 = 0.12;
pub const PITCH_MAX: f32 = 1.45;
/// In 3D, how much nearer or farther a card is drawn, at most.
const SCALE_MIN: f32 = 0.4;
const SCALE_MAX: f32 = 2.2;
/// In 3D, the spacing of the lines ruled across a plate, and the most
/// lines each way.
const GRID_STEP: f32 = 120.0;
const GRID_MAX: f32 = 18.0;

/// What a plate holding these roles is called.
pub fn plate_name(zones: &[Zone]) -> String {
    let names: Vec<String> = Zone::ALL
        .into_iter()
        .filter(|z| zones.contains(z))
        .map(|z| z.name().to_uppercase())
        .collect();
    names.join(" · ")
}

impl Camera {
    /// The view as it first opens: all the way apart, from a corner.
    pub const OPEN: Camera = Camera {
        amount: 1.0,
        yaw: SNAP,
        split: 0.0,
        pitch: 0.6,
        depth: 0.0,
    };
}

/// How the map is opened up.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Camera {
    /// 0 (the flat map) to 1 (plates clear of each other).
    pub amount: f32,
    /// The turn about the upright axis, in radians.
    pub yaw: f32,
    /// 0 to 1: how far the sub-assemblies are pulled apart sideways.
    pub split: f32,
    /// In 3D, how far above the plates the view looks from, in radians
    /// (`PITCH_MIN` to `PITCH_MAX`).
    pub pitch: f32,
    /// 0 (the exploded drawing: no perspective, a fixed tilt) to 1 (3D:
    /// the view from a point, at any height, nearer things drawn bigger).
    pub depth: f32,
}

/// One card on the flat map.
#[derive(Clone, Copy, Debug)]
pub struct Part {
    /// Where the flat layout draws it.
    pub rect: Rect,
    /// Its plate, counted from the top (0: the internet).
    pub tier: usize,
    /// The card it hangs off.
    pub parent: Option<usize>,
    /// Its sub-assembly (`assemblies`), if it's in one.
    pub group: Option<usize>,
}

/// A plate: one role's cards (of one sub-assembly, while split).
#[derive(Clone, Debug, PartialEq)]
pub struct Plate {
    pub tier: usize,
    pub group: Option<usize>,
    /// Its corners as drawn, in the flat map's order: top left, top right,
    /// bottom right, bottom left.
    pub quad: [Pos2; 4],
    /// The cards on it.
    pub cards: Vec<usize>,
    /// In 3D, the lines ruled across it, each end to end.
    pub grid: Vec<[Pos2; 2]>,
}

/// The map opened up.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// Top left of each card; None for cards not shown.
    pub pos: Vec<Option<Pos2>>,
    /// The cards in the order to draw them: the lowest plate first, and on
    /// a plate the farthest card first, so a nearer one covers it.
    pub order: Vec<usize>,
    /// The plates, lowest first (the order to draw them).
    pub plates: Vec<Plate>,
    /// Each card's line from its uplink (uplink end first). A line between
    /// plates has three points: down to the plate (its foot), then across.
    pub lines: Vec<Line>,
    /// Where lines between plates land on the lower one.
    pub feet: Vec<Pos2>,
    /// How big each card is drawn, of its own size: 1 but in 3D, where a
    /// nearer card is bigger.
    pub scale: Vec<f32>,
    /// In 3D, the circle the map stands on, under its lowest plate.
    pub ring: Vec<Pos2>,
    /// In 3D, what the view takes up however it's turned: while it turns
    /// by itself Fit frames this, so the map doesn't grow and shrink.
    pub frame: Option<Rect>,
}

impl Scene {
    /// How big card `i` is drawn, of its own size.
    pub fn scale_of(&self, i: usize) -> f32 {
        self.scale.get(i).copied().unwrap_or(1.0)
    }

    /// Everything drawn: cards (of `size`) and plates.
    pub fn bounds(&self, size: &dyn Fn(usize) -> Vec2) -> Rect {
        let mut r = Rect::NOTHING;
        for (i, p) in self.pos.iter().enumerate() {
            if let Some(p) = p {
                r = r.union(Rect::from_min_size(*p, size(i) * self.scale_of(i)));
            }
        }
        for plate in &self.plates {
            for q in plate.quad {
                r.extend_with(q);
            }
        }
        r
    }
}

/// How much the map is spread out at this turn and tilt, so that a card
/// in front never covers the name of the one behind it: cards a row or a
/// column apart on the flat map are drawn two thirds of a card apart
/// sideways, or a name's height up and down.
fn spread(yaw: f32, tilt: f32) -> f32 {
    let (s, c) = (yaw.sin().abs(), yaw.cos().abs());
    let (w, h) = (CARD.x * 0.66, CARD.y * 0.35);
    let (px, py) = (CARD.x + GAP_X, CARD.y + STACK_GAP);
    ((c * w + s * h / tilt) / px)
        .max((s * w + c * h / tilt) / py)
        .clamp(1.0, SPREAD_MAX)
}

/// Where the line from the middle of `r` toward `to` leaves it.
fn edge(r: Rect, to: Pos2) -> Pos2 {
    let c = r.center();
    let d = to - c;
    let (hw, hh) = (r.width() / 2.0, r.height() / 2.0);
    let tx = if d.x.abs() > 1e-3 {
        hw / d.x.abs()
    } else {
        f32::INFINITY
    };
    let ty = if d.y.abs() > 1e-3 {
        hh / d.y.abs()
    } else {
        f32::INFINITY
    };
    let t = tx.min(ty);
    if t >= 1.0 { c } else { c + d * t }
}

/// The sub-assembly each card is in: the tree is followed down from its
/// top to the first card with two or more branches (children that have
/// children of their own), and each of those branches, with everything
/// under it, is one. Cards above the fork, and its children with nothing
/// under them, are in none. `parent` is each shown card's uplink (None for
/// a card not shown or with no uplink shown).
pub fn assemblies(parent: &[Option<usize>], shown: &[bool]) -> Vec<Option<usize>> {
    let n = parent.len();
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut tops = Vec::new();
    let is_shown = |i: usize| shown.get(i).copied().unwrap_or(false);
    for (i, up) in parent.iter().enumerate() {
        if !is_shown(i) {
            continue;
        }
        match up.filter(|&p| p < n && p != i && is_shown(p)) {
            Some(p) => kids[p].push(i),
            None => tops.push(i),
        }
    }
    let mut out = vec![None; n];
    let mut level = tops;
    // Never more steps than cards: a loop in the links ends the walk.
    for _ in 0..=n {
        let branches: Vec<usize> = level
            .iter()
            .copied()
            .filter(|&k| !kids[k].is_empty())
            .collect();
        match branches[..] {
            [] => break,
            [one] => level = kids[one].clone(),
            _ => {
                for (g, &b) in branches.iter().enumerate() {
                    let mut todo = vec![b];
                    while let Some(k) = todo.pop() {
                        if out[k].is_some() {
                            continue;
                        }
                        out[k] = Some(g);
                        todo.extend(&kids[k]);
                    }
                }
                break;
            }
        }
    }
    out
}

/// How far each sub-assembly moves sideways fully split: in the order
/// they sit, each clear of the one before by `SPLIT_GAP`, the whole
/// spread evenly about where it was.
fn split_offsets(parts: &[Option<Part>]) -> BTreeMap<usize, f32> {
    let mut span: BTreeMap<usize, (f32, f32)> = BTreeMap::new();
    for p in parts.iter().flatten() {
        if let Some(g) = p.group {
            let e = span.entry(g).or_insert((f32::INFINITY, f32::NEG_INFINITY));
            *e = (e.0.min(p.rect.left()), e.1.max(p.rect.right()));
        }
    }
    let mut order: Vec<(usize, (f32, f32))> = span.into_iter().collect();
    order.sort_by(|a, b| (a.1.0 + a.1.1).total_cmp(&(b.1.0 + b.1.1)));
    let mut out = BTreeMap::new();
    let mut prev: Option<(f32, f32)> = None;
    for (g, (left, right)) in &order {
        let off = match prev {
            Some((off, end)) => off.max(end + SPLIT_GAP - left),
            None => 0.0,
        };
        prev = Some((off, right + off));
        out.insert(*g, off);
    }
    let mid = prev.map_or(0.0, |p| p.0) / 2.0;
    out.values_mut().for_each(|o| *o -= mid);
    out
}

/// The flat-map point `q`, turned, tilted and spread about `about`.
#[derive(Clone, Copy)]
struct Proj {
    about: Pos2,
    sin: f32,
    cos: f32,
    tilt: f32,
    spread: f32,
}

impl Proj {
    /// How far back the point is (greater: nearer the viewer).
    fn depth(&self, q: Pos2) -> f32 {
        let d = q - self.about;
        d.x * self.sin + d.y * self.cos
    }

    fn at(&self, q: Pos2, lift: f32) -> Pos2 {
        let d = q - self.about;
        let across = d.x * self.cos - d.y * self.sin;
        self.about + vec2(across, self.depth(q) * self.tilt) * self.spread + vec2(0.0, lift)
    }
}

/// Open the map up. `parts` is every card by index (None: not shown).
pub fn build(parts: &[Option<Part>], cam: Camera) -> Scene {
    // A number that isn't one (a damaged file) is the view's usual.
    let sane = |v: f32, or: f32| if v.is_finite() { v } else { or };
    let cam = Camera {
        amount: sane(cam.amount, 0.0),
        yaw: sane(cam.yaw, 0.0),
        split: sane(cam.split, 0.0),
        pitch: sane(cam.pitch, Camera::OPEN.pitch),
        depth: sane(cam.depth, 0.0),
    };
    let amount = cam.amount.clamp(0.0, 1.0);
    let split = cam.split.clamp(0.0, 1.0) * amount.min(1.0);
    // The view turns and tilts over the first third of the way; the
    // plates lift apart all the way along.
    let turn = (amount * 3.0).min(1.0);
    let turn = turn * turn * (3.0 - 2.0 * turn);
    let (yaw, tilt) = (cam.yaw * turn, 1.0 - (1.0 - TILT) * turn);

    // Sub-assemblies apart, on the flat map.
    let offs = if split > 0.0 {
        split_offsets(parts)
    } else {
        BTreeMap::new()
    };
    let flat: Vec<Option<Rect>> = parts
        .iter()
        .map(|p| {
            p.map(|p| {
                let dx = p.group.and_then(|g| offs.get(&g)).copied().unwrap_or(0.0);
                p.rect.translate(vec2(dx * split, 0.0))
            })
        })
        .collect();
    let all = flat
        .iter()
        .flatten()
        .fold(Rect::NOTHING, |r, c| r.union(*c));
    if !all.is_finite() {
        return Scene {
            pos: vec![None; parts.len()],
            scale: vec![1.0; parts.len()],
            ..Default::default()
        };
    }
    let proj = Proj {
        about: all.center(),
        sin: yaw.sin(),
        cos: yaw.cos(),
        tilt,
        spread: spread(yaw, tilt),
    };

    // The plates: each tier's cards (each sub-assembly's, while split),
    // within a margin.
    let mut on: BTreeMap<(usize, Option<usize>), Vec<usize>> = BTreeMap::new();
    for (i, p) in parts.iter().enumerate() {
        if let Some(p) = p {
            let group = if split > 0.0 { p.group } else { None };
            on.entry((p.tier, group)).or_default().push(i);
        }
    }
    let corners = |cards: &[usize]| {
        let r = cards
            .iter()
            .filter_map(|&i| flat[i])
            .fold(Rect::NOTHING, |r, c| r.union(c))
            .expand(PLATE_PAD);
        [
            r.left_top(),
            r.right_top(),
            r.right_bottom(),
            r.left_bottom(),
        ]
    };
    // How far each tier is lifted (down the screen) fully exploded: clear
    // of the tier above by `PLATE_GAP`, and never less than it.
    let mut extent: BTreeMap<usize, (f32, f32)> = BTreeMap::new();
    for (&(tier, _), cards) in &on {
        let e = extent
            .entry(tier)
            .or_insert((f32::INFINITY, f32::NEG_INFINITY));
        for q in corners(cards) {
            let y = proj.at(q, 0.0).y;
            *e = (e.0.min(y), e.1.max(y));
        }
        for &i in cards {
            let r = flat[i].expect("a card on a plate is shown");
            let y = proj.at(r.center(), 0.0).y;
            *e = (e.0.min(y - r.height() / 2.0), e.1.max(y + r.height() / 2.0));
        }
    }
    let mut lift: BTreeMap<usize, f32> = BTreeMap::new();
    let mut above: Option<(f32, f32)> = None;
    for (&tier, &(top, bottom)) in &extent {
        let l = match above {
            Some((l, end)) => l.max(end + PLATE_GAP - top),
            None => 0.0,
        };
        above = Some((l, bottom + l));
        lift.insert(tier, l * amount);
    }
    let lift_of = |tier: usize| lift.get(&tier).copied().unwrap_or(0.0);

    // 3D: the plates float one above the next, seen from a point `dist`
    // away, `pitch` above them. It comes in with the turn, so no explode
    // is still the flat map.
    let three = cam.depth.clamp(0.0, 1.0) * turn;
    let rank: BTreeMap<usize, usize> = extent.keys().enumerate().map(|(k, &t)| (t, k)).collect();
    let tiers = rank.len().saturating_sub(1) as f32;
    let (psin, pcos) = cam.pitch.clamp(PITCH_MIN, PITCH_MAX).sin_cos();
    // A thing in space keeps its shape as it turns and tips: one spread,
    // the most any turn of the exploded drawing needs.
    let spread3 = (0..=8)
        .map(|k| spread(k as f32 * std::f32::consts::FRAC_PI_8 / 2.0, TILT))
        .fold(1.0, f32::max);
    let radius = 0.5 * all.size().length() * spread3;
    let gap = (2.0 * radius * TIER_GAP).clamp(TIER_GAP_MIN, TIER_GAP_MAX) * amount;
    let stack = tiers * gap;
    let dist = (2.2 * (radius + stack / 2.0)).max(900.0);
    // Where a point of the flat map on plate `tier` is drawn, how big
    // things there are, and how near the viewer it is.
    let place = |q: Pos2, tier: usize| -> (Pos2, f32, f32) {
        let ortho = proj.at(q, lift_of(tier));
        if three <= 0.0 {
            return (ortho, 1.0, proj.depth(q));
        }
        let d = q - proj.about;
        let across = (d.x * proj.cos - d.y * proj.sin) * spread3;
        let back = proj.depth(q) * spread3;
        let up = (tiers / 2.0 - rank.get(&tier).copied().unwrap_or(0) as f32) * gap;
        let near = back * pcos + up * psin;
        let s = (1.0 / (1.0 - near / dist)).clamp(SCALE_MIN, SCALE_MAX);
        let at = proj.about + vec2(across, back * psin - up * pcos) * s;
        (ortho + (at - ortho) * three, 1.0 + (s - 1.0) * three, near)
    };

    let placed: Vec<Option<(Pos2, f32, f32)>> = (0..parts.len())
        .map(|i| {
            let (p, r) = (parts[i]?, flat[i]?);
            Some(place(r.center(), p.tier))
        })
        .collect();
    let scale: Vec<f32> = placed.iter().map(|p| p.map_or(1.0, |p| p.1)).collect();
    let pos: Vec<Option<Pos2>> = (0..parts.len())
        .map(|i| {
            let (at, s, _) = placed[i]?;
            Some(at - flat[i]?.size() * s / 2.0)
        })
        .collect();
    let drawn = |i: usize| Some(Rect::from_min_size(pos[i]?, flat[i]?.size() * scale[i]));

    let mut order: Vec<usize> = (0..parts.len()).filter(|&i| pos[i].is_some()).collect();
    if three > 0.5 {
        // From a point, what's nearer covers what's farther, whatever
        // plate either is on.
        let near = |i: usize| placed[i].map_or(0.0, |p| p.2);
        order.sort_by(|&a, &b| near(a).total_cmp(&near(b)).then(a.cmp(&b)));
    } else {
        let depth = |i: usize| flat[i].map_or(0.0, |r| proj.depth(r.center()));
        order.sort_by(|&a, &b| {
            let (ta, tb) = (parts[a].map(|p| p.tier), parts[b].map(|p| p.tier));
            tb.cmp(&ta)
                .then(depth(a).total_cmp(&depth(b)))
                .then(a.cmp(&b))
        });
    }

    let mut plates: Vec<Plate> = on
        .iter()
        .map(|(&(tier, group), cards)| Plate {
            tier,
            group,
            quad: corners(cards).map(|q| place(q, tier).0),
            cards: cards.clone(),
            grid: if three > 0.0 {
                let c = corners(cards);
                grid(Rect::from_min_max(c[0], c[2]))
                    .into_iter()
                    .map(|l| l.map(|q| place(q, tier).0))
                    .collect()
            } else {
                Vec::new()
            },
        })
        .collect();
    plates.reverse();

    let mut lines = Vec::new();
    let mut feet: Vec<Pos2> = Vec::new();
    let mut footed: HashSet<(usize, usize)> = HashSet::new();
    for (i, p) in parts.iter().enumerate() {
        let Some(p) = p else { continue };
        let Some(up) = p.parent.filter(|&u| u != i && u < parts.len()) else {
            continue;
        };
        let (Some(a), Some(b), Some(pa), Some(from)) = (drawn(up), drawn(i), parts[up], flat[up])
        else {
            continue;
        };
        // Straight down from the uplink to this card's plate, then across
        // it. On one plate, or before the plates have parted, straight
        // across.
        let foot = place(from.center(), p.tier).0;
        let pts = if pa.tier == p.tier || a.expand(6.0).contains(foot) {
            vec![edge(a, b.center()), edge(b, a.center())]
        } else {
            let start = edge(a, foot);
            if footed.insert((up, p.tier)) {
                feet.push(foot);
            }
            if b.contains(foot) {
                vec![start, edge(b, start)]
            } else {
                vec![start, foot, edge(b, foot)]
            }
        };
        lines.push(Line {
            up,
            kids: vec![i],
            into: None,
            quiet: false,
            pts,
        });
    }
    // The circle the map stands on, and what the view takes up whichever
    // way it's turned.
    let (ring, frame) = match extent.keys().next_back() {
        Some(&lowest) if three > 0.0 => {
            let r = 0.5 * all.size().length() + 2.0 * PLATE_PAD;
            let ring = (0..=64)
                .map(|k| {
                    let a = k as f32 / 64.0 * std::f32::consts::TAU;
                    place(proj.about + vec2(a.cos(), a.sin()) * r, lowest).0
                })
                .collect();
            let nearest = (radius * pcos + stack / 2.0 * psin) / dist;
            let s = (1.0 / (1.0 - nearest)).clamp(1.0, SCALE_MAX);
            let half = vec2(radius + CARD.x, radius * psin + stack / 2.0 * pcos + CARD.y) * s;
            let frame = (three >= 0.999).then(|| Rect::from_center_size(proj.about, half * 2.0));
            (ring, frame)
        }
        _ => (Vec::new(), None),
    };
    Scene {
        pos,
        order,
        plates,
        lines,
        feet,
        scale,
        ring,
        frame,
    }
}

/// The lines ruled across a plate covering `r` of the flat map: every
/// `GRID_STEP` each way, fewer and farther apart on a big plate.
fn grid(r: Rect) -> Vec<[Pos2; 2]> {
    if !r.is_positive() {
        return Vec::new();
    }
    let step = |len: f32| GRID_STEP * (len / GRID_STEP / GRID_MAX).ceil().max(1.0);
    let mut out = Vec::new();
    let sx = step(r.width());
    let mut x = (r.left() / sx).ceil() * sx;
    while x < r.right() {
        out.push([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())]);
        x += sx;
    }
    let sy = step(r.height());
    let mut y = (r.top() / sy).ceil() * sy;
    while y < r.bottom() {
        out.push([Pos2::new(r.left(), y), Pos2::new(r.right(), y)]);
        y += sy;
    }
    out
}

/// The turn a step left (`dir` below 0) or right of `yaw` comes to: the
/// next `SNAP`.
pub fn turned(yaw: f32, dir: f32) -> f32 {
    let steps = yaw / SNAP;
    let next = if dir < 0.0 {
        (steps - 0.01).ceil() - 1.0
    } else {
        (steps + 0.01).floor() + 1.0
    };
    next * SNAP
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;
    use std::f32::consts::TAU;

    fn card(x: f32, y: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), CARD)
    }

    const FULL: Camera = Camera::OPEN;
    /// The same view in 3D.
    const THREE: Camera = Camera {
        depth: 1.0,
        ..Camera::OPEN
    };

    /// internet – gateway – two switches, each with three clients, laid
    /// out as a tree: a row a tier.
    fn tree() -> Vec<Option<Part>> {
        let part = |x: f32, row: usize, parent: Option<usize>, group| {
            Some(Part {
                rect: card(x, row as f32 * (CARD.y + 56.0)),
                tier: row,
                parent,
                group,
            })
        };
        let step = CARD.x + GAP_X;
        let mut v = vec![
            part(2.5 * step, 0, None, None),
            part(2.5 * step, 1, Some(0), None),
            part(1.0 * step, 2, Some(1), Some(0)),
            part(4.0 * step, 2, Some(1), Some(1)),
        ];
        for k in 0..6 {
            let sw = 2 + k / 3;
            v.push(part(k as f32 * step, 3, Some(sw), Some(sw - 2)));
        }
        v
    }

    fn rects(parts: &[Option<Part>], s: &Scene) -> Vec<Rect> {
        (0..parts.len())
            .map(|i| Rect::from_min_size(s.pos[i].unwrap(), parts[i].unwrap().rect.size()))
            .collect()
    }

    fn close(a: Pos2, b: Pos2) -> bool {
        a.distance(b) < 0.01
    }

    #[test]
    fn explode_zero_is_the_flat_map() {
        let parts = tree();
        for yaw in [0.0, SNAP, 2.0, -1.0] {
            let s = build(
                &parts,
                Camera {
                    amount: 0.0,
                    yaw,
                    split: 1.0,
                    pitch: 0.3,
                    depth: 1.0,
                },
            );
            for (i, p) in parts.iter().enumerate() {
                assert!(close(s.pos[i].unwrap(), p.unwrap().rect.min), "card {i}");
            }
        }
    }

    #[test]
    fn plates_follow_role_order() {
        let parts = tree();
        for k in 0..8 {
            let s = build(
                &parts,
                Camera {
                    yaw: k as f32 * SNAP,
                    ..FULL
                },
            );
            // Lowest first, and no plate reaches into the one above it.
            let tiers: Vec<usize> = s.plates.iter().map(|p| p.tier).collect();
            assert_eq!(tiers, [3, 2, 1, 0], "turn {k}");
            let span = |p: &Plate| {
                let ys = p.quad.map(|q| q.y);
                (
                    ys.iter().copied().fold(f32::INFINITY, f32::min),
                    ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
                )
            };
            for w in s.plates.windows(2) {
                let (lower, upper) = (span(&w[0]), span(&w[1]));
                assert!(
                    lower.0 >= upper.1 + PLATE_GAP - 0.01,
                    "turn {k}: {lower:?} {upper:?}"
                );
            }
        }
    }

    #[test]
    fn an_empty_role_gets_no_plate() {
        let mut parts = tree();
        // No tier 2 at all: the switches are filed with the gateway.
        for p in parts.iter_mut().flatten() {
            if p.tier == 2 {
                p.tier = 1;
            }
        }
        let s = build(&parts, FULL);
        let tiers: Vec<usize> = s.plates.iter().map(|p| p.tier).collect();
        assert_eq!(tiers, [3, 1, 0]);
        // And no room kept for it: the clients sit one gap under tier 1.
        let bottom = |p: &Plate| {
            p.quad
                .map(|q| q.y)
                .into_iter()
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let top = |p: &Plate| {
            p.quad
                .map(|q| q.y)
                .into_iter()
                .fold(f32::INFINITY, f32::min)
        };
        let gap = top(&s.plates[0]) - bottom(&s.plates[1]);
        assert!(
            (PLATE_GAP..PLATE_GAP + 2.0 * CARD.y).contains(&gap),
            "{gap}"
        );
    }

    #[test]
    fn explode_only_moves_cards_along_the_axis() {
        // Past the turn, more explode lifts each plate as one: its cards
        // keep their places on it, and nothing moves sideways.
        let parts = tree();
        let at = |amount| rects(&parts, &build(&parts, Camera { amount, ..FULL }));
        let (a, b) = (at(0.5), at(1.0));
        for tier in 0..4 {
            let on: Vec<usize> = (0..parts.len())
                .filter(|&i| parts[i].unwrap().tier == tier)
                .collect();
            let d = b[on[0]].min - a[on[0]].min;
            assert!(d.x.abs() < 0.01, "tier {tier} moved sideways by {}", d.x);
            for &i in &on {
                assert!(close(a[i].min + d, b[i].min), "card {i}");
            }
            if tier > 0 {
                assert!(d.y > 1.0, "tier {tier} lifts as the map explodes");
            }
        }
    }

    #[test]
    fn a_full_turn_comes_back() {
        let parts = tree();
        let at = |yaw| build(&parts, Camera { yaw, ..FULL });
        let (a, b) = (at(SNAP), at(SNAP + TAU));
        for i in 0..parts.len() {
            assert!(
                a.pos[i].unwrap().distance(b.pos[i].unwrap()) < 0.05,
                "card {i}"
            );
        }
        // Eight steps right are a full turn; a step left undoes one right.
        let mut yaw = 0.0;
        for _ in 0..8 {
            yaw = turned(yaw, 1.0);
        }
        assert!((yaw - TAU).abs() < 1e-4);
        assert!(
            (turned(turned(0.3, 1.0), -1.0)).abs() < 1e-4,
            "0.3 → 45° → 0°"
        );
        assert!((turned(0.3, -1.0)).abs() < 1e-4, "left of 0.3 is 0°");
    }

    /// Whether `front` hides the name of `back`: the top of the card, over
    /// most of its width.
    fn hides_name(front: Rect, back: Rect) -> bool {
        let name = Rect::from_min_size(back.min, vec2(back.width() * 0.6, back.height() * 0.3));
        front.contains_rect(name)
    }

    #[test]
    fn no_card_hides_the_name_of_one_behind_it_at_the_snaps() {
        // A grid of cards as close as any layout packs them.
        let mut parts = Vec::new();
        for row in 0..5 {
            for col in 0..6 {
                parts.push(Some(Part {
                    rect: card(
                        col as f32 * (CARD.x + GAP_X),
                        row as f32 * (CARD.y + STACK_GAP),
                    ),
                    tier: 0,
                    parent: None,
                    group: None,
                }));
            }
        }
        for k in 0..8 {
            let s = build(
                &parts,
                Camera {
                    yaw: k as f32 * SNAP,
                    ..FULL
                },
            );
            let r = rects(&parts, &s);
            for (n, &i) in s.order.iter().enumerate() {
                for &j in &s.order[n + 1..] {
                    assert!(!hides_name(r[j], r[i]), "turn {k}: card {j} over {i}");
                }
            }
        }
    }

    #[test]
    fn depth_order_is_stable() {
        let parts = tree();
        let a = build(&parts, FULL);
        let b = build(&parts, FULL);
        assert_eq!(a.order, b.order);
        // Lowest plate first; on a plate, the farthest card first.
        assert_eq!(parts[a.order[0]].unwrap().tier, 3);
        assert_eq!(*a.order.last().unwrap(), 0);
        let square = build(&parts, Camera { yaw: 0.0, ..FULL });
        // Square on, one row is all one depth: by index.
        assert_eq!(&square.order[..6], &[4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn lines_join_a_card_to_its_uplink_by_way_of_its_plate() {
        let parts = tree();
        for k in 0..8 {
            let s = build(
                &parts,
                Camera {
                    yaw: k as f32 * SNAP,
                    ..FULL
                },
            );
            let r = rects(&parts, &s);
            assert_eq!(s.lines.len(), 9, "every card but the internet");
            for l in &s.lines {
                let kid = l.kids[0];
                assert_eq!(parts[kid].unwrap().parent, Some(l.up));
                let (first, last) = (l.pts[0], *l.pts.last().unwrap());
                assert!(r[l.up].expand(0.5).contains(first), "starts on the uplink");
                assert!(r[kid].expand(0.5).contains(last), "ends on the card");
                // Between plates: straight down to the foot.
                if l.pts.len() == 3 {
                    assert!((l.pts[0].x - l.pts[1].x).abs() < 0.01, "turn {k}: upright");
                    assert!(l.pts[1].y > l.pts[0].y);
                }
            }
        }
    }

    #[test]
    fn lines_from_one_uplink_share_its_riser() {
        // Forty-eight clients on one switch: one way down, one foot.
        let mut parts = vec![Some(Part {
            rect: card(0.0, 0.0),
            tier: 0,
            parent: None,
            group: None,
        })];
        for k in 0..48 {
            parts.push(Some(Part {
                rect: card(
                    (k % 8) as f32 * 230.0 - 800.0,
                    300.0 + (k / 8) as f32 * 110.0,
                ),
                tier: 1,
                parent: Some(0),
                group: None,
            }));
        }
        let s = build(&parts, FULL);
        assert_eq!(s.feet.len(), 1);
        let risers: Vec<(Pos2, Pos2)> = s
            .lines
            .iter()
            .filter(|l| l.pts.len() == 3)
            .map(|l| (l.pts[0], l.pts[1]))
            .collect();
        assert!(risers.len() >= 40);
        assert!(
            risers
                .iter()
                .all(|r| close(r.0, risers[0].0) && close(r.1, risers[0].1))
        );
    }

    #[test]
    fn hidden_cards_project_nothing() {
        let mut parts = tree();
        parts[5] = None;
        let s = build(&parts, FULL);
        assert!(s.pos[5].is_none());
        assert!(!s.order.contains(&5));
        assert!(s.lines.iter().all(|l| l.kids != [5] && l.up != 5));
        assert!(s.plates.iter().all(|p| !p.cards.contains(&5)));
        // Nothing at all shown: nothing drawn, and no panic.
        let none = build(&[None, None], FULL);
        assert!(none.order.is_empty() && none.plates.is_empty());
        assert!(build(&[], FULL).pos.is_empty());
    }

    #[test]
    fn in_three_d_nearer_cards_are_bigger_and_drawn_over_farther_ones() {
        let parts = tree();
        let s = build(&parts, THREE);
        // Not every card one size any more, and none out of range.
        let (min, max) = s
            .scale
            .iter()
            .fold((f32::INFINITY, 0.0f32), |(a, b), &k| (a.min(k), b.max(k)));
        assert!(max > min + 0.05, "{min}..{max}");
        assert!(min >= SCALE_MIN && max <= SCALE_MAX);
        // Drawn farthest first: sizes never shrink along the order.
        let sizes: Vec<f32> = s.order.iter().map(|&i| s.scale[i]).collect();
        assert!(sizes.windows(2).all(|w| w[1] >= w[0] - 1e-4), "{sizes:?}");
        // The top plate is nearer the eye than the bottom one, looking
        // down: the internet's card is bigger than a client's behind it.
        assert!(s.scale[0] > s.scale[4]);
        // The exploded drawing is untouched: every card its own size.
        assert!(build(&parts, FULL).scale.iter().all(|&k| k == 1.0));
        assert!(build(&parts, FULL).ring.is_empty());
    }

    #[test]
    fn three_d_frames_the_same_however_it_is_turned() {
        let parts = tree();
        for pitch in [PITCH_MIN, 0.6, 1.0, PITCH_MAX] {
            let at = |yaw: f32| {
                build(
                    &parts,
                    Camera {
                        yaw,
                        pitch,
                        ..THREE
                    },
                )
            };
            let frame = at(0.0).frame.expect("3D has a frame");
            for k in 0..16 {
                let s = at(k as f32 * 0.4);
                // Fit doesn't breathe as the view turns...
                assert_eq!(s.frame, Some(frame), "pitch {pitch}, turn {k}");
                // ...and nothing is ever outside what it frames.
                for (i, r) in rects3(&parts, &s).into_iter().enumerate() {
                    assert!(
                        frame.expand(1.0).contains_rect(r),
                        "pitch {pitch} turn {k} card {i}: {r:?} in {frame:?}"
                    );
                }
            }
        }
        // On its way in or out, the frame is what's drawn.
        let half = build(
            &parts,
            Camera {
                depth: 0.5,
                ..THREE
            },
        );
        assert_eq!(half.frame, None);
    }

    /// Cards as drawn, each at its own size.
    fn rects3(parts: &[Option<Part>], s: &Scene) -> Vec<Rect> {
        (0..parts.len())
            .map(|i| {
                Rect::from_min_size(
                    s.pos[i].unwrap(),
                    parts[i].unwrap().rect.size() * s.scale[i],
                )
            })
            .collect()
    }

    #[test]
    fn looking_from_higher_up_shows_more_of_each_plate() {
        let parts = tree();
        let depth_of_clients = |pitch: f32| {
            let s = build(
                &parts,
                Camera {
                    yaw: 0.0,
                    pitch,
                    ..THREE
                },
            );
            let plate = s.plates.iter().find(|p| p.tier == 3).unwrap();
            let ys = plate.quad.map(|q| q.y);
            ys.iter().copied().fold(f32::NEG_INFINITY, f32::max)
                - ys.iter().copied().fold(f32::INFINITY, f32::min)
        };
        let (low, mid, high) = (
            depth_of_clients(PITCH_MIN),
            depth_of_clients(0.6),
            depth_of_clients(PITCH_MAX),
        );
        assert!(low < mid && mid < high, "{low} {mid} {high}");
        // Out of range is brought into it, not drawn inside out.
        let odd = |pitch: f32| build(&parts, Camera { pitch, ..THREE });
        assert_eq!(odd(-3.0).pos, odd(PITCH_MIN).pos);
        assert_eq!(odd(9.0).pos, odd(PITCH_MAX).pos);
        assert!(odd(f32::NAN).pos.iter().flatten().all(|p| p.is_finite()));
    }

    #[test]
    fn three_d_rules_each_plate_and_draws_lines_to_scaled_cards() {
        let parts = tree();
        let s = build(&parts, THREE);
        assert_eq!(s.ring.len(), 65);
        for plate in &s.plates {
            assert!(plate.grid.len() <= 2 * (GRID_MAX as usize + 1));
            let around = Rect::from_points(&plate.quad).expand(1.0);
            for l in &plate.grid {
                assert!(around.contains(l[0]) && around.contains(l[1]));
            }
        }
        assert!(s.plates.iter().any(|p| !p.grid.is_empty()));
        // Lines still start on the uplink and end on the card, at the
        // sizes they're drawn.
        let r = rects3(&parts, &s);
        assert_eq!(s.lines.len(), 9);
        for l in &s.lines {
            assert!(r[l.up].expand(0.5).contains(l.pts[0]));
            assert!(r[l.kids[0]].expand(0.5).contains(*l.pts.last().unwrap()));
        }
    }

    #[test]
    fn links_that_loop_or_lead_nowhere_are_drawn_without_harm() {
        // Each card the other's uplink, one its own, one hanging off a
        // card that isn't there, one off a hidden card.
        let mut parts = tree();
        parts[2].as_mut().unwrap().parent = Some(3);
        parts[3].as_mut().unwrap().parent = Some(2);
        parts[4].as_mut().unwrap().parent = Some(4);
        parts[5].as_mut().unwrap().parent = Some(99);
        parts[6].as_mut().unwrap().parent = Some(7);
        parts[7] = None;
        let shown: Vec<bool> = parts.iter().map(Option::is_some).collect();
        let parent: Vec<Option<usize>> = parts.iter().map(|p| p.and_then(|p| p.parent)).collect();
        let groups = assemblies(&parent, &shown);
        for (p, g) in parts.iter_mut().zip(groups) {
            if let Some(p) = p {
                p.group = g;
            }
        }
        let s = build(&parts, Camera { split: 1.0, ..FULL });
        assert_eq!(s.order.len(), 9);
        assert!(s.lines.iter().all(|l| l.kids[0] != l.up));
        assert!(s.pos.iter().flatten().all(|p| p.is_finite()));
    }

    #[test]
    fn the_tree_forks_into_its_sub_assemblies() {
        // 0 internet – 1 gateway – {2 switch – {4, 5}, 3 switch – {6}, 7 printer}
        let parent = [
            None,
            Some(0),
            Some(1),
            Some(1),
            Some(2),
            Some(2),
            Some(3),
            Some(1),
        ];
        let g = assemblies(&parent, &[true; 8]);
        assert_eq!(
            g,
            [
                None,
                None,
                Some(0),
                Some(1),
                Some(0),
                Some(0),
                Some(1),
                None
            ]
        );
        // One branch only: nothing to pull apart.
        let chain = [None, Some(0), Some(1), Some(2)];
        assert_eq!(assemblies(&chain, &[true; 4]), [None; 4]);
        // Two sites under no common card: each is one.
        let two = [None, Some(0), None, Some(2)];
        assert_eq!(
            assemblies(&two, &[true; 4]),
            [Some(0), Some(0), Some(1), Some(1)]
        );
        // A loop in the links, a link to nowhere, a hidden uplink: no panic.
        let odd = [Some(1), Some(0), Some(9), Some(2)];
        assemblies(&odd, &[true, true, true, true]);
        assemblies(&odd, &[true, false, true, true]);
    }

    #[test]
    fn sub_assemblies_split_apart_without_crossing() {
        // Two switches whose clients share one grid (as Zones lays them
        // out): together they overlap; split, each is clear of the other.
        let mut parts = tree();
        for (k, i) in (4..10).enumerate() {
            let p = parts[i].as_mut().unwrap();
            p.group = Some(k % 2);
            p.parent = Some(2 + k % 2);
        }
        let span = |s: &Scene, g: usize| {
            let r = rects(&parts, s);
            (0..parts.len())
                .filter(|&i| parts[i].unwrap().group == Some(g))
                .fold(Rect::NOTHING, |a, i| a.union(r[i]))
        };
        let cam = |split| Camera {
            yaw: 0.0,
            split,
            ..FULL
        };
        let together = build(&parts, cam(0.0));
        assert!(
            span(&together, 0)
                .x_range()
                .intersects(span(&together, 1).x_range())
        );
        let apart = build(&parts, cam(1.0));
        let (a, b) = (span(&apart, 0), span(&apart, 1));
        assert!(a.right() < b.left(), "{a:?} {b:?}");
        // Each sub-assembly has plates of its own, and the cards above the
        // fork stay between them.
        assert_eq!(apart.plates.iter().filter(|p| p.tier == 3).count(), 2);
        let gw = rects(&parts, &apart)[1].center().x;
        assert!(a.center().x < gw && gw < b.center().x);
    }

    #[test]
    fn a_big_map_opens_in_good_time() {
        // The most cards a map is drawn with, every one a client of one
        // switch: built well inside a frame at any sensible frame rate.
        let mut parts = vec![Some(Part {
            rect: card(0.0, 0.0),
            tier: 0,
            parent: None,
            group: None,
        })];
        for k in 1..10_000usize {
            parts.push(Some(Part {
                rect: card((k % 100) as f32 * 224.0, (k / 100) as f32 * 106.0 + 200.0),
                tier: 1 + k % 3,
                parent: Some((k - 1) / 7),
                group: Some(k % 5),
            }));
        }
        let start = std::time::Instant::now();
        let s = build(&parts, Camera { split: 1.0, ..FULL });
        assert_eq!(s.order.len(), 10_000);
        let s = build(
            &parts,
            Camera {
                split: 1.0,
                ..THREE
            },
        );
        assert_eq!(s.order.len(), 10_000);
        assert!(start.elapsed().as_millis() < 500, "{:?}", start.elapsed());
    }
}
