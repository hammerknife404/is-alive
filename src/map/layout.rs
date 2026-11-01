//! Auto-layout, in /mapnetwork's modes:
//! - **Cluster**: a top-down tree with the internet at the top. Cards with
//!   nothing under them stack in columns beside their siblings (a trunk line
//!   down the left, a stub into each), so a switch with twenty APs stays
//!   compact instead of one very wide row.
//! - **Tree**: one row per depth, every card side by side (can get wide).
//! - **Zones**: one box per role, its cards in a grid inside.
//! - **Sites**: one box per site, each holding its part of the tree.

use super::{Map, Zone};
use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Cluster,
    Tree,
    Zones,
    Sites,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Sites, Mode::Cluster, Mode::Tree, Mode::Zones];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Cluster => "Cluster",
            Mode::Tree => "Tree",
            Mode::Zones => "Zones",
            Mode::Sites => "Sites",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Mode::Cluster => "A tree with end devices stacked in columns: compact and balanced",
            Mode::Tree => "One row per level, every card side by side (can get wide)",
            Mode::Zones => {
                "One box per role; links from one device into a box fold into one counted line"
            }
            Mode::Sites => {
                "One box per site, each with its part of the tree. Click a site's title to \
                 collapse it to one card"
            }
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Mode::Cluster => "cluster",
            Mode::Tree => "tree",
            Mode::Zones => "zones",
            Mode::Sites => "sites",
        }
    }

    pub fn from_key(k: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == k.trim())
    }
}

/// A box drawn around a group of cards (Zones and Sites modes).
#[derive(Clone, Debug)]
pub struct Group {
    pub rect: Rect,
    pub title: String,
    /// The role, in Zones mode (its colour and whether it's hidden).
    pub zone: Option<Zone>,
    pub count: usize,
    /// Its cards are laid out as a tree (Sites), so the links between them
    /// are drawn; in a grid (Zones) they'd cut across it.
    pub tree: bool,
}

/// Room inside a box: around the cards, and the title above them.
pub const BOX_PAD: f32 = 16.0;
pub const BOX_TITLE: f32 = 30.0;
const BOX_GAP: f32 = 40.0;

pub const CARD: Vec2 = vec2(200.0, 92.0);
pub const GAP_X: f32 = 24.0;
pub const GAP_Y: f32 = 56.0;
pub const STACK_GAP: f32 = 14.0;
/// Room left of a stacked column for its trunk line.
pub const INDENT: f32 = 24.0;

#[derive(Default)]
pub struct Layout {
    /// Top-left of each card; None for cards not shown.
    pub pos: Vec<Option<Pos2>>,
    /// Hangs in a stacked column (trunk and stub) rather than below a bus.
    pub stacked: Vec<bool>,
    /// In a row wrapped below the first (Cluster, a wide top level): its
    /// link runs down the trunk at this x to the gap above its row at this
    /// y, rather than through the rows above.
    pub trunk: Vec<Option<Trunk>>,
    pub size: Vec2,
    /// Boxes around groups of cards, and which box each card is in.
    pub boxes: Vec<Group>,
    pub group: Vec<Option<usize>>,
}

struct Block {
    w: f32,
    h: f32,
    items: Vec<(usize, Vec2, bool)>,
}

/// Lay out the cards `shown` allows in `mode`. Sites on a map without them
/// (only the internet, or nothing) lays out as Cluster.
pub fn layout_in(map: &Map, shown: &dyn Fn(usize) -> bool, mode: Mode) -> Layout {
    match mode {
        Mode::Cluster => tree(map, shown, true),
        Mode::Tree => tree(map, shown, false),
        Mode::Zones => grouped(map, shown, false, &|i| {
            let z = map.nodes[i].zone;
            (z != Zone::Internet).then(|| (zone_order(z) as usize, z.name().to_string(), Some(z)))
        }),
        Mode::Sites if !map.sites().is_empty() => {
            let sites = map.sites();
            grouped(map, shown, true, &|i| {
                let s = map.nodes[i].site.as_ref()?;
                let k = sites.iter().position(|x| x == s)?;
                Some((k, s.clone(), None))
            })
        }
        Mode::Sites => tree(map, shown, true),
    }
}

