//! The Map tab: pan (drag), zoom (wheel or pinch), drag a card to move it,
//! click to trace its path to the internet, Ctrl+click to see what joins
//! several cards, search, filter by role, and a findings band. F fits, +
//! and − zoom, 0 is 100% and the arrows pan from the keyboard. It rebuilds
//! as the sweep finds hosts, so a scan can be watched filling in the map.

mod export;
mod history;
mod interact;
mod legend;
mod notes;
mod paint;
mod panel;
mod reveal;
pub(crate) mod saved;
mod sidebar;
mod since;
mod tabs;

use super::layout::{self, CARD, Layout, Links, Mode};
use super::runs::{CardRuns, Out, Run, State, Tool};
use super::{Clients, Input, Link, Map, Node, Sev, Zone};
use crate::net::hostinfo::HostInfo;
use crate::tools::{export::ExportTool, sweep::SweepTool};
use crate::widgets;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Painter, Pos2, Rect, RichText, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, pos2, text::LayoutJob, vec2,
};
use interact::title_above;
use panel::card_text;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
pub use tabs::MapTabs;

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
    /// Open example map `n` (`skills::EXAMPLES`).
    Example(usize),
    /// Close the example map, back to this network.
    CloseExample,
    /// Explicitly clear live imported and discovered map data.
    ClearLive,
    /// A layout mode was picked: remember it.
    Layout(Mode),
    /// The legend was shown or hidden: remember it.
    Legend(bool),
    /// Redact or keep a value picked in a card note's editor (its text),
    /// wherever it appears: the Notes tab keeps the lists.
    NotePick(String, crate::redact::Pick),
}

/// What a right-click menu item adds to the cards it acts on.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Grow {
    Uplink,
    Path,
    Children,
    Below,
}

/// What the detail panel asked for.
enum PanelEv {
    /// How many clients to show under this uplink (None: as the level says).
    Reveal(usize, Option<usize>),
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
    /// Trace to WAN ticked or not.
    TraceWan(bool),
    /// A card's note: write, save, cancel, remove, redact.
    Note(notes::NoteEv),
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
    /// Show the legend on the map (as saved in Settings).
    pub legend: bool,
    /// What the Notes tab redacts and keeps (and the loaded backups'
    /// secrets): card notes are redacted the same way.
    pub redact: &'a crate::redact::Overrides,
    /// Settings' Animate: packets flow along lit links.
    pub animate: bool,
}

/// Far enough out for Fit to show a wide Tree of a big map whole: its
/// cards are colour tiles by then.
const MIN_ZOOM: f32 = 0.07;
const MAX_ZOOM: f32 = 2.5;

#[derive(Default)]
pub struct MapView {
    map: Map,
    key: u64,
    clients: Clients,
    /// Uplinks stepped on their own (+N or − on the card, Show all): how
    /// many clients each shows, whatever the level.
    reveal: HashMap<String, usize>,
    /// The uplink just stepped: kept still on screen through the rebuild.
    hold: Option<String>,
    search: String,
    /// The card the detail panel shows: the one clicked last.
    selected: Option<String>,
    /// The shown card was picked on its own (a plain click, search, a
    /// finding, a panel link): its whole path to the internet lights, and
    /// its children's links. Off for Ctrl+click and right-click menu
    /// selections, which light only what joins their cards.
    traced: bool,
    /// Light each selected card's whole path to the internet, not only
    /// the links between the selected cards.
    trace_wan: bool,
    /// More cards picked with Ctrl+click, lit with it.
    also: Vec<String>,
    /// Cards pinned in the selection: a plain click elsewhere keeps them.
    kept: Vec<String>,
    /// Where each pinned card sits in the detail panel. New cards fill
    /// around them; only dragging moves them.
    slots: HashMap<String, usize>,
    /// The detail panel's order, top first; dragged by hand.
    order: Vec<String>,
    /// What the open right-click menu (or a panel card's ☰) is for.
    menu: Option<Menu>,
    /// What the last plain click on a card replaced, and when: a
    /// double-click puts it back, so zooming to one card of a group keeps
    /// the group.
    undo: Option<interact::Undo>,
    /// A text box had the keyboard or a menu was open last frame: Esc
    /// there was theirs, not the map's.
    keys_busy: bool,
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
    /// The canvas as it was with no card selected: what the map fits to
    /// while the panel is open.
    wide: Option<Rect>,
    /// How wide the detail panel is this frame (it slides), and open.
    panel_now: f32,
    panel_full: f32,
    /// The files the map is drawn from: another set (an example picked, a
    /// file loaded or cleared) is another map, shown whole.
    source: u64,
    drag: Option<String>,
    /// The cards dragged last (the one dragged, or the selection it brought
    /// along): drawn over the rest.
    raised: Vec<String>,
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
    /// The map has two or more sites (Sites is then the default layout).
    multi_site: bool,
    /// Sites the user collapsed to one card. Kept through rebuilds (a
    /// Reload or a new sweep starts from an empty map), so a site back in
    /// the data comes back collapsed.
    closed: HashSet<String>,
    /// Boxes dragged by hand, by `Layout::box_key`: how far from where the
    /// layout put them. Their cards and lines go along.
    shifted: BTreeMap<String, Vec2>,
    /// The box being dragged by its title, by key.
    box_drag: Option<String>,
    /// Which links are drawn (the canvas menu's Links).
    links: Links,
    /// Sites whose title was clicked (or their row in the sidebar, or the
    /// menu): collapse or expand them.
    flip_sites: Vec<String>,
    /// When a site's title on the canvas was last clicked: the second click
    /// of a double-click there is ignored (the boxes have moved under it).
    flipped_at: f64,
    /// The sidebar's starting state was picked for the window's width.
    sized: bool,
    /// The legend is shown on the canvas.
    legend: bool,
    /// The setting the legend was last told (`Sources::legend`): a change
    /// there wins over the toggle here.
    legend_seen: Option<bool>,
    /// How much of the canvas's bottom left corner the legend took last
    /// frame (zero when hidden): Fit keeps the map clear of it.
    legend_size: Vec2,
    /// Steps to undo and redo on this map.
    history: history::History<history::Edits>,
    /// A saved map: drawn from this (every site open) rather than from the
    /// sweep and files.
    frozen: Option<Box<Map>>,
    /// What a saved map is (as found, a plan, a training map); the live
    /// map is always as found.
    purpose: saved::Purpose,
    /// A training map's answer key, kept to be saved again. Never shown:
    /// only how many, in its sub-tab's tooltip.
    faults: Vec<saved::Fault>,
    /// The saved maps bar is showing (the toolbar's toggle), and the
    /// toggle was clicked.
    bar: bool,
    bar_flip: bool,
    /// How wide each of the toolbar's groups was when last drawn (0: not
    /// yet), for which row each goes on.
    bar_w: [f32; BAR_GROUPS],
    /// Re-layout was clicked this frame: Undo names the step for it.
    relaid: bool,
    /// A test runs in its own tab (Sources::busy), as of this frame.
    busy: bool,
    /// What a card menu asked of the app (Open in a tab).
    menu_act: Option<Action>,
    /// Notes on cards, by id, as typed (also in `history`'s steps).
    notes: std::collections::BTreeMap<String, String>,
    /// How notes are redacted (the Notes tab's, as of this frame).
    ov: crate::redact::Overrides,
    /// The note being written, if one is.
    note_edit: Option<notes::Edit>,
    /// What the notes belong to: this saved map, the live map, an example.
    note_scope: notes::Scope,
    /// Where the live map keeps notes on this computer (None: a saved map,
    /// or nowhere to keep them).
    note_store: Option<notes::Store>,
    note_synced: notes::Synced,
    /// How many sweeps had started when the notes were last bound to
    /// cards: a new one drops the notes on cards known only by IP.
    note_sweeps: u64,
    /// What the cards' notes were last put on them with (see `stamp_notes`).
    stamped: u64,
    /// Counts the map's rebuilds.
    built: u64,
    /// Settings' Animate (`Sources::animate`, as of this frame): packets
    /// flow along lit links. Off, lit links still take their cards'
    /// colours, but nothing moves.
    animate: bool,
    /// The lit links as last drawn, kept while they don't change.
    lit_drawn: std::cell::RefCell<crate::map::colours::Lit>,
    /// The saved map the live map is compared with (Compare ⏷), if any.
    since: Option<since::Since>,
}

/// What a right-click menu was opened on.
#[derive(Clone, PartialEq, Debug)]
enum Menu {
    /// A card, by id (the map can rebuild while the menu is open).
    Card(String),
    /// The same menu from a card's ☰ in the detail panel.
    Panel(String),
    /// The empty canvas.
    Background,
    /// A box's title, by `Layout::box_key`.
    Box(String),
}

/// The detail panel's id, and its width until it's dragged.
const PANEL: &str = "map_detail";
const PANEL_SIZE: f32 = 320.0;

/// Why a control is greyed out on an empty map.
const NOTHING_YET: &str = "Nothing on the map yet";

/// Flag on a pinned card that's missing from the latest data.
pub const GONE_FLAG: &str =
    "Not in the latest data. Kept because it's pinned: check it, then unpin it or × to let it go.";

/// What the toolbar asked for this frame.
struct Toolbar {
    act: Option<Action>,
    search_enter: bool,
    fit: bool,
}

/// The toolbar's groups, in order: search, Clients and Scan; Re-layout and
/// Reload; ↩ ↪; the layouts and Legend; Examples and Export.
const BAR_GROUPS: usize = 5;
const BAR_LAYOUTS: usize = 3;
const BAR_END: usize = 4;
/// About how wide each group is, until it's been drawn.
const BAR_GUESS: [f32; BAR_GROUPS] = [560.0, 160.0, 60.0, 340.0, 180.0];
/// Room for the separator before a group and the gap after it.
const BAR_GAP: f32 = 16.0;
/// Narrower than this, the layouts go under the search row even when they
/// would just fit after it: one long row is hard to scan.
const BAR_ONE_ROW: f32 = 1100.0;

/// Which of the toolbar's groups (widths `w`, 0 for one not drawn yet) go
/// on each row in `full`: in order, each on the row so far if it fits,
/// else starting the next. The search group always has the first row.
/// Each group fits the window on its own, the widest (search, Clients and
/// Scan) at the narrowest the window gets.
fn bar_rows(w: &[f32; BAR_GROUPS], full: f32) -> Vec<Vec<usize>> {
    let width = |k: usize| if w[k] > 0.0 { w[k] } else { BAR_GUESS[k] } + BAR_GAP;
    let mut rows = vec![vec![0]];
    let mut used = width(0);
    for k in 1..BAR_GROUPS {
        // The layouts join the search row only with Examples and Export
        // after them: one row for the lot, or the layouts lead the next.
        let alone = k == BAR_LAYOUTS
            && rows.len() == 1
            && (full < BAR_ONE_ROW || used + width(k) + width(BAR_END) > full);
        let fits = used + width(k) <= full && !alone;
        if fits {
            rows.last_mut().expect("a row").push(k);
            used += width(k);
        } else {
            rows.push(vec![k]);
            used = width(k);
        }
    }
    rows
}

/// Cards or a site the canvas should bring into view this frame.
#[derive(Default)]
struct Focus {
    /// From the findings band or the sidebar's action items.
    on: Option<Vec<usize>>,
    /// A site title clicked in the sidebar.
    site: Option<String>,
    /// A card named in the panel (an uplink, a child); true with Ctrl
    /// held, to add it to the selection.
    pick: Option<(usize, bool)>,
    /// Enter in the search box: the first match.
    search: bool,
}

