//! The Map tab with its saved maps: the live map (from the sweep and the
//! files loaded) and maps saved as they were, each a sub-tab in a bar
//! along the bottom. The bar is hidden until a map is saved or the
//! Saved maps by the zoom controls opens it.

use super::history::Edits;
use super::saved::{self, Snapshot};
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
    Duplicate(usize),
    Export(usize),
    Delete(usize),
}

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
    /// The sub-tab Delete asked about (the dialog is open).
    deleting: Option<usize>,
    /// Open…'s file dialog, then the file read.
    opening: Option<Job<(PathBuf, Result<Snapshot, String>)>>,
}

impl MapTabs {
    /// The live map, and the maps saved before (not for the screenshot
    /// tour: its pictures show the map as a new user sees it).
    pub fn new(mode: Option<Mode>) -> Self {
        let tour = std::env::var_os("IS_ALIVE_SHOTS").is_some();
        let dir = saved::folder().filter(|_| !tour);
        Self::in_dir(MapView::new(mode), dir)
    }

    fn in_dir(live: MapView, dir: Option<PathBuf>) -> Self {
        let (found, errs) = dir.as_deref().map(saved::list).unwrap_or_default();
        if !errs.is_empty() {
            widgets::toast(
                format!(
                    "Couldn't read {} saved map{}: {}",
                    errs.len(),
                    if errs.len() == 1 { "" } else { "s" },
                    errs.join("; ")
                ),
                false,
            );
        }
        let saved: Vec<Tab> = found.into_iter().map(|(p, s)| tab(p, s)).collect();
        Self {
            live,
            bar: !saved.is_empty(),
            saved,
            shown: None,
            dir,
            renaming: None,
            deleting: None,
            opening: None,
        }
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

    /// A saved map handed over by Config Export (opened or dropped): kept
    /// with the others and shown. False when it couldn't be read.
    pub fn open_file(&mut self, path: &Path, data: &[u8]) -> bool {
        let name = crate::tools::export::file_name(path);
        match saved::parse(data) {
            Ok(s) => self.add(s, &name),
            Err(e) => {
                widgets::toast(format!("Couldn't open {name}: {e}"), false);
                false
            }
        }
    }

    /// Keep a map read from a file as a new saved map, and show it.
    fn add(&mut self, mut s: Snapshot, from: &str) -> bool {
        s.created = saved::now();
        let ok = self.keep_new(s);
        if ok {
            widgets::toast(format!("Opened {from} as a saved map"), true);
        }
        ok
    }

    /// Write `s` as a new saved map, add its sub-tab and show it.
    fn keep_new(&mut self, s: Snapshot) -> bool {
        let Some(dir) = &self.dir else {
            widgets::toast("No settings folder to keep saved maps in", false);
            return false;
        };
        let path = saved::fresh_path(dir, &s.name);
        if let Err(e) = saved::write(&path, &s) {
            widgets::toast(format!("Couldn't save the map: {e}"), false);
            return false;
        }
        self.saved.push(tab(path, s));
        self.shown = Some(self.saved.len() - 1);
        self.bar = true;
        true
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
                Some(v.snapshot(name, created, full))
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
        if self.bar {
            egui::Panel::bottom("map_saved_bar")
                .resizable(false)
                .show(ui, |ui| self.bar_ui(ui, &src));
        }
        self.delete_dialog(ui.ctx());
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
                ui.separator();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
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
            if let Some(s) = self.snapshot_shown(src, &name, saved::now())
                && self.keep_new(s)
            {
                widgets::toast(
                    format!("Saved \"{name}\": right-click its tab to rename it"),
                    true,
                );
                self.renaming = self.shown.map(|i| (i, self.saved[i].name.clone()));
            }
        }
        match ev {
            Some(TabEv::Rename(i)) => self.renaming = Some((i, self.saved[i].name.clone())),
            Some(TabEv::Update(i)) => self.update(i, src),
            Some(TabEv::Duplicate(i)) => {
                let t = &self.saved[i];
                let name = format!("{} copy", t.name);
                let full = t.view.frozen.as_deref().map(saved::copy_map);
                if let Some(full) = full {
                    let s = t.view.snapshot(&name, saved::now(), full);
                    self.keep_new(s);
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
            Some(TabEv::Delete(i)) => self.deleting = Some(i),
            None => {}
        }
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
            let label = if changed {
                format!("{} •", t.name)
            } else {
                t.name.clone()
            };
            let tip = if changed {
                "Changed since it was saved: right-click › Update with current keeps the changes"
            } else {
                "A saved map. Right-click to rename, update, duplicate, export or delete it"
            };
            let r = ui
                .selectable_label(self.shown == Some(i), label)
                .on_hover_text(tip);
            if r.clicked() {
                self.shown = Some(i);
            }
            r.context_menu(|ui| {
                ev = tab_menu(ui, i, self.shown);
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
                self.rename(i, name);
            }
        }
    }

    /// Rename a saved map: its file takes the new name too. What's in the
    /// file stays as it was (unsaved changes aren't written).
    fn rename(&mut self, i: usize, name: String) {
        let Some(dir) = self.dir.clone() else { return };
        let t = &self.saved[i];
        let mut s = match saved::read(&t.path) {
            Ok(s) => s,
            // The file went: write what's showing.
            Err(_) => match t.view.frozen.as_deref() {
                Some(full) => t
                    .view
                    .snapshot(&t.name, t.created.clone(), saved::copy_map(full)),
                None => return,
            },
        };
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

    /// Update with current: saved map `i` becomes the map showing (its own
    /// changes, or Live's map, or another saved one).
    fn update(&mut self, i: usize, src: &Sources) {
        let (name, created) = (self.saved[i].name.clone(), self.saved[i].created.clone());
        let Some(s) = self.snapshot_shown(src, &name, created) else {
            widgets::toast("Nothing on the map to save", false);
            return;
        };
        if let Err(e) = saved::write(&self.saved[i].path, &s) {
            widgets::toast(format!("Couldn't save the map: {e}"), false);
            return;
        }
        let t = &mut self.saved[i];
        if self.shown == Some(i) {
            // Its own changes: the view (and its undo) carries on.
            t.saved = s.edits;
        } else {
            let path = t.path.clone();
            *t = tab(path, s);
        }
        widgets::toast(format!("Updated \"{name}\""), true);
    }

    /// Delete…: ask first, then take the map and its file away.
    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(i) = self.deleting.filter(|&i| i < self.saved.len()) else {
            self.deleting = None;
            return;
        };
        let mut gone = false;
        let r = egui::Modal::new(egui::Id::new("map_delete")).show(ctx, |ui| {
            ui.set_max_width(320.0);
            ui.strong(format!("Delete \"{}\"?", self.saved[i].name));
            ui.label("Its file is deleted too. This can't be undone.");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Delete").color(widgets::bad()))
                    .clicked()
                {
                    gone = true;
                    ui.close();
                }
                if ui.button("Cancel").clicked() {
                    ui.close();
                }
            });
        });
        if gone {
            self.delete(i);
        }
        if r.should_close() {
            self.deleting = None;
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
        self.deleting = None;
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
            match r {
                Ok(s) => {
                    self.add(s, &name);
                }
                Err(e) => widgets::toast(format!("Couldn't open {name}: {e}"), false),
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

/// A sub-tab's right-click menu.
fn tab_menu(ui: &mut Ui, i: usize, shown: Option<usize>) -> Option<TabEv> {
    let mut ev = None;
    let mut item = |ui: &mut Ui, label: &str, tip: &str, e: TabEv| {
        if ui.button(label).on_hover_text(tip).clicked() {
            ev = Some(e);
            ui.close();
        }
    };
    item(ui, "Rename", "Give it another name", TabEv::Rename(i));
    let update_tip = match shown {
        Some(k) if k == i => "Keep the changes made to it since it was saved",
        Some(_) => "Replace it with the saved map showing now",
        None => "Replace it with the live map as it is now",
    };
    item(ui, "Update with current", update_tip, TabEv::Update(i));
    item(
        ui,
        "Duplicate",
        "A copy of it, as it is now, beside it",
        TabEv::Duplicate(i),
    );
    item(
        ui,
        "Export JSON…",
        "Save it as a file to share: Open… here (or Config Export) reads it",
        TabEv::Export(i),
    );
    ui.separator();
    item(ui, "Delete…", "Take it and its file away", TabEv::Delete(i));
    ev
}

/// How wide Saved maps is on the canvas, with its gap.
pub const TOGGLE_W: f32 = 92.0;

impl MapView {
    /// Saved maps, beside the zoom controls in the canvas corner: shows or
    /// hides the bar along the bottom.
    pub(super) fn bar_toggle(&mut self, ui: &mut Ui, rect: Rect) {
        // Left of − 100% + Center Fit (see zoom_controls).
        let right = rect.max.x - 10.0 - 208.0 - 12.0;
        let r = Rect::from_min_max(
            pos2(right - (TOGGLE_W - 8.0), rect.max.y - 32.0),
            pos2(right, rect.max.y - 10.0),
        );
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

    #[test]
    fn saved_maps_come_back_after_a_restart_renamed_and_deleted() {
        let dir = temp("persist");
        let mut t = MapTabs::in_dir(MapView::new(None), Some(dir.clone()));
        assert!(t.saved.is_empty() && !t.bar);
        assert!(t.keep_new(sample("Head office", "2026-01-01T00:00:00Z")));
        assert!(t.keep_new(sample("Branch", "2026-01-02T00:00:00Z")));
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
        t.keep_new(sample("Site", "2026-01-01T00:00:00Z"));
        let v = &mut t.saved[0].view;
        v.refresh_saved(&egui::Context::default());
        v.placed.insert("gw".into(), pos2(5.0, 5.0));
        assert!(
            t.saved[0].view.edits_differ(&t.saved[0].saved),
            "shown with a •"
        );
        let info = HostInfo::default();
        let (sweep, export) = (SweepTool::default(), ExportTool::default());
        let src = Sources {
            sweep: &sweep,
            export: &export,
            info: &info,
            busy: false,
            note: false,
        };
        t.update(0, &src);
        assert!(!t.saved[0].view.edits_differ(&t.saved[0].saved));
        let back = saved::read(&t.saved[0].path).unwrap();
        assert_eq!(back.edits.placed.get("gw"), Some(&pos2(5.0, 5.0)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_isnt_a_saved_map_is_refused() {
        let mut t = MapTabs::in_dir(MapView::new(None), None);
        assert!(!t.open_file(Path::new("x.json"), b"{\"format\": \"other\"}"));
        assert!(t.saved.is_empty());
        // No settings folder: nowhere to keep it.
        let good = saved::to_json(&saved::sample());
        assert!(!t.open_file(Path::new("x.json"), good.as_bytes()));
    }
}