fn empty(n: usize) -> Layout {
    Layout {
        pos: vec![None; n],
        stacked: vec![false; n],
        trunk: vec![None; n],
        size: Vec2::ZERO,
        boxes: Vec::new(),
        group: vec![None; n],
    }
}

/// `stack`: leaves stack in columns (Cluster), or not (Tree).
fn tree(map: &Map, shown: &dyn Fn(usize) -> bool, stack: bool) -> Layout {
    let n = map.nodes.len();
    let mut out = empty(n);
    if n == 0 || !shown(0) {
        return out;
    }
    let kids = sorted_kids(map, shown);
    let mut seen = vec![false; n];
    let root = if stack {
        let (b, trunks) = place_wrapped(0, &kids, &mut seen);
        for (i, t) in trunks {
            out.trunk[i] = Some(t);
        }
        b
    } else {
        place(0, &kids, &mut seen, stack)
    };
    let mut place_block = |b: &Block, at: Vec2| {
        for &(i, p, s) in &b.items {
            out.pos[i] = Some((at + p).to_pos2());
            out.stacked[i] = s;
        }
    };
    place_block(&root, Vec2::ZERO);
    let (mut w, mut h) = (root.w, root.h);
    // Anything a loop kept from the internet goes in a row underneath.
    let (mut x, top) = (0.0, root.h + GAP_Y);
    for i in 0..n {
        if shown(i) && !seen[i] {
            let b = place(i, &kids, &mut seen, stack);
            place_block(&b, vec2(x, top));
            x += b.w + GAP_X;
            w = w.max(x - GAP_X);
            h = h.max(top + b.h);
        }
    }
    out.size = vec2(w, h);
    out
}

/// Each card's shown children, infrastructure first.
fn sorted_kids(map: &Map, shown: &dyn Fn(usize) -> bool) -> Vec<Vec<usize>> {
    let mut kids = map.children();
    for k in &mut kids {
        k.retain(|&c| shown(c));
        k.sort_by(|&a, &b| {
            let (x, y) = (&map.nodes[a], &map.nodes[b]);
            (zone_order(x.zone), x.client, &x.label).cmp(&(zone_order(y.zone), y.client, &y.label))
        });
    }
    kids
}

pub(super) fn zone_order(z: Zone) -> u8 {
    // Infrastructure first, then servers and devices, clients last.
    match z {
        Zone::Internet => 0,
        Zone::Gateway => 1,
        Zone::Unknown => 2,
        Zone::Switch => 3,
        Zone::Gear => 4,
        Zone::Wireless => 5,
        Zone::Server => 6,
        Zone::Device => 7,
        Zone::Client => 8,
    }
}

fn place(i: usize, kids: &[Vec<usize>], seen: &mut [bool], stack: bool) -> Block {
    seen[i] = true;
    let mine: Vec<usize> = kids[i].iter().copied().filter(|&c| !seen[c]).collect();
    if mine.is_empty() {
        return Block {
            w: CARD.x,
            h: CARD.y,
            items: vec![(i, Vec2::ZERO, false)],
        };
    }
    for &c in &mine {
        seen[c] = true;
    }
    let row = kid_row(&mine, kids, seen, stack);
    assemble(i, row)
}

/// The blocks under a card, left to right: its leaves (in columns when
/// `stack` and there are a few) and then its branches, each placed.
fn kid_row(mine: &[usize], kids: &[Vec<usize>], seen: &mut [bool], stack: bool) -> Vec<Block> {
    let (branches, leaves): (Vec<usize>, Vec<usize>) = mine
        .iter()
        .partition(|&&c| kids[c].iter().any(|&g| !seen[g]));
    let mut row: Vec<Block> = Vec::new();
    // A few leaves read better side by side; more stack into columns.
    if stack && leaves.len() >= 4 {
        let rows = (((leaves.len() as f32) * 2.4).sqrt().ceil() as usize).max(5);
        for col in leaves.chunks(rows) {
            let items = col
                .iter()
                .enumerate()
                .map(|(j, &c)| (c, vec2(INDENT, j as f32 * (CARD.y + STACK_GAP)), true))
                .collect();
            row.push(Block {
                w: INDENT + CARD.x,
                h: col.len() as f32 * (CARD.y + STACK_GAP) - STACK_GAP,
                items,
            });
        }
    } else {
        row.extend(leaves.iter().map(|&c| Block {
            w: CARD.x,
            h: CARD.y,
            items: vec![(c, Vec2::ZERO, false)],
        }));
    }
    for &c in &branches {
        seen[c] = false;
        row.push(place(c, kids, seen, stack));
    }
    row
}

