//! Auto-layout, in /mapnetwork's modes:
//! - **Cluster**: a top-down tree with the internet at the top. Cards with
//!   nothing under them stack in columns beside their siblings (a trunk line
//!   down the left, a stub into each), so a switch with twenty APs stays
//!   compact instead of one very wide row.
//! - **Tree**: one row per depth, every card side by side (can get wide).
//! - **Zones**: one box per role, its cards in a grid inside.
//! - **Sites**: one box per site, each holding its part of the tree.

use super::{Link, Map, Zone};
use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};
use std::collections::{BTreeMap, HashMap};

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

/// A line from a grid card in Zones box `bx` to a card placed outside it:
/// out of the side facing that card into the gap beside it, along the gap
/// and out of the box (its bottom, or its top for a card above it), then
/// round to the card. A straight line would cut across the grid's cards.
pub fn route_out(inner: Pos2, outer: Pos2, bx: Rect) -> Vec<Pos2> {
    let p = Rect::from_min_size(inner, CARD);
    let c = Rect::from_min_size(outer, CARD);
    let (from, x) = if c.center().x < p.center().x {
        (p.left_center(), p.left() - GAP_X / 2.0)
    } else {
        (p.right_center(), p.right() + GAP_X / 2.0)
    };
    let y = if c.bottom() < bx.top() {
        bx.top() - BOX_PAD / 2.0
    } else {
        bx.bottom() + BOX_PAD / 2.0
    };
    let mut pts = vec![from, pos2(x, from.y), pos2(x, y)];
    let cx = c.center().x;
    if c.top() > y {
        pts.extend([pos2(cx, y), pos2(cx, c.top())]);
    } else if c.bottom() < y {
        pts.extend([pos2(cx, y), pos2(cx, c.bottom())]);
    } else {
        pts.push(pos2(if cx < x { c.right() } else { c.left() }, y));
    }
    pts
}

/// The shown cards a route runs through, other than its two ends `up` and
/// `i` (a pixel in from their edges, so a line along a card's side is clear).
fn blockers(
    lay: &Layout,
    at: &dyn Fn(usize) -> Option<Pos2>,
    up: usize,
    i: usize,
    pts: &[Pos2],
) -> Vec<Rect> {
    let Some(bounds) = pts
        .iter()
        .map(|p| Rect::from_min_max(*p, *p))
        .reduce(|a, b| a.union(b))
    else {
        return Vec::new();
    };
    (0..lay.pos.len())
        .filter(|&k| k != up && k != i && lay.pos[k].is_some())
        .filter_map(|k| at(k).map(|p| Rect::from_min_size(p, CARD).shrink(1.0)))
        .filter(|r| r.intersects(bounds))
        .filter(|r| {
            pts.windows(2)
                .any(|s| r.intersects(Rect::from_two_pos(s[0], s[1])))
        })
        .collect()
}

/// A dragged card still about where its column has it (nudged sideways
/// less than half a card, and not over the column's trunk): its link stays
/// on the trunk, the stub reaching to where the card now is.
fn column_route(
    lay: &Layout,
    at: &dyn Fn(usize) -> Option<Pos2>,
    up: usize,
    i: usize,
) -> Option<Vec<Pos2>> {
    if !lay.stacked.get(i).copied().unwrap_or(false) {
        return None;
    }
    let (a, b, home) = (at(up)?, at(i)?, lay.pos.get(i).copied().flatten()?);
    let dx = b.x - home.x;
    if dx >= CARD.x / 2.0 || dx <= 4.0 - INDENT / 2.0 {
        return None;
    }
    let mut pts = route(
        a,
        pos2(home.x, b.y),
        true,
        lay.trunk.get(i).copied().flatten(),
    );
    pts.last_mut()?.x = b.x;
    Some(pts)
}

