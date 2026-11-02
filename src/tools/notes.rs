//! Tech notes: a Markdown scratchpad for the visit. Whatever leaves the
//! editor (autosave, Copy, Save, Site Bundle) is the sanitized text, and
//! the editor highlights every value that will be redacted. The raw text
//! lives only in memory: after a restart the notes come back sanitized.
//!
//! Each emoji keeps the style it was added in (monochrome, colour, neon)
//! as an invisible mark after it (see `theme::EmojiStyle::marker`). The
//! autosaved notes keep the marks; Copy, Raw copy, Save .md and the Site
//! Bundle drop them, so other programs get plain emoji. Ctrl+C and Cut in
//! the editor keep them, so emoji pasted back in keep their style.

use crate::job::Job;
use crate::markdown;
use crate::net::hostinfo::HostInfo;
use crate::redact::{self, Hit, Pick};
use crate::widgets;
use eframe::egui::{
    self, Color32, FontId, Key, Modifiers, RichText, Stroke, TextFormat, Ui,
    text::{CCursor, CCursorRange, CharIndex, LayoutJob},
};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;

#[derive(PartialEq, Clone, Copy)]
enum View {
    Edit,
    Split,
    Preview,
}

/// Seconds after the last keystroke before the autosave.
const SAVE_AFTER: f64 = 1.5;
const EDITOR_ID: &str = "tech_notes_editor";

pub struct NotesTool {
    text: String,
    view: View,
    show_hits: bool,
    /// When the text last changed, while unsaved.
    dirty: Option<f64>,
    saved: Option<String>,
    err: Option<String>,
    /// Redactions found so far, to flash the count when it grows.
    known_hits: usize,
    flash: f64,
    /// The technician's own calls. Picks to redact live only in memory;
    /// allowed values are saved beside the notes, which hold them anyway.
    ov: redact::Overrides,
    /// (hash of text and overrides, hits) so the regexes run once per edit.
    cache: RefCell<(u64, Vec<Hit>)>,
    /// The same for the sanitized text (preview, status, Site Bundle).
    clean: RefCell<(u64, (String, Vec<Hit>))>,
    /// A text file opened to read, and whether its tab is showing.
    file: Option<Viewed>,
    on_file: bool,
    opening: Option<Job<Result<Viewed, String>>>,
    /// Give the editor the keyboard back once it's drawn: a toolbar click
    /// would take it away again in the same frame.
    refocus: bool,
    /// The selection (bytes) and how many emoji it holds, as the emoji menu
    /// opened: the editor loses it once the menu has the pointer.
    picker_sel: Option<(Range<usize>, usize)>,
    /// Emoji were restyled since the emoji menu opened.
    picker_restyled: bool,
    /// A selection (characters) to put back once the editor has the
    /// keyboard again: without it, the editor drops all but the cursor.
    reselect: Option<Range<usize>>,
}

/// A text or config file open in the viewer: read-only, never saved or
/// bundled with the notes.
struct Viewed {
    path: PathBuf,
    text: String,
    lines: usize,
    bytes: u64,
}

/// Larger files are refused: this is for reading configs, not big logs.
const VIEW_MAX: u64 = 5 * 1024 * 1024;

/// Read `path` as text: UTF-8 (BOM or not), UTF-16 with a BOM, else
/// whatever decodes. A file with NUL bytes and no UTF-16 BOM is binary.
fn read_text(path: &std::path::Path) -> Result<Viewed, String> {
    let name = crate::tools::export::file_name(path);
    let meta = std::fs::metadata(path).map_err(|e| format!("{name}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{name}: not a file"));
    }
    if meta.len() > VIEW_MAX {
        return Err(format!(
            "{name}: {:.1} MB; the viewer opens files up to 5 MB",
            meta.len() as f64 / 1048576.0
        ));
    }
    let data = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
    let utf16 = |b: &[u8], word: fn([u8; 2]) -> u16| {
        let w: Vec<u16> = b.as_chunks::<2>().0.iter().map(|&c| word(c)).collect();
        String::from_utf16_lossy(&w)
    };
    let text = match data.as_slice() {
        [0xef, 0xbb, 0xbf, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xff, 0xfe, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xfe, 0xff, rest @ ..] => utf16(rest, u16::from_be_bytes),
        d if d[..d.len().min(8192)].contains(&0) => {
            return Err(format!("{name}: not a text file"));
        }
        d => String::from_utf8_lossy(d).into_owned(),
    };
    let text = text.replace("\r\n", "\n");
    Ok(Viewed {
        path: path.to_path_buf(),
        lines: text.lines().count(),
        bytes: meta.len(),
        text,
    })
}

fn path() -> Option<PathBuf> {
    Some(crate::theme::data_dir()?.join("notes.md"))
}

/// Most of the autosaved notes and keep-list files read back: four bytes
/// for each character the editor takes.
const NOTES_FILE_MAX: u64 = 4 * crate::input::Field::Notes.max() as u64;

/// Values the technician said to keep, as a JSON list of strings.
fn allow_path() -> Option<PathBuf> {
    Some(crate::theme::data_dir()?.join("notes-allow.json"))
}

impl Default for NotesTool {
    fn default() -> Self {
        // Read no more than the editor holds, without its hidden characters.
        let text = path()
            .and_then(|p| crate::input::read_capped(&p, NOTES_FILE_MAX))
            .map(|t| crate::input::Field::Notes.clean(&t))
            .unwrap_or_default();
        let ov = redact::Overrides {
            redact: Vec::new(),
            allow: allow_path()
                .and_then(|p| crate::input::read_capped(&p, NOTES_FILE_MAX))
                .and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok())
                .map(|v| {
                    v.into_iter()
                        .filter(|s| s.len() <= 4096)
                        .take(10_000)
                        .collect()
                })
                .unwrap_or_default(),
            known: Vec::new(),
        };
        // A file with anything redactable in it (edited by hand, or from an
        // older build) is rewritten sanitized at once, not on the next edit.
        let unclean = !redact::find_with(&text, &ov).is_empty();
        let mut notes = Self {
            text,
            ov,
            view: View::Edit,
            show_hits: true,
            dirty: None,
            saved: None,
            err: None,
            known_hits: 0,
            flash: f64::NEG_INFINITY,
            cache: RefCell::new((u64::MAX, Vec::new())),
            clean: RefCell::new((u64::MAX, (String::new(), Vec::new()))),
            file: None,
            on_file: false,
            opening: None,
            refocus: false,
            picker_sel: None,
            picker_restyled: false,
            reselect: None,
        };
        if unclean {
            notes.write();
        }
        notes
    }
}

fn hash_with(text: &str, ov: &redact::Overrides) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (text, ov).hash(&mut h);
    h.finish()
}

fn hits_for(cache: &RefCell<(u64, Vec<Hit>)>, text: &str, ov: &redact::Overrides) -> Vec<Hit> {
    let h = hash_with(text, ov);
    let mut c = cache.borrow_mut();
    if c.0 != h {
        *c = (h, redact::find_with(text, ov));
    }
    c.1.clone()
}

impl NotesTool {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// The notes as they may leave this machine.
    pub fn sanitized(&self) -> (String, Vec<Hit>) {
        let h = hash_with(&self.text, &self.ov);
        let mut c = self.clean.borrow_mut();
        if c.0 != h {
            *c = (h, redact::sanitize_with(&self.text, &self.ov));
        }
        c.1.clone()
    }

    /// The sanitized notes for other programs: no emoji style marks.
    pub fn sanitized_plain(&self) -> String {
        crate::theme::strip_emoji_styles(&self.sanitized().0)
    }