impl MapView {
    pub fn new(mode: Option<Mode>) -> Self {
        // The screenshot tour can start with clients shown
        // (IS_ALIVE_SHOTS_CLIENTS=some, more or all).
        let clients = match std::env::var("IS_ALIVE_SHOTS_CLIENTS").as_deref() {
            Ok("some") => Clients::Some,
            Ok("more") => Clients::More,
            Ok("all") => Clients::All,
            _ => {
                if cfg!(test) || std::env::var_os("IS_ALIVE_SHOTS").is_some() {
                    Clients::Off
                } else {
                    crate::theme::data_dir()
                        .map(|dir| saved::live_clients(&dir))
                        .unwrap_or(Clients::Off)
                }
            }
        };
        Self {
            follow: true,
            side: true,
            legend: true,
            mode,
            clients,
            note_scope: notes::Scope::Live,
            animate: true,
            ..Default::default()
        }
    }

    /// The layout in use: the one picked, else Sites on a map with two or
    /// more sites and Cluster on the rest.
    fn mode(&self) -> Mode {
        match self.mode {
            Some(m) => m,
            None if self.multi_site => Mode::Sites,
            None => Mode::Cluster,
        }
    }

    /// Collapse a site to one card, or expand it again (rebuilds).
    fn flip(&mut self, site: &str) {
        if !self.closed.remove(site) {
            self.closed.insert(site.to_string());
        }
        self.stale = true;
    }

