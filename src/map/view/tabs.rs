//! The Map tab with its saved maps: the live map (from the sweep and the
//! files loaded) and maps saved as they were, each a sub-tab in a bar
//! along the bottom. The bar is hidden until a map is saved or the
//! Saved maps by the zoom controls opens it.

use super::history::Edits;
use super::saved::{self, Purpose, Snapshot};
use super::*;
use crate::job::{Job, pump};
use std::path::{Path, PathBuf};

/// One saved map: its file, and its own view (with its own undo).
struct Tab {
    path: PathBuf,
    name: String,
    created: String,
    /// The edits as they are in the file, to tell unsaved changes by.
    saved: Edits,
    view: MapView,
}

/// What a sub-tab's right-click menu asked for.
enum TabEv {
    Rename(usize),
    Update(usize),
    Replace(usize),
    Duplicate(usize),
    Export(usize),
    Delete(usize),
    /// Mark it as found, a plan or a training map.
    Purpose(usize, Purpose),
}

/// What the dialog over the map is asking before it's done.
#[derive(Clone, Copy)]
enum Ask {
    Delete(usize),
    /// Saved map `i` overwritten with the map showing.
    Replace(usize),
}

/// Longest sub-tab label; the full name is in its tooltip.
const LABEL_MAX: usize = 28;

pub struct MapTabs {
    live: MapView,
    saved: Vec<Tab>,
    /// The saved map showing; None for Live.
    shown: Option<usize>,
    /// The bar along the bottom is open.
    bar: bool,
    /// Where saved maps are kept (None: no settings folder).
    dir: Option<PathBuf>,
    /// The sub-tab being renamed, with the name as typed so far.
    renaming: Option<(usize, String)>,
    /// What the dialog open over the map asks about (Delete, Replace).
    asking: Option<Ask>,
    /// Open…'s file dialog, then the file read.
    opening: Option<Job<(PathBuf, Result<Snapshot, String>)>>,
    /// Saved maps' files that couldn't be read, and why.
    skipped: Vec<String>,
    /// The Map tab has shown, so `skipped` was told once.
    told: bool,
    /// How card notes are redacted, as every map was last told.
    ov: crate::redact::Overrides,
}

impl MapTabs {
    /// The live map, and the maps saved before (not for the screenshot
    /// tour: its pictures show the map as a new user sees it).
    pub fn new(mode: Option<Mode>) -> Self {
        let tour = std::env::var_os("IS_ALIVE_SHOTS").is_some();
        let dir = saved::folder().filter(|_| !tour);
        let mut live = MapView::new(mode);
        live.note_store = Some(super::notes::Store::open());
        Self::in_dir(live, dir)
    }

    fn in_dir(live: MapView, dir: Option<PathBuf>) -> Self {
        let (found, skipped) = dir.as_deref().map(saved::list).unwrap_or_default();
        let saved: Vec<Tab> = found.into_iter().map(|(p, s)| tab(p, s)).collect();
        Self {
            live,
            bar: !saved.is_empty() || !skipped.is_empty(),
            saved,
            shown: None,
            dir,
            renaming: None,
            asking: None,
            opening: None,
            skipped,
            told: false,
            ov: Default::default(),
        }
    }

    /// Saved maps that couldn't be read, said once the Map tab shows (and
    /// kept in the bar after).
    fn tell_skipped(&mut self) {
        if self.told {
            return;
        }
        self.told = true;
        let n = self.skipped.len();
        if n > 0 {
            widgets::toast(
                format!(
                    "Couldn't read {n} saved map{}: {}",
                    if n == 1 { "" } else { "s" },
                    self.skipped.join("; ")
                ),
                false,
            );
        }
    }

    /// Once a frame, whichever tab shows: card notes are redacted as the
    /// Notes tab redacts now (every map's, shown or not, as Update writes
    /// one that isn't). When that changes, what's kept is rewritten at
    /// once, as notes.md is: the live map's kept notes, and the notes in
    /// saved maps' files.
    pub fn redact(&mut self, ov: &crate::redact::Overrides) {
        if self.ov != *ov {
            self.ov = ov.clone();
            self.live.ov = ov.clone();
            for t in &mut self.saved {
                t.view.ov = ov.clone();
            }
            for i in 0..self.saved.len() {
                self.redact_file(i);
            }
        }
        self.live.sync_notes();
    }

    /// Saved map `i`'s file with its notes redacted as now, when that
    /// changes any. Only the notes: its unsaved changes stay unsaved.
    fn redact_file(&mut self, i: usize) {
        let t = &self.saved[i];
        let ov = &self.ov;
        let dirty = |n: &String| super::notes::clean(n, ov) != *n;
        if !t.saved.notes.values().any(dirty) {
            return;
        }
        let Ok(mut s) = saved::read(&t.path) else {
            return;
        };
        if !s.edits.notes.values().any(dirty) {
            return;
        }
        for n in s.edits.notes.values_mut() {
            *n = super::notes::clean(n, ov);
        }
        if let Err(e) = saved::write(&t.path, &s) {
            widgets::toast(
                format!("Couldn't redact the notes in \"{}\": {e}", t.name),
                false,
            );
        }
    }

    /// Every card note's text, on every map and kept for the live one:
    /// the Notes tab's keep list holds on to values still in one.
    pub fn note_texts(&self) -> Vec<String> {
        let mut v = self.live.note_texts();
        for t in &self.saved {
            v.extend(t.view.note_texts());
        }
        v
    }