    /// Write the sanitized notes, replacing the file in one step.
    fn write(&mut self) {
        self.dirty = None;
        let Some(p) = path() else { return };
        let (clean, _) = self.sanitized();
        let tmp = p.with_extension("md.tmp");
        let r = p
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| crate::theme::write_private(&tmp, clean))
            .and_then(|_| std::fs::rename(&tmp, &p));
        match r {
            Ok(()) => {
                self.saved = Some(chrono::Local::now().format("%H:%M").to_string());
                self.err = None;
                self.write_allow();
            }
            Err(e) => self.err = Some(format!("Couldn't autosave notes: {e}")),
        }
    }

    /// The allow list, trimmed to values still in the notes.
    fn write_allow(&mut self) {
        let text = &self.text;
        self.ov.allow.retain(|a| text.contains(a.as_str()));
        let Some(p) = allow_path() else { return };
        let r = if self.ov.allow.is_empty() {
            match std::fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else {
            crate::theme::write_private(
                &p,
                serde_json::to_string(&self.ov.allow).unwrap_or_default(),
            )
        };
        if let Err(e) = r {
            self.err = Some(format!("Couldn't save the allowed values: {e}"));
        }
    }

    /// Secret values from the loaded backups and exports. When they change,
    /// the saved notes are rewritten so none of them stays on disk.
    pub fn set_known(&mut self, known: &[String], now: f64) {
        if self.ov.known != known {
            self.ov.known = known.to_vec();
            if !self.text.is_empty() {
                self.changed(now);
            }
        }
    }

    /// The technician's own redact and keep calls, for the Site Bundle.
    pub fn overrides(&self) -> &redact::Overrides {
        &self.ov
    }

    /// What the Redact button would do now.
    fn pick(&self, ctx: &egui::Context, hits: &[Hit]) -> Option<Pick> {
        let sel = widgets::text_selection(ctx, egui::Id::new(EDITOR_ID), &self.text)?;
        redact::pick(&self.text, sel, hits)
    }

    fn apply(&mut self, ctx: &egui::Context, pick: Pick) {
        match self.ov.apply(&self.text, pick) {
            Ok(m) => widgets::toast(m, true),
            Err(m) => return widgets::toast(m, false),
        }
        self.changed(ctx.input(|i| i.time));
    }

    /// Save now if there are unsaved changes (on exit).
    pub fn flush(&mut self) {
        if self.dirty.is_some() {
            self.write();
        }
    }

    fn changed(&mut self, now: f64) {
        self.dirty = Some(now);
    }

    /// Replace the selection (or insert at the cursor) via `f`, which gets
    /// the selected text and returns the new text and where the cursor goes
    /// within it (in characters).
    fn edit_at_cursor(&mut self, ctx: &egui::Context, f: impl FnOnce(&str) -> (String, usize)) {
        let id = egui::Id::new(EDITOR_ID);
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        let chars = self.text.chars().count();
        let (a, b) = state
            .cursor
            .char_range()
            .map(|r| {
                let (x, y) = (r.primary.index.0, r.secondary.index.0);
                (x.min(y).min(chars), x.max(y).min(chars))
            })
            .unwrap_or((chars, chars));
        let byte = |c: usize| {
            self.text
                .char_indices()
                .nth(c)
                .map_or(self.text.len(), |(i, _)| i)
        };
        let (ba, bb) = (byte(a), byte(b));
        let (new, at) = f(&self.text[ba..bb]);
        self.text.replace_range(ba..bb, &new);
        let cursor = CCursor::new(CharIndex(a + at));
        state.cursor.set_char_range(Some(CCursorRange::one(cursor)));
        state.store(ctx, id);
        self.refocus = true;
        self.changed(ctx.input(|i| i.time));
    }

    /// Wrap the selection (or an empty spot at the cursor) in `mark`:
    /// ** for bold, * for italic, ` for code.
    fn wrap(&mut self, ctx: &egui::Context, mark: &str) {
        self.edit_at_cursor(ctx, |sel| {
            let n = mark.chars().count();
            let len = sel.chars().count();
            (
                format!("{mark}{sel}{mark}"),
                if len == 0 { n } else { len + 2 * n },
            )
        })
    }

    /// Put `prefix` at the start of the cursor's line.
    fn line_prefix(&mut self, ctx: &egui::Context, prefix: &str) {
        let id = egui::Id::new(EDITOR_ID);
        let state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        let c = state
            .cursor
            .char_range()
            .map_or(self.text.chars().count(), |r| r.primary.index.0);
        let b = self
            .text
            .char_indices()
            .nth(c)
            .map_or(self.text.len(), |(i, _)| i);
        let start = self.text[..b].rfind('\n').map_or(0, |i| i + 1);
        let line_chars = self.text[start..b].chars().count();
        let start_char = c - line_chars;
        let mut state = state;
        self.text.insert_str(start, prefix);
        let to = CCursor::new(CharIndex(start_char + line_chars + prefix.chars().count()));
        state.cursor.set_char_range(Some(CCursorRange::one(to)));
        state.store(ctx, id);
        self.refocus = true;
        self.changed(ctx.input(|i| i.time));
    }

    /// Set the emoji in the selection (`sel`, bytes) to `style`, keeping the
    /// selection (and giving the editor the keyboard back); redaction
    /// markers' 🔑 are left alone. Returns where the selection is now.
    fn restyle(
        &mut self,
        ctx: &egui::Context,
        sel: Range<usize>,
        style: crate::theme::EmojiStyle,
    ) -> Range<usize> {
        let skip = redact::markers(&self.text);
        let (text, n) = crate::theme::restyle_emoji(&self.text, sel.clone(), style, &skip);
        if n == 0 {
            return sel;
        }
        // Every mark that changes is before the selection's end, once that
        // is past the marks of an emoji it ends inside.
        let end = crate::theme::inside_emoji(&self.text, sel.end).map_or(sel.end, |(_, e)| e);
        let a = self.text[..sel.start].chars().count();
        let b = (self.text[..end].chars().count() + text.chars().count())
            .saturating_sub(self.text.chars().count())
            .max(a);
        self.text = text;
        let id = egui::Id::new(EDITOR_ID);
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(CharIndex(a)),
            CCursor::new(CharIndex(b)),
        )));
        state.store(ctx, id);
        self.reselect = Some(a..b);
        self.refocus = true;
        self.changed(ctx.input(|i| i.time));
        widgets::toast(
            format!("{n} emoji restyled: {}", notes_style_name(style)),
            true,
        );
        sel.start..byte_at(&self.text, b)
    }

    /// Backspace (and Ctrl+H) and Delete take an emoji and its invisible
    /// marks in one press: the marks go first, then the TextEdit deletes the
    /// emoji. (Ctrl+Backspace and Ctrl+Delete take them as one word.)
    fn delete_emoji_marks(&mut self, ctx: &egui::Context) {
        let (back, del) = ctx.input(|i| {
            let plain = i.modifiers.is_none();
            (
                plain && i.key_pressed(Key::Backspace) || i.modifiers.ctrl && i.key_pressed(Key::H),
                plain && i.key_pressed(Key::Delete),
            )
        });
        if !back && !del {
            return;
        }
        let id = egui::Id::new(EDITOR_ID);
        let Some(mut state) = egui::TextEdit::load_state(ctx, id) else {
            return;
        };
        let Some(r) = state.cursor.char_range() else {
            return;
        };
        if r.primary.index != r.secondary.index {
            return;
        }
        let c = r.primary.index.0;
        let b = byte_at(&self.text, c);
        let marks = if back {
            let prev = self.text[..b].chars().next_back().map_or(0, char::len_utf8);
            crate::theme::inside_emoji(&self.text, b - prev)
                .filter(|&(_, end)| end == b)
                .map(|(start, end)| emoji_char_end(&self.text, start)..end)
        } else {
            self.text[b..]
                .chars()
                .next()
                .filter(|e| crate::theme::is_emoji(*e))
                .map(|e| b + e.len_utf8()..crate::theme::emoji_end(&self.text, b).0)
        };
        let Some(marks) = marks.filter(|m| !m.is_empty()) else {
            return;
        };
        let n = self.text[marks.clone()].chars().count();
        self.text.replace_range(marks, "");
        if back {
            let to = CCursor::new(CharIndex(c - n));
            state.cursor.set_char_range(Some(CCursorRange::one(to)));
            state.store(ctx, id);
        }
    }

    /// A cursor between an emoji and its invisible marks moves past them
    /// (or before the emoji, going left), so arrows step over an emoji in
    /// one press and nothing is typed between it and its style. Either end
    /// of a selection moves the same way, so an emoji is selected (typed
    /// over, deleted, cut) with its marks or not at all.
    fn step_over_emoji_marks(&self, ctx: &egui::Context) {
        // From memory, not the editor's output: Enter on a list item may
        // have changed the text and the cursor since.
        let id = egui::Id::new(EDITOR_ID);
        let Some(mut state) = egui::TextEdit::load_state(ctx, id) else {
            return;
        };
        let Some(mut r) = state.cursor.char_range() else {
            return;
        };
        let step = |c: &mut CCursor, left: bool| {
            let b = byte_at(&self.text, c.index.0);
            let Some((start, end)) = crate::theme::inside_emoji(&self.text, b) else {
                return false;
            };
            c.index = CharIndex(self.text[..if left { start } else { end }].chars().count());
            true
        };
        let left = ctx.input(|i| i.key_pressed(Key::ArrowLeft));
        let moved = step(&mut r.primary, left);
        if step(&mut r.secondary, false) || moved {
            state.cursor.set_char_range(Some(r));
            state.store(ctx, id);
        }
    }

    /// Autosave; called every frame whichever tab is showing, so leaving
    /// Notes right after typing still saves.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.dirty.is_some_and(|t| now - t >= SAVE_AFTER) {
            self.write();
        }
        if let Some(t) = self.dirty {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                (SAVE_AFTER - (now - t)).max(0.05),
            ));
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, info: &HostInfo, look: &mut crate::theme::Settings) {
        let font = FontId::new(
            look.notes_size as f32,
            crate::fonts::family(ui.ctx(), &look.notes_font),
        );
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let hits = hits_for(&self.cache, &self.text, &self.ov);
        if hits.len() > self.known_hits {
            self.flash = now;
        }
        self.known_hits = hits.len();

        let mut opened = None;
        crate::job::pump(&mut self.opening, |r| opened = Some(r));
        match opened {
            Some(Ok(v)) => {
                self.file = Some(v);
                self.on_file = true;
            }
            Some(Err(e)) => self.err = Some(e),
            None => {}
        }
        if self.file.is_some() {
            self.tabs(ui);
        }
        if self.on_file && self.file.is_some() {
            ui.horizontal(|ui| {
                ui.weak("Read-only");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    font_menus(ui, look)
                });
            });
        } else {
            self.on_file = false;
            self.toolbar(ui, info, &hits, &font, look);
        }
        widgets::error(ui, &self.err);
        // The status row, as on every tab: counts and what's redacted on
        // the left, the ways out on the right.
        ui.separator();
        if self.on_file {
            self.file_footer(ui);
        } else {
            self.footer(ui, &hits, now);
        }
        ui.add_space(2.0);
        if self.on_file {
            if let Some(v) = &self.file {
                viewer(ui, v, &font);
            }
            return;
        }

        let narrow = ui.available_width() < 720.0;
        let view = if narrow && self.view == View::Split {
            View::Edit
        } else {
            self.view
        };
        match view {
            View::Edit => self.editor(ui, &font),
            View::Preview => self.preview(ui, &font),
            View::Split => self.split(ui, &font, look),
        }
    }

    /// Editor and preview side by side, with a divider that drags between
    /// 20% and 80% of the width (double-click: back to half).
    fn split(&mut self, ui: &mut Ui, font: &FontId, look: &mut crate::theme::Settings) {
        const GAP: f32 = 10.0;
        const MIN: f32 = 160.0;
        let full = ui.available_rect_before_wrap();
        let w = full.width();
        let left_w = (w * look.notes_split).clamp(MIN.min(w / 2.0), (w - GAP - MIN).max(w / 2.0));
        let left =
            egui::Rect::from_min_size(full.min, egui::vec2(left_w - GAP / 2.0, full.height()));
        let handle = egui::Rect::from_min_size(
            egui::pos2(left.right(), full.top()),
            egui::vec2(GAP, full.height()),
        );
        let right = egui::Rect::from_min_max(egui::pos2(handle.right(), full.top()), full.max);

        let r = ui.interact(
            handle,
            ui.id().with("notes_split"),
            egui::Sense::click_and_drag(),
        );
        let active = r.hovered() || r.dragged();
        if active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if r.dragged()
            && let Some(p) = r.interact_pointer_pos()
        {
            look.notes_split = ((p.x - full.left()) / w).clamp(0.2, 0.8);
        }
        if r.double_clicked() {
            look.notes_split = 0.5;
        }
        if r.drag_stopped() || r.double_clicked() {
            crate::theme::save(look);
        }
        let colour = if active {
            crate::theme::accent()
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke.color
        };
        let x = handle.center().x;
        ui.painter().vline(
            x,
            handle.y_range(),
            Stroke::new(if active { 2.0 } else { 1.0 }, colour),
        );
        // A grip in the middle, so it reads as something to drag.
        for dy in [-6.0, 0.0, 6.0] {
            ui.painter()
                .circle_filled(egui::pos2(x, handle.center().y + dy), 1.5, colour);
        }

        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
            self.editor(ui, font)
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
            self.preview(ui, font)
        });
        ui.allocate_rect(full, egui::Sense::hover());
    }

    fn toolbar(
        &mut self,
        ui: &mut Ui,
        info: &HostInfo,
        hits: &[Hit],
        font: &FontId,
        look: &mut crate::theme::Settings,
    ) {
        let ctx = ui.ctx().clone();
        // Wraps onto a second row when the window is narrow.
        ui.horizontal_wrapped(|ui| {
            for (v, name, tip) in [
                (View::Edit, "Edit", "Editor only"),
                (View::Split, "Split", "Editor and preview"),
                (View::Preview, "Preview", "Formatted, as saved"),
            ] {
                if ui
                    .selectable_label(self.view == v, name)
                    .on_hover_text(tip)
                    .clicked()
                {
                    self.view = v;
                }
            }
            ui.separator();
            if ui
                .button(RichText::new("B").strong())
                .on_hover_text("Bold (Ctrl+B)")
                .clicked()
            {
                self.wrap(&ctx, "**");
            }
            if ui
                .button(RichText::new("I").italics())
                .on_hover_text("Italic (Ctrl+I)")
                .clicked()
            {
                self.wrap(&ctx, "*");
            }
            if ui
                .button(RichText::new("</>").monospace())
                .on_hover_text("Inline code")
                .clicked()
            {
                self.wrap(&ctx, "`");
            }
            if ui.button("H").on_hover_text("Heading").clicked() {
                self.line_prefix(&ctx, "## ");
            }
            if ui.button("•").on_hover_text("Bullet list").clicked() {
                self.line_prefix(&ctx, "- ");
            }
            if ui
                .button("☐")
                .on_hover_text("Task box (tick it in the preview)")
                .clicked()
            {
                self.line_prefix(&ctx, "- [ ] ");
            }
            let mut emoji = None;
            let mut restyle = None;
            let selected = self.picker_sel.as_ref().map_or(0, |p| p.1);
            // Picking a style keeps the menu open; picking an emoji closes it.
            let stay = egui::containers::menu::MenuConfig::new()
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            let menu = egui::containers::menu::MenuButton::new(RichText::new("☺").size(16.0))
                .config(stay)
                .ui(ui, |ui| {
                    // Every style is on offer in every theme.
                    ui.horizontal(|ui| {
                        ui.weak("Style").on_hover_text(
                            "For the emoji you add from here on. Each emoji keeps its own \
                             style; the app's emoji style is in Settings.",
                        );
                        for s in crate::theme::EmojiStyle::ALL {
                            if ui
                                .selectable_label(look.notes_emoji == s, notes_style_name(s))
                                .on_hover_text(notes_style_tip(s))
                                .clicked()
                            {
                                look.notes_emoji = s;
                                crate::theme::save(look);
                                if selected > 0 {
                                    restyle = Some(s);
                                }
                            }
                        }
                    });
                    let hint = match selected {
                        0 => {
                            "For emoji added from here: those in the notes keep theirs.".to_string()
                        }
                        1 => "Also restyles the selected emoji.".to_string(),
                        n => format!("Also restyles the {n} selected emoji."),
                    };
                    ui.add(
                        egui::Label::new(RichText::new(hint).weak())
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    ui.separator();
                    // Drawn in the style they'll be added in.
                    let style = match look.notes_emoji {
                        crate::theme::EmojiStyle::Theme => crate::theme::emoji_style(),
                        s => s,
                    };
                    egui::Grid::new("notes_emoji")
                        .spacing([2.0, 2.0])
                        .show(ui, |ui| {
                            for (i, (e, code, what)) in markdown::SHORTCODES.iter().enumerate() {
                                let c = e.chars().next().unwrap_or(' ');
                                let glyph = RichText::new(*e)
                                    .size(font.size * crate::theme::EMOJI_SCALE)
                                    .color(crate::theme::emoji_colour_in(
                                        style,
                                        c,
                                        ui.visuals().text_color(),
                                    ));
                                if ui
                                    .button(glyph)
                                    .on_hover_text(format!(
                                        "{what}\n:{code}:  (Preview draws the code as the emoji)"
                                    ))
                                    .clicked()
                                {
                                    emoji = Some(*e);
                                    ui.close();
                                }
                                if i % 8 == 7 {
                                    ui.end_row();
                                }
                            }
                        });
                });
            let (menu, open) = menu;
            if menu.clicked() {
                let id = egui::Id::new(EDITOR_ID);
                self.picker_sel = widgets::text_selection(&ctx, id, &self.text)
                    .map(|r| (r.clone(), emoji_in(&self.text, r)))
                    .filter(|(_, n)| *n > 0);
                self.picker_restyled = false;
            }
            menu.on_hover_text("Emoji");
            if let Some(s) = restyle
                && let Some((sel, n)) = self.picker_sel.clone()
            {
                self.picker_sel = Some((self.restyle(&ctx, sel, s), n));
                self.picker_restyled = true;
            }
            // Escape closes the menu but also takes the keyboard from the
            // editor: after a restyle, give it back with the selection.
            let escaped = ctx.input(|i| i.key_pressed(Key::Escape));
            if self.picker_restyled && (escaped || open.is_none()) {
                self.picker_restyled = false;
                if escaped && let Some((sel, _)) = &self.picker_sel {
                    let chars = |b: usize| self.text[..b].chars().count();
                    self.reselect = Some(chars(sel.start)..chars(sel.end));
                    self.refocus = true;
                }
            }
            if let Some(e) = emoji {
                // The cursor lands after its style mark.
                let e = crate::theme::styled_emoji(e, look.notes_emoji);
                let n = e.chars().count();
                self.edit_at_cursor(&ctx, |_| (e, n));
            }
            ui.separator();
            if ui
                .button("Time")
                .on_hover_text("Insert the date, time, time zone and clock source")
                .clicked()
            {
                let stamp = crate::net::clock::stamp();
                let n = stamp.chars().count();
                self.edit_at_cursor(&ctx, |_| (stamp, n));
            }
            if ui
                .button("Host info")
                .on_hover_text("Insert this computer's name, LAN, MAC, gateway and WAN")
                .clicked()
            {
                let mut block = info.markdown();
                // On a line of its own.
                let at = widgets::text_selection(&ctx, egui::Id::new(EDITOR_ID), &self.text)
                    .map_or(self.text.len(), |r| r.start);
                if at > 0 && !self.text[..at].ends_with('\n') {
                    block.insert(0, '\n');
                }
                let n = block.chars().count();
                self.edit_at_cursor(&ctx, |_| (block, n));
            }
            ui.separator();
            let pick = self.pick(&ctx, hits);
            let (label, tip) = match &pick {
                Some(Pick::Unredact(_)) => {
                    ("Unredact", "Show this value again, everywhere (Ctrl+R)")
                }
                _ => (
                    "Redact",
                    "Hide the selected text, everywhere it appears (Ctrl+R)",
                ),
            };
            if ui
                .add_enabled(pick.is_some(), egui::Button::new(label))
                .on_hover_text(tip)
                .on_disabled_hover_text("Select text to redact")
                .clicked()
                && let Some(p) = pick
            {
                self.apply(&ctx, p);
            }
            self.overrides_menu(ui);
            // Font on the right; on its own row when this one is full.
            if ui.available_size_before_wrap().x <= 130.0 {
                ui.end_row();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                font_menus(ui, look)
            });
        });
    }

    /// Notes and the open file, when there is one.
    fn tabs(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui.selectable_label(!self.on_file, "Notes").clicked() {
                self.on_file = false;
            }
            let Some(v) = &self.file else { return };
            let name = crate::tools::export::file_name(&v.path);
            if ui
                .selectable_label(self.on_file, &name)
                .on_hover_text(v.path.display().to_string())
                .clicked()
            {
                self.on_file = true;
            }
            if ui
                .small_button("×")
                .on_hover_text("Close the file")
                .clicked()
            {
                self.file = None;
                self.on_file = false;
            }
        });
        ui.separator();
    }

    /// Ask for a text file to read, off the UI thread.
    fn open(&mut self, ctx: &egui::Context) {
        if self.opening.is_some() {
            return;
        }
        self.err = None;
        self.opening = Some(Job::spawn(ctx, |tx| {
            let picked = rfd::FileDialog::new()
                .set_title("Open a text file")
                .add_filter(
                    "Text and config files",
                    &[
                        "txt", "log", "cfg", "conf", "config", "ini", "md", "json", "xml", "csv",
                        "yaml", "yml", "toml", "ps1", "bat", "cmd", "sh", "reg", "inf",
                    ],
                )
                .add_filter("All files", &["*"])
                .pick_file();
            if let Some(p) = picked {
                tx.send(read_text(&p));
            }
        }));
    }

    /// Under an open file: what it is, and Open / Copy / Close.
    fn file_footer(&mut self, ui: &mut Ui) {
        let Some(v) = &self.file else { return };
        let (mut close, mut open) = (false, false);
        ui.horizontal(|ui| {
            ui.weak(format!(
                "{} · {} line{} · {:.1} KB",
                v.path.display(),
                v.lines,
                if v.lines == 1 { "" } else { "s" },
                v.bytes as f64 / 1024.0
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui.button("Close").clicked();
                if ui
                    .button("Copy")
                    .on_hover_text("Copy the whole file")
                    .clicked()
                {
                    widgets::copy(ui.ctx(), v.text.clone());
                    widgets::toast("File copied", true);
                }
                open = ui
                    .button("Open…")
                    .on_hover_text("Open another text file")
                    .clicked();
            });
        });
        if open {
            self.open(ui.ctx());
        }
        if close {
            self.file = None;
            self.on_file = false;
        }
    }

    /// Under the toolbar: counts, what's redacted, autosave, and the ways out.
    fn footer(&mut self, ui: &mut Ui, hits: &[Hit], now: f64) {
        ui.horizontal_wrapped(|ui| {
            let (words, lines) = (
                self.text.split_whitespace().count(),
                self.text.lines().count(),
            );
            let s = |n: usize| if n == 1 { "" } else { "s" };
            ui.weak(format!(
                "{words} word{} · {lines} line{}",
                s(words),
                s(lines)
            ));
            ui.separator();
            if hits.is_empty() {
                ui.weak("Nothing redacted");
            } else {
                let age = (now - self.flash) as f32;
                let warn = widgets::warn();
                let glow = if age < 2.0 {
                    ui.ctx().request_repaint();
                    0.5 + 0.5 * (age * 6.0).cos().abs() * (1.0 - age / 2.0)
                } else {
                    0.0
                };
                let text = RichText::new(format!("🔑 {}", redact::summary(hits))).color(warn);
                egui::Frame::new()
                    .fill(warn.gamma_multiply(0.10 + 0.25 * glow))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::symmetric(6, 1))
                    .show(ui, |ui| ui.label(text))
                    .response
                    .on_hover_text("Hidden in saves, copies and the Site Bundle");
                ui.checkbox(&mut self.show_hits, "Highlight")
                    .on_hover_text("Highlight them in the editor");
            }
            ui.separator();
            let saved = match (&self.dirty, &self.saved) {
                (Some(_), _) => "Saving…".to_string(),
                (None, Some(t)) => format!("Autosaved {t}"),
                (None, None) => "Autosave on".to_string(),
            };
            ui.weak(saved).on_hover_text(
                path().map_or("No profile folder".into(), |p| p.display().to_string()),
            );
            if ui.available_size_before_wrap().x <= 340.0 {
                ui.end_row();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let empty = self.is_empty();
                if widgets::confirm_button(ui, "notes_clear", !empty, "Clear", "Empty the notes") {
                    self.text.clear();
                    self.changed(now);
                }
                ui.add_enabled_ui(!empty, |ui| {
                    if ui
                        .button("Save .md…")
                        .on_hover_text("Save as Markdown, redacted")
                        .clicked()
                    {
                        // Dated, so one visit's notes don't land on another's.
                        widgets::save_file(
                            format!("notes-{}.md", chrono::Local::now().format("%Y-%m-%d")),
                            "Markdown",
                            vec!["md"],
                            self.sanitized_plain(),
                        );
                    }
                    if widgets::confirm_button(
                        ui,
                        "notes_raw_copy",
                        true,
                        "Raw copy",
                        "Copy as written, nothing hidden",
                    ) {
                        widgets::copy(ui.ctx(), crate::theme::strip_emoji_styles(&self.text));
                        widgets::toast("Copied unredacted", false);
                    }
                    if ui.button("Copy").on_hover_text("Copy, redacted").clicked() {
                        widgets::copy(ui.ctx(), self.sanitized_plain());
                        widgets::toast("Notes copied", true);
                    }
                });
                if ui
                    .button("Open…")
                    .on_hover_text("Open a text or config file to read")
                    .clicked()
                {
                    self.open(ui.ctx());
                }
            });
        });
    }

    /// What was redacted or kept by hand, each with a way back.
    fn overrides_menu(&mut self, ui: &mut Ui) {
        let n = self.ov.redact.len() + self.ov.allow.len();
        if n == 0 {
            return;
        }
        let mut changed = false;
        ui.menu_button(format!("By hand ({n})"), |ui| {
            if !self.ov.redact.is_empty() {
                ui.label(RichText::new("Redacted").strong());
                ui.weak("In memory only; after a restart just the marker remains.");
                let mut drop = None;
                for (i, v) in self.ov.redact.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("×")
                            .on_hover_text("Stop redacting it")
                            .clicked()
                        {
                            drop = Some(i);
                        }
                        let shown: String = v.chars().take(2).collect();
                        ui.monospace(format!("{shown}… ({} chars)", v.chars().count()));
                    });
                }
                if let Some(i) = drop {
                    self.ov.redact.remove(i);
                    changed = true;
                }
            }
            if !self.ov.allow.is_empty() {
                ui.label(RichText::new("Kept as written").strong());
                ui.weak("Shown in saves and copies.");
                let mut drop = None;
                for (i, v) in self.ov.allow.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("×")
                            .on_hover_text("Redact it again")
                            .clicked()
                        {
                            drop = Some(i);
                        }
                        let shown: String = v.chars().take(40).collect();
                        ui.monospace(shown);
                    });
                }
                if let Some(i) = drop {
                    self.ov.allow.remove(i);
                    changed = true;
                }
            }
        })
        .response
        .on_hover_text("Redacted or kept by hand");
        if changed {
            let now = ui.input(|i| i.time);
            self.changed(now);
        }
    }

    fn editor(&mut self, ui: &mut Ui, font: &FontId) {
        let id = egui::Id::new(EDITOR_ID);
        let focused = ui.memory(|m| m.has_focus(id));
        let ctx = ui.ctx().clone();
        if focused && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::R)) {
            let hits = hits_for(&self.cache, &self.text, &self.ov);
            if let Some(p) = self.pick(&ctx, &hits) {
                self.apply(&ctx, p);
            }
        }
        if focused {
            for (key, mark) in [(Key::B, "**"), (Key::I, "*")] {
                if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, key)) {
                    self.wrap(&ctx, mark);
                }
            }
            self.delete_emoji_marks(&ctx);
        }
        // Put back a selection the editor dropped while it hadn't the keyboard
        // (restyling from the emoji menu); forget it if it isn't getting it back.
        if focused && let Some(r) = self.reselect.take() {
            let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
            state.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(CharIndex(r.start)),
                CCursor::new(CharIndex(r.end)),
            )));
            state.store(&ctx, id);
        } else if !self.refocus {
            self.reselect = None;
        }
        let show = self.show_hits;
        let cache = &self.cache;
        let ov = &self.ov;
        let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            let text = buf.as_str();
            let hits: Vec<Range<usize>> = hits_for(cache, text, ov)
                .into_iter()
                .map(|h| h.span)
                .collect();
            let mut job = highlight(ui, text, if show { &hits } else { &[] }, font);
            job.wrap.max_width = wrap;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let keep = widgets::selection_before_right_click(&ctx, id);
        let out = egui::ScrollArea::vertical()
            .id_salt("notes_edit_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                let fill = ui.available_size();
                egui::TextEdit::multiline(&mut self.text)
                    .id(id)
                    .char_limit(crate::input::Field::Notes.room())
                    .desired_width(f32::INFINITY)
                    .min_size(fill)
                    .lock_focus(true)
                    .hint_text("Notes in Markdown. Select text and press Ctrl+R (or right-click, Redact) to redact it.")
                    .layouter(&mut layouter)
                    .show(ui)
            })
            .inner;
        if std::mem::take(&mut self.refocus) {
            out.response.request_focus();
        }
        if out.response.changed() {
            widgets::tidy(
                &ctx,
                &out.response,
                crate::input::Field::Notes,
                &mut self.text,
            );
            let now = ui.input(|i| i.time);
            self.changed(now);
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                self.continue_list(&ctx, &out);
            }
        }
        if out.response.has_focus() {
            self.step_over_emoji_marks(&ctx);
        }
        widgets::restore_selection(&ctx, id, &out.response, keep);
        out.response.context_menu(|ui| self.editor_menu(ui));
    }

    /// Right-click in the editor: the clipboard, formatting and Redact.
    fn editor_menu(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let id = egui::Id::new(EDITOR_ID);
        if widgets::text_edit_menu(ui, id, true) {
            ui.close();
            return;
        }
        ui.separator();
        let hits = hits_for(&self.cache, &self.text, &self.ov);
        let pick = self.pick(&ctx, &hits);
        let label = match &pick {
            Some(Pick::Unredact(_)) => "Unredact",
            _ => "Redact",
        };
        if ui
            .add_enabled(
                pick.is_some(),
                egui::Button::new(label).shortcut_text("Ctrl+R"),
            )
            .on_disabled_hover_text("Select text to redact")
            .clicked()
            && let Some(p) = pick
        {
            self.apply(&ctx, p);
            ui.close();
        }
        ui.separator();
        for (name, keys, mark) in [
            ("Bold", "Ctrl+B", "**"),
            ("Italic", "Ctrl+I", "*"),
            ("Inline code", "", "`"),
        ] {
            if ui
                .add(egui::Button::new(name).shortcut_text(keys))
                .clicked()
            {
                self.wrap(&ctx, mark);
                ui.close();
            }
        }
        for (name, prefix) in [
            ("Heading", "## "),
            ("Bullet list", "- "),
            ("Task box", "- [ ] "),
        ] {
            if ui.button(name).clicked() {
                self.line_prefix(&ctx, prefix);
                ui.close();
            }
        }
    }

    /// Enter on a list item starts the next one; on an empty item, ends the list.
    fn continue_list(&mut self, ctx: &egui::Context, out: &egui::text_edit::TextEditOutput) {
        let Some(r) = out.cursor_range else { return };
        let c = r.primary.index.0;
        let b = self
            .text
            .char_indices()
            .nth(c)
            .map_or(self.text.len(), |(i, _)| i);
        if !self.text[..b].ends_with('\n') {
            return;
        }
        let prev_start = self.text[..b - 1].rfind('\n').map_or(0, |i| i + 1);
        let prev = &self.text[prev_start..b - 1];
        let Some(marker) = markdown::continue_list(prev) else {
            return;
        };
        let id = egui::Id::new(EDITOR_ID);
        let mut state = out.state.clone();
        let to = if marker.is_empty() {
            // Empty item: drop its marker and stay on that line.
            let prev_chars = self.text[prev_start..b].chars().count();
            self.text.replace_range(prev_start..b, "\n");
            c - prev_chars + 1
        } else {
            self.text.insert_str(b, &marker);
            c + marker.chars().count()
        };
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(CharIndex(to)))));
        state.store(ctx, id);
    }

    fn preview(&mut self, ui: &mut Ui, font: &FontId) {
        let (clean, _) = self.sanitized();
        egui::ScrollArea::vertical()
            .id_salt("notes_preview_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                if clean.trim().is_empty() {
                    ui.weak("Formatted, as saved.");
                    return;
                }
                // Line numbers match the raw text, so a ticked box maps back.
                if let Some(line) = markdown::render(ui, &clean, font) {
                    markdown::toggle_task(&mut self.text, line);
                    let now = ui.input(|i| i.time);
                    self.changed(now);
                }
            });
    }
}