    /// Every site with how many cards it holds, collapsed or not.
    fn site_counts(&self) -> Vec<(String, usize, bool)> {
        self.map
            .sites()
            .into_iter()
            .map(|s| match self.map.closed.get(&s) {
                Some(v) => {
                    let n = v.len();
                    (s, n, true)
                }
                None => {
                    let n = self
                        .map
                        .nodes
                        .iter()
                        .filter(|n| n.site.as_ref() == Some(&s) && n.zone != Zone::Internet)
                        .count();
                    (s, n, false)
                }
            })
            .collect()
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

    /// What a finished sweep found is on this map: it has hosts, and the
    /// map's files don't leave it off as another network's. Built here and
    /// now, as the map itself rebuilds only while it's on screen.
    pub fn shows_sweep(&self, src: &Sources) -> bool {
        let input = self.input(src);
        !input.sweep.is_empty() && !super::build(&input).sweep_apart
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
            if new.nodes[..base].iter().any(|n| n.id == *id)
                || new.folded.contains(id)
                || new.tucked(id)
            {
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
        if let Some(t) = self.notes.remove(from) {
            self.notes.insert(to.to_string(), t);
        }
        self.note_synced.rename(from, to);
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

    fn input<'a>(&'a self, src: &'a Sources) -> Input<'a> {
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
            reveal: Some(&self.reveal),
        }
    }

    /// Rebuild when anything feeding the map changed.
    fn refresh(&mut self, ctx: &egui::Context, src: &Sources) {
        self.drop_released(ctx);
        if self.frozen.is_some() {
            return self.refresh_saved(ctx);
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        src.sweep.generation().hash(&mut h);
        src.export.generation().hash(&mut h);
        self.clients.hash(&mut h);
        let mut reveal: Vec<_> = self.reveal.iter().collect();
        reveal.sort();
        reveal.hash(&mut h);
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
            self.hold = None;
            return;
        }
        let mut s = std::collections::hash_map::DefaultHasher::new();
        src.export.map_sources().hash(&mut s);
        let source = s.finish();
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
        map.close_sites(&self.closed);
        for (from, to) in self.carry_gone(&self.map, &mut map) {
            self.rename(&from, &to);
        }
        // Another map, or one after none: show all of it, not the last
        // map's zoom and spot. Reload (the same files) and a sweep filling
        // the map in keep the view.
        if source != self.source || self.map.nodes.is_empty() {
            self.source = source;
            self.new_map();
        }
        self.map = map;
        self.built += 1;
        let scope = match src.export.mapnetwork() {
            // The blank map is a start for any site: its cards are no one
            // network.
            Some((p, _)) if crate::skills::is_blank(p) => notes::Scope::Blank,
            Some((p, _)) if crate::skills::example(p).is_some() => {
                notes::Scope::Example(crate::tools::export::file_name(p))
            }
            _ => notes::Scope::Live,
        };
        self.notes_rebuilt(scope, src.sweep.runs());
        self.relayout(ctx);
        self.hold_still();
    }

    /// Start a new map's view: fitted whole, nothing moved, nothing but the
    /// pinned cards selected.
    fn new_map(&mut self) {
        self.follow = true;
        self.aim = None;
        self.placed.clear();
        self.raised.clear();
        self.undo = None;
        self.history.clear();
        self.drop_notes();
        self.select(None);
    }

    fn relayout(&mut self, ctx: &egui::Context) {
        self.multi_site = self.map.sites().len() > 1;
        self.lay = layout::layout_in(&self.map, &|_| true, self.mode());
        self.lay.links = self.links;
        // Boxes moved by hand stay where they were moved to.
        for g in 0..self.lay.boxes.len() {
            if let Some(d) = self
                .lay
                .box_key(self.mode(), g)
                .and_then(|k| self.shifted.get(&k).copied())
            {
                self.lay.shift_box(g, d);
            }
        }
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
        self.raised.retain(|s| ids.contains(s.as_str()));
        // A pinned card inside a collapsed site keeps its pin for when the
        // site opens again.
        let map = &self.map;
        self.kept
            .retain(|s| ids.contains(s.as_str()) || map.tucked(s));
        self.slots
            .retain(|s, _| ids.contains(s.as_str()) || map.tucked(s));
        self.runs.keep_only(&ids);
        if self
            .note_edit
            .as_ref()
            .and_then(|e| e.ids.first())
            .is_some_and(|id| !ids.contains(id.as_str()))
        {
            self.note_edit = None;
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|s| !ids.contains(s.as_str()))
        {
            self.selected = self.also.pop();
        }
    }

    /// Every selected card: pinned ones, Ctrl+clicked ones, the one clicked last.
    /// One pass over the cards, however many are selected: a big map with
    /// every card selected asks for this several times a frame.
    fn selection(&self) -> Vec<usize> {
        let ids: Vec<&str> = self
            .kept
            .iter()
            .chain(&self.also)
            .chain(&self.selected)
            .map(String::as_str)
            .collect();
        if ids.is_empty() {
            return Vec::new();
        }
        let want: HashSet<&str> = ids.iter().copied().collect();
        let mut at: HashMap<&str, usize> = HashMap::new();
        for (i, n) in self.map.nodes.iter().enumerate() {
            if want.contains(n.id.as_str()) {
                at.entry(n.id.as_str()).or_insert(i);
            }
        }
        let mut seen = HashSet::new();
        ids.iter()
            .filter_map(|s| at.get(s).copied())
            .filter(|&i| seen.insert(i))
            .collect()
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
        let picked: HashSet<&String> = ids.iter().collect();
        self.slots.retain(|s, _| self.kept.contains(s));
        let mut free: Vec<String> = self
            .order
            .iter()
            .filter(|s| picked.contains(s) && !self.slots.contains_key(*s))
            .cloned()
            .collect();
        // Selection runs oldest to newest, so the newest lands on top.
        let mut listed: HashSet<String> = free.iter().cloned().collect();
        let mut new: Vec<String> = ids
            .iter()
            .filter(|id| !self.slots.contains_key(*id) && listed.insert((*id).clone()))
            .cloned()
            .collect();
        new.reverse();
        new.append(&mut free);
        let mut free = new;
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
    /// For the screenshot tour: Zones, with the access points' box selected
    /// (its title clicked), or the box holding the most cards on a map
    /// without one. False until the layout is in.
    pub fn zones_for_shots(&mut self) -> bool {
        if self.mode != Some(Mode::Zones) {
            self.mode = Some(Mode::Zones);
            self.stale = true;
            return false;
        }
        if self.stale || self.lay.boxes.is_empty() {
            return false;
        }
        let aps =
            (0..self.lay.boxes.len()).find(|&g| self.lay.boxes[g].zone == Some(Zone::Wireless));
        let pick =
            aps.or_else(|| (0..self.lay.boxes.len()).max_by_key(|&g| self.lay.in_box(g).len()));
        if let Some(g) = pick
            && self.selection().len() != self.lay.in_box(g).len()
        {
            self.select(None);
            self.select_box(g, false);
        }
        true
    }

    /// For the screenshot tour: the map's own layout again, nothing selected.
    pub fn default_layout_for_shots(&mut self) {
        self.mode = None;
        self.select(None);
        self.stale = true;
    }

    /// For the screenshot tour: select the card most findings name, so the
    /// panel has something in every section.
    /// `IS_ALIVE_SHOTS_SELECT=<card id>` picks the card instead.
    pub fn select_for_shots(&mut self) {
        let named = std::env::var("IS_ALIVE_SHOTS_SELECT")
            .ok()
            .and_then(|id| self.index(&id));
        let best = named.or_else(|| {
            (0..self.map.nodes.len()).max_by_key(|&i| {
                let named = self
                    .map
                    .findings
                    .iter()
                    .filter(|f| f.nodes.contains(&i))
                    .count();
                (named + self.map.nodes[i].flags.len(), usize::MAX - i)
            })
        });
        if best.is_some() && self.selection().is_empty() {
            self.select(best);
        }
        // The tour shows a card's note: on the card, in its panel.
        if let Some(i) = best
            && std::env::var_os("IS_ALIVE_SHOTS").is_some()
            && self.notes.is_empty()
        {
            let id = self.map.nodes[i].id.clone();
            self.notes
                .insert(id, "Spare PSU on the shelf above the rack.\nAsk Dana before rebooting it: it runs the phones.".into());
            self.settle_edits();
        }
    }

    /// A plain click: this card alone (pinned ones stay), lit with its
    /// whole path to the internet. None lets go of all but the pinned.
    fn select(&mut self, i: Option<usize>) {
        self.selected = i.map(|i| self.map.nodes[i].id.clone());
        self.also.clear();
        // Pinned cards left on their own weren't clicked: they light only
        // what joins them, and the panel offers Trace to WAN.
        self.traced = i.is_some();
    }

    /// Select card `i` with `others` (the right-click menu's "Select
    /// children" and the like, a site, a finding): `i` is shown first and
    /// the rest join it, in their order, under it in the panel. Only what
    /// joins them lights. Cards a folded site hides are left out.
    fn select_group(&mut self, i: usize, others: &[usize]) {
        self.select(Some(i));
        self.traced = false;
        let mut added = vec![self.map.nodes[i].id.clone()];
        for &c in others {
            let shown = self.lay.pos.get(c).is_some_and(Option::is_some);
            let id = &self.map.nodes[c].id;
            if c != i && shown && !added.contains(id) {
                added.push(id.clone());
                if !self.kept.contains(id) {
                    self.also.push(id.clone());
                }
            }
        }
        // The panel: this card on top, the ones it brought under it.
        self.order.retain(|s| !added.contains(s));
        self.order.splice(0..0, added);
    }

    /// What the right-click menu on card `i` acts on: the whole selection
    /// when `i` is one of several selected cards, else `i` alone.
    fn menu_group(&self, i: usize) -> Vec<usize> {
        let sel = self.selection();
        if sel.len() > 1 && sel.contains(&i) {
            // The card right-clicked first, the rest as selected.
            let mut g = vec![i];
            g.extend(sel.into_iter().filter(|&k| k != i));
            g
        } else {
            vec![i]
        }
    }

    /// The cards a right-click menu item adds to `group` (each card's
    /// uplink, path, children or everything below), nearest first.
    fn grow(&self, group: &[usize], more: Grow) -> Vec<usize> {
        let mut out: Vec<usize> = group.to_vec();
        let mut seen: HashSet<usize> = group.iter().copied().collect();
        let kids = match more {
            Grow::Children | Grow::Below => self.map.children(),
            Grow::Uplink | Grow::Path => Vec::new(),
        };
        for &g in group {
            let add: Vec<usize> = match more {
                Grow::Uplink => self.map.nodes[g].parent().into_iter().collect(),
                Grow::Path => self.map.path(g).into_iter().rev().collect(),
                Grow::Children => kids[g].clone(),
                Grow::Below => below_in(&kids, g),
            };
            out.extend(add.into_iter().filter(|&k| seen.insert(k)));
        }
        out
    }

    /// What a selection lights: (cards, links by their lower card). A card
    /// picked on its own (a plain click): its whole path to the internet
    /// and its children's links. One card picked otherwise: its uplink and
    /// its children's links. More: only the part of the tree that joins
    /// them, so nothing past the topmost selected card lights up. With
    /// Trace to WAN, each card's path to the internet too.
    fn lit(&self, sel: &[usize]) -> (HashSet<usize>, HashSet<usize>) {
        let n = self.map.nodes.len();
        let mut cards: HashSet<usize> = sel.iter().copied().filter(|&i| i < n).collect();
        let mut links = HashSet::new();
        if cards.is_empty() {
            return (cards, links);
        }
        let parent = |i: usize| self.map.nodes[i].parent();
        let shown = self.tracing();
        let traced: Vec<usize> = if self.trace_wan {
            sel.to_vec()
        } else {
            shown.into_iter().collect()
        };
        for s in traced {
            let path = self.map.path(s);
            links.extend(path.iter().skip(1).copied());
            cards.extend(path);
        }
        if let Some(s) = shown {
            for (k, node) in self.map.nodes.iter().enumerate() {
                if node.parent() == Some(s) {
                    cards.insert(k);
                    links.insert(k);
                }
            }
        }
        if sel.len() == 1 {
            let s = sel[0];
            if let Some(u) = parent(s) {
                cards.insert(u);
                links.insert(s);
            }
            for (k, node) in self.map.nodes.iter().enumerate() {
                if node.parent() == Some(s) {
                    cards.insert(k);
                    links.insert(k);
                }
            }
            return (cards, links);
        }
        // How many selected cards each card's branch holds, and how many
        // its whole tree does: a link joins selected cards when the branch
        // under it holds some of its tree's selection, but not all.
        let mut under = vec![0usize; n];
        let mut root = vec![0usize; n];
        for &s in sel {
            let path = self.map.path(s);
            for &k in &path {
                under[k] += 1;
            }
            root[path[0]] += 1;
        }
        for (k, &held) in under.iter().enumerate() {
            if held == 0 || parent(k).is_none() {
                continue;
            }
            let top = self.map.path(k)[0];
            if held < root[top] {
                links.insert(k);
                cards.insert(k);
                cards.extend(parent(k));
            }
        }
        (cards, links)
    }

    /// Ctrl+click: add the card to the selection, or take it out (pinned too).
    /// Adding one makes a selection picked together: only what joins the
    /// cards lights from then on. Taking one out leaves a card clicked on
    /// its own lit as it was, its path to the internet and all, unless it's
    /// the one taken out.
    fn toggle(&mut self, i: usize) {
        let was = self.selected.clone();
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
        if self.selected != was {
            self.traced = false;
        }
    }

    /// The card lit with its whole path to the internet: the one shown,
    /// when it was picked on its own.
    fn tracing(&self) -> Option<usize> {
        self.selected
            .as_deref()
            .and_then(|s| self.index(s))
            .filter(|_| self.traced)
    }

    /// The selection lights only what joins its cards, so Trace to WAN has
    /// paths to add (a card clicked on its own shows its path already).
    fn joins_only(&self) -> bool {
        self.selection().len() > 1 || self.tracing().is_none()
    }

    /// Re-layout: every dragged card and box back where the layout puts it.
    fn relayout_all(&mut self) {
        self.placed.clear();
        if !self.shifted.is_empty() {
            self.shifted.clear();
            self.stale = true;
        }
        self.relaid = true;
    }

    /// A card or a box is being dragged.
    fn dragging(&self) -> bool {
        self.drag.is_some() || self.box_drag.is_some()
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
            || n.note.to_lowercase().contains(&q)
    }

    /// Bounds of the laid-out map, with cards dragged elsewhere.
    fn bounds(&self) -> Rect {
        let mut r = Rect::NOTHING;
        for i in 0..self.map.nodes.len() {
            if let Some(p) = self.target(i) {
                r = r.union(layout::card_rect(&self.map.nodes[i], p));
            }
        }
        for b in &self.lay.boxes {
            r = r.union(b.rect);
        }
        r
    }

    fn fit(&mut self, view: Rect) {
        self.fit_to(view, self.bounds());
    }

    /// Fit asked for (the Fit button, a double-click on the background):
    /// the whole map in the view as it is now, panel or not, and kept in
    /// view as it grows.
    fn fit_view(&mut self, view: Rect) {
        let view = self.settled(view);
        self.follow = true;
        self.wide = Some(view);
        self.fit(view);
    }

    /// Select the cards a finding names, lighting what joins them, and
    /// bring them into view (one card: centred, close enough to read).
    fn show_cards(&mut self, view: Rect, cards: &[usize]) {
        let cards: Vec<usize> = cards
            .iter()
            .copied()
            .filter(|&i| i < self.map.nodes.len())
            .collect();
        match cards.as_slice() {
            [] => {}
            [one] => {
                self.select(Some(*one));
                self.focus(view, *one);
            }
            [first, ..] => {
                self.select_group(*first, &cards);
                let area = cards
                    .iter()
                    .filter_map(|&i| Some(layout::card_rect(&self.map.nodes[i], self.target(i)?)))
                    .fold(Rect::NOTHING, |r, c| r.union(c));
                self.fit_to(self.settled(view), area);
                self.follow = false;
            }
        }
    }

    /// The canvas `rect` as it will be once the detail panel has slid open
    /// (anything selected) or shut: a fit asked for now frames that, not
    /// the canvas mid-slide, nor the canvas before a selection made this
    /// frame opens the panel.
    fn settled(&self, rect: Rect) -> Rect {
        let end = if self.selection().is_empty() {
            0.0
        } else {
            self.panel_full
        };
        let mut r = rect;
        r.max.x = (r.max.x + self.panel_now - end).max(r.min.x + 100.0);
        r
    }

    /// Ease the view so the middle of the map sits in the middle of `view`,
    /// keeping the zoom.
    fn center(&mut self, view: Rect) {
        let b = self.bounds();
        if !b.is_positive() {
            return;
        }
        let zoom = self.aim.map_or(self.zoom, |a| a.1);
        self.aim = Some((view.size() / 2.0 - b.center().to_vec2() * zoom, zoom));
        self.follow = false;
    }

    /// Ease the view to show the world rect `b`, clear of the zoom
    /// controls along the bottom and the legend.
    fn fit_to(&mut self, view: Rect, b: Rect) {
        if !b.is_positive() {
            return;
        }
        let zoom_in = |room: Rect| {
            (room.width() / b.width())
                .min(room.height() / b.height())
                .clamp(MIN_ZOOM, 1.0)
        };
        let room = fit_room(view, self.legend_size, &zoom_in);
        let zoom = zoom_in(room);
        let pan = room.center() - view.min - b.center().to_vec2() * zoom;
        self.aim = Some((pan, zoom));
    }

    fn focus(&mut self, view: Rect, i: usize) {
        let view = self.settled(view);
        if let Some(p) = self.target(i) {
            let zoom = self.zoom.max(0.85);
            let c = layout::card_rect(&self.map.nodes[i], p).center();
            self.aim = Some((view.size() / 2.0 - c.to_vec2() * zoom, zoom));
            self.follow = false;
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, src: Sources) -> Option<Action> {
        let ctx = ui.ctx().clone();
        self.animate = src.animate;
        if self.legend_seen != Some(src.legend) {
            self.legend_seen = Some(src.legend);
            self.legend = src.legend;
        }
        let mode = self.mode;
        // A drag whose button came up while another tab showed never saw
        // its end: it's over, and its move a step (or undo would wait for
        // it forever).
        let lost = ctx.dragged_id().is_none() && ctx.drag_stopped_id().is_none();
        if self.dragging() && lost && !ctx.input(|i| i.pointer.any_down()) {
            self.drag = None;
            self.box_drag = None;
            self.track_edits();
        }
        self.history_keys(&ctx);
        self.refresh(&ctx, &src);
        self.refresh_since(&src, ctx.input(|i| i.time));
        self.settle_edits();
        self.stamp_notes();
        self.busy = src.busy;
        let empty = self.map.nodes.is_empty();
        let bar = self.toolbar(ui, &src, empty);
        // The first thing asked for wins: a toolbar click isn't lost to
        // the accuracy note drawn after it.
        let mut act = bar.act;
        act = act.or(self.notices(ui, &src, empty));
        if empty {
            ui.separator();
            self.track_edits();
            self.sync_notes();
            return act.or(empty_state(ui, src.info));
        }
        // A narrow window starts with the sidebar shut: the map needs the
        // room more (the › button and the legend stand in for it).
        if !self.sized {
            self.sized = true;
            self.side &= ui.available_width() >= 900.0;
        }
        let mut focus = Focus {
            search: bar.search_enter,
            ..Default::default()
        };
        self.findings_band(ui, &mut focus.on);
        act = act.or(self.detail_panel(ui, &src, &mut focus.pick));
        // Left: roles and action items, as /mapnetwork's sidebar has them.
        self.sidebar(ui, &mut focus.on, &mut focus.site);
        self.canvas(ui, bar.fit, focus);
        // Esc next frame closes a menu or leaves a text box: not the map's.
        self.keys_busy = ctx.text_edit_focused() || egui::Popup::is_any_open(&ctx);
        self.track_edits();
        self.sync_notes();
        self.stamp_notes();
        // A card menu's ask (Open in a tab) when nothing else was asked.
        let act = act.or(self.menu_act.take());
        // An undone or redone layout is the one remembered, as if picked.
        let act = match self.mode {
            Some(m) if self.mode != mode && act.is_none() => Some(Action::Layout(m)),
            _ => act,
        };
        // The legend shown or hidden: saved (a frame later if something
        // else was asked for this one).
        if self.legend != src.legend && act.is_none() {
            return Some(Action::Legend(self.legend));
        }
        act
    }

    /// Search, clients, scan, fit and layout, and Export. Its groups go on
    /// as few rows as the window has room for, in order, each on the first
    /// row it fits on (Examples and Export at a row's right end), so none
    /// is ever cut off at the window's edge.
    fn toolbar(&mut self, ui: &mut Ui, src: &Sources, empty: bool) -> Toolbar {
        let ctx = ui.ctx().clone();
        let mut bar = Toolbar {
            act: None,
            search_enter: false,
            fit: false,
        };
        let full = ui.available_width();
        for row in bar_rows(&self.bar_w, full) {
            ui.horizontal(|ui| {
                // A row never takes more than the window: the right end
                // stays on screen.
                ui.set_max_width(full);
                for (n, &k) in row.iter().enumerate() {
                    if k == BAR_END {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let r = ui.scope(|ui| {
                                bar.act = bar.act.take().or(self.toolbar_end(ui, src, empty));
                            });
                            self.bar_w[k] = r.response.rect.width();
                        });
                        continue;
                    }
                    if k == BAR_LAYOUTS && n > 0 {
                        ui.separator();
                    }
                    let r = ui.scope_builder(
                        egui::UiBuilder::new()
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| self.bar_group(ui, &ctx, k, src, empty, &mut bar),
                    );
                    self.bar_w[k] = r.response.rect.width();
                }
            });
        }
        bar
    }

    /// The toolbar's group `k` (see `bar_rows`).
    fn bar_group(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        k: usize,
        src: &Sources,
        empty: bool,
        bar: &mut Toolbar,
    ) {
        match k {
            0 => self.bar_find(ui, src, empty, bar),
            1 => self.bar_edit(ui, empty, bar),
            2 => self.history_buttons(ui),
            BAR_LAYOUTS => {
                if let Some(m) = self.layout_buttons(ui, ctx, empty) {
                    bar.fit = true;
                    bar.act = Some(Action::Layout(m));
                }
            }
            _ => {}
        }
    }

    /// Search, Clients and Scan.
    fn bar_find(&mut self, ui: &mut Ui, src: &Sources, empty: bool, bar: &mut Toolbar) {
        // Narrower with text in it: room for the count and ×.
        let width = if self.search.is_empty() { 220.0 } else { 140.0 };
        let r = widgets::line(
            ui,
            crate::input::Field::Text,
            &mut self.search,
            width,
            "Search name, IP, MAC, note",
        )
        .on_hover_text("Finds cards by name, IP, MAC, vendor, details or note");
        bar.search_enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if !self.search.is_empty() {
            let n = self.map.nodes.iter().filter(|n| self.matches(n)).count();
            ui.weak(format!("{n} match{}", if n == 1 { "" } else { "es" }));
            if ui.small_button("×").on_hover_text("Clear search").clicked() {
                self.search.clear();
            }
        }
        ui.separator();
        // A map with no end devices at all (an infrastructure-only
        // /mapnetwork map) has nothing for the switch to show or hide.
        let any = empty || !self.map.under.is_empty() || self.map.nodes.iter().any(|n| n.client);
        ui.add_enabled_ui(any && self.frozen.is_none(), |ui| {
            egui::ComboBox::from_id_salt("map-client-visibility")
                .selected_text(format!("Clients: {}", self.clients.name()))
                .show_ui(ui, |ui| {
                    for c in Clients::ALL {
                        if ui
                            .selectable_label(self.clients == c, c.name())
                            .on_hover_text(c.tip())
                            .on_disabled_hover_text(if self.frozen.is_some() {
                                "A saved map keeps the clients it was saved with"
                            } else {
                                reveal::no_clients_why(src)
                            })
                            .clicked()
                        {
                            self.clients = c;
                            if !cfg!(test)
                                && std::env::var_os("IS_ALIVE_SHOTS").is_none()
                                && let Some(dir) = crate::theme::data_dir()
                            {
                                let result = saved::save_live_clients(&dir, c);
                                if let Err(e) = result {
                                    widgets::toast(
                                        format!("Could not save client visibility: {e}"),
                                        false,
                                    );
                                }
                            }
                            // The level sets every uplink again.
                            self.reveal.clear();
                        }
                    }
                });
        });
        if empty {
            return;
        }
        ui.separator();
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
        if ui
            .add_enabled(
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
        {
            bar.act = Some(Action::Sweep);
        }
    }

    /// Re-layout and Reload.
    fn bar_edit(&mut self, ui: &mut Ui, empty: bool, bar: &mut Toolbar) {
        ui.add_enabled_ui(!empty, |ui| {
            // Always there, so the buttons after it never move; it
            // has something to do once a card was dragged.
            let moved = !self.placed.is_empty() || !self.shifted.is_empty();
            if ui
                .add_enabled(moved, egui::Button::new("Re-layout"))
                .on_hover_text("Put dragged cards and boxes back where the layout had them")
                .on_disabled_hover_text(if empty {
                    NOTHING_YET
                } else {
                    "Puts dragged cards and boxes back where the layout had them: nothing \
                     was dragged"
                })
                .clicked()
            {
                self.relayout_all();
                bar.fit = true;
            }
            if ui
                .add_enabled(self.frozen.is_none(), egui::Button::new("⟲ Reload"))
                .on_hover_text(
                    "Read the map's files again and rebuild it, keeping pins and the \
                     selection. (It already updates by itself as a sweep finds hosts or a \
                     file is loaded.)",
                )
                .on_disabled_hover_text(if empty {
                    NOTHING_YET
                } else {
                    "A saved map doesn't change with the files: Live has the map they make"
                })
                .clicked()
            {
                bar.act = Some(Action::Reload);
            }
        });
    }

    /// Sites, Cluster, Tree, Zones; the one picked, if it changed.
    fn layout_buttons(&mut self, ui: &mut Ui, ctx: &egui::Context, empty: bool) -> Option<Mode> {
        let mut picked = None;
        ui.add_enabled_ui(!empty, |ui| {
            let now = self.mode();
            for m in Mode::ALL {
                if ui
                    .selectable_label(now == m, m.name())
                    .on_hover_text(m.tip())
                    .on_disabled_hover_text(NOTHING_YET)
                    .clicked()
                    && now != m
                {
                    self.mode = Some(m);
                    self.placed.clear();
                    self.relayout(ctx);
                    picked = Some(m);
                }
            }
            // The legend's switch sits with the other ways to show the map.
            ui.separator();
            if ui
                .selectable_label(self.legend, "Legend")
                .on_hover_text("Show what the colours, lines and badges mean (L)")
                .on_disabled_hover_text(NOTHING_YET)
                .clicked()
            {
                self.legend = !self.legend;
            }
        });
        picked
    }

    /// The toolbar's right end, drawn right to left: Export, Examples.
    fn toolbar_end(&self, ui: &mut Ui, src: &Sources, empty: bool) -> Option<Action> {
        self.export_button(ui, empty);
        examples_menu(ui, src)
    }

    /// Under the toolbar: the map's summary, the accuracy note, and how far
    /// a running sweep has got.
    fn notices(&mut self, ui: &mut Ui, src: &Sources, empty: bool) -> Option<Action> {
        let mut act = None;
        // A plan or a training map says so first: it isn't what was found.
        if let Some(c) = self.purpose.caption() {
            ui.label(RichText::new(c).strong().color(widgets::warn()));
        }
        if !self.map.summary.is_empty() {
            ui.weak(&self.map.summary);
        }
        if !empty && src.note && src.export.mapnetwork().is_none() && self.frozen.is_none() {
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
        act
    }

    /// The card panel on the right (it keeps the last card while it slides
    /// shut), and what was done in it. `pick` is a card it asked to show.
    fn detail_panel(
        &mut self,
        ui: &mut Ui,
        src: &Sources,
        pick: &mut Option<(usize, bool)>,
    ) -> Option<Action> {
        let mut act = None;
        if self.selected.is_some() {
            self.shown.clone_from(&self.selected);
        }
        let mut open = !self.selection().is_empty();
        let mut ev: Option<PanelEv> = None;
        let mut menus = Vec::new();
        let stack = self.ordered();
        let right = ui.available_rect_before_wrap().right();
        let mut edit = self.note_edit.take();
        egui::Panel::right(PANEL)
            .resizable(true)
            .default_size(PANEL_SIZE)
            .size_range(260.0..=520.0)
            .show_collapsible(ui, &mut open, |ui| {
                act = self.detail(ui, stack, src.busy, pick, &mut ev, &mut menus, &mut edit);
            });
        self.note_edit = edit;
        self.card_menus(menus);
        self.panel_now = right - ui.available_rect_before_wrap().right();
        self.panel_full = egui::PanelState::load(ui.ctx(), egui::Id::new(PANEL))
            .map_or(PANEL_SIZE, |p| p.size().x);
        if !open {
            self.select(None);
        }
        match ev {
            Some(PanelEv::Drop(i)) => self.toggle(i),
            Some(PanelEv::TraceWan(on)) => self.trace_wan = on,
            Some(PanelEv::Keep(i)) => self.keep(i),
            Some(PanelEv::Move(from, to)) => self.move_card(from, to),
            Some(PanelEv::Run(i, tool)) => self.run_card(ui.ctx(), i, tool),
            Some(PanelEv::Stop(i, tool)) => self.runs.stop(&self.map.nodes[i].id, tool),
            Some(PanelEv::Clear(i, tool)) => self.runs.clear(&self.map.nodes[i].id, tool),
            Some(PanelEv::Reveal(p, n)) => self.reveal_step(p, n),
            Some(PanelEv::Note(e)) => self.note_event(e),
            _ => {}
        }
        act
    }

    /// The map itself: fit and focus, input, animation and painting, and
    /// the controls over it.
    fn canvas(&mut self, ui: &mut Ui, fit: bool, focus: Focus) {
        let ctx = ui.ctx().clone();
        let Focus {
            on: focus_on,
            site: focus_site,
            pick,
            search: search_enter,
        } = focus;
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        // L shows or hides the legend, unless a text box or menu has the keys.
        if !self.keys_busy
            && !ctx.text_edit_focused()
            && ui.input(|i| i.modifiers.is_none() && i.key_pressed(egui::Key::L))
        {
            self.legend = !self.legend;
        }
        // N writes a note on the one card selected.
        if !self.keys_busy
            && !ctx.text_edit_focused()
            && ui.input(|i| i.modifiers.is_none() && i.key_pressed(egui::Key::N))
            && let [i] = self.selection()[..]
        {
            self.open_note(&[i]);
        }
        if self.zoom == 0.0 {
            self.zoom = 1.0;
            self.fit(rect);
            if let Some((p, z)) = self.aim.take() {
                (self.pan, self.zoom) = (p, z);
            }
        }
        // While cards are selected the panel takes the right of the view;
        // fitting to what's left would zoom the whole map out as it opens.
        // So the map keeps the fit it had without the panel (the panel lies
        // over its right edge), panned only to keep the selected card clear.
        let selected = self.selection().first().copied();
        if selected.is_none() || self.wide.is_none_or(|w| w.min != rect.min) {
            self.wide = Some(rect);
        }
        let wide = self.wide.map_or(rect, |w| w.union(rect));
        if fit {
            self.fit_view(rect);
        } else if self.follow && !self.dragging() {
            self.fit(wide);
            if let (Some(i), Some((pan, zoom))) = (selected, self.aim)
                && let Some(p) = self.target(i)
            {
                let right = rect.min.x + pan.x + (p.x + CARD.x) * zoom;
                let over = right - (rect.right() - 16.0);
                if over > 0.0 {
                    self.aim = Some((pan - vec2(over, 0.0), zoom));
                }
            }
        }
        if let Some(cards) = focus_on {
            self.show_cards(rect, &cards);
        }
        match pick {
            // Ctrl+click on a name in the panel adds that card.
            Some((i, true)) => {
                if !self.selection().contains(&i) {
                    self.toggle(i);
                }
                self.focus(rect, i);
            }
            Some((i, false)) => {
                self.select(Some(i));
                self.focus(rect, i);
            }
            None => {}
        }
        if search_enter && let Some(i) = self.map.nodes.iter().position(|n| self.matches(n)) {
            self.select(Some(i));
            self.focus(rect, i);
        }
        if let Some(site) = focus_site {
            self.show_site(rect, &site);
        }
        let hover = self.interact(ui, rect, &resp);
        for site in std::mem::take(&mut self.flip_sites) {
            self.flip(&site);
            ctx.request_repaint();
        }
        self.animate(&ctx);
        let lines = self.lines_now();
        self.paint(ui, rect, hover, &lines);
        self.reveal_chips(ui, rect);
        // With the sidebar shut, a button that opens it again (the legend
        // keeps the roles' fading).
        if !self.side
            && ui
                .put(
                    Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(24.0, 24.0)),
                    egui::Button::new("›").corner_radius(6),
                )
                .on_hover_text("Show the sites, roles and action items")
                .clicked()
        {
            self.side = true;
        }
        if self.legend {
            let entries = self.legend_entries(&lines);
            self.legend_panel(ui, rect, &entries);
        } else {
            self.legend_size = Vec2::ZERO;
        }
        self.zoom_controls(ui, rect);
        self.links_pill(ui, rect);
        self.bar_toggle(ui, rect);
    }
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

/// Every card below `i` in the child table `kids`, nearest first.
fn below_in(kids: &[Vec<usize>], i: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut next = vec![i];
    let mut seen = vec![false; kids.len()];
    seen[i] = true;
    while !next.is_empty() {
        let mut level = Vec::new();
        for p in next {
            for &c in &kids[p] {
                if !seen[c] {
                    seen[c] = true;
                    level.push(c);
                }
            }
        }
        out.extend(&level);
        next = level;
    }
    out
}

/// Where Fit puts the map in `view`: a margin all round, more at the
/// bottom, where the zoom controls sit, and clear of a `legend` that size
/// in the bottom left corner: above it or beside it, whichever lets the
/// map be drawn bigger (`zoom` says how big in a room).
fn fit_room(view: Rect, legend: Vec2, zoom: &dyn Fn(Rect) -> f32) -> Rect {
    let room = Rect::from_min_max(view.min + vec2(30.0, 24.0), view.max - vec2(30.0, 44.0));
    if legend == Vec2::ZERO {
        return room;
    }
    let mut above = room;
    above.max.y = above.max.y.min(view.max.y - 10.0 - legend.y - 12.0);
    let mut beside = room;
    beside.min.x = beside.min.x.max(view.min.x + 10.0 + legend.x + 12.0);
    let usable = |r: Rect| r.width() > 60.0 && r.height() > 60.0;
    match (usable(above), usable(beside)) {
        (true, true) if zoom(beside) > zoom(above) => beside,
        (true, _) => above,
        (false, true) => beside,
        (false, false) => room,
    }
}

/// A small filled chevron: pointing down when `open`, right when not.
fn chevron(p: &egui::Painter, c: Pos2, r: f32, open: bool, colour: Color32) {
    let pts = if open {
        vec![
            c + vec2(-r, -r * 0.5),
            c + vec2(r, -r * 0.5),
            c + vec2(0.0, r * 0.6),
        ]
    } else {
        vec![
            c + vec2(-r * 0.5, -r),
            c + vec2(r * 0.6, 0.0),
            c + vec2(-r * 0.5, r),
        ]
    };
    p.add(Shape::convex_polygon(pts, colour, Stroke::NONE));
}

/// Points `period` apart along a polyline at `phase` (and every `period`
/// on) from its start, none before `from` and none outside `clip`: a lit
/// line's packets, from where they're its own (see `paint`).
fn along_from(pts: &[Pos2], from: f32, phase: f32, period: f32, clip: Rect) -> Vec<Pos2> {
    let mut out = Vec::new();
    if period <= 0.0 {
        return out;
    }
    let mut next = phase + ((from - phase) / period).ceil() * period;
    let mut walked = 0.0;
    for seg in pts.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let len = a.distance(b);
        while len > 0.0 && next <= walked + len {
            let at = a + (b - a) * ((next - walked) / len);
            if clip.contains(at) {
                out.push(at);
            }
            next += period;
        }
        walked += len;
    }
    out
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

/// Before anything is mapped: what the map is built from, the first
/// steps, and the example maps.
fn empty_state(ui: &mut Ui, info: &HostInfo) -> Option<Action> {
    let subnet = info
        .lan
        .map(|(ip, bits)| crate::net::targets::subnet_cidr(ip, bits));
    let scan_tip = match &subnet {
        Some(s) => format!("Sweep {s} and watch the map fill in"),
        None => "No LAN address on this machine".to_string(),
    };
    let hit = widgets::empty_state(
        ui,
        "Nothing to map yet",
        "Built from a Subnet Sweep, a UniFi or SonicWall backup, or a /mapnetwork map. A backup \
         gives the real uplinks; a sweep alone can only place devices under the gateway.",
        &[
            ("Scan this network", &scan_tip, subnet.is_some()),
            (
                "Open a backup or map…",
                "Opens Config Export (or drop the file on the window)",
                true,
            ),
        ],
    );
    let mut act = match hit {
        Some(0) => Some(Action::Sweep),
        Some(1) => Some(Action::Export),
        _ => None,
    };
    ui.vertical_centered(|ui| {
        ui.add_space(14.0);
        ui.weak("Or look around one of the example maps (fictional sites):");
        ui.add_space(4.0);
        for (i, (label, f)) in crate::skills::EXAMPLES.iter().enumerate() {
            let blank = *f == crate::skills::BLANK;
            if blank {
                ui.add_space(6.0);
                ui.weak("Or start from a blank one:");
                ui.add_space(4.0);
            }
            if ui.link(*label).on_hover_text(example_tip(blank)).clicked() {
                act = Some(Action::Example(i));
            }
        }
    });
    act
}

/// What an example in the Examples menu (or the empty map's list) is.
fn example_tip(blank: bool) -> &'static str {
    if blank {
        "The Internet and one gateway at one site, nothing else: a start for a map drawn by \
         hand, or a small map to try things on. Saved, it's marked a plan"
    } else {
        "Opens it like a /mapnetwork map loaded in Config Export"
    }
}

/// The toolbar's Examples menu; the example picked, if any.
fn examples_menu(ui: &mut Ui, src: &Sources) -> Option<Action> {
    let mut picked = None;
    // The example on the map now, ticked; picking it again reads it again.
    let shown = src
        .export
        .mapnetwork()
        .and_then(|m| m.0.file_name().map(|f| f.to_os_string()));
    ui.menu_button("Examples ⏷", |ui| {
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
    use crate::map::Node;

    /// The toolbar wraps group by group, so nothing is ever cut off: one
    /// row on a wide window, the layouts with Examples and Export under
    /// the rest at the default size, and three rows at the narrowest.
    #[test]
    fn the_toolbar_wraps_rather_than_cut_anything_off() {
        let w = [540.0, 150.0, 50.0, 300.0, 160.0];
        assert_eq!(bar_rows(&w, 1400.0), [vec![0, 1, 2, 3, 4]]);
        assert_eq!(bar_rows(&w, 1100.0), [vec![0, 1, 2], vec![3, 4]]);
        assert_eq!(bar_rows(&w, 740.0), [vec![0, 1], vec![2, 3, 4]]);
        assert_eq!(bar_rows(&w, 600.0), [vec![0], vec![1, 2, 3], vec![4]]);
        // A row's groups never add up to more than the window (one alone
        // can't be helped).
        for full in (560..1500).step_by(20) {
            for row in bar_rows(&w, full as f32) {
                let need: f32 = row.iter().map(|&k| w[k] + BAR_GAP).sum();
                assert!(row.len() == 1 || need <= full as f32, "{full}: {row:?}");
            }
        }
    }

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
    fn a_selection_lights_only_what_joins_it() {
        // 0 internet - 1 gateway - 2 core - {3 access - {5 ap - 7, 6 ap}, 4 server}
        let mut v = view(8);
        for (c, p) in [(1, 0), (2, 1), (3, 2), (4, 2), (5, 3), (6, 3), (7, 5)] {
            v.map.nodes[c].up = Some((p, crate::map::Link::Wired, None));
        }
        let set = |v: &[usize]| v.iter().copied().collect::<HashSet<usize>>();
        // An AP and its switch: that one link, nothing above the switch.
        let (cards, links) = v.lit(&[5, 3]);
        assert_eq!(links, set(&[5]));
        assert_eq!(cards, set(&[5, 3]));
        // Two APs: the links through their switch, still nothing above it.
        let (cards, links) = v.lit(&[7, 6]);
        assert_eq!(links, set(&[7, 5, 6]));
        assert_eq!(cards, set(&[7, 5, 6, 3]));
        // One card picked with Ctrl+click: its uplink and its children.
        v.toggle(3);
        let (_, links) = v.lit(&v.selection());
        assert_eq!(links, set(&[3, 5, 6]));
        // Trace to WAN adds each card's path to the internet.
        v.trace_wan = true;
        let (cards, links) = v.lit(&[5, 3]);
        assert_eq!(links, set(&[1, 2, 3, 5]));
        assert!(cards.contains(&0));
    }

    /// 0 internet - 1 gateway - 2 core - {3 access - {5 ap - 7, 6 ap}, 4 server}
    fn tree8() -> MapView {
        let mut v = view(8);
        for (c, p) in [(1, 0), (2, 1), (3, 2), (4, 2), (5, 3), (6, 3), (7, 5)] {
            v.map.nodes[c].up = Some((p, crate::map::Link::Wired, None));
        }
        v.lay = layout::layout_in(&v.map, &|_| true, Mode::Cluster);
        v
    }

    fn set(v: &[usize]) -> HashSet<usize> {
        v.iter().copied().collect()
    }

    #[test]
    fn a_plain_click_lights_the_whole_path_to_the_internet() {
        let mut v = tree8();
        v.select(Some(3));
        let (cards, links) = v.lit(&v.selection());
        // Up to the internet, and down to its children.
        assert_eq!(links, set(&[1, 2, 3, 5, 6]));
        assert_eq!(cards, set(&[0, 1, 2, 3, 5, 6]));
        // The same from search, a finding naming one card, a panel link.
        v.select(Some(7));
        let (cards, _) = v.lit(&v.selection());
        assert!(cards.contains(&0) && cards.contains(&1));
        // A pinned card joins the clicked one; only the clicked one traces.
        v.keep(7);
        v.select(Some(4));
        let (cards, links) = v.lit(&v.selection());
        assert_eq!(links, set(&[1, 2, 3, 4, 5, 7]));
        assert!(cards.contains(&0));
    }

    #[test]
    fn taking_another_card_out_leaves_a_clicked_card_its_path() {
        let mut v = tree8();
        v.select(Some(4));
        v.keep(4);
        v.select(Some(7));
        // × on the pinned card (or Ctrl+click on it): 7 was clicked on its
        // own and still lights its way to the internet.
        v.toggle(4);
        assert_eq!(v.selection(), [7]);
        let (cards, _) = v.lit(&v.selection());
        assert!(cards.contains(&0));
        assert!(!v.joins_only(), "no Trace to WAN header for a clicked card");
        // Taking out the clicked card leaves the pinned one, not traced.
        v.select(Some(4));
        v.keep(4);
        v.select(Some(7));
        v.toggle(7);
        assert_eq!(v.selection(), [4]);
        let (cards, _) = v.lit(&v.selection());
        assert!(!cards.contains(&0));
        assert!(v.joins_only());
    }

    #[test]
    fn a_pinned_card_left_alone_offers_trace_to_wan() {
        let mut v = tree8();
        v.select(Some(7));
        v.keep(7);
        // A click on the background or Esc: the pin stays, untraced, and
        // the panel's header (with Trace to WAN) is there to bring it back.
        v.select(None);
        assert_eq!(v.selection(), [7]);
        assert!(v.joins_only());
        let (cards, _) = v.lit(&v.selection());
        assert!(!cards.contains(&0));
        v.trace_wan = true;
        let (cards, _) = v.lit(&v.selection());
        assert!(cards.contains(&0));
    }

    #[test]
    fn a_search_miss_on_the_lit_path_can_be_clicked() {
        let mut v = tree8();
        for (k, n) in v.map.nodes.iter_mut().enumerate() {
            n.label = format!("dev{k}");
        }
        for k in 0..8 {
            let p = v.lay.pos[k].unwrap();
            v.pos.insert(format!("n{k}"), p);
        }
        let mid = |v: &MapView, k: usize| v.lay.pos[k].unwrap() + CARD / 2.0;
        v.select(Some(7));
        v.search = "dev7".into();
        // 5 is on 7's path, drawn in sight: it can be picked.
        assert_eq!(v.hit(mid(&v, 5)), Some(5));
        // 4 is off the path, faded out: it can't.
        assert_eq!(v.hit(mid(&v, 4)), None);
    }

    #[test]
    fn fits_frame_the_canvas_the_panel_leaves() {
        let mut v = tree8();
        v.panel_full = 320.0;
        let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0));
        // Nothing selected, the panel shut: the whole canvas.
        assert_eq!(v.settled(full), full);
        // A finding selects cards this frame: the panel will open over the
        // right of the canvas.
        v.show_cards(full, &[5, 6]);
        assert_eq!(v.settled(full).right(), 680.0);
        // A double-click puts the selection back while the panel is half
        // shut: the fit is to the canvas with it open again.
        v.panel_now = 150.0;
        let mid = Rect::from_min_size(pos2(0.0, 0.0), vec2(850.0, 600.0));
        v.fit_view(mid);
        assert_eq!(v.wide.map(|w| w.right()), Some(680.0));
        // Sliding shut: the canvas it will have, the whole width.
        v.select(None);
        assert_eq!(v.settled(mid).right(), 1000.0);
    }

    #[test]
    fn a_dragged_card_pushes_the_map_only_near_the_edge() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        assert_eq!(interact::edge_push(r, pos2(400.0, 300.0)), Vec2::ZERO);
        assert_eq!(interact::edge_push(r, pos2(790.0, 300.0)).y, 0.0);
        assert!(interact::edge_push(r, pos2(790.0, 300.0)).x < 0.0);
        assert_eq!(interact::edge_push(r, pos2(810.0, 300.0)).x, -1.0);
        // Well past it (over the panel, out of the window): nothing.
        assert_eq!(interact::edge_push(r, pos2(880.0, 300.0)), Vec2::ZERO);
    }

    #[test]
    fn ctrl_clicking_two_cards_lights_only_what_joins_them() {
        let mut v = tree8();
        v.select(Some(7));
        v.toggle(6);
        let (cards, links) = v.lit(&v.selection());
        assert_eq!(links, set(&[7, 5, 6]));
        assert_eq!(cards, set(&[7, 5, 6, 3]));
        // Shift+click goes the same way (both call toggle); taking one out
        // leaves the other on its own, still not traced.
        v.toggle(6);
        let (_, links) = v.lit(&v.selection());
        assert_eq!(links, set(&[7]));
        // A plain click traces again.
        v.select(Some(6));
        let (cards, _) = v.lit(&v.selection());
        assert!(cards.contains(&0));
    }

    #[test]
    fn the_menu_on_one_of_several_selected_cards_acts_on_all_of_them() {
        let mut v = tree8();
        v.select(Some(7));
        v.toggle(4);
        // On a selected card: the whole selection, that card first.
        assert_eq!(v.menu_group(4), [4, 7]);
        assert_eq!(v.menu_group(7), [7, 4]);
        // On a card outside it: that card alone.
        assert_eq!(v.menu_group(6), [6]);
        // Uplinks: each card's, added to the selection, not replacing it.
        let g = v.menu_group(4);
        let up = v.grow(&g, Grow::Uplink);
        assert_eq!(up, [4, 7, 2, 5]);
        v.select_group(4, &up);
        assert_eq!(set(&v.selection()), set(&[4, 7, 2, 5]));
        assert_eq!(v.ordered()[0], 4, "the card right-clicked shows first");
        // Joined, not traced: nothing above the core lights.
        let (cards, _) = v.lit(&v.selection());
        assert!(!cards.contains(&1));
        // Paths, children and everything below: the union over each.
        let g = [4, 7];
        assert_eq!(set(&v.grow(&g, Grow::Path)), set(&[0, 1, 2, 3, 4, 5, 7]));
        assert_eq!(set(&v.grow(&[3, 4], Grow::Children)), set(&[3, 4, 5, 6]));
        assert_eq!(set(&v.grow(&[3, 4], Grow::Below)), set(&[3, 4, 5, 6, 7]));
        // Pinning from the menu pins every card it acts on.
        v.select(Some(7));
        v.toggle(4);
        for k in v.menu_group(4) {
            v.keep(k);
        }
        v.select(Some(1));
        assert_eq!(set(&v.selection()), set(&[1, 4, 7]));
    }

    #[test]
    fn select_group_with_uplinks_and_children() {
        // 0 internet - 1 gateway - 2 switch - {3 ap - 5 client, 4 server}
        let mut v = view(6);
        for (c, p) in [(1, 0), (2, 1), (3, 2), (4, 2), (5, 3)] {
            v.map.nodes[c].up = Some((p, crate::map::Link::Wired, None));
        }
        v.lay = layout::layout_in(&v.map, &|_| true, Mode::Cluster);
        assert_eq!(below_in(&v.map.children(), 2), [3, 4, 5]);
        assert_eq!(below_in(&v.map.children(), 5), Vec::<usize>::new());
        let path = v.map.path(5);
        v.select_group(5, &path);
        assert_eq!(v.selected.as_deref(), Some("n5"));
        assert_eq!(v.also, ["n0", "n1", "n2", "n3"]);
        assert_eq!(v.ordered(), [5, 0, 1, 2, 3]);
        v.select_group(2, &v.map.children()[2].clone());
        assert_eq!(v.selection().len(), 3);
        assert_eq!(v.ordered(), [2, 3, 4]);
        // A card the layout doesn't show (a folded site) stays out.
        v.lay.pos[4] = None;
        v.select_group(2, &below_in(&v.map.children(), 2));
        assert_eq!(v.also, ["n3", "n5"]);
    }

    #[test]
    fn a_double_click_puts_back_what_its_first_click_replaced() {
        let mut v = tree8();
        v.select(Some(7));
        v.toggle(6);
        v.ordered();
        let before = v.selection();
        // The first click of a double-click on 6 picked it alone.
        v.undo = Some(interact::Undo {
            at: 0.0,
            selected: v.selected.clone(),
            also: v.also.clone(),
            traced: v.traced,
            order: v.order.clone(),
        });
        v.select(Some(6));
        let u = v.undo.take().unwrap();
        v.restore(u);
        assert_eq!(v.selection(), before);
        assert!(!v.traced);
    }

    #[test]
    fn dragged_and_selected_cards_draw_and_pick_on_top() {
        let mut v = view(3);
        for i in 0..3 {
            v.pos.insert(format!("n{i}"), Pos2::ZERO);
        }
        // All three on one spot: the last drawn wins.
        assert_eq!(v.hit(pos2(5.0, 5.0)), Some(2));
        v.select(Some(0));
        assert_eq!(v.stack(), [1, 2, 0]);
        assert_eq!(v.hit(pos2(5.0, 5.0)), Some(0));
        v.raised = vec!["n1".into()];
        assert_eq!(v.stack(), [2, 0, 1]);
        assert_eq!(v.hit(pos2(5.0, 5.0)), Some(1));
        // Faded out by the search: not picked, unless it's selected.
        v.search = "nothing like it".into();
        assert_eq!(v.hit(pos2(5.0, 5.0)), Some(0));
        v.select(None);
        assert_eq!(v.hit(pos2(5.0, 5.0)), None);
    }

    #[test]
    fn fit_leaves_room_for_the_zoom_controls() {
        let mut v = view(1);
        let view = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        v.fit_to(
            view,
            Rect::from_min_size(pos2(0.0, 0.0), vec2(2000.0, 2000.0)),
        );
        let (pan, zoom) = v.aim.unwrap();
        (v.pan, v.zoom) = (pan, zoom);
        let bottom = v.to_screen(view, pos2(0.0, 2000.0)).y;
        assert!(bottom <= view.max.y - 44.0 + 0.01, "{bottom}");
    }

    #[test]
    fn center_puts_the_map_middle_mid_view_at_the_same_zoom() {
        let mut v = view(3);
        v.lay = layout::layout_in(&v.map, &|_| true, Mode::Cluster);
        v.zoom = 0.5;
        v.follow = true;
        let view = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0));
        v.center(view);
        let (pan, zoom) = v.aim.expect("eases there");
        assert_eq!(zoom, 0.5);
        assert!(!v.follow, "the user moved the view");
        // Once eased there, the map's middle lands on the view's middle.
        (v.pan, v.zoom) = (pan, zoom);
        let mid = v.to_screen(view, v.bounds().center());
        assert!((mid - view.center()).length() < 0.01, "{mid:?}");
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
    fn flow_dots_are_spaced_along_the_whole_route() {
        let pts = [pos2(0.0, 0.0), pos2(0.0, 30.0), pos2(40.0, 30.0)];
        let all = Rect::EVERYTHING;
        let d = along_from(&pts, 0.0, 5.0, 20.0, all);
        assert_eq!(
            d,
            [
                pos2(0.0, 5.0),
                pos2(0.0, 25.0),
                pos2(15.0, 30.0),
                pos2(35.0, 30.0)
            ]
        );
        assert!(along_from(&[pos2(0.0, 0.0)], 0.0, 0.0, 10.0, all).is_empty());
        // From partway along: the same phase (counted from the start), the
        // points before it left out, as are those off screen.
        assert_eq!(
            along_from(&pts, 20.0, 45.0, 20.0, all),
            [pos2(0.0, 25.0), pos2(15.0, 30.0), pos2(35.0, 30.0)]
        );
        let clip = Rect::from_min_max(pos2(-1.0, -1.0), pos2(20.0, 40.0));
        assert_eq!(
            along_from(&pts, 0.0, 5.0, 20.0, clip),
            [pos2(0.0, 5.0), pos2(0.0, 25.0), pos2(15.0, 30.0)]
        );
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

    /// What one frame of the map in a test showed: the text on the canvas's
    /// right-click menu (and its separators), on the open ☰ menu in the
    /// panel, and where each card's ☰ button is.
    #[derive(Default)]
    struct Shown {
        canvas: Vec<(String, Rect)>,
        panel: Vec<(String, Rect)>,
        buttons: HashMap<usize, Rect>,
        /// The text in view in the detail panel.
        detail: Vec<(String, Rect)>,
        /// Separators on the canvas's menu.
        canvas_lines: usize,
    }

    /// One frame as the tab draws it: the panel and its ☰ menus, then the
    /// canvas (cards where `v.pos` has them, at zoom 1).
    fn frame(ctx: &egui::Context, v: &mut MapView, events: Vec<egui::Event>) -> Shown {
        use egui::{LayerId, Order, Popup, UiBuilder};
        fn texts(s: &Shape, out: &mut Vec<(String, Rect)>) {
            match s {
                Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
                Shape::LineSegment { .. } => out.push(("—".into(), s.visual_bounding_rect())),
                Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let on = |ctx: &egui::Context, id: egui::Id| {
            let mut out = Vec::new();
            ctx.graphics(|g| {
                if let Some(l) = g.get(LayerId::new(Order::Foreground, id)) {
                    l.all_entries().for_each(|c| texts(&c.shape, &mut out));
                }
            });
            out
        };
        let mut shown = Shown::default();
        // A tenth of a second apart: a press and release are a click, and
        // `click` takes long enough that no two make a double-click.
        let input = egui::RawInput {
            events,
            time: Some(ctx.input(|i| i.time) + 0.1),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let (mut pick, mut ev, mut menus) = (None, None, Vec::new());
            let panel = Rect::from_min_size(pos2(820.0, 0.0), vec2(320.0, 900.0));
            let stack = v.ordered();
            let mut edit = v.note_edit.take();
            ui.scope_builder(UiBuilder::new().max_rect(panel), |ui| {
                v.detail(ui, stack, false, &mut pick, &mut ev, &mut menus, &mut edit);
            });
            v.note_edit = edit;
            if let Some(PanelEv::Note(e)) = ev.take() {
                v.note_event(e);
            }
            let popups: Vec<egui::Id> = menus
                .iter()
                .map(|(_, b)| Popup::default_response_id(b))
                .collect();
            shown.buttons = menus.iter().map(|(i, b)| (*i, b.rect)).collect();
            v.card_menus(menus);
            let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0));
            let resp = ui.interact(canvas, egui::Id::new("canvas"), Sense::click_and_drag());
            v.interact(ui, canvas, &resp);
            // Separators drawn as "—" are counted, not listed.
            let lines = |mut t: Vec<(String, Rect)>| {
                let n = t.len();
                t.retain(|(s, _)| s != "—");
                (n - t.len(), t)
            };
            (shown.canvas_lines, shown.canvas) =
                lines(on(ui.ctx(), Popup::default_response_id(&resp)));
            for id in popups {
                shown.panel.extend(lines(on(ui.ctx(), id)).1);
            }
            ui.ctx().graphics(|g| {
                if let Some(l) = g.get(LayerId::background()) {
                    l.all_entries()
                        .for_each(|c| texts(&c.shape, &mut shown.detail));
                }
            });
            shown.detail.retain(|(_, r)| panel.contains_rect(*r));
        });
        out.textures_delta.clear();
        // As the tab does at the end of each frame.
        v.keys_busy = ctx.text_edit_focused() || egui::Popup::is_any_open(ctx);
        shown
    }

    /// A click with `button` at `at`, and frames after it for a menu it
    /// opens to be drawn.
    fn click(ctx: &egui::Context, v: &mut MapView, at: Pos2, button: egui::PointerButton) -> Shown {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(ctx, v, vec![egui::Event::PointerMoved(at)]);
        frame(ctx, v, vec![press(true)]);
        frame(ctx, v, vec![press(false)]);
        frame(ctx, v, vec![]);
        frame(ctx, v, vec![])
    }

    fn words(t: &[(String, Rect)]) -> Vec<String> {
        t.iter().map(|(s, _)| s.clone()).collect()
    }

    fn item(t: &[(String, Rect)], label: &str) -> Pos2 {
        t.iter()
            .find(|(s, _)| s == label)
            .unwrap_or_else(|| panic!("no {label:?} in {:?}", words(t)))
            .1
            .center()
    }

    #[test]
    fn a_cards_menu_button_in_the_panel_is_its_right_click_menu() {
        let ctx = egui::Context::default();
        let mut v = on_canvas();
        // With IPs, and an access point: every section of the menu.
        for (i, n) in v.map.nodes.iter_mut().enumerate() {
            n.ip = Some(std::net::Ipv4Addr::new(10, 0, 0, i as u8));
        }
        v.map.nodes[5].zone = Zone::Wireless;
        let middle = |v: &MapView, i: usize| v.pos[&v.map.nodes[i].id] + CARD / 2.0;
        let (right, left) = (egui::PointerButton::Secondary, egui::PointerButton::Primary);

        // Two cards picked: right-click one of them on the canvas.
        v.select(Some(7));
        v.toggle(4);
        let at = middle(&v, 4);
        let rc = click(&ctx, &mut v, at, right);
        assert!(words(&rc.canvas).contains(&"2 selected cards".to_string()));
        assert_eq!(v.menu_cards(), [4, 7], "acts on both");
        // Esc closes it and keeps the selection.
        frame(&ctx, &mut v, vec![]);
        let esc = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut v, vec![esc.clone()]);
        let shown = frame(&ctx, &mut v, vec![]);
        assert!(shown.canvas.is_empty() && v.menu.is_none());
        assert_eq!(set(&v.selection()), set(&[4, 7]));

        // The same card's ☰ in the panel: the very same items.
        let menu = click(&ctx, &mut v, shown.buttons[&4].center(), left);
        assert!(menu.canvas.is_empty());
        assert_eq!(words(&menu.panel), words(&rc.canvas));
        assert_eq!(v.menu_cards(), [4, 7], "acts on both");
        assert_eq!(v.menu, Some(Menu::Panel("n4".into())));
        // Esc closes it too, the selection kept.
        frame(&ctx, &mut v, vec![esc]);
        let shown = frame(&ctx, &mut v, vec![]);
        assert!(shown.panel.is_empty() && v.menu.is_none());
        assert_eq!(set(&v.selection()), set(&[4, 7]));

        // An item picked there acts on both cards, and closes the menu.
        let menu = click(&ctx, &mut v, shown.buttons[&4].center(), left);
        click(&ctx, &mut v, item(&menu.panel, "Pin all 2"), left);
        assert!(v.is_kept(4) && v.is_kept(7));
        let shown = frame(&ctx, &mut v, vec![]);
        assert!(shown.panel.is_empty() && v.menu.is_none());
        assert!(v.menu_cards().is_empty());

        // On a card alone: that card's items, the same both ways.
        for k in [4, 7] {
            v.keep(k);
        }
        v.select(Some(5));
        let shown = frame(&ctx, &mut v, vec![]);
        let menu = click(&ctx, &mut v, shown.buttons[&5].center(), left);
        assert_eq!(words(&menu.panel)[0], "card 5");
        for item in ["Ping", "Open in a tab", "Open web admin", "Copy IP"] {
            assert!(has(&words(&menu.panel), item), "no {item:?}");
        }
        frame(&ctx, &mut v, vec![]);
        click(
            &ctx,
            &mut v,
            item(&menu.panel, "Select its path to the WAN"),
            left,
        );
        assert_eq!(set(&v.selection()), set(&[0, 1, 2, 3, 5]));
        let at = middle(&v, 5);
        let rc = click(&ctx, &mut v, at, right);
        assert!(words(&rc.canvas).contains(&"5 selected cards".to_string()));
        let at = frame(&ctx, &mut v, vec![]).buttons[&5].center();
        let menu = click(&ctx, &mut v, at, left);
        assert_eq!(words(&menu.panel), words(&rc.canvas));
        for item in ["Ping all 5", "Port Check all 5", "Copy IPs"] {
            assert!(has(&words(&menu.panel), item), "no {item:?}");
        }
    }

