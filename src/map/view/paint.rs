//! Drawing the map: zones, sites, links with their flow dots, and cards.

use super::*;
use std::cell::RefCell;
use std::collections::BTreeMap;

impl MapView {
    /// Draw the map; `hover` is the card under the pointer.
    pub(super) fn paint(&self, ui: &Ui, rect: Rect, hover: Option<usize>) {
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
        let hot_site = if hover.is_none() {
            self.site_title_at(rect, ui.input(|i| i.pointer.hover_pos()))
        } else {
            None
        };
        // Titles written above their boxes sit in the gap the links run
        // through: drawn after the links, on a patch of the canvas.
        let mut above = Vec::new();
        for b in &self.lay.boxes {
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
            p.rect(
                r,
                (10.0 * z) as u8,
                c.gamma_multiply(0.07 * a),
                Stroke::new(1.2, c.gamma_multiply(0.5 * a)),
                StrokeKind::Inside,
            );
            // A site's title folds it: brighter under the pointer.
            let hot = b.tree && hot_site.as_deref() == Some(b.title.as_str());
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
                    c.gamma_multiply(a),
                );
            }
        }

        // Links: the ones not lit as they always are; then the lit ones,
        // cut into pieces where they part (see `layout::pieces`), each in
        // the colour of the cards it leads to, with packets in those colours
        // flowing down them.
        let glow = glow_strength(vis);
        let behind = if vis.panel_fill.a() == 255 {
            vis.panel_fill
        } else {
            vis.extreme_bg_color
        };
        // Each colour as it's drawn on this canvas, and whether it's grey.
        let shade: RefCell<HashMap<Color32, (Color32, bool)>> = RefCell::default();
        let ink = |c: Color32| {
            *shade
                .borrow_mut()
                .entry(c)
                .or_insert_with(|| (legible(c, behind), chroma(c) < GREY))
        };
        let leg = |c: Color32| ink(c).0;
        let mut flows = false;
        // One stretch of line: its halo, then the stroke, solid, dashed
        // (wireless) or dotted (assumed). `s0` is how far along its line
        // it starts, so a line drawn in pieces dashes as one.
        let stroke =
            |pts: &[Pos2], s0: f32, c: Color32, halo: Option<Color32>, link: Link, w: f32| {
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
            };
        // A packet: a dot in its card's colour, ringed in the canvas's so it
        // shows on a line of its own colour; a grey card's is a hollow ring,
        // which no other colour's dot looks like.
        let packet = |at: Pos2, c: Color32, r: f32, alpha: f32| {
            let (lc, grey) = ink(c);
            p.circle_filled(at, r * 2.4, lc.gamma_multiply(0.22 * alpha));
            p.circle_filled(at, r + 1.0, behind.gamma_multiply(alpha));
            if grey {
                p.circle_stroke(at, r - 0.5, Stroke::new(1.5, lc.gamma_multiply(alpha)));
            } else {
                p.circle_filled(at, r, lc.gamma_multiply(alpha));
            }
        };
        let gap = 36.0 * z.max(0.6);
        let travel = now as f32 * 60.0 * z.max(0.6);
        let clip = rect.expand(12.0);
        let width = |link: Link, on: bool| {
            let base = if link == Link::Backbone { 3.2 } else { 2.0 };
            base * z.clamp(0.6, 1.4) + if on { 0.8 } else { 0.0 }
        };
        let colour = |n: &Node, link: Link| match link {
            Link::Backbone => zone_colour(Zone::Gateway),
            Link::Assumed => vis.weak_text_color(),
            _ => self.accent(n),
        };
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
        let lines = layout::lines(
            &self.map,
            &self.lay,
            &|i| self.pos.get(&self.map.nodes[i].id).copied(),
            &|i| self.placed.contains_key(&self.map.nodes[i].id),
        );
        let is_lit = |line: &layout::Line| line.kids.iter().any(|k| lit_links.contains(k));
        let screen =
            |w: &[Pos2]| -> Vec<Pos2> { w.iter().map(|w| self.to_screen(rect, *w)).collect() };
        for line in &lines {
            let on = is_lit(line);
            // Inside one box, links would cut across its grid: shown only
            // while lit.
            if line.quiet && !on {
                continue;
            }
            let Some(g) = line.into.and_then(|g| self.lay.boxes.get(g)) else {
                if on {
                    continue;
                }
                // The link's port or note is a chip on its card (see `card`).
                let k = line.kids[0];
                let link = link_of(k);
                let alpha = fade(k).min(fade(line.up));
                let base = colour(&self.map.nodes[k], link);
                let halo = (glow > 0.0 && link != Link::Assumed)
                    .then(|| base.gamma_multiply(glow.max(0.12) * alpha));
                let c = base.gamma_multiply(alpha * 0.85);
                stroke(&screen(&line.pts), 0.0, c, halo, link, width(link, false));
                continue;
            };
            // A trunk into a box carries its strongest link: one solid uplink
            // makes the shared line solid.
            let alpha = line
                .kids
                .iter()
                .map(|&k| fade(k).min(fade(line.up)))
                .fold(0.0, f32::max);
            let pts = screen(&line.pts);
            let first = line
                .kids
                .iter()
                .copied()
                .max_by_key(|&k| layout::strength(link_of(k)))
                .expect("a line carries a link");
            let link = link_of(first);
            let w = width(link, on) + 0.6;
            let boxed = g
                .zone
                .map_or_else(|| colour(&self.map.nodes[first], link), zone_colour);
            if on {
                // Lit: the colour of the lit cards it leads to if they share
                // one, else the box's.
                let colours = distinct(
                    line.kids
                        .iter()
                        .filter(|k| lit_links.contains(k))
                        .map(|&k| lit_colour(k)),
                );
                let (c, mixed) = path_colour(&colours, boxed);
                let lc = leg(c);
                let loud = if mixed { 1.0 } else { 1.6 };
                let halo = lc.gamma_multiply(glow.max(0.12) * alpha * loud);
                stroke(&pts, 0.0, lc.gamma_multiply(alpha), Some(halo), link, w);
                if self.animate {
                    flows = true;
                    let slots = packet_slots(&colours, boxed);
                    let period = slots.len() as f32 * gap;
                    let off = travel.rem_euclid(period);
                    let r = (w * 0.75).max(1.6);
                    for (slot, &sc) in slots.iter().enumerate() {
                        let phase = off + slot as f32 * gap;
                        for at in along_from(&pts, 0.0, phase, period, clip) {
                            packet(at, sc, r, alpha);
                        }
                    }
                }
            } else {
                let halo = (glow > 0.0 && link != Link::Assumed)
                    .then(|| boxed.gamma_multiply(glow.max(0.12) * alpha));
                stroke(&pts, 0.0, boxed.gamma_multiply(alpha * 0.85), halo, link, w);
            }
            if line.kids.len() > 1 && alpha > 0.3 {
                p.text(
                    *pts.last().expect("route has points") + vec2(5.0, -4.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("×{}", line.kids.len()),
                    FontId::proportional((10.0 * z).clamp(9.5, 12.0)),
                    vis.weak_text_color().gamma_multiply(alpha),
                );
            }
        }

        // The lit links, in pieces. Walked deepest first, so a column's
        // trunk is drawn as one line down to its last lit card and the
        // others add their own stubs.
        let lit: Vec<&layout::Line> = lines
            .iter()
            .filter(|l| l.into.is_none() && is_lit(l))
            .collect();
        let lens: Vec<f32> = lit.iter().map(|l| arc(&l.pts)).collect();
        let mut deep: Vec<usize> = (0..lit.len()).collect();
        deep.sort_by(|&a, &b| lens[b].total_cmp(&lens[a]));
        let (pieces, steps) = layout::pieces(&lit);
        // How each piece looks: the colour its lit cards share, else its
        // uplink's; the strongest of their links; the least faded of them.
        let looks: Vec<Look> = pieces
            .iter()
            .map(|pc| {
                let up = self.map.nodes[pc.feeds[0]].parent().unwrap_or(0);
                let colours = distinct(pc.feeds.iter().map(|&k| lit_colour(k)));
                let (colour, mixed) = path_colour(&colours, self.accent(&self.map.nodes[up]));
                let link = pc
                    .feeds
                    .iter()
                    .map(|&k| link_of(k))
                    .max_by_key(|&l| layout::strength(l))
                    .unwrap_or(Link::Wired);
                let alpha = pc
                    .feeds
                    .iter()
                    .map(|&k| fade(k).min(fade(up)))
                    .fold(0.0, f32::max);
                Look {
                    colour,
                    mixed,
                    link,
                    alpha,
                }
            })
            .collect();
        // Pieces that look alike, one after another along a line, are drawn
        // as one polyline so its corners join: (most lines along it, points,
        // how far along its line it starts, look).
        let mut runs: Vec<(usize, Vec<Pos2>, f32, Look)> = Vec::new();
        let mut drawn = vec![false; pieces.len()];
        for &li in &deep {
            let mut run: Option<(usize, Vec<Pos2>, f32, Look)> = None;
            for st in &steps[li] {
                if std::mem::replace(&mut drawn[st.piece], true) {
                    runs.extend(run.take());
                    continue;
                }
                let (look, fed) = (looks[st.piece], pieces[st.piece].feeds.len());
                match &mut run {
                    Some(r) if r.3 == look && r.1.last() == Some(&st.a) => {
                        r.1.push(st.b);
                        r.0 = r.0.max(fed);
                    }
                    _ => {
                        runs.extend(run.take());
                        run = Some((fed, vec![st.a, st.b], st.s, look));
                    }
                }
            }
            runs.extend(run);
        }
        // Shared stretches first, so the stubs leaving them sit on top.
        runs.sort_by_key(|r| std::cmp::Reverse(r.0));
        for (_, pts, s0, look) in &runs {
            let lc = leg(look.colour);
            let loud = if look.mixed { 1.0 } else { 1.6 };
            let halo = lc.gamma_multiply(glow.max(0.12) * look.alpha * loud);
            stroke(
                &screen(pts),
                s0 * z,
                lc.gamma_multiply(look.alpha),
                Some(halo),
                look.link,
                width(look.link, true),
            );
        }
        if self.animate && !lit.is_empty() {
            flows = true;
            let mut by_up: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
            for &li in &deep {
                by_up.entry(lit[li].up).or_default().push(li);
            }
            for (up, group) in by_up {
                // The colours in the order they branch off going down; each
                // has its slot in the packets' cycle.
                let parts = |li: usize| {
                    let own = steps[li].iter().find(|s| pieces[s.piece].feeds.len() == 1);
                    own.map_or(lens[li], |s| s.s)
                };
                let mut branches: Vec<(f32, Color32)> = group
                    .iter()
                    .map(|&li| (parts(li), lit_colour(lit[li].kids[0])))
                    .collect();
                branches.sort_by(|a, b| a.0.total_cmp(&b.0));
                let colours = distinct(branches.iter().map(|b| b.1));
                let slots = packet_slots(&colours, self.accent(&self.map.nodes[up]));
                let period = slots.len() as f32 * gap;
                let off = travel.rem_euclid(period);
                // Each colour's packets run down its deepest line, and down
                // the rest only from where they leave the ones before.
                let mut covered: Vec<HashSet<usize>> = vec![HashSet::new(); slots.len()];
                for li in group {
                    let k = lit[li].kids[0];
                    let c = lit_colour(k);
                    let slot = colours
                        .iter()
                        .position(|&x| x == c)
                        .unwrap_or(0)
                        .min(slots.len() - 1);
                    let Some(from) = steps[li].iter().find(|s| !covered[slot].contains(&s.piece))
                    else {
                        continue;
                    };
                    let from = from.s * z;
                    covered[slot].extend(steps[li].iter().map(|s| s.piece));
                    let r = (width(link_of(k), true) * 0.75).max(1.6);
                    let alpha = fade(k).min(fade(up));
                    let phase = off + slot as f32 * gap;
                    for at in along_from(&screen(&lit[li].pts), from, phase, period, clip) {
                        packet(at, slots[slot], r, alpha);
                    }
                }
            }
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
            let r = Rect::from_min_size(s, CARD * z);
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
        let dark = vis.dark_mode;
        let accent = self.accent(n);
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
        if n.gone {
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
            p.text(
                pos2(r.max.x - 12.0 * z - shift, r.min.y + 12.0 * z),
                egui::Align2::CENTER_CENTER,
                "📌",
                FontId::proportional((13.0 * z).max(9.0)),
                crate::theme::emoji_colour('📌', vis.strong_text_color()).gamma_multiply(a),
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
            if inner >= small.size * 0.55 * 5.0 && r.height() >= small.size * 1.3 {
                let g = text(&n.label, small, strong, 1, inner);
                let y = r.center().y - g.size().y / 2.0;
                p.galley(pos2(r.min.x + pad + 2.0 * z, y), g, strong);
            }
            if !n.flags.is_empty() {
                p.circle_filled(
                    pos2(r.max.x - 4.0, r.min.y + 4.0),
                    3.0,
                    widgets::warn().gamma_multiply(a),
                );
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
        let chips: Vec<&str> = uplink
            .into_iter()
            .chain(n.chips.iter().map(String::as_str))
            .filter(|c| !(boxed && n.site.as_deref() == Some(*c)))
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
                p.rect_filled(
                    chip,
                    (chip.height() / 2.0) as u8,
                    accent.gamma_multiply(0.16 * a),
                );
                p.galley(pos2(x + 5.0, chip.center().y - g.size().y / 2.0), g, accent);
                x += w + 4.0;
            }
        }
        if !full_badge {
            if !n.flags.is_empty() {
                p.circle_filled(
                    pos2(r.max.x - 4.0, r.min.y + 4.0),
                    3.0,
                    widgets::warn().gamma_multiply(a),
                );
            }
            return;
        }
        if !n.flags.is_empty() {
            let c = pos2(r.max.x - 12.0 * z, r.min.y + 12.0 * z);
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
    }
}

/// Height of one line of text in `font`, as laid out in a card.
fn ui_row_height(font: &FontId) -> f32 {
    font.size * 1.25
}

/// How a lit piece of line looks (see `paint`).
#[derive(Clone, Copy, PartialEq)]
struct Look {
    colour: Color32,
    /// Its cards' colours differ: the uplink's colour, its halo quieter.
    mixed: bool,
    link: Link,
    alpha: f32,
}

/// Packets of a colour with less chroma than this are hollow rings: the
/// internet's and unknown cards' greys, the clients' grey-blue, a white
/// client. A greyish dot on a coloured line is hard to tell from a faded
/// one; every theme's role colours are well above it.
const GREY: f32 = 0.08;

/// The colour a lit stretch of line takes from the colours of the cards it
/// leads to: theirs when they share one; else its uplink's, as a blend of
/// them would look like a card that isn't on the map. True when mixed.
pub(super) fn path_colour(cards: &[Color32], uplink: Color32) -> (Color32, bool) {
    match cards.split_first() {
        Some((&c, rest)) if rest.iter().all(|&x| x == c) => (c, false),
        Some(_) => (uplink, true),
        None => (uplink, false),
    }
}

/// The colour of each slot in the packets' cycle along the lines from one
/// uplink, for its cards' colours in the order they branch off: the first
/// four their own, any more sharing a fifth in the uplink's colour.
pub(super) fn packet_slots(colours: &[Color32], uplink: Color32) -> Vec<Color32> {
    let mut slots: Vec<Color32> = colours.iter().copied().take(4).collect();
    if colours.len() > 4 || slots.is_empty() {
        slots.push(uplink);
    }
    slots
}

/// The colours in order, each once.
fn distinct(colours: impl Iterator<Item = Color32>) -> Vec<Color32> {
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

/// `c` as a line or packet on the canvas `bg`: as it is, unless it's too
/// near the canvas's lightness to see; then lighter or darker, keeping its
/// hue and as much of its chroma as fits.
pub(super) fn legible(c: Color32, bg: Color32) -> Color32 {
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

/// How colourful `c` is (OKLab chroma): about 0 for greys, 0.1 to 0.3 for
/// the roles' colours.
pub(super) fn chroma(c: Color32) -> f32 {
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
    use crate::map::Map;

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

    #[test]
    fn each_piece_of_a_mixed_column_keeps_the_colour_it_leads_to() {
        let (m, l) = screenshot();
        let colour = |k: usize| zone_colour(m.nodes[k].zone);
        let gw = colour(1);
        let lines = layout::lines(&m, &l, &|k| l.pos[k], &|_| false);
        // The second card (.254, teal) and this machine (grey) lit.
        let lit: Vec<&layout::Line> = lines
            .iter()
            .filter(|x| x.kids == [3] || x.kids == [5])
            .collect();
        let (pieces, steps) = layout::pieces(&lit);
        let of =
            |pc: &layout::Piece| path_colour(&distinct(pc.feeds.iter().map(|&k| colour(k))), gw);
        let teal = colour(3);
        let grey = colour(5);
        // This machine's line: the gateway's colour (mixed) down to where
        // .254's stub leaves, then its own grey, stub and all.
        let mine = &steps[lit.iter().position(|x| x.kids == [5]).unwrap()];
        let seen: Vec<(Color32, bool)> = mine.iter().map(|s| of(&pieces[s.piece])).collect();
        let split = seen.iter().position(|c| !c.1).unwrap();
        assert!(split > 0);
        assert!(seen[..split].iter().all(|&c| c == (gw, true)));
        assert!(seen[split..].iter().all(|&c| c == (grey, false)));
        // .254's own stub is teal.
        let its = &steps[lit.iter().position(|x| x.kids == [3]).unwrap()];
        assert_eq!(of(&pieces[its.last().unwrap().piece]), (teal, false));
        // Lit alone, the whole of .254's line is teal (the 2.png case).
        let alone: Vec<&layout::Line> = lines.iter().filter(|x| x.kids == [3]).collect();
        let (pieces, _) = layout::pieces(&alone);
        assert!(pieces.iter().all(|pc| of(pc) == (teal, false)));
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
