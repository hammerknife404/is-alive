//! The card panel beside the map: a card's details, and each card's runs
//! and results.

use super::*;

impl MapView {
    /// The detail panel: every selected card, stacked, the one clicked last
    /// on top. Each has /mapnetwork's sections: action items, details and
    /// connections, then what can be run against it. `menus` gets each
    /// card's ☰ button, for `card_menus` to open the right-click menu from.
    /// `edit` is the note being written, shown in its card.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn detail(
        &self,
        ui: &mut Ui,
        mut sel: Vec<usize>,
        busy: bool,
        pick: &mut Option<(usize, bool)>,
        ev: &mut Option<PanelEv>,
        menus: &mut Vec<(usize, egui::Response)>,
        edit: &mut Option<notes::Edit>,
    ) -> Option<Action> {
        let mut act = None;
        // Sliding shut: the last card shown, without the header.
        let closing = sel.is_empty();
        if closing {
            sel.extend(self.shown.as_deref().and_then(|s| self.index(s)));
        }
        egui::ScrollArea::vertical()
            .id_salt("map_detail_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add_space(4.0);
                // A card clicked on its own already lights its path to the
                // internet: Trace to WAN is for cards picked together
                // (Ctrl+click, the right-click menu, a site, a finding) and
                // pinned cards left on their own, which light only what
                // joins them.
                if !closing && self.joins_only() {
                    ui.horizontal(|ui| self.panel_header(ui, &sel, ev));
                }
                ui.add_space(4.0);
                // Where the stack starts, so a card's place in it doesn't
                // change as the panel scrolls.
                let origin = ui.min_rect().top();
                for (k, &i) in sel.iter().enumerate() {
                    if k > 0 {
                        ui.add_space(6.0);
                    }
                    let layer = ui.layer_id();
                    let first = ui.ctx().graphics(|g| g.get(layer).map(|l| l.next_idx()));
                    let (a, frame, menu) =
                        self.card_panel(ui, k, i, busy, sel.len() > 1, pick, ev, edit);
                    act = act.take().or(a);
                    menus.extend(menu.map(|m| (i, m)));
                    // A card whose place in the stack changed (moved, or
                    // pushed down by a new one) glides there.
                    let at = frame.rect.top() - origin;
                    let shown = slide(ui.ctx(), &self.map.nodes[i].id, at);
                    if (shown - at).abs() > 0.1
                        && let Some(first) = first
                    {
                        // Shapes move; their clipping stays inside the
                        // panel, so a gliding card never draws over the
                        // header or outside the panel.
                        let t = egui::emath::TSTransform::from_translation(vec2(0.0, shown - at));
                        let view = ui.clip_rect();
                        ui.ctx().graphics_mut(|g| {
                            let list = g.entry(layer);
                            for n in first.0..list.next_idx().0 {
                                list.mutate_shape(egui::layers::ShapeIdx(n), |c| {
                                    c.shape.transform(t);
                                    c.clip_rect = t.mul_rect(c.clip_rect).intersect(view);
                                });
                            }
                        });
                    }
                    // Drop another card's grip here to put it in this place.
                    if let Some(from) = frame.dnd_release_payload::<usize>() {
                        *ev = Some(PanelEv::Move(*from, k));
                    } else if frame.dnd_hover_payload::<usize>().is_some_and(|f| *f != k) {
                        ui.painter().hline(
                            frame.rect.x_range(),
                            frame.rect.top() - 3.0,
                            Stroke::new(2.0, crate::theme::accent()),
                        );
                    }
                }
            });
        act
    }

    /// Each card's ☰ in the panel: the card's right-click menu, the very
    /// same one (on one of several selected cards it acts on them all).
    pub(super) fn card_menus(&mut self, menus: Vec<(usize, egui::Response)>) {
        let mut open = None;
        for (i, button) in menus {
            // Scrolled out of the panel: the menu would hang loose, with
            // nothing to show which card it's for, so it closes.
            if !button.interact_rect.is_positive() {
                egui::Popup::close_id(&button.ctx, egui::Popup::default_response_id(&button));
                continue;
            }
            if egui::Popup::menu(&button)
                .show(|ui| self.card_menu(ui, i))
                .is_some()
            {
                open = Some(i);
            }
        }
        match open {
            Some(i) => self.menu = Some(Menu::Panel(self.map.nodes[i].id.clone())),
            None if matches!(self.menu, Some(Menu::Panel(_))) => self.menu = None,
            None => {}
        }
    }

    /// Over the stack: how many are selected, Trace to WAN, Copy all.
    fn panel_header(&self, ui: &mut Ui, sel: &[usize], ev: &mut Option<PanelEv>) {
        if sel.len() > 1 {
            ui.strong(format!("{} selected", sel.len()))
                .on_hover_text("Ctrl+click a card to add or remove it; Esc clears");
        }
        let mut trace = self.trace_wan;
        if ui
            .checkbox(&mut trace, "Trace to WAN")
            .on_hover_text(
                "Light each selected card's whole path to the internet. Off, only the links \
                 that join the selected cards light (one card: its uplink and its children). \
                 A card clicked on its own always shows its path",
            )
            .changed()
        {
            *ev = Some(PanelEv::TraceWan(trace));
        }
        if sel.len() > 1 {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("Copy all")
                    .on_hover_text("Copy every selected card's details as text")
                    .clicked()
                {
                    let s: Vec<String> =
                        sel.iter().map(|&k| card_text(&self.map.nodes[k])).collect();
                    widgets::copy(ui.ctx(), s.join("\n"));
                    widgets::toast(format!("{} cards copied", sel.len()), true);
                }
            });
        }
    }

    /// One card's section in the detail panel, `at` from the top; with its
    /// ☰ button.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn card_panel(
        &self,
        ui: &mut Ui,
        at: usize,
        i: usize,
        busy: bool,
        stacked: bool,
        pick: &mut Option<(usize, bool)>,
        ev: &mut Option<PanelEv>,
        edit: &mut Option<notes::Edit>,
    ) -> (Option<Action>, egui::Response, Option<egui::Response>) {
        let mut act = None;
        let mut menu = None;
        let n = &self.map.nodes[i];
        let accent = self.accent(n);
        // Every section: a small caps title, then body-size text, keys in a
        // fixed column so values line up from one section to the next.
        const KEY_W: f32 = 76.0;
        let section = |ui: &mut Ui, title: &str| {
            ui.add_space(8.0);
            ui.label(
                RichText::new(title.to_uppercase())
                    .small()
                    .strong()
                    .color(ui.visuals().weak_text_color()),
            );
        };
        let kept = self.is_kept(i);
        let frame = egui::Frame::group(ui.style())
            .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.5)))
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Header: a grip to drag it up or down, the kind, then ☰,
                // pin and ×.
                ui.horizontal(|ui| {
                    if stacked {
                        ui.dnd_drag_source(ui.id().with(("map_grip", i)), at, |ui| {
                            // Hover only: the drag belongs to the source around it.
                            let (r, resp) =
                                ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
                            let c = ui.visuals().weak_text_color();
                            for row in 0..3 {
                                for col in 0..2 {
                                    ui.painter().circle_filled(
                                        r.min
                                            + vec2(2.5 + col as f32 * 5.0, 3.0 + row as f32 * 5.0),
                                        1.3,
                                        c,
                                    );
                                }
                            }
                            resp.on_hover_text("Drag to move it up or down");
                            if ui.response().hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                        });
                    }
                    ui.label(RichText::new(&n.kind).monospace().small().color(accent));
                    if self.note_of(i).is_some() {
                        let (r, resp) = ui.allocate_exact_size(vec2(9.0, 11.0), Sense::hover());
                        paint::note_glyph(ui.painter(), ui.visuals(), r, 1.0);
                        resp.on_hover_text("Has a note (below)");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("×")
                            .on_hover_text(if stacked {
                                "Take it out of the selection"
                            } else {
                                "Close (Esc)"
                            })
                            .clicked()
                        {
                            *ev = Some(PanelEv::Drop(i));
                        }
                        let pin = RichText::new("📌")
                            .color(crate::theme::emoji_colour('📌', ui.visuals().text_color()));
                        let pin = ui
                            .add(egui::Button::new(pin).small().selected(kept))
                            .on_hover_text(if kept {
                                "Pinned: stays selected and keeps its place here. Click to unpin"
                            } else {
                                "Pin: keep it selected and in this place in the panel"
                            });
                        if pin.clicked() {
                            *ev = Some(PanelEv::Keep(i));
                        }
                        // ☰ the size of the pin, its three lines drawn: the
                        // font's ☰ is a blur at this size.
                        let b = ui.add(egui::Button::new("").small().min_size(pin.rect.size()));
                        let open =
                            egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&b));
                        let stroke = if open {
                            Stroke::new(1.5, crate::theme::accent())
                        } else {
                            Stroke::new(1.5, ui.style().interact(&b).fg_stroke.color)
                        };
                        let c = b.rect.center();
                        for dy in [-3.5, 0.0, 3.5] {
                            ui.painter().hline(c.x - 5.0..=c.x + 5.0, c.y + dy, stroke);
                        }
                        // Its name for screen readers, as the lines aren't text.
                        b.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Card menu")
                        });
                        menu = Some(b.on_hover_text("Menu (same as right-clicking the card)"));
                    });
                });
                ui.label(RichText::new(&n.label).size(17.0).strong());
                if !n.sub.is_empty() {
                    ui.label(RichText::new(&n.sub).monospace().color(accent));
                }
                ui.horizontal_wrapped(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.5, accent);
                    ui.label(RichText::new(n.zone.name()).color(accent));
                    for c in &n.chips {
                        ui.label(RichText::new(c).small().weak());
                    }
                });
                if let Some((p, to)) = self.reveal_row(ui, i) {
                    *ev = Some(PanelEv::Reveal(p, to));
                }

                // Action items: its own flags and the findings that name it.
                let findings: Vec<&crate::map::Finding> = self
                    .map
                    .findings
                    .iter()
                    .filter(|f| f.nodes.contains(&i))
                    .collect();
                if !n.flags.is_empty() || !findings.is_empty() {
                    section(ui, "Action items");
                    for f in &n.flags {
                        widgets::warning(ui, f);
                    }
                    for f in findings {
                        let c = match f.sev {
                            Sev::High => widgets::bad(),
                            Sev::Med => widgets::warn(),
                            Sev::Low => ui.visuals().weak_text_color(),
                        };
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui_with_layout(
                                vec2(34.0, ui.spacing().interact_size.y),
                                egui::Layout::left_to_right(egui::Align::Min),
                                |ui| {
                                    ui.set_min_width(34.0 - ui.spacing().item_spacing.x);
                                    ui.label(RichText::new(f.sev.label()).monospace().color(c));
                                },
                            );
                            ui.add(egui::Label::new(&f.text).wrap());
                        });
                    }
                }

                // New or changed since the saved map compared with.
                self.since_panel(ui, i);

                // Details: what's known about it, then the source's notes.
                section(ui, "Details");
                let mut facts: Vec<(&str, String)> = Vec::new();
                if let Some(ip) = n.ip {
                    facts.push(("IP", ip.to_string()));
                }
                // A detail line already says more about the MAC (vendor, private).
                if let Some(mac) = n.mac
                    && !n.details.iter().any(|d| d.starts_with("MAC "))
                {
                    facts.push(("MAC", crate::net::mac::fmt(&mac)));
                }
                if !n.open.is_empty() {
                    let ports: Vec<String> = n.open.iter().map(u16::to_string).collect();
                    facts.push(("Open ports", ports.join(", ")));
                }
                if !n.seen_by.is_empty() {
                    facts.push(("Seen by", n.seen_by.join(", ")));
                }
                egui::Grid::new(("map_facts", i))
                    .num_columns(2)
                    .min_col_width(KEY_W - 8.0)
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        for (k, v) in &facts {
                            ui.weak(*k);
                            ui.add(egui::Label::new(RichText::new(v).monospace()).selectable(true));
                            ui.end_row();
                        }
                    });
                for d in drawn_note(n)
                    .into_iter()
                    .chain(n.details.iter().map(String::as_str))
                {
                    ui.horizontal_top(|ui| {
                        ui.weak("·");
                        ui.add(egui::Label::new(d).wrap().selectable(true));
                    });
                }
                if facts.is_empty() && n.details.is_empty() && drawn_note(n).is_none() {
                    ui.weak("Nothing more is known about it.");
                }

                // The technician's note, or its editor.
                let mut note_ev = None;
                self.note_section(ui, i, edit, &mut note_ev);
                if let Some(e) = note_ev {
                    *ev = Some(PanelEv::Note(e));
                }

                // Connections: up to its parent, down to what hangs off it.
                let kids: Vec<usize> = (0..self.map.nodes.len())
                    .filter(|&k| self.map.nodes[k].parent() == Some(i))
                    .collect();
                if n.up.is_some() || !kids.is_empty() {
                    section(ui, "Connections");
                }
                // A name links to its card: a click shows it, Ctrl+click
                // adds it to the selection.
                let row =
                    |ui: &mut Ui, k: usize, via: String, tip: &str| -> Option<(usize, bool)> {
                        let m = &self.map.nodes[k];
                        let c = zone_colour(m.zone);
                        let mut go = None;
                        ui.horizontal(|ui| {
                            let (r, _) =
                                ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, c);
                            if ui.link(&m.label).on_hover_text(tip).clicked() {
                                let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
                                go = Some((k, add));
                            }
                            ui.weak(via);
                        });
                        go
                    };
                if let Some((p, link, port)) = &n.up {
                    let how = match link {
                        Link::Backbone => "uplink · WAN".to_string(),
                        Link::Wired => format!("uplink · {}", port.as_deref().unwrap_or("wired")),
                        Link::Wireless => "uplink · wireless".into(),
                        Link::Assumed => "uplink · assumed".into(),
                    };
                    let tip = match link {
                        Link::Assumed => {
                            "Show it (Ctrl+click adds it). Assumed: it answered here, but its \
                             switch port isn't known"
                        }
                        _ => SHOW_IT,
                    };
                    if let Some(go) = row(ui, *p, how, tip) {
                        *pick = Some(go);
                    }
                }
                for &k in kids.iter().take(14) {
                    let via = match &self.map.nodes[k].up {
                        Some((_, Link::Wireless, _)) => "wireless".to_string(),
                        Some((_, Link::Assumed, _)) => "assumed".to_string(),
                        Some((_, _, Some(port))) => port.clone(),
                        _ => "below it".to_string(),
                    };
                    if let Some(go) = row(ui, k, via, SHOW_IT) {
                        *pick = Some(go);
                    }
                }
                if kids.len() > 14 {
                    ui.weak(format!("…{} more below it", kids.len() - 14));
                }

                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if n.ip.is_some() {
                        for (tool, label, hover) in [
                            (
                                Tool::Ping,
                                "Ping",
                                "Ping it 4 times; the result shows below",
                            ),
                            (
                                Tool::Ports,
                                "Ports",
                                "Check its common ports; the result shows below",
                            ),
                            (
                                Tool::Trace,
                                "Trace",
                                "Trace the route to it; the result shows below",
                            ),
                        ] {
                            if ui
                                .add_enabled(
                                    !self.runs.active(&n.id, tool),
                                    egui::Button::new(label).small(),
                                )
                                .on_hover_text(hover)
                                .on_disabled_hover_text("Running: see below")
                                .clicked()
                            {
                                *ev = Some(PanelEv::Run(i, tool));
                            }
                        }
                    }
                    // Web only when a web port is known to be open.
                    if let Some(ip) = n.ip
                        && interact::web_open(&n.open)
                    {
                        let url = interact::web_url(ip.into(), &n.open);
                        if ui
                            .small_button("Web")
                            .on_hover_text(format!("Open {url} in your browser"))
                            .clicked()
                        {
                            widgets::open_url(url);
                        }
                    }
                    // SSH and RDP only when the port was found open and this
                    // computer has a client for it.
                    if let Some(ip) = n.ip {
                        for kind in [crate::connect::Kind::Ssh, crate::connect::Kind::Rdp] {
                            if !n.open.contains(&kind.port()) {
                                continue;
                            }
                            let Some(plan) = crate::connect::plan(kind, ip) else {
                                continue;
                            };
                            if ui
                                .small_button(kind.label())
                                .on_hover_text(format!(
                                    "Opens {}. It asks for the log-in; nothing is stored.",
                                    plan.what
                                ))
                                .clicked()
                            {
                                crate::connect::launch(plan);
                            }
                        }
                    }
                    // Wake when its card's MAC is known and a real one.
                    if let Some(mac) = n.mac.filter(crate::net::wol::wakeable)
                        && ui
                            .small_button("Wake")
                            .on_hover_text(format!(
                                "Send a Wake-on-LAN packet to {}. It wakes if Wake-on-LAN is \
                                 on in its BIOS and network card; Ping it a minute later.",
                                crate::net::mac::fmt(&mac)
                            ))
                            .clicked()
                    {
                        crate::net::wol::wake_all(vec![(mac, n.ip)]);
                    }
                    if ui
                        .small_button("Copy")
                        .on_hover_text("Copy this card's details as text")
                        .clicked()
                    {
                        widgets::copy(ui.ctx(), card_text(n));
                        widgets::toast("Card details copied", true);
                    }
                    let writing = edit.as_ref().is_some_and(|e| e.ids.contains(&n.id));
                    if self.note_of(i).is_none()
                        && !writing
                        && ui
                            .small_button("Add note")
                            .on_hover_text("Write a note on this card (N)")
                            .clicked()
                    {
                        *ev = Some(PanelEv::Note(notes::NoteEv::Open(i)));
                    }
                });

                let runs = self.runs.runs(&n.id);
                let reveal = revealing(ui.ctx(), &n.id);
                if !runs.is_empty() {
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new(
                        RichText::new("RESULTS")
                            .small()
                            .strong()
                            .color(ui.visuals().weak_text_color()),
                    )
                    .id_salt(("map_runs", &n.id))
                    .default_open(true)
                    .open(reveal.map(|_| true))
                    .show(ui, |ui| {
                        for r in runs {
                            let top = ui.cursor().top();
                            let e = run_section(ui, &n.id, r, busy);
                            if reveal == Some(r.tool) {
                                let rect = Rect::from_x_y_ranges(
                                    ui.max_rect().x_range(),
                                    top..=ui.min_rect().bottom(),
                                );
                                ui.scroll_to_rect(rect, Some(egui::Align::Center));
                                ui.ctx().data_mut(|d| d.remove::<Reveal>(reveal_id()));
                            }
                            if let Some(e) = e {
                                match e {
                                    RunEv::Again => *ev = Some(PanelEv::Run(i, r.tool)),
                                    RunEv::Stop => *ev = Some(PanelEv::Stop(i, r.tool)),
                                    RunEv::Clear => *ev = Some(PanelEv::Clear(i, r.tool)),
                                    RunEv::Tab => {
                                        act = Some(tab(r.tool, r.ip.to_string()));
                                    }
                                }
                            }
                        }
                    });
                }
            });
        (act, frame.response, menu)
    }
}