/// A card centred over one row of blocks.
fn assemble(i: usize, row: Vec<Block>) -> Block {
    let row_w = row.iter().map(|b| b.w).sum::<f32>() + GAP_X * (row.len() - 1) as f32;
    let row_h = row.iter().map(|b| b.h).fold(0.0, f32::max);
    let w = row_w.max(CARD.x);
    let row_x = (w - row_w) / 2.0;
    let mut items = vec![(i, vec2((w - CARD.x) / 2.0, 0.0), false)];
    let mut x = row_x;
    for b in row {
        for (c, p, s) in b.items {
            items.push((c, vec2(x, CARD.y + GAP_Y) + p, s));
        }
        x += b.w + GAP_X;
    }
    Block {
        w,
        h: CARD.y + GAP_Y + row_h,
        items,
    }
}

/// Where a wrapped row's link runs: the trunk's x, and the y of the gap
/// above the row.
pub type Trunk = (f32, f32);

/// Wider than this (width over height) and the top level wraps.
const WRAP_ABOVE: f32 = 2.0;
/// The shape a wrapped map aims for: about a window's.
const WRAP_AIM: f32 = 1.6;

/// `place` for the top of a Cluster map. A top level too wide for its
/// height (many sites off the internet) wraps into rows, left-aligned
/// beside a trunk down the left edge; returns the trunk (x, y of the gap
/// above its row) for each card whose link takes it.
fn place_wrapped(i: usize, kids: &[Vec<usize>], seen: &mut [bool]) -> (Block, Vec<(usize, Trunk)>) {
    seen[i] = true;
    let mine: Vec<usize> = kids[i].iter().copied().filter(|&c| !seen[c]).collect();
    if mine.is_empty() {
        seen[i] = false;
        return (place(i, kids, seen, true), Vec::new());
    }
    for &c in &mine {
        seen[c] = true;
    }
    let row = kid_row(&mine, kids, seen, true);
    let rows = wrap_rows(&row);
    if rows.len() < 2 {
        return (assemble(i, row), Vec::new());
    }
    let channel = GAP_X;
    let trunk_x = channel / 2.0;
    let mut items = Vec::new();
    let mut trunks = Vec::new();
    let mut y = CARD.y + GAP_Y;
    let mut w = 0.0_f32;
    let mut row_ws = Vec::new();
    let mut blocks = row.into_iter();
    for (r, &count) in rows.iter().enumerate() {
        let mut x = channel;
        let mut row_h = 0.0_f32;
        for b in blocks.by_ref().take(count) {
            for (c, p, s) in b.items {
                if r > 0 && mine.contains(&c) {
                    trunks.push((c, (trunk_x, y - GAP_Y / 2.0)));
                }
                items.push((c, vec2(x, y) + p, s));
            }
            x += b.w + GAP_X;
            row_h = row_h.max(b.h);
        }
        row_ws.push(x - GAP_X - channel);
        w = w.max(x - GAP_X);
        y += row_h + GAP_Y;
    }
    // Over the middle of the first row, whose links drop straight to it.
    let root_x = channel + (row_ws[0] - CARD.x).max(0.0) / 2.0;
    items.insert(0, (i, vec2(root_x, 0.0), false));
    (
        Block {
            w,
            h: y - GAP_Y,
            items,
        },
        trunks,
    )
}

