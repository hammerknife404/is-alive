//! Tech notes: a Markdown scratchpad for the visit. Whatever leaves the
//! editor (autosave, Copy, Save, Site Bundle) is the sanitized text, and
//! the editor highlights every value that will be redacted. The raw text
//! lives only in memory: after a restart the notes come back sanitized.

use crate::markdown;
use crate::net::hostinfo::HostInfo;
use crate::redact::{self, Hit};
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
    clear_armed: Option<f64>,
    /// (hash of text, hits) so the regexes run once per edit.
    cache: RefCell<(u64, Vec<Hit>)>,
}

fn path() -> Option<PathBuf> {
    Some(crate::theme::data_dir()?.join("notes.md"))
}

impl Default for NotesTool {
    fn default() -> Self {
        let text = path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        Self {
            text,
            view: View::Split,
            show_hits: true,
            dirty: None,
            saved: None,
            err: None,
            known_hits: 0,
            flash: f64::NEG_INFINITY,
            clear_armed: None,
            cache: RefCell::new((u64::MAX, Vec::new())),
        }
    }
}

fn hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn hits_for(cache: &RefCell<(u64, Vec<Hit>)>, text: &str) -> Vec<Hit> {
    let h = hash(text);
    let mut c = cache.borrow_mut();
    if c.0 != h {
        *c = (h, redact::find(text));
    }
    c.1.clone()
}

impl NotesTool {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// The notes as they may leave this machine.
    pub fn sanitized(&self) -> (String, Vec<Hit>) {
        redact::sanitize(&self.text)
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
            .and_then(|_| std::fs::write(&tmp, clean))
            .and_then(|_| std::fs::rename(&tmp, &p));
        match r {
            Ok(()) => {
                self.saved = Some(chrono::Local::now().format("%H:%M").to_string());
                self.err = None;
            }
            Err(e) => self.err = Some(format!("Couldn't autosave notes: {e}")),
        }
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
        ctx.memory_mut(|m| m.request_focus(id));
        self.changed(ctx.input(|i| i.time));
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
        ctx.memory_mut(|m| m.request_focus(id));
        self.changed(ctx.input(|i| i.time));
    }