/// Round the cards in `block` instead of through them: from the side of
/// `p` facing `c`, out to a gutter at `g` beside the block (a column at x =
/// `g`, or with `across` a row at y = `g`), along it, and into the side of
/// `c` facing the gutter.
fn detour(p: Rect, c: Rect, block: Rect, g: f32, across: bool) -> Vec<Pos2> {
    // Worked out as a column; a row is the same thing with x and y swapped.
    let t = |q: Pos2| if across { pos2(q.y, q.x) } else { q };
    let tr = |r: Rect| Rect::from_two_pos(t(r.min), t(r.max));
    let (p, c, block) = (tr(p), tr(c), tr(block));
    let down = c.center().y >= p.center().y;
    let (from, near) = if down {
        (p.center_bottom(), block.top())
    } else {
        (p.center_top(), block.bottom())
    };
    let room = ((near - from.y).abs() / 2.0).clamp(6.0, GAP_Y / 2.0);
    let y0 = if down { from.y + room } else { from.y - room };
    let mut pts = vec![from, pos2(from.x, y0), pos2(g, y0)];
    if g < c.left() {
        pts.extend([pos2(g, c.center().y), c.left_center()]);
    } else if g > c.right() {
        pts.extend([pos2(g, c.center().y), c.right_center()]);
    } else if down {
        pts.push(pos2(g, c.top()));
    } else {
        pts.push(pos2(g, c.bottom()));
    }
    pts.dedup_by(|b, a| a.distance(*b) < 0.5);
    pts.into_iter().map(t).collect()
}

/// A route's length.
fn length(pts: &[Pos2]) -> f32 {
    pts.windows(2).map(|s| s[0].distance(s[1])).sum()
}

/// The line from `up` to `i` when either was dragged by hand. Out of a
/// Zones grid along its gaps (`route_out`) when one end sits in one;
/// otherwise the first of these that runs through no other card: on its
/// column's trunk (`column`, a card nudged out of its column), straight
/// across (`route_free`), or round the cards in the way through the gutter
/// beside them. When none is clear, the one through fewest cards.
fn route_placed(
    lay: &Layout,
    at: &dyn Fn(usize) -> Option<Pos2>,
    up: usize,
    i: usize,
    column: Option<Vec<Pos2>>,
) -> Vec<Pos2> {
    let (Some(a), Some(b)) = (at(up), at(i)) else {
        return Vec::new();
    };
    let direct = route_free(a, b);
    // The grid box card `k` sits inside, if any.
    let grid = |k: usize| {
        let g = lay.group.get(k).copied().flatten()?;
        let bx = lay.boxes.get(g).filter(|bx| !bx.tree)?;
        let r = Rect::from_min_size(at(k)?, CARD);
        bx.rect.contains_rect(r).then_some((g, bx.rect))
    };
    let crosses = |pts: &[Pos2], g: usize| {
        let cards: Vec<Rect> = (0..lay.group.len())
            .filter(|&k| k != up && k != i && lay.group[k] == Some(g))
            .filter_map(|k| at(k).map(|p| Rect::from_min_size(p, CARD).shrink(1.0)))
            .collect();
        pts.windows(2).any(|s| {
            let seg = Rect::from_two_pos(s[0], s[1]);
            cards.iter().any(|r| r.intersects(seg))
        })
    };
    let outside = |r: Rect, p: Pos2| !r.intersects(Rect::from_min_size(p, CARD));
    if let Some((g, r)) = grid(up)
        && outside(r, b)
        && crosses(&direct, g)
    {
        return route_out(a, b, r);
    }
    if let Some((g, r)) = grid(i)
        && outside(r, a)
        && crosses(&direct, g)
    {
        let mut pts = route_out(b, a, r);
        pts.reverse();
        return pts;
    }
    let blocked = |pts: &[Pos2]| blockers(lay, at, up, i, pts);
    let mut tried: Vec<(usize, f32, Vec<Pos2>)> = Vec::new();
    for pts in column.into_iter().chain([direct]) {
        let n = blocked(&pts).len();
        if n == 0 {
            return pts;
        }
        tried.push((n, length(&pts), pts));
    }
    // Round the cards the straight line runs through: the gutter half a gap
    // beside them, whichever side makes the shorter route.
    let block = blocked(&tried[tried.len() - 1].2)
        .into_iter()
        .reduce(|x, y| x.union(y))
        .map(|r| r.expand(1.0));
    if let Some(u) = block {
        let (p, c) = (Rect::from_min_size(a, CARD), Rect::from_min_size(b, CARD));
        let gutters = [
            (u.left() - GAP_X / 2.0, false),
            (u.right() + GAP_X / 2.0, false),
            (u.top() - STACK_GAP / 2.0, true),
            (u.bottom() + STACK_GAP / 2.0, true),
        ];
        let mut clear: Vec<Vec<Pos2>> = Vec::new();
        for (g, across) in gutters {
            let pts = detour(p, c, u, g, across);
            let n = blocked(&pts).len();
            if n == 0 {
                clear.push(pts);
            } else {
                tried.push((n, length(&pts), pts));
            }
        }
        if let Some(best) = clear
            .into_iter()
            .min_by(|x, y| length(x).total_cmp(&length(y)))
        {
            return best;
        }
    }
    tried
        .into_iter()
        .min_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)))
        .map(|t| t.2)
        .unwrap_or_default()
}