    /// tree8 laid out on the canvas, each card labelled "card i".
    fn on_canvas() -> MapView {
        let mut v = tree8();
        v.zoom = 1.0;
        for (i, n) in v.map.nodes.iter_mut().enumerate() {
            n.label = format!("card {i}");
            let at = pos2(10.0 + (i % 3) as f32 * 260.0, 10.0 + (i / 3) as f32 * 130.0);
            v.pos.insert(n.id.clone(), at);
        }
        v
    }

    /// The canvas's right-click menu on card `i`, in a fresh context.
    fn menu_on(v: &mut MapView, i: usize) -> Vec<String> {
        let ctx = egui::Context::default();
        let at = v.pos[&v.map.nodes[i].id] + CARD / 2.0;
        words(&click(&ctx, v, at, egui::PointerButton::Secondary).canvas)
    }

    fn has(menu: &[String], item: &str) -> bool {
        menu.iter().any(|s| s == item)
    }

    /// A card with clients it doesn't draw: the menu offers its +N and −
    /// chips' steps (not on a saved map, which keeps its cards).
    #[test]
    fn the_card_menu_shows_a_cards_clients_a_few_at_a_time() {
        let mut v = on_canvas();
        let id = v.map.nodes[2].id.clone();
        v.map.under.insert(id.clone(), (0, 20));
        let menu = menu_on(&mut v, 2);
        assert!(has(&menu, "Show 8 more clients") && has(&menu, "Show all 20 clients"));
        assert!(!has(&menu, "Fold clients"));
        let ctx = egui::Context::default();
        let at = v.pos[&id] + CARD / 2.0;
        let rc = click(&ctx, &mut v, at, egui::PointerButton::Secondary);
        frame(&ctx, &mut v, vec![]);
        let left = egui::PointerButton::Primary;
        click(&ctx, &mut v, item(&rc.canvas, "Show 8 more clients"), left);
        assert_eq!(v.reveal.get(&id), Some(&crate::map::STEP));
        // Stepped open: the next few, the rest, or back to the level.
        v.map.under.insert(id.clone(), (8, 30));
        let menu = menu_on(&mut v, 2);
        for item in [
            "Show 8 more clients",
            "Show the other 22 clients",
            "Fold clients",
        ] {
            assert!(has(&menu, item), "no {item:?} in {menu:?}");
        }
        // The panel's buttons are the same steps, shorter beside its
        // "8 of 30 clients shown".
        let short: Vec<String> = v.reveal_items(2).into_iter().map(|it| it.short).collect();
        assert_eq!(short, ["Show 8 more", "Show all 22", "Fold clients"]);
        // Cards without clients, and a saved map: none.
        assert!(!menu_on(&mut v, 3).iter().any(|s| s.contains("clients")));
        v.frozen = Some(Box::default());
        assert!(!menu_on(&mut v, 2).iter().any(|s| s.contains("clients")));
    }