impl MapView {
    /// Start a quick `tool` run on card `i`, its result in the card's
    /// section of the panel: the panel's Ping, Ports and Trace, and the
    /// card menu's Test items.
    pub(super) fn run_card(&mut self, ctx: &egui::Context, i: usize, tool: Tool) {
        let n = &self.map.nodes[i];
        if let Some(ip) = n.ip {
            self.runs.start(&n.id, tool, ip.into());
            self.runs.poll(ctx);
        }
    }
}

/// A result for the panel to scroll to: the card, the test, and when it
/// was asked for.
type Reveal = (String, Tool, f64);

fn reveal_id() -> egui::Id {
    egui::Id::new("map_reveal")
}

/// Scroll the panel to card `id`'s `tool` result the next time it's
/// drawn: a test started from the card menu, which can be picked with the
/// panel scrolled anywhere, or shut.
pub(super) fn reveal(ctx: &egui::Context, id: &str, tool: Tool) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp::<Reveal>(reveal_id(), (id.to_string(), tool, now)));
}

/// The result of card `id` the panel is to scroll to, if any. Asked for
/// over two seconds ago, it's dropped: the card wasn't shown, and the
/// panel shouldn't jump to it later.
fn revealing(ctx: &egui::Context, id: &str) -> Option<Tool> {
    let now = ctx.input(|i| i.time);
    ctx.data(|d| d.get_temp::<Reveal>(reveal_id()))
        .filter(|(card, _, at)| card == id && now - at < 2.0)
        .map(|(_, tool, _)| tool)
}