    /// The live map's card notes for the Site Bundle, redacted with `ov`
    /// (the Notes tab's, as of now) and without emoji style marks, as the
    /// Notes part: each card's name, role, address and note, as JSON. None
    /// without any, or when the live map shows an example (its notes are
    /// about no real site).
    pub fn bundle_notes(&self, ov: &crate::redact::Overrides) -> Option<(usize, String)> {
        if matches!(self.live.note_scope, super::notes::Scope::Example(_)) {
            return None;
        }
        let rows: Vec<serde_json::Value> = self
            .live
            .map
            .nodes
            .iter()
            .filter_map(|n| Some((n, self.live.notes.get(&n.id)?)))
            .map(|(n, raw)| {
                serde_json::json!({
                    "card": n.id,
                    "name": n.label,
                    "kind": n.kind,
                    "ip": n.ip.map(|ip| ip.to_string()),
                    "mac": n.mac.as_ref().map(crate::net::mac::fmt),
                    "site": n.site,
                    "note": crate::theme::strip_emoji_styles(&super::notes::clean(raw, ov)),
                })
            })
            .collect();
        (!rows.is_empty()).then(|| {
            (
                rows.len(),
                serde_json::to_string_pretty(&rows).unwrap_or_default(),
            )
        })
    }

    /// The map on screen: Live or a saved one.
    fn view(&mut self) -> &mut MapView {
        match self.shown {
            Some(i) => &mut self.saved[i].view,
            None => &mut self.live,
        }
    }

    /// Rebuild the live map from the data on the next frame (Reload).
    pub fn reload(&mut self) {
        self.live.reload();
    }

    /// What a finished sweep found is on the live map (not left off it as
    /// another network's).
    pub fn shows_sweep(&self, src: &Sources) -> bool {
        self.live.shows_sweep(src)
    }

