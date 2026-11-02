//! Notes on cards: a few lines the technician writes on a card (or on
//! several at once), shown as a small note on the card's top edge, in its
//! tooltip and its panel, found by search, and carried by Copy details,
//! the exports, saved maps and the Site Bundle. Each change is a step to
//! undo.
//!
//! Redaction is the Notes tab's own (`redact::find_with`, one shared
//! `Overrides`): what the technician redacts by hand and the secret values
//! of the backups and exports loaded in Config Export, less the values they
//! chose to keep (the same keep list as the Notes tab's). A note is kept as
//! typed only in memory; whatever leaves the map (a file, an export, a
//! copy, the tooltip) is the redacted text, as with notes.md.
//!
//! Where a note is kept, so it never lands on another network's card
//! (`ip:10.0.0.1` is at every customer, and a /mapnetwork file names its
//! cards as it likes):
//! - a saved map: in its file, on the card (an unsaved change until the
//!   map is updated, like its other changes);
//! - an example: on this computer, by the example and the card;
//! - the live map: on this computer, by the card's hardware address. A
//!   vendor-assigned MAC is one device wherever it shows up, so its note
//!   follows it and nothing else. A card without one (no MAC, a randomized
//!   or shared one, this machine) keeps its note only until is-alive
//!   closes or another map starts, and the panel says so: saving the map
//!   keeps it.

use super::*;
use crate::input::Field;
use crate::redact::{self, Hit, Pick};
use serde_json::{Map as Obj, Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// What a map's notes belong to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scope {
    /// A saved map: its file keeps them.
    #[default]
    Saved,
    /// The live map: kept by device.
    Live,
    /// An example (its file's name): kept by example and card.
    Example(String),
}

/// The live map's store's file in the profile folder.
const STORE_FILE: &str = "map-notes.json";
const STORE_FORMAT: &str = "is-alive-card-notes";
/// Most notes kept on this computer; the oldest go first.
const STORE_MAX: usize = 5_000;
/// Largest store file read.
const STORE_FILE_MAX: u64 = 16 << 20;
/// Longest key read back.
const KEY_MAX: usize = 600;
/// Lines of a note in a card's tooltip.
const TIP_LINES: usize = 3;

/// True for a MAC that is one device wherever it's seen: assigned by its
/// vendor (not randomized or set by hand), not multicast, and not one of
/// the shared addresses routers stand in for each other with (VRRP, HSRP,
/// GLBP), which are the same at every site.
pub fn device_mac(m: &crate::net::mac::Mac) -> bool {
    let vrrp = m[..4] == [0x00, 0x00, 0x5e, 0x00] && matches!(m[4], 0x01 | 0x02);
    let hsrp = m[..5] == [0x00, 0x00, 0x0c, 0x07, 0xac]
        || (m[..4] == [0x00, 0x00, 0x0c, 0x9f] && m[4] & 0xf0 == 0xf0);
    let glbp = m[..5] == [0x00, 0x07, 0xb4, 0x00, 0x01];
    let zero = m[..5] == [0; 5];
    !crate::net::mac::local_admin(m) && m[0] & 1 == 0 && !(vrrp || hsrp || glbp || zero)
}

/// Where card `n`'s note is kept on this computer, or None when it isn't
/// (a saved map's file keeps it, or the card has nothing to know it by on
/// another visit).
pub fn key(scope: &Scope, n: &Node) -> Option<String> {
    match scope {
        Scope::Saved => None,
        Scope::Example(f) => Some(format!("example:{f}/{}", n.id)),
        Scope::Live => n
            .mac
            .filter(device_mac)
            // This laptop goes to every site: what's written on it is
            // about one of them.
            .filter(|_| n.kind != "THIS MACHINE")
            .map(|m| format!("mac:{}", crate::net::mac::fmt(&m))),
    }
}

/// `text` as it may leave the map: what the technician asked to hide
/// replaced with markers.
pub fn clean(text: &str, ov: &redact::Overrides) -> String {
    redact::sanitize_with(text, ov).0
}

/// A note's first lines for a tooltip, each cut short.
pub fn first_lines(note: &str) -> String {
    let lines: Vec<&str> = note.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out: Vec<String> = lines
        .iter()
        .take(TIP_LINES)
        .map(|l| interact::ellipsis(l.trim(), 80))
        .collect();
    if lines.len() > TIP_LINES {
        out.push("…".into());
    }
    out.join("\n")
}

/// Notes kept on this computer for the live map and the examples: the
/// redacted text, by `key`, with when each was written.
#[derive(Default)]
pub struct Store {
    path: Option<PathBuf>,
    notes: BTreeMap<String, (String, String)>,
}

