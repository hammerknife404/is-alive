//! The Map tab: pan (drag), zoom (wheel or pinch), drag a card to move it,
//! click to trace its path to the internet, search, filter by role, and a
//! findings band. It rebuilds as the sweep finds hosts, so a scan can be
//! watched filling in the map.

use super::layout::{self, CARD, Layout, Mode};
use super::runs::{CardRuns, Out, Run, State, Tool};
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
    /// Read the map's files again and rebuild it.
    Reload,
    /// A layout mode was picked: remember it.
    Layout(Mode),
}

/// What the detail panel asked for.
enum PanelEv {
    /// × on a card: take it out of the selection.
    Drop(usize),
    /// The pin: keep it selected, or not.
    Keep(usize),
    /// Run a spot check from the card.
    Run(usize, Tool),
    /// Stop a card's run, keeping what it found.
    Stop(usize, Tool),
    /// × on a run: take its result out of the card.
    Clear(usize, Tool),
    /// A card dragged from one place in the stack to another.
    Move(usize, usize),
}

pub struct Sources<'a> {
    pub sweep: &'a SweepTool,
    pub export: &'a ExportTool,
    pub info: &'a HostInfo,
    /// Ping, Port Check or Traceroute is running in its own tab: a card's
    /// "Open in tab" waits for it. The card's own runs don't.
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
    /// Where each pinned card sits in the detail panel. New cards fill
    /// around them; only dragging moves them.
    slots: HashMap<String, usize>,
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
    /// Cards the user dragged somewhere on the canvas.
    placed: HashMap<String, Pos2>,
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
    /// Spot checks run from cards, with their results.
    runs: CardRuns,
    /// The layout mode picked; None until one is (then Sites when the map
    /// has sites, else Cluster).
    mode: Option<Mode>,
    /// Rebuild on the next frame even if nothing seems to have changed.
    stale: bool,
    /// The map has two or more sites (Sites mode is offered).
    multi_site: bool,
}

/// Flag on a pinned card that's missing from the latest data.
pub const GONE_FLAG: &str =
    "Not in the latest data. Kept because it's pinned: check it, then unpin it or × to let it go.";

impl MapView {
    pub fn new(mode: Option<Mode>) -> Self {
        Self {
            follow: true,
            side: true,
            mode,
            ..Default::default()
        }
    }

    /// The layout in use: the one picked, if this map can do it.
    fn mode(&self) -> Mode {
        match self.mode {
            Some(Mode::Sites) | None if self.multi_site => Mode::Sites,
            Some(Mode::Sites) | None => Mode::Cluster,
            Some(m) => m,
        }
    }

    /// A card's colour: its role's, or the gone red.
    fn accent(&self, n: &Node) -> Color32 {
        if n.gone {
            gone_colour()
        } else {
            zone_colour(n.zone)
        }
    }

    /// Rebuild from the data on the next frame (Reload map).
    pub fn reload(&mut self) {
        self.stale = true;
    }

    /// Pinned cards missing from `new`, carried over from `old` and marked
    /// gone, under their nearest ancestor that's still there. A card only
    /// folded away (clients off, or past the cap) isn't gone: its pin just
    /// lapses, as before. One whose id changed (a sweep host a backup now
    /// knows by MAC) is found by MAC or IP, and its pin moves to the new
    /// card: returned as (old id, new id).
    fn carry_gone(&self, old: &Map, new: &mut Map) -> Vec<(String, String)> {
        let mut moved = Vec::new();
        if new.nodes.is_empty() {
            return moved;
        }
        // Only cards from the data count as parents or matches, never the
        // gone ones pushed below.
        let base = new.nodes.len();
        for id in &self.kept {
            if new.nodes[..base].iter().any(|n| n.id == *id) || new.folded.contains(id) {
                continue;
            }
            let Some(o) = old.nodes.iter().find(|n| n.id == *id) else {
                continue;
            };
            if !o.gone
                && let Some(n) = new.nodes[..base].iter().find(|n| {
                    !self.kept.contains(&n.id)
                        && ((o.mac.is_some() && n.mac == o.mac) || (o.ip.is_some() && n.ip == o.ip))
                })
            {
                moved.push((id.clone(), n.id.clone()));
                continue;
            }
            // Its parent, or the nearest card above it still in the data.
            let parent = old
                .path(old.nodes.iter().position(|n| n.id == *id).unwrap_or(0))
                .into_iter()
                .rev()
                .skip(1)
                .find_map(|a| {
                    new.nodes[..base]
                        .iter()
                        .position(|n| n.id == old.nodes[a].id)
                })
                .unwrap_or(0);
            let mut g = o.clone();
            g.up = Some((parent, Link::Assumed, None));
            if !g.gone {
                g.gone = true;
                g.kind = "NOT IN LATEST DATA".into();
                g.flags.insert(0, GONE_FLAG.into());
            }
            new.nodes.push(g);
        }
        moved
    }

