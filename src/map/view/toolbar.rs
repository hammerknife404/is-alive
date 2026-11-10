//! The controls floating over the Map: the toolbar at the canvas's top left
//! (find, the layout, Scan, undo and redo, the findings, ⋯ for the rest),
//! Layers, Legend and Export at its top right, and what the map needs to
//! say (a plan, the accuracy note, a sweep's progress) under the toolbar.
//! The canvas keeps the whole window; nothing sits in rows above it.

use super::*;

/// Narrower than this, the layouts fold into one drop-down.
const COMPACT: f32 = 980.0;
/// Narrower than this beside a floating inspector (the smallest windows),
/// Layers and Legend go into ⋯ too: there's room for the toolbar only.
const TIGHT: f32 = 460.0;
/// Room around the controls, from the canvas's edges.
const EDGE: f32 = 10.0;

/// What the saved-maps level (`MapTabs`) puts in the toolbar.
#[derive(Default)]
pub(super) struct TabsMenu {
    /// The saved map shown, by name; None on the live map.
    pub saved: Option<String>,
    /// Compare is offered: the live map, not an example.
    pub can_compare: bool,
    /// Saved maps of this site to compare with: label, full name, picked.
    pub compare: Vec<(String, String, bool)>,
    /// The live map is compared with one now.
    pub comparing: bool,
    /// Clear map has something to clear.
    pub clearable: bool,
}

/// What the toolbar asked of the saved-maps level.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum TabsAsk {
    CloseSaved,
    DeleteSaved,
    /// Compare with `TabsMenu::compare[n]`.
    Compare(usize),
    StopCompare,
    Clear,
}

/// A rounded box over the canvas, in the panel's colours.
fn float_frame(ui: &Ui) -> egui::Frame {
    let v = ui.visuals();
    egui::Frame::new()
        .fill(v.panel_fill)
        .stroke(v.widgets.noninteractive.bg_stroke)
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .shadow(egui::Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(60),
        })
}

/// The findings by how bad: (high, medium, low).
fn sev_counts(map: &Map) -> (usize, usize, usize) {
    let n = |s: Sev| map.findings.iter().filter(|f| f.sev == s).count();
    (n(Sev::High), n(Sev::Med), n(Sev::Low))
}