/// What opens a card's IP in a tool's own tab, for a full run.
pub(super) fn tab(tool: Tool, ip: String) -> Action {
    match tool {
        Tool::Ping => Action::Ping(ip),
        Tool::Ports => Action::Ports(ip),
        Tool::Trace => Action::Trace(ip),
    }
}

/// A card's name in the panel's Connections.
const SHOW_IT: &str = "Show it (Ctrl+click adds it to the selection)";

/// What a run's section in a card asked for.
pub(super) enum RunEv {
    Again,
    Stop,
    Clear,
    Tab,
}

/// One run in a card's Results: the tool, its condensed line and buttons;
/// opened, every reply, port or hop.
pub(super) fn run_section(ui: &mut Ui, card: &str, r: &Run, busy: bool) -> Option<RunEv> {
    let mut out = None;
    let id = ui.make_persistent_id(("map_run", card, r.tool));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
        .show_header(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(RichText::new(r.tool.name()).small().strong());
            if r.active() {
                widgets::spinner(ui);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Clear this result")
                    .clicked()
                {
                    out = Some(RunEv::Clear);
                }
                if r.active() {
                    if ui.small_button("■").on_hover_text("Stop").clicked() {
                        out = Some(RunEv::Stop);
                    }
                } else if ui.small_button("⟲").on_hover_text("Run it again").clicked() {
                    out = Some(RunEv::Again);
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Open in tab").small())
                    .on_hover_text(format!("Run it in full in the {} tab", r.tool.name()))
                    .on_disabled_hover_text("Wait for the test running in its tab to finish")
                    .clicked()
                {
                    out = Some(RunEv::Tab);
                }
            });
        })
        .body(|ui| run_detail(ui, r));
    let colour = match &r.state {
        State::Failed(_) => widgets::bad(),
        _ => ui.visuals().text_color(),
    };
    ui.add(
        egui::Label::new(RichText::new(r.summary()).monospace().small().color(colour))
            .wrap()
            .selectable(true),
    );
    ui.add_space(2.0);
    out
}