/// How solid a link is drawn, weakest first: the order links are drawn in.
pub fn strength(l: Link) -> u8 {
    match l {
        Link::Assumed => 0,
        Link::Wireless => 1,
        Link::Wired => 2,
        Link::Backbone => 3,
    }
}

/// One line on the map: a card's link to its uplink, or a trunk carrying
/// every link from one card into another box (Zones).
pub struct Line {
    pub up: usize,
    /// The cards whose links it carries; more than one is a trunk.
    pub kids: Vec<usize>,
    /// The box a trunk runs into.
    pub into: Option<usize>,
    /// Between two cards in one Zones box: shown only while lit, as it
    /// would cut across the box's grid.
    pub quiet: bool,
    /// The route, in world space.
    pub pts: Vec<Pos2>,
}

/// Every line on the map: links weakest first (cards stacked under one
/// uplink share its line, so a solid link drawn last covers the dashes of
/// an assumed one), then the trunks. `at` is where each card is drawn;
/// `moved` says it was dragged by hand, which joins it straight to its
/// uplink on the sides facing each other, out of any trunk (but never
/// through another card: see `route_placed`).
pub fn lines(
    map: &Map,
    lay: &Layout,
    at: &dyn Fn(usize) -> Option<Pos2>,
    moved: &dyn Fn(usize) -> bool,
) -> Vec<Line> {
    let group = |i: usize| lay.group.get(i).copied().flatten();
    let mut out = Vec::new();
    let mut trunks: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    let mut order: Vec<usize> = (0..map.nodes.len()).collect();
    order.sort_by_key(|&i| map.nodes[i].link().map_or(0, strength));
    for i in order {
        let Some(up) = map.nodes[i].parent() else {
            continue;
        };
        let free = moved(i) || moved(up);
        if let Some(g) = group(i)
            && group(up) != Some(g)
            && !free
        {
            trunks.entry((up, g)).or_default().push(i);
            continue;
        }
        let tree = group(i)
            .and_then(|g| lay.boxes.get(g))
            .is_some_and(|b| b.tree);
        let quiet = group(i).is_some() && group(i) == group(up) && !tree && !free;
        let (Some(a), Some(b)) = (at(up), at(i)) else {
            continue;
        };
        let pts = if free {
            let column = if moved(up) {
                None
            } else {
                column_route(lay, at, up, i)
            };
            route_placed(lay, at, up, i, column)
        } else {
            route(
                a,
                b,
                lay.stacked.get(i).copied().unwrap_or(false),
                lay.trunk.get(i).copied().flatten(),
            )
        };
        out.push(Line {
            up,
            kids: vec![i],
            into: None,
            quiet,
            pts,
        });
    }
    // Several trunks into one box side spread along it, in the order their
    // uplinks sit, so their lines and ×N counts don't land on one point.
    let mut ends: BTreeMap<(usize, bool), Vec<(f32, usize)>> = BTreeMap::new();
    for &(up, g) in trunks.keys() {
        let (Some(a), Some(b)) = (at(up), lay.boxes.get(g)) else {
            continue;
        };
        let below = b.rect.top() > a.y + CARD.y + 8.0;
        ends.entry((g, below)).or_default().push((a.x, up));
    }
    let mut entry: HashMap<(usize, usize), f32> = HashMap::new();
    for ((g, _), mut ups) in ends {
        let b = lay.boxes[g].rect;
        ups.sort_by(|x, y| x.0.total_cmp(&y.0));
        let n = ups.len() as f32;
        let gap = (b.width() / (n + 1.0)).min(CARD.x * 0.4);
        for (k, (_, up)) in ups.into_iter().enumerate() {
            let x = b.center().x + (k as f32 - (n - 1.0) / 2.0) * gap;
            entry.insert((up, g), x);
        }
    }
    for ((up, g), kids) in trunks {
        let (Some(a), Some(b)) = (at(up), lay.boxes.get(g)) else {
            continue;
        };
        let from = pos2(a.x + CARD.x / 2.0, a.y + CARD.y);
        let x = entry.get(&(up, g)).copied().unwrap_or(b.rect.center().x);
        let pts = if b.rect.top() > from.y + 8.0 {
            // Below: into the box's top edge.
            let into = pos2(x - CARD.x / 2.0, b.rect.top());
            route(a, into, false, None)
        } else {
            // Beside it (same row): under both boxes and up into its
            // bottom, rather than along the cards' edges.
            let own = group(up).and_then(|g| lay.boxes.get(g));
            let floor = own.map_or(from.y, |o| o.rect.bottom()).max(b.rect.bottom()) + BOX_PAD;
            vec![
                from,
                pos2(from.x, floor),
                pos2(x, floor),
                pos2(x, b.rect.bottom()),
            ]
        };
        out.push(Line {
            up,
            kids,
            into: Some(g),
            quiet: false,
            pts,
        });
    }
    out
}

