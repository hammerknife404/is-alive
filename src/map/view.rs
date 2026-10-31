//! The Map tab: pan (drag), zoom (wheel or pinch), drag a card to move it,
//! click to trace its path to the internet, search, filter by role, and a
//! findings band. It rebuilds as the sweep finds hosts, so a scan can be
//! watched filling in the map.

use super::layout::{self, CARD, Layout};
use super::{Input, Link, Map, Node, Sev, Zone};
use crate::net::hostinfo::HostInfo;
use crate::tools::{export::ExportTool, sweep::SweepTool};
use crate::widgets;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Painter, Pos2, Rect, RichText, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, pos2, text::LayoutJob, vec2,
};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

/// What the tab asks the app to do.
pub enum Action {
    /// Sweep this machine's subnet (the map fills in as it runs).
    Sweep,
    /// Open Config Export to load a backup.
    Export,
    Ping(String),
    Ports(String),
    Trace(String),
    /// Open the Site Bundle window.
    Bundle,
    /// The accuracy note was closed: don't show it again.
    HideNote,
}

/// What the detail panel asked for.
enum PanelEv {
    /// × on a card: take it out of the selection.
    Drop(usize),
    /// The pin: keep it selected, or not.
    Keep(usize),
    /// A card dragged from one place in the stack to another.
    Move(usize, usize),
}

pub struct Sources<'a> {
    pub sweep: &'a SweepTool,
    pub export: &'a ExportTool,
    pub info: &'a HostInfo,
    /// Ping or Port Check is running: their buttons wait.
    pub busy: bool,
    /// Show the accuracy note (until the user closes it).
    pub note: bool,
}

const MIN_ZOOM: f32 = 0.15;
const MAX_ZOOM: f32 = 2.5;

#[derive(Default)]
pub struct MapView {
    map: Map,
    key: u64,
    clients: bool,
    search: String,
    /// The card the detail panel shows: the one clicked last.
    selected: Option<String>,
    /// More cards picked with Ctrl+click, lit with it.
    also: Vec<String>,
    /// Cards pinned in the selection: a plain click elsewhere keeps them.
    kept: Vec<String>,
    /// The detail panel's order, top first; dragged by hand.
    order: Vec<String>,
    /// The card a right-click opened the menu for.
    menu_for: Option<usize>,
    /// What the detail panel shows while it slides shut.
    shown: Option<String>,
    hidden: HashSet<Zone>,
    lay: Layout,
    /// Where each card is drawn now, easing toward the layout.
    pos: HashMap<String, Pos2>,
    born: HashMap<String, f64>,
    /// Cards the user dragged somewhere.
    pinned: HashMap<String, Pos2>,
    pan: Vec2,
    zoom: f32,
    /// (pan, zoom) the view is easing to.
    aim: Option<(Vec2, f32)>,
    /// Keep the whole map in view as it grows, until the user pans or zooms.
    follow: bool,
    drag: Option<String>,
    /// When the map was last rebuilt, to pace rebuilds during a sweep.
    built_at: f64,
    /// The left sidebar is open (closed: a › button on the canvas).
    side: bool,
}

impl MapView {
    pub fn new() -> Self {
        Self {
            follow: true,
            side: true,
            ..Default::default()
        }
    }

