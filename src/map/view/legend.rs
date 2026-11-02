//! The legend floating on the canvas: what each colour, card, badge, line
//! and box on the map means, listing only what the map has now. Every
//! swatch is drawn with the map's own drawing code (see `paint`). A role's
//! row fades that role out or back in, as the sidebar's does.

use super::paint::{
    card_frame, chip_pill, draw_box, draw_link, link_width, pin, trunk_font, warn_badge,
};
use super::*;
use crate::map::legend::{self, Entry, Ink, Slot, Swatch};

/// One row, a section's heading or an entry's.
const ROW: f32 = 17.0;
/// The swatch beside each entry.
const SWATCH: Vec2 = vec2(30.0, 13.0);
/// Room between the swatch and the text, and between columns.
const GAP: f32 = 6.0;
const COL_GAP: f32 = 16.0;
/// Inside the panel's edge, and its title's height.
const PAD: f32 = 8.0;
const TITLE: f32 = 18.0;
/// The most of the canvas's height the legend takes.
const MAX_SHARE: f32 = 0.4;
/// Cards in a swatch are drawn as at this zoom: corners and bar to scale.
const CARD_Z: f32 = 0.5;

/// Where the legend goes and how its rows fall into columns.
struct Plan {
    panel: Rect,
    /// The rows' area (scrolls when the rows don't fit in it).
    body: Rect,
    /// Each item's top-left, from the top-left of the rows' area.
    at: Vec<(Slot, Pos2)>,
    content: Vec2,
    /// The rows don't all fit: a scroll bar shows.
    scrolls: bool,
    /// Each entry's label and count laid out.
    labels: Vec<(
        std::sync::Arc<egui::Galley>,
        Option<std::sync::Arc<egui::Galley>>,
    )>,
}

/// A legend colour as the map draws it.
fn ink(vis: &egui::Visuals, i: Ink) -> Color32 {
    match i {
        Ink::Role(z) => zone_colour(z),
        Ink::Gone => gone_colour(),
        Ink::Weak => vis.weak_text_color(),
        Ink::Warn => widgets::warn(),
    }
}

impl MapView {
    /// The legend's entries for the map as it's drawn now.
    pub(super) fn legend_entries(&self, lines: &[layout::Line]) -> Vec<Entry> {
        let pinned = self
            .kept
            .iter()
            .filter(|id| self.index(id).is_some())
            .count();
        legend::entries(
            &self.map,
            &self.lay.boxes,
            lines,
            self.mode() == Mode::Sites,
            pinned,
            true,
        )
    }

    /// The corner the legend took of the canvas `rect` last frame (as Fit
    /// keeps clear of it), if it's shown.
    fn legend_area(&self, rect: Rect) -> Option<Rect> {
        (self.legend && self.legend_size != Vec2::ZERO).then(|| {
            Rect::from_min_max(
                pos2(rect.min.x + 10.0, rect.max.y - 10.0 - self.legend_size.y),
                pos2(rect.min.x + 10.0 + self.legend_size.x, rect.max.y - 10.0),
            )
        })
    }

    /// Cards just dropped (`ids`), if one is mostly under the legend, where
    /// it can't be grabbed again: they glide up together, just clear of it.
    pub(super) fn clear_of_legend(&mut self, rect: Rect, ids: &[String]) {
        let Some(area) = self.legend_area(rect) else {
            return;
        };
        let lift = ids
            .iter()
            .filter_map(|id| Some((self.index(id)?, self.pos.get(id)?)))
            .map(|(i, &p)| {
                let size = layout::card_size(&self.map.nodes[i]);
                Rect::from_min_size(self.to_screen(rect, p), size * self.zoom)
            })
            .filter(|r| {
                let o = r.intersect(area);
                o.is_positive() && o.area() > r.area() / 2.0
            })
            .map(|r| r.max.y - (area.min.y - 8.0))
            .fold(0.0, f32::max);
        if lift <= 0.0 {
            return;
        }
        for id in ids {
            if let Some(&p) = self.pos.get(id) {
                self.placed
                    .insert(id.clone(), p - vec2(0.0, lift / self.zoom));
            }
        }
    }