    /// A pinned card's id changed: the pin, its panel slot, the selection
    /// and its card results move to the new id.
    fn rename(&mut self, from: &str, to: &str) {
        for v in [&mut self.kept, &mut self.also, &mut self.order] {
            for s in v.iter_mut().filter(|s| *s == from) {
                *s = to.to_string();
            }
        }
        if self.selected.as_deref() == Some(from) {
            self.selected = Some(to.to_string());
        }
        if let Some(k) = self.slots.remove(from) {
            self.slots.insert(to.to_string(), k);
        }
        if let Some(p) = self.pos.remove(from) {
            self.pos.insert(to.to_string(), p);
        }
        self.runs.rename(from, to);
    }

    /// Take out gone cards that were let go (unpinned or ×).
    fn drop_released(&mut self, ctx: &egui::Context) {
        if !self
            .map
            .nodes
            .iter()
            .any(|n| n.gone && !self.kept.contains(&n.id))
        {
            return;
        }
        // Gone cards sit at the end and hang only off cards from the data
        // (never off each other), so taking them out moves no parent.
        let kept = &self.kept;
        self.map.nodes.retain(|n| !n.gone || kept.contains(&n.id));
        self.relayout(ctx);
    }

    /// Keep cards' runs going while another tab is shown.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.runs.poll(ctx);
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
        self.drop_released(ctx);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        src.sweep.generation().hash(&mut h);
        src.export.generation().hash(&mut h);
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
        if key == self.key && !self.stale {
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
        self.stale = false;
        let input = self.input(src);
        let mut map = if input.is_empty() {
            Map::default()
        } else {
            super::build(&input)
        };
        for (from, to) in self.carry_gone(&self.map, &mut map) {
            self.rename(&from, &to);
        }
        self.map = map;
        self.relayout(ctx);
    }