    /// Keep every map's card runs going while another tab is shown.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.live.poll(ctx);
        for t in &mut self.saved {
            t.view.poll(ctx);
        }
    }

    /// For the screenshot tour: a card selected on the map showing.
    pub fn select_for_shots(&mut self) {
        self.view().select_for_shots();
    }

    /// For the screenshot tour: the map in Zones with its access points' box
    /// selected. False until the Zones layout is in (a frame later).
    pub fn zones_for_shots(&mut self) -> bool {
        self.view().zones_for_shots()
    }

    /// For the screenshot tour: back to the map's own layout.
    pub fn default_layout_for_shots(&mut self) {
        self.view().default_layout_for_shots();
    }

    /// A saved map handed over by Config Export (opened or dropped): kept
    /// with the others and shown. Why not, when it couldn't be.
    pub fn open_file(&mut self, path: &Path, data: &[u8]) -> Result<(), String> {
        let name = crate::tools::export::file_name(path);
        let r = saved::parse(data).and_then(|s| self.add(s, &name));
        if let Err(e) = &r {
            widgets::toast(format!("Couldn't open {name}: {e}"), false);
        }
        r
    }

    /// Keep a map read from a file as a new saved map, and show it.
    fn add(&mut self, mut s: Snapshot, from: &str) -> Result<(), String> {
        s.created = saved::now();
        self.keep_new(s)?;
        widgets::toast(format!("Opened {from} as a saved map"), true);
        Ok(())
    }

    /// Write `s` as a new saved map, add its sub-tab and show it. A name
    /// another saved map has gets a number: "Acme (2)".
    fn keep_new(&mut self, mut s: Snapshot) -> Result<(), String> {
        let Some(dir) = &self.dir else {
            return Err("no settings folder to keep saved maps in".into());
        };
        s.name = self.unique_name(&s.name, None);
        // A file opened from elsewhere: its notes redacted as here.
        for n in s.edits.notes.values_mut() {
            *n = super::notes::clean(n, &self.ov);
        }
        let path = saved::fresh_path(dir, &s.name);
        saved::write(&path, &s).map_err(|e| format!("couldn't save the map: {e}"))?;
        self.saved.push(tab(path, s));
        if let Some(t) = self.saved.last_mut() {
            t.view.ov = self.ov.clone();
        }
        self.shown = Some(self.saved.len() - 1);
        self.bar = true;
        Ok(())
    }

    /// `name`, or with a number if another saved map (not `skip`) has it.
    fn unique_name(&self, name: &str, skip: Option<usize>) -> String {
        let taken = |n: &str| {
            self.saved
                .iter()
                .enumerate()
                .any(|(k, t)| Some(k) != skip && t.name == n)
        };
        (1..)
            .map(|k| match k {
                1 => name.to_string(),
                k => format!("{name} ({k})"),
            })
            .find(|n| !taken(n))
            .unwrap_or_else(|| name.to_string())
    }

    /// The map showing, as a saved map called `name`. None when it's
    /// empty.
    fn snapshot_shown(&self, src: &Sources, name: &str, created: String) -> Option<Snapshot> {
        match self.shown {
            Some(i) => {
                let v = &self.saved[i].view;
                let full = saved::copy_map(v.frozen.as_deref()?);
                Some(v.snapshot(name, created, full))
            }
            None => {
                let v = &self.live;
                if v.map.nodes.is_empty() {
                    return None;
                }
                // Built again with every site open: collapsed ones open
                // again from the saved map.
                let full = crate::map::build(&v.input(src));
                let mut s = v.snapshot(name, created, full);
                // The blank example is a start to fill in: a plan, not
                // what was found.
                if src
                    .export
                    .mapnetwork()
                    .is_some_and(|(p, _)| crate::skills::is_blank(p))
                {
                    s.purpose = Purpose::Plan;
                }
                Some(s)
            }
        }
    }

    /// The name a map gets when saved: the saved map's own, or the date.
    fn default_name(&self) -> String {
        match self.shown {
            Some(i) => format!("{} copy", self.saved[i].name),
            None => format!("Map {}", chrono::Local::now().format("%Y-%m-%d %H:%M")),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, src: Sources) -> Option<Action> {
        self.poll_open();
        self.tell_skipped();
        self.redact(src.redact);
        if self.bar {
            egui::Panel::bottom("map_saved_bar")
                .resizable(false)
                .show(ui, |ui| self.bar_ui(ui, &src));
        }
        self.confirm_dialog(ui.ctx(), &src);
        let bar = self.bar;
        let saved = self.shown.is_some();
        let view = self.view();
        view.bar = bar;
        let act = view.ui(ui, src);
        let flip = std::mem::take(&mut view.bar_flip);
        let empty = view.map.nodes.is_empty();
        if flip {
            self.bar = !self.bar;
        }
        // Nothing mapped, so no canvas to hold Saved maps: a link instead.
        if empty && !self.bar && !self.saved.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(10.0);
                if ui.link("Show the saved maps").clicked() {
                    self.bar = true;
                }
            });
        }
        match act {
            // Scanning, loading and examples are the live map's.
            Some(
                Action::Sweep
                | Action::Export
                | Action::Reload
                | Action::Example(_)
                | Action::CloseExample,
            ) if saved => {
                self.shown = None;
                act
            }
            // A saved map's layout isn't the one new maps start with.
            Some(Action::Layout(_)) if saved => None,
            act => act,
        }
    }

    /// Live, a sub-tab for each saved map, + Save current and Open….
    fn bar_ui(&mut self, ui: &mut Ui, src: &Sources) {
        let mut ev = None;
        let mut save = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("⏷")
                    .on_hover_text(
                        "Hide this bar (Saved maps, by the zoom controls, opens it again)",
                    )
                    .clicked()
                {
                    self.bar = false;
                }
                if ui
                    .button("Open…")
                    .on_hover_text("Open a saved map's .json file (someone else's too)")
                    .clicked()
                {
                    self.pick_file(ui.ctx());
                }
                let empty = match self.shown {
                    Some(_) => false,
                    None => self.live.map.nodes.is_empty(),
                };
                save = ui
                    .add_enabled(!empty, egui::Button::new("+ Save current"))
                    .on_hover_text(
                        "Keep the map showing as it is now, with your moves, pins and folds, \
                         as a sub-tab here",
                    )
                    .on_disabled_hover_text(NOTHING_YET)
                    .clicked();
                self.skipped_note(ui);
                ui.separator();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    // The wheel scrolls the tabs sideways, and the bar
                    // takes room of its own rather than covering them.
                    ui.style_mut().always_scroll_the_only_direction = true;
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                    egui::ScrollArea::horizontal()
                        .id_salt("map_saved_tabs")
                        .show(ui, |ui| {
                            ui.horizontal(|ui| ev = self.tabs_row(ui));
                        });
                });
            });
        });
        if save {
            let name = self.default_name();
            if let Some(s) = self.snapshot_shown(src, &name, saved::now()) {
                match self.keep_new(s) {
                    Ok(()) => {
                        let i = self.saved.len() - 1;
                        widgets::toast(
                            format!(
                                "Saved \"{}\": right-click its tab to rename it",
                                self.saved[i].name
                            ),
                            true,
                        );
                        self.renaming = Some((i, self.saved[i].name.clone()));
                    }
                    Err(e) => widgets::toast(format!("Couldn't save the map: {e}"), false),
                }
            }
        }
        match ev {
            Some(TabEv::Rename(i)) => self.renaming = Some((i, self.saved[i].name.clone())),
            Some(TabEv::Update(i)) => self.update(i),
            Some(TabEv::Replace(i)) => self.asking = Some(Ask::Replace(i)),
            Some(TabEv::Duplicate(i)) => {
                let t = &self.saved[i];
                let name = format!("{} copy", t.name);
                let full = t.view.frozen.as_deref().map(saved::copy_map);
                if let Some(full) = full {
                    let s = t.view.snapshot(&name, saved::now(), full);
                    if let Err(e) = self.keep_new(s) {
                        widgets::toast(format!("Couldn't copy the map: {e}"), false);
                    }
                }
            }
            Some(TabEv::Export(i)) => {
                let t = &self.saved[i];
                if let Some(full) = t.view.frozen.as_deref().map(saved::copy_map) {
                    let s = t.view.snapshot(&t.name, t.created.clone(), full);
                    widgets::save_file(
                        format!("{}.json", saved::slug(&t.name)),
                        "is-alive map",
                        vec!["json"],
                        saved::to_json(&s),
                    );
                }
            }
            Some(TabEv::Delete(i)) => self.asking = Some(Ask::Delete(i)),
            Some(TabEv::Purpose(i, p)) => self.set_purpose(i, p),
            None => {}
        }
    }

    /// Saved maps' files that couldn't be read: how many, and why on
    /// hover.
    fn skipped_note(&self, ui: &mut Ui) {
        let n = self.skipped.len();
        if n == 0 {
            return;
        }
        ui.label(RichText::new(format!("⚠ {n} couldn't be read")).color(widgets::warn()))
            .on_hover_text(format!(
                "These files in {} aren't saved maps this version can open:\n{}",
                self.dir
                    .as_deref()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default(),
                self.skipped.join("\n")
            ));
    }

    /// The sub-tabs; what one's menu asked for.
    fn tabs_row(&mut self, ui: &mut Ui) -> Option<TabEv> {
        let mut ev = None;
        ui.weak("Maps");
        if ui
            .selectable_label(self.shown.is_none(), "Live")
            .on_hover_text("The map from the sweep and the files loaded now")
            .clicked()
        {
            self.shown = None;
        }
        for i in 0..self.saved.len() {
            if self.renaming.as_ref().is_some_and(|r| r.0 == i) {
                self.rename_box(ui);
                continue;
            }
            let t = &self.saved[i];
            let changed = t.view.edits_differ(&t.saved);
            let (purpose, faults) = (t.view.purpose, t.view.faults.len());
            let what = if changed {
                "Changed since it was saved: right-click › Update with current keeps the changes"
            } else {
                "A saved map. Right-click to mark what it is, rename, update, duplicate, export \
                 or delete it"
            };
            let note = purpose_note(purpose, faults)
                .map(|n| format!("{n}\n"))
                .unwrap_or_default();
            let tip = format!(
                "{}\n{note}{what}\n{} · saved {}",
                t.name,
                crate::tools::export::file_name(&t.path),
                t.created.get(..10).unwrap_or(&t.created)
            );
            let label = tab_text(ui, &t.name, purpose, changed);
            let r = ui
                .selectable_label(self.shown == Some(i), label)
                .on_hover_text(tip);
            if r.clicked() {
                self.shown = Some(i);
            }
            r.context_menu(|ui| {
                ev = tab_menu(ui, i, self.shown, changed, purpose, faults);
            });
        }
        if self.saved.is_empty() {
            ui.weak("Save the map as it is now to come back to it later");
        }
        ev
    }

    /// The name box in place of a sub-tab: Enter or a click away keeps
    /// the name, Esc doesn't.
    fn rename_box(&mut self, ui: &mut Ui) {
        let Some((i, name)) = &mut self.renaming else {
            return;
        };
        let i = *i;
        let r = widgets::line(ui, crate::input::Field::Text, name, 160.0, "Map name");
        if !r.has_focus() && !r.lost_focus() {
            r.request_focus();
        }
        if r.lost_focus() {
            let keep = !ui.input(|k| k.key_pressed(egui::Key::Escape));
            let name = crate::input::line(name, saved::NAME_MAX);
            self.renaming = None;
            if keep && !name.is_empty() && name != self.saved[i].name {
                let name = self.unique_name(&name, Some(i));
                self.rename(i, name);
            }
        }
    }

    /// Saved map `i` as its file has it (unsaved changes left out); what's
    /// showing if the file went.
    fn on_file(&self, i: usize) -> Option<Snapshot> {
        let t = &self.saved[i];
        match saved::read(&t.path) {
            Ok(s) => Some(s),
            Err(_) => t.view.frozen.as_deref().map(|full| {
                t.view
                    .snapshot(&t.name, t.created.clone(), saved::copy_map(full))
            }),
        }
    }

    /// Rename a saved map: its file takes the new name too. What's in the
    /// file stays as it was (unsaved changes aren't written).
    fn rename(&mut self, i: usize, name: String) {
        let Some(dir) = self.dir.clone() else { return };
        let Some(mut s) = self.on_file(i) else { return };
        let t = &self.saved[i];
        s.name = name.clone();
        let path = if saved::slug(&name) == saved::slug(&t.name) {
            t.path.clone()
        } else {
            saved::fresh_path(&dir, &name)
        };
        match saved::write(&path, &s) {
            Ok(()) => {
                let t = &mut self.saved[i];
                if path != t.path {
                    let _ = std::fs::remove_file(&t.path);
                }
                t.path = path;
                t.name = name;
            }
            Err(e) => widgets::toast(format!("Couldn't rename the map: {e}"), false),
        }
    }

    /// Mark saved map `i` as found, a plan or a training map, in its file
    /// at once (not a step to undo: it's what the map is, not a change to
    /// it). Unsaved changes stay unsaved. A map with an answer key stays a
    /// training map.
    fn set_purpose(&mut self, i: usize, p: Purpose) {
        let t = &self.saved[i];
        if t.view.purpose == p || (p != Purpose::Training && !t.view.faults.is_empty()) {
            return;
        }
        let Some(mut s) = self.on_file(i) else { return };
        s.purpose = p;
        if let Err(e) = saved::write(&t.path, &s) {
            widgets::toast(format!("Couldn't save the map: {e}"), false);
            return;
        }
        let t = &mut self.saved[i];
        t.view.purpose = p;
        let what = match p {
            Purpose::AsFound => "as found",
            Purpose::Plan => "a plan",
            Purpose::Training => "a training map",
        };
        widgets::toast(format!("\"{}\" is now marked {what}", t.name), true);
    }

    /// Update with current: saved map `i` keeps the changes made to it
    /// since it was saved, shown or not. Its view (and its undo) carries
    /// on.
    fn update(&mut self, i: usize) {
        let t = &self.saved[i];
        let Some(full) = t.view.frozen.as_deref().map(saved::copy_map) else {
            return;
        };
        let s = t.view.snapshot(&t.name, t.created.clone(), full);
        if let Err(e) = saved::write(&t.path, &s) {
            widgets::toast(format!("Couldn't save the map: {e}"), false);
            return;
        }
        let t = &mut self.saved[i];
        // As the view has them (its notes as typed; the file's redacted).
        t.saved = t.view.edits();
        widgets::toast(format!("Updated \"{}\"", t.name), true);
    }

    /// Replace with the map showing (asked first): saved map `i` becomes
    /// Live's map or another saved one, its own map and undo gone.
    fn replace(&mut self, i: usize, src: &Sources) {
        let (name, created) = (self.saved[i].name.clone(), self.saved[i].created.clone());
        let Some(s) = self.snapshot_shown(src, &name, created) else {
            widgets::toast("Nothing on the map to save", false);
            return;
        };
        if let Err(e) = saved::write(&self.saved[i].path, &s) {
            widgets::toast(format!("Couldn't save the map: {e}"), false);
            return;
        }
        let path = self.saved[i].path.clone();
        self.saved[i] = tab(path, s);
        self.saved[i].view.ov = self.ov.clone();
        widgets::toast(format!("Replaced \"{name}\""), true);
    }

    /// Delete… and Replace…: ask first, then do it.
    fn confirm_dialog(&mut self, ctx: &egui::Context, src: &Sources) {
        let Some(ask) = self.asking else { return };
        let (Ask::Delete(i) | Ask::Replace(i)) = ask;
        if i >= self.saved.len() {
            self.asking = None;
            return;
        }
        let t = &self.saved[i];
        let (title, body, verb) = match ask {
            Ask::Delete(_) => (
                format!("Delete \"{}\"?", t.name),
                "Its file is deleted too. This can't be undone.".to_string(),
                "Delete",
            ),
            Ask::Replace(_) => (
                format!("Replace \"{}\" with the map showing?", t.name),
                format!(
                    "Its file then holds {} instead{}. This can't be undone.",
                    match self.shown {
                        Some(k) => format!("\"{}\"", self.saved[k].name),
                        None => "the live map".into(),
                    },
                    if t.view.edits_differ(&t.saved) {
                        ", and its own unsaved changes are lost"
                    } else {
                        ""
                    }
                ),
                "Replace",
            ),
        };
        let mut yes = false;
        let r = egui::Modal::new(egui::Id::new("map_confirm")).show(ctx, |ui| {
            ui.set_max_width(320.0);
            ui.strong(title);
            ui.label(body);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new(verb).color(widgets::bad()))
                    .clicked()
                {
                    yes = true;
                    ui.close();
                }
                if ui.button("Cancel").clicked() {
                    ui.close();
                }
            });
        });
        if yes {
            match ask {
                Ask::Delete(i) => self.delete(i),
                Ask::Replace(i) => self.replace(i, src),
            }
        }
        if r.should_close() {
            self.asking = None;
        }
    }

    fn delete(&mut self, i: usize) {
        let t = self.saved.remove(i);
        if let Err(e) = std::fs::remove_file(&t.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            widgets::toast(format!("Couldn't delete the map's file: {e}"), false);
        }
        self.shown = match self.shown {
            Some(k) if k == i => None,
            Some(k) if k > i => Some(k - 1),
            k => k,
        };
        self.renaming = None;
        self.asking = None;
        widgets::toast(format!("Deleted \"{}\"", t.name), true);
    }

    /// Open…: pick a saved map's file, read it in the background.
    fn pick_file(&mut self, ctx: &egui::Context) {
        if self.opening.is_some() {
            return;
        }
        self.opening = Some(Job::spawn(ctx, |tx| {
            let Some(path) = rfd::FileDialog::new()
                .set_title("Open a saved map")
                .add_filter("is-alive map", &["json"])
                .pick_file()
            else {
                return;
            };
            let r = saved::read(&path);
            tx.send((path, r));
        }));
    }

    fn poll_open(&mut self) {
        let mut got = None;
        pump(&mut self.opening, |r| got = Some(r));
        if let Some((path, r)) = got {
            let name = crate::tools::export::file_name(&path);
            if let Err(e) = r.and_then(|s| self.add(s, &name)) {
                widgets::toast(format!("Couldn't open {name}: {e}"), false);
            }
        }
    }
}

