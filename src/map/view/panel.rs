//! The Map's inspector: a card's details, and each card's runs and
//! results, in the pieces every inspector is built from (`crate::inspector`).

use super::*;
use crate::inspector::{self as insp, Act};

impl MapView {
    /// The inspector's body: every selected card, stacked, the one clicked
    /// last on top. Each has /mapnetwork's sections: action items, details
    /// and connections, then what can be run against it. `menus` gets each
    /// card's ⋯ button, for `card_menus` to open the right-click menu from.
    /// `edit` is the note being written, shown in its card.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn detail(
        &self,
        ui: &mut Ui,
        sel: Vec<usize>,
        busy: bool,
        pick: &mut Option<(usize, bool)>,
        ev: &mut Option<PanelEv>,
        menus: &mut Vec<(usize, egui::Response)>,
        edit: &mut Option<notes::Edit>,
    ) -> Option<Action> {
        let mut act = None;
        // A card clicked on its own already lights its path to the
        // internet: Trace to WAN is for cards picked together (Ctrl+click,
        // the right-click menu, a site, a finding) and pinned cards left on
        // their own, which light only what joins them.
        if self.joins_only() {
            self.panel_header(ui, &sel, ev);
        }
        // Where the stack starts, so a card's place in it doesn't change as
        // the panel scrolls.
        let origin = ui.min_rect().top();
        for (k, &i) in sel.iter().enumerate() {
            if k > 0 {
                ui.add_space(8.0);
            }
            let layer = ui.layer_id();
            let first = ui.ctx().graphics(|g| g.get(layer).map(|l| l.next_idx()));
            let (a, frame, menu) = self.card_panel(ui, k, i, busy, sel.len() > 1, pick, ev, edit);
            act = act.take().or(a);
            menus.extend(menu.map(|m| (i, m)));
            // A card whose place in the stack changed (moved, or pushed down
            // by a new one) glides there.
            let at = frame.rect.top() - origin;
            let shown = slide(ui.ctx(), &self.map.nodes[i].id, at);
            if (shown - at).abs() > 0.1
                && let Some(first) = first
            {
                // Shapes move; their clipping stays inside the panel, so a
                // gliding card never draws over the header or outside it.
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
                    frame.rect.top() - 4.0,
                    Stroke::new(2.0, crate::theme::accent()),
                );
            }
        }
        act
    }

    /// Each card's ⋯ in the inspector: the card's right-click menu, the
    /// very same one (on one of several selected cards it acts on them all).
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
        ui.horizontal_wrapped(|ui| {
            ui.set_min_height(insp::ICON.y);
            if sel.len() > 1 {
                ui.strong(format!("{} selected", sel.len()))
                    .on_hover_text("Ctrl+click a card to add or remove it; Esc clears");
            }
            let mut trace = self.trace_wan;
            if ui
                .checkbox(&mut trace, "Trace to WAN")
                .on_hover_text(
                    "Light each selected card's whole path to the internet. Off, only the \
                     links that join the selected cards light (one card: its uplink and its \
                     children). A card clicked on its own always shows its path",
                )
                .changed()
            {
                *ev = Some(PanelEv::TraceWan(trace));
            }
            if sel.len() > 1
                && ui
                    .add(egui::Button::new("Copy all").min_size(vec2(0.0, insp::ICON.y)))
                    .on_hover_text("Copy every selected card's details as text")
                    .clicked()
            {
                let s: Vec<String> = sel.iter().map(|&k| card_text(&self.map.nodes[k])).collect();
                widgets::copy(ui.ctx(), s.join("\n"));
                widgets::toast(format!("{} cards copied", sel.len()), true);
            }
        });
        ui.add_space(6.0);
    }

    /// One card in the inspector, `at` from the top of the stack: framed
    /// when `stacked` with others; with its ⋯ button.
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
        let kept = self.is_kept(i);
        let frame = if stacked {
            egui::Frame::group(ui.style())
                .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.5)))
                .inner_margin(8)
        } else {
            egui::Frame::new()
        };
        let frame = frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Header: a grip to drag it up or down (stacked), its role and
            // kind; pin, ⋯ and × on the right; the name.
            insp::header_with(
                ui,
                &n.kind,
                Some(accent),
                &n.label,
                |ui| {
                    if stacked {
                        grip(ui, i, at);
                    }
                },
                |ui| {
                    let close = if stacked {
                        "Take it out of the selection"
                    } else {
                        "Close (Esc)"
                    };
                    if insp::icon_button(ui, "×", close).clicked() {
                        *ev = Some(PanelEv::Drop(i));
                    }
                    let b = insp::icon_button(ui, "⋯", "Menu (same as right-clicking the card)");
                    // Its name for screen readers, as "⋯" says nothing.
                    b.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Card menu")
                    });
                    menu = Some(b);
                    let pin = RichText::new("📌")
                        .color(crate::theme::emoji_colour('📌', ui.visuals().text_color()));
                    if ui
                        .add(
                            egui::Button::new(pin)
                                .frame(kept)
                                .selected(kept)
                                .min_size(insp::ICON),
                        )
                        .on_hover_text(if kept {
                            "Pinned: stays selected and keeps its place here. Click to unpin"
                        } else {
                            "Pin: keep it selected and in this place in the panel"
                        })
                        .clicked()
                    {
                        *ev = Some(PanelEv::Keep(i));
                    }
                    if self.note_of(i).is_some() {
                        let (r, resp) = ui.allocate_exact_size(vec2(9.0, 11.0), Sense::hover());
                        paint::note_glyph(ui.painter(), ui.visuals(), r, 1.0);
                        resp.on_hover_text("Has a note (below)");
                    }
                },
            );
            // Its address (or what stands for one), then its zone and tags.
            if !n.sub.is_empty() {
                insp::address(ui, &n.sub);
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
            ui.add_space(6.0);
            self.card_actions(ui, i, ev);

            // Action items: its own flags and the findings that name it.
            let findings: Vec<&crate::map::Finding> = self
                .map
                .findings
                .iter()
                .filter(|f| f.nodes.contains(&i))
                .collect();
            if !n.flags.is_empty() || !findings.is_empty() {
                insp::section(ui, "Action items");
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
            insp::section(ui, "Details");
            let mut facts: Vec<(&str, String)> = Vec::new();
            if let Some(ip) = n.ip.map(|ip| ip.to_string())
                && ip != n.sub
            {
                facts.push(("IP", ip));
            }
            // A detail line already says more about the MAC (vendor, private).
            if let Some(mac) = n.mac
                && !n.details.iter().any(|d| d.starts_with("MAC "))
            {
                facts.push(("MAC", crate::net::mac::fmt(&mac)));
            }
            for (k, v) in &facts {
                insp::row(ui, k, v, true);
            }
            if !n.seen_by.is_empty() {
                insp::row(ui, "Seen by", &n.seen_by.join(", "), false);
            }
            for d in drawn_note(n)
                .into_iter()
                .chain(n.details.iter().map(String::as_str))
            {
                ui.horizontal_top(|ui| {
                    ui.weak("·");
                    ui.add(egui::Label::new(d).wrap().selectable(true));
                });
            }
            if facts.is_empty()
                && n.seen_by.is_empty()
                && n.details.is_empty()
                && drawn_note(n).is_none()
            {
                insp::note(ui, "Nothing more is known about it.");
            }
            if !n.open.is_empty() {
                insp::section(ui, "Open ports");
                let ports: Vec<String> = n
                    .open
                    .iter()
                    .map(|&p| match crate::net::ports::service_name(p) {
                        "" => p.to_string(),
                        s => format!("{p} {s}"),
                    })
                    .collect();
                insp::chips(ui, &ports);
            }

            // Its quick tests, as they run and once they're done.
            let runs = self.runs.runs(&n.id);
            let reveal = revealing(ui.ctx(), &n.id);
            if !runs.is_empty() {
                insp::section(ui, "Results");
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
                    match e {
                        Some(RunEv::Again) => *ev = Some(PanelEv::Run(i, r.tool)),
                        Some(RunEv::Stop) => *ev = Some(PanelEv::Stop(i, r.tool)),
                        Some(RunEv::Clear) => *ev = Some(PanelEv::Clear(i, r.tool)),
                        Some(RunEv::Tab) => act = Some(tab(r.tool, r.ip.to_string())),
                        None => {}
                    }
                }
            }

            // The technician's note, its editor, or Add note.
            let writing = edit.as_ref().is_some_and(|e| e.ids.contains(&n.id));
            let mut note_ev = None;
            if self.note_of(i).is_none() && !writing {
                insp::section(ui, "Note");
                if ui
                    .add(egui::Button::new("Add note").min_size(vec2(0.0, insp::ICON.y)))
                    .on_hover_text("Write a note on this card (N)")
                    .clicked()
                {
                    note_ev = Some(notes::NoteEv::Open(i));
                }
            } else {
                self.note_section(ui, i, edit, &mut note_ev);
            }
            if let Some(e) = note_ev {
                *ev = Some(PanelEv::Note(e));
            }

            // Connections: up to its parent, down to what hangs off it.
            let kids: Vec<usize> = (0..self.map.nodes.len())
                .filter(|&k| self.map.nodes[k].parent() == Some(i))
                .collect();
            if n.up.is_some() || !kids.is_empty() {
                insp::section(ui, "Connections");
            }
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
                if let Some(go) = self.link_row(ui, *p, &how, tip) {
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
                if let Some(go) = self.link_row(ui, k, &via, SHOW_IT) {
                    *pick = Some(go);
                }
            }
            if kids.len() > 14 {
                ui.weak(format!("…{} more below it", kids.len() - 14));
            }
            ui.add_space(4.0);
        });
        (act, frame.response, menu)
    }

    /// What can be done to card `i` from here: Ping, Ports and Trace (the
    /// result shows under Results), and Web, SSH and RDP when they're open.
    /// Wake, Copy and the rest are in ⋯.
    fn card_actions(&self, ui: &mut Ui, i: usize, ev: &mut Option<PanelEv>) {
        let n = &self.map.nodes[i];
        let Some(ip) = n.ip else {
            return;
        };
        enum Do {
            Run(Tool),
            Web(String),
            Connect(crate::connect::Plan),
        }
        let mut acts: Vec<(String, String, bool, Do)> = Vec::new();
        for (tool, tip) in [
            (
                Tool::Ping,
                "Ping it 4 times; the result shows under Results",
            ),
            (
                Tool::Ports,
                "Check its common ports; the result shows under Results",
            ),
            (
                Tool::Trace,
                "Trace the route to it; the result shows under Results",
            ),
        ] {
            let running = self.runs.active(&n.id, tool);
            let tip = if running { "Running: see Results" } else { tip };
            acts.push((tool.name().to_string(), tip.into(), !running, Do::Run(tool)));
        }
        // Web only when a web port is known to be open.
        if interact::web_open(&n.open) {
            let url = interact::web_url(ip.into(), &n.open);
            acts.push((
                "Web".into(),
                format!("Open {url} in your browser"),
                true,
                Do::Web(url),
            ));
        }
        // SSH and RDP only when the port was found open and this computer
        // has a client for it.
        for kind in [crate::connect::Kind::Ssh, crate::connect::Kind::Rdp] {
            if !n.open.contains(&kind.port()) {
                continue;
            }
            if let Some(plan) = crate::connect::plan(kind, ip) {
                acts.push((
                    kind.label().to_string(),
                    format!(
                        "Opens {}. It asks for the log-in; nothing is stored.",
                        plan.what
                    ),
                    true,
                    Do::Connect(plan),
                ));
            }
        }
        let shown: Vec<Act> = acts
            .iter()
            .map(|(label, tip, enabled, _)| Act {
                label,
                tip,
                enabled: *enabled,
            })
            .collect();
        if let Some(k) = insp::actions(ui, &shown) {
            match acts.swap_remove(k).3 {
                Do::Run(tool) => *ev = Some(PanelEv::Run(i, tool)),
                Do::Web(url) => widgets::open_url(url),
                Do::Connect(plan) => crate::connect::launch(plan),
            }
        }
    }

    /// A card in Connections, a row the height of a button: a click shows
    /// it, Ctrl+click adds it to the selection.
    fn link_row(&self, ui: &mut Ui, k: usize, via: &str, tip: &str) -> Option<(usize, bool)> {
        let m = &self.map.nodes[k];
        let (r, resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), insp::ICON.y), Sense::click());
        let resp = resp.on_hover_text(tip);
        if resp.hovered() {
            ui.painter()
                .rect_filled(r, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let p = ui.painter();
        p.circle_filled(pos2(r.min.x + 9.0, r.center().y), 4.0, zone_colour(m.zone));
        let font = egui::TextStyle::Body.resolve(ui.style());
        let small = egui::TextStyle::Small.resolve(ui.style());
        let via_w = p
            .layout_no_wrap(via.to_string(), small.clone(), Color32::WHITE)
            .size()
            .x;
        let name = p.layout(
            m.label.clone(),
            font,
            if resp.hovered() {
                crate::theme::accent()
            } else {
                ui.visuals().text_color()
            },
            (r.width() - 32.0 - via_w).max(40.0),
        );
        p.galley(
            pos2(r.min.x + 20.0, r.center().y - name.size().y / 2.0),
            name,
            Color32::WHITE,
        );
        p.text(
            pos2(r.max.x - 4.0, r.center().y),
            egui::Align2::RIGHT_CENTER,
            via,
            small,
            ui.visuals().weak_text_color(),
        );
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Link, true, m.label.as_str())
        });
        resp.clicked().then(|| {
            let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
            (k, add)
        })
    }
}