/// Everything a run found, one row each.
pub(super) fn run_detail(ui: &mut Ui, r: &Run) {
    let weak = ui.visuals().weak_text_color();
    let cell = |ui: &mut Ui, s: String| {
        ui.label(RichText::new(s).monospace().small());
    };
    egui::Grid::new(("map_run_rows", r.ip, r.tool))
        .num_columns(3)
        .spacing([10.0, 1.0])
        .show(ui, |ui| match &r.out {
            Out::Ping(replies) => {
                for p in replies {
                    cell(ui, format!("#{}", p.seq));
                    match p.rtt {
                        Some(ms) => {
                            cell(ui, widgets::fmt_ms(ms));
                            cell(ui, p.ttl.map(|t| format!("ttl {t}")).unwrap_or_default());
                        }
                        None => {
                            ui.label(RichText::new(&p.note).small().color(widgets::bad()));
                            ui.label("");
                        }
                    }
                    ui.end_row();
                }
            }
            Out::Ports { found, .. } => {
                for (port, state, ms) in found {
                    cell(
                        ui,
                        format!("{port} {}", crate::net::ports::service_name(*port)),
                    );
                    let (label, c) = match state {
                        crate::net::TcpState::Open => ("open".to_string(), widgets::ok()),
                        crate::net::TcpState::Closed => ("closed".into(), weak),
                        crate::net::TcpState::Filtered => ("filtered".into(), widgets::warn()),
                        crate::net::TcpState::Error(e) => (format!("error: {e}"), widgets::bad()),
                    };
                    ui.label(RichText::new(label).monospace().small().color(c));
                    cell(ui, widgets::fmt_ms(*ms));
                    ui.end_row();
                }
            }
            Out::Trace { hops, .. } => {
                for h in hops {
                    cell(ui, h.ttl.to_string());
                    let at = match (h.addr, &h.name) {
                        (Some(a), Some(n)) => format!("{a} {n}"),
                        (Some(a), None) => a.to_string(),
                        (None, _) => "*".into(),
                    };
                    ui.add(
                        egui::Label::new(RichText::new(at).monospace().small()).selectable(true),
                    );
                    cell(ui, h.best.map(widgets::fmt_ms).unwrap_or_default());
                    ui.end_row();
                }
            }
        });
}