/// How many of `row`'s blocks go in each line: one line when the row isn't
/// much wider than tall, else the split whose shape is nearest `WRAP_AIM`.
fn wrap_rows(row: &[Block]) -> Vec<usize> {
    let shape = |lines: &[usize]| {
        let mut it = row.iter();
        let (mut w, mut h) = (0.0_f32, CARD.y);
        for &n in lines {
            let line: Vec<&Block> = it.by_ref().take(n).collect();
            let lw = line.iter().map(|b| b.w).sum::<f32>() + GAP_X * (n.max(1) - 1) as f32;
            w = w.max(lw);
            h += GAP_Y + line.iter().map(|b| b.h).fold(0.0, f32::max);
        }
        let w = if lines.len() > 1 { w + GAP_X } else { w };
        w / h
    };
    let one = vec![row.len()];
    if row.len() < 4 || shape(&one) <= WRAP_ABOVE {
        return one;
    }
    let total: f32 = row.iter().map(|b| b.w + GAP_X).sum();
    let widest = row.iter().map(|b| b.w).fold(0.0, f32::max);
    let score = |l: &[usize]| (shape(l) / WRAP_AIM).ln().abs();
    let mut best = one;
    for k in 2..=row.len() {
        let limit = (total / k as f32).max(widest);
        let mut lines = Vec::new();
        let (mut n, mut x) = (0usize, 0.0_f32);
        for b in row {
            if n > 0 && x + b.w > limit {
                lines.push(n);
                (n, x) = (0, 0.0);
            }
            n += 1;
            x += b.w + GAP_X;
        }
        lines.push(n);
        if score(&lines) < score(&best) {
            best = lines;
        }
    }
    best
}

/// One box per group `key` gives (order, title, zone), its cards in a grid
/// in tree order, or (`as_tree`) as the group's part of the tree, in
/// Cluster's style; cards with no group (the internet, devices at no site)
/// go in a row across the top. Boxes wrap into rows.
fn grouped(
    map: &Map,
    shown: &dyn Fn(usize) -> bool,
    as_tree: bool,
    key: &dyn Fn(usize) -> Option<(usize, String, Option<Zone>)>,
) -> Layout {
    let n = map.nodes.len();
    let mut out = empty(n);
    let depths: Vec<usize> = (0..n).map(|i| map.path(i).len()).collect();
    let sort = |v: &mut Vec<usize>| {
        v.sort_by(|&a, &b| {
            let (x, y) = (&map.nodes[a], &map.nodes[b]);
            (depths[a], zone_order(x.zone), x.client, &x.label).cmp(&(
                depths[b],
                zone_order(y.zone),
                y.client,
                &y.label,
            ))
        })
    };
    let mut loose: Vec<usize> = Vec::new();
    let mut groups: BTreeMap<(usize, String), (Option<Zone>, Vec<usize>)> = BTreeMap::new();
    for i in (0..n).filter(|&i| shown(i)) {
        match key(i) {
            Some((k, title, zone)) => groups.entry((k, title)).or_insert((zone, vec![])).1.push(i),
            None => loose.push(i),
        }
    }
    sort(&mut loose);

    // Each box's size, with its cards' places inside it.
    struct Boxed {
        w: f32,
        h: f32,
        title: String,
        zone: Option<Zone>,
        items: Vec<(usize, Vec2, bool)>,
    }
    let kids = sorted_kids(map, shown);
    let boxes: Vec<Boxed> = groups
        .into_iter()
        .map(|((_, title), (zone, mut v))| {
            sort(&mut v);
            if as_tree {
                let (w, h, items) = subtree(&kids, &v);
                // Room for the title, chevron and "· N inside" at 11 pt
                // caps, so a collapsed site's one card doesn't crop it.
                let title_w = 14.0 + 7.2 * (title.chars().count() + 12) as f32;
                let (inner, w) = (w, w.max(title_w));
                let x0 = BOX_PAD + (w - inner) / 2.0;
                return Boxed {
                    w: 2.0 * BOX_PAD + w,
                    h: BOX_TITLE + BOX_PAD + h,
                    title,
                    zone,
                    items: items
                        .into_iter()
                        .map(|(i, p, s)| (i, vec2(x0, BOX_TITLE) + p, s))
                        .collect(),
                };
            }
            let cols = ((v.len() as f32 * 1.5).sqrt().ceil() as usize).clamp(1, 6);
            let rows = v.len().div_ceil(cols);
            let items = v
                .iter()
                .enumerate()
                .map(|(j, &i)| {
                    let (c, r) = ((j % cols) as f32, (j / cols) as f32);
                    let at = vec2(
                        BOX_PAD + c * (CARD.x + GAP_X),
                        BOX_TITLE + r * (CARD.y + STACK_GAP),
                    );
                    (i, at, false)
                })
                .collect();
            Boxed {
                w: 2.0 * BOX_PAD + cols as f32 * CARD.x + (cols - 1) as f32 * GAP_X,
                h: BOX_TITLE
                    + BOX_PAD
                    + rows as f32 * CARD.y
                    + (rows.saturating_sub(1)) as f32 * STACK_GAP,
                title,
                zone,
                items,
            }
        })
        .collect();

    // Rows of boxes no wider than about a 3:2 page of them.
    let area: f32 = boxes.iter().map(|b| b.w * b.h).sum();
    let widest = boxes.iter().map(|b| b.w).fold(0.0, f32::max);
    let max_w = widest.max(area.sqrt() * 1.5);
    let top = if loose.is_empty() {
        0.0
    } else {
        CARD.y + GAP_Y + 20.0
    };
    let (mut x, mut y, mut row_h, mut w) = (0.0_f32, top, 0.0_f32, 0.0_f32);
    for b in boxes {
        if x > 0.0 && x + b.w > max_w {
            x = 0.0;
            y += row_h + GAP_Y + 20.0;
            row_h = 0.0;
        }
        let at = vec2(x, y);
        let g = out.boxes.len();
        for (i, p, s) in &b.items {
            out.pos[*i] = Some((at + *p).to_pos2());
            out.stacked[*i] = *s;
            out.group[*i] = Some(g);
        }
        out.boxes.push(Group {
            rect: Rect::from_min_size(at.to_pos2(), vec2(b.w, b.h)),
            title: b.title,
            zone: b.zone,
            count: b.items.len(),
            tree: as_tree,
        });
        x += b.w + BOX_GAP;
        row_h = row_h.max(b.h);
        w = w.max(x - BOX_GAP);
    }
    // The loose cards, centred across the top.
    let loose_w = loose.len() as f32 * (CARD.x + GAP_X) - GAP_X;
    w = w.max(loose_w);
    let mut lx = ((w - loose_w) / 2.0).max(0.0);
    for i in loose {
        out.pos[i] = Some(pos2(lx, 0.0));
        lx += CARD.x + GAP_X;
    }
    out.size = vec2(w, y + row_h);
    out
}