/// A saved map's sub-tab, from the map as read or written.
fn tab(path: PathBuf, s: Snapshot) -> Tab {
    Tab {
        path,
        name: s.name.clone(),
        created: s.created.clone(),
        saved: s.edits.clone(),
        view: MapView::from_saved(s),
    }
}

/// A name cut to fit a sub-tab.
fn short(name: &str) -> String {
    if name.chars().count() <= LABEL_MAX {
        return name.to_string();
    }
    let cut: String = name.chars().take(LABEL_MAX - 1).collect();
    format!("{}…", cut.trim_end())
}

/// What a sub-tab's tooltip says a plan or a training map is. Of the
/// answer key, only how many faults: never what they are.
fn purpose_note(p: Purpose, faults: usize) -> Option<String> {
    let mut s = p.caption()?.to_string();
    if faults > 0 {
        s += &format!(
            "\nAnswer key: {faults} fault{}, hidden",
            if faults == 1 { "" } else { "s" }
        );
    }
    Some(s)
}

/// A sub-tab's label: its name cut to fit, a small tag for a plan or a
/// training map, and • when it has changes not saved.
fn tab_text(ui: &Ui, name: &str, p: Purpose, changed: bool) -> egui::WidgetText {
    let dot = if changed { " •" } else { "" };
    let Some(tag) = p.tag() else {
        return format!("{}{dot}", short(name)).into();
    };
    let mut job = egui::text::LayoutJob::default();
    let mut add = |t: RichText| {
        t.append_to(
            &mut job,
            ui.style(),
            egui::FontSelection::Default,
            egui::Align::Center,
        )
    };
    add(RichText::new(short(name)));
    add(RichText::new(format!(" {}", tag.to_uppercase()))
        .small()
        .strong()
        .color(widgets::warn()));
    if changed {
        add(RichText::new(dot));
    }
    job.into()
}

