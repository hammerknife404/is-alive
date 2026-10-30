//! Auto-layout: a top-down tree with the internet at the top. Cards with
//! nothing under them stack in columns beside their siblings (a trunk line
//! down the left, a stub into each), so a switch with twenty APs stays
//! compact instead of one very wide row.

use super::{Map, Zone};
use eframe::egui::{Pos2, Vec2, pos2, vec2};

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
}

struct Block {
    w: f32,
    h: f32,
    items: Vec<(usize, Vec2, bool)>,
}

/// Lay out the cards `shown` allows, from the internet down.
pub fn layout(map: &Map, shown: &dyn Fn(usize) -> bool) -> Layout {
    let n = map.nodes.len();
    let mut out = Layout {
        pos: vec![None; n],
        stacked: vec![false; n],
        size: Vec2::ZERO,
    };
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
    let root = place(0, &kids, &mut seen);
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
            let b = place(i, &kids, &mut seen);
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

fn place(i: usize, kids: &[Vec<usize>], seen: &mut [bool]) -> Block {
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
    if leaves.len() >= 4 {
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
        row.push(place(c, kids, seen));
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
        let l = layout(&m, &|_| true);
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
        let l = layout(&m, &|i| i != 3);
        assert!(l.pos[3].is_none());
        assert_eq!(l.size.x, CARD.x);
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
