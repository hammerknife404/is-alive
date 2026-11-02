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

    /// The card under the world point `w`: the one drawn on top. Cards
    /// faded out (a hidden role, a search miss off the selection's path)
    /// can't be picked unless they're selected; a search miss on the path
    /// is drawn in sight, so it can be.
    pub(super) fn hit(&self, w: Pos2) -> Option<usize> {
        let picked = self.selection();
        let lit = if self.search.trim().is_empty() {
            HashSet::new()
        } else {
            self.lit(&picked).0
        };
        let sel: HashSet<usize> = picked.into_iter().collect();
        self.stack().into_iter().rev().find(|&i| {
            (sel.contains(&i) || !self.faded(i, &lit))
                && self
                    .pos
                    .get(&self.map.nodes[i].id)
                    .is_some_and(|p| layout::card_rect(&self.map.nodes[i], *p).contains(w))
        })
    }

    /// Faded out, as `paint` draws it: a hidden role, or a search miss
    /// that isn't on the selection's path (`lit`).
    pub(super) fn faded(&self, i: usize, lit: &HashSet<usize>) -> bool {
        let n = &self.map.nodes[i];
        self.hidden.contains(&n.zone) || (!self.matches(n) && !lit.contains(&i))
    }

    /// The order cards are drawn in, bottom first: the rest, then the
    /// selected ones and those an open menu is for, then the ones dragged
    /// last, so a card dropped on another stays on top.
    pub(super) fn stack(&self) -> Vec<usize> {
        let sel: HashSet<usize> = self.selection().into_iter().collect();
        let menu: HashSet<usize> = self.menu_cards().into_iter().collect();
        let raised: HashSet<&str> = self.raised.iter().map(String::as_str).collect();
        let mut v: Vec<usize> = (0..self.map.nodes.len()).collect();
        v.sort_by_key(|&i| {
            if raised.contains(self.map.nodes[i].id.as_str()) {
                2
            } else if sel.contains(&i) || menu.contains(&i) {
                1
            } else {
                0
            }
        });
        v
    }

    /// The cards the open right-click menu acts on.
    pub(super) fn menu_cards(&self) -> Vec<usize> {
        match &self.menu {
            Some(Menu::Card(id)) => self.index(id).map(|i| self.menu_group(i)),
            _ => None,
        }
        .unwrap_or_default()
    }

    /// Where a site box's title is on screen: its top strip, or the line
    /// just above the box when zoomed out too far to write in the strip.
    pub(super) fn title_rect(&self, rect: Rect, b: &layout::Group) -> Rect {
        let r = Rect::from_min_size(self.to_screen(rect, b.rect.min), b.rect.size() * self.zoom);
        if title_above(self.zoom) {
            Rect::from_min_max(pos2(r.min.x, r.min.y - 12.0), pos2(r.max.x, r.min.y + 2.0))
        } else {
            Rect::from_min_max(
                r.min,
                pos2(r.max.x, r.min.y + layout::BOX_TITLE * self.zoom),
            )
        }
    }

    /// The site whose box title is under the screen point `at`.
    pub(super) fn site_title_at(&self, rect: Rect, at: Option<Pos2>) -> Option<String> {
        let at = at.filter(|p| rect.contains(*p))?;
        self.lay
            .boxes
            .iter()
            .find(|b| b.tree && self.title_rect(rect, b).contains(at))
            .map(|b| b.title.clone())
    }

    /// Pointer and keyboard on the canvas. Returns the card under the
    /// pointer (none while a menu or tooltip covers it).
    pub(super) fn interact(&mut self, ui: &Ui, rect: Rect, resp: &egui::Response) -> Option<usize> {
        let now = ui.input(|i| i.time);
        let pointer = resp.hover_pos();
        let over = pointer.and_then(|p| self.hit(self.to_world(rect, p)));
        let title = if over.is_none() {
            self.site_title_at(rect, pointer)
        } else {
            None
        };
        self.drag_cards(ui, rect, resp);
        // A site's title folds it and the boxes move: the second click of
        // a double-click there would land on whatever moved under it.
        let settling = now - self.flipped_at < 0.4;
        if resp.clicked() && !settling {
            let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
            if let Some(site) = &title {
                self.flip_sites.push(site.clone());
                self.flipped_at = now;
            } else {
                match over {
                    Some(i) if add => self.toggle(i),
                    // Ctrl+click on the canvas leaves the selection be.
                    None if add => {}
                    _ => {
                        // Only the first click of a double-click counts.
                        if self.undo.as_ref().is_none_or(|u| now - u.at > 0.5) {
                            self.undo = Some(Undo {
                                at: now,
                                selected: self.selected.clone(),
                                also: self.also.clone(),
                                traced: self.traced,
                                order: self.order.clone(),
                            });
                        }
                        self.select(over);
                    }
                }
            }
        }
        if resp.secondary_clicked() {
            self.menu = Some(match over {
                Some(i) => Menu::Card(self.map.nodes[i].id.clone()),
                None => Menu::Background,
            });
        }
        if let Some(menu) = self.menu.clone() {
            let shown = resp.context_menu(|ui| match &menu {
                Menu::Card(id) => match self.index(id) {
                    Some(i) => self.card_menu(ui, i),
                    // Gone in a rebuild: nothing left to act on.
                    None => ui.close(),
                },
                Menu::Background => self.background_menu(ui, rect),
            });
            if shown.is_none() {
                self.menu = None;
            }
        }
        if resp.double_clicked() && !settling {
            // Zooming in or fitting doesn't let go of what was selected:
            // the click before this one is undone.
            let undo = self.undo.take().filter(|u| now - u.at < 0.6);
            match over {
                Some(i) => {
                    let id = &self.map.nodes[i].id;
                    if let Some(u) = undo
                        && (u.selected.as_ref() == Some(id) || u.also.contains(id))
                    {
                        self.restore(u);
                    }
                    self.focus(rect, i);
                }
                None => {
                    if let Some(u) = undo {
                        self.restore(u);
                    }
                    self.fit_view(rect);
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
        // Esc that closed a menu or left a text box was theirs.
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !self.keys_busy {
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
        // A +N or − chip shows its own tooltip, not the card's.
        if let Some(i) = over
            && self.drag.is_none()
            && self.menu.is_none()
            && !pointer.is_some_and(|p| self.on_chip(rect, p))
        {
            self.card_tip(resp, i);
        }
        over
    }

    /// Put back the selection a click replaced.
    pub(super) fn restore(&mut self, u: Undo) {
        self.selected = u.selected;
        self.also = u.also;
        self.traced = u.traced;
        self.order = u.order;
    }

    /// Dragging a card moves it (a selected one brings the rest of the
    /// selection along); dragging the background pans. Near the canvas
    /// edge, a dragged card scrolls the map.
    fn drag_cards(&mut self, ui: &Ui, rect: Rect, resp: &egui::Response) {
        if resp.drag_started() {
            // Where the button went down, not where the pointer is once it
            // counts as a drag: a quick flick off a card still moves it.
            self.drag = ui
                .input(|i| i.pointer.press_origin())
                .or(resp.interact_pointer_pos())
                .and_then(|p| self.hit(self.to_world(rect, p)))
                .map(|i| self.map.nodes[i].id.clone());
            if let Some(id) = &self.drag {
                let sel: Vec<String> = self
                    .selection()
                    .into_iter()
                    .map(|i| self.map.nodes[i].id.clone())
                    .collect();
                self.raised = if sel.contains(id) {
                    sel
                } else {
                    vec![id.clone()]
                };
            }
        }
        if resp.dragged() {
            let mut d = resp.drag_delta();
            if self.drag.is_some() {
                let at = ui.input(|i| i.pointer.latest_pos());
                // The cards follow the pointer only as far as the canvas
                // edge: never under the detail panel or out of the window.
                if let Some(p) = at {
                    d = rect.clamp(p) - rect.clamp(p - d);
                }
                let push = at.map_or(Vec2::ZERO, |p| edge_push(rect, p));
                if push != Vec2::ZERO {
                    // The map scrolls under the pointer; the cards stay
                    // with it.
                    let step = push * 420.0 * ui.input(|i| i.stable_dt).min(0.05);
                    self.pan += step;
                    d -= step;
                    self.aim = None;
                    ui.ctx().request_repaint();
                }
                for id in self.raised.clone() {
                    let at = self.pos.get(&id).copied().unwrap_or_default() + d / self.zoom;
                    self.placed.insert(id.clone(), at);
                    self.pos.insert(id, at);
                }
                self.follow = false;
            } else {
                self.pan += d;
                self.follow = false;
                self.aim = None;
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
    }

    /// The hover tooltip on a card: its name and what's known, and what a
    /// click does.
    fn card_tip(&self, resp: &egui::Response, i: usize) {
        let n = &self.map.nodes[i];
        let picked = self.selection().contains(&i);
        resp.clone().on_hover_ui_at_pointer(|ui| {
            ui.set_max_width(320.0);
            ui.strong(&n.label);
            let what: Vec<&str> = [n.kind.as_str(), n.sub.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect();
            if !what.is_empty() {
                ui.label(RichText::new(what.join(" · ")).small().weak());
            }
            for f in &n.flags {
                widgets::warning(ui, f);
            }
            for d in n.details.iter().take(8) {
                ui.label(RichText::new(d).small());
            }
            ui.add_space(2.0);
            ui.weak(if picked {
                "Right-click for more"
            } else {
                "Click for its path to the internet; right-click for more"
            });
        });
    }

    /// The right-click menu on card `i`: on one of several selected cards,
    /// every item acts on all of them.
    fn card_menu(&mut self, ui: &mut Ui, i: usize) {
        let group = self.menu_group(i);
        let n = group.len();
        let many = n > 1;
        if many {
            ui.label(RichText::new(format!("{n} selected cards")).strong())
                .on_hover_text("Everything here acts on all of them");
        } else {
            ui.label(RichText::new(&self.map.nodes[i].label).strong());
        }
        ui.separator();

        // Pin: all of them, or (all pinned already) none.
        let pinned = group.iter().all(|&k| self.is_kept(k));
        let pin = match (many, pinned) {
            (false, false) => "Pin".to_string(),
            (false, true) => "Unpin".to_string(),
            (true, false) => format!("Pin all {n}"),
            (true, true) => format!("Unpin all {n}"),
        };
        if ui
            .button(pin)
            .on_hover_text("A pinned card stays selected when you click elsewhere")
            .clicked()
        {
            for &k in &group {
                if pinned || !self.is_kept(k) {
                    self.keep(k);
                }
            }
            ui.close();
        }
        let sel = self.selection();
        // Select only this card lets go of the others, but not of pinned
        // ones: shown only when there's one to let go of.
        let others = sel.iter().filter(|&&k| k != i);
        let loose = others.clone().any(|&k| !self.is_kept(k));
        let only_tip = if others.clone().any(|&k| self.is_kept(k)) {
            "Let go of the others; pinned cards stay"
        } else {
            "Let go of the others"
        };
        if many {
            if loose
                && ui
                    .button("Select only this card")
                    .on_hover_text(only_tip)
                    .clicked()
            {
                self.select_group(i, &[]);
                ui.close();
            }
            if ui
                .add(egui::Button::new("Clear selection").shortcut_text("Esc"))
                .on_hover_text("Pinned cards stay")
                .clicked()
            {
                self.select(None);
                ui.close();
            }
        } else if sel.contains(&i) {
            if ui
                .add(egui::Button::new("Remove from selection").shortcut_text("Ctrl+click"))
                .clicked()
            {
                self.toggle(i);
                ui.close();
            }
        } else {
            let only = if loose {
                ui.button("Select only this card").on_hover_text(only_tip)
            } else {
                ui.button("Select")
            };
            if only.clicked() {
                self.select_group(i, &[]);
                ui.close();
            }
            if !sel.is_empty()
                && ui
                    .add(egui::Button::new("Add to selection").shortcut_text("Ctrl+click"))
                    .clicked()
            {
                self.toggle(i);
                ui.close();
            }
        }

        // Select more: each one is shown only when it adds cards.
        let (it, its, them) = if many {
            ("these cards", "their", "them")
        } else {
            ("this card", "its", "it")
        };
        let up = self.grow(&group, Grow::Uplink);
        let up_n = up.len();
        let path = self.grow(&group, Grow::Path);
        let kids = self.grow(&group, Grow::Children);
        let below = self.grow(&group, Grow::Below);
        let uplink_tip = match self.map.nodes[i].parent() {
            Some(u) if !many => format!(
                "This card and {}, the card it hangs off",
                self.map.nodes[u].label
            ),
            _ => "These cards and the cards they hang off".to_string(),
        };
        let items = [
            (
                up.len() > n,
                format!("Select with {its} uplink{}", if many { "s" } else { "" }),
                uplink_tip,
                up,
            ),
            (
                path.len() > up_n,
                format!(
                    "Select {its} path{} to the WAN",
                    if many { "s" } else { "" }
                ),
                format!("{} and every uplink up to the internet", cap(it)),
                path,
            ),
            (
                kids.len() > n,
                format!("Select with {its} direct children"),
                format!("{} and the cards hanging straight off {them}", cap(it)),
                kids.clone(),
            ),
            (
                below.len() > kids.len(),
                format!("Select with everything below {them}"),
                format!("{} and every card under {them}, all the way down", cap(it)),
                below,
            ),
        ];
        if items.iter().any(|t| t.0) {
            ui.separator();
        }
        for (show, label, tip, cards) in items {
            if show
                && ui
                    .add(egui::Button::new(label).shortcut_text(format!("{} cards", cards.len())))
                    .on_hover_text(tip)
                    .clicked()
            {
                self.select_group(i, &cards);
                ui.close();
            }
        }

        ui.separator();
        if self.mode() == Mode::Sites {
            let mut sites: Vec<String> = Vec::new();
            for &k in &group {
                if let Some(s) = &self.map.nodes[k].site
                    && !sites.contains(s)
                {
                    sites.push(s.clone());
                }
            }
            let open: Vec<String> = sites
                .iter()
                .filter(|s| !self.map.closed.contains_key(*s))
                .cloned()
                .collect();
            let (verb, flip) = if open.is_empty() {
                ("Expand", sites)
            } else {
                ("Collapse", open)
            };
            let label = match flip.as_slice() {
                [] => None,
                [one] => Some(format!("{verb} {one}")),
                more => Some(format!("{verb} {} sites", more.len())),
            };
            if let Some(label) = label
                && ui
                    .button(label)
                    .on_hover_text(if verb == "Expand" {
                        "Open the site up again"
                    } else {
                        "Fold the site to one card"
                    })
                    .clicked()
            {
                self.flip_sites.extend(flip);
                ui.close();
            }
        }
        let moved: Vec<String> = group
            .iter()
            .map(|&k| self.map.nodes[k].id.clone())
            .filter(|id| self.placed.contains_key(id))
            .collect();
        if !moved.is_empty()
            && ui
                .button("Put back in the layout")
                .on_hover_text(format!("Back where the layout had {them}"))
                .clicked()
        {
            for id in moved {
                self.placed.remove(&id);
            }
            ui.close();
        }
        let copy = if many {
            format!("Copy details of all {n}")
        } else {
            "Copy details".to_string()
        };
        if ui.button(copy).clicked() {
            let text: Vec<String> = group
                .iter()
                .map(|&k| card_text(&self.map.nodes[k]))
                .collect();
            widgets::copy(ui.ctx(), text.join("\n"));
            widgets::toast(
                if many {
                    format!("{n} cards copied")
                } else {
                    "Card details copied".to_string()
                },
                true,
            );
            ui.close();
        }
    }

    /// The right-click menu on the empty canvas: the view, the selection,
    /// the layout, export.
    fn background_menu(&mut self, ui: &mut Ui, rect: Rect) {
        if ui
            .add(egui::Button::new("Fit").shortcut_text("Double-click"))
            .on_hover_text("Show the whole map")
            .clicked()
        {
            self.fit_view(rect);
            ui.close();
        }
        if ui
            .button("Center")
            .on_hover_text("Move the middle of the map to the middle of the view, at this zoom")
            .clicked()
        {
            self.center(rect);
            ui.close();
        }
        if !self.selection().is_empty()
            && ui
                .add(egui::Button::new("Clear selection").shortcut_text("Esc"))
                .on_hover_text("Pinned cards stay")
                .clicked()
        {
            self.select(None);
            ui.close();
        }
        if !self.placed.is_empty()
            && ui
                .button("Re-layout")
                .on_hover_text("Put dragged cards back where the layout had them")
                .clicked()
        {
            self.placed.clear();
            self.fit_view(rect);
            ui.close();
        }
        ui.separator();
        if ui
            .button("Export SVG…")
            .on_hover_text("Save the map as SVG")
            .clicked()
        {
            self.export_svg(ui.visuals().dark_mode);
            ui.close();
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

/// What a plain click replaced, for a double-click to put back.
pub(super) struct Undo {
    pub(super) at: f64,
    pub(super) selected: Option<String>,
    pub(super) also: Vec<String>,
    pub(super) traced: bool,
    pub(super) order: Vec<String>,
}

/// Zoomed out this far, a site box's title strip is too short to write
/// in: the name goes just above the box.
pub(super) fn title_above(zoom: f32) -> bool {
    layout::BOX_TITLE * zoom < 13.0
}

/// How hard a card dragged near the canvas edge pushes the map, each way
/// (-1 to 1): nothing more than 24 px in, nor once the pointer is well past
/// the edge (over the detail panel, or out of the window).
pub(super) fn edge_push(rect: Rect, p: Pos2) -> Vec2 {
    const EDGE: f32 = 24.0;
    if !rect.expand(EDGE).contains(p) {
        return Vec2::ZERO;
    }
    let axis = |at: f32, lo: f32, hi: f32| {
        if at < lo + EDGE {
            ((lo + EDGE - at) / EDGE).min(1.0)
        } else if at > hi - EDGE {
            -((at - (hi - EDGE)) / EDGE).min(1.0)
        } else {
            0.0
        }
    };
    vec2(
        axis(p.x, rect.left(), rect.right()),
        axis(p.y, rect.top(), rect.bottom()),
    )
}

/// The first letter in capitals.
fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}