    /// Lay the legend out for the canvas `rect`: bottom left, at most
    /// MAX_SHARE of its height, columns side by side as far as Saved maps
    /// and the zoom controls (above them on a narrow canvas), scrolling past
    /// that.
    fn plan(&self, ui: &Ui, rect: Rect, entries: &[Entry]) -> Plan {
        let vis = ui.visuals();
        let font = FontId::proportional(11.5);
        let labels: Vec<_> = entries
            .iter()
            .map(|e| {
                let off = e.role.is_some_and(|z| self.hidden.contains(&z));
                let tc = if off {
                    vis.weak_text_color()
                } else {
                    vis.text_color()
                };
                let label = ui
                    .painter()
                    .layout_no_wrap(e.label.clone(), font.clone(), tc);
                let count = e.count.map(|n| {
                    ui.painter()
                        .layout_no_wrap(n.to_string(), font.clone(), vis.weak_text_color())
                });
                (label, count)
            })
            .collect();
        // Each slot's width: a heading's, or an entry's swatch and text.
        let width = |slot: &Slot| match (slot.head, slot.entry) {
            (Some(h), _) => {
                ui.painter()
                    .layout_no_wrap(h.title().into(), FontId::proportional(9.5), Color32::WHITE)
                    .size()
                    .x
            }
            (_, Some(k)) => {
                let (l, c) = &labels[k];
                SWATCH.x + GAP + l.size().x + c.as_ref().map_or(0.0, |c| GAP + c.size().x)
            }
            _ => 0.0,
        };
        // Whole rows: when they scroll, none is cut in half at the bottom.
        let fit_rows =
            (((rect.height() * MAX_SHARE - TITLE - 2.0 * PAD) / ROW).floor() as usize).max(3);
        let max_h = fit_rows as f32 * ROW;
        let total = entries.len() + 4;
        // As many rows to a column as fit in `max_w`; more when the columns
        // would be too wide, and then the rows scroll.
        let arrange = |max_w: f32| {
            let inner_w = max_w - 2.0 * PAD;
            let mut rows = fit_rows;
            loop {
                let slots = legend::slots(entries, rows);
                let cols = slots.iter().map(|s| s.col + 1).max().unwrap_or(1);
                let mut col_w = vec![0.0f32; cols];
                for s in &slots {
                    col_w[s.col] = col_w[s.col].max(width(s));
                }
                let mut x = vec![0.0f32; cols];
                for c in 1..cols {
                    x[c] = x[c - 1] + col_w[c - 1] + COL_GAP;
                }
                let size = vec2(
                    x[cols - 1] + col_w[cols - 1],
                    slots.iter().map(|s| s.row + 1).max().unwrap_or(1) as f32 * ROW,
                );
                if size.x <= inner_w || rows >= total {
                    let at: Vec<(Slot, Pos2)> = slots
                        .iter()
                        .map(|s| (*s, pos2(x[s.col], s.row as f32 * ROW)))
                        .collect();
                    break (at, size);
                }
                rows += 1;
            }
        };
        // Beside the zoom controls and Saved maps when it fits there without
        // scrolling (or scrolls less than above them), else above them, the
        // canvas wide. Both stop where Saved maps starts (`tabs::corner_left`).
        let beside_w = super::tabs::corner_left(rect) - 8.0 - (rect.min.x + 10.0);
        let above_w = rect.width() - 20.0;
        let above = || {
            let bottom = super::tabs::corner_top(rect) - 8.0;
            (above_w, bottom, arrange(above_w))
        };
        let (max_w, bottom, (at, content)) = if beside_w >= 220.0 {
            let side = arrange(beside_w);
            if side.1.y <= max_h + 0.5 {
                (beside_w, rect.max.y - 10.0, side)
            } else {
                let up = above();
                if up.2.1.y < side.1.y {
                    up
                } else {
                    (beside_w, rect.max.y - 10.0, side)
                }
            }
        } else {
            above()
        };
        let inner_w = max_w - 2.0 * PAD;
        let scrolls = content.y > max_h + 0.5;
        let view = vec2(
            content.x.min(inner_w) + if scrolls { 12.0 } else { 0.0 },
            content.y.min(max_h),
        );
        let size = vec2(
            (view.x + 2.0 * PAD).max(110.0).min(max_w),
            view.y + TITLE + 2.0 * PAD,
        );
        let panel = Rect::from_min_size(pos2(rect.min.x + 10.0, bottom - size.y), size);
        let body = Rect::from_min_size(
            panel.min + vec2(PAD, PAD + TITLE),
            vec2(size.x - 2.0 * PAD, view.y),
        );
        Plan {
            panel,
            body,
            at,
            content,
            scrolls,
            labels,
        }
    }