/// A sub-tab's right-click menu. `changed`: it has changes not saved.
/// `faults`: how many in its answer key.
fn tab_menu(
    ui: &mut Ui,
    i: usize,
    shown: Option<usize>,
    changed: bool,
    purpose: Purpose,
    faults: usize,
) -> Option<TabEv> {
    let mut ev = None;
    ui.menu_button(format!("What it is: {}", purpose.name()), |ui| {
        for p in Purpose::ALL {
            let tip = match p {
                Purpose::AsFound => "Found on a network: a sweep, backups, a /mapnetwork map",
                Purpose::Plan => "Drawn by hand: a network to build or change, not discovered",
                Purpose::Training => "Made up for practice, not discovered",
            };
            // An answer key makes it a training map.
            let on = p == Purpose::Training || faults == 0;
            if ui
                .add_enabled(on, egui::Button::selectable(p == purpose, p.name()))
                .on_hover_text(tip)
                .on_disabled_hover_text(format!(
                    "It has a training answer key ({faults} fault{}, hidden): it stays a \
                     training map",
                    if faults == 1 { "" } else { "s" }
                ))
                .clicked()
            {
                if p != purpose {
                    ev = Some(TabEv::Purpose(i, p));
                }
                ui.close();
            }
        }
    });
    ui.separator();
    let mut item = |ui: &mut Ui, on: bool, label: &str, tip: &str, e: TabEv| {
        if ui
            .add_enabled(on, egui::Button::new(label))
            .on_hover_text(tip)
            .on_disabled_hover_text("No changes since it was saved")
            .clicked()
        {
            ev = Some(e);
            ui.close();
        }
    };
    item(ui, true, "Rename", "Give it another name", TabEv::Rename(i));
    item(
        ui,
        changed,
        "Update with current",
        "Keep the changes made to it since it was saved (marked •)",
        TabEv::Update(i),
    );
    // Only from another map: on its own tab it would replace itself.
    if shown != Some(i) {
        let tip = match shown {
            Some(_) => "Overwrite it with the saved map showing now (asks first)",
            None => "Overwrite it with the live map as it is now (asks first)",
        };
        item(
            ui,
            true,
            "Replace with the map showing…",
            tip,
            TabEv::Replace(i),
        );
    }
    item(
        ui,
        true,
        "Duplicate",
        "A copy of it, as it is now, beside it",
        TabEv::Duplicate(i),
    );
    item(
        ui,
        true,
        "Export JSON…",
        "Save it as a file to share: Open… here (or Config Export) reads it",
        TabEv::Export(i),
    );
    ui.separator();
    item(
        ui,
        true,
        "Delete…",
        "Take it and its file away (asks first)",
        TabEv::Delete(i),
    );
    ev
}