/// The findings chip's words: the worst two kinds there are.
pub(super) fn findings_label((high, med, low): (usize, usize, usize)) -> Option<String> {
    let mut parts = Vec::new();
    for (n, what) in [(high, "high"), (med, "med"), (low, "low")] {
        if n > 0 && parts.len() < 2 {
            parts.push(format!("{n} {what}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

impl MapView {
    /// Every control over the canvas `rect`, drawn after the map so they
    /// sit on top of it. Clicks on them never reach the map.
    pub(super) fn controls(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        src: &Sources,
        empty: bool,
    ) -> Option<Action> {
        // Last frame's boxes take the pointer first: a drag or click on a
        // toolbar's padding doesn't pan the map or clear the selection.
        for (n, r) in std::mem::take(&mut self.float_rects)
            .into_iter()
            .enumerate()
        {
            ui.interact(r, ui.id().with(("map_float", n)), Sense::click_and_drag());
        }
        let mut act = None;
        let right = self.right_controls(ui, rect, empty);
        let room = Rect::from_min_max(
            rect.min + vec2(EDGE, EDGE),
            pos2(
                (right.min.x - 12.0).max(rect.min.x + 200.0),
                rect.max.y - 60.0,
            ),
        );
        let mut col = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(room)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        col.spacing_mut().item_spacing.y = 6.0;
        let bar = float_frame(&col).show(&mut col, |ui| {
            ui.horizontal_wrapped(|ui| {
                act = act.take().or(self.toolbar_items(ui, rect, src, empty));
            });
        });
        self.float_rects.push(bar.response.rect);
        self.float_rects.push(right);
        act = act.or(self.notices(&mut col, src, empty));
        self.clear_dialog(ui.ctx());
        act
    }

    /// The toolbar folds the layouts into a drop-down and Export into ⋯:
    /// when the canvas `rect` is narrow with the inspector open, so picking a
    /// card (which opens it) doesn't change the toolbar.
    fn compact(&self, rect: Rect, empty: bool) -> bool {
        let w = if empty {
            rect.width()
        } else {
            rect.width() + self.panel_now - self.panel_full
        };
        w < COMPACT
    }

    /// Layers, Legend and Export, right-aligned at the top; the rect they took.
    fn right_controls(&mut self, ui: &mut Ui, rect: Rect, empty: bool) -> Rect {
        // Placed by its width last frame (a first guess before that), so
        // it hugs the canvas's right edge and takes no more room than it needs.
        let w = if self.right_w > 0.0 {
            self.right_w
        } else {
            220.0
        };
        // Beside a floating inspector, not under it; in ⋯ when that leaves
        // too little room.
        let beside = self.beside_panel(rect);
        let edge = beside.max.x;
        self.controls_in_menu = !empty && self.cover > 0.0 && beside.width() < TIGHT;
        if self.controls_in_menu {
            return Rect::from_min_size(pos2(edge - EDGE, rect.min.y + EDGE), Vec2::ZERO);
        }
        let at = Rect::from_min_max(
            pos2(edge - EDGE - w, rect.min.y + EDGE),
            pos2(edge - EDGE, rect.min.y + EDGE + 40.0),
        );
        let compact = self.compact(rect, empty);
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(at)
                .layout(egui::Layout::left_to_right(egui::Align::Min)),
        );
        let frame = float_frame(&row).show(&mut row, |ui| {
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!empty, |ui| {
                    if ui
                        .selectable_label(self.side, "Layers")
                        .on_hover_text("Sites, roles to fade out, and the findings, worst first")
                        .on_disabled_hover_text(NOTHING_YET)
                        .clicked()
                    {
                        self.side = !self.side;
                    }
                    if ui
                        .selectable_label(self.legend, "Legend")
                        .on_hover_text("What the colours, lines and badges mean (L)")
                        .on_disabled_hover_text(NOTHING_YET)
                        .clicked()
                    {
                        self.legend = !self.legend;
                    }
                });
                // On a narrow canvas Export is in ⋯ instead.
                if !compact {
                    self.export_button(ui, empty);
                }
            });
        });
        let r = frame.response.rect;
        if (r.width() - self.right_w).abs() > 0.5 {
            self.right_w = r.width();
            ui.ctx().request_repaint();
        }
        r
    }

    /// The toolbar's own items, left to right; what was asked of the app.
    fn toolbar_items(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        src: &Sources,
        empty: bool,
    ) -> Option<Action> {
        let mut act = None;
        let compact = self.compact(rect, empty);
        self.export_in_menu = compact;
        if let Some(name) = self.tabs_menu.saved.clone() {
            self.saved_chip(ui, &name);
            ui.separator();
        }
        // Narrower with text in it: room for the count and ×.
        let width = match (self.search.is_empty(), compact) {
            (true, false) => 200.0,
            (true, true) => 140.0,
            (false, _) => 120.0,
        };
        let r = widgets::line(
            ui,
            crate::input::Field::Text,
            &mut self.search,
            width,
            "Find name, IP, MAC, note",
        )
        .on_hover_text(
            "Finds cards by name, IP, MAC, vendor, details or note; Enter selects the first",
        );
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.search_next = true;
        }
        if !self.search.is_empty() {
            let n = self.map.nodes.iter().filter(|n| self.matches(n)).count();
            ui.weak(format!("{n} match{}", if n == 1 { "" } else { "es" }));
            if ui.small_button("×").on_hover_text("Clear search").clicked() {
                self.search.clear();
            }
        }
        ui.separator();
        // The drop-down lays itself out in a row of its own, which always
        // "fits": on a narrow strip it goes to the next row by hand.
        if compact && ui.available_size_before_wrap().x < 120.0 {
            ui.end_row();
        }
        if let Some(m) = self.layout_picker(ui, compact, empty) {
            self.fit_next = true;
            act = Some(Action::Layout(m));
        }
        ui.separator();
        if !empty && self.scan_button(ui, src) {
            act = Some(Action::Sweep);
        }
        self.history_buttons(ui);
        if let Some(label) = findings_label(sev_counts(&self.map)) {
            ui.separator();
            let (high, ..) = sev_counts(&self.map);
            let colour = if high > 0 {
                widgets::bad()
            } else {
                widgets::warn()
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(format!("⚠ {label}")).color(colour))
                        .selected(self.side)
                        .corner_radius(11),
                )
                .on_hover_text("The findings, worst first: open Layers to see them all")
                .clicked()
            {
                self.side = !self.side;
            }
        }
        ui.menu_button("⋯", |ui| {
            act = act.take().or(self.more_menu(ui, src, empty));
        })
        .response
        .on_hover_text("Clients, Re-layout, Reload, Compare, Examples, Saved maps, Clear map");
        act
    }

    /// The saved map on show: its name, with Close and Delete.
    fn saved_chip(&mut self, ui: &mut Ui, name: &str) {
        let accent = crate::theme::accent();
        let short: String = name.chars().take(28).collect();
        let short = if short.len() < name.len() {
            format!("{short}…")
        } else {
            short
        };
        let b = ui
            .add(
                egui::Button::new(RichText::new(format!("Saved: {short} ⏷")).color(accent))
                    .selected(true)
                    .corner_radius(11),
            )
            .on_hover_text(format!(
                "{name}\nA saved map. Close it to go back to the live map"
            ));
        egui::Popup::menu(&b).show(|ui| {
            if ui
                .button("Close map")
                .on_hover_text("Back to the live map; the saved map stays in Saved maps")
                .clicked()
            {
                self.tabs_ask = Some(TabsAsk::CloseSaved);
                ui.close();
            }
            if ui.button("Delete saved map…").clicked() {
                self.tabs_ask = Some(TabsAsk::DeleteSaved);
                ui.close();
            }
        });
    }

    /// Sites, Cluster, Tree, Zones: in a row, or one drop-down on a narrow
    /// canvas. The one picked, if it changed.
    fn layout_picker(&mut self, ui: &mut Ui, compact: bool, empty: bool) -> Option<Mode> {
        let mut picked = None;
        let now = self.mode();
        ui.add_enabled_ui(!empty, |ui| {
            if compact {
                egui::ComboBox::from_id_salt("map_layout")
                    .width(84.0)
                    .selected_text(now.name())
                    .show_ui(ui, |ui| {
                        for m in Mode::ALL {
                            if ui
                                .selectable_label(now == m, m.name())
                                .on_hover_text(m.tip())
                                .clicked()
                                && now != m
                            {
                                picked = Some(m);
                            }
                        }
                    })
                    .response
                    .on_hover_text("How the map is laid out")
                    .on_disabled_hover_text(NOTHING_YET);
            } else {
                for m in Mode::ALL {
                    if ui
                        .selectable_label(now == m, m.name())
                        .on_hover_text(m.tip())
                        .on_disabled_hover_text(NOTHING_YET)
                        .clicked()
                        && now != m
                    {
                        picked = Some(m);
                    }
                }
            }
        });
        if let Some(m) = picked {
            self.mode = Some(m);
            self.placed.clear();
            self.relayout(ui.ctx());
        }
        picked
    }

    /// Scan: sweep this machine's subnet; true when clicked.
    fn scan_button(&self, ui: &mut Ui, src: &Sources) -> bool {
        let subnet = src
            .info
            .lan
            .map(|(ip, bits)| crate::net::targets::subnet_cidr(ip, bits));
        let subnet = subnet.as_deref().unwrap_or("this machine's subnet");
        let tip = if src.export.example_open() {
            format!("Close the example and sweep {subnet}, filling in the map as hosts answer")
        } else if src.sweep.finished().is_some() {
            format!("Sweep {subnet} again and update the map as hosts answer")
        } else {
            format!("Sweep {subnet} and add what answers to the map")
        };
        ui.add_enabled(
            src.info.lan.is_some() && !src.sweep.is_running(),
            egui::Button::new("Scan"),
        )
        .on_hover_text(&tip)
        .on_disabled_hover_text(if src.sweep.is_running() {
            "A sweep is running"
        } else {
            "No LAN address on this machine"
        })
        .clicked()
    }

    /// ⋯: everything the map does less often.
    fn more_menu(&mut self, ui: &mut Ui, src: &Sources, empty: bool) -> Option<Action> {
        ui.set_min_width(220.0);
        let mut act = None;
        // Beside a floating inspector on the smallest windows.
        if self.controls_in_menu {
            if ui
                .add(egui::Button::selectable(self.side, "Layers"))
                .on_hover_text("Sites, roles to fade out, and the findings, worst first")
                .clicked()
            {
                self.side = !self.side;
                ui.close();
            }
            if ui
                .add(egui::Button::selectable(self.legend, "Legend").shortcut_text("L"))
                .on_hover_text("What the colours, lines and badges mean")
                .clicked()
            {
                self.legend = !self.legend;
                ui.close();
            }
            ui.separator();
        }
        self.clients_menu(ui, src, empty);
        ui.separator();
        let moved = !self.placed.is_empty() || !self.shifted.is_empty();
        if ui
            .add_enabled(moved, egui::Button::new("Re-layout"))
            .on_hover_text("Put dragged cards and boxes back where the layout had them")
            .on_disabled_hover_text("Nothing was dragged")
            .clicked()
        {
            self.relayout_all();
            self.fit_next = true;
            ui.close();
        }
        if ui
            .add_enabled(!empty && self.frozen.is_none(), egui::Button::new("Reload"))
            .on_hover_text(
                "Read the map's files again and rebuild it, keeping pins and the selection. \
                 (It already updates by itself as a sweep finds hosts or a file is loaded.)",
            )
            .on_disabled_hover_text(if empty {
                NOTHING_YET
            } else {
                "A saved map doesn't change with the files: Live has the map they make"
            })
            .clicked()
        {
            act = Some(Action::Reload);
            ui.close();
        }
        if self.tabs_menu.can_compare {
            self.compare_items(ui);
        }
        act = act.or(examples_items(ui, src));
        // Beside Layers and Legend when there's room; here when there isn't.
        if self.export_in_menu {
            ui.add_enabled_ui(!empty, |ui| {
                ui.menu_button("Export", |ui| self.export_items(ui))
                    .response
                    .on_disabled_hover_text(NOTHING_YET);
            });
        }
        if ui
            .add(egui::Button::selectable(self.bar, "Saved maps"))
            .on_hover_text(
                "Show or hide the saved maps along the bottom: keep this map as it is now and \
                 come back to it",
            )
            .clicked()
        {
            self.bar_flip = true;
            ui.close();
        }
        if !self.map.findings.is_empty()
            && ui
                .button("Copy findings")
                .on_hover_text("Copy the findings as text, worst first")
                .clicked()
        {
            let mut s = format!("Findings: {}\n", self.map.summary);
            let mut f: Vec<_> = self.map.findings.iter().collect();
            f.sort_by_key(|x| x.sev);
            for x in f {
                s += &format!("[{}] {}\n", x.sev.label(), x.text);
            }
            widgets::copy(ui.ctx(), s);
            widgets::toast("Findings copied", true);
            ui.close();
        }
        if self.tabs_menu.saved.is_none() {
            ui.separator();
            if ui
                .add_enabled(self.tabs_menu.clearable, egui::Button::new("Clear map…"))
                .on_hover_text(
                    "Close the configs opened and forget the sweep's results, for this session. \
                     Saved maps and the files on disk are kept",
                )
                .on_disabled_hover_text(NOTHING_YET)
                .clicked()
            {
                self.ask_clear = true;
                ui.close();
            }
        }
        act
    }

    /// Clients ▸: how many end devices the map draws.
    fn clients_menu(&mut self, ui: &mut Ui, src: &Sources, empty: bool) {
        // A map with no end devices at all (an infrastructure-only
        // /mapnetwork map) has nothing for the switch to show or hide.
        let any = empty || !self.map.under.is_empty() || self.map.nodes.iter().any(|n| n.client);
        let enabled = any && self.frozen.is_none();
        let why = if self.frozen.is_some() {
            "A saved map keeps the clients it was saved with"
        } else {
            reveal::no_clients_why(src)
        };
        ui.add_enabled_ui(enabled, |ui| {
            ui.menu_button(format!("Clients: {}", self.clients.name()), |ui| {
                for c in Clients::ALL {
                    if ui
                        .selectable_label(self.clients == c, c.name())
                        .on_hover_text(c.tip())
                        .clicked()
                    {
                        self.set_clients(c);
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("How many end devices the map draws under their uplinks")
            .on_disabled_hover_text(why);
        });
    }

    /// Show `c`'s clients, and remember it for the live map.
    fn set_clients(&mut self, c: Clients) {
        self.clients = c;
        if !cfg!(test)
            && std::env::var_os("IS_ALIVE_SHOTS").is_none()
            && let Some(dir) = crate::theme::data_dir()
            && let Err(e) = saved::save_live_clients(&dir, c)
        {
            widgets::toast(format!("Could not save client visibility: {e}"), false);
        }
        // The level sets every uplink again.
        self.reveal.clear();
    }

    /// Compare ▸: the live map against a saved map of this site.
    fn compare_items(&mut self, ui: &mut Ui) {
        let label = if self.tabs_menu.comparing {
            "Compare (on)"
        } else {
            "Compare"
        };
        ui.menu_button(label, |ui| {
            ui.set_min_width(260.0);
            ui.weak("Mark what's new, changed or gone since:");
            for (n, (label, name, on)) in self.tabs_menu.compare.iter().enumerate() {
                if ui
                    .add(egui::Button::selectable(*on, label.as_str()))
                    .on_hover_text(name)
                    .clicked()
                {
                    self.tabs_ask = Some(TabsAsk::Compare(n));
                    ui.close();
                }
            }
            if self.tabs_menu.compare.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "No saved map of this site yet. Saved maps › + Save current keeps \
                             this one, to compare with on the next visit.",
                        )
                        .weak(),
                    )
                    .wrap(),
                );
            }
            if self.tabs_menu.comparing {
                ui.separator();
                if ui.button("Stop comparing").clicked() {
                    self.tabs_ask = Some(TabsAsk::StopCompare);
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(
            "Mark what's new, changed or gone since a saved map of this site: the last visit, say",
        );
    }

    /// Under the toolbar: a plan or training map says so, the accuracy
    /// note, and how far a running sweep has got.
    fn notices(&mut self, ui: &mut Ui, src: &Sources, empty: bool) -> Option<Action> {
        let mut act = None;
        let warn = widgets::warn();
        let note = |ui: &mut Ui| {
            float_frame(ui)
                .fill(ui.visuals().panel_fill)
                .stroke(Stroke::new(1.0, warn.gamma_multiply(0.5)))
        };
        // A plan or a training map says so first: it isn't what was found.
        if let Some(c) = self.purpose.caption() {
            let r = note(ui).show(ui, |ui| {
                ui.label(RichText::new(c).strong().color(warn));
            });
            self.float_rects.push(r.response.rect);
        }
        if !empty && src.note && src.export.mapnetwork().is_none() && self.frozen.is_none() {
            let r = note(ui).show(ui, |ui| {
                ui.set_max_width(460.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").color(warn));
                    ui.add(
                        egui::Label::new(
                            "A quick picture from what this app can see. For an accurate map, \
                             run /mapnetwork on a Site Bundle.",
                        )
                        .wrap(),
                    );
                });
                ui.horizontal(|ui| {
                    if ui
                        .small_button("Site Bundle…")
                        .on_hover_text("Zip it all for /mapnetwork")
                        .clicked()
                    {
                        act = Some(Action::Bundle);
                    }
                    if ui
                        .small_button("Don't show again")
                        .on_hover_text("Settings › Map brings it back")
                        .clicked()
                    {
                        act = Some(Action::HideNote);
                    }
                });
            });
            self.float_rects.push(r.response.rect);
        }
        // A sweep fills the map in as it runs: show how far along it is.
        if let Some((done, total, since, range)) = src.sweep.progress() {
            let r = float_frame(ui).show(ui, |ui| {
                ui.horizontal(|ui| {
                    widgets::spinner(ui);
                    ui.label(format!("Scanning {range}"));
                    if total > 0 {
                        widgets::progress(ui, done, total, since);
                    } else {
                        ui.weak("starting…");
                    }
                });
            });
            self.float_rects.push(r.response.rect);
        }
        act
    }

    /// "Clear the map?", asked from ⋯ › Clear map….
    fn clear_dialog(&mut self, ctx: &egui::Context) {
        if !self.ask_clear {
            return;
        }
        let mut done = false;
        let r = egui::Modal::new(egui::Id::new("map_clear")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.strong("Clear the map?");
            ui.label(
                "Closes the configs opened and forgets the sweep's results, for this session. \
                 Saved maps and the files on disk are kept, and Notes keeps hiding the \
                 configs' secrets.",
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Clear map").color(widgets::bad()))
                    .clicked()
                {
                    self.tabs_ask = Some(TabsAsk::Clear);
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
        if done || r.should_close() {
            self.ask_clear = false;
        }
    }
}

/// Examples ▸: the example maps, and Close the example when one is open.
fn examples_items(ui: &mut Ui, src: &Sources) -> Option<Action> {
    let mut picked = None;
    // The example on the map now, ticked; picking it again reads it again.
    let shown = src
        .export
        .mapnetwork()
        .and_then(|m| m.0.file_name().map(|f| f.to_os_string()));
    ui.menu_button("Examples", |ui| {
        ui.weak("Fictional sites, as /mapnetwork maps");
        for (i, (label, f)) in crate::skills::EXAMPLES.iter().enumerate() {
            let blank = *f == crate::skills::BLANK;
            if blank {
                ui.separator();
            }
            let on =
                shown.is_some() && shown.as_deref() == crate::skills::example_path(i).file_name();
            if ui
                .selectable_label(on, *label)
                .on_hover_text(example_tip(blank))
                .clicked()
            {
                picked = Some(Action::Example(i));
                ui.close();
            }
        }
        if src.export.example_open() {
            ui.separator();
            if ui
                .button("Close the example")
                .on_hover_text("Back to this network's map (a Scan closes it too)")
                .clicked()
            {
                picked = Some(Action::CloseExample);
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Open an example map (a fictional site drawn by /mapnetwork) or a blank one");
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of the Map tab on a 1280 × 720 laptop screen, showing
    /// example 0; every text drawn, with where.
    fn frame(
        ctx: &egui::Context,
        v: &mut MapView,
        src: &Sources,
        events: Vec<egui::Event>,
    ) -> Vec<(String, Rect)> {
        frame_at(ctx, v, src, events, vec2(1280.0, 720.0))
    }

    /// `frame`, on a screen `size` points.
    fn frame_at(
        ctx: &egui::Context,
        v: &mut MapView,
        src: &Sources,
        events: Vec<egui::Event>,
        size: Vec2,
    ) -> Vec<(String, Rect)> {
        fn texts(s: &Shape, out: &mut Vec<(String, Rect)>) {
            match s {
                Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
                Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let input = egui::RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(ctx.input(|i| i.time) + 0.1),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            v.ui(ui, Sources { ..*src });
        });
        out.textures_delta.clear();
        let mut found = Vec::new();
        for c in &out.shapes {
            texts(&c.shape, &mut found);
        }
        found
    }

    fn click(ctx: &egui::Context, v: &mut MapView, src: &Sources, at: Pos2) -> Vec<(String, Rect)> {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(ctx, v, src, vec![egui::Event::PointerMoved(at)]);
        frame(ctx, v, src, vec![press(true)]);
        frame(ctx, v, src, vec![press(false)]);
        frame(ctx, v, src, vec![]);
        frame(ctx, v, src, vec![])
    }

    fn at(t: &[(String, Rect)], label: &str) -> Pos2 {
        t.iter()
            .find(|(s, _)| s == label)
            .unwrap_or_else(|| {
                panic!(
                    "no {label:?} in {:?}",
                    t.iter().map(|x| &x.0).collect::<Vec<_>>()
                )
            })
            .1
            .center()
    }

    /// Everything that left the rows above the map is still there: in
    /// the toolbar, under ⋯, or behind Layers, on a laptop-sized window.
    #[test]
    fn what_left_the_rows_above_the_map_is_one_click_away() {
        let ctx = egui::Context::default();
        let sweep = crate::tools::sweep::SweepTool::default();
        let export = crate::tools::export::ExportTool::with_example(0);
        let info = crate::net::hostinfo::HostInfo::default();
        let ov = crate::redact::Overrides::default();
        let src = Sources {
            sweep: &sweep,
            export: &export,
            info: &info,
            busy: false,
            note: false,
            legend: false,
            redact: &ov,
            animate: false,
        };
        let mut v = MapView::new(None);
        let mut shown = Vec::new();
        for _ in 0..4 {
            shown = frame(&ctx, &mut v, &src, vec![]);
        }
        // The map opens clean: no Layers panel, no legend.
        assert!(!v.side && !v.legend);
        // On a laptop the layouts are one drop-down (it shows the one in
        // use), so the toolbar has room beside a card's inspector.
        for label in ["Cluster", "Layers", "Legend", "⋯"] {
            at(&shown, label);
        }
        assert!(
            shown.iter().any(|(s, _)| s.contains(" high")),
            "the findings chip"
        );
        // The toolbar sits on the canvas's top edge: nothing is above the map.
        assert!(at(&shown, "Cluster").y < 60.0);
        let menu = click(&ctx, &mut v, &src, at(&shown, "⋯"));
        for label in [
            "Re-layout",
            "Reload",
            "Examples",
            "Export",
            "Saved maps",
            "Copy findings",
            "Clear map…",
        ] {
            at(&menu, label);
        }
        assert!(menu.iter().any(|(s, _)| s.starts_with("Clients: ")));
        frame(
            &ctx,
            &mut v,
            &src,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let shown = frame(&ctx, &mut v, &src, vec![]);
        let after = click(&ctx, &mut v, &src, at(&shown, "Layers"));
        assert!(v.side, "Layers opens the panel");
        at(&after, "ROLES");
    }

    /// Sweep's On map: the card at that address is selected, and an
    /// address the map hasn't got selects nothing.
    #[test]
    fn a_card_asked_for_by_address_is_selected() {
        let ctx = egui::Context::default();
        let sweep = crate::tools::sweep::SweepTool::default();
        let export = crate::tools::export::ExportTool::with_example(0);
        let info = crate::net::hostinfo::HostInfo::default();
        let ov = crate::redact::Overrides::default();
        let src = Sources {
            sweep: &sweep,
            export: &export,
            info: &info,
            busy: false,
            note: false,
            legend: false,
            redact: &ov,
            animate: false,
        };
        let mut v = MapView::new(None);
        frame(&ctx, &mut v, &src, vec![]);
        let (i, ip) = v
            .map
            .nodes
            .iter()
            .enumerate()
            .find_map(|(i, n)| n.ip.map(|ip| (i, ip)))
            .expect("a card with an address");
        v.pick_ip = Some(ip);
        frame(&ctx, &mut v, &src, vec![]);
        assert_eq!(v.selection(), vec![i]);
        v.select(None);
        v.pick_ip = Some(std::net::Ipv4Addr::new(192, 0, 2, 250));
        frame(&ctx, &mut v, &src, vec![]);
        assert!(v.selection().is_empty());
        assert!(v.pick_ip.is_none(), "asked once");
    }

    /// Example 0's files, with no host info and the default redaction.
    fn example() -> (
        crate::tools::export::ExportTool,
        crate::net::hostinfo::HostInfo,
        crate::redact::Overrides,
    ) {
        (
            crate::tools::export::ExportTool::with_example(0),
            crate::net::hostinfo::HostInfo::default(),
            crate::redact::Overrides::default(),
        )
    }

    /// A card with an address on example 0, not the internet.
    fn a_device(v: &MapView) -> usize {
        v.map
            .nodes
            .iter()
            .position(|n| n.ip.is_some() && n.zone != Zone::Internet)
            .expect("a device")
    }

    /// On a laptop the inspector floats over the map: the canvas keeps its
    /// width and the map its zoom, and the controls on the right move
    /// beside it rather than under it.
    #[test]
    fn a_cards_inspector_floats_over_the_map_on_a_laptop() {
        let ctx = egui::Context::default();
        let sweep = crate::tools::sweep::SweepTool::default();
        let (export, info, ov) = example();
        let src = Sources {
            sweep: &sweep,
            export: &export,
            info: &info,
            busy: false,
            note: false,
            legend: false,
            redact: &ov,
            animate: false,
        };
        let mut v = MapView::new(None);
        let mut shown = Vec::new();
        for _ in 0..6 {
            shown = frame(&ctx, &mut v, &src, vec![]);
        }
        let zoom = v.zoom;
        let layers = at(&shown, "Layers");
        assert!(
            at(&shown, "Fit").x > 1280.0 - 120.0,
            "zoom controls in the corner"
        );
        let i = a_device(&v);
        v.select(Some(i));
        for _ in 0..12 {
            shown = frame(&ctx, &mut v, &src, vec![]);
        }
        // Its inspector: the name, and what can be run against it.
        at(&shown, &v.map.nodes[i].label);
        for label in ["Ping", "Port Check", "Traceroute", "DETAILS"] {
            at(&shown, label);
        }
        // Over the canvas, not beside it.
        assert_eq!(v.panel_now, 0.0);
        assert_eq!(v.cover, 320.0);
        assert!(
            (v.zoom - zoom).abs() < 1e-3,
            "zoom {zoom} became {}",
            v.zoom
        );
        // Layers, Legend and the zoom controls sit left of it.
        let edge = 1280.0 - 320.0;
        assert!(at(&shown, "Layers").x < edge && at(&shown, "Layers").x < layers.x);
        assert!(at(&shown, "Legend").x < edge);
        assert!(at(&shown, "Fit").x < edge);
        // Esc lets the card go and the inspector shuts.
        frame(
            &ctx,
            &mut v,
            &src,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let shown = frame(&ctx, &mut v, &src, vec![]);
        assert!(v.selection().is_empty());
        assert!(!shown.iter().any(|(s, _)| s == "Traceroute"));
        assert_eq!(v.cover, 0.0);
    }

    /// From 1440 points wide it docks beside the canvas; the map keeps its
    /// zoom all the same.
    #[test]
    fn a_cards_inspector_docks_on_a_wide_window() {
        let ctx = egui::Context::default();
        let sweep = crate::tools::sweep::SweepTool::default();
        let (export, info, ov) = example();
        let src = Sources {
            sweep: &sweep,
            export: &export,
            info: &info,
            busy: false,
            note: false,
            legend: false,
            redact: &ov,
            animate: false,
        };
        let size = vec2(1600.0, 900.0);
        let mut v = MapView::new(None);
        for _ in 0..6 {
            frame_at(&ctx, &mut v, &src, vec![], size);
        }
        let zoom = v.zoom;
        v.select(Some(a_device(&v)));
        let mut shown = Vec::new();
        for _ in 0..12 {
            shown = frame_at(&ctx, &mut v, &src, vec![], size);
        }
        at(&shown, "Traceroute");
        assert!((v.panel_now - 360.0).abs() < 1.0, "docked: {}", v.panel_now);
        assert_eq!(v.cover, 0.0);
        assert!(
            (v.zoom - zoom).abs() < 1e-3,
            "zoom {zoom} became {}",
            v.zoom
        );
    }

    /// On a laptop, opening Layers shuts the floating inspector, the card
    /// still selected; picking a card opens it again, and so does shutting
    /// Layers.
    #[test]
    fn layers_shuts_a_floating_inspector_and_keeps_the_selection() {
        let ctx = egui::Context::default();
        let sweep = crate::tools::sweep::SweepTool::default();
        let (export, info, ov) = example();
        let src = Sources {
            sweep: &sweep,
            export: &export,
            info: &info,
            busy: false,
            note: false,
            legend: false,
            redact: &ov,
            animate: false,
        };
        let mut v = MapView::new(None);
        let mut shown = Vec::new();
        for _ in 0..4 {
            shown = frame(&ctx, &mut v, &src, vec![]);
        }
        let i = a_device(&v);
        v.select(Some(i));
        for _ in 0..4 {
            shown = frame(&ctx, &mut v, &src, vec![]);
        }
        assert!(shown.iter().any(|(s, _)| s == "Traceroute"));
        let shown = click(&ctx, &mut v, &src, at(&shown, "Layers"));
        assert!(v.side, "Layers opens");
        assert_eq!(v.selection(), vec![i], "the card stays selected");
        assert!(
            !shown.iter().any(|(s, _)| s == "Traceroute"),
            "inspector shut"
        );
        assert_eq!(v.cover, 0.0);
        // A card picked with Layers open: its inspector shows.
        v.select(Some(i));
        let shown = frame(&ctx, &mut v, &src, vec![]);
        assert!(shown.iter().any(|(s, _)| s == "Traceroute"));
        // Hidden again, then Layers shut: it comes back.
        v.insp_hidden = true;
        v.side = false;
        let shown = frame(&ctx, &mut v, &src, vec![]);
        assert!(shown.iter().any(|(s, _)| s == "Traceroute"));
    }

    #[test]
    fn the_findings_chip_names_the_worst_two_kinds() {
        assert_eq!(findings_label((0, 0, 0)), None);
        assert_eq!(findings_label((2, 4, 2)).as_deref(), Some("2 high · 4 med"));
        assert_eq!(findings_label((0, 3, 1)).as_deref(), Some("3 med · 1 low"));
        assert_eq!(findings_label((1, 0, 0)).as_deref(), Some("1 high"));
        assert_eq!(findings_label((0, 0, 5)).as_deref(), Some("5 low"));
    }
}