/// What a card drawn by hand says about it (none when discovered).
pub(super) fn drawn_note(n: &crate::map::Node) -> Option<&'static str> {
    if n.drawn {
        Some("Drawn by hand, not discovered")
    } else if n.drawn_up {
        Some("Its link up was drawn by hand, not discovered")
    } else {
        None
    }
}

/// A card's details as text, for Copy.
pub(super) fn card_text(n: &crate::map::Node) -> String {
    let mut s = format!("{} ({})\n", n.label, n.kind);
    if !n.sub.is_empty() {
        s += &format!("{}\n", n.sub);
    }
    for d in drawn_note(n)
        .map(str::to_string)
        .into_iter()
        .chain(n.flags.iter().map(|f| format!("Flag: {f}")))
        .chain(n.details.iter().cloned())
    {
        s += &format!("{d}\n");
    }
    // Its note, redacted, each line indented under it.
    if !n.note.is_empty() {
        s += "Note:\n";
        for l in n.note.lines() {
            s += &format!("  {l}\n");
        }
    }
    s
}

/// How long a panel card takes to glide to a new place in the stack.
pub(super) const SLIDE_SECS: f64 = 0.22;

/// Where to draw the panel card `id`, laid out `y` down the stack: easing
/// (cubic ease-out) from where it was shown toward `y` whenever `y` moves.
/// A card seen for the first time is just drawn where it is.
pub(super) fn slide(ctx: &egui::Context, id: &str, y: f32) -> f32 {
    let key = egui::Id::new(("panel_slide", id));
    let now = ctx.input(|i| i.time);
    let at = |(from, to, t0): (f32, f32, f64), t: f64| {
        let k = ((t - t0) / SLIDE_SECS).clamp(0.0, 1.0) as f32;
        from + (to - from) * (1.0 - (1.0 - k).powi(3))
    };
    let mut s = ctx
        .data(|d| d.get_temp::<(f32, f32, f64)>(key))
        .unwrap_or((y, y, now));
    if (s.1 - y).abs() > 0.5 {
        s = (at(s, now), y, now);
    }
    ctx.data_mut(|d| d.insert_temp(key, s));
    if now - s.2 < SLIDE_SECS {
        ctx.request_repaint();
    }
    at(s, now)
}