/// How many emoji start in `range` (bytes), not counting redaction markers' 🔑.
fn emoji_in(text: &str, range: Range<usize>) -> usize {
    let skip = redact::markers(text);
    text[range.clone()]
        .char_indices()
        .filter(|&(i, c)| {
            crate::theme::is_emoji(c) && !skip.iter().any(|m| m.contains(&(range.start + i)))
        })
        .count()
}

/// The byte where character `c` starts (the end, past the last one).
fn byte_at(text: &str, c: usize) -> usize {
    text.char_indices().nth(c).map_or(text.len(), |(i, _)| i)
}

/// The end of the emoji character at byte `i`, before its marks.
fn emoji_char_end(text: &str, i: usize) -> usize {
    i + text[i..].chars().next().map_or(0, char::len_utf8)
}

/// The picker's name for a style: an emoji with no mark follows the app.
fn notes_style_name(s: crate::theme::EmojiStyle) -> &'static str {
    match s {
        crate::theme::EmojiStyle::Theme => "Same as app",
        s => s.name(),
    }
}

fn notes_style_tip(s: crate::theme::EmojiStyle) -> &'static str {
    match s {
        crate::theme::EmojiStyle::Theme => "Follows the emoji style in Settings",
        crate::theme::EmojiStyle::Mono => "In the text's colour, whatever the app's style",
        crate::theme::EmojiStyle::Color => "Each in its own colour, whatever the app's style",
        crate::theme::EmojiStyle::Neon => "Miami's neons, whatever the app's style",
    }
}