    #[test]
    fn the_card_menu_tests_and_copies_only_what_a_card_has() {
        let mut v = on_canvas();
        let ip = |n: u8| Some(std::net::Ipv4Addr::new(10, 0, 0, n));
        // No IP: nothing to test or copy but the details.
        let menu = menu_on(&mut v, 7);
        for item in [
            "Ping",
            "Port Check",
            "Traceroute",
            "Open in a tab",
            "Copy IP",
        ] {
            assert!(!has(&menu, item), "{item:?} in {menu:?}");
        }
        assert!(has(&menu, "Copy details"));
        // A client with an IP and a MAC: the tests, both copies (showing
        // what they copy), no web admin.
        v.map.nodes[7].ip = ip(7);
        v.map.nodes[7].mac = Some([0, 0x11, 0x22, 0x33, 0x44, 0x55]);
        v.map.nodes[7].zone = Zone::Client;
        let menu = menu_on(&mut v, 7);
        for item in [
            "Ping",
            "Port Check",
            "Traceroute",
            "Open in a tab",
            "Copy IP",
            "10.0.0.7",
            "Copy MAC",
            "00:11:22:33:44:55",
        ] {
            assert!(has(&menu, item), "no {item:?} in {menu:?}");
        }
        assert!(!has(&menu, "Open web admin"));
        // A switch: its web admin too.
        v.map.nodes[3].ip = ip(3);
        v.map.nodes[3].zone = Zone::Switch;
        assert!(has(&menu_on(&mut v, 3), "Open web admin"));
        // Two picked, one with an IP: Ping and Port Check that one, no
        // Traceroute or web admin; one IP line.
        v.select(Some(7));
        v.toggle(6);
        let menu = menu_on(&mut v, 7);
        assert_eq!(menu[0], "2 selected cards");
        for item in [
            "Ping the 1 with an IP",
            "Port Check the 1 with an IP",
            "Copy IPs",
        ] {
            assert!(has(&menu, item), "no {item:?} in {menu:?}");
        }
        for item in ["Traceroute", "Open in a tab", "Open web admin"] {
            assert!(!has(&menu, item), "{item:?} in {menu:?}");
        }
        v.map.nodes[6].ip = ip(6);
        assert!(has(&menu_on(&mut v, 7), "Ping all 2"));
    }

