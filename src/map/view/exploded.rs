//! The exploded view on the Map tab: the layout opened up by
//! `map::explode`, its plates drawn under the cards, and the controls for
//! it beside the zoom buttons. Cards, lines, selection and the inspector
//! are the map's own: only where things are drawn changes.

use super::*;
use crate::map::explode::{self, Camera, Part, Scene};

/// How fast Spin turns the 3D view, in radians a second: once round in
/// about 40 seconds.
const SPIN: f32 = 0.16;
/// How far a drag turns the 3D view, in radians a point.
const ORBIT: f32 = 0.008;

/// The plates, top to bottom: the roles each holds.
const TIERS: [&[Zone]; 7] = [
    &[Zone::Internet],
    &[Zone::Gateway],
    &[Zone::Switch],
    &[Zone::Wireless, Zone::Gear, Zone::Server],
    &[Zone::Device],
    &[Zone::Client],
    &[Zone::Unknown],
];

fn tier(z: Zone) -> usize {
    TIERS
        .iter()
        .position(|t| t.contains(&z))
        .unwrap_or(TIERS.len() - 1)
}

impl MapView {
    /// The map is drawn opened up (or is on its way there or back).
    pub(super) fn exploded(&self) -> bool {
        self.ex_scene.is_some()
    }

    /// How big card `i` is drawn, of its own size: 1 but in 3D, where a
    /// nearer card is bigger.
    pub(super) fn card_scale(&self, i: usize) -> f32 {
        self.scene().map_or(1.0, |s| s.scale_of(i))
    }

    /// The view is in 3D (or on its way there).
    pub(super) fn three_d(&self) -> bool {
        self.ex_on && self.ex.depth >= 0.5
    }

    /// 3D on or off: the view from a point, turned any way with a drag.
    /// It opens the map up if it's flat.
    pub(super) fn three_d_toggle(&mut self) {
        let on = !self.three_d();
        self.ex.depth = if on { 1.0 } else { 0.0 };
        if !on {
            self.ex_spin = false;
        }
        if on && !self.ex_on {
            self.explode_toggle();
        }
        self.follow = true;
        self.whole = true;
    }

    /// Where the layout puts card `i` on the flat map (or where it was
    /// dragged to).
    pub(super) fn flat_target(&self, i: usize) -> Option<Pos2> {
        let n = &self.map.nodes[i];
        self.placed.get(&n.id).copied().or(self.lay.pos[i])
    }

    /// Open the map up, or put it back flat. Either way the whole of it
    /// comes into view.
    pub(super) fn explode_toggle(&mut self) {
        self.ex_on = !self.ex_on;
        if self.ex_on && self.ex.amount < 0.05 {
            self.ex.amount = 1.0;
        }
        self.drag = None;
        self.box_drag = None;
        self.follow = true;
        self.whole = true;
    }

    /// Turn the view a step left (`dir` below 0) or right.
    pub(super) fn explode_turn(&mut self, dir: f32) {
        self.ex.yaw = explode::turned(self.ex.yaw, dir);
    }

    /// For the screenshot tour: opened up and split this far, in 3D or
    /// not, nothing selected (None: flat again). False until the view has
    /// settled.
    pub fn explode_for_shots(&mut self, split: Option<f32>, three_d: bool) -> bool {
        let on = split.is_some();
        if self.ex_on != on {
            self.select(None);
            self.explode_toggle();
        }
        if let Some(split) = split {
            self.ex.split = split;
        }
        let depth = if three_d { 1.0 } else { 0.0 };
        if self.ex.depth != depth {
            self.select(None);
            self.ex.depth = depth;
        }
        let want = if on { self.ex.amount } else { 0.0 };
        self.ex_now.amount == want
            && self.ex_now.split == self.ex.split
            && self.ex_now.depth == self.ex.depth
            && self.aim.is_none()
    }

    /// A drag by `d` on the map in 3D turns the view: across turns it
    /// about its upright axis, up and down tips it.
    pub(super) fn orbit(&mut self, d: Vec2) {
        self.ex.yaw += d.x * ORBIT;
        self.ex.pitch =
            (self.ex.pitch + d.y * ORBIT * 0.75).clamp(explode::PITCH_MIN, explode::PITCH_MAX);
        // Under the hand, not easing after it; and the zoom is the user's
        // now (Fit frames it again).
        self.ex_now.yaw = self.ex.yaw;
        self.ex_now.pitch = self.ex.pitch;
        self.follow = false;
    }