    /// The legend on the canvas `rect`, from `entries`; records its size
    /// for Fit. Its × turns it off.
    pub(super) fn legend_panel(&mut self, ui: &mut Ui, rect: Rect, entries: &[Entry]) {
        if entries.is_empty() {
            self.legend_size = Vec2::ZERO;
            return;
        }
        let plan = self.plan(ui, rect, entries);
        // What it takes of the bottom left corner (it can sit above the
        // zoom controls).
        self.legend_size = vec2(
            plan.panel.max.x - rect.min.x - 10.0,
            rect.max.y - 10.0 - plan.panel.min.y,
        );
        let vis = ui.visuals().clone();
        // The panel takes clicks and drags, so none reach the map under it
        // (a card dropped under it is moved out, see `drag_cards`).
        ui.interact(
            plan.panel,
            ui.id().with("map_legend"),
            Sense::click_and_drag(),
        );
        let p = ui.painter_at(rect);
        p.rect(
            plan.panel,
            8,
            vis.window_fill,
            Stroke::new(1.0, vis.widgets.noninteractive.bg_stroke.color),
            StrokeKind::Inside,
        );
        p.text(
            plan.panel.min + vec2(PAD, PAD + TITLE / 2.0 - 2.0),
            egui::Align2::LEFT_CENTER,
            "Legend",
            FontId::proportional(12.5),
            vis.strong_text_color(),
        );
        let x = Rect::from_center_size(
            pos2(
                plan.panel.max.x - PAD - 6.0,
                plan.panel.min.y + PAD + TITLE / 2.0 - 2.0,
            ),
            vec2(18.0, 18.0),
        );
        if ui
            .put(x, egui::Button::new("×").frame(false))
            .on_hover_text("Hide the legend (L; the Legend button brings it back)")
            .clicked()
        {
            self.legend = false;
        }
        let now = ui.input(|i| i.time);
        let z = self.zoom.clamp(0.6, 1.2);
        let mut flip = None;
        let mut body = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(plan.body)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        // A scroll bar that's always there when rows are out of sight, not
        // one that shows only under the pointer.
        if plan.scrolls {
            body.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
        }
        egui::ScrollArea::vertical()
            .id_salt("map_legend_rows")
            .max_height(plan.body.height())
            .auto_shrink([false, true])
            .show(&mut body, |ui| {
                let (area, _) = ui.allocate_exact_size(plan.content, Sense::hover());
                let p = ui.painter().clone();
                for (slot, off) in &plan.at {
                    let top = area.min + off.to_vec2();
                    match (slot.head, slot.entry) {
                        (Some(s), _) => {
                            p.text(
                                top + vec2(0.0, ROW / 2.0 + 1.0),
                                egui::Align2::LEFT_CENTER,
                                s.title(),
                                FontId::proportional(9.5),
                                vis.weak_text_color(),
                            );
                        }
                        (None, Some(k)) => {
                            let e = &entries[k];
                            let (label, count) = &plan.labels[k];
                            let off = e.role.is_some_and(|z| self.hidden.contains(&z));
                            let s = Rect::from_min_size(
                                top + vec2(0.0, (ROW - SWATCH.y) / 2.0),
                                SWATCH,
                            );
                            let row = Rect::from_min_size(
                                top,
                                vec2(SWATCH.x + GAP + label.size().x, ROW),
                            );
                            swatch(
                                &p.with_clip_rect(row.intersect(p.clip_rect())),
                                &vis,
                                e,
                                s,
                                off,
                                z,
                                now,
                            );
                            let tx = s.max.x + GAP;
                            let ty = top.y + (ROW - label.size().y) / 2.0;
                            let lw = label.size().x;
                            p.galley(pos2(tx, ty), label.clone(), vis.text_color());
                            if let Some(c) = count {
                                p.galley(pos2(tx + lw + GAP, ty), c.clone(), vis.weak_text_color());
                            }
                            let full = row.with_max_x(
                                tx + lw + count.as_ref().map_or(0.0, |c| GAP + c.size().x),
                            );
                            if e.role.is_none() {
                                ui.interact(full, ui.id().with(("legend_tip", k)), Sense::hover())
                                    .on_hover_text(tip(e.swatch));
                            }
                            if let Some(zone) = e.role {
                                let r = ui
                                    .interact(full, ui.id().with(("legend", zone)), Sense::click())
                                    .on_hover_text("Click to fade this role out or back in");
                                if r.clicked() {
                                    flip = Some(zone);
                                }
                                if r.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            });
        if let Some(z) = flip
            && !self.hidden.remove(&z)
        {
            self.hidden.insert(z);
        }
    }
}

/// What an entry means, for its tooltip.
fn tip(s: Swatch) -> &'static str {
    match s {
        Swatch::Card => "The internet: the top of every path",
        Swatch::Folded => "One card standing for several: open it from the sidebar or its menu",
        Swatch::Gone => {
            "Pinned, then missing from the latest data: check it, then unpin it or × to let it go"
        }
        Swatch::Warn => "Hover or select the card for what's wrong",
        Swatch::Pin => "Stays selected while you click others",
        Swatch::Chip => "The uplink's port, or a note such as inferred, on the card it leads to",
        Swatch::Line(Link::Backbone) => "The internet to a firewall, in the gateway's colour",
        Swatch::Line(Link::Assumed) => {
            "Placed here because it answered, not because the data shows the link \
             (in a box's colour, for lines into one)"
        }
        Swatch::Line(_) => "In the colour of the card it leads to (a box's, for lines into one)",
        Swatch::Trunk(_) => {
            "Every link from one card into a box, folded into one line in the colour \
             of the box it runs into"
        }
        Swatch::Lit => "Click a card: its path to the internet lights up, packets flowing",
        Swatch::Box => "A site's cards (click its title to fold it), or a role's in Zones",
    }
}

/// An entry's swatch in `s`, drawn as the map draws it (`z` its zoom for
/// lines). `off`: a role faded out, drawn faded as on the map.
fn swatch(p: &Painter, vis: &egui::Visuals, e: &Entry, s: Rect, off: bool, z: f32, now: f64) {
    let c = ink(vis, e.ink);
    let glow = glow_strength(vis);
    let a = if off { 0.15 } else { 1.0 };
    let mid = [s.left_center(), s.right_center()];
    match e.swatch {
        Swatch::Card => card_frame(p, vis, s, CARD_Z, c, a, false, false, false),
        Swatch::Gone => card_frame(p, vis, s, CARD_Z, c, a, false, false, true),
        Swatch::Folded => {
            card_frame(p, vis, s, CARD_Z, c, a, false, false, false);
            let chip = Rect::from_min_max(
                pos2(s.min.x + 6.0, s.max.y - 6.0),
                pos2(s.max.x - 5.0, s.max.y - 2.0),
            );
            chip_pill(p, chip, c, a);
        }
        Swatch::Warn => warn_badge(p, s.center(), 0.75, a, now),
        Swatch::Pin => pin(p, vis, s.center(), 0.75, a),
        Swatch::Chip => {
            let chip = Rect::from_center_size(s.center(), vec2(s.width(), s.height()));
            chip_pill(p, chip, c, a);
            p.text(
                chip.center(),
                egui::Align2::CENTER_CENTER,
                "port",
                FontId::proportional(9.0),
                c.gamma_multiply(a),
            );
        }
        Swatch::Line(link) => {
            draw_link(
                p,
                &mid,
                c,
                c,
                link,
                link_width(link, z, false),
                a,
                false,
                z,
                glow,
                now,
            );
        }
        Swatch::Trunk(link) => {
            // As a trunk into a box: a little wider, its count at its end.
            let end = s.max.x - 12.0;
            let w = link_width(link, z, false) + 0.6;
            let pts = [s.left_center(), pos2(end, s.center().y)];
            draw_link(p, &pts, c, c, link, w, a, false, z, glow, now);
            p.text(
                pos2(end + 1.0, s.center().y),
                egui::Align2::LEFT_CENTER,
                "×N",
                trunk_font(z),
                vis.weak_text_color().gamma_multiply(a),
            );
        }
        Swatch::Lit => {
            let w = link_width(Link::Wired, z, true);
            draw_link(p, &mid, c, c, Link::Wired, w, a, true, z, glow, now);
        }
        Swatch::Box => draw_box(p, s, CARD_Z, c, a),
    }
}
