//! Drawing the map: zones, sites, links with their flow dots, and cards.

use super::*;
use crate::map::colours;
use std::cell::RefCell;

impl MapView {
    /// Every line on the map as drawn now (see `layout::lines`).
    /// Card `i`'s link is routed on its own, round the cards in its way:
    /// the card was dragged by hand, or it's the way into a site box
    /// dragged elsewhere (its usual route would cut through what's now
    /// between them). A Zones box keeps its folded line, which follows it.
    pub(super) fn freed(&self, i: usize) -> bool {
        let n = &self.map.nodes[i];
        if self.placed.contains_key(&n.id) || self.shifted.is_empty() {
            return self.placed.contains_key(&n.id);
        }
        let group = |k: usize| self.lay.group.get(k).copied().flatten();
        let Some(g) = group(i) else { return false };
        let site = self.lay.boxes.get(g).is_some_and(|b| b.tree);
        let moved = self
            .lay
            .box_key(self.mode(), g)
            .is_some_and(|k| self.shifted.contains_key(&k));
        site && moved && n.parent().is_some_and(|p| group(p) != Some(g))
    }

    /// Lines the Links view leaves out are gone, but for a selected
    /// card's lit path.
    pub(super) fn lines_now(&self) -> Vec<layout::Line> {
        let all = layout::lines(
            &self.map,
            &self.lay,
            &|i| self.pos.get(&self.map.nodes[i].id).copied(),
            &|i| self.freed(i),
        );
        if self.links == Links::All {
            return all;
        }
        let below = layout::has_below(&self.map);
        let lit = self.lit(&self.selection()).1;
        all.into_iter()
            .filter(|l| {
                self.links.keeps(&self.map, l, &|k| below[k])
                    || l.kids.iter().any(|k| lit.contains(k))
            })
            .collect()
    }