impl Store {
    /// The profile folder's store (none for the screenshot tour, which
    /// shows the map as a new user sees it).
    pub fn open() -> Self {
        let tour = std::env::var_os("IS_ALIVE_SHOTS").is_some();
        let path = crate::theme::data_dir()
            .filter(|_| !tour)
            .map(|d| d.join(STORE_FILE));
        Self::at(path)
    }

    /// The store in `path`, read back defensively: a size cap, every note
    /// cleaned and cut as the editor would, at most `STORE_MAX` (the
    /// newest). A file that isn't one reads as empty.
    pub fn at(path: Option<PathBuf>) -> Self {
        let mut s = Store {
            path,
            notes: BTreeMap::new(),
        };
        let Some(text) = s
            .path
            .as_deref()
            .and_then(|p| crate::input::read_capped(p, STORE_FILE_MAX))
        else {
            return s;
        };
        let Ok(doc) = serde_json::from_str::<Value>(&text) else {
            return s;
        };
        if doc.get("format").and_then(Value::as_str) != Some(STORE_FORMAT) {
            return s;
        }
        let Some(all) = doc.get("notes").and_then(Value::as_object) else {
            return s;
        };
        for (k, v) in all {
            let k = crate::input::line(k, KEY_MAX);
            let note = Field::CardNote.clean(v.get("note").and_then(Value::as_str).unwrap_or(""));
            let at = crate::input::line(v.get("at").and_then(Value::as_str).unwrap_or(""), 40);
            if !k.is_empty() && !note.trim().is_empty() {
                s.notes.insert(k, (note, at));
            }
        }
        s.trim();
        s
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.notes.get(key).map(|(n, _)| n.as_str())
    }

    /// Keep `note` under `key`; true when that changed anything.
    fn put(&mut self, key: &str, note: String) -> bool {
        if self.get(key) == Some(note.as_str()) {
            return false;
        }
        self.notes.insert(key.to_string(), (note, saved::now()));
        self.trim();
        true
    }

    fn remove(&mut self, key: &str) -> bool {
        self.notes.remove(key).is_some()
    }