/// How wide Saved maps is on the canvas.
const TOGGLE_W: f32 = 84.0;
/// The zoom controls' room from the canvas's right edge, and the gap
/// before Saved maps (see zoom_controls).
const ZOOM_W: f32 = 10.0 + 208.0 + 12.0;

/// Where Saved maps goes: left of the zoom controls, or above them on a
/// canvas too narrow for both in a row. None with no room at all (the
/// bar's own button and the empty map's link still reach it).
fn toggle_rect(rect: Rect) -> Option<Rect> {
    let right = rect.max.x - ZOOM_W;
    if right - TOGGLE_W >= rect.min.x + 8.0 {
        return Some(Rect::from_min_max(
            pos2(right - TOGGLE_W, rect.max.y - 32.0),
            pos2(right, rect.max.y - 10.0),
        ));
    }
    let r = Rect::from_min_max(
        pos2(rect.max.x - 10.0 - TOGGLE_W, rect.max.y - 60.0),
        pos2(rect.max.x - 10.0, rect.max.y - 38.0),
    );
    (r.min.x >= rect.min.x + 8.0 && r.min.y >= rect.min.y + 40.0).then_some(r)
}

/// Where the controls along the canvas's bottom edge start: the legend
/// stops short of it.
pub fn corner_left(rect: Rect) -> f32 {
    match toggle_rect(rect) {
        Some(r) if r.max.y > rect.max.y - 30.0 => r.min.x,
        _ => rect.max.x - ZOOM_W + 12.0,
    }
}

/// Where the controls in the canvas's bottom right corner start from the
/// top: the zoom controls, or Saved maps when it sits above them. A legend
/// laid out above them stops short of it.
pub fn corner_top(rect: Rect) -> f32 {
    let zoom = rect.max.y - 32.0;
    match toggle_rect(rect) {
        Some(r) if r.max.y <= rect.max.y - 30.0 => r.min.y.min(zoom),
        _ => zoom,
    }
}