/// A straight stretch of line between two points where lines from one
/// uplink join or part: a stacked column's trunk between two stubs, the
/// bus above a row, a stub.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub pts: [Pos2; 2],
    /// The cards whose lines run along it.
    pub feeds: Vec<usize>,
    /// How far along the first line through it it starts.
    pub s0: f32,
}

/// One piece as a line runs through it, in that line's direction: `s` is
/// how far along the line it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub piece: usize,
    pub s: f32,
    pub a: Pos2,
    pub b: Pos2,
}

/// The `lines` cut into pieces where they meet: each line is cut at its
/// own bends and wherever another line from the same uplink bends or ends
/// on it, and the cuts that coincide (to half a unit, either way round)
/// are one piece, fed by every line through it. Returns the pieces, and
/// each line's steps through them from its uplink down.
pub fn pieces(lines: &[&Line]) -> (Vec<Piece>, Vec<Vec<Step>>) {
    const NEAR: f32 = 0.5;
    let key = |p: Pos2| ((p.x * 2.0).round() as i64, (p.y * 2.0).round() as i64);
    // Every line's bends, by uplink.
    let mut bends: HashMap<usize, Vec<(usize, Pos2)>> = HashMap::new();
    for (li, line) in lines.iter().enumerate() {
        let v = bends.entry(line.up).or_default();
        v.extend(line.pts.iter().map(|&p| (li, p)));
    }
    let mut out: Vec<Piece> = Vec::new();
    // Pieces by uplink and their two ends, either way round.
    type End = (i64, i64);
    let mut found: HashMap<(usize, End, End), usize> = HashMap::new();
    let mut steps = Vec::with_capacity(lines.len());
    for (li, line) in lines.iter().enumerate() {
        let others = &bends[&line.up];
        let mut mine = Vec::new();
        let mut walked = 0.0;
        for seg in line.pts.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            let len = a.distance(b);
            if len < NEAR {
                walked += len;
                continue;
            }
            let dir = (b - a) / len;
            let mut cuts: Vec<(f32, Pos2)> = others
                .iter()
                .filter(|(o, _)| *o != li)
                .filter_map(|&(_, p)| {
                    let t = (p - a).dot(dir);
                    let on = (a + dir * t).distance(p) <= NEAR;
                    (on && t > NEAR && t < len - NEAR).then_some((t, p))
                })
                .collect();
            cuts.sort_by(|x, y| x.0.total_cmp(&y.0));
            cuts.push((len, b));
            let mut from = (0.0, a);
            for (t, p) in cuts {
                if t - from.0 < NEAR {
                    continue;
                }
                let (ka, kb) = (key(from.1), key(p));
                let k = (line.up, ka.min(kb), ka.max(kb));
                let piece = *found.entry(k).or_insert_with(|| {
                    out.push(Piece {
                        pts: [from.1, p],
                        feeds: Vec::new(),
                        s0: walked + from.0,
                    });
                    out.len() - 1
                });
                for &kid in &line.kids {
                    if !out[piece].feeds.contains(&kid) {
                        out[piece].feeds.push(kid);
                    }
                }
                mine.push(Step {
                    piece,
                    s: walked + from.0,
                    a: from.1,
                    b: p,
                });
                from = (t, p);
            }
            walked += len;
        }
        steps.push(mine);
    }
    (out, steps)
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
    fn a_dragged_card_keeps_its_link_out_of_the_zones_trunk() {
        // internet → gw → core → 3 clients: Zones puts them in boxes.
        let mut m = tree(&[None, Some(0), Some(1), Some(2), Some(2), Some(2)]);
        m.nodes[1].zone = Zone::Gateway;
        for n in &mut m.nodes[3..] {
            n.zone = Zone::Client;
        }
        let l = layout_in(&m, &|_| true, Mode::Zones);
        let at = |i: usize| l.pos[i];
        let lines = lines(&m, &l, &at, &|_| false);
        // core → clients: one trunk into the clients' box.
        let trunk = lines.iter().find(|x| x.up == 2 && x.into.is_some());
        assert_eq!(trunk.map(|t| t.kids.len()), Some(3));
        // Between the clients nothing; dragged, card 3 gets its own line,
        // joined on the sides facing its uplink.
        let moved = |i: usize| i == 3;
        let lines = super::lines(&m, &l, &at, &moved);
        let own = lines.iter().find(|x| x.kids == [3]).expect("its own line");
        assert!(own.into.is_none() && !own.quiet);
        assert_eq!(own.pts, route_free(at(2).unwrap(), at(3).unwrap()));
        let trunk = lines.iter().find(|x| x.up == 2 && x.into.is_some());
        assert_eq!(trunk.map(|t| t.kids.len()), Some(2));
    }

    #[test]
    fn trunks_into_one_box_enter_it_apart() {
        // internet → gw → 2 switches, each with 2 clients: two trunks into
        // the clients' box, from left to right as their switches sit.
        let mut m = tree(&[
            None,
            Some(0),
            Some(1),
            Some(1),
            Some(2),
            Some(2),
            Some(3),
            Some(3),
        ]);
        m.nodes[1].zone = Zone::Gateway;
        for n in &mut m.nodes[4..] {
            n.zone = Zone::Client;
        }
        let l = layout_in(&m, &|_| true, Mode::Zones);
        let lines = lines(&m, &l, &|i| l.pos[i], &|_| false);
        let end = |up: usize| {
            let t = lines.iter().find(|x| x.up == up && x.into.is_some());
            *t.expect("a trunk").pts.last().unwrap()
        };
        let (a, b) = (end(2), end(3));
        assert_ne!(a, b);
        assert_eq!(a.y, b.y);
        let left = l.pos[2].unwrap().x < l.pos[3].unwrap().x;
        assert_eq!(a.x < b.x, left);
        let bx = l.boxes[l.group[4].unwrap()].rect;
        assert!(bx.x_range().contains(a.x) && bx.x_range().contains(b.x));
    }

    #[test]
    fn a_card_dragged_out_of_a_zones_grid_is_joined_round_it() {
        // internet → gw → core → 6 switches, each with an AP.
        let mut p = vec![None, Some(0), Some(1)];
        p.extend(std::iter::repeat_n(Some(2), 6));
        p.extend((3..9).map(Some));
        let mut m = tree(&p);
        m.nodes[1].zone = Zone::Gateway;
        for n in &mut m.nodes[9..] {
            n.zone = Zone::Wireless;
        }
        let l = layout_in(&m, &|_| true, Mode::Zones);
        let g = l.group[3].unwrap();
        let bx = l.boxes[g].rect;
        // The AP under the grid's last switch, dragged left of the box.
        let (sw, ap) = (8, 14);
        assert_eq!(l.group[sw], Some(g));
        let left = pos2(bx.left() - CARD.x - 60.0, l.pos[sw].unwrap().y);
        let at = |k: usize| if k == ap { Some(left) } else { l.pos[k] };
        let moved = |k: usize| k == ap;
        let cards: Vec<Rect> = (3..9)
            .filter(|&k| k != sw)
            .map(|k| Rect::from_min_size(l.pos[k].unwrap(), CARD).shrink(1.0))
            .collect();
        let cuts = |pts: &[Pos2]| {
            pts.windows(2).any(|s| {
                let seg = Rect::from_two_pos(s[0], s[1]);
                cards.iter().any(|r| r.intersects(seg))
            })
        };
        // Straight across, it would cut through the switches beside it.
        assert!(cuts(&route_free(at(sw).unwrap(), left)));
        let lines = lines(&m, &l, &at, &moved);
        let own = lines.iter().find(|x| x.kids == [ap]).expect("its own line");
        assert!(!cuts(&own.pts), "{:?}", own.pts);
        assert_eq!(
            own.pts.first(),
            Some(&Rect::from_min_size(l.pos[sw].unwrap(), CARD).left_center())
        );
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

    /// internet → gateway → four cards stacked in one column (2..=5).
    fn column() -> (Map, Layout) {
        let mut m = tree(&[None, Some(0), Some(1), Some(1), Some(1), Some(1)]);
        m.nodes[1].zone = Zone::Gateway;
        let l = layout_in(&m, &|_| true, Mode::Cluster);
        assert!((2..6).all(|k| l.stacked[k]));
        (m, l)
    }

    /// Every segment of every line that runs into a card other than the
    /// two it joins: (card, line's cards).
    fn through_cards(
        lines: &[Line],
        at: &dyn Fn(usize) -> Option<Pos2>,
        n: usize,
    ) -> Vec<(usize, Vec<usize>)> {
        let mut hits = Vec::new();
        for line in lines {
            for k in (0..n).filter(|&k| k != line.up && !line.kids.contains(&k)) {
                let r = Rect::from_min_size(at(k).unwrap(), CARD).shrink(1.0);
                if line
                    .pts
                    .windows(2)
                    .any(|s| r.intersects(Rect::from_two_pos(s[0], s[1])))
                {
                    hits.push((k, line.kids.clone()));
                }
            }
        }
        hits
    }

    #[test]
    fn a_card_nudged_out_of_its_column_stays_on_the_trunk() {
        let (m, l) = column();
        let home = l.pos[3].unwrap();
        let nudged = home + vec2(8.0, 0.0);
        let at = |k: usize| if k == 3 { Some(nudged) } else { l.pos[k] };
        // Straight across it would run through the card above.
        assert!(!blockers(&l, &at, 1, 3, &route_free(at(1).unwrap(), nudged)).is_empty());
        let lines = lines(&m, &l, &at, &|k| k == 3);
        assert!(through_cards(&lines, &at, 6).is_empty());
        let own = lines.iter().find(|x| x.kids == [3]).unwrap();
        // The column's route, its stub reaching the card where it is now.
        let mut want = route(at(1).unwrap(), home, true, None);
        want.last_mut().unwrap().x = nudged.x;
        assert_eq!(own.pts, want);
        let trunk = home.x - INDENT / 2.0;
        assert_eq!(own.pts[own.pts.len() - 2].x, trunk);
        assert_eq!(*own.pts.last().unwrap(), nudged + vec2(0.0, CARD.y / 2.0));
    }

    #[test]
    fn a_card_dragged_beside_its_column_is_routed_round_the_cards() {
        let (m, l) = column();
        let n = m.nodes.len();
        for d in [vec2(150.0, 0.0), vec2(240.0, 30.0), vec2(-260.0, 10.0)] {
            let dragged = l.pos[3].unwrap() + d;
            let at = |k: usize| if k == 3 { Some(dragged) } else { l.pos[k] };
            assert!(!blockers(&l, &at, 1, 3, &route_free(at(1).unwrap(), dragged)).is_empty());
            let lines = lines(&m, &l, &at, &|k| k == 3);
            assert!(through_cards(&lines, &at, n).is_empty(), "{d:?}");
            let own = lines.iter().find(|x| x.kids == [3]).unwrap();
            for s in own.pts.windows(2) {
                assert!(s[0].x == s[1].x || s[0].y == s[1].y, "{:?}", own.pts);
            }
            // It ends on the dragged card's edge.
            let r = Rect::from_min_size(dragged, CARD);
            let end = *own.pts.last().unwrap();
            assert!(r.expand(0.01).contains(end) && !r.shrink(0.5).contains(end));
        }
    }

    #[test]
    fn a_stacked_column_cuts_into_pieces_where_its_stubs_leave() {
        let (m, l) = column();
        let all = lines(&m, &l, &|k| l.pos[k], &|_| false);
        let col: Vec<&Line> = all.iter().filter(|x| x.up == 1).collect();
        assert_eq!(col.len(), 4);
        let (pieces, steps) = pieces(&col);
        // Each line's steps join end to end, and each starts where the
        // last left off along it.
        for (line, st) in col.iter().zip(&steps) {
            assert_eq!(st[0].a, line.pts[0]);
            assert_eq!(st.last().unwrap().b, *line.pts.last().unwrap());
            for w in st.windows(2) {
                assert_eq!(w[0].b, w[1].a);
                assert!((w[0].s + w[0].a.distance(w[0].b) - w[1].s).abs() < 1e-3);
            }
        }
        // Down from the gateway: three pieces (drop, bus, trunk to the first
        // stub) carry all four, then the trunk carries 3, 2, 1; a stub each.
        let mut counts: Vec<usize> = pieces.iter().map(|p| p.feeds.len()).collect();
        counts.sort_unstable();
        assert_eq!(counts, [1, 1, 1, 1, 1, 2, 3, 4, 4, 4]);
        let deepest = steps.iter().max_by_key(|s| s.len()).unwrap();
        let fed: Vec<usize> = deepest
            .iter()
            .map(|s| pieces[s.piece].feeds.len())
            .collect();
        assert_eq!(fed, [4, 4, 4, 3, 2, 1, 1]);
        // A shared piece starts as far along every line through it.
        for st in &steps {
            for s in st {
                assert!((pieces[s.piece].s0 - s.s).abs() < 1e-3);
            }
        }
        // A stub is fed by its own card alone.
        for (line, st) in col.iter().zip(&steps) {
            assert_eq!(pieces[st.last().unwrap().piece].feeds, line.kids);
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