    /// Draw the map with its `lines` (`lines_now`); `hover` is the card
    /// under the pointer.
    pub(super) fn paint(&self, ui: &Ui, rect: Rect, hover: Option<usize>, lines: &[layout::Line]) {
        let p = ui.painter_at(rect);
        let vis = ui.visuals();
        let z = self.zoom;
        let now = ui.input(|i| i.time);
        let focused = ui.input(|i| i.viewport().focused != Some(false));

        // On a see-through theme, calm the backdrop behind the map.
        if vis.panel_fill.a() < 255 {
            p.rect_filled(rect, 6, vis.extreme_bg_color.gamma_multiply(0.55));
        }
        // Dot grid that pans and zooms with the map.
        let step = 28.0 * z;
        if step > 14.0 {
            let dot = vis.weak_text_color().gamma_multiply(0.22);
            let o = rect.min + vec2(self.pan.x.rem_euclid(step), self.pan.y.rem_euclid(step));
            let mut y = o.y;
            let mut dots = Vec::new();
            while y < rect.max.y {
                let mut x = o.x;
                while x < rect.max.x {
                    dots.push(Shape::circle_filled(pos2(x, y), 1.0, dot));
                    x += step;
                }
                y += step;
            }
            p.extend(dots);
        }

        let sel = self.selection();
        let (lit, lit_links) = self.lit(&sel);
        let picked: HashSet<usize> = sel.iter().copied().collect();
        let fade = |i: usize| -> f32 {
            let n = &self.map.nodes[i];
            let mut a: f32 = 1.0;
            // A search miss on the selection's path stays in sight, dimmed
            // (and can be clicked, see `hit`).
            if self.faded(i, &lit) {
                a = 0.15;
            } else if !sel.is_empty() && (!lit.contains(&i) || !self.matches(n)) {
                a = 0.45;
            }
            let age = (now - self.born.get(&n.id).copied().unwrap_or(0.0)) as f32;
            a * (age / 0.35).clamp(0.0, 1.0)
        };
        let at = |i: usize| {
            self.pos
                .get(&self.map.nodes[i].id)
                .map(|w| self.to_screen(rect, *w))
        };

        // Boxes (Zones and Sites modes), under everything else.
        let hot_box = if hover.is_none() {
            self.box_title_at(rect, ui.input(|i| i.pointer.hover_pos()))
        } else {
            None
        };
        // Titles written above their boxes sit in the gap the links run
        // through: drawn after the links, on a patch of the canvas.
        let mut above = Vec::new();
        for (g, b) in self.lay.boxes.iter().enumerate() {
            let r = Rect::from_min_size(self.to_screen(rect, b.rect.min), b.rect.size() * z);
            if !r.intersects(rect) {
                continue;
            }
            let c = b.zone.map_or(vis.weak_text_color(), zone_colour);
            let a = if b.zone.is_some_and(|zn| self.hidden.contains(&zn)) {
                0.2
            } else {
                1.0
            };
            draw_box(&p, r, z, c, a);
            // Every card in it selected (its title clicked): the box is too.
            let cards = self.lay.in_box(g);
            if !cards.is_empty() && cards.iter().all(|i| picked.contains(i)) {
                p.rect_stroke(
                    r,
                    (10.0 * z) as u8,
                    Stroke::new(2.0, c.gamma_multiply(a)),
                    StrokeKind::Outside,
                );
            }
            // A title clicks (a site's folds it) and drags the box:
            // brighter under the pointer.
            let hot = hot_box == Some(g);
            if title_above(z) {
                // Too short a title strip to write in: the name sits just
                // above the box, small, in the gap between boxes.
                let title = if b.tree && self.map.closed.contains_key(&b.title) {
                    format!("{} · FOLDED", b.title.to_uppercase())
                } else {
                    b.title.to_uppercase()
                };
                let tc = if hot { vis.strong_text_color() } else { c };
                let mut job = LayoutJob::single_section(
                    title,
                    egui::TextFormat::simple(FontId::proportional(8.0), tc.gamma_multiply(a)),
                );
                job.wrap = egui::text::TextWrapping {
                    max_width: r.width().max(20.0),
                    max_rows: 1,
                    break_anywhere: true,
                    overflow_character: Some('…'),
                };
                let g = p.layout_job(job);
                let at = pos2(r.min.x + 2.0, r.min.y - g.size().y - 1.0);
                above.push((at, g, c));
            } else {
                let mid = r.min.y + layout::BOX_TITLE * z / 2.0;
                let mut x = r.min.x + layout::BOX_PAD * z;
                let mut title = format!("{} · {}", b.title.to_uppercase(), b.count);
                // A site's title folds it: a chevron says which way.
                if b.tree {
                    let closed = self.map.closed.get(&b.title);
                    let tc = if hot { vis.strong_text_color() } else { c };
                    chevron(&p, pos2(x + 5.0 * z, mid), 4.5 * z, closed.is_none(), tc);
                    x += 14.0 * z;
                    if let Some(v) = closed {
                        title = format!("{} · {} inside", b.title.to_uppercase(), v.len());
                    }
                }
                // Clipped to its box: zoomed out, the text stays legible
                // and so outgrows it.
                p.with_clip_rect(r.shrink(2.0).intersect(rect)).text(
                    pos2(x, mid),
                    egui::Align2::LEFT_CENTER,
                    title,
                    FontId::proportional((11.0 * z).max(9.5)),
                    if hot { vis.strong_text_color() } else { c }.gamma_multiply(a),
                );
            }
        }

        // Links: the ones not lit as they always are; then the lit ones,
        // cut into pieces where they part (see `layout::pieces`), each in
        // the colour of the cards it leads to, with packets in those colours
        // flowing down them.
        let glow = glow_strength(vis);
        let behind = canvas_fill(vis);
        // Each colour as it's drawn on this canvas, and whether it's grey.
        let shade: RefCell<HashMap<Color32, (Color32, bool)>> = RefCell::default();
        let ink = |c: Color32| {
            *shade
                .borrow_mut()
                .entry(c)
                .or_insert_with(|| colours::packet_look(c, behind))
        };
        let leg = |c: Color32| ink(c).0;
        let mut flows = false;
        // A packet in its card's colour (see `draw_packet`).
        let packet = |at: Pos2, c: Color32, r: f32, alpha: f32| {
            let (lc, grey) = ink(c);
            draw_packet(&p, at, lc, grey, r, alpha, behind);
        };
        let gap = packet_gap(z);
        let travel = packet_travel(now, z);
        let clip = rect.expand(12.0);
        let width = |link: Link, on: bool| link_width(link, z, on);
        let colour = |n: &Node, link: Link| link_colour(vis, link, self.accent(n));
        let link_of = |k: usize| self.map.nodes[k].link().unwrap_or(Link::Wired);
        // Lit, a link takes its card's colour (an assumed one too, rather
        // than grey), so the whole path reads as one.
        let lit_colour = |k: usize| {
            let n = &self.map.nodes[k];
            if n.link() == Some(Link::Backbone) {
                zone_colour(Zone::Gateway)
            } else {
                self.accent(n)
            }
        };
        let is_lit = |line: &layout::Line| line.kids.iter().any(|k| lit_links.contains(k));
        let screen =
            |w: &[Pos2]| -> Vec<Pos2> { w.iter().map(|w| self.to_screen(rect, *w)).collect() };
        // A trunk's ×N count, written over the lines once they're all drawn.
        let mut counts: Vec<(Pos2, usize, f32)> = Vec::new();
        for line in lines {
            let on = is_lit(line);
            // A trunk into a box carries its strongest link: one solid uplink
            // makes the shared line solid.
            if let Some(g) = line.into.and_then(|g| self.lay.boxes.get(g)) {
                let alpha = line
                    .kids
                    .iter()
                    .map(|&k| fade(k).min(fade(line.up)))
                    .fold(0.0, f32::max);
                let pts = screen(&line.pts);
                if line.kids.len() > 1 && alpha > 0.3 {
                    let end = *pts.last().expect("route has points");
                    counts.push((end, line.kids.len(), alpha));
                }
                // Lit, it's drawn in pieces with the rest (below).
                if on {
                    continue;
                }
                let first = line
                    .kids
                    .iter()
                    .copied()
                    .max_by_key(|&k| layout::strength(link_of(k)))
                    .expect("a line carries a link");
                let link = link_of(first);
                let w = width(link, false) + 0.6;
                let boxed = g
                    .zone
                    .map_or_else(|| colour(&self.map.nodes[first], link), zone_colour);
                draw_link(&p, &pts, boxed, link, w, alpha, z, glow);
                continue;
            }
            // Inside one box, links would cut across its grid: shown only
            // while lit.
            if on || line.quiet {
                continue;
            }
            // The link's port or note is a chip on its card (see `card`).
            let k = line.kids[0];
            let link = link_of(k);
            let alpha = fade(k).min(fade(line.up));
            let base = colour(&self.map.nodes[k], link);
            draw_link(
                &p,
                &screen(&line.pts),
                base,
                link,
                width(link, false),
                alpha,
                z,
                glow,
            );
        }

        // The lit links, trunks and all, in pieces (a trunk carrying only
        // its lit cards), each in the colour of the lit cards it leads to.
        let lit: Vec<layout::Line> = lines
            .iter()
            .filter(|l| is_lit(l))
            .map(|l| layout::Line {
                kids: l
                    .kids
                    .iter()
                    .copied()
                    .filter(|k| lit_links.contains(k))
                    .collect(),
                ..l.clone()
            })
            .collect();
        let uplink = |up: usize| self.accent(&self.map.nodes[up]);
        let boxed = |up: usize, g: usize| {
            let zone = self.lay.boxes.get(g).and_then(|b| b.zone);
            zone.map_or_else(|| uplink(up), zone_colour)
        };
        let alpha = |k: usize| {
            let up = self.map.nodes[k].parent().unwrap_or(k);
            fade(k).min(fade(up))
        };
        let ink = colours::Ink {
            card: &lit_colour,
            uplink: &uplink,
            boxed: &boxed,
            link: &link_of,
            alpha: &alpha,
        };
        let mut drawn = self.lit_drawn.borrow_mut();
        drawn.update(lit, &ink);
        let lit_width = |link: Link, trunk: bool| width(link, true) + if trunk { 0.6 } else { 0.0 };
        for run in &drawn.runs {
            let look = run.look;
            draw_lit(
                &p,
                &screen(&run.pts),
                run.s0 * z,
                leg(look.colour),
                look.mixed,
                look.link,
                lit_width(look.link, look.trunk),
                look.alpha,
                z,
                glow,
            );
        }
        if self.animate && !drawn.lines.is_empty() {
            flows = true;
            for pk in &drawn.packets {
                let line = &drawn.lines[pk.line];
                let period = pk.slots as f32 * gap;
                let phase = travel.rem_euclid(period) + pk.slot as f32 * gap;
                let r = packet_radius(lit_width(pk.link, line.into.is_some()));
                for at in along_from(&screen(&line.pts), pk.from * z, phase, period, clip) {
                    packet(at, pk.colour, r, pk.alpha);
                }
            }
        }
        drop(drawn);
        for (end, n, alpha) in counts {
            p.text(
                end + vec2(5.0, -4.0),
                egui::Align2::LEFT_BOTTOM,
                format!("×{n}"),
                trunk_font(z),
                vis.weak_text_color().gamma_multiply(alpha),
            );
        }

        for (at, g, c) in above {
            let back = Rect::from_min_size(at, g.size()).expand2(vec2(2.0, 0.5));
            p.rect_filled(back, 2, behind);
            p.with_clip_rect(rect).galley(at, g, c);
        }

        // Cards: infrastructure over clients, selected ones over the rest,
        // the ones dragged last on top. The cards an open menu acts on are
        // marked as if hovered.
        let menu: HashSet<usize> = self.menu_cards().into_iter().collect();
        let raised: HashSet<&str> = self.raised.iter().map(String::as_str).collect();
        let mut pulse = false;
        for i in self.stack() {
            let Some(s) = at(i) else { continue };
            let r = Rect::from_min_size(s, layout::card_size(&self.map.nodes[i]) * z);
            if !r.intersects(rect) {
                continue;
            }
            pulse |= !self.map.nodes[i].flags.is_empty();
            // A dimmed card dropped on others hides them, rather than
            // letting their text show through its own.
            if fade(i) < 1.0 && raised.contains(self.map.nodes[i].id.as_str()) {
                p.rect_filled(r, (7.0 * z) as u8, behind);
            }
            self.card(
                &p,
                vis,
                i,
                r,
                fade(i),
                picked.contains(&i),
                hover == Some(i) || menu.contains(&i),
                now,
            );
        }
        if focused && (flows || pulse) {
            // Flowing links at the display's rate; the warning pulse is slow
            // enough for 20 fps, and it's on screen as long as the tab is.
            if flows {
                ui.ctx().request_repaint_after(crate::backdrop::SMOOTH);
            } else {
                crate::backdrop::repaint_in(ui.ctx(), std::time::Duration::from_millis(50));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn card(
        &self,
        p: &Painter,
        vis: &egui::Visuals,
        i: usize,
        r: Rect,
        a: f32,
        selected: bool,
        hovered: bool,
        now: f64,
    ) {
        let n = &self.map.nodes[i];
        let z = self.zoom;
        let accent = self.accent(n);
        card_frame(p, vis, r, z, accent, a, selected, hovered, n.gone);
        let text = |s: &str, font: FontId, colour: Color32, rows: usize, width: f32| {
            let mut job =
                LayoutJob::single_section(s.to_string(), egui::TextFormat::simple(font, colour));
            job.wrap = egui::text::TextWrapping {
                max_width: width,
                max_rows: rows,
                break_anywhere: rows == 1,
                overflow_character: Some('…'),
            };
            p.layout_job(job)
        };
        let weak = vis.weak_text_color().gamma_multiply(a);
        let strong = vis.strong_text_color().gamma_multiply(a);
        // Pinned in the selection: the thumbtack, left of any warning badge.
        if self.is_kept(i) {
            let shift = if n.flags.is_empty() { 0.0 } else { 20.0 * z };
            pin(
                p,
                vis,
                pos2(r.max.x - 12.0 * z - shift, r.min.y + 12.0 * z),
                z,
                a,
            );
        }
        // Text never shrinks past a readable size: a card too small for all
        // of it drops lines instead, least useful first (kind, then chips,
        // then the address). The name always shows.
        let kind_f = FontId::monospace((9.5 * z).max(9.0));
        let label_f = FontId::proportional((13.0 * z).max(11.0));
        let sub_f = FontId::monospace((10.5 * z).max(10.0));
        let chip_f = FontId::proportional((9.5 * z).max(9.0));
        let pad = (12.0 * z).clamp(5.0, 14.0);
        let vpad = (7.0 * z).clamp(3.0, 9.0);
        let inner = r.width() - pad * 2.0;
        let room = r.height() - 2.0 * vpad;
        // Zoomed out past the readable size (a big site fitted to the
        // window): the name alone, smaller, so the overview still reads.
        // Too narrow even for that: a colour tile, no text ("Ha…" on every
        // card says nothing); hovering names it.
        if inner < label_f.size * 0.55 * 8.0 {
            let small = FontId::proportional((13.0 * z).clamp(8.0, 11.0));
            let pad = (12.0 * z).clamp(3.0, 5.0);
            let inner = r.width() - pad * 2.0 - 2.0 * z;
            // A client's tile is about half a card high: it still gets
            // its name while that fits at all.
            if inner >= small.size * 0.55 * 5.0 && r.height() >= small.size {
                let g = text(&n.label, small, strong, 1, inner);
                let y = r.center().y - g.size().y / 2.0;
                p.galley(pos2(r.min.x + pad + 2.0 * z, y), g, strong);
            }
            if !n.flags.is_empty() {
                warn_dot(p, r, a);
            }
            if !n.note.is_empty() {
                note_glyph(p, vis, note_rect(r, z), a);
            }
            return;
        }
        let full_badge = z >= 0.6;
        let badge = if !full_badge || n.flags.is_empty() {
            0.0
        } else {
            20.0 * z
        } + if self.is_kept(i) { 20.0 * z } else { 0.0 };
        let row = |f: &FontId| ui_row_height(f);
        // Two lines of name when the address still fits under them; else one.
        let sub_h = row(&sub_f);
        let has_sub = !n.sub.is_empty();
        let mut label = text(&n.label, label_f.clone(), strong, 2, inner - badge);
        if label.rows.len() > 1 && label.size().y + if has_sub { sub_h } else { 0.0 } > room {
            label = text(&n.label, label_f.clone(), strong, 1, inner - badge);
        }
        let mut need = label.size().y;
        let show_sub = has_sub && need + sub_h <= room;
        if show_sub {
            need += sub_h;
        }
        // The uplink's port or note ("port 3", "inferred") rides on the
        // card it leads to, not as tiny text at the end of the line.
        let uplink = n.up.as_ref().and_then(|u| u.2.as_deref());
        // In Sites the box already names the site.
        let boxed = self.mode() == Mode::Sites;
        // A +N chip on the card stands in for its count of clients.
        let counted = self.has_more_chip(i);
        // Drawn by hand (a plan, a training map): said first. The filters
        // skip it: a site called "drawn" mustn't make a drawn card look
        // discovered.
        let chips: Vec<&str> = super::super::drawn_chip(n)
            .into_iter()
            .chain(
                uplink
                    .into_iter()
                    .chain(n.chips.iter().map(String::as_str))
                    .filter(|c| !(boxed && n.site.as_deref() == Some(*c)))
                    .filter(|c| !(counted && super::super::is_count_chip(c))),
            )
            .collect();
        let chip_h = row(&chip_f) + 4.0;
        let show_chips = !chips.is_empty() && need + chip_h + 2.0 <= room;
        if show_chips {
            need += chip_h + 2.0;
        }
        let kind_h = row(&kind_f) + 1.0;
        let show_kind = !n.kind.is_empty() && need + kind_h <= room;
        if show_kind {
            need += kind_h;
        }
        // What shows sits centred in the space above the chips.
        let body = need - if show_chips { chip_h + 2.0 } else { 0.0 };
        let top_room = room - if show_chips { chip_h + 2.0 } else { 0.0 };
        let mut y = r.min.y + vpad + ((top_room - body) / 2.0).max(0.0);
        if show_kind {
            let g = text(&n.kind, kind_f, weak, 1, inner - badge);
            p.galley(pos2(r.min.x + pad, y), g, weak);
            y += kind_h;
        }
        let h = label.size().y;
        p.galley(pos2(r.min.x + pad, y), label, strong);
        y += h;
        if show_sub {
            let g = text(&n.sub, sub_f, accent.gamma_multiply(a), 1, inner);
            p.galley(pos2(r.min.x + pad, y), g, accent);
        }
        if show_chips {
            let mut x = r.min.x + pad;
            let cy = r.max.y - vpad;
            for c in chips {
                let g = text(c, chip_f.clone(), accent.gamma_multiply(a), 1, inner);
                let w = g.size().x + 10.0;
                if x + w > r.max.x - pad + 4.0 {
                    break;
                }
                let chip = Rect::from_min_max(pos2(x, cy - chip_h), pos2(x + w, cy));
                chip_pill(p, chip, accent, a);
                p.galley(pos2(x + 5.0, chip.center().y - g.size().y / 2.0), g, accent);
                x += w + 4.0;
            }
        }
        if !n.note.is_empty() {
            note_glyph(p, vis, note_rect(r, z), a);
        }
        if !full_badge {
            if !n.flags.is_empty() {
                warn_dot(p, r, a);
            }
            return;
        }
        if !n.flags.is_empty() {
            warn_badge(p, pos2(r.max.x - 12.0 * z, r.min.y + 12.0 * z), z, a, now);
        }
    }
}

/// Height of one line of text in `font`, as laid out in a card.
fn ui_row_height(font: &FontId) -> f32 {
    font.size * 1.25
}

// The pieces the map is drawn with, shared with the legend so its swatches
// are the map's own drawing.

/// A site's or a role's box at zoom `z`, in colour `c`, `a` its opacity.
pub(super) fn draw_box(p: &Painter, r: Rect, z: f32, c: Color32, a: f32) {
    p.rect(
        r,
        (10.0 * z) as u8,
        c.gamma_multiply(0.07 * a),
        Stroke::new(1.2, c.gamma_multiply(0.5 * a)),
        StrokeKind::Inside,
    );
}

/// How wide a link is drawn at zoom `z`: the internet's uplink wider, a
/// lit one a little more.
pub(super) fn link_width(link: Link, z: f32, on: bool) -> f32 {
    let base = if link == Link::Backbone { 3.2 } else { 2.0 };
    base * z.clamp(0.6, 1.4) + if on { 0.8 } else { 0.0 }
}

/// A link's colour: the internet's uplink in the gateway's, an assumed one
/// grey, the rest in their card's colour (`accent`).
pub(super) fn link_colour(vis: &egui::Visuals, link: Link, accent: Color32) -> Color32 {
    match link {
        Link::Backbone => zone_colour(Zone::Gateway),
        Link::Assumed => vis.weak_text_color(),
        _ => accent,
    }
}

/// The font of a trunk's ×count at zoom `z`.
pub(super) fn trunk_font(z: f32) -> FontId {
    FontId::proportional((10.0 * z).clamp(9.5, 12.0))
}

/// The canvas behind the map: lit lines and packets are kept legible on
/// it (`colours::legible`), and a packet is ringed in it.
pub(super) fn canvas_fill(vis: &egui::Visuals) -> Color32 {
    if vis.panel_fill.a() == 255 {
        vis.panel_fill
    } else {
        vis.extreme_bg_color
    }
}

/// One stretch of line at zoom `z`: its halo, then the stroke, solid,
/// dashed (wireless) or dotted (assumed). `s0` is how far along its line
/// it starts, so a line drawn in pieces dashes as one.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_stroke(
    p: &Painter,
    pts: &[Pos2],
    s0: f32,
    c: Color32,
    halo: Option<Color32>,
    link: Link,
    w: f32,
    z: f32,
) {
    if let Some(h) = halo {
        let hs = Stroke::new(w + 5.0 * z.clamp(0.6, 1.4), h);
        if link == Link::Wireless {
            p.extend(dashes(pts, s0, 7.0 * z, 5.0 * z, hs));
        } else {
            p.add(Shape::line(pts.to_vec(), hs));
        }
    }
    let s = Stroke::new(w, c);
    match link {
        Link::Wireless => p.extend(dashes(pts, s0, 7.0 * z, 5.0 * z, s)),
        // Whole pixels, at least 2 on and 3 off: zoomed out, sub-pixel
        // dots beat against the pixel grid and the line seems to break.
        Link::Assumed => p.extend(dashes(
            pts,
            s0,
            (2.5 * z).max(2.0).round(),
            (4.0 * z).max(3.0).round(),
            Stroke::new(w.max(1.5), c),
        )),
        _ => {
            p.add(Shape::line(pts.to_vec(), s));
        }
    }
}

/// A link that isn't lit, at zoom `z`: in `base` (see `link_colour`), a
/// little quieter, over a soft halo when the theme glows (none under the
/// grey assumed ones, which should stay quiet).
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_link(
    p: &Painter,
    pts: &[Pos2],
    base: Color32,
    link: Link,
    w: f32,
    alpha: f32,
    z: f32,
    glow: f32,
) {
    let halo =
        (glow > 0.0 && link != Link::Assumed).then(|| base.gamma_multiply(glow.max(0.12) * alpha));
    draw_stroke(
        p,
        pts,
        0.0,
        base.gamma_multiply(alpha * 0.85),
        halo,
        link,
        w,
        z,
    );
}

/// A stretch of lit line in `lc` (its colour made legible on the canvas,
/// see `colours::piece_looks`), over a halo: quieter where its cards'
/// colours are `mixed`. `s0` as in `draw_stroke`.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_lit(
    p: &Painter,
    pts: &[Pos2],
    s0: f32,
    lc: Color32,
    mixed: bool,
    link: Link,
    w: f32,
    alpha: f32,
    z: f32,
    glow: f32,
) {
    let loud = if mixed { 1.0 } else { 1.6 };
    let halo = lc.gamma_multiply(glow.max(0.12) * alpha * loud);
    draw_stroke(p, pts, s0, lc.gamma_multiply(alpha), Some(halo), link, w, z);
}

/// How far apart a lit line's packets run at zoom `z`.
pub(super) fn packet_gap(z: f32) -> f32 {
    36.0 * z.max(0.6)
}

/// How far the packets have travelled by `now`, at zoom `z`.
pub(super) fn packet_travel(now: f64, z: f32) -> f32 {
    now as f32 * 60.0 * z.max(0.6)
}

/// A packet's radius on a lit line `w` wide.
pub(super) fn packet_radius(w: f32) -> f32 {
    (w * 0.75).max(1.6)
}

/// A packet at `at`: a dot in its card's colour as the canvas shows it
/// (`colours::packet_look`), ringed in the canvas's (`behind`) so it shows
/// on a line of its own colour; a grey card's (`grey`) is a hollow ring,
/// which no other colour's dot looks like.
pub(super) fn draw_packet(
    p: &Painter,
    at: Pos2,
    lc: Color32,
    grey: bool,
    r: f32,
    alpha: f32,
    behind: Color32,
) {
    p.circle_filled(at, r * 2.4, lc.gamma_multiply(0.22 * alpha));
    p.circle_filled(at, r + 1.0, behind.gamma_multiply(alpha));
    if grey {
        p.circle_stroke(at, r - 0.5, Stroke::new(1.5, lc.gamma_multiply(alpha)));
    } else {
        p.circle_filled(at, r, lc.gamma_multiply(alpha));
    }
}

/// A card's body at zoom `z`: halo or shadow, fill tinted with its colour
/// (`accent`), border (dashed when `gone`) and the colour bar down its
/// left side. `a` is its opacity.
#[allow(clippy::too_many_arguments)]
pub(super) fn card_frame(
    p: &Painter,
    vis: &egui::Visuals,
    r: Rect,
    z: f32,
    accent: Color32,
    a: f32,
    selected: bool,
    hovered: bool,
    gone: bool,
) {
    let dark = vis.dark_mode;
    let round = (7.0 * z) as u8;
    let fill = if dark && vis.panel_fill.a() == 255 {
        vis.panel_fill.lerp_to_gamma(Color32::WHITE, 0.06)
    } else {
        vis.window_fill
    }
    // Tinted with its role's colour, so the map reads by colour at a glance.
    .lerp_to_gamma(accent, if dark { 0.13 } else { 0.07 });
    let glow = glow_strength(vis);
    if glow > 0.0 && !selected && !hovered {
        let halo = egui::Shadow {
            offset: [0, 0],
            blur: (12.0 * z) as u8,
            spread: 0,
            color: accent.gamma_multiply(glow * 0.8 * a),
        };
        p.add(halo.as_shape(r, round));
    }
    if selected || hovered {
        let shadow = egui::Shadow {
            offset: [0, (4.0 * z) as i8],
            blur: (14.0 * z) as u8,
            spread: 0,
            color: if selected {
                accent.gamma_multiply(0.35 * a)
            } else {
                Color32::from_black_alpha((60.0 * a) as u8)
            },
        };
        p.add(shadow.as_shape(r, round));
    }
    let border = if selected {
        Stroke::new(2.0, accent.gamma_multiply(a))
    } else if hovered {
        Stroke::new(1.5, accent.gamma_multiply(0.8 * a))
    } else {
        Stroke::new(1.0, accent.gamma_multiply(0.5 * a))
    };
    if gone {
        // Dashed: it isn't in the data any more.
        p.rect_filled(r, round, fill.gamma_multiply(a));
        let q = r.shrink(1.0);
        let outline = [
            q.left_top(),
            q.right_top(),
            q.right_bottom(),
            q.left_bottom(),
            q.left_top(),
        ];
        p.extend(Shape::dashed_line(
            &outline,
            Stroke::new(border.width.max(1.5), accent.gamma_multiply(a)),
            6.0 * z,
            4.0 * z,
        ));
    } else {
        p.rect(r, round, fill.gamma_multiply(a), border, StrokeKind::Inside);
    }
    let bar = Rect::from_min_max(r.min, pos2(r.min.x + 4.0 * z, r.max.y));
    p.rect_filled(
        bar,
        CornerRadius {
            nw: round,
            sw: round,
            ne: 0,
            se: 0,
        },
        accent.gamma_multiply(a),
    );
}

/// A chip's pill on a card (its text drawn over it).
pub(super) fn chip_pill(p: &Painter, chip: Rect, accent: Color32, a: f32) {
    p.rect_filled(
        chip,
        (chip.height() / 2.0) as u8,
        accent.gamma_multiply(0.16 * a),
    );
}

/// The thumbtack on a pinned card, centred at `at`.
pub(super) fn pin(p: &Painter, vis: &egui::Visuals, at: Pos2, z: f32, a: f32) {
    p.text(
        at,
        egui::Align2::CENTER_CENTER,
        "📌",
        FontId::proportional((13.0 * z).max(9.0)),
        crate::theme::emoji_colour('📌', vis.strong_text_color()).gamma_multiply(a),
    );
}

/// A card's warning badge centred at `c`: "!" on amber, with a ring
/// pulsing out of it.
pub(super) fn warn_badge(p: &Painter, c: Pos2, z: f32, a: f32, now: f64) {
    let warn = widgets::warn();
    let t = ((now * 0.8).fract()) as f32;
    p.circle_stroke(
        c,
        (7.0 + 7.0 * t) * z,
        Stroke::new(1.5, warn.gamma_multiply((1.0 - t) * 0.6 * a)),
    );
    p.circle_filled(c, 7.0 * z, warn.gamma_multiply(a));
    p.text(
        c,
        egui::Align2::CENTER_CENTER,
        "!",
        FontId::proportional(11.0 * z),
        Color32::BLACK.gamma_multiply(a),
    );
}

/// Where the note mark sits on card `r` at zoom `z`: on its top edge,
/// right of the middle (clear of the pin and the warning badge at the
/// right, the chips below and a link coming in at the top's middle),
/// never smaller than can be seen.
pub(super) fn note_rect(r: Rect, z: f32) -> Rect {
    let w = (12.0 * z).clamp(7.0, 14.0);
    let h = w * 1.25;
    Rect::from_min_size(
        pos2(r.min.x + r.width() * 0.6, r.min.y - h * 0.4),
        vec2(w, h),
    )
}

/// The note mark in `r`: a sheet with its top right corner folded, solid
/// in the text colour so it shows on any card and either theme, with two
/// lines of "writing" once it's big enough for them.
pub(super) fn note_glyph(p: &Painter, vis: &egui::Visuals, r: Rect, a: f32) {
    let ink = vis.strong_text_color().gamma_multiply(a);
    let paper = vis.window_fill.gamma_multiply(a);
    let f = r.width() * 0.38;
    let sheet = vec![
        r.left_top(),
        pos2(r.max.x - f, r.min.y),
        pos2(r.max.x, r.min.y + f),
        r.right_bottom(),
        r.left_bottom(),
    ];
    p.add(Shape::convex_polygon(sheet, ink, Stroke::new(1.0, paper)));
    let fold = vec![
        pos2(r.max.x - f, r.min.y),
        pos2(r.max.x - f, r.min.y + f),
        pos2(r.max.x, r.min.y + f),
    ];
    p.add(Shape::convex_polygon(fold, paper, Stroke::NONE));
    if r.height() >= 11.0 {
        let line = Stroke::new(1.0, paper);
        for k in [0.55, 0.78] {
            let y = r.min.y + r.height() * k;
            p.hline(r.min.x + 2.0..=r.max.x - 2.0, y, line);
        }
    }
}

/// A card too small for the badge: an amber dot in its corner.
pub(super) fn warn_dot(p: &Painter, r: Rect, a: f32) {
    p.circle_filled(
        pos2(r.max.x - 4.0, r.min.y + 4.0),
        3.0,
        widgets::warn().gamma_multiply(a),
    );
}

/// Dashes `on` long, `off` apart along a polyline, the pattern counted from
/// `s0` before its start: the pieces of one line dash as one. From 0, the
/// same dashes as `Shape::dashed_line`.
fn dashes(pts: &[Pos2], s0: f32, on: f32, off: f32, stroke: Stroke) -> Vec<Shape> {
    let period = on + off;
    if period <= 0.0 || on <= 0.0 {
        return vec![Shape::line(pts.to_vec(), stroke)];
    }
    let mut out = Vec::new();
    let mut start = s0.rem_euclid(period);
    for seg in pts.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let len = a.distance(b);
        if len <= 0.0 {
            continue;
        }
        let end = start + len;
        let at = |s: f32| a + (b - a) * ((s - start) / len);
        let mut d = (start / period).floor() * period;
        while d < end {
            let (lo, hi) = (d.max(start), (d + on).min(end));
            if hi > lo {
                out.push(Shape::line_segment([at(lo), at(hi)], stroke));
            }
            d += period;
        }
        start = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashes_run_on_across_the_pieces_of_a_line() {
        let s = Stroke::new(1.0, Color32::WHITE);
        let ends = |v: Vec<Shape>| -> Vec<[Pos2; 2]> {
            v.into_iter()
                .map(|x| match x {
                    Shape::LineSegment { points, .. } => points,
                    _ => panic!("a dash"),
                })
                .collect()
        };
        let line = [pos2(0.0, 0.0), pos2(0.0, 20.0), pos2(10.0, 20.0)];
        let whole = ends(dashes(&line, 0.0, 4.0, 6.0, s));
        let theirs = ends(Shape::dashed_line(&line, s, 4.0, 6.0));
        assert_eq!(whole.len(), theirs.len());
        for (a, b) in whole.iter().zip(&theirs) {
            assert!(a[0].distance(b[0]) < 1e-4 && a[1].distance(b[1]) < 1e-4);
        }
        // The same line in two pieces, the second told where it starts.
        let mut halves = ends(dashes(&line[..2], 0.0, 4.0, 6.0, s));
        halves.extend(ends(dashes(&line[1..], 20.0, 4.0, 6.0, s)));
        assert_eq!(halves, whole);
        // Cut mid-dash: the dash is finished, not restarted.
        let cut = pos2(0.0, 12.0);
        let mut parts = ends(dashes(&[line[0], cut], 0.0, 4.0, 6.0, s));
        parts.extend(ends(dashes(&[cut, line[1]], 12.0, 4.0, 6.0, s)));
        assert_eq!(parts[1], [pos2(0.0, 10.0), cut]);
        assert_eq!(parts[2], [cut, pos2(0.0, 14.0)]);
    }
}
