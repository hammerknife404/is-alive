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

    /// The cards the open right-click menu (or ☰ menu) acts on.
    pub(super) fn menu_cards(&self) -> Vec<usize> {
        match &self.menu {
            Some(Menu::Card(id) | Menu::Panel(id)) => self.index(id).map(|i| self.menu_group(i)),
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

    /// The box whose title is under the screen point `at`: the one drawn
    /// on top where moved boxes overlap.
    pub(super) fn box_title_at(&self, rect: Rect, at: Option<Pos2>) -> Option<usize> {
        let at = at.filter(|p| rect.contains(*p))?;
        self.lay
            .boxes
            .iter()
            .rposition(|b| self.title_rect(rect, b).contains(at))
    }

    /// The box `key` names, in the layout now.
    pub(super) fn box_index(&self, key: &str) -> Option<usize> {
        (0..self.lay.boxes.len())
            .find(|&g| self.lay.box_key(self.mode(), g).as_deref() == Some(key))
    }

    /// Select box `g`'s cards (`add`: along with what's selected).
    pub(super) fn select_box(&mut self, g: usize, add: bool) {
        let cards = self.lay.in_box(g);
        let Some(&first) = cards.first() else { return };
        if add {
            let sel: HashSet<usize> = self.selection().into_iter().collect();
            for &c in cards.iter().filter(|c| !sel.contains(c)) {
                self.toggle(c);
            }
        } else {
            self.select_group(first, &cards[1..]);
        }
    }

    /// Move box `g` by `d` (world), with its cards and the cards dragged
    /// out of it by hand, as one: its lines follow, folded as before.
    pub(super) fn shift_box(&mut self, g: usize, d: Vec2) {
        let Some(key) = self.lay.box_key(self.mode(), g) else {
            return;
        };
        let cards: Vec<usize> = (0..self.lay.group.len())
            .filter(|&i| self.lay.group[i] == Some(g))
            .collect();
        self.lay.shift_box(g, d);
        let total = self.shifted.get(&key).copied().unwrap_or(Vec2::ZERO) + d;
        if total.length() < 0.01 {
            self.shifted.remove(&key);
        } else {
            self.shifted.insert(key, total);
        }
        for i in cards {
            let id = self.map.nodes[i].id.clone();
            if let Some(p) = self.placed.get_mut(&id) {
                *p += d;
            }
            if let Some(p) = self.pos.get_mut(&id) {
                *p += d;
            }
        }
    }

    /// Pointer and keyboard on the canvas. Returns the card under the
    /// pointer (none while a menu or tooltip covers it).
    pub(super) fn interact(&mut self, ui: &Ui, rect: Rect, resp: &egui::Response) -> Option<usize> {
        let now = ui.input(|i| i.time);
        let pointer = resp.hover_pos();
        // A card's note mark sits half above it: it's the card's too.
        let on_note = pointer.and_then(|p| self.note_hit(rect, p));
        let over = pointer
            .and_then(|p| self.hit(self.to_world(rect, p)))
            .or(on_note);
        let title = if over.is_none() {
            self.site_title_at(rect, pointer)
        } else {
            None
        };
        // A box's title: a click selects its cards (a site's folds it),
        // a drag moves the box.
        let boxed = if over.is_none() {
            self.box_title_at(rect, pointer)
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
            } else if let Some(g) = boxed {
                self.select_box(g, add);
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
            let group = boxed.or_else(|| {
                pointer.filter(|p| rect.contains(*p)).and_then(|p| {
                    let world = self.to_world(rect, p);
                    self.lay.boxes.iter().rposition(|b| b.rect.contains(world))
                })
            });
            let key = group.and_then(|g| self.lay.box_key(self.mode(), g));
            self.menu = Some(match (over, key) {
                (Some(i), _) => Menu::Card(self.map.nodes[i].id.clone()),
                (None, Some(k)) => Menu::Box(k),
                (None, None) => Menu::Background,
            });
        }
        // A ☰ menu open in the detail panel is the panel's to show.
        if let Some(menu) = self.menu.clone()
            && !matches!(menu, Menu::Panel(_))
        {
            let shown = resp.context_menu(|ui| match &menu {
                Menu::Card(id) => match self.index(id) {
                    Some(i) => self.card_menu(ui, i),
                    // Gone in a rebuild: nothing left to act on.
                    None => ui.close(),
                },
                Menu::Background => self.background_menu(ui, rect),
                Menu::Box(key) => match self.box_index(key) {
                    Some(g) => self.box_menu(ui, rect, g),
                    // Gone in a rebuild (another layout).
                    None => ui.close(),
                },
                // Left to the panel, above; never shown here.
                Menu::Panel(_) => ui.close(),
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
                // A double-click on a card's note mark opens its note.
                Some(i) if on_note == Some(i) => {
                    self.select(Some(i));
                    self.open_note(&[i]);
                }
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
        // Esc that closed a menu or left a text box was theirs. It lets the
        // cards go (shutting the inspector); with none to let go, on a small
        // window, it shuts Layers floating over the map.
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !self.keys_busy {
            let had = self.selection().len();
            self.select(None);
            let small = ui.ctx().content_rect().width() < 1100.0;
            if self.selection().len() == had && small {
                self.side = false;
            }
        }
        if !self.keys_busy {
            self.keys(ui, rect);
        }
        let icon = if self.dragging() || (resp.dragged() && over.is_none()) {
            egui::CursorIcon::Grabbing
        } else if over.is_some() || title.is_some() || boxed.is_some() {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Grab
        };
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(icon);
        }
        // A +N or − chip shows its own tooltip, not the card's.
        if let Some(i) = over
            && !self.dragging()
            && self.menu.is_none()
            && !pointer.is_some_and(|p| self.on_chip(rect, p))
        {
            self.card_tip(resp, i);
        }
        over
    }

    /// The view from the keyboard: F fits, + and − zoom, 0 is 100%, the
    /// arrows pan. Not with Ctrl or Alt held (Ctrl+plus zooms the app).
    fn keys(&mut self, ui: &Ui, rect: Rect) {
        use egui::Key;
        let (fit, zoom, pan) = ui.input(|i| {
            if i.modifiers.command || i.modifiers.alt {
                return (false, None, Vec2::ZERO);
            }
            let k = |key| i.key_pressed(key);
            let zoom = if k(Key::Plus) || k(Key::Equals) {
                Some(1.25)
            } else if k(Key::Minus) {
                Some(0.8)
            } else if k(Key::Num0) {
                Some(0.0)
            } else {
                None
            };
            let step = 80.0;
            let pan = [
                (Key::ArrowLeft, vec2(step, 0.0)),
                (Key::ArrowRight, vec2(-step, 0.0)),
                (Key::ArrowUp, vec2(0.0, step)),
                (Key::ArrowDown, vec2(0.0, -step)),
            ]
            .into_iter()
            .filter(|(key, _)| k(*key))
            .map(|(_, d)| d)
            .fold(Vec2::ZERO, |a, d| a + d);
            (k(Key::F), zoom, pan)
        });
        if fit {
            self.fit_view(rect);
        }
        if let Some(f) = zoom {
            let z = self.aim.map_or(self.zoom, |a| a.1);
            self.zoom_about_middle(rect, if f == 0.0 { 1.0 } else { z * f });
        }
        if pan != Vec2::ZERO {
            let (to, z) = self.aim.unwrap_or((self.pan, self.zoom));
            self.aim = Some((to + pan, z));
            self.follow = false;
        }
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
            // Not on a card: on a box's title, the box moves.
            if self.drag.is_none() {
                let origin = ui
                    .input(|i| i.pointer.press_origin())
                    .or(resp.interact_pointer_pos());
                self.box_drag = self
                    .box_title_at(rect, origin)
                    .and_then(|g| self.lay.box_key(self.mode(), g));
            }
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
            if self.dragging() {
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
                if let Some(g) = self.box_drag.clone().and_then(|k| self.box_index(&k)) {
                    self.shift_box(g, d / self.zoom);
                } else if self.drag.is_some() {
                    for id in self.raised.clone() {
                        let at = self.pos.get(&id).copied().unwrap_or_default() + d / self.zoom;
                        self.placed.insert(id.clone(), at);
                        self.pos.insert(id, at);
                    }
                }
                self.follow = false;
            } else {
                self.pan += d;
                self.follow = false;
                self.aim = None;
            }
        }
        if resp.drag_stopped() {
            self.box_drag = None;
            if self.drag.take().is_some() {
                self.clear_of_legend(rect, &self.raised.clone());
            }
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
            if let Some(d) = super::panel::drawn_note(n) {
                ui.label(RichText::new(d).small().strong());
            }
            for f in &n.flags {
                widgets::warning(ui, f);
            }
            for d in n.details.iter().take(8) {
                ui.label(RichText::new(d).small());
            }
            if !n.note.is_empty() {
                ui.add_space(2.0);
                ui.horizontal_top(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(9.0, 11.0), Sense::hover());
                    paint::note_glyph(ui.painter(), ui.visuals(), r, 1.0);
                    ui.add(egui::Label::new(notes::first_lines(&n.note)).wrap());
                });
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
    /// every item acts on all of them. A card's ☰ in the detail panel opens
    /// this same menu. Its sections, each shown only when it has items:
    /// pin and select, the note, select more, clients, test, copy, then the
    /// layout.
    pub(super) fn card_menu(&mut self, ui: &mut Ui, i: usize) {
        let group = self.menu_group(i);
        self.menu_header(ui, i, &group);
        let mut sep = Sep(true);
        self.menu_pick(ui, &mut sep, i, &group);
        sep.next();
        self.menu_note(ui, &mut sep, i, &group);
        sep.next();
        self.menu_more(ui, &mut sep, i, &group);
        sep.next();
        self.menu_clients(ui, &mut sep, i, &group);
        sep.next();
        self.menu_test(ui, &mut sep, i, &group);
        self.menu_wake(ui, &mut sep, &group);
        sep.next();
        self.menu_copy(ui, &mut sep, &group);
        sep.next();
        self.menu_layout(ui, &mut sep, &group);
    }

    /// The menu's title: the card's name, with its role and IP under it;
    /// or how many cards it acts on.
    fn menu_header(&self, ui: &mut Ui, i: usize, group: &[usize]) {
        if group.len() > 1 {
            ui.label(RichText::new(format!("{} selected cards", group.len())).strong())
                .on_hover_text("Everything here acts on all of them");
            return;
        }
        let n = &self.map.nodes[i];
        ui.label(RichText::new(&n.label).strong());
        let ip = n.ip.map(|ip| ip.to_string()).filter(|ip| *ip != n.label);
        let about: Vec<String> = [Some(n.kind.clone()), ip]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect();
        if !about.is_empty() {
            ui.label(RichText::new(about.join(" · ")).small().weak());
        }
    }

    /// Pin, and select this card (or let go of the others).
    fn menu_pick(&mut self, ui: &mut Ui, sep: &mut Sep, i: usize, group: &[usize]) {
        let n = group.len();
        let many = n > 1;
        // Pin: all of them, or (all pinned already) none.
        let pinned = group.iter().all(|&k| self.is_kept(k));
        let pin = match (many, pinned) {
            (false, false) => "Pin".to_string(),
            (false, true) => "Unpin".to_string(),
            (true, false) => format!("Pin all {n}"),
            (true, true) => format!("Unpin all {n}"),
        };
        if sep
            .add(ui, egui::Button::new(pin))
            .on_hover_text("A pinned card stays selected when you click elsewhere")
            .clicked()
        {
            for &k in group {
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
                && sep
                    .add(ui, egui::Button::new("Select only this card"))
                    .on_hover_text(only_tip)
                    .clicked()
            {
                self.select_group(i, &[]);
                ui.close();
            }
            if sep
                .add(
                    ui,
                    egui::Button::new("Clear selection").shortcut_text("Esc"),
                )
                .on_hover_text("Pinned cards stay")
                .clicked()
            {
                self.select(None);
                ui.close();
            }
        } else if sel.contains(&i) {
            if sep
                .add(
                    ui,
                    egui::Button::new("Remove from selection").shortcut_text("Ctrl+click"),
                )
                .clicked()
            {
                self.toggle(i);
                ui.close();
            }
        } else {
            let only = if loose {
                sep.add(ui, egui::Button::new("Select only this card"))
                    .on_hover_text(only_tip)
            } else {
                sep.add(ui, egui::Button::new("Select"))
            };
            if only.clicked() {
                self.select_group(i, &[]);
                ui.close();
            }
            if !sel.is_empty()
                && sep
                    .add(
                        ui,
                        egui::Button::new("Add to selection").shortcut_text("Ctrl+click"),
                    )
                    .clicked()
            {
                self.toggle(i);
                ui.close();
            }
        }
    }

    /// Note: add, edit or remove the card's note; on several cards, the
    /// same note on each, or theirs taken off.
    fn menu_note(&mut self, ui: &mut Ui, sep: &mut Sep, i: usize, group: &[usize]) {
        for (label, tip, remove) in self.note_items(group) {
            if sep
                .add(ui, egui::Button::new(label))
                .on_hover_text(tip)
                .clicked()
            {
                if remove {
                    self.remove_notes(group);
                } else {
                    // The card right-clicked first: its panel shows the
                    // editor.
                    let mut cards = vec![i];
                    cards.extend(group.iter().copied().filter(|&k| k != i));
                    self.open_note(&cards);
                }
                ui.close();
            }
        }
    }

    /// Select more: each one is shown only when it adds cards.
    fn menu_more(&mut self, ui: &mut Ui, sep: &mut Sep, i: usize, group: &[usize]) {
        let n = group.len();
        let many = n > 1;
        let (it, its, them) = if many {
            ("these cards", "their", "them")
        } else {
            ("this card", "its", "it")
        };
        let up = self.grow(group, Grow::Uplink);
        let up_n = up.len();
        let path = self.grow(group, Grow::Path);
        let kids = self.grow(group, Grow::Children);
        let below = self.grow(group, Grow::Below);
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
        for (show, label, tip, cards) in items {
            if show
                && sep
                    .add(
                        ui,
                        egui::Button::new(label).shortcut_text(format!("{} cards", cards.len())),
                    )
                    .on_hover_text(tip)
                    .clicked()
            {
                self.select_group(i, &cards);
                ui.close();
            }
        }
    }

    /// Clients (one card with clients it doesn't draw, or shown by itself):
    /// the next few, all of them, or fold them back, as its +N and − chips
    /// and its panel's buttons do.
    fn menu_clients(&mut self, ui: &mut Ui, sep: &mut Sep, i: usize, group: &[usize]) {
        if group.len() != 1 {
            return;
        }
        for it in self.reveal_items(i) {
            if sep
                .add(ui, egui::Button::new(it.label))
                .on_hover_text(it.tip)
                .clicked()
            {
                self.reveal_step(it.uplink, it.to);
                ui.close();
            }
        }
    }

    /// Test: Ping, Port Check and Traceroute, run as the panel's buttons
    /// run them, the result in the card's section of the panel; or in full
    /// in the tool's tab. Then the device's web admin page. On several
    /// cards: Ping or Port Check each one that has an IP.
    fn menu_test(&mut self, ui: &mut Ui, sep: &mut Sep, i: usize, group: &[usize]) {
        let with_ip: Vec<usize> = group
            .iter()
            .copied()
            .filter(|&k| self.map.nodes[k].ip.is_some())
            .collect();
        if with_ip.is_empty() {
            return;
        }
        if group.len() > 1 {
            self.menu_test_all(ui, sep, &with_ip, group.len());
            return;
        }
        let n = &self.map.nodes[i];
        let (id, ip) = (n.id.clone(), n.ip.map(std::net::IpAddr::from));
        let admin = ip.filter(|_| has_admin(n)).map(|ip| web_url(ip, &n.open));
        for (tool, tip) in TESTS {
            if sep
                .add_enabled(
                    ui,
                    !self.runs.active(&id, tool),
                    egui::Button::new(tool.name()),
                )
                .on_hover_text(tip)
                .on_disabled_hover_text("Running: see its section in the panel")
                .clicked()
            {
                self.menu_run(ui.ctx(), &[i], tool);
                ui.close();
            }
        }
        if let Some(ip) = ip {
            sep.before(ui);
            ui.menu_button("Open in a tab", |ui| {
                for (tool, _) in TESTS {
                    if ui
                        .add_enabled(!self.busy, egui::Button::new(tool.name()))
                        .on_hover_text(format!("Run it in full in the {} tab", tool.name()))
                        .on_disabled_hover_text("Wait for the test running in its tab to finish")
                        .clicked()
                    {
                        self.menu_act = Some(panel::tab(tool, ip.to_string()));
                        ui.close();
                    }
                }
            });
        }
        if let Some(url) = admin
            && sep
                .add(ui, egui::Button::new("Open web admin"))
                .on_hover_text(format!("Open {url} in your browser"))
                .clicked()
        {
            widgets::open_url(url);
            ui.close();
        }
    }

    /// Wake-on-LAN: the card, or every selected card with a real MAC.
    fn menu_wake(&mut self, ui: &mut Ui, sep: &mut Sep, group: &[usize]) {
        let cards: Vec<(crate::net::mac::Mac, Option<std::net::Ipv4Addr>)> = group
            .iter()
            .filter_map(|&k| {
                let n = &self.map.nodes[k];
                n.mac.filter(crate::net::wol::wakeable).map(|m| (m, n.ip))
            })
            .collect();
        if cards.is_empty() {
            return;
        }
        let (label, tip) = if group.len() > 1 {
            (
                format!("Wake all ({})", cards.len()),
                format!(
                    "Send Wake-on-LAN to the {} selected cards with a known MAC",
                    cards.len()
                ),
            )
        } else {
            (
                "Wake (Wake-on-LAN)".to_string(),
                format!(
                    "Send a Wake-on-LAN packet to {}",
                    crate::net::mac::fmt(&cards[0].0)
                ),
            )
        };
        if sep
            .add(ui, egui::Button::new(label))
            .on_hover_text(tip)
            .clicked()
        {
            crate::net::wol::wake_all(cards);
            ui.close();
        }
    }

    /// Ping all, Port Check all: each of `cards` (those of `n` that have
    /// an IP) gets the panel's quick run; a few at a time, the rest queued.
    /// A card already running that test is left to finish, as the panel's
    /// button is greyed out while it runs.
    fn menu_test_all(&mut self, ui: &mut Ui, sep: &mut Sep, cards: &[usize], n: usize) {
        for (tool, tip) in TESTS.into_iter().take(2) {
            let idle: Vec<usize> = cards
                .iter()
                .copied()
                .filter(|&k| !self.runs.active(&self.map.nodes[k].id, tool))
                .collect();
            let (k, busy) = (idle.len(), cards.len() - idle.len());
            let label = all_label(tool, k, cards.len(), n);
            let tip = if busy > 0 && k > 0 {
                format!("{tip}\nNot the {busy} already running it")
            } else {
                tip.to_string()
            };
            let off = if k == 0 {
                "Running: see their sections in the panel".to_string()
            } else {
                format!("At most {MAX_TESTS} at once: select fewer cards")
            };
            if sep
                .add_enabled(ui, k > 0 && k <= MAX_TESTS, egui::Button::new(label))
                .on_hover_text(tip)
                .on_disabled_hover_text(off)
                .clicked()
            {
                self.menu_run(ui.ctx(), &idle, tool);
                ui.close();
            }
        }
    }

    /// Run `tool` on each of `cards`, as the panel's buttons do, and scroll
    /// the panel to the result. A card that isn't selected joins the
    /// selection (or is selected, when nothing is), so its result shows.
    /// Several: a toast says where the results are, and the panel goes to
    /// the first.
    fn menu_run(&mut self, ctx: &egui::Context, cards: &[usize], tool: Tool) {
        if let [i] = cards
            && !self.selection().contains(i)
        {
            if self.selection().is_empty() {
                self.select(Some(*i));
            } else {
                self.toggle(*i);
            }
        }
        for &k in cards {
            self.run_card(ctx, k, tool);
        }
        if cards.len() > 1 {
            widgets::toast(started(tool, cards.len()), true);
        }
        let first = self.ordered().into_iter().find(|k| cards.contains(k));
        if let Some(k) = first {
            panel::reveal(ctx, &self.map.nodes[k].id, tool);
        }
    }

    /// Copy: the IP and the MAC (one line per card for several), the
    /// details.
    fn menu_copy(&mut self, ui: &mut Ui, sep: &mut Sep, group: &[usize]) {
        let n = group.len();
        let many = n > 1;
        for (what, list) in self.copy_lists(group) {
            let k = list.len();
            let button = match list.as_slice() {
                [] => continue,
                [one] if !many => egui::Button::new(format!("Copy {what}")).shortcut_text(one),
                _ => egui::Button::new(format!("Copy {what}s")).shortcut_text(format!("{k} cards")),
            };
            let tip = if many {
                "One per line, for each card that has one".to_string()
            } else {
                format!("Copy its {what} address")
            };
            if sep.add(ui, button).on_hover_text(tip).clicked() {
                widgets::copy(ui.ctx(), list.join("\n"));
                widgets::toast(
                    if many {
                        format!("{k} {what}s copied")
                    } else {
                        format!("{what} copied")
                    },
                    true,
                );
                ui.close();
            }
        }
        let copy = if many {
            format!("Copy details of all {n}")
        } else {
            "Copy details".to_string()
        };
        if sep
            .add(ui, egui::Button::new(copy))
            .on_hover_text("Name, role, flags, details and note, as text")
            .clicked()
        {
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

    /// What Copy IP and Copy MAC copy: a line for each card in `group`
    /// that has one, in order.
    pub(super) fn copy_lists(&self, group: &[usize]) -> [(&'static str, Vec<String>); 2] {
        let lines = |what: &dyn Fn(&Node) -> Option<String>| -> Vec<String> {
            group
                .iter()
                .filter_map(|&k| what(&self.map.nodes[k]))
                .collect()
        };
        [
            ("IP", lines(&|n| n.ip.map(|ip| ip.to_string()))),
            ("MAC", lines(&|n| n.mac.map(|m| crate::net::mac::fmt(&m)))),
        ]
    }

    /// The layout: put dragged cards back; collapse or expand their site
    /// (Sites layout; Collapse only when the map has another site, since
    /// collapsing the only one folds the whole map into one card).
    fn menu_layout(&mut self, ui: &mut Ui, sep: &mut Sep, group: &[usize]) {
        let them = if group.len() > 1 { "them" } else { "it" };
        let moved: Vec<String> = group
            .iter()
            .map(|&k| self.map.nodes[k].id.clone())
            .filter(|id| self.placed.contains_key(id))
            .collect();
        if !moved.is_empty()
            && sep
                .add(ui, egui::Button::new("Put back in the layout"))
                .on_hover_text(format!("Back where the layout had {them}"))
                .clicked()
        {
            for id in moved {
                self.placed.remove(&id);
            }
            ui.close();
        }
        if self.mode() != Mode::Sites {
            return;
        }
        let mut sites: Vec<String> = Vec::new();
        for &k in group {
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
        let expand = open.is_empty();
        if !expand && !self.multi_site {
            return;
        }
        let (verb, flip) = if expand {
            ("Expand", sites)
        } else {
            ("Collapse", open)
        };
        let (label, tip) = match flip.as_slice() {
            [] => return,
            [one] if expand => (
                format!("Expand {}", ellipsis(one, SITE_CHARS)),
                format!("Open {one} up again"),
            ),
            [one] => (
                format!("Collapse {}", ellipsis(one, SITE_CHARS)),
                format!("Fold {one} into one card"),
            ),
            more => (format!("{verb} {} sites", more.len()), more.join(", ")),
        };
        if sep
            .add(ui, egui::Button::new(label))
            .on_hover_text(tip)
            .clicked()
        {
            self.flip_sites.extend(flip);
            ui.close();
        }
    }

    /// The right-click menu on the empty canvas: the view, the selection,
    /// the layout, the legend, export.
    /// The right-click menu on box `g`'s title: select its cards, put it
    /// back, which links show.
    fn box_menu(&mut self, ui: &mut Ui, rect: Rect, g: usize) {
        let Some(b) = self.lay.boxes.get(g).cloned() else {
            ui.close();
            return;
        };
        ui.label(RichText::new(&b.title).strong());
        let cards = self.lay.in_box(g);
        ui.separator();
        if !cards.is_empty() {
            let n = cards.len();
            let s = if n == 1 { "" } else { "s" };
            if ui
                .button(format!("Select its {n} card{s}"))
                .on_hover_text("Or click the box's title (Zones); Ctrl+click adds them")
                .clicked()
            {
                self.select_box(g, false);
                ui.close();
            }
            let sel = self.selection();
            if !sel.is_empty()
                && !cards.iter().all(|c| sel.contains(c))
                && ui
                    .button(format!("Add its {n} card{s} to the selection"))
                    .clicked()
            {
                self.select_box(g, true);
                ui.close();
            }
        }
        self.menu_layout(ui, &mut Sep(true), &cards);
        let key = self.lay.box_key(self.mode(), g);
        if let Some(d) = key.as_ref().and_then(|k| self.shifted.get(k).copied())
            && ui
                .button("Put the box back")
                .on_hover_text("Back where the layout had it, with its cards")
                .clicked()
        {
            self.shift_box(g, -d);
            if let Some(k) = &key {
                self.shifted.remove(k);
            }
            self.fit_view(rect);
            ui.close();
        }
        ui.menu_button("Move group", |ui| {
            for (label, delta) in [
                ("Left", vec2(-40.0, 0.0)),
                ("Right", vec2(40.0, 0.0)),
                ("Up", vec2(0.0, -40.0)),
                ("Down", vec2(0.0, 40.0)),
            ] {
                if ui.button(label).clicked() {
                    self.shift_box(g, delta);
                    ui.close();
                }
            }
            ui.weak("Or drag the box by its title");
        });
        if ui.button("Copy group name").clicked() {
            ui.ctx().copy_text(b.title.clone());
            ui.close();
        }
        // The box's empty background opens this menu too, and in Sites and
        // Zones boxes cover most of the map: the map's own items follow,
        // so Fit, Legend and Export are a right-click away anywhere.
        ui.separator();
        self.background_menu(ui, rect);
    }

    /// Which links the map draws, as a submenu.
    pub(super) fn links_menu(&mut self, ui: &mut Ui) {
        ui.menu_button("Links", |ui| {
            for l in Links::ALL {
                if ui
                    .add(egui::Button::selectable(self.links == l, l.name()))
                    .on_hover_text(l.tip())
                    .clicked()
                {
                    self.links = l;
                    self.lay.links = l;
                    ui.close();
                }
            }
            ui.separator();
            ui.weak("A selected card's path always shows");
        });
    }

    fn background_menu(&mut self, ui: &mut Ui, rect: Rect) {
        if ui
            .add(egui::Button::new("Fit").shortcut_text("F"))
            .on_hover_text("Show the whole map (or double-click the background)")
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
        if (!self.placed.is_empty() || !self.shifted.is_empty())
            && ui
                .button("Re-layout")
                .on_hover_text("Put dragged cards and boxes back where the layout had them")
                .clicked()
        {
            self.relayout_all();
            self.fit_view(rect);
            ui.close();
        }
        self.links_menu(ui);
        if ui
            .add(egui::Button::selectable(self.legend, "Legend").shortcut_text("L"))
            .on_hover_text("Show or hide what the colours, lines and badges mean")
            .clicked()
        {
            self.legend = !self.legend;
            ui.close();
        }
        ui.separator();
        ui.menu_button("Export", |ui| self.export_items(ui));
    }

    /// Ease cards toward the layout and the view toward its aim.
    pub(super) fn animate(&mut self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        // Straight there while nothing may move.
        let k = if self.animate {
            1.0 - (-dt * 11.0).exp()
        } else {
            1.0
        };
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

/// A separator owed before the next menu item: one stands only between
/// sections that have items, never two in a row.
struct Sep(bool);

impl Sep {
    /// Draw the separator owed, if any.
    fn before(&mut self, ui: &mut Ui) {
        if std::mem::take(&mut self.0) {
            ui.separator();
        }
    }

    fn add(&mut self, ui: &mut Ui, w: impl egui::Widget) -> egui::Response {
        self.before(ui);
        ui.add(w)
    }

    fn add_enabled(&mut self, ui: &mut Ui, on: bool, w: impl egui::Widget) -> egui::Response {
        self.before(ui);
        ui.add_enabled(on, w)
    }

    /// A new section: its first item gets a separator above it.
    fn next(&mut self) {
        self.0 = true;
    }
}

/// The card menu's tests, with what each does.
const TESTS: [(Tool, &str); 3] = [
    (Tool::Ping, "Ping it 4 times; the result shows in its panel"),
    (
        Tool::Ports,
        "Check its common ports; the result shows in its panel",
    ),
    (
        Tool::Trace,
        "Trace the route to it; the result shows in its panel",
    ),
];

/// Most cards one Ping all or Port Check all takes (they queue, a few
/// probing at a time).
const MAX_TESTS: usize = 32;

/// A site's name in the card menu is cut to this many characters.
const SITE_CHARS: usize = 28;

/// A card with a web admin page worth offering: the gateway or firewall,
/// switches, access points and other gear. A server or device only when a
/// web port is known to be open, or it's one that has a page (a NAS, ESXi
/// or Proxmox, a BMC, a backup appliance, a UPS card); not every server.
pub(super) fn has_admin(n: &Node) -> bool {
    if n.client {
        return false;
    }
    match n.zone {
        Zone::Gateway | Zone::Switch | Zone::Wireless | Zone::Gear => true,
        Zone::Server | Zone::Device => web_open(&n.open) || admin_words(n),
        _ => false,
    }
}

/// A web port is known to be open.
pub(super) fn web_open(open: &[u16]) -> bool {
    [443, 8443, 80].iter().any(|p| open.contains(p))
}

/// The card's kind, name, line or chips name a device that has a web
/// admin page.
fn admin_words(n: &Node) -> bool {
    const WORDS: [&str; 20] = [
        "NAS", "SAN", "SYNOLOGY", "QNAP", "TRUENAS", "UNRAID", "ESXI", "VCENTER", "PROXMOX", "PVE",
        "XCP", "IDRAC", "ILO", "IPMI", "BMC", "BDR", "DATTO", "UPS", "PDU", "NVR",
    ];
    std::iter::once(&n.kind)
        .chain([&n.label, &n.sub])
        .chain(&n.chips)
        .any(|s| {
            s.to_uppercase()
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|w| WORDS.contains(&w))
        })
}

/// Ping all or Port Check all: `k` cards to run, of the `ip` that have an
/// IP, of the `n` the menu acts on. None left (all running): as if all.
pub(super) fn all_label(tool: Tool, k: usize, ip: usize, n: usize) -> String {
    let name = tool.name();
    if k == n || (k == 0 && ip == n) {
        format!("{name} all {n}")
    } else if k == ip || k == 0 {
        format!("{name} the {ip} with an IP")
    } else {
        format!("{name} the other {k}")
    }
}

/// The toast when a test starts on `k` cards.
fn started(tool: Tool, k: usize) -> String {
    let what = match tool {
        Tool::Ping => format!("Pinging {k} cards"),
        Tool::Ports => format!("Checking the ports of {k} cards"),
        Tool::Trace => format!("Tracing the route to {k} cards"),
    };
    format!("{what}: each result shows in its card's section of the panel")
}

/// A device's web page: https on its own port, unless its known open ports
/// say 8443 or plain http; an IPv6 address in brackets.
pub(super) fn web_url(ip: std::net::IpAddr, open: &[u16]) -> String {
    let host = match ip {
        std::net::IpAddr::V4(v4) => v4.to_string(),
        std::net::IpAddr::V6(v6) => format!("[{v6}]"),
    };
    let (scheme, port) = [
        (443, "https", ""),
        (8443, "https", ":8443"),
        (80, "http", ""),
    ]
    .into_iter()
    .find(|(p, ..)| open.contains(p))
    .map_or(("https", ""), |(_, s, p)| (s, p));
    format!("{scheme}://{host}{port}")
}

/// `s` cut to `max` characters, the last one an ellipsis.
pub(super) fn ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}