    pub fn ui(&mut self, ui: &mut Ui, info: &HostInfo) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        if self.dirty.is_some_and(|t| now - t >= SAVE_AFTER) {
            self.write();
        }
        if let Some(t) = self.dirty {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                (SAVE_AFTER - (now - t)).max(0.05),
            ));
        }
        let hits = hits_for(&self.cache, &self.text);
        if hits.len() > self.known_hits {
            self.flash = now;
        }
        self.known_hits = hits.len();

        self.toolbar(ui, info);
        self.status(ui, &hits, now);
        widgets::error(ui, &self.err);
        ui.separator();

        let narrow = ui.available_width() < 720.0;
        let view = if narrow && self.view == View::Split {
            View::Edit
        } else {
            self.view
        };
        match view {
            View::Edit => self.editor(ui),
            View::Preview => self.preview(ui),
            View::Split => {
                ui.columns(2, |c| {
                    self.editor(&mut c[0]);
                    self.preview(&mut c[1]);
                });
            }
        }
    }

    fn toolbar(&mut self, ui: &mut Ui, info: &HostInfo) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            for (v, name, tip) in [
                (View::Edit, "Edit", "Just the editor"),
                (View::Split, "Split", "Editor and preview side by side"),
                (
                    View::Preview,
                    "Preview",
                    "The notes as they'll be exported: sanitized and formatted",
                ),
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
            let wrap = |s: &mut Self, mark: &'static str| {
                s.edit_at_cursor(&ctx, |sel| {
                    let n = mark.chars().count();
                    let len = sel.chars().count();
                    (
                        format!("{mark}{sel}{mark}"),
                        if len == 0 { n } else { len + 2 * n },
                    )
                })
            };
            if ui
                .button(RichText::new("B").strong())
                .on_hover_text("Bold (Ctrl+B)")
                .clicked()
            {
                wrap(self, "**");
            }
            if ui
                .button(RichText::new("I").italics())
                .on_hover_text("Italic (Ctrl+I)")
                .clicked()
            {
                wrap(self, "*");
            }
            if ui
                .button(RichText::new("</>").monospace())
                .on_hover_text("Inline code")
                .clicked()
            {
                wrap(self, "`");
            }
            if ui.button("H").on_hover_text("Heading").clicked() {
                self.line_prefix(&ctx, "## ");
            }
            if ui
                .button("•")
                .on_hover_text("Bullet list (Enter continues it)")
                .clicked()
            {
                self.line_prefix(&ctx, "- ");
            }
            if ui
                .button("☐")
                .on_hover_text("Task box: tick it in the preview")
                .clicked()
            {
                self.line_prefix(&ctx, "- [ ] ");
            }
            ui.separator();
            if ui
                .button("Time")
                .on_hover_text("Insert the date and time")
                .clicked()
            {
                let stamp = chrono::Local::now()
                    .format("**%Y-%m-%d %H:%M** ")
                    .to_string();
                let n = stamp.chars().count();
                self.edit_at_cursor(&ctx, |_| (stamp, n));
            }
            if ui
                .button("Host info")
                .on_hover_text("Insert this machine's LAN, gateway and WAN")
                .clicked()
            {
                let line = format!(
                    "{} · {}\n",
                    chrono::Local::now().format("%Y-%m-%d %H:%M"),
                    info.summary().lines().collect::<Vec<_>>().join(" · ")
                );
                let n = line.chars().count();
                self.edit_at_cursor(&ctx, |_| (line, n));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let empty = self.is_empty();
                let now = ui.input(|i| i.time);
                let armed = self.clear_armed.is_some_and(|t| now - t < 3.0);
                let clear = if armed {
                    egui::Button::new(RichText::new("Clear: sure?").color(widgets::bad()))
                } else {
                    egui::Button::new("Clear")
                };
                if ui
                    .add_enabled(!empty, clear)
                    .on_hover_text("Empty the notes (click twice)")
                    .clicked()
                {
                    if armed {
                        self.text.clear();
                        self.clear_armed = None;
                        self.changed(now);
                    } else {
                        self.clear_armed = Some(now);
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_secs(3));
                    }
                }
                ui.add_enabled_ui(!empty, |ui| {
                    widgets::save_button(ui, "notes", || self.sanitized().0);
                    widgets::copy_button(ui, || self.sanitized().0);
                });
                ui.weak("sanitized:");
            });
        });
    }

    /// Word count, what will be redacted (flashing when it grows), and the
    /// autosave state.
    fn status(&mut self, ui: &mut Ui, hits: &[Hit], now: f64) {
        ui.horizontal(|ui| {
            let words = self.text.split_whitespace().count();
            ui.weak(format!(
                "{words} word{} · {} line{}",
                if words == 1 { "" } else { "s" },
                self.text.lines().count(),
                if self.text.lines().count() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
            ui.separator();
            if hits.is_empty() {
                ui.label(
                    RichText::new("✔ nothing to redact")
                        .color(widgets::ok())
                        .small(),
                );
            } else {
                let age = (now - self.flash) as f32;
                let warn = widgets::warn();
                let glow = if age < 2.0 {
                    ui.ctx().request_repaint();
                    0.5 + 0.5 * (age * 6.0).cos().abs() * (1.0 - age / 2.0)
                } else {
                    0.0
                };
                let text = RichText::new(format!("⚠ {}", redact::summary(hits))).color(warn);
                let r = egui::Frame::new()
                    .fill(warn.gamma_multiply(0.10 + 0.25 * glow))
                    .corner_radius(4)
                    .inner_margin(egui::Margin::symmetric(6, 1))
                    .show(ui, |ui| ui.label(text))
                    .response;
                let mut tip = "Replaced with [REDACTED: …] in everything that leaves the editor: \
autosave, Copy, Save and Site Bundle."
                    .to_string();
                for h in hits.iter().take(12) {
                    let s: String = self.text[h.span.clone()].chars().take(3).collect();
                    tip += &format!("\n• {}: {s}…", h.kind.label());
                }
                r.on_hover_text(tip);
                ui.checkbox(&mut self.show_hits, "Highlight")
                    .on_hover_text("Mark each value that will be redacted in the editor");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = match (&self.dirty, &self.saved) {
                    (Some(_), _) => "Saving…".to_string(),
                    (None, Some(t)) => format!("Autosaved {t} (sanitized)"),
                    (None, None) => "Autosave keeps a sanitized copy".to_string(),
                };
                ui.weak(label).on_hover_text(format!(
                    "Saved to {} with sensitive values replaced, so nothing redactable is ever \
written to disk. After a restart the notes come back sanitized.",
                    path().map_or("(no profile folder)".into(), |p| p.display().to_string())
                ));
            });
        });
    }

    fn editor(&mut self, ui: &mut Ui) {
        let id = egui::Id::new(EDITOR_ID);
        let focused = ui.memory(|m| m.has_focus(id));
        let ctx = ui.ctx().clone();
        if focused {
            for (key, mark) in [(Key::B, "**"), (Key::I, "*")] {
                if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, key)) {
                    self.edit_at_cursor(&ctx, |sel| {
                        let n = mark.chars().count();
                        let len = sel.chars().count();
                        (
                            format!("{mark}{sel}{mark}"),
                            if len == 0 { n } else { len + 2 * n },
                        )
                    });
                }
            }
        }
        let show = self.show_hits;
        let cache = &self.cache;
        let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            let text = buf.as_str();
            let hits: Vec<Range<usize>> =
                hits_for(cache, text).into_iter().map(|h| h.span).collect();
            let mut job = highlight(ui, text, if show { &hits } else { &[] });
            job.wrap.max_width = wrap;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let out = egui::ScrollArea::vertical()
            .id_salt("notes_edit_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                let fill = ui.available_size();
                egui::TextEdit::multiline(&mut self.text)
                    .id(id)
                    .desired_width(f32::INFINITY)
                    .min_size(fill)
                    .lock_focus(true)
                    .hint_text(
                        "Site notes in Markdown: # headings, - lists, - [ ] tasks, **bold**, `code`.\n\
Passwords, keys, tokens and patient details are highlighted here and replaced before anything is \
saved, copied or bundled.",
                    )
                    .layouter(&mut layouter)
                    .show(ui)
            })
            .inner;
        if out.response.changed() {
            let now = ui.input(|i| i.time);
            self.changed(now);
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                self.continue_list(&ctx, &out);
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

    fn preview(&mut self, ui: &mut Ui) {
        let (clean, _) = self.sanitized();
        egui::ScrollArea::vertical()
            .id_salt("notes_preview_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                if clean.trim().is_empty() {
                    ui.weak("The preview shows the notes as they'll leave this machine.");
                    return;
                }
                // Line numbers match the raw text, so a ticked box maps back.
                if let Some(line) = markdown::render(ui, &clean) {
                    markdown::toggle_task(&mut self.text, line);
                    let now = ui.input(|i| i.time);
                    self.changed(now);
                }
            });
    }
}

/// Editor colouring: headings, list markers and code, and a highlight on
/// every value that will be redacted.
fn highlight(ui: &Ui, text: &str, hits: &[Range<usize>]) -> LayoutJob {
    const PLAIN: u8 = 0;
    const HEAD: u8 = 1;
    const MARK: u8 = 2;
    const CODE: u8 = 3;
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
    for h in hits {
        for s in &mut style[h.clone()] {
            *s |= HIT;
        }
    }
    let vis = ui.visuals();
    let font = FontId::monospace(13.0);
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
                _ => vis.text_color(),
            },
        );
        if s & !HIT == HEAD {
            f.font_id = FontId::monospace(14.0);
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
        job.append(&text[start..i], 0.0, f);
        start = i;
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_covers_the_text_on_char_boundaries() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let t = "# Héading\n- pass: ünïcode\n`x` done";
            let hits: Vec<_> = redact::find(t).into_iter().map(|h| h.span).collect();
            assert_eq!(hits.len(), 1);
            let job = highlight(ui, t, &hits);
            assert_eq!(job.text, t);
        });
        out.textures_delta.clear();
    }
}