    fn input<'a>(&self, src: &'a Sources) -> Input<'a> {
        let sw = src.sweep;
        Input {
            sweep: sw.seen(),
            range: sw.finished().map(|f| f.0),
            ports_checked: sw.checked_ports(),
            gateway: src.info.gateway,
            this: src.info.lan.map(|l| l.0),
            wan: src.info.wan,
            unifi: src.export.unifi().map(|u| u.1.json.as_str()),
            sonicwall: src.export.sonicwall().map(|s| s.1.json.as_str()),
            mapnetwork: src.export.mapnetwork().map(|m| m.1),
            clients: self.clients,
        }
    }

    /// Rebuild when anything feeding the map changed.
    fn refresh(&mut self, ctx: &egui::Context, src: &Sources) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        src.sweep.generation().hash(&mut h);
        self.clients.hash(&mut h);
        if let Some((p, r)) = src.export.unifi() {
            (p, r.json.len()).hash(&mut h);
        }
        if let Some((p, r)) = src.export.sonicwall() {
            (p, r.json.len()).hash(&mut h);
        }
        if let Some((p, d)) = src.export.mapnetwork() {
            (p, d.json.len()).hash(&mut h);
        }
        (src.info.gateway, src.info.lan, src.info.wan).hash(&mut h);
        let key = h.finish();
        if key == self.key {
            return;
        }
        // A sweep adds hosts every frame: rebuild a few times a second.
        let now = ctx.input(|i| i.time);
        if src.sweep.is_running() && now - self.built_at < 0.25 {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            return;
        }
        self.built_at = now;
        self.key = key;
        let input = self.input(src);
        self.map = if input.is_empty() {
            Map::default()
        } else {
            super::build(&input)
        };
        self.relayout(ctx);
    }

    fn relayout(&mut self, ctx: &egui::Context) {
        self.lay = layout::layout(&self.map, &|_| true);
        let now = ctx.input(|i| i.time);
        // New cards grow out of their parent.
        for (i, n) in self.map.nodes.iter().enumerate() {
            if self.pos.contains_key(&n.id) {
                continue;
            }
            let start = n
                .parent()
                .and_then(|p| self.pos.get(&self.map.nodes[p].id).copied())
                .or(self.lay.pos[i])
                .unwrap_or(Pos2::ZERO);
            self.pos.insert(n.id.clone(), start);
            self.born.insert(n.id.clone(), now);
        }
        let ids: HashSet<&str> = self.map.nodes.iter().map(|n| n.id.as_str()).collect();
        self.pos.retain(|k, _| ids.contains(k.as_str()));
        self.born.retain(|k, _| ids.contains(k.as_str()));
        self.also.retain(|s| ids.contains(s.as_str()));
        self.kept.retain(|s| ids.contains(s.as_str()));
        if self
            .selected
            .as_ref()
            .is_some_and(|s| !ids.contains(s.as_str()))
        {
            self.selected = self.also.pop();
        }
    }

    /// Every selected card: pinned ones, Ctrl+clicked ones, the one clicked last.
    fn selection(&self) -> Vec<usize> {
        let mut v: Vec<usize> = Vec::new();
        for i in self
            .kept
            .iter()
            .chain(&self.also)
            .chain(&self.selected)
            .filter_map(|s| self.index(s))
        {
            if !v.contains(&i) {
                v.push(i);
            }
        }
        v
    }

    /// Pin a card in the selection, or let it go.
    fn keep(&mut self, i: usize) {
        let id = self.map.nodes[i].id.clone();
        match self.kept.iter().position(|s| *s == id) {
            Some(k) => {
                self.kept.remove(k);
            }
            None => self.kept.push(id),
        }
    }

    fn is_kept(&self, i: usize) -> bool {
        self.kept.contains(&self.map.nodes[i].id)
    }

    /// The panel's cards in their order: new ones on top, then as dragged.
    fn ordered(&mut self) -> Vec<usize> {
        let sel = self.selection();
        let ids: Vec<String> = sel.iter().map(|&i| self.map.nodes[i].id.clone()).collect();
        self.order.retain(|s| ids.contains(s));
        // Selection runs oldest to newest, so the newest lands on top.
        for id in &ids {
            if !self.order.contains(id) {
                self.order.insert(0, id.clone());
            }
        }
        self.order.iter().filter_map(|s| self.index(s)).collect()
    }

    /// One card selected, the rest let go.
    fn select(&mut self, i: Option<usize>) {
        self.selected = i.map(|i| self.map.nodes[i].id.clone());
        self.also.clear();
    }

    /// Ctrl+click: add the card to the selection, or take it out (pinned too).
    fn toggle(&mut self, i: usize) {
        let id = self.map.nodes[i].id.clone();
        if let Some(k) = self.kept.iter().position(|s| *s == id) {
            self.kept.remove(k);
            if self.selected.as_deref() == Some(id.as_str()) {
                self.selected = self.also.pop();
            }
            self.also.retain(|s| *s != id);
        } else if self.selected.as_deref() == Some(id.as_str()) {
            self.selected = self.also.pop();
        } else if let Some(k) = self.also.iter().position(|s| *s == id) {
            self.also.remove(k);
        } else {
            self.also.extend(self.selected.take());
            self.selected = Some(id);
        }
    }

    fn target(&self, i: usize) -> Option<Pos2> {
        let n = &self.map.nodes[i];
        self.pinned.get(&n.id).copied().or(self.lay.pos[i])
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.map.nodes.iter().position(|n| n.id == id)
    }

    fn matches(&self, n: &Node) -> bool {
        let q = self.search.trim().to_lowercase();
        q.is_empty()
            || [&n.label, &n.sub, &n.kind]
                .iter()
                .any(|s| s.to_lowercase().contains(&q))
            || n.details.iter().any(|d| d.to_lowercase().contains(&q))
    }

    /// Bounds of the laid-out map, with pinned cards.
    fn bounds(&self) -> Rect {
        let mut r = Rect::NOTHING;
        for i in 0..self.map.nodes.len() {
            if let Some(p) = self.target(i) {
                r = r.union(Rect::from_min_size(p, CARD));
            }
        }
        r
    }

    fn fit(&mut self, view: Rect) {
        let b = self.bounds();
        if !b.is_positive() {
            return;
        }
        let zoom = ((view.width() - 60.0) / b.width())
            .min((view.height() - 60.0) / b.height())
            .clamp(MIN_ZOOM, 1.0);
        let pan = view.size() / 2.0 - b.center().to_vec2() * zoom;
        self.aim = Some((pan, zoom));
    }

    fn focus(&mut self, view: Rect, i: usize) {
        if let Some(p) = self.target(i) {
            let zoom = self.zoom.max(0.85);
            let c = p + CARD / 2.0;
            self.aim = Some((view.size() / 2.0 - c.to_vec2() * zoom, zoom));
            self.follow = false;
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, src: Sources) -> Option<Action> {
        let mut act = None;
        let ctx = ui.ctx().clone();
        self.refresh(&ctx, &src);
        let empty = self.map.nodes.is_empty();

        // Toolbar.
        let mut search_enter = false;
        let mut fit = false;
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .char_limit(widgets::FIELD_MAX)
                    .desired_width(220.0)
                    .hint_text("Search name, IP, MAC, vendor"),
            );
            search_enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if !self.search.is_empty() {
                let n = self.map.nodes.iter().filter(|n| self.matches(n)).count();
                ui.weak(format!("{n} match{}", if n == 1 { "" } else { "es" }));
                if ui.small_button("×").on_hover_text("Clear search").clicked() {
                    self.search.clear();
                }
            }
            ui.separator();
            ui.checkbox(&mut self.clients, "Clients")
                .on_hover_text("Show end devices (up to 10 per switch or AP)");
            ui.separator();
            let subnet = src
                .info
                .lan
                .map(|(ip, bits)| crate::net::targets::subnet_cidr(ip, bits));
            if !empty {
                let tip = format!(
                    "Sweep {} again and update the map as hosts answer",
                    subnet.as_deref().unwrap_or("this machine's subnet")
                );
                if ui
                    .add_enabled(
                        subnet.is_some() && !src.sweep.is_running(),
                        egui::Button::new("Scan"),
                    )
                    .on_hover_text(&tip)
                    .on_disabled_hover_text(if src.sweep.is_running() {
                        "A sweep is running"
                    } else {
                        "No LAN address on this machine"
                    })
                    .clicked()
                {
                    act = Some(Action::Sweep);
                }
            }
            ui.add_enabled_ui(!empty, |ui| {
                if ui
                    .button("Fit")
                    .on_hover_text("Fit the map (or double-click the background)")
                    .clicked()
                {
                    fit = true;
                }
                if ui
                    .add_enabled(!self.pinned.is_empty(), egui::Button::new("Re-layout"))
                    .on_hover_text("Undo card moves")
                    .clicked()
                {
                    self.pinned.clear();
                    fit = true;
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!empty, |ui| {
                    if ui
                        .button("Export SVG…")
                        .on_hover_text("Save the map as SVG")
                        .clicked()
                    {
                        let pos: Vec<Option<Pos2>> =
                            (0..self.map.nodes.len()).map(|i| self.target(i)).collect();
                        widgets::save_file(
                            "is-alive-map.svg".into(),
                            "SVG",
                            vec!["svg"],
                            super::svg::render(
                                &self.map,
                                &pos,
                                &self.lay.stacked,
                                ui.visuals().dark_mode,
                            ),
                        );
                    }
                });
            });
        });
        if !self.map.summary.is_empty() {
            ui.weak(&self.map.summary);
        }
        if !empty && src.note && src.export.mapnetwork().is_none() {
            act = accuracy_note(ui);
        }
        // A sweep fills the map in as it runs: show how far along it is.
        if let Some((done, total, since, range)) = src.sweep.progress() {
            ui.horizontal(|ui| {
                widgets::spinner(ui);
                ui.label(format!("Scanning {range}"));
                if total > 0 {
                    widgets::progress(ui, done, total, since);
                } else {
                    ui.weak("starting…");
                }
            });
        }
        if empty {
            ui.separator();
            act = empty_state(ui, src.info);
            return act;
        }
        let mut focus_on: Option<Vec<usize>> = None;
        self.findings_band(ui, &mut focus_on);

        // Detail panel; it keeps the last card while it slides shut.
        if self.selected.is_some() {
            self.shown.clone_from(&self.selected);
        }
        let mut open = !self.selection().is_empty();
        let mut pick: Option<usize> = None;
        let mut ev: Option<PanelEv> = None;
        let stack = self.ordered();
        egui::Panel::right("map_detail")
            .resizable(true)
            .default_size(320.0)
            .size_range(260.0..=520.0)
            .show_collapsible(ui, &mut open, |ui| {
                act = act
                    .take()
                    .or(self.detail(ui, stack, src.busy, &mut pick, &mut ev));
            });
        if !open {
            self.select(None);
        }
        match ev {
            Some(PanelEv::Drop(i)) => self.toggle(i),
            Some(PanelEv::Keep(i)) => self.keep(i),
            Some(PanelEv::Move(from, to)) if from < self.order.len() && to < self.order.len() => {
                let id = self.order.remove(from);
                self.order.insert(to, id);
            }
            _ => {}
        }

        // Left: roles and action items, as /mapnetwork's sidebar has them.
        self.sidebar(ui, &mut focus_on);

        // Canvas.
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if self.zoom == 0.0 {
            self.zoom = 1.0;
            self.fit(rect);
            if let Some((p, z)) = self.aim.take() {
                (self.pan, self.zoom) = (p, z);
            }
        }
        if fit {
            self.follow = true;
            self.fit(rect);
        } else if self.follow && self.drag.is_none() {
            self.fit(rect);
        }
        if let Some(&first) = focus_on.as_ref().and_then(|ids| ids.first()) {
            self.select(Some(first));
            self.focus(rect, first);
        }
        if let Some(i) = pick {
            self.select(Some(i));
            self.focus(rect, i);
        }
        if search_enter && let Some(i) = self.map.nodes.iter().position(|n| self.matches(n)) {
            self.select(Some(i));
            self.focus(rect, i);
        }
        self.interact(ui, rect, &resp);
        self.animate(&ctx);
        self.paint(ui, rect);
        // With the sidebar shut, the roles go back on the canvas, beside a
        // button that opens it again.
        if !self.side {
            if ui
                .put(
                    Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(24.0, 24.0)),
                    egui::Button::new("›").corner_radius(6),
                )
                .on_hover_text("Show the roles and action items")
                .clicked()
            {
                self.side = true;
            }
            self.legend(ui, rect);
        }
        self.zoom_controls(ui, rect);
        act
    }

    fn findings_band(&mut self, ui: &mut Ui, focus: &mut Option<Vec<usize>>) {
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
    fn detail(
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
                for (k, &i) in sel.iter().enumerate() {
                    if k > 0 {
                        ui.add_space(6.0);
                    }
                    let (a, frame) = self.card_panel(ui, k, i, busy, sel.len() > 1, pick, ev);
                    act = act.take().or(a);
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
    fn card_panel(
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
        let dark = ui.visuals().dark_mode;
        let accent = zone_colour(n.zone, dark);
        let section = |ui: &mut Ui, title: &str| {
            ui.add_space(6.0);
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
                                "Pinned: stays selected when you click other cards. Click to unpin"
                            } else {
                                "Pin: keep it selected when you click other cards"
                            })
                            .clicked()
                        {
                            *ev = Some(PanelEv::Keep(i));
                        }
                    });
                });
                ui.label(RichText::new(&n.label).heading());
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
                            ui.label(RichText::new(f.sev.label()).monospace().small().color(c));
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
                        ui.add(
                            egui::Label::new(RichText::new(d).small())
                                .wrap()
                                .selectable(true),
                        );
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
                    let c = zone_colour(m.zone, dark);
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

                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if let Some(ip) = n.ip {
                        let tip = if busy {
                            "Wait for the running test to finish"
                        } else {
                            ""
                        };
                        let mut run = |ui: &mut Ui, label: &str, hover: &str, a: Action| {
                            if ui
                                .add_enabled(!busy, egui::Button::new(label))
                                .on_hover_text(hover)
                                .on_disabled_hover_text(tip)
                                .clicked()
                            {
                                act = Some(a);
                            }
                        };
                        run(
                            ui,
                            "Ping",
                            "Ping it 4 times (opens Ping)",
                            Action::Ping(ip.to_string()),
                        );
                        run(
                            ui,
                            "Port Check",
                            "Check its common ports (opens Port Check)",
                            Action::Ports(ip.to_string()),
                        );
                        run(
                            ui,
                            "Traceroute",
                            "Trace the route to it (opens Traceroute)",
                            Action::Trace(ip.to_string()),
                        );
                        let web = [
                            (443, "https", ""),
                            (8443, "https", ":8443"),
                            (80, "http", ""),
                        ]
                        .into_iter()
                        .find(|(p, ..)| n.open.contains(p));
                        if let Some((_, scheme, port)) = web
                            && ui
                                .button("Open web page")
                                .on_hover_text(format!("{scheme}://{ip}{port} in your browser"))
                                .clicked()
                        {
                            ui.ctx()
                                .open_url(egui::OpenUrl::new_tab(format!("{scheme}://{ip}{port}")));
                        }
                    }
                    if ui
                        .button("Copy")
                        .on_hover_text("Copy this card's details as text")
                        .clicked()
                    {
                        widgets::copy(ui.ctx(), card_text(n));
                        widgets::toast("Card details copied", true);
                    }
                });
            });
        (act, frame.response)
    }

    fn to_screen(&self, rect: Rect, w: Pos2) -> Pos2 {
        rect.min + self.pan + w.to_vec2() * self.zoom
    }

    fn to_world(&self, rect: Rect, s: Pos2) -> Pos2 {
        ((s - rect.min - self.pan) / self.zoom).to_pos2()
    }

    fn hit(&self, w: Pos2) -> Option<usize> {
        (0..self.map.nodes.len()).rev().find(|&i| {
            self.pos
                .get(&self.map.nodes[i].id)
                .is_some_and(|p| Rect::from_min_size(*p, CARD).contains(w))
        })
    }

    fn interact(&mut self, ui: &Ui, rect: Rect, resp: &egui::Response) {
        let pointer = resp.hover_pos();
        let over = pointer.and_then(|p| self.hit(self.to_world(rect, p)));
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
                        self.pinned.insert(id.clone(), at);
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
        if resp.clicked() {
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
        } else if over.is_some() {
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
    fn animate(&mut self, ctx: &egui::Context) {
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

    fn paint(&self, ui: &Ui, rect: Rect) {
        let p = ui.painter_at(rect);
        let vis = ui.visuals();
        let dark = vis.dark_mode;
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

        // Links.
        let mut flows = false;
        for (i, n) in self.map.nodes.iter().enumerate() {
            let Some((up, link, label)) = &n.up else {
                continue;
            };
            let (Some(a), Some(b)) = (self.pos.get(&self.map.nodes[*up].id), self.pos.get(&n.id))
            else {
                continue;
            };
            let pts: Vec<Pos2> = layout::route(*a, *b, self.lay.stacked[i])
                .into_iter()
                .map(|w| self.to_screen(rect, w))
                .collect();
            let alpha = fade(i).min(fade(*up));
            let on = !sel.is_empty() && lit.contains(&i) && lit.contains(up);
            let base = match link {
                Link::Backbone => zone_colour(Zone::Gateway, dark),
                Link::Assumed => vis.weak_text_color(),
                _ => zone_colour(n.zone, dark),
            };
            let w = match link {
                Link::Backbone => 3.0,
                _ => 1.6,
            } * z.clamp(0.6, 1.4)
                + if on { 0.8 } else { 0.0 };
            let c = base.gamma_multiply(alpha * if on { 1.0 } else { 0.6 });
            let stroke = Stroke::new(w, c);
            match link {
                Link::Wireless => p.extend(Shape::dashed_line(&pts, stroke, 7.0 * z, 5.0 * z)),
                Link::Assumed => p.extend(Shape::dashed_line(&pts, stroke, 2.5 * z, 4.0 * z)),
                _ => {
                    p.add(Shape::line(pts.clone(), stroke));
                }
            }
            if on {
                // Packets flowing away from the internet.
                // A growing offset moves the dashes downstream; kept within
                // one period (a negative one draws past the line's start).
                let off = (now as f32 * 40.0 * z).rem_euclid(18.0 * z);
                let glow = Stroke::new(w + 1.0, Color32::WHITE.gamma_multiply(0.55 * alpha));
                p.extend(Shape::dashed_line_with_offset(
                    &pts,
                    glow,
                    &[4.0 * z],
                    &[14.0 * z],
                    off,
                ));
                flows = true;
            }
            if let Some(l) = label
                && z >= 0.7
                && alpha > 0.3
            {
                let end = *pts.last().expect("route has points");
                // A stacked card's stub is short: just the number over it.
                let (pos, align, l) = if self.lay.stacked[i] {
                    (
                        end - vec2(layout::INDENT / 4.0 * z, 2.0),
                        egui::Align2::CENTER_BOTTOM,
                        l.trim_start_matches("port "),
                    )
                } else {
                    (end + vec2(4.0, -3.0), egui::Align2::LEFT_BOTTOM, l.as_str())
                };
                p.text(
                    pos,
                    align,
                    l,
                    FontId::proportional(10.0 * z.min(1.2)),
                    vis.weak_text_color().gamma_multiply(alpha),
                );
            }
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
            ui.ctx().request_repaint_after(if flows {
                crate::backdrop::SMOOTH
            } else {
                std::time::Duration::from_millis(50)
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn card(
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
        let accent = zone_colour(n.zone, dark);
        let round = (7.0 * z) as u8;
        let fill = if dark && vis.panel_fill.a() == 255 {
            vis.panel_fill.lerp_to_gamma(Color32::WHITE, 0.06)
        } else {
            vis.window_fill
        };
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
            Stroke::new(
                1.0,
                vis.widgets.noninteractive.bg_stroke.color.gamma_multiply(a),
            )
        };
        p.rect(r, round, fill.gamma_multiply(a), border, StrokeKind::Inside);
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

    /// − 100% + Fit in the canvas corner, for when there's no wheel.
    fn zoom_controls(&mut self, ui: &mut Ui, rect: Rect) {
        let h = 22.0;
        let mut x = rect.max.x - 10.0;
        let y = rect.max.y - 10.0 - h;
        let mut button = |ui: &mut Ui, w: f32, text: &str, tip: &str| {
            x -= w;
            let r = Rect::from_min_size(pos2(x, y), vec2(w, h));
            x -= 4.0;
            ui.put(r, egui::Button::new(text).corner_radius(6))
                .on_hover_text(tip)
                .clicked()
        };
        let fit = button(ui, 36.0, "Fit", "Show the whole map");
        let zoom_in = button(ui, 24.0, "+", "Zoom in");
        let pct = format!("{:.0}%", self.zoom * 100.0);
        let reset = button(ui, 48.0, &pct, "Back to 100%");
        let zoom_out = button(ui, 24.0, "−", "Zoom out");
        let target = if zoom_in {
            Some(self.zoom * 1.25)
        } else if zoom_out {
            Some(self.zoom / 1.25)
        } else if reset {
            Some(1.0)
        } else {
            None
        };
        if fit {
            self.follow = true;
            self.fit(rect);
        } else if let Some(z) = target {
            // Zoom about the middle of the canvas.
            let z = z.clamp(MIN_ZOOM, MAX_ZOOM);
            let mid = self.to_world(rect, rect.center());
            self.aim = Some((rect.size() / 2.0 - mid.to_vec2() * z, z));
            self.follow = false;
        }
    }

    /// Roles to fade in and out, and the action items, worst first: the
    /// sidebar /mapnetwork's maps have.
    fn sidebar(&mut self, ui: &mut Ui, focus: &mut Option<Vec<usize>>) {
        let dark = ui.visuals().dark_mode;
        let counts: Vec<(Zone, usize)> = Zone::ALL
            .iter()
            .map(|&z| (z, self.map.nodes.iter().filter(|n| n.zone == z).count()))
            .filter(|c| c.1 > 0 && c.0 != Zone::Internet)
            .collect();
        let mut findings: Vec<(Sev, String, Vec<usize>)> = self
            .map
            .findings
            .iter()
            .map(|f| (f.sev, f.text.clone(), f.nodes.clone()))
            .collect();
        findings.sort_by_key(|f| f.0);
        let (mut hide, mut flip) = (false, None);
        egui::Panel::left("map_side")
            .resizable(true)
            .default_size(230.0)
            .size_range(180.0..=380.0)
            .show_collapsible(ui, &mut self.side, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(format!("{} cards", self.map.nodes.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        hide = ui
                            .small_button("‹")
                            .on_hover_text("Hide this panel")
                            .clicked();
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("map_side_scroll")
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        egui::CollapsingHeader::new(RichText::new("ROLES").small().strong())
                            .id_salt("map_side_roles")
                            .default_open(true)
                            .show(ui, |ui| {
                                for (zone, n) in &counts {
                                    let off = self.hidden.contains(zone);
                                    let c = zone_colour(*zone, dark);
                                    let r = ui
                                        .horizontal(|ui| {
                                            let (d, _) = ui.allocate_exact_size(
                                                vec2(12.0, 12.0),
                                                Sense::hover(),
                                            );
                                            if off {
                                                ui.painter().circle_stroke(
                                                    d.center(),
                                                    4.5,
                                                    Stroke::new(1.5, c),
                                                );
                                            } else {
                                                ui.painter().circle_filled(d.center(), 5.0, c);
                                            }
                                            let text = RichText::new(zone.name());
                                            ui.label(if off { text.weak() } else { text });
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| ui.weak(n.to_string()),
                                            );
                                        })
                                        .response
                                        .interact(Sense::click())
                                        .on_hover_text("Click to fade this role out or back in");
                                    if r.clicked() {
                                        flip = Some(*zone);
                                    }
                                    if r.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                }
                            });
                        if !findings.is_empty() {
                            egui::CollapsingHeader::new(
                                RichText::new(format!("ACTION ITEMS ({})", findings.len()))
                                    .small()
                                    .strong(),
                            )
                            .id_salt("map_side_actions")
                            .default_open(true)
                            .show(ui, |ui| {
                                for (sev, text, nodes) in &findings {
                                    let c = match sev {
                                        Sev::High => widgets::bad(),
                                        Sev::Med => widgets::warn(),
                                        Sev::Low => ui.visuals().weak_text_color(),
                                    };
                                    let r = ui
                                        .horizontal_top(|ui| {
                                            let (d, _) = ui.allocate_exact_size(
                                                vec2(10.0, 16.0),
                                                Sense::hover(),
                                            );
                                            ui.painter().circle_filled(
                                                d.center() - vec2(0.0, 2.0),
                                                4.0,
                                                c,
                                            );
                                            ui.add(egui::Label::new(text).wrap());
                                        })
                                        .response;
                                    if !nodes.is_empty() {
                                        let r = r
                                            .interact(Sense::click())
                                            .on_hover_text("Show on the map");
                                        if r.clicked() {
                                            *focus = Some(nodes.clone());
                                        }
                                        if r.hovered() {
                                            ui.ctx()
                                                .set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                    }
                                    ui.add_space(4.0);
                                }
                            });
                        }
                    });
            });
        if hide {
            self.side = false;
        }
        if let Some(z) = flip
            && !self.hidden.remove(&z)
        {
            self.hidden.insert(z);
        }
    }

    /// Role legend in the canvas corner; click a role to fade it out.
    fn legend(&mut self, ui: &mut Ui, rect: Rect) {
        let dark = ui.visuals().dark_mode;
        let mut counts: Vec<(Zone, usize)> = Zone::ALL
            .iter()
            .map(|&z| (z, self.map.nodes.iter().filter(|n| n.zone == z).count()))
            .filter(|c| c.1 > 0 && c.0 != Zone::Internet)
            .collect();
        if self.map.hidden > 0 {
            counts.retain(|c| c.1 > 0);
        }
        let font = FontId::proportional(11.5);
        let mut x = rect.min.x + 10.0;
        let y = rect.max.y - 10.0;
        let bg = ui.visuals().window_fill.gamma_multiply(0.92);
        for (zone, n) in counts {
            let label = format!("{}  {n}", zone.name());
            let g = ui
                .painter()
                .layout_no_wrap(label, font.clone(), ui.visuals().text_color());
            let r = Rect::from_min_size(
                pos2(x, y - g.size().y - 8.0),
                vec2(g.size().x + 26.0, g.size().y + 8.0),
            );
            // Room left for the zoom controls.
            if r.max.x > rect.max.x - 180.0 {
                break;
            }
            let resp = ui
                .interact(r, ui.id().with(("legend", zone)), Sense::click())
                .on_hover_text("Click to fade this role out or back in");
            if resp.clicked() && !self.hidden.remove(&zone) {
                self.hidden.insert(zone);
            }
            let off = self.hidden.contains(&zone);
            let p = ui.painter();
            p.rect(
                r,
                9,
                bg,
                Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
                StrokeKind::Inside,
            );
            let c = zone_colour(zone, dark);
            let dot = pos2(r.min.x + 11.0, r.center().y);
            if off {
                p.circle_stroke(dot, 4.0, Stroke::new(1.5, c));
            } else {
                p.circle_filled(dot, 4.5, c);
            }
            let tc = if off {
                ui.visuals().weak_text_color()
            } else {
                ui.visuals().text_color()
            };
            p.galley(pos2(r.min.x + 20.0, r.min.y + 4.0), g, tc);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            x = r.max.x + 6.0;
        }
    }
}

/// A card's details as text, for Copy.
fn card_text(n: &crate::map::Node) -> String {
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

/// The map is a quick picture, not a survey: say so, and point to the
/// reviewed path. Site Bundle opens it; × closes the note for good.
fn accuracy_note(ui: &mut Ui) -> Option<Action> {
    let warn = widgets::warn();
    let mut act = None;
    egui::Frame::new()
        .fill(warn.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, warn.gamma_multiply(0.5)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            // One row: the text wraps in the room the buttons leave.
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚠").color(warn));
                let room = (ui.available_width() - 150.0).max(160.0);
                ui.allocate_ui(egui::vec2(room, 0.0), |ui| {
                    ui.add(
                        egui::Label::new(
                            "Built from what this app can see, so it may be incomplete or wrong. \
For an accurate map, make a Site Bundle and run /mapnetwork in Claude.",
                        )
                        .wrap(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new("×").frame(false))
                        .on_hover_text("Close this note")
                        .clicked()
                    {
                        act = Some(Action::HideNote);
                    }
                    if ui
                        .small_button("Site Bundle…")
                        .on_hover_text("Zip it all for /mapnetwork")
                        .clicked()
                    {
                        act = Some(Action::Bundle);
                    }
                });
            });
        });
    ui.add_space(2.0);
    act
}

pub fn zone_colour(z: Zone, dark: bool) -> Color32 {
    let [r, g, b] = z.rgb(dark);
    Color32::from_rgb(r, g, b)
}

fn empty_state(ui: &mut Ui, info: &HostInfo) -> Option<Action> {
    let mut act = None;
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).max(12.0));
        ui.label(RichText::new("Nothing to map yet").heading());
        ui.add_space(6.0);
        ui.label(
            "The map is built from what this app has seen: a Subnet Sweep, and a UniFi backup or \
SonicWall export loaded in Config Export, or a /mapnetwork map opened there. A backup gives the real uplinks; a sweep alone \
can only place devices under the gateway.",
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let subnet = info
                .lan
                .map(|(ip, bits)| crate::net::targets::subnet_cidr(ip, bits));
            let w = 360.0;
            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
            if ui
                .add_enabled(subnet.is_some(), egui::Button::new("Scan this network"))
                .on_hover_text(format!(
                    "Sweep {} and watch the map fill in",
                    subnet.as_deref().unwrap_or("this machine's subnet")
                ))
                .clicked()
            {
                act = Some(Action::Sweep);
            }
            if ui
                .button("Load a UniFi or SonicWall export…")
                .on_hover_text("Opens Config Export (or drop the file on the window)")
                .clicked()
            {
                act = Some(Action::Export);
            }
        });
    });
    act
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Node;

    fn view(n: usize) -> MapView {
        let mut v = MapView::new();
        v.map.nodes = (0..n)
            .map(|i| Node {
                id: format!("n{i}"),
                ..Default::default()
            })
            .collect();
        v
    }

    #[test]
    fn ctrl_click_adds_and_removes() {
        let mut v = view(4);
        v.select(Some(1));
        v.toggle(2);
        v.toggle(3);
        assert_eq!(v.selection(), [1, 2, 3]);
        assert_eq!(v.selected.as_deref(), Some("n3"));
        // Taking out the shown card shows the one picked before it.
        v.toggle(3);
        assert_eq!(v.selection(), [1, 2]);
        assert_eq!(v.selected.as_deref(), Some("n2"));
        v.toggle(1);
        assert_eq!(v.selection(), [2]);
        // A plain click starts over.
        v.toggle(0);
        v.select(Some(3));
        assert_eq!(v.selection(), [3]);
        v.select(None);
        assert!(v.selection().is_empty());
    }

    #[test]
    fn pinned_cards_stay_selected() {
        let mut v = view(4);
        v.select(Some(1));
        v.keep(1);
        // A plain click elsewhere and Esc keep the pinned one.
        v.select(Some(2));
        assert_eq!(v.selection(), [1, 2]);
        v.select(None);
        assert_eq!(v.selection(), [1]);
        // Ctrl+click on it lets it go.
        v.toggle(1);
        assert!(v.selection().is_empty());
        assert!(v.kept.is_empty());
    }

    #[test]
    fn the_panel_puts_new_cards_on_top_and_keeps_moves() {
        let mut v = view(4);
        v.select(Some(0));
        assert_eq!(v.ordered(), [0]);
        v.toggle(1);
        v.toggle(2);
        assert_eq!(v.ordered(), [2, 1, 0]);
        // Dragged to the top, it stays there as others come and go.
        let id = v.order.remove(2);
        v.order.insert(0, id);
        v.toggle(3);
        assert_eq!(v.ordered(), [3, 0, 2, 1]);
        v.toggle(2);
        assert_eq!(v.ordered(), [3, 0, 1]);
    }
}