    /// tree8 on the canvas, cards 4, 5 and 7 with a loopback IP each.
    fn testable() -> MapView {
        let mut v = on_canvas();
        for i in [4, 5, 7] {
            v.map.nodes[i].ip = Some(std::net::Ipv4Addr::new(127, 0, 0, i as u8));
        }
        v
    }

    /// Pick `label` on the canvas's right-click menu on card `i`.
    fn menu_pick(ctx: &egui::Context, v: &mut MapView, i: usize, label: &str) -> Shown {
        let at = v.pos[&v.map.nodes[i].id] + CARD / 2.0;
        let menu = click(ctx, v, at, egui::PointerButton::Secondary);
        click(
            ctx,
            v,
            item(&menu.canvas, label),
            egui::PointerButton::Primary,
        );
        let mut shown = frame(ctx, v, vec![]);
        // Time for the panel to scroll.
        for _ in 0..6 {
            shown = frame(ctx, v, vec![]);
        }
        shown
    }

    /// Boxes cover most of the map in Zones and Sites, so a right-click
    /// on a box's empty background gives the box's items and the map's
    /// own (Fit, Legend, Export) too.
    #[test]
    fn a_boxs_background_menu_keeps_the_maps_items() {
        let mut v = on_canvas();
        v.mode = Some(Mode::Zones);
        v.lay.boxes.push(crate::map::layout::Group {
            rect: Rect::from_min_max(pos2(0.0, 400.0), pos2(780.0, 495.0)),
            title: "Access points".into(),
            zone: Some(Zone::Wireless),
            count: 0,
            tree: false,
        });
        let ctx = egui::Context::default();
        let menu = words(
            &click(
                &ctx,
                &mut v,
                pos2(400.0, 450.0),
                egui::PointerButton::Secondary,
            )
            .canvas,
        );
        for item in [
            "Access points",
            "Move group",
            "Copy group name",
            "Fit",
            "Center",
            "Links",
            "Legend",
            "Export",
        ] {
            assert!(has(&menu, item), "no {item:?} in {menu:?}");
        }
        // Outside every box: the map's own menu, as before.
        let menu = words(
            &click(
                &egui::Context::default(),
                &mut v,
                pos2(790.0, 450.0),
                egui::PointerButton::Secondary,
            )
            .canvas,
        );
        assert!(
            has(&menu, "Fit") && !has(&menu, "Copy group name"),
            "{menu:?}"
        );
    }