/// The cards `v` (in sort order) as their own part of the tree: each card
/// whose parent isn't among them starts a subtree, side by side. Returns
/// the width, height and (card, place, stacked).
fn subtree(kids: &[Vec<usize>], v: &[usize]) -> (f32, f32, Vec<(usize, Vec2, bool)>) {
    let n = kids.len();
    let mut member = vec![false; n];
    for &i in v {
        member[i] = true;
    }
    let inner: Vec<Vec<usize>> = kids
        .iter()
        .map(|k| k.iter().copied().filter(|&c| member[c]).collect())
        .collect();
    let mut has_parent = vec![false; n];
    for k in &inner {
        for &c in k {
            has_parent[c] = true;
        }
    }
    let mut seen = vec![false; n];
    let mut items = Vec::new();
    let (mut x, mut h) = (0.0_f32, 0.0_f32);
    // Roots first, then anything a loop kept from them.
    let roots = v.iter().filter(|&&i| member[i] && !has_parent[i]);
    let rest = v.iter().filter(|&&i| member[i] && has_parent[i]);
    for &i in roots.chain(rest) {
        if seen[i] {
            continue;
        }
        let b = place(i, &inner, &mut seen, true);
        for (c, p, s) in b.items {
            items.push((c, vec2(x, 0.0) + p, s));
        }
        x += b.w + GAP_X;
        h = h.max(b.h);
    }
    ((x - GAP_X).max(CARD.x), h.max(CARD.y), items)
}