/// The dots to drag a stacked card up or down by.
fn grip(ui: &mut Ui, i: usize, at: usize) {
    ui.dnd_drag_source(ui.id().with(("map_grip", i)), at, |ui| {
        // Hover only: the drag belongs to the source around it.
        let (r, resp) = ui.allocate_exact_size(vec2(12.0, insp::ICON.y), Sense::hover());
        let c = ui.visuals().weak_text_color();
        for row in 0..3 {
            for col in 0..2 {
                ui.painter().circle_filled(
                    pos2(
                        r.min.x + 3.5 + col as f32 * 5.0,
                        r.center().y - 5.0 + row as f32 * 5.0,
                    ),
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
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.label(RichText::new(r.tool.name()).strong());
            if r.active() {
                widgets::spinner(ui);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if insp::icon_button(ui, "×", "Clear this result").clicked() {
                    out = Some(RunEv::Clear);
                }
                if r.active() {
                    if insp::icon_button(ui, "■", "Stop").clicked() {
                        out = Some(RunEv::Stop);
                    }
                } else if insp::icon_button(ui, "⟲", "Run it again").clicked() {
                    out = Some(RunEv::Again);
                }
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new("Open in tab")
                            .frame(false)
                            .min_size(vec2(0.0, insp::ICON.y)),
                    )
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
    ui.add_space(4.0);
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