impl MapView {
    /// Saved maps, beside the zoom controls in the canvas corner: shows or
    /// hides the bar along the bottom.
    pub(super) fn bar_toggle(&mut self, ui: &mut Ui, rect: Rect) {
        let Some(r) = toggle_rect(rect) else { return };
        if ui
            .put(
                r,
                egui::Button::new("Saved maps")
                    .selected(self.bar)
                    .corner_radius(6),
            )
            .on_hover_text(
                "Show or hide the saved maps along the bottom: keep this map as it is now and \
                 come back to it",
            )
            .clicked()
        {
            self.bar_flip = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("is-alive-tabs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn sample(name: &str, created: &str) -> Snapshot {
        let mut s = saved::sample();
        s.name = name.into();
        s.created = created.into();
        s
    }

    fn secrets() -> crate::redact::Overrides {
        crate::redact::Overrides {
            redact: vec!["4471".into()],
            allow: vec![],
            known: vec!["Bobcat2019".into()],
        }
    }

    /// A saved map's file holds its notes redacted as the Notes tab
    /// redacts now: a file opened from elsewhere at once, and a value
    /// redacted later (by hand, or a backup's secret loaded) is rewritten
    /// in the file at once, leaving its unsaved changes unsaved.
    #[test]
    fn saved_maps_files_are_redacted_when_the_redaction_changes() {
        let dir = temp("redact");
        let mut t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        t.redact(&crate::redact::Overrides {
            known: vec!["Bobcat2019".into()],
            ..Default::default()
        });
        let mut s = sample("Received", "2026-01-01T00:00:00Z");
        s.edits
            .notes
            .insert("sw".into(), "door code 4471, admin Bobcat2019".into());
        assert!(t.keep_new(s).is_ok());
        let path = t.saved[0].path.clone();
        let file = || std::fs::read_to_string(&path).unwrap();
        assert!(!file().contains("Bobcat2019"), "opened: redacted as here");
        assert!(file().contains("4471"));
        // An unsaved change, then 4471 redacted by hand.
        t.saved[0].view.kept.push("gw".into());
        let f = file();
        t.redact(&secrets());
        let after = file();
        assert!(
            !after.contains("4471") && after.contains("door code"),
            "{after}"
        );
        assert_ne!(f, after);
        assert!(
            t.saved[0].view.edits_differ(&t.saved[0].saved),
            "the pin still unsaved"
        );
        assert_eq!(
            saved::read(&path).unwrap().edits.kept,
            t.saved[0].saved.kept
        );
    }

    /// The Site Bundle's card notes: redacted, without emoji style marks,
    /// and none from an example.
    #[test]
    fn the_bundles_card_notes_are_redacted() {
        let mut live = MapView::new(None);
        live.map.nodes = saved::sample().map.nodes;
        live.notes.insert(
            "sw".into(),
            "door code 4471\nadmin Bobcat2019 \u{26A0}\u{FE0F}\u{FE02}".into(),
        );
        let t = MapTabs::in_dir(live, None);
        let (n, json) = t.bundle_notes(&secrets()).expect("a note");
        assert_eq!(n, 1);
        assert!(
            !json.contains("4471") && !json.contains("Bobcat2019"),
            "{json}"
        );
        assert!(json.contains("REDACTED: by hand") && json.contains("REDACTED: from config"));
        assert!(!json.contains('\u{FE02}') && json.contains('\u{26A0}'));
        let mut t = t;
        t.live.note_scope = super::super::notes::Scope::Example("example-Acme.dc.html".into());
        assert!(t.bundle_notes(&secrets()).is_none());
    }

    #[test]
    fn saved_maps_come_back_after_a_restart_renamed_and_deleted() {
        let dir = temp("persist");
        let mut t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        assert!(t.saved.is_empty() && !t.bar);
        assert!(
            t.keep_new(sample("Head office", "2026-01-01T00:00:00Z"))
                .is_ok()
        );
        assert!(t.keep_new(sample("Branch", "2026-01-02T00:00:00Z")).is_ok());
        assert_eq!(t.shown, Some(1));
        assert!(t.bar);
        t.rename(0, "HQ main".into());
        assert_eq!(t.saved[0].path, dir.join("hq-main.json"));
        assert!(!dir.join("head-office.json").exists());
        // A restart: both back, in the order saved, the rename kept.
        let mut t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        let names: Vec<&str> = t.saved.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["HQ main", "Branch"]);
        assert!(t.bar, "the bar shows once a map is saved");
        t.shown = Some(1);
        t.delete(0);
        assert_eq!(t.shown, Some(0), "still showing Branch");
        assert!(!dir.join("hq-main.json").exists());
        let t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        assert_eq!(t.saved.len(), 1);
        assert_eq!(t.saved[0].name, "Branch");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_map_keeps_its_changes_once_updated() {
        let dir = temp("update");
        let mut t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        t.keep_new(sample("Site", "2026-01-01T00:00:00Z")).unwrap();
        let v = &mut t.saved[0].view;
        v.refresh_saved(&egui::Context::default());
        v.placed.insert("gw".into(), pos2(5.0, 5.0));
        assert!(
            t.saved[0].view.edits_differ(&t.saved[0].saved),
            "shown with a •"
        );
        // Live showing: Update with current still keeps the saved map's own
        // changes, not the live map.
        t.shown = None;
        t.update(0);
        assert!(!t.saved[0].view.edits_differ(&t.saved[0].saved));
        let back = saved::read(&t.saved[0].path).unwrap();
        assert_eq!(back.edits.placed.get("gw"), Some(&pos2(5.0, 5.0)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_maps_stays_on_the_canvas_clear_of_the_legend() {
        let wide = Rect::from_min_size(pos2(240.0, 0.0), vec2(800.0, 500.0));
        let r = toggle_rect(wide).unwrap();
        assert_eq!(r.max.x, wide.max.x - ZOOM_W);
        assert_eq!(corner_left(wide), r.min.x);
        // 250 px: above the zoom controls, inside the canvas.
        let narrow = Rect::from_min_size(pos2(240.0, 0.0), vec2(250.0, 500.0));
        let r = toggle_rect(narrow).unwrap();
        assert!(r.min.x >= narrow.min.x && r.max.y < narrow.max.y - 32.0);
        assert_eq!(corner_left(narrow), narrow.max.x - ZOOM_W + 12.0);
        // A legend above the corner's controls stays above Saved maps too.
        assert!(corner_top(narrow) <= r.min.y);
        assert_eq!(corner_top(wide), wide.max.y - 32.0);
        let tiny = Rect::from_min_size(pos2(240.0, 0.0), vec2(60.0, 500.0));
        assert!(toggle_rect(tiny).is_none());
    }

    #[test]
    fn a_file_that_isnt_a_saved_map_is_refused() {
        let mut t = MapTabs::in_dir(MapView::new(None), None);
        assert!(
            t.open_file(Path::new("x.json"), b"{\"format\": \"other\"}")
                .is_err()
        );
        assert!(t.saved.is_empty());
        // No settings folder: nowhere to keep it.
        let good = saved::to_json(&saved::sample());
        assert!(t.open_file(Path::new("x.json"), good.as_bytes()).is_err());
    }

    #[test]
    fn maps_with_the_same_name_are_told_apart() {
        let dir = temp("names");
        let mut t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        for _ in 0..3 {
            t.keep_new(sample("Acme tree", "2026-01-01T00:00:00Z"))
                .unwrap();
        }
        let names: Vec<&str> = t.saved.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Acme tree", "Acme tree (2)", "Acme tree (3)"]);
        // Renamed to a name taken: a number too; its own name is free.
        assert_eq!(t.unique_name("Acme tree", Some(1)), "Acme tree (2)");
        assert_eq!(t.unique_name("Acme tree (3)", Some(2)), "Acme tree (3)");
        assert_eq!(short("Short"), "Short");
        let long = short(&"x".repeat(80));
        assert_eq!(long.chars().count(), LABEL_MAX);
        assert!(long.ends_with('…'));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_a_saved_map_is_is_kept_after_a_restart() {
        let dir = temp("purpose");
        let mut t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        t.keep_new(sample("Site", "2026-01-01T00:00:00Z")).unwrap();
        assert_eq!(t.saved[0].view.purpose, Purpose::AsFound);
        // An unsaved change stays unsaved.
        t.saved[0].view.placed.insert("gw".into(), pos2(5.0, 5.0));
        t.set_purpose(0, Purpose::Plan);
        assert_eq!(t.saved[0].view.purpose, Purpose::Plan);
        let t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        assert_eq!(t.saved[0].view.purpose, Purpose::Plan);
        assert!(!t.saved[0].saved.placed.contains_key("gw"));
        assert_eq!(purpose_note(Purpose::AsFound, 0), None);
        assert_eq!(
            purpose_note(Purpose::Plan, 0).as_deref(),
            Some("Plan — drawn by hand, not discovered")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shared_training_map_opens_as_one_and_stays_one() {
        let dir = temp("training");
        let mut t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        let mut s = sample("Practice", "2026-01-01T00:00:00Z");
        s.purpose = Purpose::Training;
        s.faults = vec![saved::Fault {
            card: Some("sw".into()),
            title: "Secret loop".into(),
            detail: "on port 7".into(),
        }];
        let text = saved::to_json(&s);
        t.open_file(Path::new("practice.json"), text.as_bytes())
            .unwrap();
        let v = &t.saved[0].view;
        assert_eq!((v.purpose, v.faults.len()), (Purpose::Training, 1));
        let note = purpose_note(v.purpose, v.faults.len()).unwrap();
        assert!(note.ends_with("Answer key: 1 fault, hidden"), "{note}");
        assert!(!note.contains("Secret") && !note.contains("port 7"));
        // With an answer key it can't be called as found (or a plan).
        t.set_purpose(0, Purpose::AsFound);
        t.set_purpose(0, Purpose::Plan);
        assert_eq!(t.saved[0].view.purpose, Purpose::Training);
        // Kept through a rename, an update and a restart.
        t.rename(0, "Practice 2".into());
        t.update(0);
        let t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        let v = &t.saved[0].view;
        assert_eq!((v.purpose, v.faults.len()), (Purpose::Training, 1));
        assert_eq!(v.faults[0].title, "Secret loop");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_that_couldnt_be_read_are_kept_to_be_told() {
        let dir = temp("skipped");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("garbage.json"), "not json").unwrap();
        let t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        assert_eq!(t.skipped.len(), 1);
        assert!(t.skipped[0].starts_with("garbage.json"));
        assert!(t.bar && !t.told, "the bar shows the note");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