/// Orthogonal route from parent to child, in world space. Branches drop to
/// a bus halfway down and into the child's top; stacked cards come off a
/// trunk left of their column into the card's left edge.
///
/// With a `trunk` (a row wrapped below the first), the link drops to the
/// bus, runs to the trunk on the left, down it to the gap above the
/// child's row, and along that gap into the child as it would from a bus.
pub fn route(parent: Pos2, child: Pos2, stacked: bool, trunk: Option<Trunk>) -> Vec<Pos2> {
    let from = pos2(parent.x + CARD.x / 2.0, parent.y + CARD.y);
    if let Some((tx, gap)) = trunk {
        let bus = from.y + GAP_Y / 2.0;
        let mut pts = vec![from, pos2(from.x, bus), pos2(tx, bus), pos2(tx, gap)];
        if stacked {
            let col = child.x - INDENT / 2.0;
            let mid = child.y + CARD.y / 2.0;
            pts.extend([pos2(col, gap), pos2(col, mid), pos2(child.x, mid)]);
        } else {
            let cx = child.x + CARD.x / 2.0;
            pts.extend([pos2(cx, gap), pos2(cx, child.y)]);
        }
        return pts;
    }
    if stacked {
        let trunk = child.x - INDENT / 2.0;
        let bus = from.y + (GAP_Y / 2.0).min(((child.y - from.y) / 2.0).max(0.0));
        let mid = child.y + CARD.y / 2.0;
        vec![
            from,
            pos2(from.x, bus),
            pos2(trunk, bus),
            pos2(trunk, mid),
            pos2(child.x, mid),
        ]
    } else {
        let to = pos2(child.x + CARD.x / 2.0, child.y);
        let bus = from.y + ((to.y - from.y) / 2.0).max(0.0);
        if (to.x - from.x).abs() < 0.5 {
            vec![from, to]
        } else {
            vec![from, pos2(from.x, bus), pos2(to.x, bus), to]
        }
    }
}

