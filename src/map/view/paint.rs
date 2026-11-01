//! Drawing the map: zones, sites, links with their flow dots, and cards.

use super::*;

impl MapView {
    pub(super) fn paint(&self, ui: &Ui, rect: Rect) {
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
        let mut lit: HashSet<usize> = HashSet::new();
        for &s in &sel {
            lit.extend(self.map.path(s));
            lit.extend(
                (0..self.map.nodes.len()).filter(|&k| self.map.nodes[k].parent() == Some(s)),
            );
        }
        let fade = |i: usize| -> f32 {
            let n = &self.map.nodes[i];
            let mut a: f32 = 1.0;
            if self.hidden.contains(&n.zone) || !self.matches(n) {
                a = 0.15;
            } else if !sel.is_empty() && !lit.contains(&i) {
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
            if z >= 0.25 {
                let mid = r.min.y + layout::BOX_TITLE * z / 2.0;
                let mut x = r.min.x + layout::BOX_PAD * z;
                let mut title = format!("{} · {}", b.title.to_uppercase(), b.count);
                // A site's title folds it: a chevron says which way.
                if b.tree {
                    let closed = self.map.closed.get(&b.title);
                    let hot = self
                        .site_title_at(rect, ui.input(|i| i.pointer.hover_pos()))
                        .as_deref()
                        == Some(b.title.as_str());
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
                    FontId::proportional((11.0 * z).max(7.0)),
                    c.gamma_multiply(a),
                );
            }
        }

        // Links.
        let glow = glow_strength(vis);
        let mut flows = false;
        // One link's line: halo, stroke, the flow when lit, and a label at
        // its end. True while it flows.
        let draw = |pts: &[Pos2],
                    base: Color32,
                    lit_colour: Color32,
                    link: Link,
                    w: f32,
                    alpha: f32,
                    on: bool,
                    label: Option<(Pos2, egui::Align2, &str)>|
         -> bool {
            // Lit, an assumed link takes its card's colour rather than grey,
            // so the whole path reads as one.
            let base = if on && link == Link::Assumed {
                lit_colour
            } else {
                base
            };
            let c = base.gamma_multiply(alpha * if on { 1.0 } else { 0.85 });
            // A soft halo in the link's colour under it (not under the
            // grey assumed ones, which should stay quiet unless lit).
            if (glow > 0.0 || on) && (link != Link::Assumed || on) {
                let halo = Stroke::new(
                    w + 5.0 * z.clamp(0.6, 1.4),
                    base.gamma_multiply(glow.max(0.12) * alpha * if on { 1.6 } else { 1.0 }),
                );
                if link == Link::Wireless {
                    p.extend(Shape::dashed_line(pts, halo, 7.0 * z, 5.0 * z));
                } else {
                    p.add(Shape::line(pts.to_vec(), halo));
                }
            }
            let stroke = Stroke::new(w, c);
            match link {
                Link::Wireless => p.extend(Shape::dashed_line(pts, stroke, 7.0 * z, 5.0 * z)),
                Link::Assumed => p.extend(Shape::dashed_line(pts, stroke, 2.5 * z, 4.0 * z)),
                _ => {
                    p.add(Shape::line(pts.to_vec(), stroke));
                }
            }
            if on {
                // Packets flowing away from the internet: glowing dots in a
                // light tint of the link's colour, moving downstream.
                let gap = 36.0 * z.max(0.6);
                let off = (now as f32 * 60.0 * z.max(0.6)).rem_euclid(gap);
                let core = base
                    .lerp_to_gamma(Color32::WHITE, 0.6)
                    .gamma_multiply(alpha);
                let r = (w * 0.75).max(1.6);
                for at in along(pts, off, gap) {
                    p.circle_filled(at, r * 2.4, base.gamma_multiply(0.22 * alpha));
                    p.circle_filled(at, r, core);
                }
            }
            if let Some((pos, align, l)) = label
                && z >= 0.7
                && alpha > 0.3
            {
                p.text(
                    pos,
                    align,
                    l,
                    FontId::proportional(10.0 * z.min(1.2)),
                    vis.weak_text_color().gamma_multiply(alpha),
                );
            }
            on
        };
        let width = |link: Link, on: bool| {
            let base = if link == Link::Backbone { 3.2 } else { 2.0 };
            base * z.clamp(0.6, 1.4) + if on { 0.8 } else { 0.0 }
        };
        let colour = |n: &Node, link: Link| match link {
            Link::Backbone => zone_colour(Zone::Gateway),
            Link::Assumed => vis.weak_text_color(),
            _ => self.accent(n),
        };
        let group = |i: usize| self.lay.group.get(i).copied().flatten();
        // Links from one card into another box fold into one trunk:
        // (parent, box) → (children, any lit, brightest).
        let mut trunks: std::collections::BTreeMap<(usize, usize), (Vec<usize>, bool, f32)> =
            Default::default();
        for (i, n) in self.map.nodes.iter().enumerate() {
            let Some((up, link, label)) = &n.up else {
                continue;
            };
            let alpha = fade(i).min(fade(*up));
            let on = !sel.is_empty() && lit.contains(&i) && lit.contains(up);
            if let Some(g) = group(i)
                && group(*up) != Some(g)
            {
                let t = trunks.entry((*up, g)).or_default();
                t.0.push(i);
                t.1 |= on;
                t.2 = t.2.max(alpha);
                continue;
            }
            // Inside one box, links would cut across its grid: shown only
            // while one end is selected.
            // (Sites boxes hold a tree, so theirs are drawn.)
            let tree = group(i)
                .and_then(|g| self.lay.boxes.get(g))
                .is_some_and(|b| b.tree);
            if group(i).is_some() && group(i) == group(*up) && !on && !tree {
                continue;
            }
            let (Some(a), Some(b)) = (self.pos.get(&self.map.nodes[*up].id), self.pos.get(&n.id))
            else {
                continue;
            };
            let pts: Vec<Pos2> = layout::route(*a, *b, self.lay.stacked[i])
                .into_iter()
                .map(|w| self.to_screen(rect, w))
                .collect();
            let end = *pts.last().expect("route has points");
            // A stacked card's stub is short: just the number over it.
            let label = label.as_deref().map(|l| {
                if self.lay.stacked[i] {
                    (
                        end - vec2(layout::INDENT / 4.0 * z, 2.0),
                        egui::Align2::CENTER_BOTTOM,
                        l.trim_start_matches("port "),
                    )
                } else {
                    (end + vec2(4.0, -3.0), egui::Align2::LEFT_BOTTOM, l)
                }
            });
            flows |= draw(
                &pts,
                colour(n, *link),
                self.accent(n),
                *link,
                width(*link, on),
                alpha,
                on,
                label,
            );
        }
        for ((up, g), (kids, on, alpha)) in &trunks {
            let (Some(a), Some(b)) = (
                self.pos.get(&self.map.nodes[*up].id),
                self.lay.boxes.get(*g),
            ) else {
                continue;
            };
            let from = pos2(a.x + CARD.x / 2.0, a.y + CARD.y);
            let world: Vec<Pos2> = if b.rect.top() > from.y + 8.0 {
                // Below: into the middle of the box's top edge.
                let into = pos2(b.rect.center().x - CARD.x / 2.0, b.rect.top());
                layout::route(*a, into, false)
            } else {
                // Beside it (same row): under both boxes and up into its
                // bottom, rather than along the cards' edges.
                let own = group(*up).and_then(|g| self.lay.boxes.get(g));
                let floor =
                    own.map_or(from.y, |o| o.rect.bottom()).max(b.rect.bottom()) + layout::BOX_PAD;
                let x = b.rect.center().x;
                vec![
                    from,
                    pos2(from.x, floor),
                    pos2(x, floor),
                    pos2(x, b.rect.bottom()),
                ]
            };
            let pts: Vec<Pos2> = world.into_iter().map(|w| self.to_screen(rect, w)).collect();
            let first = &self.map.nodes[kids[0]];
            let link = first.link().unwrap_or(Link::Wired);
            let count = format!("×{}", kids.len());
            let end = *pts.last().expect("route has points");
            let label = (kids.len() > 1).then(|| {
                (
                    end + vec2(5.0, -4.0),
                    egui::Align2::LEFT_BOTTOM,
                    count.as_str(),
                )
            });
            let base = b.zone.map_or_else(|| colour(first, link), zone_colour);
            flows |= draw(
                &pts,
                base,
                self.accent(first),
                link,
                width(link, *on) + 0.6,
                *alpha,
                *on,
                label,
            );
        }

        // Cards: infrastructure over clients.
        let hover = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|h| rect.contains(*h))
            .and_then(|h| self.hit(self.to_world(rect, h)));
        let mut pulse = false;
        for i in 0..self.map.nodes.len() {
            let Some(s) = at(i) else { continue };
            let r = Rect::from_min_size(s, CARD * z);
            if !r.intersects(rect) {
                continue;
            }
            pulse |= !self.map.nodes[i].flags.is_empty();
            self.card(
                &p,
                vis,
                i,
                r,
                fade(i),
                sel.contains(&i),
                hover == Some(i),
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
        let pad = 12.0 * z;
        let inner = r.width() - pad * 2.0;
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
        // Zoomed out: just the name, at a size that can still be read.
        if z < 0.6 {
            if r.width() > 30.0 {
                let font = FontId::proportional((13.0 * z).max(8.5));
                let g = text(&n.label, font, strong, 2, r.width() - 10.0 * z.max(0.5));
                let at = pos2(
                    r.min.x + 6.0 * z.max(0.5) + 2.0,
                    r.center().y - g.size().y / 2.0,
                );
                p.galley(at, g, strong);
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
        let mut y = r.min.y + 7.0 * z;
        let badge = if n.flags.is_empty() { 0.0 } else { 20.0 * z }
            + if self.is_kept(i) { 20.0 * z } else { 0.0 };
        let g = text(&n.kind, FontId::monospace(9.5 * z), weak, 1, inner - badge);
        p.galley(pos2(r.min.x + pad, y), g.clone(), weak);
        y += g.size().y + 2.0 * z;
        let g = text(&n.label, FontId::proportional(13.0 * z), strong, 2, inner);
        p.galley(pos2(r.min.x + pad, y), g.clone(), strong);
        y += g.size().y + 1.0 * z;
        if !n.sub.is_empty() && y + 12.0 * z < r.max.y - 14.0 * z {
            let g = text(
                &n.sub,
                FontId::monospace(10.5 * z),
                accent.gamma_multiply(a),
                1,
                inner,
            );
            p.galley(pos2(r.min.x + pad, y), g, accent);
        }
        // Chips along the bottom, as many as fit.
        let mut x = r.min.x + pad;
        let cy = r.max.y - 7.0 * z;
        for c in &n.chips {
            let g = text(
                c,
                FontId::proportional(9.5 * z),
                accent.gamma_multiply(a),
                1,
                inner,
            );
            let w = g.size().x + 10.0 * z;
            if x + w > r.max.x - pad + 4.0 * z {
                break;
            }
            let chip = Rect::from_min_max(
                pos2(x, cy - g.size().y - 2.0 * z),
                pos2(x + w, cy + 1.0 * z),
            );
            p.rect_filled(
                chip,
                (chip.height() / 2.0) as u8,
                accent.gamma_multiply(0.16 * a),
            );
            p.galley(pos2(x + 5.0 * z, chip.min.y + 1.5 * z), g, accent);
            x += w + 4.0 * z;
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