    /// At most `STORE_MAX`: the oldest go.
    fn trim(&mut self) {
        while self.notes.len() > STORE_MAX {
            let oldest = self
                .notes
                .iter()
                .min_by(|a, b| (&a.1.1, a.0).cmp(&(&b.1.1, b.0)))
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => self.notes.remove(&k),
                None => break,
            };
        }
    }

    /// Every note's text (for the keep list, which keeps values still in
    /// use).
    pub fn texts(&self) -> impl Iterator<Item = &str> {
        self.notes.values().map(|(n, _)| n.as_str())
    }

    /// Write the store, whole or not at all; the file goes once it's empty.
    fn save(&self) -> Result<(), String> {
        let Some(p) = &self.path else { return Ok(()) };
        if self.notes.is_empty() {
            return match std::fs::remove_file(p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        let notes: Obj<String, Value> = self
            .notes
            .iter()
            .map(|(k, (n, at))| (k.clone(), json!({"note": n, "at": at})))
            .collect();
        let doc = json!({"format": STORE_FORMAT, "version": 1, "notes": notes});
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = p.with_extension("json.tmp");
        crate::theme::write_private(&tmp, serde_json::to_string_pretty(&doc).unwrap_or_default())
            .and_then(|_| std::fs::rename(&tmp, p))
            .map_err(|e| e.to_string())
    }
}

/// What was last written to the store: each card's key, and a hash of
/// the notes and the redaction it was written with.
#[derive(Default)]
pub(super) struct Synced {
    hash: u64,
    redact: u64,
    keys: BTreeMap<String, String>,
}

/// A note being written in a card's panel.
pub(super) struct Edit {
    /// The cards it's for, by id: the first is the one whose panel shows
    /// it.
    pub ids: Vec<String>,
    pub text: String,
    /// Just opened: scroll the panel to it and give it the keyboard.
    pub fresh: bool,
}

/// What a card's note section asked for.
pub(super) enum NoteEv {
    Open(usize),
    Save,
    Cancel,
    Remove(usize),
    /// Redact or keep a value, everywhere (the Notes tab's lists too).
    Pick(Pick),
}

fn hash_of(t: &impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

/// The editor's id (one editor at a time).
fn editor_id() -> egui::Id {
    egui::Id::new("map_card_note_editor")
}

impl MapView {
    /// Put each card's redacted note on it, when the notes, the redaction
    /// or the map changed.
    pub(super) fn stamp_notes(&mut self) {
        let h = hash_of(&(&self.notes, &self.ov, self.built));
        if h == self.stamped {
            return;
        }
        self.stamped = h;
        let (notes, ov) = (&self.notes, &self.ov);
        for n in &mut self.map.nodes {
            match notes.get(&n.id) {
                Some(t) => n.note = clean(t, ov),
                None => n.note.clear(),
            }
        }
    }

    /// Forget the notes kept only in memory, as another map starts; the
    /// kept ones come back from the store.
    pub(super) fn drop_notes(&mut self) {
        self.notes.clear();
        self.note_synced = Synced::default();
        self.note_edit = None;
    }

    /// After a rebuild: the notes the store keeps for cards now on the map.
    /// Not a step to undo.
    pub(super) fn refill_notes(&mut self) {
        let Some(store) = &self.note_store else {
            return;
        };
        for n in &self.map.nodes {
            if self.notes.contains_key(&n.id) {
                continue;
            }
            if let Some(k) = key(&self.note_scope, n)
                && let Some(t) = store.get(&k)
            {
                self.notes.insert(n.id.clone(), t.to_string());
                self.note_synced.keys.insert(n.id.clone(), k);
            }
        }
    }

    /// Once a frame on the live map: notes written, changed or taken off
    /// go to the store, redacted. A change to the redaction (a value kept,
    /// a backup's secrets loaded) is written through to every note there.
    pub(super) fn sync_notes(&mut self) {
        let Some(store) = &mut self.note_store else {
            return;
        };
        let red = hash_of(&self.ov);
        // After a rebuild too: a card may have a MAC to be kept by now.
        let h = hash_of(&(&self.notes, red, &self.note_scope, self.built));
        if h == self.note_synced.hash {
            return;
        }
        let mut changed = false;
        if red != self.note_synced.redact {
            for (n, _) in store.notes.values_mut() {
                let c = clean(n, &self.ov);
                if c != *n {
                    *n = c;
                    changed = true;
                }
            }
        }
        let mut keys = BTreeMap::new();
        for (id, text) in &self.notes {
            let k = self
                .map
                .nodes
                .iter()
                .find(|n| n.id == *id)
                .and_then(|n| key(&self.note_scope, n))
                .or_else(|| self.note_synced.keys.get(id).cloned());
            if let Some(k) = k {
                changed |= store.put(&k, clean(text, &self.ov));
                keys.insert(id.clone(), k);
            }
        }
        for (id, k) in &self.note_synced.keys {
            if !self.notes.contains_key(id) && !keys.values().any(|v| v == k) {
                changed |= store.remove(k);
            }
        }
        if changed && let Err(e) = store.save() {
            widgets::toast(format!("Couldn't keep the card notes: {e}"), false);
        }
        self.note_synced = Synced {
            hash: h,
            redact: red,
            keys,
        };
    }

    /// Every note's text this map holds, keeps or is being written (for
    /// the keep list).
    pub(super) fn note_texts(&self) -> Vec<String> {
        self.notes
            .values()
            .cloned()
            .chain(self.note_edit.iter().map(|e| e.text.clone()))
            .chain(
                self.note_store
                    .iter()
                    .flat_map(|s| s.texts().map(str::to_string)),
            )
            .collect()
    }

    /// Start writing a note on `cards` (the first one's panel shows the
    /// editor, and it joins the selection if it isn't in it). The text
    /// they share, if they all have the same; else empty.
    pub(super) fn open_note(&mut self, cards: &[usize]) {
        let Some(&first) = cards.first() else { return };
        if !self.selection().contains(&first) {
            if self.selection().is_empty() {
                self.select(Some(first));
            } else {
                self.toggle(first);
            }
        }
        let ids: Vec<String> = cards
            .iter()
            .map(|&k| self.map.nodes[k].id.clone())
            .collect();
        let texts: Vec<Option<&String>> = ids.iter().map(|id| self.notes.get(id)).collect();
        let text = match texts.first() {
            Some(Some(t)) if texts.iter().all(|x| *x == Some(*t)) => (*t).clone(),
            _ => String::new(),
        };
        self.note_edit = Some(Edit {
            ids,
            text,
            fresh: true,
        });
    }

    /// Save the note being written on each of its cards (an empty one
    /// takes theirs off): one step to undo.
    pub(super) fn save_note(&mut self) {
        let Some(e) = self.note_edit.take() else {
            return;
        };
        let text = Field::CardNote.clean(e.text.trim_end());
        let text = text.trim_start_matches('\n').to_string();
        for id in e.ids {
            if text.trim().is_empty() {
                self.notes.remove(&id);
            } else {
                self.notes.insert(id, text.clone());
            }
        }
    }

    /// Take the notes off `cards`.
    pub(super) fn remove_notes(&mut self, cards: &[usize]) {
        for &k in cards {
            let id = &self.map.nodes[k].id;
            self.notes.remove(id);
            if self.note_edit.as_ref().is_some_and(|e| e.ids.contains(id)) {
                self.note_edit = None;
            }
        }
    }

    /// The note on card `i` as typed, if it has one.
    pub(super) fn note_of(&self, i: usize) -> Option<&String> {
        self.notes.get(&self.map.nodes[i].id)
    }

    /// Where card `i`'s note is kept, in a line for its panel.
    pub(super) fn note_kept(&self, i: usize) -> &'static str {
        match (&self.note_scope, self.note_store.is_some()) {
            (Scope::Saved, _) => {
                "Kept in this saved map's file once you update it (right-click its tab)"
            }
            (_, false) => "Kept until is-alive closes",
            (Scope::Example(_), true) => "Kept on this computer, with this example",
            (Scope::Live, true) => match key(&self.note_scope, &self.map.nodes[i]) {
                Some(_) => {
                    "Kept on this computer with this device (by its MAC), on any map that \
                     shows it"
                }
                None => {
                    "Kept only until is-alive closes or another map starts: this card has no \
                     hardware address to know it by next time. Save the map (+ Save current) \
                     to keep its note"
                }
            },
        }
    }

    /// The card menu's note items: Add note… or Edit note… and Remove
    /// note on one card; on several, Add note to N cards (the same note on
    /// each) and Remove notes when some have one.
    pub(super) fn note_items(&self, group: &[usize]) -> Vec<(String, &'static str, bool)> {
        let n = group.len();
        let noted = group.iter().filter(|&&k| self.note_of(k).is_some()).count();
        let mut out = Vec::new();
        if n == 1 {
            if noted == 1 {
                out.push(("Edit note…".to_string(), "Change its note (N)", false));
                out.push(("Remove note".to_string(), "Take its note off", true));
            } else {
                out.push((
                    "Add note…".to_string(),
                    "Write a note on this card (N)",
                    false,
                ));
            }
            return out;
        }
        let tip = if noted > 0 {
            "The same note on each; it takes the place of the ones they have"
        } else {
            "The same note on each"
        };
        out.push((format!("Add note to {n} cards…"), tip, false));
        if noted > 0 {
            let label = if noted == 1 {
                "Remove note".to_string()
            } else {
                format!("Remove notes ({noted})")
            };
            out.push((label, "Take their notes off", true));
        }
        out
    }

    /// Card `i`'s note in its panel: the note (highlighting what's
    /// redacted wherever it goes), Edit and Remove, and where it's kept;
    /// or the editor, when it's being written. Nothing when it has none.
    pub(super) fn note_section(
        &self,
        ui: &mut Ui,
        i: usize,
        edit: &mut Option<Edit>,
        ev: &mut Option<NoteEv>,
    ) {
        let n = &self.map.nodes[i];
        let editing = edit.as_ref().is_some_and(|e| e.ids.first() == Some(&n.id));
        let note = self.note_of(i);
        if !editing && note.is_none() {
            return;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let title = match edit.as_ref().filter(|_| editing).map(|e| e.ids.len()) {
                Some(k) if k > 1 => format!("NOTE · {k} CARDS"),
                _ => "NOTE".to_string(),
            };
            let (r, _) = ui.allocate_exact_size(vec2(9.0, 11.0), Sense::hover());
            paint::note_glyph(ui.painter(), ui.visuals(), r, 1.0);
            ui.label(
                RichText::new(title)
                    .small()
                    .strong()
                    .color(ui.visuals().weak_text_color()),
            );
        });
        if editing && let Some(e) = edit.as_mut() {
            self.note_editor(ui, i, e, ev);
            return;
        }
        let Some(note) = note else { return };
        let hits = redact::find_with(note, &self.ov);
        let font = egui::TextStyle::Body.resolve(ui.style());
        let spans: Vec<_> = hits.iter().map(|h| h.span.clone()).collect();
        let mut job = crate::tools::notes::highlight(ui, note, &spans, &font);
        job.wrap.max_width = ui.available_width();
        ui.add(egui::Label::new(job).wrap().selectable(true));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui
                .small_button("Edit")
                .on_hover_text("Change it (N)")
                .clicked()
            {
                *ev = Some(NoteEv::Open(i));
            }
            if ui
                .small_button("Remove")
                .on_hover_text("Take the note off")
                .clicked()
            {
                *ev = Some(NoteEv::Remove(i));
            }
            hits_badge(ui, &hits);
        });
        ui.add(egui::Label::new(RichText::new(self.note_kept(i)).small().weak()).wrap());
    }

    /// The note editor: several lines, what will be redacted highlighted,
    /// Redact or Unredact on a selection (Ctrl+R, or right-click), Save
    /// (Ctrl+Enter) and Cancel (Esc).
    fn note_editor(&self, ui: &mut Ui, i: usize, e: &mut Edit, ev: &mut Option<NoteEv>) {
        let ctx = ui.ctx().clone();
        let id = editor_id();
        let focused = ctx.memory(|m| m.has_focus(id));
        // Before the box sees them: Ctrl+Enter would add a line.
        if focused {
            let (save, red) = ui.input_mut(|inp| {
                (
                    inp.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
                    inp.consume_key(egui::Modifiers::COMMAND, egui::Key::R),
                )
            });
            if save {
                *ev = Some(NoteEv::Save);
            }
            if red && let Some(p) = self.note_pick(&ctx, &e.text) {
                *ev = Some(NoteEv::Pick(p));
            }
        }
        let ov = &self.ov;
        let font = egui::TextStyle::Body.resolve(ui.style());
        let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            let text = buf.as_str();
            let spans: Vec<_> = redact::find_with(text, ov)
                .into_iter()
                .map(|h| h.span)
                .collect();
            let mut job = crate::tools::notes::highlight(ui, text, &spans, &font);
            job.wrap.max_width = wrap;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let pick = self.note_pick(&ctx, &e.text);
        let mut picked = None;
        let r = widgets::area(
            ui,
            id,
            Field::CardNote,
            &mut e.text,
            3,
            "What to remember about it. Select a value and press Ctrl+R to redact it.",
            Some(&mut layouter),
            |ui| {
                ui.separator();
                let label = match &pick {
                    Some(Pick::Unredact(_)) => "Unredact",
                    _ => "Redact",
                };
                let go = ui
                    .add_enabled(
                        pick.is_some(),
                        egui::Button::new(label).shortcut_text("Ctrl+R"),
                    )
                    .on_disabled_hover_text("Select text to redact")
                    .clicked();
                if go {
                    picked = pick.clone();
                }
                go
            },
        );
        if let Some(p) = picked {
            *ev = Some(NoteEv::Pick(p));
        }
        if std::mem::take(&mut e.fresh) {
            r.request_focus();
            r.scroll_to_me(Some(egui::Align::Center));
        }
        if r.lost_focus() && ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
            *ev = Some(NoteEv::Cancel);
        }
        let hits = redact::find_with(&e.text, &self.ov);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui
                .add(egui::Button::new("Save").small())
                .on_hover_text(if e.ids.len() > 1 {
                    "Put it on each of them (Ctrl+Enter)"
                } else {
                    "Keep it (Ctrl+Enter)"
                })
                .clicked()
            {
                *ev = Some(NoteEv::Save);
            }
            if ui
                .small_button("Cancel")
                .on_hover_text("Leave the note as it was (Esc)")
                .clicked()
            {
                *ev = Some(NoteEv::Cancel);
            }
            let label = match &pick {
                Some(Pick::Unredact(_)) => "Unredact",
                _ => "Redact",
            };
            if ui
                .add_enabled(pick.is_some(), egui::Button::new(label).small())
                .on_hover_text(if label == "Unredact" {
                    "Show this value again, everywhere: the Notes tab too (Ctrl+R)"
                } else {
                    "Hide the selected text wherever it appears: the Notes tab too (Ctrl+R)"
                })
                .on_disabled_hover_text("Select text to redact")
                .clicked()
                && let Some(p) = pick.clone()
            {
                *ev = Some(NoteEv::Pick(p));
            }
            if !hits_badge(ui, &hits) {
                let k = e.text.chars().count();
                ui.weak(format!("{k} / {}", Field::CardNote.max()));
            }
        });
        let others = e.ids.len().saturating_sub(1);
        let noted = e
            .ids
            .iter()
            .filter(|id| self.notes.contains_key(*id))
            .count();
        let kept = if others > 0 {
            let s = if others == 1 { "" } else { "s" };
            match noted {
                0 => format!("Goes on this card and {others} other{s}"),
                1 => format!(
                    "Goes on this card and {others} other{s}, in place of the note one of \
                     them has"
                ),
                k => format!(
                    "Goes on this card and {others} other{s}, in place of the notes {k} of \
                     them have"
                ),
            }
        } else {
            self.note_kept(i).to_string()
        };
        ui.add(egui::Label::new(RichText::new(kept).small().weak()).wrap());
    }

    /// What Redact would do with the editor's selection now.
    fn note_pick(&self, ctx: &egui::Context, text: &str) -> Option<Pick> {
        let sel = widgets::text_selection(ctx, editor_id(), text)?;
        redact::pick(text, sel, &redact::find_with(text, &self.ov))
    }

    /// What the panel's note section asked for.
    pub(super) fn note_event(&mut self, ev: NoteEv) {
        match ev {
            NoteEv::Open(i) => self.open_note(&[i]),
            NoteEv::Save => self.save_note(),
            NoteEv::Cancel => self.note_edit = None,
            NoteEv::Remove(i) => self.remove_notes(&[i]),
            NoteEv::Pick(p) => {
                if let Some(e) = &self.note_edit {
                    self.menu_act = Some(Action::NotePick(e.text.clone(), p));
                }
            }
        }
    }

    /// The card with a note whose mark is at the screen point `at` (the
    /// one on top), in the canvas `rect`.
    pub(super) fn note_hit(&self, rect: Rect, at: Pos2) -> Option<usize> {
        self.stack().into_iter().rev().find(|&i| {
            let n = &self.map.nodes[i];
            !n.note.is_empty()
                && self.pos.get(&n.id).is_some_and(|&p| {
                    let r = Rect::from_min_size(
                        self.to_screen(rect, p),
                        layout::card_size(n) * self.zoom,
                    );
                    paint::note_rect(r, self.zoom).expand(2.0).contains(at)
                })
        })
    }
}