    #[test]
    fn a_test_from_the_menu_scrolls_the_panel_to_its_result() {
        let ctx = egui::Context::default();
        let mut v = testable();
        // A long card: its Results far below the top of the panel.
        v.map.nodes[4].flags = (0..60).map(|k| format!("flag {k}")).collect();
        let shown = menu_pick(&ctx, &mut v, 4, "Port Check");
        assert_eq!(v.selection(), [4]);
        let texts = words(&shown.detail);
        assert!(has(&texts, "Port Check"), "result not in view: {texts:?}");
        assert!(!has(&texts, "⚠ flag 0"), "the panel didn't scroll");
    }

    #[test]
    fn a_test_on_a_card_outside_the_selection_adds_it() {
        let ctx = egui::Context::default();
        let mut v = testable();
        v.select(Some(7));
        menu_pick(&ctx, &mut v, 5, "Ping");
        assert_eq!(set(&v.selection()), set(&[5, 7]));
        assert!(!v.runs.runs("n5").is_empty());
    }

    #[test]
    fn ping_all_leaves_a_running_test_to_finish() {
        let ctx = egui::Context::default();
        let mut v = testable();
        v.select(Some(4));
        v.toggle(5);
        v.toggle(7);
        // Card 4 already pinging (another address, to tell its run apart).
        let other: std::net::IpAddr = "127.0.0.99".parse().unwrap();
        v.runs.start("n4", Tool::Ping, other);
        let menu = menu_on(&mut v, 4);
        assert!(has(&menu, "Ping the other 2"), "{menu:?}");
        assert!(has(&menu, "Port Check all 3"), "{menu:?}");
        menu_pick(&ctx, &mut v, 4, "Ping the other 2");
        assert_eq!(v.runs.runs("n4")[0].ip, other, "card 4's ping restarted");
        assert!(!v.runs.runs("n5").is_empty());
        assert!(!v.runs.runs("n7").is_empty());
    }

