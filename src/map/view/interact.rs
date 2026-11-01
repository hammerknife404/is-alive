//! Screen/world transforms, hit testing, pointer and keyboard input, and
//! the glide toward a new layout.

use super::*;

impl MapView {
    pub(super) fn to_screen(&self, rect: Rect, w: Pos2) -> Pos2 {
        rect.min + self.pan + w.to_vec2() * self.zoom
    }

    pub(super) fn to_world(&self, rect: Rect, s: Pos2) -> Pos2 {
        ((s - rect.min - self.pan) / self.zoom).to_pos2()
    }

    pub(super) fn hit(&self, w: Pos2) -> Option<usize> {
        (0..self.map.nodes.len()).rev().find(|&i| {
            self.pos
                .get(&self.map.nodes[i].id)
                .is_some_and(|p| Rect::from_min_size(*p, CARD).contains(w))
        })
    }

    /// The site whose box title strip is under the screen point `at`.
    pub(super) fn site_title_at(&self, rect: Rect, at: Option<Pos2>) -> Option<String> {
        let w = self.to_world(rect, at.filter(|p| rect.contains(*p))?);
        self.lay
            .boxes
            .iter()
            .find(|b| {
                b.tree && {
                    let mut strip = b.rect;
                    strip.max.y = strip.min.y + layout::BOX_TITLE;
                    strip.contains(w)
                }
            })
            .map(|b| b.title.clone())
    }