    /// The scene Export's pictures draw: the view as shown, but in 3D
    /// the exploded drawing from the same side (a picture's cards are all
    /// one size).
    pub(super) fn picture_scene(&self) -> Option<std::borrow::Cow<'_, Scene>> {
        let scene = self.scene()?;
        if self.ex_now.depth <= 0.0 {
            return Some(std::borrow::Cow::Borrowed(scene));
        }
        let cam = Camera {
            depth: 0.0,
            ..self.ex_now
        };
        Some(std::borrow::Cow::Owned(explode::build(&self.parts(), cam)))
    }

    /// The flat layout as `explode::build` takes it.
    fn parts(&self) -> Vec<Option<Part>> {
        let n = self.map.nodes.len();
        let flat: Vec<Option<Pos2>> = (0..n).map(|i| self.flat_target(i)).collect();
        let parent: Vec<Option<usize>> = (0..n)
            .map(|i| {
                self.map.nodes[i]
                    .parent()
                    .filter(|&p| p < n && flat[p].is_some())
            })
            .collect();
        let shown: Vec<bool> = flat.iter().map(Option::is_some).collect();
        let groups = explode::assemblies(&parent, &shown);
        (0..n)
            .map(|i| {
                let node = &self.map.nodes[i];
                Some(Part {
                    rect: layout::card_rect(node, flat[i]?),
                    tier: tier(node.zone),
                    parent: parent[i],
                    group: groups[i],
                })
            })
            .collect()
    }

    /// Once a frame, before anything asks where a card is: ease the view
    /// toward what's asked for, and open the layout up that far. Flat
    /// again, the map is the layout's own.
    pub(super) fn explode_update(&mut self, ctx: &egui::Context) {
        let want = if self.ex_on {
            self.ex
        } else {
            Camera {
                amount: 0.0,
                ..self.ex
            }
        };
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let k = if self.animate {
            1.0 - (-dt * 9.0).exp()
        } else {
            1.0
        };
        let mut moving = false;
        let mut ease = |now: &mut f32, to: f32| {
            if (to - *now).abs() < 0.002 {
                *now = to;
            } else {
                *now += (to - *now) * k;
                moving = true;
            }
        };
        ease(&mut self.ex_now.amount, want.amount);
        ease(&mut self.ex_now.yaw, want.yaw);
        ease(&mut self.ex_now.split, want.split);
        ease(&mut self.ex_now.pitch, want.pitch);
        ease(&mut self.ex_now.depth, want.depth);
        // Spin: a slow turn of its own, while the packets would flow (it
        // holds still once the map has been left alone a while).
        if self.ex_spin && self.three_d() && self.animate && self.flowing {
            self.ex.yaw += dt * SPIN;
            self.ex_now.yaw = self.ex.yaw;
            moving = true;
        }
        if moving {
            ctx.request_repaint();
        }
        if !self.ex_on && self.ex_now.amount == 0.0 {
            self.ex_scene = None;
            // Turned and split as asked the next time it opens.
            self.ex_now = want;
            return;
        }
        self.ex_scene = Some(explode::build(&self.parts(), self.ex_now));
    }

    /// The scene, if it's of the map as it is now (a rebuild since leaves
    /// it a frame behind).
    pub(super) fn scene(&self) -> Option<&Scene> {
        self.ex_scene
            .as_ref()
            .filter(|s| s.pos.len() == self.map.nodes.len())
    }

    /// The exploded view's lines, weakest links first as the flat map
    /// draws them.
    pub(super) fn explode_lines(&self, scene: &Scene) -> Vec<layout::Line> {
        let mut lines = scene.lines.clone();
        lines.sort_by_key(|l| self.map.nodes[l.kids[0]].link().map_or(0, layout::strength));
        lines
    }

    /// The plates, under the lines and cards: each a sheet in its role's
    /// colour, named at its top corner, with a ring where each line from
    /// the plate above lands.
    pub(super) fn paint_plates(&self, p: &Painter, rect: Rect, vis: &egui::Visuals, scene: &Scene) {
        // Fading in as the map opens.
        let a = (self.ex_now.amount * 2.5).min(1.0);
        let z = self.zoom;
        for plate in &scene.plates {
            let quad = plate.quad.map(|q| self.to_screen(rect, q));
            let bounds = Rect::from_points(&quad);
            if !bounds.intersects(rect) {
                continue;
            }
            let zones: Vec<Zone> = plate
                .cards
                .iter()
                .map(|&i| self.map.nodes[i].zone)
                .collect();
            let shown = zones.iter().any(|zn| !self.hidden.contains(zn));
            let a = if shown { a } else { a * 0.25 };
            let c = zone_colour(zones.first().copied().unwrap_or_default());
            // In 3D each plate is a lit sheet ruled with a grid: a soft
            // wide edge under a bright thin one.
            let lit = self.ex_now.depth;
            if lit > 0.0 {
                let rule = Stroke::new(1.0, c.gamma_multiply(0.16 * a * lit));
                p.extend(
                    plate
                        .grid
                        .iter()
                        .map(|l| Shape::line_segment(l.map(|q| self.to_screen(rect, q)), rule)),
                );
                let mut edge = quad.to_vec();
                edge.push(quad[0]);
                p.add(Shape::line(
                    edge,
                    Stroke::new(5.0, c.gamma_multiply(0.14 * a * lit)),
                ));
            }
            p.add(Shape::convex_polygon(
                quad.to_vec(),
                c.gamma_multiply((0.07 + 0.03 * lit) * a),
                Stroke::new(1.0, c.gamma_multiply((0.55 + 0.35 * lit) * a)),
            ));
            // Named above its top-left corner as drawn, while it's wide
            // enough to carry a name: zoomed out to colour tiles, the
            // plates' names still say what's where.
            if bounds.width() > 90.0 {
                let at = quad
                    .into_iter()
                    .min_by(|x, y| (x.x + x.y * 2.0).total_cmp(&(y.x + y.y * 2.0)))
                    .expect("four corners");
                p.text(
                    at + vec2(2.0, -3.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{} · {}", explode::plate_name(&zones), plate.cards.len()),
                    FontId::proportional((11.0 * z).clamp(9.5, 13.0)),
                    c.gamma_multiply(a),
                );
            }
        }
        // The circle the map stands on in 3D.
        if scene.ring.len() > 2 {
            let glow = crate::theme::accent();
            let pts: Vec<Pos2> = scene
                .ring
                .iter()
                .map(|q| self.to_screen(rect, *q))
                .collect();
            let lit = self.ex_now.depth * a;
            p.add(Shape::line(
                pts.clone(),
                Stroke::new(6.0, glow.gamma_multiply(0.10 * lit)),
            ));
            p.add(Shape::line(
                pts,
                Stroke::new(1.0, glow.gamma_multiply(0.55 * lit)),
            ));
        }
        let ring = vis.weak_text_color().gamma_multiply(0.8 * a);
        for foot in &scene.feet {
            let at = self.to_screen(rect, *foot);
            if rect.contains(at) {
                p.circle_stroke(at, (3.5 * z).max(1.5), Stroke::new(1.0, ring));
            }
        }
    }

    /// Explode and 3D, above the zoom buttons in the corner of `place`;
    /// and, while the map is opened up, beside them: turn left and right,
    /// how far it's exploded, how far split, and (in 3D) Spin.
    pub(super) fn explode_controls(&mut self, ui: &mut Ui, place: Rect) {
        let h = crate::theme::TARGET_H;
        let (w, w3) = (74.0, 40.0);
        // Left of Save, where a narrow canvas puts it above the zoom
        // buttons too.
        let (right, y) = match tabs::toggle_rect(place) {
            Some(save) if save.max.y < place.max.y - 30.0 => (save.min.x - 6.0, save.min.y),
            _ => (place.max.x - 10.0, place.max.y - 10.0 - 2.0 * h - 8.0),
        };
        if right - w < place.min.x + 4.0 {
            return;
        }
        let r = Rect::from_min_size(pos2(right - w, y), vec2(w, h));
        if ui
            .put(r, egui::Button::new("Explode").selected(self.ex_on))
            .on_hover_text(
                "Exploded view: the map pulled apart into a plate for each role, like an \
                 exploded drawing, with a line from each uplink down to its cards (E)",
            )
            .clicked()
        {
            self.explode_toggle();
        }
        let mut left = right - w - 4.0;
        if left - w3 >= place.min.x + 4.0 {
            let r = Rect::from_min_size(pos2(left - w3, y), vec2(w3, h));
            if ui
                .put(r, egui::Button::new("3D").selected(self.three_d()))
                .on_hover_text(
                    "3D: the opened-up map seen from a point, nearer cards bigger. Drag to \
                     turn and tip it, Shift+drag to move it, scroll to zoom (3)",
                )
                .clicked()
            {
                self.three_d_toggle();
            }
            left -= w3 + 4.0;
        }
        if !self.ex_on {
            return;
        }
        // Spin only in 3D; the sliders give way when room is short.
        let spin = if self.three_d() { 52.0 + 4.0 } else { 0.0 };
        let label = 68.0;
        let fixed = 8.0 + 2.0 * 32.0 + 2.0 * (label + 4.0) + spin;
        let room = left - 2.0 - (place.min.x + 4.0);
        // No room beside a floating inspector: the keys still turn it.
        if room < fixed + 2.0 * 40.0 {
            return;
        }
        let slider = ((room - fixed) / 2.0).min(96.0);
        let row = fixed + 2.0 * slider;
        let panel = Rect::from_min_size(pos2(left - 2.0 - row, y - 4.0), vec2(row, h + 8.0));
        // The row takes clicks, so none reach the map under it.
        ui.interact(panel, ui.id().with("map_explode"), Sense::click_and_drag());
        let vis = ui.visuals().clone();
        ui.painter().rect(
            panel,
            8,
            vis.window_fill,
            Stroke::new(1.0, vis.widgets.noninteractive.bg_stroke.color),
            StrokeKind::Inside,
        );
        let mut at = panel.min.x + 4.0;
        let top = panel.min.y + 4.0;
        let mut place = |w: f32| {
            let r = Rect::from_min_size(pos2(at, top), vec2(w, h));
            at += w + 4.0;
            r
        };
        let how = if self.three_d() {
            "drag the map to turn it freely"
        } else {
            "Alt+drag on the map turns it freely"
        };
        if ui
            .put(place(28.0), egui::Button::new("⟲"))
            .on_hover_text(format!("Turn the view left ([); {how}"))
            .clicked()
        {
            self.explode_turn(-1.0);
        }
        if ui
            .put(place(28.0), egui::Button::new("⟳"))
            .on_hover_text(format!("Turn the view right (]); {how}"))
            .clicked()
        {
            self.explode_turn(1.0);
        }
        let dial = |ui: &mut Ui, r: Rect, v: &mut f32, label: &str, tip: &str| {
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(r)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            child.spacing_mut().slider_width = slider;
            child
                .add(
                    egui::Slider::new(v, 0.0..=1.0)
                        .show_value(false)
                        .text(label),
                )
                .on_hover_text(tip)
                .changed()
        };
        let mut cam = self.ex;
        let r = place(slider + label);
        let exploded = dial(
            ui,
            r,
            &mut cam.amount,
            "Explode",
            "How far the plates are pulled apart: all the way left is the flat map",
        );
        let r = place(slider + label);
        let split = dial(
            ui,
            r,
            &mut cam.split,
            "Split",
            "Pull each branch of the network (a switch and everything under it) away from \
             the others, onto plates of its own",
        );
        if self.three_d() {
            let r = place(52.0);
            let can = self.animate;
            let b = egui::Button::new("Spin").selected(self.ex_spin && can);
            if ui
                .put(r, |ui: &mut Ui| ui.add_enabled(can, b))
                .on_hover_text(
                    "Turn the view slowly by itself. It holds still once the map has been \
                     left alone for half a minute",
                )
                .on_disabled_hover_text("Animate is off (Settings)")
                .clicked()
            {
                self.ex_spin = !self.ex_spin;
            }
        }
        if exploded || split {
            self.ex = cam;
            // All the way down is the flat map: back out of the view.
            if cam.amount <= 0.0 {
                self.explode_toggle();
            }
        }
    }
}