/// Route between two cards placed by hand, out of the side that faces the
/// other card: bottom to top when one is clearly below the other, top to
/// bottom when above, else side to side with the bend halfway across.
/// Cards on top of each other get a straight line between their middles.
pub fn route_free(parent: Pos2, child: Pos2) -> Vec<Pos2> {
    const CLEAR: f32 = 12.0;
    let p = Rect::from_min_size(parent, CARD);
    let c = Rect::from_min_size(child, CARD);
    let vertical = |from: Pos2, to: Pos2| {
        let mid = (from.y + to.y) / 2.0;
        if (to.x - from.x).abs() < 0.5 {
            vec![from, to]
        } else {
            vec![from, pos2(from.x, mid), pos2(to.x, mid), to]
        }
    };
    if c.top() >= p.bottom() + CLEAR {
        vertical(p.center_bottom(), c.center_top())
    } else if c.bottom() + CLEAR <= p.top() {
        vertical(p.center_top(), c.center_bottom())
    } else if c.left() >= p.right() + CLEAR || c.right() + CLEAR <= p.left() {
        let right = c.left() >= p.right();
        let (from, to) = if right {
            (p.right_center(), c.left_center())
        } else {
            (p.left_center(), c.right_center())
        };
        let mid = (from.x + to.x) / 2.0;
        if (to.y - from.y).abs() < 0.5 {
            vec![from, to]
        } else {
            vec![from, pos2(mid, from.y), pos2(mid, to.y), to]
        }
    } else {
        vec![p.center(), c.center()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{Link, Node};

    fn tree(parents: &[Option<usize>]) -> Map {
        Map {
            nodes: parents
                .iter()
                .enumerate()
                .map(|(i, p)| Node {
                    id: i.to_string(),
                    label: format!("n{i:02}"),
                    zone: if i == 0 { Zone::Internet } else { Zone::Switch },
                    up: p.map(|p| (p, Link::Wired, None)),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn overlaps(l: &Layout) -> bool {
        let r: Vec<_> = l
            .pos
            .iter()
            .flatten()
            .map(|p| eframe::egui::Rect::from_min_size(*p, CARD))
            .collect();
        (0..r.len()).any(|i| (i + 1..r.len()).any(|j| r[i].intersects(r[j].shrink(1.0))))
    }

    #[test]
    fn no_card_overlaps_and_parents_sit_above() {
        // internet → gw → core → 12 leaves + 2 branches with 3 and 6 leaves.
        let mut p = vec![None, Some(0), Some(1)];
        p.extend(std::iter::repeat_n(Some(2), 12));
        p.push(Some(2));
        let a = p.len() - 1;
        p.push(Some(2));
        let b = p.len() - 1;
        p.extend(std::iter::repeat_n(Some(a), 3));
        p.extend(std::iter::repeat_n(Some(b), 6));
        let m = tree(&p);
        let l = layout_in(&m, &|_| true, Mode::Cluster);
        assert!(l.pos.iter().all(Option::is_some));
        assert!(!overlaps(&l));
        for (i, n) in m.nodes.iter().enumerate() {
            if let Some(up) = n.parent() {
                assert!(l.pos[up].unwrap().y < l.pos[i].unwrap().y);
            }
        }
        // Twelve leaves stack into columns: the tree is far narrower than a row.
        assert!(l.size.x < 12.0 * (CARD.x + GAP_X));
        assert!(l.stacked[3]);
        assert!(!l.stacked[a]);
    }

    #[test]
    fn hidden_cards_take_no_room() {
        let m = tree(&[None, Some(0), Some(1), Some(1)]);
        let l = layout_in(&m, &|i| i != 3, Mode::Cluster);
        assert!(l.pos[3].is_none());
        assert_eq!(l.size.x, CARD.x);
    }

    #[test]
    fn tree_mode_never_stacks() {
        let mut p = vec![None, Some(0)];
        p.extend(std::iter::repeat_n(Some(1), 8));
        let m = tree(&p);
        let l = layout_in(&m, &|_| true, Mode::Tree);
        assert!(l.stacked.iter().all(|s| !s));
        assert!(!overlaps(&l));
        // All eight leaves in one row.
        let ys: std::collections::HashSet<i32> =
            (2..10).map(|i| l.pos[i].unwrap().y as i32).collect();
        assert_eq!(ys.len(), 1);
    }

    #[test]
    fn zones_box_every_card_but_the_internet() {
        let mut m = tree(&[None, Some(0), Some(1), Some(1), Some(2), Some(2)]);
        m.nodes[1].zone = Zone::Gateway;
        m.nodes[4].zone = Zone::Wireless;
        m.nodes[5].zone = Zone::Wireless;
        let l = layout_in(&m, &|_| true, Mode::Zones);
        assert_eq!(l.group[0], None);
        let titles: Vec<&str> = l.boxes.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, ["Gateway / firewall", "Switches", "Access points"]);
        assert_eq!(l.boxes.iter().map(|b| b.count).sum::<usize>(), 5);
        assert!(!overlaps(&l));
        for (i, g) in l.group.iter().enumerate() {
            if let Some(g) = g {
                let r = l.boxes[*g].rect;
                assert!(r.contains_rect(Rect::from_min_size(l.pos[i].unwrap(), CARD)));
            }
        }
        // No two boxes overlap, and the internet sits above them all.
        for (i, a) in l.boxes.iter().enumerate() {
            assert!(l.pos[0].unwrap().y + CARD.y < a.rect.top());
            for b in &l.boxes[i + 1..] {
                assert!(!a.rect.intersects(b.rect));
            }
        }
    }

    #[test]
    fn sites_mode_boxes_each_site_as_a_tree() {
        let mut m = tree(&[None, Some(0), Some(1), Some(0), Some(3)]);
        let none = layout_in(&m, &|_| true, Mode::Sites);
        assert!(none.boxes.is_empty());
        m.fill_sites("Here");
        let one = layout_in(&m, &|_| true, Mode::Sites);
        assert_eq!(one.boxes.len(), 1);
        assert_eq!(one.boxes[0].count, 4);
        assert!(one.boxes[0].tree);
        assert!(!overlaps(&one));
        // A parent sits above its child inside the box.
        assert!(one.pos[1].unwrap().y < one.pos[2].unwrap().y);
        for n in &mut m.nodes[1..] {
            n.site = None;
        }
        for (i, s) in [(1, "HQ"), (2, "HQ"), (3, "Branch"), (4, "Branch")] {
            m.nodes[i].site = Some(s.into());
        }
        let l = layout_in(&m, &|_| true, Mode::Sites);
        let titles: Vec<&str> = l.boxes.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, ["HQ", "Branch"]);
        assert_eq!(l.group[0], None);
        assert_eq!(l.group[2], l.group[1]);
        assert_ne!(l.group[3], l.group[1]);
    }

    #[test]
    fn a_wide_top_level_wraps_into_rows_off_a_trunk() {
        // The internet, 20 sites, two APs under each.
        let mut parents = vec![None];
        for s in 0..20 {
            parents.push(Some(0));
            let site = 1 + s * 3;
            parents.push(Some(site));
            parents.push(Some(site));
        }
        let m = tree(&parents);
        let l = layout_in(&m, &|_| true, Mode::Cluster);
        let shape = l.size.x / l.size.y;
        assert!((1.0..=WRAP_ABOVE).contains(&shape), "{shape}");
        // Tree stays one row per level.
        let t = layout_in(&m, &|_| true, Mode::Tree);
        assert!(t.size.x / t.size.y > WRAP_ABOVE);
        assert!(t.trunk.iter().all(Option::is_none));
        // Sites under the first row take the trunk; the first row and the
        // APs don't.
        let top = l.pos[1].unwrap().y;
        for (i, p) in l.pos.iter().enumerate() {
            let p = p.unwrap();
            let site = m.nodes[i].parent() == Some(0);
            assert_eq!(l.trunk[i].is_some(), site && p.y > top, "card {i}");
            if let Some((x, gap)) = l.trunk[i] {
                assert!(x < p.x && gap < p.y);
            }
        }
        // No two cards overlap.
        let rects: Vec<Rect> = l
            .pos
            .iter()
            .map(|p| Rect::from_min_size(p.unwrap(), CARD))
            .collect();
        for (a, ra) in rects.iter().enumerate() {
            for rb in &rects[a + 1..] {
                assert!(!ra.intersects(*rb));
            }
        }
    }

    #[test]
    fn a_narrow_top_level_does_not_wrap() {
        let m = tree(&[None, Some(0), Some(0), Some(1), Some(1), Some(2)]);
        let l = layout_in(&m, &|_| true, Mode::Cluster);
        assert!(l.trunk.iter().all(Option::is_none));
    }

    #[test]
    fn mode_keys_round_trip() {
        for m in Mode::ALL {
            assert_eq!(Mode::from_key(m.key()), Some(m));
        }
    }

    #[test]
    fn a_moved_card_is_joined_on_the_side_facing_it() {
        let p = pos2(0.0, 0.0);
        // Below: out of the bottom, into the top.
        let r = route_free(p, pos2(300.0, 300.0));
        assert_eq!(r[0], pos2(CARD.x / 2.0, CARD.y));
        assert_eq!(*r.last().unwrap(), pos2(300.0 + CARD.x / 2.0, 300.0));
        // Above: out of the top, into the bottom.
        let r = route_free(p, pos2(-50.0, -300.0));
        assert_eq!(r[0], pos2(CARD.x / 2.0, 0.0));
        assert_eq!(
            *r.last().unwrap(),
            pos2(-50.0 + CARD.x / 2.0, -300.0 + CARD.y)
        );
        // Beside, on the right: out of the right side, into the left.
        let r = route_free(p, pos2(CARD.x + 80.0, 20.0));
        assert_eq!(r[0], pos2(CARD.x, CARD.y / 2.0));
        assert_eq!(*r.last().unwrap(), pos2(CARD.x + 80.0, 20.0 + CARD.y / 2.0));
        // Beside, on the left.
        let r = route_free(p, pos2(-CARD.x - 80.0, 0.0));
        assert_eq!(r, [pos2(0.0, CARD.y / 2.0), pos2(-80.0, CARD.y / 2.0)]);
        for r in [
            route_free(p, pos2(300.0, 300.0)),
            route_free(p, pos2(260.0, 30.0)),
        ] {
            for s in r.windows(2) {
                assert!(s[0].x == s[1].x || s[0].y == s[1].y);
            }
        }
    }

    #[test]
    fn routes_are_orthogonal() {
        for stacked in [false, true] {
            for trunk in [None, Some((-12.0, 400.0))] {
                let r = route(pos2(0.0, 0.0), pos2(300.0, 428.0), stacked, trunk);
                for s in r.windows(2) {
                    assert!(s[0].x == s[1].x || s[0].y == s[1].y);
                }
            }
        }
    }
}