/// "🔑 2 redacted: …" in the warning colour, when something is: hidden in
/// whatever leaves the map. True when it showed.
fn hits_badge(ui: &mut Ui, hits: &[Hit]) -> bool {
    if hits.is_empty() {
        return false;
    }
    let warn = widgets::warn();
    egui::Frame::new()
        .fill(warn.gamma_multiply(0.12))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(5, 0))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("🔑 {}", redact::summary(hits)))
                        .small()
                        .color(warn),
                )
                .extend(),
            )
        })
        .response
        .on_hover_text(
            "Hidden in saved maps, the notes kept on this computer, copies, exports and the \
             Site Bundle",
        );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::export::{Drawn, Format, Look};

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("is-alive-notes-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join(STORE_FILE)
    }

    const DEV: [u8; 6] = [0x00, 0x1b, 0x21, 0x0a, 0x0b, 0x0c];

    fn card(id: &str, ip: &str, mac: Option<[u8; 6]>, up: Option<usize>) -> Node {
        Node {
            id: id.into(),
            label: format!("{id} label"),
            kind: "SWITCH".into(),
            ip: ip.parse().ok(),
            mac,
            up: up.map(|p| (p, Link::Wired, None)),
            ..Default::default()
        }
    }

    /// A live map at a customer: the internet, a gateway known only by IP,
    /// and a switch known by its MAC.
    fn live(path: &std::path::Path) -> MapView {
        let mut v = MapView::new(None);
        v.note_store = Some(Store::at(Some(path.to_path_buf())));
        v.map.nodes = vec![
            card("internet", "", None, None),
            card("ip:10.0.0.1", "10.0.0.1", None, Some(0)),
            card("mac:sw", "10.0.0.2", Some(DEV), Some(1)),
        ];
        v.lay = layout::layout_in(&v.map, &|_| true, Mode::Cluster);
        v
    }

    #[test]
    fn a_note_is_kept_by_device_never_by_ip_or_a_files_names() {
        let n = |id: &str, mac: Option<[u8; 6]>| card(id, "10.0.0.1", mac, None);
        let k = |n: &Node| key(&Scope::Live, n);
        assert_eq!(k(&n("ip:10.0.0.1", None)), None);
        assert_eq!(k(&n("fw", None)), None, "a /mapnetwork file's own id");
        assert_eq!(
            k(&n("ip:10.0.0.1", Some(DEV))).as_deref(),
            Some("mac:00:1b:21:0a:0b:0c")
        );
        // Randomized, multicast and shared router addresses are the same
        // at many sites.
        for m in [
            [0x02, 0x42, 0xac, 0x11, 0x00, 0x02],
            [0x01, 0x00, 0x5e, 0x00, 0x00, 0x01],
            [0x00, 0x00, 0x5e, 0x00, 0x01, 0x01],
            [0x00, 0x00, 0x0c, 0x07, 0xac, 0x0a],
            [0x00, 0x00, 0x0c, 0x9f, 0xf0, 0x01],
            [0x00, 0x07, 0xb4, 0x00, 0x01, 0x02],
            [0; 6],
        ] {
            assert_eq!(k(&n("x", Some(m))), None, "{m:02x?}");
        }
        // This laptop goes to every site.
        let mut me = n("ip:10.0.0.5", Some(DEV));
        me.kind = "THIS MACHINE".into();
        assert_eq!(k(&me), None);
        let ex = Scope::Example("example-Acme.dc.html".into());
        assert_eq!(
            key(&ex, &n("fw", None)).as_deref(),
            Some("example:example-Acme.dc.html/fw")
        );
        assert_eq!(key(&Scope::Saved, &n("x", Some(DEV))), None);
    }

    /// The same IP at the next customer gets none of this one's notes; the
    /// same device (by MAC) gets its own back, after a restart too.
    #[test]
    fn live_notes_never_leak_to_another_network() {
        let path = temp("leak");
        let mut v = live(&path);
        v.notes
            .insert("ip:10.0.0.1".into(), "Gateway: ISP router, call Bob".into());
        v.notes.insert("mac:sw".into(), "Port 7 loose".into());
        v.sync_notes();
        let kept = std::fs::read_to_string(&path).unwrap();
        assert!(kept.contains("Port 7 loose") && kept.contains("mac:00:1b:21:0a:0b:0c"));
        assert!(!kept.contains("call Bob"), "an IP card's note isn't kept");
        assert_eq!(
            v.note_kept(1).split(':').next(),
            Some("Kept only until is-alive closes or another map starts")
        );

        // Another network: a new map starts; its 10.0.0.1 has no note.
        v.new_map();
        v.map.nodes = vec![
            card("internet", "", None, None),
            card("ip:10.0.0.1", "10.0.0.1", None, Some(0)),
        ];
        v.refill_notes();
        assert!(v.notes.is_empty(), "{:?}", v.notes);
        v.sync_notes();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("Port 7 loose")
        );

        // After a restart, the switch (found under another id) has its note.
        let mut v = live(&path);
        v.map.nodes[2].id = "ip:10.0.0.2".into();
        v.refill_notes();
        assert_eq!(
            v.notes.get("ip:10.0.0.2").map(String::as_str),
            Some("Port 7 loose")
        );
        assert!(!v.notes.contains_key("ip:10.0.0.1"));
        // Taken off: gone from the computer too.
        v.notes.clear();
        v.sync_notes();
        assert!(!path.exists(), "the empty store's file goes");
    }

    #[test]
    fn the_store_is_read_back_defensively() {
        let path = temp("store");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let long = "x".repeat(5000);
        let doc = json!({"format": STORE_FORMAT, "version": 1, "notes": {
            "mac:00:1b:21:0a:0b:0c": {"note": format!("a\u{202E}b\r\n{long}"), "at": "2026"},
            "mac:00:1b:21:0a:0b:0d": {"note": 7},
        }});
        std::fs::write(&path, doc.to_string()).unwrap();
        let s = Store::at(Some(path.clone()));
        let n = s.get("mac:00:1b:21:0a:0b:0c").unwrap();
        assert!(n.starts_with("ab\n"));
        assert_eq!(n.chars().count(), Field::CardNote.max());
        assert_eq!(s.get("mac:00:1b:21:0a:0b:0d"), None);
        std::fs::write(
            &path,
            "{\"format\":\"other\",\"notes\":{\"k\":{\"note\":\"x\"}}}",
        )
        .unwrap();
        assert_eq!(Store::at(Some(path)).texts().count(), 0);
    }

    fn secrets() -> redact::Overrides {
        redact::Overrides {
            // Redacted by hand (in Notes or a card note's editor).
            redact: vec!["Hunter2Secret".into()],
            allow: vec![],
            // From a backup loaded in Config Export.
            known: vec!["Bobcat2019".into()],
        }
    }

    const NOTE: &str = "wifi key Hunter2Secret\nadmin pw Bobcat2019";

    /// Typed with a password picked by hand and a backup's secret: the
    /// note's every way out of the map is redacted, as notes.md is.
    #[test]
    fn whatever_leaves_the_map_is_redacted() {
        let path = temp("redact");
        let mut v = live(&path);
        v.ov = secrets();
        v.notes.insert("mac:sw".into(), NOTE.into());
        let raw = |s: &str| s.contains("Hunter2Secret") || s.contains("Bobcat2019");
        // Kept on this computer.
        v.sync_notes();
        let kept = std::fs::read_to_string(&path).unwrap();
        assert!(!raw(&kept) && kept.contains("REDACTED: by hand"), "{kept}");
        assert!(kept.contains("REDACTED: from config"));
        // On the card: tooltip, search, Copy details, legend.
        v.stamp_notes();
        let n = v.map.nodes[2].clone();
        assert!(!raw(&n.note) && !raw(&first_lines(&n.note)));
        assert!(!raw(&panel::card_text(&n)) && panel::card_text(&n).contains("Note:"));
        v.search = "Bobcat".into();
        assert!(!v.matches(&n), "a hidden value isn't found by search");
        v.search = "wifi key".into();
        assert!(v.matches(&n));
        // A saved map's file.
        let s = v.snapshot("x", saved::now(), saved::copy_map(&v.map));
        let file = saved::to_json(&s);
        assert!(!raw(&file) && file.contains("wifi key"), "{file}");
        // Every export.
        let moved = vec![false; v.map.nodes.len()];
        let d = Drawn {
            map: &v.map,
            lay: &v.lay,
            pos: &v.lay.pos,
            moved: &moved,
            legend: true,
        };
        let look = Look::new(&egui::Visuals::dark(), false);
        for f in [
            Format::Svg,
            Format::Csv,
            Format::DrawIo,
            Format::Dot,
            Format::Mermaid,
        ] {
            let text = f.text(&d, look).expect("text");
            assert!(!raw(&text), "{f:?}");
        }
        // The Site Bundle's part: from the same stamped cards.
        assert!(n.note.contains("REDACTED"));
    }

    /// A backup loaded after the note was written: what's kept is
    /// rewritten redacted at once; keeping the value (Unredact, in Notes or
    /// here) shows it again everywhere.
    #[test]
    fn redaction_changes_reach_the_kept_notes() {
        let path = temp("later");
        let mut v = live(&path);
        v.notes.insert("mac:sw".into(), NOTE.into());
        v.sync_notes();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("Bobcat2019")
        );
        v.ov.known = vec!["Bobcat2019".into()];
        v.sync_notes();
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("Bobcat2019")
        );
        // Kept as written: the Notes tab's keep list holds for card notes.
        let hit = redact::find_with(NOTE, &v.ov)[0].clone();
        let mut ov = v.ov.clone();
        ov.apply(NOTE, Pick::Unredact(hit)).unwrap();
        assert_eq!(ov.allow, ["Bobcat2019"]);
        v.ov = ov;
        v.stamp_notes();
        assert!(v.map.nodes[2].note.contains("Bobcat2019"));
    }

    #[test]
    fn notes_are_added_edited_and_removed_on_one_card_or_several() {
        let mut v = live(&temp("menu"));
        let labels = |v: &MapView, g: &[usize]| -> Vec<String> {
            v.note_items(g).into_iter().map(|(l, _, _)| l).collect()
        };
        assert_eq!(labels(&v, &[2]), ["Add note…"]);
        v.open_note(&[2]);
        assert_eq!(v.selection(), [2], "its card joins the selection");
        v.note_edit.as_mut().unwrap().text = "  Spare PSU\n\n".into();
        v.save_note();
        assert_eq!(v.notes["mac:sw"], "  Spare PSU");
        assert_eq!(labels(&v, &[2]), ["Edit note…", "Remove note"]);
        // Several: the same note on each, theirs replaced.
        assert_eq!(labels(&v, &[1, 2]), ["Add note to 2 cards…", "Remove note"]);
        v.open_note(&[1, 2]);
        assert_eq!(v.note_edit.as_ref().unwrap().text, "", "they differ");
        v.note_edit.as_mut().unwrap().text = "Rack B".into();
        v.save_note();
        assert_eq!(
            (v.notes["ip:10.0.0.1"].as_str(), v.notes["mac:sw"].as_str()),
            ("Rack B", "Rack B")
        );
        v.open_note(&[1, 2]);
        assert_eq!(
            v.note_edit.as_ref().unwrap().text,
            "Rack B",
            "they share it"
        );
        assert_eq!(
            labels(&v, &[1, 2]),
            ["Add note to 2 cards…", "Remove notes (2)"]
        );
        v.remove_notes(&[1, 2]);
        assert!(v.notes.is_empty() && v.note_edit.is_none());
        // Saved empty: no note.
        v.open_note(&[2]);
        v.note_edit.as_mut().unwrap().text = " \n ".into();
        v.save_note();
        assert!(v.notes.is_empty());
    }

    /// A note written, changed and removed: each a step, named for it.
    #[test]
    fn note_edits_are_undone() {
        let mut v = live(&temp("undo"));
        v.track_edits();
        v.open_note(&[2]);
        v.note_edit.as_mut().unwrap().text = "Port 7 loose".into();
        v.save_note();
        v.track_edits();
        assert_eq!(v.history.undo_label(), Some("note on mac:sw label"));
        v.remove_notes(&[2]);
        v.track_edits();
        assert_eq!(v.history.undo_label(), Some("remove note on mac:sw label"));
        v.step(true);
        assert_eq!(v.notes["mac:sw"], "Port 7 loose");
        v.step(true);
        assert!(v.notes.is_empty());
        v.step(false);
        assert_eq!(v.notes["mac:sw"], "Port 7 loose");
        v.open_note(&[1, 2]);
        v.note_edit.as_mut().unwrap().text = "Rack B".into();
        v.save_note();
        v.track_edits();
        assert_eq!(v.history.undo_label(), Some("notes on 2 cards"));
    }

    #[test]
    fn the_legend_lists_notes_only_when_a_card_has_one() {
        let mut v = live(&temp("legend"));
        let lines = v.lines_now();
        let has = |v: &MapView| {
            v.legend_entries(&lines)
                .iter()
                .find(|e| e.swatch == crate::map::legend::Swatch::Note)
                .map(|e| (e.label.clone(), e.count))
        };
        v.stamp_notes();
        assert_eq!(has(&v), None);
        v.notes.insert("mac:sw".into(), "x".into());
        v.stamp_notes();
        assert_eq!(has(&v), Some(("Has a note".into(), Some(1))));
    }

    #[test]
    fn a_tooltip_shows_the_first_lines() {
        assert_eq!(first_lines("one\n\n two \nthree"), "one\ntwo\nthree");
        assert_eq!(first_lines("a\nb\nc\nd"), "a\nb\nc\n…");
        assert!(first_lines(&"x".repeat(200)).chars().count() <= 81);
    }
}