/// Font and size menus, laid out right to left; saved when changed.
fn font_menus(ui: &mut Ui, look: &mut crate::theme::Settings) {
    let before = (look.notes_font.clone(), look.notes_size);
    egui::ComboBox::from_id_salt("notes_size")
        .width(44.0)
        .selected_text(look.notes_size.to_string())
        .show_ui(ui, |ui| {
            for &n in crate::theme::NOTES_SIZES {
                ui.selectable_value(&mut look.notes_size, n, n.to_string());
            }
        })
        .response
        .on_hover_text("Text size");
    egui::ComboBox::from_id_salt("notes_font")
        .width(130.0)
        .selected_text(look.notes_font.as_str())
        .show_ui(ui, |ui| {
            for name in crate::fonts::names() {
                ui.selectable_value(&mut look.notes_font, name.to_string(), name);
            }
        })
        .response
        .on_hover_text("Font (the monospace ones this computer has are listed too)");
    if (look.notes_font.clone(), look.notes_size) != before {
        crate::theme::save(look);
    }
}

/// An open file, read-only: line numbers, no wrapping, select and copy.
fn viewer(ui: &mut Ui, v: &Viewed, font: &FontId) {
    let numbers: String = (1..=v.lines.max(1)).map(|n| format!("{n}\n")).collect();
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
        let mut job = LayoutJob::simple(
            buf.as_str().to_string(),
            font.clone(),
            ui.visuals().text_color(),
            f32::INFINITY,
        );
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    egui::ScrollArea::both()
        .id_salt("notes_viewer")
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(numbers.trim_end())
                            .font(font.clone())
                            .color(ui.visuals().weak_text_color()),
                    )
                    .selectable(false),
                );
                ui.separator();
                let mut text = v.text.as_str();
                let id = egui::Id::new("notes_viewer_text");
                let keep = widgets::selection_before_right_click(ui.ctx(), id);
                let r = ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .id(id)
                        .font(font.clone())
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .layouter(&mut layouter),
                );
                widgets::restore_selection(ui.ctx(), id, &r, keep);
                r.context_menu(|ui| {
                    if widgets::text_edit_menu(ui, id, false) {
                        ui.close();
                    }
                });
            });
        });
}

