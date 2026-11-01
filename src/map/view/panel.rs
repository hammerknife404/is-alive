//! The card panel beside the map: findings band, a card's details, and
//! each card's runs and results.

use super::*;

impl MapView {
    pub(super) fn findings_band(&mut self, ui: &mut Ui, focus: &mut Option<Vec<usize>>) {
        let f = &self.map.findings;
        if f.is_empty() {
            return;
        }
        let id = ui.make_persistent_id("map_findings");
        let state =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        let mut toggle = false;
        state
            .show_header(ui, |ui| {
                // The whole header opens it, not just the arrow.
                let r = ui
                    .add(egui::Label::new(RichText::new("Findings").strong()).sense(Sense::click()))
                    .on_hover_text("Show or hide the findings, worst first");
                toggle |= r.clicked();
                for (sev, colour) in [
                    (Sev::High, widgets::bad()),
                    (Sev::Med, widgets::warn()),
                    (Sev::Low, ui.visuals().weak_text_color()),
                ] {
                    let n = f.iter().filter(|x| x.sev == sev).count();
                    if n > 0 {
                        ui.label(RichText::new(format!("• {n} {}", sev.label())).color(colour));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("Copy")
                        .on_hover_text("Copy the findings as text, worst first")
                        .clicked()
                    {
                        let mut s = format!("Findings: {}\n", self.map.summary);
                        for x in f {
                            s += &format!("[{}] {}\n", x.sev.label(), x.text);
                        }
                        widgets::copy(ui.ctx(), s);
                        widgets::toast("Findings copied", true);
                    }
                });
            })
            .body_unindented(|ui| {
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .id_salt("findings_list")
                    .show(ui, |ui| {
                        for x in f {
                            ui.horizontal(|ui| {
                                let c = match x.sev {
                                    Sev::High => widgets::bad(),
                                    Sev::Med => widgets::warn(),
                                    Sev::Low => ui.visuals().weak_text_color(),
                                };
                                ui.label(
                                    RichText::new(format!("{:<4}", x.sev.label()))
                                        .monospace()
                                        .color(c),
                                );
                                let r =
                                    ui.add(egui::Label::new(&x.text).wrap().sense(Sense::click()));
                                if !x.nodes.is_empty() {
                                    let r = r.on_hover_text("Show on the map");
                                    if r.clicked() {
                                        *focus = Some(x.nodes.clone());
                                    }
                                    if r.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                }
                            });
                        }
                    });
            });
        if toggle {
            let mut s = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                false,
            );
            s.toggle(ui);
            s.store(ui.ctx());
        }
        ui.separator();
    }

    /// The detail panel: every selected card, stacked, the one clicked last
    /// on top. Each has /mapnetwork's sections: action items, details and
    /// connections, then what can be run against it.
    pub(super) fn detail(
        &self,
        ui: &mut Ui,
        mut sel: Vec<usize>,
        busy: bool,
        pick: &mut Option<usize>,
        ev: &mut Option<PanelEv>,
    ) -> Option<Action> {
        let mut act = None;
        if sel.is_empty() {
            sel.extend(self.shown.as_deref().and_then(|s| self.index(s)));
        }
        egui::ScrollArea::vertical()
            .id_salt("map_detail_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add_space(4.0);
                if sel.len() > 1 {
                    ui.horizontal(|ui| {
                        ui.strong(format!("{} selected", sel.len()))
                            .on_hover_text("Ctrl+click a card to add or remove it; Esc clears");
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
                    });
                    ui.add_space(4.0);
                }
                // Where the stack starts, so a card's place in it doesn't
                // change as the panel scrolls.
                let origin = ui.min_rect().top();
                for (k, &i) in sel.iter().enumerate() {
                    if k > 0 {
                        ui.add_space(6.0);
                    }
                    let layer = ui.layer_id();
                    let first = ui.ctx().graphics(|g| g.get(layer).map(|l| l.next_idx()));
                    let (a, frame) = self.card_panel(ui, k, i, busy, sel.len() > 1, pick, ev);
                    act = act.take().or(a);
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

    /// One card's section in the detail panel, `at` from the top.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn card_panel(
        &self,
        ui: &mut Ui,
        at: usize,
        i: usize,
        busy: bool,
        stacked: bool,
        pick: &mut Option<usize>,
        ev: &mut Option<PanelEv>,
    ) -> (Option<Action>, egui::Response) {
        let mut act = None;
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
                // Header: a grip to drag it up or down, the kind, then pin and ×.
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if stacked
                            && ui
                                .small_button("×")
                                .on_hover_text("Take it out of the selection")
                                .clicked()
                        {
                            *ev = Some(PanelEv::Drop(i));
                        }
                        let pin = RichText::new("📌")
                            .color(crate::theme::emoji_colour('📌', ui.visuals().text_color()));
                        if ui
                            .add(egui::Button::new(pin).small().selected(kept))
                            .on_hover_text(if kept {
                                "Pinned: stays selected and keeps its place here. Click to unpin"
                            } else {
                                "Pin: keep it selected and in this place in the panel"
                            })
                            .clicked()
                        {
                            *ev = Some(PanelEv::Keep(i));
                        }
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
                for d in &n.details {
                    ui.horizontal_top(|ui| {
                        ui.weak("·");
                        ui.add(egui::Label::new(d.as_str()).wrap().selectable(true));
                    });
                }
                if facts.is_empty() && n.details.is_empty() {
                    ui.weak("Nothing more is known about it.");
                }

                // Connections: up to its parent, down to what hangs off it.
                let kids: Vec<usize> = (0..self.map.nodes.len())
                    .filter(|&k| self.map.nodes[k].parent() == Some(i))
                    .collect();
                if n.up.is_some() || !kids.is_empty() {
                    section(ui, "Connections");
                }
                let row = |ui: &mut Ui, k: usize, via: String, tip: &str| -> bool {
                    let m = &self.map.nodes[k];
                    let c = zone_colour(m.zone);
                    let mut go = false;
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, c);
                        go = ui.link(&m.label).on_hover_text(tip).clicked();
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
                            "Assumed: it answered here, but its switch port isn't known"
                        }
                        _ => "Show it",
                    };
                    if row(ui, *p, how, tip) {
                        *pick = Some(*p);
                    }
                }
                for &k in kids.iter().take(14) {
                    let via = match &self.map.nodes[k].up {
                        Some((_, Link::Wireless, _)) => "wireless".to_string(),
                        Some((_, Link::Assumed, _)) => "assumed".to_string(),
                        Some((_, _, Some(port))) => port.clone(),
                        _ => "below it".to_string(),
                    };
                    if row(ui, k, via, "Show it") {
                        *pick = Some(k);
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
                    if let Some(ip) = n.ip {
                        let web = [
                            (443, "https", ""),
                            (8443, "https", ":8443"),
                            (80, "http", ""),
                        ]
                        .into_iter()
                        .find(|(p, ..)| n.open.contains(p));
                        if let Some((_, scheme, port)) = web
                            && ui
                                .small_button("Web")
                                .on_hover_text(format!(
                                    "Open {scheme}://{ip}{port} in your browser"
                                ))
                                .clicked()
                        {
                            ui.ctx()
                                .open_url(egui::OpenUrl::new_tab(format!("{scheme}://{ip}{port}")));
                        }
                    }
                    // A +N more card opens its uplink up; an opened uplink folds back.
                    if n.id.starts_with("more:")
                        && let Some(p) = n.parent()
                        && ui
                            .small_button(format!("Show all {}", n.label.trim_start_matches('+')))
                            .on_hover_text("A card for each of them")
                            .clicked()
                    {
                        *ev = Some(PanelEv::Open(p, true));
                    }
                    if self.open.contains(&n.id)
                        && ui
                            .small_button("Fold clients")
                            .on_hover_text("Back to 10 cards and a +N more")
                            .clicked()
                    {
                        *ev = Some(PanelEv::Open(i, false));
                    }
                    if ui
                        .small_button("Copy")
                        .on_hover_text("Copy this card's details as text")
                        .clicked()
                    {
                        widgets::copy(ui.ctx(), card_text(n));
                        widgets::toast("Card details copied", true);
                    }
                });

                let runs = self.runs.runs(&n.id);
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
                    .show(ui, |ui| {
                        for r in runs {
                            if let Some(e) = run_section(ui, &n.id, r, busy) {
                                match e {
                                    RunEv::Again => *ev = Some(PanelEv::Run(i, r.tool)),
                                    RunEv::Stop => *ev = Some(PanelEv::Stop(i, r.tool)),
                                    RunEv::Clear => *ev = Some(PanelEv::Clear(i, r.tool)),
                                    RunEv::Tab => {
                                        let ip = r.ip.to_string();
                                        act = Some(match r.tool {
                                            Tool::Ping => Action::Ping(ip),
                                            Tool::Ports => Action::Ports(ip),
                                            Tool::Trace => Action::Trace(ip),
                                        });
                                    }
                                }
                            }
                        }
                    });
                }
            });
        (act, frame.response)
    }
}

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

/// A card's details as text, for Copy.
pub(super) fn card_text(n: &crate::map::Node) -> String {
    let mut s = format!("{} ({})\n", n.label, n.kind);
    if !n.sub.is_empty() {
        s += &format!("{}\n", n.sub);
    }
    for d in n
        .flags
        .iter()
        .map(|f| format!("Flag: {f}"))
        .chain(n.details.iter().cloned())
    {
        s += &format!("{d}\n");
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