    fn relayout(&mut self, ctx: &egui::Context) {
        self.multi_site = self.map.sites().len() > 1;
        self.lay = layout::layout_in(&self.map, &|_| true, self.mode());
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
        self.slots.retain(|s, _| ids.contains(s.as_str()));
        self.runs.keep_only(&ids);
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

    /// Pin a card in the selection, or let it go. A pinned card holds the
    /// panel slot it's in (the top, if it isn't in the panel yet); let go,
    /// it stays put and moves with the rest from then on.
    fn keep(&mut self, i: usize) {
        let id = self.map.nodes[i].id.clone();
        match self.kept.iter().position(|s| *s == id) {
            Some(k) => {
                self.kept.remove(k);
                self.slots.remove(&id);
            }
            None => {
                let at = self.order.iter().position(|s| *s == id).unwrap_or(0);
                self.slots.insert(id.clone(), at);
                self.kept.push(id);
            }
        }
    }

    fn is_kept(&self, i: usize) -> bool {
        self.kept.contains(&self.map.nodes[i].id)
    }

    /// The panel's cards in their order: pinned ones in their slots, the
    /// rest around them, new ones on top, then as dragged.
    fn ordered(&mut self) -> Vec<usize> {
        let sel = self.selection();
        let ids: Vec<String> = sel.iter().map(|&i| self.map.nodes[i].id.clone()).collect();
        self.slots.retain(|s, _| self.kept.contains(s));
        let mut free: Vec<String> = self
            .order
            .iter()
            .filter(|s| ids.contains(s) && !self.slots.contains_key(*s))
            .cloned()
            .collect();
        // Selection runs oldest to newest, so the newest lands on top.
        for id in &ids {
            if !self.slots.contains_key(id) && !free.contains(id) {
                free.insert(0, id.clone());
            }
        }
        // Pinned cards go back in their slots, lowest first so each insert
        // leaves the ones above it alone. A slot past the end shows last but
        // is kept, so the card returns to it as the panel fills up again.
        let mut fixed: Vec<(usize, &String)> = ids
            .iter()
            .filter_map(|s| self.slots.get(s).map(|&k| (k, s)))
            .collect();
        fixed.sort();
        for (k, id) in fixed {
            free.insert(k.min(free.len()), id.clone());
        }
        self.order = free;
        self.order.iter().filter_map(|s| self.index(s)).collect()
    }

    /// A card dragged from one panel slot to another. Pinned cards take the
    /// slots they're now shown in.
    fn move_card(&mut self, from: usize, to: usize) {
        if from >= self.order.len() || to >= self.order.len() {
            return;
        }
        let id = self.order.remove(from);
        self.order.insert(to, id);
        for (k, id) in self.order.iter().enumerate() {
            if let Some(slot) = self.slots.get_mut(id) {
                *slot = k;
            }
        }
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
            self.slots.remove(&id);
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
        self.placed.get(&n.id).copied().or(self.lay.pos[i])
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

    /// Bounds of the laid-out map, with cards dragged elsewhere.
    fn bounds(&self) -> Rect {
        let mut r = Rect::NOTHING;
        for i in 0..self.map.nodes.len() {
            if let Some(p) = self.target(i) {
                r = r.union(Rect::from_min_size(p, CARD));
            }
        }
        for b in &self.lay.boxes {
            r = r.union(b.rect);
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
                    .add_enabled(!self.placed.is_empty(), egui::Button::new("Re-layout"))
                    .on_hover_text("Undo card moves")
                    .clicked()
                {
                    self.placed.clear();
                    fit = true;
                }
                if ui
                    .button("⟲ Reload")
                    .on_hover_text(
                        "Read the map's files again and rebuild it, keeping pins and the \
                         selection. (It already updates by itself as a sweep finds hosts or a \
                         file is loaded.)",
                    )
                    .clicked()
                {
                    act = Some(Action::Reload);
                }
                ui.separator();
                let now = self.mode();
                for m in Mode::ALL {
                    if m == Mode::Sites && !self.multi_site {
                        continue;
                    }
                    if ui
                        .selectable_label(now == m, m.name())
                        .on_hover_text(m.tip())
                        .clicked()
                        && now != m
                    {
                        self.mode = Some(m);
                        self.placed.clear();
                        self.relayout(&ctx);
                        fit = true;
                        act = Some(Action::Layout(m));
                    }
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
                                &self.lay.boxes,
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
            Some(PanelEv::Move(from, to)) => self.move_card(from, to),
            Some(PanelEv::Run(i, tool)) => {
                let n = &self.map.nodes[i];
                if let Some(ip) = n.ip {
                    self.runs.start(&n.id, tool, ip.into());
                    self.runs.poll(ui.ctx());
                }
            }
            Some(PanelEv::Stop(i, tool)) => self.runs.stop(&self.map.nodes[i].id, tool),
            Some(PanelEv::Clear(i, tool)) => self.runs.clear(&self.map.nodes[i].id, tool),
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
        let accent = self.accent(n);
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

                ui.add_space(6.0);
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
                    ui.add_space(4.0);
                    egui::CollapsingHeader::new(RichText::new("RESULTS").small().strong())
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

        // Boxes (Zones and Sites modes), under everything else.
        for b in &self.lay.boxes {
            let r = Rect::from_min_size(self.to_screen(rect, b.rect.min), b.rect.size() * z);
            if !r.intersects(rect) {
                continue;
            }
            let c = b.zone.map_or(vis.weak_text_color(), zone_colour);
            let a = if b.zone.is_some_and(|zn| self.hidden.contains(&zn)) {
                0.2
            } else {
                1.0
            };
            p.rect(
                r,
                (10.0 * z) as u8,
                c.gamma_multiply(0.07 * a),
                Stroke::new(1.2, c.gamma_multiply(0.5 * a)),
                StrokeKind::Inside,
            );
            if z >= 0.25 {
                p.text(
                    r.min + vec2(layout::BOX_PAD * z, layout::BOX_TITLE * z / 2.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{} · {}", b.title.to_uppercase(), b.count),
                    FontId::proportional((11.0 * z).max(7.0)),
                    c.gamma_multiply(a),
                );
            }
        }

        // Links.
        let glow = glow_strength(vis);
        let mut flows = false;
        // One link's line: halo, stroke, the flow when lit, and a label at
        // its end. True while it flows.
        let draw = |pts: &[Pos2],
                    base: Color32,
                    link: Link,
                    w: f32,
                    alpha: f32,
                    on: bool,
                    label: Option<(Pos2, egui::Align2, &str)>|
         -> bool {
            let c = base.gamma_multiply(alpha * if on { 1.0 } else { 0.85 });
            // A soft halo in the link's colour under it (not under the
            // grey assumed ones, which should stay quiet).
            if glow > 0.0 && link != Link::Assumed {
                let halo = Stroke::new(
                    w + 5.0 * z.clamp(0.6, 1.4),
                    base.gamma_multiply(glow * alpha * if on { 1.4 } else { 1.0 }),
                );
                if link == Link::Wireless {
                    p.extend(Shape::dashed_line(pts, halo, 7.0 * z, 5.0 * z));
                } else {
                    p.add(Shape::line(pts.to_vec(), halo));
                }
            }
            let stroke = Stroke::new(w, c);
            match link {
                Link::Wireless => p.extend(Shape::dashed_line(pts, stroke, 7.0 * z, 5.0 * z)),
                Link::Assumed => p.extend(Shape::dashed_line(pts, stroke, 2.5 * z, 4.0 * z)),
                _ => {
                    p.add(Shape::line(pts.to_vec(), stroke));
                }
            }
            if on {
                // Packets flowing away from the internet.
                // A growing offset moves the dashes downstream; kept within
                // one period (a negative one draws past the line's start).
                let off = (now as f32 * 40.0 * z).rem_euclid(18.0 * z);
                let glow = Stroke::new(w + 1.0, Color32::WHITE.gamma_multiply(0.55 * alpha));
                p.extend(Shape::dashed_line_with_offset(
                    pts,
                    glow,
                    &[4.0 * z],
                    &[14.0 * z],
                    off,
                ));
            }
            if let Some((pos, align, l)) = label
                && z >= 0.7
                && alpha > 0.3
            {
                p.text(
                    pos,
                    align,
                    l,
                    FontId::proportional(10.0 * z.min(1.2)),
                    vis.weak_text_color().gamma_multiply(alpha),
                );
            }
            on
        };
        let width = |link: Link, on: bool| {
            let base = if link == Link::Backbone { 3.2 } else { 2.0 };
            base * z.clamp(0.6, 1.4) + if on { 0.8 } else { 0.0 }
        };
        let colour = |n: &Node, link: Link| match link {
            Link::Backbone => zone_colour(Zone::Gateway),
            Link::Assumed => vis.weak_text_color(),
            _ => self.accent(n),
        };
        let group = |i: usize| self.lay.group.get(i).copied().flatten();
        // Links from one card into another box fold into one trunk:
        // (parent, box) → (children, any lit, brightest).
        let mut trunks: std::collections::BTreeMap<(usize, usize), (Vec<usize>, bool, f32)> =
            Default::default();
        for (i, n) in self.map.nodes.iter().enumerate() {
            let Some((up, link, label)) = &n.up else {
                continue;
            };
            let alpha = fade(i).min(fade(*up));
            let on = !sel.is_empty() && lit.contains(&i) && lit.contains(up);
            if let Some(g) = group(i)
                && group(*up) != Some(g)
            {
                let t = trunks.entry((*up, g)).or_default();
                t.0.push(i);
                t.1 |= on;
                t.2 = t.2.max(alpha);
                continue;
            }
            // Inside one box, links would cut across its grid: shown only
            // while one end is selected.
            if group(i).is_some() && group(i) == group(*up) && !on {
                continue;
            }
            let (Some(a), Some(b)) = (self.pos.get(&self.map.nodes[*up].id), self.pos.get(&n.id))
            else {
                continue;
            };
            let pts: Vec<Pos2> = layout::route(*a, *b, self.lay.stacked[i])
                .into_iter()
                .map(|w| self.to_screen(rect, w))
                .collect();
            let end = *pts.last().expect("route has points");
            // A stacked card's stub is short: just the number over it.
            let label = label.as_deref().map(|l| {
                if self.lay.stacked[i] {
                    (
                        end - vec2(layout::INDENT / 4.0 * z, 2.0),
                        egui::Align2::CENTER_BOTTOM,
                        l.trim_start_matches("port "),
                    )
                } else {
                    (end + vec2(4.0, -3.0), egui::Align2::LEFT_BOTTOM, l)
                }
            });
            flows |= draw(
                &pts,
                colour(n, *link),
                *link,
                width(*link, on),
                alpha,
                on,
                label,
            );
        }
        for ((up, g), (kids, on, alpha)) in &trunks {
            let (Some(a), Some(b)) = (
                self.pos.get(&self.map.nodes[*up].id),
                self.lay.boxes.get(*g),
            ) else {
                continue;
            };
            let from = pos2(a.x + CARD.x / 2.0, a.y + CARD.y);
            let world: Vec<Pos2> = if b.rect.top() > from.y + 8.0 {
                // Below: into the middle of the box's top edge.
                let into = pos2(b.rect.center().x - CARD.x / 2.0, b.rect.top());
                layout::route(*a, into, false)
            } else {
                // Beside it (same row): under both boxes and up into its
                // bottom, rather than along the cards' edges.
                let own = group(*up).and_then(|g| self.lay.boxes.get(g));
                let floor =
                    own.map_or(from.y, |o| o.rect.bottom()).max(b.rect.bottom()) + layout::BOX_PAD;
                let x = b.rect.center().x;
                vec![
                    from,
                    pos2(from.x, floor),
                    pos2(x, floor),
                    pos2(x, b.rect.bottom()),
                ]
            };
            let pts: Vec<Pos2> = world.into_iter().map(|w| self.to_screen(rect, w)).collect();
            let first = &self.map.nodes[kids[0]];
            let link = first.link().unwrap_or(Link::Wired);
            let count = format!("×{}", kids.len());
            let end = *pts.last().expect("route has points");
            let label = (kids.len() > 1).then(|| {
                (
                    end + vec2(5.0, -4.0),
                    egui::Align2::LEFT_BOTTOM,
                    count.as_str(),
                )
            });
            let base = b.zone.map_or_else(|| colour(first, link), zone_colour);
            flows |= draw(&pts, base, link, width(link, *on) + 0.6, *alpha, *on, label);
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
        let accent = self.accent(n);
        let round = (7.0 * z) as u8;
        let fill = if dark && vis.panel_fill.a() == 255 {
            vis.panel_fill.lerp_to_gamma(Color32::WHITE, 0.06)
        } else {
            vis.window_fill
        }
        // Tinted with its role's colour, so the map reads by colour at a glance.
        .lerp_to_gamma(accent, if dark { 0.13 } else { 0.07 });
        let glow = glow_strength(vis);
        if glow > 0.0 && !selected && !hovered {
            let halo = egui::Shadow {
                offset: [0, 0],
                blur: (12.0 * z) as u8,
                spread: 0,
                color: accent.gamma_multiply(glow * 0.8 * a),
            };
            p.add(halo.as_shape(r, round));
        }
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
            Stroke::new(1.0, accent.gamma_multiply(0.5 * a))
        };
        if n.gone {
            // Dashed: it isn't in the data any more.
            p.rect_filled(r, round, fill.gamma_multiply(a));
            let q = r.shrink(1.0);
            let outline = [
                q.left_top(),
                q.right_top(),
                q.right_bottom(),
                q.left_bottom(),
                q.left_top(),
            ];
            p.extend(Shape::dashed_line(
                &outline,
                Stroke::new(border.width.max(1.5), accent.gamma_multiply(a)),
                6.0 * z,
                4.0 * z,
            ));
        } else {
            p.rect(r, round, fill.gamma_multiply(a), border, StrokeKind::Inside);
        }
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
                                    let c = zone_colour(*zone);
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
            let c = zone_colour(zone);
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

/// What a run's section in a card asked for.
enum RunEv {
    Again,
    Stop,
    Clear,
    Tab,
}

/// One run in a card's Results: the tool, its condensed line and buttons;
/// opened, every reply, port or hop.
fn run_section(ui: &mut Ui, card: &str, r: &Run, busy: bool) -> Option<RunEv> {
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
fn run_detail(ui: &mut Ui, r: &Run) {
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

/// Pinned cards missing from the latest data: a red no role uses.
pub fn gone_colour() -> Color32 {
    if crate::theme::current().is_light() {
        Color32::from_rgb(200, 30, 30)
    } else {
        Color32::from_rgb(255, 64, 64)
    }
}

/// A role's colour in the current theme.
pub fn zone_colour(z: Zone) -> Color32 {
    z.colour_in(crate::theme::current())
}

/// How strongly links and cards glow: none on light themes, most on RGB
/// and Miami.
fn glow_strength(vis: &egui::Visuals) -> f32 {
    match crate::theme::current() {
        _ if !vis.dark_mode => 0.0,
        crate::theme::Theme::Rgb | crate::theme::Theme::Miami => 0.32,
        _ => 0.16,
    }
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
        let mut v = MapView::new(None);
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

    #[test]
    fn a_pinned_card_missing_from_new_data_stays_marked_until_let_go() {
        let mut v = view(4);
        v.map.nodes[2].up = Some((1, Link::Wired, None));
        v.select(Some(2));
        v.keep(2);
        v.select(Some(3));
        let old = Map {
            nodes: v.map.nodes.clone(),
            ..Default::default()
        };
        // The new data has no n2, and n1 is now at index 0.
        let mut new = Map {
            nodes: old.nodes.iter().filter(|n| n.id != "n2").cloned().collect(),
            ..Default::default()
        };
        new.nodes.swap(0, 1);
        v.carry_gone(&old, &mut new);
        let g = new.nodes.last().unwrap();
        assert_eq!((g.id.as_str(), g.gone), ("n2", true));
        assert_eq!(g.flags[0], GONE_FLAG);
        // Under its old parent, by id, with an assumed link.
        assert!(matches!(g.up, Some((0, Link::Assumed, _))));
        assert_eq!(new.nodes[0].id, "n1");
        // Rebuilt again, it isn't flagged twice.
        let mut again = Map {
            nodes: new.nodes[..3].to_vec(),
            ..Default::default()
        };
        v.carry_gone(&new, &mut again);
        assert_eq!(again.nodes[3].flags.len(), new.nodes[3].flags.len());
        // Unpinned, it goes; the rest stay, still selected.
        v.map = again;
        v.relayout(&egui::Context::default());
        assert_eq!(v.selection(), [3, 2]);
        v.keep(3);
        v.drop_released(&egui::Context::default());
        assert_eq!(v.map.nodes.len(), 3);
        assert!(v.map.nodes.iter().all(|n| !n.gone));
        assert_eq!(v.selection(), [2]);
    }

    fn without(old: &Map, gone: &[&str]) -> Map {
        Map {
            nodes: old
                .nodes
                .iter()
                .filter(|n| !gone.contains(&n.id.as_str()))
                .cloned()
                .collect(),
            ..Default::default()
        }
    }

    fn copy(m: &Map) -> Map {
        Map {
            nodes: m.nodes.clone(),
            ..Default::default()
        }
    }

    #[test]
    fn a_gone_card_never_hangs_off_another_gone_card() {
        // n0 → n1 → n2 (pinned) → n3 (pinned).
        let mut v = view(4);
        for i in 1..4 {
            v.map.nodes[i].up = Some((i - 1, Link::Wired, None));
        }
        v.keep(2);
        v.keep(3);
        let old = copy(&v.map);
        let mut new = without(&old, &["n2", "n3"]);
        v.carry_gone(&old, &mut new);
        // Both under n1, the nearest card still in the data.
        assert_eq!(new.nodes[2].id, "n2");
        assert_eq!(new.nodes[3].id, "n3");
        assert!(matches!(new.nodes[2].up, Some((1, Link::Assumed, _))));
        assert!(matches!(new.nodes[3].up, Some((1, Link::Assumed, _))));
        // Letting n2 go leaves n3 under n1, not under itself.
        v.map = new;
        v.relayout(&egui::Context::default());
        v.keep(2);
        v.drop_released(&egui::Context::default());
        assert_eq!(v.map.nodes[2].id, "n3");
        assert_eq!(v.map.nodes[2].parent(), Some(1));
    }

    #[test]
    fn a_pinned_card_only_folded_away_isnt_called_gone() {
        let mut v = view(3);
        v.keep(2);
        let old = copy(&v.map);
        let mut new = without(&old, &["n2"]);
        new.folded.insert("n2".into());
        assert!(v.carry_gone(&old, &mut new).is_empty());
        assert_eq!(new.nodes.len(), 2);
    }

    #[test]
    fn a_pin_follows_a_card_whose_id_changed() {
        let mut v = view(2);
        v.map.nodes.push(Node {
            id: "ip:10.0.0.5".into(),
            ip: Some("10.0.0.5".parse().unwrap()),
            up: Some((0, Link::Assumed, None)),
            ..Default::default()
        });
        v.select(Some(2));
        v.keep(2);
        v.ordered();
        let old = copy(&v.map);
        let mut new = without(&old, &["ip:10.0.0.5"]);
        new.nodes.push(Node {
            id: "mac:aa:bb:cc:dd:ee:ff".into(),
            ip: Some("10.0.0.5".parse().unwrap()),
            up: Some((1, Link::Wired, None)),
            ..Default::default()
        });
        let moved = v.carry_gone(&old, &mut new);
        assert_eq!(
            moved,
            [(
                "ip:10.0.0.5".to_string(),
                "mac:aa:bb:cc:dd:ee:ff".to_string()
            )]
        );
        // No red duplicate.
        assert_eq!(new.nodes.len(), 3);
        assert!(new.nodes.iter().all(|n| !n.gone));
        for (from, to) in moved {
            v.rename(&from, &to);
        }
        v.map = new;
        v.relayout(&egui::Context::default());
        assert!(v.is_kept(2));
        assert_eq!(v.slots.get("mac:aa:bb:cc:dd:ee:ff"), Some(&0));
        assert_eq!(v.selected.as_deref(), Some("mac:aa:bb:cc:dd:ee:ff"));
    }

    #[test]
    fn a_card_that_wasnt_pinned_just_goes() {
        let mut v = view(3);
        v.select(Some(2));
        let old = Map {
            nodes: v.map.nodes.clone(),
            ..Default::default()
        };
        let mut new = Map {
            nodes: old.nodes[..2].to_vec(),
            ..Default::default()
        };
        v.carry_gone(&old, &mut new);
        assert_eq!(new.nodes.len(), 2);
    }

    /// Cards 0, 1, 2 selected in that order: the panel shows 2, 1, 0.
    fn three_in_panel(n: usize) -> MapView {
        let mut v = view(n);
        v.select(Some(0));
        v.toggle(1);
        v.toggle(2);
        assert_eq!(v.ordered(), [2, 1, 0]);
        v
    }

    #[test]
    fn a_pinned_card_holds_its_slot_as_others_are_selected() {
        let mut v = three_in_panel(6);
        v.keep(0);
        for i in 3..6 {
            v.toggle(i);
            assert_eq!(v.ordered()[2], 0);
        }
        assert_eq!(v.ordered(), [5, 4, 0, 3, 2, 1]);
        // A plain click elsewhere doesn't move it either.
        v.select(Some(3));
        assert_eq!(v.ordered(), [3, 0]);
        v.toggle(4);
        v.toggle(5);
        assert_eq!(v.ordered(), [5, 4, 0, 3]);
    }

    #[test]
    fn dragging_a_pinned_card_gives_it_a_new_slot() {
        let mut v = three_in_panel(6);
        v.keep(0);
        v.ordered();
        v.move_card(2, 0);
        assert_eq!(v.ordered(), [0, 2, 1]);
        v.toggle(3);
        v.toggle(4);
        assert_eq!(v.ordered(), [0, 4, 3, 2, 1]);
        v.toggle(3);
        assert_eq!(v.ordered(), [0, 4, 2, 1]);
    }

    #[test]
    fn dragging_another_card_past_a_pinned_one_updates_its_slot() {
        let mut v = three_in_panel(4);
        v.keep(1);
        v.ordered();
        // 2 dragged below 1 and 0: 1 now shows at the top and stays there.
        v.move_card(0, 2);
        assert_eq!(v.ordered(), [1, 0, 2]);
        v.toggle(3);
        assert_eq!(v.ordered(), [1, 3, 0, 2]);
    }

    #[test]
    fn an_unpinned_card_stays_put_then_moves_with_the_rest() {
        let mut v = three_in_panel(5);
        v.keep(0);
        v.toggle(3);
        assert_eq!(v.ordered(), [3, 2, 0, 1]);
        v.keep(0);
        assert_eq!(v.ordered(), [3, 2, 0, 1]);
        v.toggle(4);
        assert_eq!(v.ordered(), [4, 3, 2, 0, 1]);
    }

    #[test]
    fn a_slot_past_the_end_shows_last_and_comes_back() {
        let mut v = three_in_panel(7);
        v.toggle(3);
        v.toggle(4);
        assert_eq!(v.ordered(), [4, 3, 2, 1, 0]);
        v.keep(0);
        // Down to two cards: it shows last.
        v.select(Some(5));
        assert_eq!(v.ordered(), [5, 0]);
        // Filled up again: back in slot 4.
        for i in [1, 2, 3, 6] {
            v.toggle(i);
        }
        assert_eq!(v.ordered()[4], 0);
        assert_eq!(v.ordered().len(), 6);
    }

    #[test]
    fn pinning_from_the_menu_puts_an_unselected_card_on_top() {
        let mut v = three_in_panel(4);
        v.keep(3);
        assert_eq!(v.ordered(), [3, 2, 1, 0]);
        v.select(Some(1));
        assert_eq!(v.ordered(), [3, 1]);
    }

    #[test]
    fn letting_go_of_a_pinned_card_forgets_its_slot() {
        let mut v = three_in_panel(4);
        v.keep(0);
        v.toggle(0);
        assert!(v.slots.is_empty());
        assert_eq!(v.ordered(), [2, 1]);
        v.toggle(0);
        assert_eq!(v.ordered(), [0, 2, 1]);
    }
}
