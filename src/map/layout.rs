//! Auto-layout, in /mapnetwork's modes:
//! - **Cluster**: a top-down tree with the internet at the top. Cards with
//!   nothing under them stack in columns beside their siblings (a trunk line
//!   down the left, a stub into each), so a switch with twenty APs stays
//!   compact instead of one very wide row.
//! - **Tree**: one row per depth, every card side by side (can get wide).
//! - **Zones**: one box per role, its cards in a grid inside.
//! - **Sites**: one box per site, for maps that have sites.

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
            Mode::Sites => "One box per site",
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
/// lays out as Cluster.
pub fn layout_in(map: &Map, shown: &dyn Fn(usize) -> bool, mode: Mode) -> Layout {
    match mode {
        Mode::Cluster => tree(map, shown, true),
        Mode::Tree => tree(map, shown, false),
        Mode::Zones => grouped(map, shown, &|i| {
            let z = map.nodes[i].zone;
            (z != Zone::Internet).then(|| (zone_order(z) as usize, z.name().to_string(), Some(z)))
        }),
        Mode::Sites if map.sites().len() > 1 => {
            let sites = map.sites();
            grouped(map, shown, &|i| {
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
    let mut kids = map.children();
    for k in &mut kids {
        k.retain(|&c| shown(c));
        k.sort_by(|&a, &b| {
            let (x, y) = (&map.nodes[a], &map.nodes[b]);
            (zone_order(x.zone), x.client, &x.label).cmp(&(zone_order(y.zone), y.client, &y.label))
        });
    }
    let mut seen = vec![false; n];
    let root = place(0, &kids, &mut seen, stack);
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

fn zone_order(z: Zone) -> u8 {
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

/// One box per group `key` gives (order, title, zone), its cards in a grid
/// in tree order; cards with no group (the internet, devices at no site) go
/// in a row across the top. Boxes wrap into rows.
fn grouped(
    map: &Map,
    shown: &dyn Fn(usize) -> bool,
    key: &dyn Fn(usize) -> Option<(usize, String, Option<Zone>)>,
) -> Layout {
    let n = map.nodes.len();
    let mut out = empty(n);
    let depth = |i: usize| map.path(i).len();
    let sort = |v: &mut Vec<usize>| {
        v.sort_by(|&a, &b| {
            let (x, y) = (&map.nodes[a], &map.nodes[b]);
            (depth(a), zone_order(x.zone), x.client, &x.label).cmp(&(
                depth(b),
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
        items: Vec<(usize, Vec2)>,
    }
    let boxes: Vec<Boxed> = groups
        .into_iter()
        .map(|((_, title), (zone, mut v))| {
            sort(&mut v);
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
                    (i, at)
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
        for (i, p) in &b.items {
            out.pos[*i] = Some((at + *p).to_pos2());
            out.group[*i] = Some(g);
        }
        out.boxes.push(Group {
            rect: Rect::from_min_size(at.to_pos2(), vec2(b.w, b.h)),
            title: b.title,
            zone: b.zone,
            count: b.items.len(),
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

/// Orthogonal route from parent to child, in world space. Branches drop to
/// a bus halfway down and into the child's top; stacked cards come off a
/// trunk left of their column into the card's left edge.
pub fn route(parent: Pos2, child: Pos2, stacked: bool) -> Vec<Pos2> {
    let from = pos2(parent.x + CARD.x / 2.0, parent.y + CARD.y);
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
    fn sites_mode_needs_two_sites() {
        let mut m = tree(&[None, Some(0), Some(1), Some(0), Some(3)]);
        let one = layout_in(&m, &|_| true, Mode::Sites);
        assert!(one.boxes.is_empty());
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
    fn mode_keys_round_trip() {
        for m in Mode::ALL {
            assert_eq!(Mode::from_key(m.key()), Some(m));
        }
    }

    #[test]
    fn routes_are_orthogonal() {
        for stacked in [false, true] {
            let r = route(pos2(0.0, 0.0), pos2(300.0, 200.0), stacked);
            for s in r.windows(2) {
                assert!(s[0].x == s[1].x || s[0].y == s[1].y);
            }
        }
    }
}