    #[test]
    fn ping_all_counts_the_cards_it_will_run() {
        use interact::all_label;
        assert_eq!(all_label(Tool::Ping, 3, 3, 3), "Ping all 3");
        assert_eq!(all_label(Tool::Ping, 2, 2, 3), "Ping the 2 with an IP");
        assert_eq!(all_label(Tool::Ports, 2, 3, 3), "Port Check the other 2");
        assert_eq!(all_label(Tool::Ping, 1, 2, 3), "Ping the other 1");
        // All running: greyed out, named as if all.
        assert_eq!(all_label(Tool::Ping, 0, 3, 3), "Ping all 3");
        assert_eq!(all_label(Tool::Ping, 0, 2, 3), "Ping the 2 with an IP");
    }

    #[test]
    fn copy_ip_and_mac_give_a_line_for_each_card_that_has_one() {
        let mut v = view(3);
        v.map.nodes[0].ip = Some(std::net::Ipv4Addr::new(10, 0, 0, 5));
        v.map.nodes[0].mac = Some([0xaa, 0xbb, 0xcc, 0, 0, 1]);
        v.map.nodes[2].ip = Some(std::net::Ipv4Addr::new(10, 0, 0, 7));
        let [(ip, ips), (mac, macs)] = v.copy_lists(&[0]);
        assert_eq!((ip, mac), ("IP", "MAC"));
        assert_eq!(ips, ["10.0.0.5"]);
        assert_eq!(macs, ["aa:bb:cc:00:00:01"]);
        // Several: one per line, in the menu's order, skipping the card
        // without one.
        let [(_, ips), (_, macs)] = v.copy_lists(&[2, 1, 0]);
        assert_eq!(ips.join("\n"), "10.0.0.7\n10.0.0.5");
        assert_eq!(macs.join("\n"), "aa:bb:cc:00:00:01");
        assert!(v.copy_lists(&[1]).iter().all(|(_, l)| l.is_empty()));
    }

    #[test]
    fn collapse_is_offered_only_on_a_map_with_another_site() {
        let long = "Acme Distribution LLC — Multi-Site";
        let mut v = on_canvas();
        v.mode = Some(Mode::Sites);
        for n in &mut v.map.nodes {
            n.site = Some(long.to_string());
        }
        // The only site: collapsing it would fold the whole map.
        let menu = menu_on(&mut v, 4);
        assert!(!menu.iter().any(|s| s.starts_with("Collapse")), "{menu:?}");
        // Folded already (from before): it can still be opened.
        v.map.closed.insert(long.to_string(), Vec::new());
        assert!(has(
            &menu_on(&mut v, 4),
            "Expand Acme Distribution LLC — Mul…"
        ));
        v.map.closed.clear();
        // With another site: offered, the long name cut.
        v.map.nodes[7].site = Some("Branch".into());
        v.multi_site = true;
        let menu = menu_on(&mut v, 4);
        assert!(
            has(&menu, "Collapse Acme Distribution LLC — Mul…"),
            "{menu:?}"
        );
        assert!(has(&menu_on(&mut v, 7), "Collapse Branch"));
    }

    /// Add note… on the menu opens the editor in the card's panel; typed
    /// and saved with Ctrl+Enter, the note shows there, and the menu
    /// offers Edit and Remove. Esc leaves an edit as it was.
    #[test]
    fn a_note_is_written_from_the_menu_in_the_cards_panel() {
        let ctx = egui::Context::default();
        let mut v = on_canvas();
        let left = egui::PointerButton::Primary;
        let at = v.pos[&v.map.nodes[4].id] + CARD / 2.0;
        let rc = click(&ctx, &mut v, at, egui::PointerButton::Secondary);
        frame(&ctx, &mut v, vec![]);
        click(&ctx, &mut v, item(&rc.canvas, "Add note…"), left);
        assert!(v.note_edit.is_some());
        assert_eq!(v.selection(), [4], "its card shows in the panel");
        frame(&ctx, &mut v, vec![]);
        frame(&ctx, &mut v, vec![egui::Event::Text("Spare PSU".into())]);
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(
            &ctx,
            &mut v,
            vec![key(egui::Key::Enter, egui::Modifiers::COMMAND)],
        );
        assert_eq!(v.notes.get("n4").map(String::as_str), Some("Spare PSU"));
        assert!(v.note_edit.is_none());
        v.stamp_notes();
        let shown = frame(&ctx, &mut v, vec![]);
        assert!(
            has(&words(&shown.detail), "Spare PSU"),
            "{:?}",
            words(&shown.detail)
        );
        let menu = menu_on(&mut v, 4);
        assert!(has(&menu, "Edit note…") && has(&menu, "Remove note"));
        // Edit, type, Esc: as it was.
        v.open_note(&[4]);
        frame(&ctx, &mut v, vec![]);
        frame(&ctx, &mut v, vec![egui::Event::Text(" (gone)".into())]);
        frame(
            &ctx,
            &mut v,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
        );
        frame(&ctx, &mut v, vec![]);
        assert!(v.note_edit.is_none());
        assert_eq!(v.notes["n4"], "Spare PSU");
        assert_eq!(set(&v.selection()), set(&[4]), "Esc was the editor's");
    }

    #[test]
    fn the_card_menu_sets_sections_apart_with_one_separator() {
        // A card on its own (no uplink, nothing below), no IP: Pin and
        // Select, Add note…, then Copy details, with one separator under
        // the title and one between each, none for the empty sections in
        // between.
        let ctx = egui::Context::default();
        let mut v = on_canvas();
        v.map.nodes[4].up = None;
        let at = v.pos[&v.map.nodes[4].id] + CARD / 2.0;
        let shown = click(&ctx, &mut v, at, egui::PointerButton::Secondary);
        assert_eq!(
            words(&shown.canvas),
            ["card 4", "Pin", "Select", "Add note…", "Copy details"]
        );
        assert_eq!(shown.canvas_lines, 3);
    }

    #[test]
    fn web_admin_is_https_unless_the_ports_say_otherwise() {
        use interact::web_url;
        let v4: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        assert_eq!(web_url(v4, &[]), "https://10.0.0.1");
        assert_eq!(web_url(v4, &[22, 80]), "http://10.0.0.1");
        assert_eq!(web_url(v4, &[80, 8443]), "https://10.0.0.1:8443");
        assert_eq!(web_url(v4, &[80, 443]), "https://10.0.0.1");
        let v6: std::net::IpAddr = "fe80::1".parse().unwrap();
        assert_eq!(web_url(v6, &[]), "https://[fe80::1]");
    }

    #[test]
    fn long_names_are_cut_with_an_ellipsis() {
        use interact::ellipsis;
        assert_eq!(ellipsis("HQ", 28), "HQ");
        assert_eq!(ellipsis("abcdef", 6), "abcdef");
        assert_eq!(ellipsis("abc defg", 5), "abc…");
        assert_eq!(
            ellipsis("Acme Distribution LLC — Multi-Site", 28),
            "Acme Distribution LLC — Mul…"
        );
    }

    #[test]
    fn web_admin_is_for_network_gear_and_servers_that_have_a_page() {
        let card = |zone, kind: &str| Node {
            zone,
            kind: kind.into(),
            ..Default::default()
        };
        for z in [Zone::Gateway, Zone::Switch, Zone::Wireless, Zone::Gear] {
            assert!(interact::has_admin(&card(z, "")), "{z:?}");
        }
        for z in [Zone::Client, Zone::Internet, Zone::Unknown] {
            assert!(!interact::has_admin(&card(z, "NAS")), "{z:?}");
        }
        // A server or device: not just any (a domain controller has no
        // page), but a NAS, a hypervisor, a UPS card, or one with a web
        // port known to be open.
        for kind in ["DOMAIN CONTROLLER", "SQL SERVER", "FILE SERVER", "PRINTER"] {
            assert!(!interact::has_admin(&card(Zone::Server, kind)), "{kind}");
        }
        for kind in ["NAS", "ESXI HOST", "BDR", "UPS", "IDRAC"] {
            assert!(interact::has_admin(&card(Zone::Server, kind)), "{kind}");
        }
        let mut ups = card(Zone::Device, "UPS");
        assert!(interact::has_admin(&ups));
        ups.kind = "POWER".into();
        ups.chips = vec!["Synology".into()];
        assert!(interact::has_admin(&ups), "a chip names it");
        let mut dc = card(Zone::Server, "DOMAIN CONTROLLER");
        dc.open = vec![135, 443];
        assert!(interact::has_admin(&dc), "https is open");
        // A client never, whatever it is.
        let mut nas = card(Zone::Server, "NAS");
        nas.client = true;
        assert!(!interact::has_admin(&nas));
    }
}