/// Editor colouring: headings, list markers and code, and a highlight on
/// every value that will be redacted.
pub(crate) fn highlight(ui: &Ui, text: &str, hits: &[Range<usize>], font: &FontId) -> LayoutJob {
    const PLAIN: u8 = 0;
    const HEAD: u8 = 1;
    const MARK: u8 = 2;
    const CODE: u8 = 3;
    const DONE: u8 = 4;
    const HIT: u8 = 8;
    let mut style = vec![PLAIN; text.len()];
    let mut pos = 0;
    let mut fenced = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        let lead = line.len() - t.len();
        let span = pos..pos + line.len();
        if t.starts_with("```") {
            style[span].fill(CODE);
            fenced = !fenced;
        } else if fenced {
            style[span].fill(CODE);
        } else if t.starts_with('#') && t.trim_start_matches('#').starts_with(' ') {
            style[span].fill(HEAD);
        } else {
            let marker = ["- [ ] ", "- [x] ", "- [X] ", "- ", "* ", "+ ", "> "]
                .iter()
                .find(|m| t.starts_with(**m))
                .map(|m| m.len())
                .or_else(|| {
                    let d = t.chars().take_while(char::is_ascii_digit).count();
                    (d > 0 && t[d..].starts_with(". ")).then_some(d + 2)
                });
            if let Some(m) = marker {
                style[pos + lead..pos + lead + m].fill(MARK);
            }
            // `inline code`
            let mut open = None;
            for (i, c) in line.char_indices() {
                if c == '`' {
                    match open.take() {
                        Some(s) => style[pos + s..=pos + i].fill(CODE),
                        None => open = Some(i),
                    }
                }
            }
        }
        pos += line.len();
    }
    for m in redact::markers(text) {
        style[m].fill(DONE);
    }
    for h in hits {
        for s in &mut style[h.clone()] {
            *s |= HIT;
        }
    }
    // An emoji's invisible marks share its section, so its style holds.
    for (i, c) in text.char_indices() {
        if crate::theme::is_emoji(c) {
            let (end, _) = crate::theme::emoji_end(text, i);
            let s = style[i];
            style[i..end].fill(s);
        }
    }
    let vis = ui.visuals();
    let font = font.clone();
    let warn = widgets::warn();
    let mut job = LayoutJob::default();
    let mut start = 0;
    for i in 1..=text.len() {
        if i < text.len() && (style[i] == style[start] || !text.is_char_boundary(i)) {
            continue;
        }
        let s = style[start];
        let mut f = TextFormat::simple(
            font.clone(),
            match s & !HIT {
                HEAD => crate::theme::accent(),
                MARK => crate::theme::accent(),
                CODE => vis.weak_text_color(),
                DONE => widgets::warn().gamma_multiply(0.8),
                _ => vis.text_color(),
            },
        );
        if s & !HIT == HEAD {
            f.font_id = FontId::new(font.size + 1.0, font.family.clone());
        }
        if s & !HIT == CODE {
            f.background = vis.code_bg_color;
        }
        if s & HIT != 0 {
            f.background = warn.gamma_multiply(0.30);
            f.underline = Stroke::new(1.0, warn);
            f.color = if vis.dark_mode {
                Color32::WHITE
            } else {
                Color32::BLACK
            };
        }
        crate::theme::append_with_emoji(&mut job, &text[start..i], &f);
        start = i;
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes(text: &str) -> NotesTool {
        NotesTool {
            text: text.into(),
            view: View::Split,
            show_hits: true,
            dirty: None,
            saved: None,
            err: None,
            known_hits: 0,
            flash: 0.0,
            ov: Default::default(),
            cache: RefCell::new((u64::MAX, Vec::new())),
            clean: RefCell::new((u64::MAX, (String::new(), Vec::new()))),
            file: None,
            on_file: false,
            opening: None,
            refocus: false,
            picker_sel: None,
            picker_restyled: false,
            reselect: None,
        }
    }

    fn select(ctx: &egui::Context, n: &NotesTool, what: &str) {
        let at = n.text[..n.text.find(what).unwrap()].chars().count();
        let id = egui::Id::new(EDITOR_ID);
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(CharIndex(at)),
            CCursor::new(CharIndex(at + what.chars().count())),
        )));
        state.store(ctx, id);
    }

    #[test]
    fn the_debug_files_open_as_they_say() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/notes-debug");
        let open = |n: &str| read_text(&dir.join(n));
        let crlf = open("viewer-crlf-tabs.txt").unwrap();
        assert!(!crlf.text.contains('\r') && crlf.text.contains("X0\t1\t10.20.0.1"));
        assert!(crlf.text.lines().any(|l| l.len() > 2000), "the long line");
        let bom = open("viewer-utf8-bom.cfg").unwrap();
        assert!(bom.text.starts_with("# viewer-utf8-bom.cfg"));
        for f in ["viewer-utf16le.txt", "viewer-utf16be.txt"] {
            let t = open(f).unwrap().text;
            assert!(t.starts_with(f) && t.contains("emoji 🔥"), "{f}: {t}");
        }
        let e = open("viewer-binary.bin").err().unwrap();
        assert!(e.contains("not a text file"), "{e}");
        // The checklist itself draws in Preview.
        let md = std::fs::read_to_string(dir.join("notes-debug.md")).unwrap();
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            crate::markdown::render(ui, &md, &FontId::proportional(13.0));
        });
        out.textures_delta.clear();
    }

    #[test]
    fn opens_text_files_and_refuses_binary_ones() {
        let dir = std::env::temp_dir().join(format!("is-alive-view-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |n: &str, d: &[u8]| {
            let p = dir.join(n);
            std::fs::write(&p, d).unwrap();
            p
        };
        let v = read_text(&write(
            "a.cfg",
            b"\xef\xbb\xbfhostname fw1\r\ninterface X0\r\n",
        ))
        .unwrap();
        assert_eq!(
            (v.text.as_str(), v.lines),
            ("hostname fw1\ninterface X0\n", 2)
        );
        let mut wide = vec![0xff, 0xfe];
        wide.extend(
            "[global]\nname=x\n"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        assert_eq!(
            read_text(&write("b.ini", &wide)).unwrap().text,
            "[global]\nname=x\n"
        );
        let e = read_text(&write("c.bin", b"MZ\x90\x00\x03")).err().unwrap();
        assert!(e.contains("not a text file"), "{e}");
        let big = write("d.log", &vec![b'a'; (VIEW_MAX + 1) as usize]);
        assert!(read_text(&big).err().unwrap().contains("up to 5 MB"));
        // Drawn without trouble, line numbers and all.
        let v = read_text(&write("e.txt", b"one\ntwo\nthree")).unwrap();
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            viewer(ui, &v, &FontId::monospace(13.0));
        });
        out.textures_delta.clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn secrets_from_a_loaded_backup_are_redacted() {
        let mut n = notes("staff wifi: see board. Bobcat2019 works");
        assert!(n.sanitized().0.contains("Bobcat2019"));
        n.set_known(&["Bobcat2019".into()], 1.0);
        assert!(n.dirty.is_some(), "the saved copy is rewritten");
        let clean = n.sanitized().0;
        assert!(
            !clean.contains("Bobcat2019") && !clean.contains("4471"),
            "{clean}"
        );
        assert!(clean.contains("[🔑 REDACTED: from config]"));
    }

    #[test]
    fn redact_and_unredact_by_hand() {
        let ctx = egui::Context::default();
        let mut n = notes("Lockbox 4471, again 4471\nOffice365\npassword: hunter2");
        let hits = |n: &NotesTool| hits_for(&n.cache, &n.text, &n.ov);
        // Nothing is guessed: only the loaded config's secret is redacted.
        assert!(hits(&n).is_empty());
        n.set_known(&["Office365".into()], 0.0);
        assert_eq!(hits(&n).len(), 1);

        // Select a value: Redact, everywhere.
        select(&ctx, &n, "4471");
        let p = n.pick(&ctx, &hits(&n)).unwrap();
        assert!(matches!(p, Pick::Redact(_)));
        n.apply(&ctx, p);
        assert_eq!(hits(&n).len(), 3);
        assert!(
            n.sanitized()
                .0
                .starts_with("Lockbox [🔑 REDACTED: by hand], again [🔑 REDACTED: by hand]")
        );

        // Cursor on a config value that's fine to show: Unredact keeps it.
        select(&ctx, &n, "fice");
        let p = n.pick(&ctx, &hits(&n)).unwrap();
        assert!(matches!(p, Pick::Unredact(_)));
        n.apply(&ctx, p);
        assert!(n.sanitized().0.contains("\nOffice365\n"));
        assert_eq!(n.ov.allow, ["Office365"]);

        // A hand pick is undone the same way.
        select(&ctx, &n, "4471");
        let p = n.pick(&ctx, &hits(&n)).unwrap();
        n.apply(&ctx, p);
        assert!(n.ov.redact.is_empty());
        assert!(n.sanitized().0.ends_with("password: hunter2"));
        // Nothing selected and no hit under the cursor: nothing to do.
        select(&ctx, &n, "");
        assert!(n.pick(&ctx, &hits(&n)).is_none());
    }

    #[test]
    fn emoji_styles_stay_in_the_notes_but_not_in_copies() {
        let n = notes("fix 🔥\u{FE02} now ✅\u{FE0F}\u{FE01}, pass: x");
        // Autosave and Preview keep the marks.
        assert!(
            n.sanitized()
                .0
                .starts_with("fix 🔥\u{FE02} now ✅\u{FE0F}\u{FE01}")
        );
        assert_eq!(n.sanitized_plain(), "fix 🔥 now ✅\u{FE0F}, pass: x");
    }

    #[test]
    fn a_style_picked_with_text_selected_restyles_its_emoji() {
        let ctx = egui::Context::default();
        let mut n = notes("🔥 a ✅\u{FE00} [🔑 REDACTED: by hand] b ⭐");
        select(&ctx, &n, "a ✅\u{FE00} [🔑 REDACTED: by hand] b ⭐");
        let sel = widgets::text_selection(&ctx, egui::Id::new(EDITOR_ID), &n.text).unwrap();
        assert_eq!(emoji_in(&n.text, sel.clone()), 2, "not the marker's 🔑");
        let now = n.restyle(&ctx, sel, crate::theme::EmojiStyle::Color);
        assert_eq!(
            &n.text[now],
            "a ✅\u{FE01} [🔑 REDACTED: by hand] b ⭐\u{FE01}"
        );
        assert_eq!(
            n.text,
            "🔥 a ✅\u{FE01} [🔑 REDACTED: by hand] b ⭐\u{FE01}"
        );
        // Still selected, to the end.
        let sel = widgets::text_selection(&ctx, egui::Id::new(EDITOR_ID), &n.text).unwrap();
        assert_eq!(
            &n.text[sel],
            "a ✅\u{FE01} [🔑 REDACTED: by hand] b ⭐\u{FE01}"
        );
    }

    /// One frame of the editor, focused, with `keys` pressed.
    fn type_keys(ctx: &egui::Context, n: &mut NotesTool, keys: &[Key]) {
        let events = keys.iter().map(|&k| press(k, Modifiers::NONE)).collect();
        frame(ctx, n, events);
    }

    fn press(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// One frame of the editor, focused, with `events`.
    fn frame(ctx: &egui::Context, n: &mut NotesTool, mut events: Vec<egui::Event>) {
        let held = events.iter().fold(Modifiers::NONE, |m, e| match e {
            egui::Event::Key { modifiers, .. } => m | *modifiers,
            _ => m,
        });
        events.insert(0, egui::Event::ModifiersChanged(held));
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(EDITOR_ID)));
        let mut out = ctx.run_ui(input, |ui| n.editor(ui, &FontId::monospace(13.0)));
        out.textures_delta.clear();
    }

    fn cursor(ctx: &egui::Context) -> usize {
        let state = egui::TextEdit::load_state(ctx, egui::Id::new(EDITOR_ID)).unwrap();
        state.cursor.char_range().unwrap().primary.index.0
    }

    fn put_cursor(ctx: &egui::Context, at: usize) {
        let id = egui::Id::new(EDITOR_ID);
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        let c = CCursor::new(CharIndex(at));
        state.cursor.set_char_range(Some(CCursorRange::one(c)));
        state.store(ctx, id);
    }

    #[test]
    fn an_emoji_and_its_marks_go_together() {
        let ctx = egui::Context::default();
        let mut n = notes("a🔥\u{FE0F}\u{FE02}b");
        type_keys(&ctx, &mut n, &[]);
        // Backspace after the marks takes the emoji too.
        put_cursor(&ctx, 4);
        type_keys(&ctx, &mut n, &[Key::Backspace]);
        assert_eq!((n.text.as_str(), cursor(&ctx)), ("ab", 1));
        // Delete before it, the same.
        n.text = "a🔥\u{FE01}b".into();
        put_cursor(&ctx, 1);
        type_keys(&ctx, &mut n, &[Key::Delete]);
        assert_eq!(n.text, "ab");
        // A cursor left between an emoji and its mark steps past it, or
        // before the emoji going left.
        n.text = "a🔥\u{FE01}b".into();
        put_cursor(&ctx, 2);
        type_keys(&ctx, &mut n, &[]);
        assert_eq!(cursor(&ctx), 3);
        put_cursor(&ctx, 3);
        type_keys(&ctx, &mut n, &[Key::ArrowLeft]);
        type_keys(&ctx, &mut n, &[]);
        assert_eq!(cursor(&ctx), 1);
    }

    #[test]
    fn highlight_covers_the_text_on_char_boundaries() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let t = "# Héading\n- pass: ünïcode\n`x` done";
            let hits: Vec<_> = redact::find(t).into_iter().map(|h| h.span).collect();
            assert_eq!(hits.len(), 1);
            let job = highlight(ui, t, &hits, &FontId::monospace(13.0));
            assert_eq!(job.text, t);
        });
        out.textures_delta.clear();
    }

    fn selection(ctx: &egui::Context) -> (usize, usize) {
        let state = egui::TextEdit::load_state(ctx, egui::Id::new(EDITOR_ID)).unwrap();
        let r = state.cursor.char_range().unwrap();
        (r.secondary.index.0, r.primary.index.0)
    }

    #[test]
    fn a_selection_takes_an_emoji_with_its_marks_or_not_at_all() {
        let ctx = egui::Context::default();
        let (shift, ctrl) = (Modifiers::SHIFT, Modifiers::CTRL);
        let mut n = notes("a🔥\u{FE02}b");
        type_keys(&ctx, &mut n, &[]);
        // Shift+Right over the emoji selects its mark too: deleting or
        // typing over it leaves no stray mark (other programs would get it).
        put_cursor(&ctx, 1);
        frame(&ctx, &mut n, vec![press(Key::ArrowRight, shift)]);
        assert_eq!(selection(&ctx), (1, 3));
        type_keys(&ctx, &mut n, &[Key::Delete]);
        assert_eq!(
            (n.text.as_str(), n.sanitized_plain().as_str()),
            ("ab", "ab")
        );
        n.text = "a🔥\u{FE02}b".into();
        put_cursor(&ctx, 1);
        frame(&ctx, &mut n, vec![press(Key::ArrowRight, shift)]);
        frame(&ctx, &mut n, vec![egui::Event::Text("X".into())]);
        assert_eq!(n.text, "aXb");
        // Shift+Left from after it selects the emoji, not just its mark,
        // so Backspace doesn't silently drop its style.
        n.text = "c🔥\u{FE01}d".into();
        put_cursor(&ctx, 3);
        frame(&ctx, &mut n, vec![press(Key::ArrowLeft, shift)]);
        assert_eq!(selection(&ctx), (3, 1));
        type_keys(&ctx, &mut n, &[Key::Backspace]);
        assert_eq!(n.text, "cd");
        // Nor does the mark move onto the emoji before.
        n.text = "✅🔥\u{FE01}".into();
        put_cursor(&ctx, 1);
        frame(&ctx, &mut n, vec![press(Key::ArrowRight, shift)]);
        type_keys(&ctx, &mut n, &[Key::Delete]);
        assert_eq!(n.text, "✅");
        // A selection's far end there moves past the marks too.
        n.text = "a🔥\u{FE01}b".into();
        select(&ctx, &n, "a🔥");
        type_keys(&ctx, &mut n, &[]);
        assert_eq!(selection(&ctx), (0, 3));
        // Words take an emoji's marks with it; Ctrl+H is Backspace.
        n.text = "word 🔥\u{FE02} x".into();
        put_cursor(&ctx, 5);
        frame(&ctx, &mut n, vec![press(Key::Delete, ctrl)]);
        assert_eq!(n.text, "word  x");
        n.text = "word 🔥\u{FE02}".into();
        put_cursor(&ctx, 7);
        frame(&ctx, &mut n, vec![press(Key::Backspace, ctrl)]);
        assert_eq!(n.text, "word ");
        n.text = "word 🔥\u{FE02}".into();
        put_cursor(&ctx, 7);
        frame(&ctx, &mut n, vec![press(Key::H, ctrl)]);
        assert_eq!(n.text, "word ");
    }

    #[test]
    fn enter_on_an_empty_item_stays_before_the_next_lines_emoji() {
        let ctx = egui::Context::default();
        let mut n = notes("- a\n- \n✅\u{FE01} done");
        type_keys(&ctx, &mut n, &[]);
        put_cursor(&ctx, 6);
        type_keys(&ctx, &mut n, &[Key::Enter]);
        type_keys(&ctx, &mut n, &[]);
        assert_eq!(
            (n.text.as_str(), cursor(&ctx)),
            ("- a\n\n\n✅\u{FE01} done", 5)
        );
    }

    #[test]
    fn restyling_keeps_the_selection_through_the_editor() {
        use crate::theme::EmojiStyle;
        let ctx = egui::Context::default();
        let id = egui::Id::new(EDITOR_ID);
        // Just the emoji selected, not its mark: the selection still
        // covers it once the mark has gone.
        let mut n = notes("🔥\u{FE00}x");
        let now = n.restyle(&ctx, 0..'🔥'.len_utf8(), EmojiStyle::Theme);
        assert_eq!((n.text.as_str(), &n.text[now]), ("🔥x", "🔥"));
        // The menu has the keyboard: the editor drops the selection, then
        // puts it back once it has the keyboard again.
        let mut n = notes("a 🔥 b ✅");
        let all = 0..n.text.len();
        let now = n.restyle(&ctx, all, EmojiStyle::Color);
        assert!(n.refocus);
        ctx.memory_mut(|m| m.surrender_focus(id));
        let mut out = ctx.run_ui(Default::default(), |ui| {
            n.editor(ui, &FontId::monospace(13.0))
        });
        out.textures_delta.clear();
        assert!(ctx.memory(|m| m.has_focus(id)));
        type_keys(&ctx, &mut n, &[]);
        assert_eq!(
            widgets::text_selection(&ctx, id, &n.text),
            Some(now.clone())
        );
        assert_eq!(&n.text[now], "a 🔥\u{FE01} b ✅\u{FE01}");
        // Picking the style they already have changes nothing.
        n.dirty = None;
        let all = 0..n.text.len();
        assert_eq!(n.restyle(&ctx, all.clone(), EmojiStyle::Color), all);
        assert_eq!(n.dirty, None);
    }
}