    pub(super) fn interact(&mut self, ui: &Ui, rect: Rect, resp: &egui::Response) {
        let pointer = resp.hover_pos();
        let over = pointer.and_then(|p| self.hit(self.to_world(rect, p)));
        let title = if over.is_none() {
            self.site_title_at(rect, pointer)
        } else {
            None
        };
        if resp.drag_started() {
            self.drag = resp
                .interact_pointer_pos()
                .and_then(|p| self.hit(self.to_world(rect, p)))
                .map(|i| self.map.nodes[i].id.clone());
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            match self.drag.clone() {
                Some(id) => {
                    // A selected card brings the rest of the selection along.
                    let sel = self.selection();
                    let group: Vec<String> = if sel.iter().any(|&i| self.map.nodes[i].id == id) {
                        sel.iter().map(|&i| self.map.nodes[i].id.clone()).collect()
                    } else {
                        vec![id]
                    };
                    for id in group {
                        let at = self.pos.get(&id).copied().unwrap_or_default() + d / self.zoom;
                        self.placed.insert(id.clone(), at);
                        self.pos.insert(id, at);
                    }
                    self.follow = false;
                }
                None => {
                    self.pan += d;
                    self.follow = false;
                    self.aim = None;
                }
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
        if resp.clicked()
            && let Some(site) = &title
        {
            self.flip_site = Some(site.clone());
        } else if resp.clicked() {
            let ctrl = ui.input(|i| i.modifiers.command);
            match over {
                Some(i) if ctrl => self.toggle(i),
                // Ctrl+click on the canvas leaves the selection be.
                None if ctrl => {}
                _ => self.select(over),
            }
        }
        // Right-click a card: pin it, select it, copy it.
        if resp.secondary_clicked() {
            self.menu_for = over;
        }
        if self.menu_for.is_some() {
            resp.context_menu(|ui| {
                let Some(i) = self.menu_for.filter(|&i| i < self.map.nodes.len()) else {
                    ui.close();
                    return;
                };
                ui.label(RichText::new(&self.map.nodes[i].label).strong());
                ui.separator();
                let pin = if self.is_kept(i) {
                    "📌 Unpin"
                } else {
                    "📌 Pin (keep selected)"
                };
                if ui.button(pin).clicked() {
                    self.keep(i);
                    ui.close();
                }
                if ui.button("Select only this").clicked() {
                    self.select(Some(i));
                    ui.close();
                }
                if ui.button("Add to selection (Ctrl+click)").clicked() {
                    if !self.selection().contains(&i) {
                        self.toggle(i);
                    }
                    ui.close();
                }
                ui.separator();
                let up = self.map.nodes[i].parent();
                let path = self.map.path(i);
                let kids: Vec<usize> = self.map.children()[i].clone();
                let below = self.below(i);
                if let Some(u) = up
                    && ui
                        .button(format!(
                            "Select with its uplink ({})",
                            self.map.nodes[u].label
                        ))
                        .on_hover_text("This card and the one it hangs off")
                        .clicked()
                {
                    self.select_with(i, &[u]);
                    ui.close();
                }
                if path.len() > 2
                    && ui
                        .button(format!("Select the path to the WAN ({} cards)", path.len()))
                        .on_hover_text("This card and every uplink up to the internet")
                        .clicked()
                {
                    self.select_with(i, &path);
                    ui.close();
                }
                if !kids.is_empty()
                    && ui
                        .button(format!("Select with its direct children ({})", kids.len()))
                        .on_hover_text("This card and the cards hanging straight off it")
                        .clicked()
                {
                    self.select_with(i, &kids);
                    ui.close();
                }
                if below.len() > kids.len()
                    && ui
                        .button(format!("Select with everything below ({})", below.len()))
                        .on_hover_text("This card and every card under it, all the way down")
                        .clicked()
                {
                    self.select_with(i, &below);
                    ui.close();
                }
                ui.separator();
                if let Some(site) = self.map.nodes[i].site.clone()
                    && self.mode() == Mode::Sites
                {
                    let label = if self.map.closed.contains_key(&site) {
                        format!("Expand {site}")
                    } else {
                        format!("Collapse {site}")
                    };
                    if ui.button(label).clicked() {
                        self.flip_site = Some(site);
                        ui.close();
                    }
                }
                if ui.button("Copy details").clicked() {
                    widgets::copy(ui.ctx(), card_text(&self.map.nodes[i]));
                    widgets::toast("Card details copied", true);
                    ui.close();
                }
            });
        }
        if resp.double_clicked() {
            match over {
                Some(i) => self.focus(rect, i),
                None => {
                    self.follow = true;
                    self.fit(rect);
                }
            }
        }
        if resp.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let f = pinch * (scroll * 0.0015).exp();
            if (f - 1.0).abs() > 1e-4
                && let Some(p) = pointer
            {
                let w = self.to_world(rect, p);
                self.zoom = (self.zoom * f).clamp(MIN_ZOOM, MAX_ZOOM);
                self.pan = p - rect.min - w.to_vec2() * self.zoom;
                self.follow = false;
                self.aim = None;
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.select(None);
        }
        let icon = if self.drag.is_some() || (resp.dragged() && over.is_none()) {
            egui::CursorIcon::Grabbing
        } else if over.is_some() || title.is_some() {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Grab
        };
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(icon);
        }
        if let Some(i) = over
            && self.drag.is_none()
        {
            let n = &self.map.nodes[i];
            if !n.details.is_empty() || !n.flags.is_empty() {
                resp.clone().on_hover_ui_at_pointer(|ui| {
                    ui.set_max_width(320.0);
                    ui.strong(&n.label);
                    for f in &n.flags {
                        widgets::warning(ui, f);
                    }
                    for d in n.details.iter().take(8) {
                        ui.label(RichText::new(d).small());
                    }
                    ui.weak("Click for more");
                });
            }
        }
    }

    /// Ease cards toward the layout and the view toward its aim.
    pub(super) fn animate(&mut self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        let k = 1.0 - (-dt * 11.0).exp();
        let mut moving = false;
        for i in 0..self.map.nodes.len() {
            let Some(t) = self.target(i) else { continue };
            let id = &self.map.nodes[i].id;
            if self.drag.as_deref() == Some(id.as_str()) {
                continue;
            }
            let p = self.pos.entry(id.clone()).or_insert(t);
            let d = t - *p;
            if d.length() > 0.3 {
                *p += d * k;
                moving = true;
            } else {
                *p = t;
            }
        }
        if let Some((pan, zoom)) = self.aim {
            let (dp, dz) = (pan - self.pan, zoom - self.zoom);
            if dp.length() < 0.5 && dz.abs() < 0.001 {
                (self.pan, self.zoom) = (pan, zoom);
                self.aim = None;
            } else {
                self.pan += dp * k;
                self.zoom += dz * k;
                moving = true;
            }
        }
        let now = ctx.input(|i| i.time);
        moving |= self.born.values().any(|b| now - b < 0.4);
        if moving {
            ctx.request_repaint();
        }
    }
}
