//! Small shared UI pieces.

use crate::input::Field;
use eframe::egui::{self, Color32, RichText, Ui};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Toasts: a short confirmation at the bottom of the window ("Copied",
/// "Saved notes.md"), or an error. Any thread can raise one.
static TOASTS: Mutex<Vec<(String, bool, Instant)>> = Mutex::new(Vec::new());
static CTX: OnceLock<egui::Context> = OnceLock::new();
const TOAST_FOR: f32 = 2.4;

/// Hash of the last text this app put on the clipboard, so exit clears
/// only our own copy, never something copied from another app since.
static LAST_COPY: Mutex<Option<u64>> = Mutex::new(None);

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Put `text` on the clipboard. On Windows it's marked to stay out of
/// clipboard history (Win+V), cloud clipboard sync and clipboard managers,
/// as password managers do.
pub fn copy(ctx: &egui::Context, text: String) {
    if let Ok(mut l) = LAST_COPY.lock() {
        *l = Some(hash(&text));
    }
    #[cfg(windows)]
    {
        use arboard::SetExtWindows;
        let done = arboard::Clipboard::new()
            .and_then(|mut c| c.set().exclude_from_monitoring().text(text.clone()));
        if done.is_ok() {
            return;
        }
    }
    ctx.copy_text(text);
}

/// On exit: clear the clipboard if it still holds what this app copied.
pub fn clear_own_clipboard() {
    let Some(ours) = LAST_COPY.lock().ok().and_then(|l| *l) else {
        return;
    };
    if let Ok(mut c) = arboard::Clipboard::new()
        && c.get_text().is_ok_and(|t| hash(&t) == ours)
    {
        let _ = c.clear();
    }
}

/// Let other threads wake the UI for a toast; call once at startup.
pub fn init(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
}

pub fn toast(text: impl Into<String>, ok: bool) {
    let text = text.into();
    if let Ok(mut t) = TOASTS.lock() {
        // The same message again (a paste repeated) shows once, afresh.
        t.retain(|x| x.0 != text || x.1 != ok);
        t.push((text, ok, Instant::now()));
        t.retain(|x| x.2.elapsed().as_secs_f32() < TOAST_FOR);
        let n = t.len();
        if n > 3 {
            t.drain(..n - 3);
        }
    }
    if let Some(c) = CTX.get() {
        c.request_repaint();
    }
}

/// Draw the toasts over everything, newest at the bottom; call once a frame.
pub fn show_toasts(ctx: &egui::Context) {
    let Ok(mut list) = TOASTS.lock() else { return };
    list.retain(|x| x.2.elapsed().as_secs_f32() < TOAST_FOR);
    if list.is_empty() {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("toasts"),
    ));
    let vis = ctx.global_style().visuals.clone();
    let screen = ctx.content_rect();
    let mut y = screen.bottom() - 40.0;
    for (text, good, at) in list.iter().rev() {
        let age = at.elapsed().as_secs_f32();
        // In over 0.15 s, out over the last 0.4 s, rising a few points.
        let a = (age / 0.15).min(1.0) * ((TOAST_FOR - age) / 0.4).clamp(0.0, 1.0);
        let rise = 6.0 * (1.0 - (age / 0.15).min(1.0));
        let (icon, colour) = if *good { ("✔", ok()) } else { ("⚠", bad()) };
        let g = painter.layout_no_wrap(
            format!("{icon}  {text}"),
            egui::FontId::proportional(13.5),
            vis.strong_text_color(),
        );
        let r = egui::Rect::from_center_size(
            egui::pos2(screen.center().x, y + rise),
            g.size() + egui::vec2(28.0, 14.0),
        );
        painter.add(
            egui::Shadow {
                offset: [0, 4],
                blur: 16,
                spread: 0,
                color: Color32::from_black_alpha((80.0 * a) as u8),
            }
            .as_shape(r, 10),
        );
        painter.rect(
            r,
            10,
            vis.window_fill.gamma_multiply(a),
            egui::Stroke::new(1.0, colour.gamma_multiply(0.7 * a)),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            r.min + egui::vec2(14.0, 7.0),
            g,
            vis.strong_text_color().gamma_multiply(a),
        );
        y = r.top() - 22.0;
    }
    ctx.request_repaint_after(Duration::from_millis(16));
}

/// Status colours, tuned per theme so they stay readable.
pub fn ok() -> Color32 {
    crate::theme::status().0
}

pub fn warn() -> Color32 {
    crate::theme::status().1
}

pub fn bad() -> Color32 {
    crate::theme::status().2
}

pub enum Action {
    None,
    Start,
    Stop,
}

/// Start/Stop button pair. `enter` lets a text field's Enter key start the run;
/// Esc stops it.
pub fn run_button(ui: &mut Ui, running: bool, enter: bool) -> Action {
    if running {
        spinner(ui);
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if ui.button("■ Stop").on_hover_text("Stop (Esc)").clicked() || esc {
            return Action::Stop;
        }
    } else if ui
        .button("▶ Start")
        .on_hover_text("Start (Enter)")
        .clicked()
        || enter
    {
        return Action::Start;
    }
    Action::None
}

/// Scan speed presets: (name, threads, probes per second, tooltip).
/// Threads only cap how many probes are in flight; against targets that answer
/// fast (RST, live hosts) the rate limit is what keeps a scan from bursting.
pub const SPEEDS: &[(&str, usize, u32, &str)] = &[
    ("Gentle", 4, 10, "10 probes/s. Least likely to trip IDS/EDR"),
    ("Normal", 16, 50, "50 probes/s"),
    ("Fast", 64, 500, "500 probes/s. Expect IDS/EDR alerts"),
    ("Unthrottled", 256, 0, "No limit. Will be flagged as a scan"),
];
pub const DEFAULT_SPEED: usize = 1;

/// A TextEdit's selection in `text` as a byte range (empty at a bare
/// cursor); None before it has one.
pub fn text_selection(
    ctx: &egui::Context,
    id: egui::Id,
    text: &str,
) -> Option<std::ops::Range<usize>> {
    let state = egui::TextEdit::load_state(ctx, id)?;
    let r = state.cursor.char_range()?;
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
    let (a, b) = (r.primary.index.0, r.secondary.index.0);
    Some(byte(a.min(b))..byte(a.max(b)))
}

/// Draws `add` with the popups it opens free to use the window's height:
/// egui lays a new popup out at most 400 points tall, so a long list in a
/// drop-down or menu would scroll with room to spare.
pub fn tall_popups<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let was = ctx.global_style().spacing.default_area_size;
    let tall = ctx.content_rect().height();
    ctx.global_style_mut(|s| s.spacing.default_area_size.y = tall);
    let r = add(ui);
    ctx.global_style_mut(|s| s.spacing.default_area_size = was);
    r
}

/// Busy indicator. egui's own spinner asks for a redraw every frame, which
/// means 60 full window redraws a second (unbounded without vsync) for as
/// long as a tool runs; this one turns at 20 frames a second.
pub fn spinner(ui: &mut Ui) {
    let size = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let t = ui.input(|i| i.time) as f32;
    let start = t * 5.0;
    let r = rect.height() / 2.0 - 2.0;
    let pts: Vec<egui::Pos2> = (0..=16)
        .map(|i| {
            let a = start + i as f32 / 16.0 * std::f32::consts::PI * 1.4;
            rect.center() + r * egui::vec2(a.cos(), a.sin())
        })
        .collect();
    ui.painter().add(egui::Shape::line(
        pts,
        egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
    ));
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(50));
}

/// Speed preset picker plus the raw threads / rate values it sets.
pub fn speed(ui: &mut Ui, id: &str, threads: &mut usize, rate: &mut u32) {
    let current = SPEEDS
        .iter()
        .find(|s| s.1 == *threads && s.2 == *rate)
        .map_or("Custom", |s| s.0);
    ui.label("Speed");
    egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (name, t, r, tip) in SPEEDS {
                let sel = ui
                    .selectable_label(current == *name, *name)
                    .on_hover_text(*tip);
                if sel.clicked() {
                    (*threads, *rate) = (*t, *r);
                }
            }
        });
    ui.add(egui::DragValue::new(rate).range(0..=100_000).suffix(" /s"))
        .on_hover_text("Probes per second (0 = no limit)");
    ui.add(egui::DragValue::new(threads).range(1..=1024).suffix(" thr"))
        .on_hover_text("Max probes in flight at once");
}

/// Progress bar with live rate and time left.
pub fn progress(ui: &mut Ui, done: usize, total: usize, since: std::time::Instant) {
    let secs = since.elapsed().as_secs_f64();
    let rate = if secs > 0.5 { done as f64 / secs } else { 0.0 };
    let mut text = format!("{done} / {total}");
    if rate > 0.0 {
        let left = (total - done.min(total)) as f64 / rate;
        text += &format!("  ·  {rate:.0}/s  ·  ~{} left", fmt_secs(left));
    }
    ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).text(text));
}

fn fmt_secs(s: f64) -> String {
    let s = s.ceil() as u64;
    if s < 60 {
        format!("{s} s")
    } else {
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

/// Width of the label that starts a toolbar row, so every row's first
/// input lines up under the one above it.
pub const LEAD_W: f32 = 76.0;
/// Width of a tab's main input (Host, Range, Domain, Name or IP).
pub const MAIN_W: f32 = 240.0;
/// Widest an empty state's text runs before it wraps.
const EMPTY_W: f32 = 560.0;

/// A label; at the start of a row it takes `LEAD_W`, so the rows of a
/// toolbar line up.
pub fn lead(ui: &mut Ui, text: &str) {
    let at_start = (ui.cursor().min.x - ui.max_rect().min.x).abs() < 0.5;
    if !at_start {
        ui.label(text);
        return;
    }
    let h = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        egui::vec2(LEAD_W, h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(LEAD_W - ui.spacing().item_spacing.x);
            ui.label(text);
        },
    );
}

/// A muted line under a toolbar row, lined up with its inputs.
pub fn toolbar_note(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(LEAD_W);
        ui.add(egui::Label::new(RichText::new(text).weak()).wrap());
    });
}

/// The row under a tab's toolbar: what the run found on the left, its
/// output actions on the right (add them in reverse: Save…, then Copy).
/// Drawn on every tab, empty or not, so content starts at the same height.
pub fn status_row(ui: &mut Ui, left: impl FnOnce(&mut Ui), right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(ui.spacing().interact_size.y);
        left(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

/// Muted line for a view that ran but has nothing to show ("No runs match
/// the filter").
pub fn empty_hint(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| ui.weak(text));
}

/// What a tab shows before its first run: a title, a line on what to do,
/// and buttons for the likely first step. Returns the index of the button
/// clicked. Each action is (label, hover text, enabled).
pub fn empty_state(
    ui: &mut Ui,
    title: &str,
    hint: &str,
    actions: &[(&str, &str, bool)],
) -> Option<usize> {
    let avail = ui.available_rect_before_wrap();
    let w = avail.width().min(EMPTY_W);
    let top = (avail.height() * 0.12).clamp(12.0, 90.0);
    let rect = egui::Rect::from_min_max(
        egui::pos2(avail.center().x - w / 2.0, avail.top() + top),
        egui::pos2(avail.center().x + w / 2.0, avail.bottom()),
    );
    let mut hit = None;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Center)),
        |ui| {
            ui.label(RichText::new(title).size(17.0).strong());
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(hint).weak()).wrap());
            if actions.is_empty() {
                return;
            }
            ui.add_space(10.0);
            let pad = ui.spacing().button_padding.x * 2.0;
            let gap = ui.spacing().item_spacing.x;
            let total: f32 = actions
                .iter()
                .map(|(label, ..)| {
                    egui::WidgetText::from(*label)
                        .into_galley(
                            ui,
                            Some(egui::TextWrapMode::Extend),
                            f32::INFINITY,
                            egui::TextStyle::Button,
                        )
                        .size()
                        .x
                        + pad
                })
                .sum::<f32>()
                + gap * (actions.len() - 1) as f32;
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                for (i, (label, tip, enabled)) in actions.iter().enumerate() {
                    if ui
                        .add_enabled(*enabled, egui::Button::new(*label))
                        .on_hover_text(*tip)
                        .clicked()
                    {
                        hit = Some(i);
                    }
                }
            });
        },
    );
    hit
}

/// Single-line text field; returns true if Enter was pressed in it.
pub fn field(
    ui: &mut Ui,
    label: &str,
    kind: Field,
    value: &mut String,
    width: f32,
    hint: &str,
) -> bool {
    lead(ui, label);
    let r = line(ui, kind, value, width, hint);
    r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// A single-line text box for `kind`: its length limit, hidden characters
/// dropped as they're typed or pasted, and a right-click menu.
pub fn line(
    ui: &mut Ui,
    kind: Field,
    value: &mut String,
    width: f32,
    hint: &str,
) -> egui::Response {
    let r = add_text_edit(
        ui,
        egui::TextEdit::singleline(value)
            .char_limit(kind.room())
            .desired_width(width)
            .hint_text(hint),
    );
    tidy(ui.ctx(), &r, kind, value);
    r
}

/// After text box `r` (holding a `kind`, told it takes `kind.room()`)
/// changed: drop the characters it doesn't keep, cut it to `kind.max()`
/// just before the cursor, as the box would have, leave the cursor where it
/// was, and say so when a paste lost hidden characters or was cut.
pub fn tidy(ctx: &egui::Context, r: &egui::Response, kind: Field, value: &mut String) {
    if !r.changed() {
        return;
    }
    let pasted = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
    let state = egui::TextEdit::load_state(ctx, r.id);
    let range = state.as_ref().and_then(|s| s.cursor.char_range());
    let at = range.map_or(usize::MAX, |c| kind.kept_before(value, c.primary.index.0));
    let (clean, cut) = kind.fit(value, at);
    if clean == *value {
        return;
    }
    let dropped = kind.dropped(value);
    // The box itself stopped the paste at its room: more was lost.
    let full = value.chars().count() >= kind.room();
    if let (Some(mut state), Some(mut range)) = (state, range) {
        for c in [&mut range.primary, &mut range.secondary] {
            let k = kind.kept_before(value, c.index.0);
            c.index = egui::text::CharIndex(if k >= cut.end {
                k - cut.len()
            } else {
                k.min(cut.start)
            });
        }
        state.cursor.set_char_range(Some(range));
        state.store(ctx, r.id);
    }
    *value = clean;
    if pasted && dropped > 0 {
        toast(
            format!(
                "Took out {dropped} invisible or control character{}",
                if dropped == 1 { "" } else { "s" }
            ),
            true,
        );
    }
    if pasted && (!cut.is_empty() || full) {
        toast(
            format!("Pasted text cut to {} characters", kind.max()),
            false,
        );
    }
}

pub fn num<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    v: &mut T,
    range: std::ops::RangeInclusive<T>,
    suffix: &str,
) {
    lead(ui, label);
    ui.add(egui::DragValue::new(v).range(range).suffix(suffix));
}

pub fn copy_button(ui: &mut Ui, text: impl FnOnce() -> String) {
    if ui
        .button("Copy")
        .on_hover_text("Copy results to clipboard")
        .clicked()
    {
        copy(ui.ctx(), text());
        toast("Copied to the clipboard", true);
    }
}

/// Save button: opens a native save dialog off the UI thread and writes `text`.
pub fn save_button(ui: &mut Ui, stem: &str, text: impl FnOnce() -> String) {
    if ui
        .button("Save…")
        .on_hover_text("Save results to a text file")
        .clicked()
    {
        save_text(format!("is-alive-{stem}.txt"), text());
    }
}

pub fn save_text(file_name: String, text: String) {
    save_file(file_name, "Text", vec!["txt", "log"], text);
}

/// Export menu: each format opens a save dialog with `entries` rendered in it.
pub fn export_menu<'a>(
    ui: &mut Ui,
    stem: &str,
    entries: impl FnOnce() -> Vec<&'a crate::log::Entry>,
) {
    let mut picked = None;
    ui.menu_button("Export ⏷", |ui| {
        for f in crate::log::Format::ALL {
            if ui.button(f.name()).on_hover_text(f.hint()).clicked() {
                picked = Some(f);
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Save as CSV, JSON, Markdown or HTML");
    if let Some(f) = picked {
        let text = f.export(&entries());
        save_file(
            format!("is-alive-{stem}.{}", f.ext()),
            f.name(),
            vec![f.ext()],
            text,
        );
    }
}

/// The saved file's path, ending in one of `exts`: a name typed as
/// "notes.bat" or "report.xlsm" saves as "notes.bat.txt", so nothing this
/// app writes can be run by a double-click or open with macros.
pub fn safe_path(path: std::path::PathBuf, exts: &[&str]) -> std::path::PathBuf {
    let ok = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)));
    match exts.first() {
        Some(x) if !ok => {
            let mut s = path.into_os_string();
            s.push(format!(".{x}"));
            s.into()
        }
        _ => path,
    }
}

pub fn save_file(file_name: String, filter: &'static str, exts: Vec<&'static str>, text: String) {
    std::thread::spawn(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(&file_name)
            .add_filter(filter, &exts)
            .save_file()
        else {
            return;
        };
        let path = safe_path(path, &exts);
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        if let Err(e) = std::fs::write(&path, text) {
            toast(format!("Couldn't save {name}"), false);
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Save failed")
                .set_description(format!("{}: {e}", path.display()))
                .show();
        } else {
            toast(format!("Saved {name}"), true);
        }
    });
}

pub fn error(ui: &mut Ui, err: &Option<String>) {
    if let Some(e) = err {
        error_text(ui, e);
    }
}

/// "⚠ …" in the error colour.
pub fn error_text(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(format!("⚠ {text}")).color(bad()));
}

/// "⚠ …" in the warning colour: worth knowing, not a failure.
pub fn warning(ui: &mut Ui, text: &str) -> egui::Response {
    ui.label(RichText::new(format!("⚠ {text}")).color(warn()))
}

/// A button for something that can't be undone: the first click arms it
/// ("Clear: sure?") for three seconds, the second does it.
pub fn confirm_button(ui: &mut Ui, id: &str, enabled: bool, label: &str, tip: &str) -> bool {
    let id = ui.id().with(("confirm", id));
    let now = ui.input(|i| i.time);
    let armed = ui
        .data(|d| d.get_temp::<f64>(id))
        .is_some_and(|t| now - t < 3.0);
    let button = if armed {
        egui::Button::new(RichText::new(format!("{label}: sure?")).color(bad()))
    } else {
        egui::Button::new(label)
    };
    let clicked = ui
        .add_enabled(enabled, button)
        .on_hover_text(format!("{tip} (click twice)"))
        .clicked();
    if clicked && armed {
        ui.data_mut(|d| d.remove::<f64>(id));
        return true;
    }
    if clicked {
        ui.data_mut(|d| d.insert_temp(id, now));
        ui.ctx().request_repaint_after(Duration::from_secs(3));
    }
    false
}

pub fn fmt_ms(ms: f64) -> String {
    if ms < 1.0 {
        format!("{ms:.2} ms")
    } else if ms < 100.0 {
        format!("{ms:.1} ms")
    } else {
        format!("{ms:.0} ms")
    }
}

/// Latency sparkline. `None` samples are drawn as red loss ticks.
pub fn latency_chart(ui: &mut Ui, samples: &[Option<f64>], height: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let vis = ui.visuals();
    p.rect_filled(rect, 4.0, vis.extreme_bg_color);
    if samples.is_empty() {
        return;
    }
    // Floor of 0.1 ms so LAN pings aren't a flat line; the scale eases
    // to a new peak instead of jumping.
    let peak = samples.iter().flatten().fold(0.1f64, |a, &b| a.max(b)) * 1.15;
    let max =
        ui.ctx()
            .animate_value_with_time(ui.id().with("chart_scale"), peak as f32, 0.3) as f64;
    let n = samples.len().max(2) - 1;
    let x = |i: usize| rect.left() + 4.0 + (rect.width() - 8.0) * i as f32 / n as f32;
    let y = |v: f64| rect.bottom() - 4.0 - (rect.height() - 8.0) * (v / max) as f32;

    let mut line = Vec::new();
    for (i, s) in samples.iter().enumerate() {
        match s {
            Some(v) => line.push(egui::pos2(x(i), y(*v))),
            None => {
                p.line_segment(
                    [
                        egui::pos2(x(i), rect.top() + 2.0),
                        egui::pos2(x(i), rect.bottom() - 2.0),
                    ],
                    egui::Stroke::new(1.5, bad().gamma_multiply(0.6)),
                );
                flush(&p, &mut line);
            }
        }
    }
    flush(&p, &mut line);
    p.text(
        rect.left_top() + egui::vec2(6.0, 4.0),
        egui::Align2::LEFT_TOP,
        fmt_ms(max / 1.15),
        egui::FontId::monospace(10.0),
        vis.weak_text_color(),
    );

    fn flush(p: &egui::Painter, line: &mut Vec<egui::Pos2>) {
        match line.len() {
            0 => {}
            1 => {
                p.circle_filled(line[0], 2.0, ok());
            }
            _ => {
                p.add(egui::Shape::line(
                    std::mem::take(line),
                    egui::Stroke::new(1.5, ok()),
                ));
            }
        }
        line.clear();
    }
}

/// Draw a floating window with a short title bar: egui sizes the bar (and
/// its close button) from the Heading style, which is sized for page
/// titles, so it is smaller while `show` runs.
pub fn compact_title(ctx: &egui::Context, show: impl FnOnce()) {
    let heading = egui::TextStyle::Heading;
    let before = ctx.global_style().text_styles.get(&heading).cloned();
    ctx.global_style_mut(|s| {
        s.text_styles
            .insert(heading.clone(), egui::FontId::proportional(14.5));
    });
    show();
    if let Some(f) = before {
        ctx.global_style_mut(|s| {
            s.text_styles.insert(heading, f);
        });
    }
}

/// Send every copy egui made this frame (Ctrl+C or Ctrl+X in any text box,
/// a selectable label) through `copy`, as the app's own Copy buttons go:
/// kept out of Windows clipboard history and cloud sync, and cleared on
/// exit if it's still there. Call once, after the UI.
pub fn route_copies(ctx: &egui::Context) {
    let mut texts = Vec::new();
    ctx.output_mut(|o| {
        o.commands.retain(|c| match c {
            egui::OutputCommand::CopyText(t) => {
                texts.push(t.clone());
                false
            }
            _ => true,
        })
    });
    for t in texts {
        copy(ctx, t);
    }
}

/// The selection in text box `id` as a right-click lands: egui's text box
/// moves the cursor on a press of any button, which would leave the menu's
/// Cut, Copy and Redact nothing to work on. Pair with `restore_selection`.
pub fn selection_before_right_click(
    ctx: &egui::Context,
    id: egui::Id,
) -> Option<egui::text::CCursorRange> {
    if !ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary)) {
        return None;
    }
    egui::TextEdit::load_state(ctx, id)?.cursor.char_range()
}

/// Put back what `selection_before_right_click` saved, if the right-click
/// was on this box (`resp`).
pub fn restore_selection(
    ctx: &egui::Context,
    id: egui::Id,
    resp: &egui::Response,
    saved: Option<egui::text::CCursorRange>,
) {
    let Some(range) = saved else { return };
    if !resp.hovered() {
        return;
    }
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
}

/// Add a text box with a right-click menu (Cut, Copy, Paste, Select all)
/// that keeps the selection it was opened on. For a box without its own
/// `.id(…)`; one with an id goes through `add_text_edit_with_id`.
pub fn add_text_edit(ui: &mut Ui, edit: egui::TextEdit<'_>) -> egui::Response {
    // A box with no id of its own takes the next automatic one.
    let id = ui.next_auto_id();
    add_text_edit_with_id(ui, id, edit)
}

/// `add_text_edit` for a box given `.id(id)`.
pub fn add_text_edit_with_id(
    ui: &mut Ui,
    id: egui::Id,
    edit: egui::TextEdit<'_>,
) -> egui::Response {
    let keep = selection_before_right_click(ui.ctx(), id);
    let r = ui.add(edit);
    restore_selection(ui.ctx(), r.id, &r, keep);
    r.context_menu(|ui| {
        if text_edit_menu(ui, r.id, true) {
            ui.close();
        }
    });
    r
}

/// A right-click menu for a text box: Cut, Copy, Paste and Select all, as
/// the keyboard shortcuts do them (Paste and Cut only when `editable`).
/// Returns true when an item was picked.
pub fn text_edit_menu(ui: &mut Ui, id: egui::Id, editable: bool) -> bool {
    let ctx = ui.ctx().clone();
    let selected = egui::TextEdit::load_state(&ctx, id)
        .and_then(|s| s.cursor.char_range())
        .is_some_and(|r| r.primary != r.secondary);
    let mut picked = false;
    // The command reaches the box on the next frame, so it gets the focus
    // back (and keeps its selection) first.
    let send = |cmd: egui::ViewportCommand| {
        ctx.memory_mut(|m| m.request_focus(id));
        ctx.send_viewport_cmd(cmd);
    };
    if editable
        && ui
            .add_enabled(selected, egui::Button::new("Cut").shortcut_text("Ctrl+X"))
            .clicked()
    {
        send(egui::ViewportCommand::RequestCut);
        picked = true;
    }
    if ui
        .add_enabled(selected, egui::Button::new("Copy").shortcut_text("Ctrl+C"))
        .clicked()
    {
        send(egui::ViewportCommand::RequestCopy);
        picked = true;
    }
    if editable
        && ui
            .add(egui::Button::new("Paste").shortcut_text("Ctrl+V"))
            .clicked()
    {
        send(egui::ViewportCommand::RequestPaste);
        picked = true;
    }
    if ui
        .add(egui::Button::new("Select all").shortcut_text("Ctrl+A"))
        .clicked()
    {
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
        // The far end is clamped to the text's length when it's drawn.
        let all = egui::text::CCursorRange::two(
            egui::text::CCursor::new(egui::text::CharIndex(0)),
            egui::text::CCursor::new(egui::text::CharIndex(usize::MAX / 2)),
        );
        state.cursor.set_char_range(Some(all));
        state.store(&ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
        picked = true;
    }
    picked
}

/// Take every link egui was asked to open this frame (`ctx.open_url`,
/// `hyperlink`) and open it with `open_url`. Call once, after the UI.
pub fn open_requested_links(ctx: &egui::Context) {
    let mut urls = Vec::new();
    ctx.output_mut(|o| {
        o.commands.retain(|c| match c {
            egui::OutputCommand::OpenUrl(u) => {
                urls.push(u.url.clone());
                false
            }
            _ => true,
        })
    });
    for url in urls {
        open_url(url);
    }
}

/// Open an http(s) link in the default browser, off the UI thread, with
/// the system's own launcher by absolute path (never a PATH lookup). A
/// toast says so if it can't.
pub fn open_url(url: String) {
    if !is_web_link(&url) {
        toast("Only web links open from here", false);
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = launch(&url) {
            toast(format!("Couldn't open the browser ({e}): {url}"), false);
        }
    });
}

/// http or https, and nothing that could read as an option to the launcher.
fn is_web_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

#[cfg(windows)]
fn launch(url: &str) -> Result<(), String> {
    let root = std::env::var_os("SystemRoot").ok_or("SystemRoot not set")?;
    // explorer.exe hands a URL to the default browser; its exit code
    // means nothing, so only failing to start it counts.
    std::process::Command::new(std::path::PathBuf::from(root).join("explorer.exe"))
        .arg(url)
        .spawn()
        .map(drop)
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn launch(url: &str) -> Result<(), String> {
    let s = std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(|e| e.to_string())?;
    s.success()
        .then_some(())
        .ok_or_else(|| format!("open exited with {s}"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn launch(url: &str) -> Result<(), String> {
    let tool = crate::net::system_tool("xdg-open").ok_or("xdg-open not found")?;
    let mut cmd = std::process::Command::new(tool);
    cmd.arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Run from an AppImage, LD_LIBRARY_PATH leads with the AppImage's own
    // libraries; a browser started with them can fail to load. Hand on the
    // user's own value only.
    let appdir = std::env::var("APPDIR").ok();
    match std::env::var("LD_LIBRARY_PATH") {
        Ok(v) => match host_library_path(&v, appdir.as_deref()) {
            Some(keep) => cmd.env("LD_LIBRARY_PATH", keep),
            None => cmd.env_remove("LD_LIBRARY_PATH"),
        },
        Err(_) => &mut cmd,
    };
    let s = cmd.status().map_err(|e| e.to_string())?;
    s.success()
        .then_some(())
        .ok_or_else(|| format!("xdg-open exited with {s}"))
}

/// `LD_LIBRARY_PATH` without the AppImage's entries (those under `appdir`,
/// or any `.mount_*` AppImage mount when `appdir` isn't known); None when
/// nothing is left.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn host_library_path(value: &str, appdir: Option<&str>) -> Option<String> {
    let keep: Vec<&str> = value
        .split(':')
        .filter(|p| !p.is_empty())
        .filter(|p| match appdir {
            Some(d) if !d.is_empty() => !p.starts_with(d),
            _ => !p.contains("/.mount_"),
        })
        .collect();
    (!keep.is_empty()).then(|| keep.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn saves_only_with_an_allowed_extension() {
        let p = |s: &str| safe_path(PathBuf::from(s), &["txt", "log"]);
        assert_eq!(p("a.txt"), PathBuf::from("a.txt"));
        assert_eq!(p("a.LOG"), PathBuf::from("a.LOG"));
        assert_eq!(p("a.bat"), PathBuf::from("a.bat.txt"));
        assert_eq!(p("report.xlsm"), PathBuf::from("report.xlsm.txt"));
        assert_eq!(p("noext"), PathBuf::from("noext.txt"));
    }

    #[test]
    fn only_plain_web_links_open() {
        assert!(super::is_web_link(
            "https://github.com/hammerknife404/is-alive/wiki"
        ));
        assert!(super::is_web_link("http://10.0.0.1:8443"));
        assert!(!super::is_web_link("file:///etc/passwd"));
        assert!(!super::is_web_link("javascript:alert(1)"));
        assert!(!super::is_web_link("https://x.example/a b"));
        assert!(!super::is_web_link("--help"));
    }

    #[test]
    fn the_appimage_libraries_stay_behind() {
        let v = "/tmp/.mount_is-aliXYZ/usr/lib:/opt/mine/lib";
        assert_eq!(
            super::host_library_path(v, Some("/tmp/.mount_is-aliXYZ")),
            Some("/opt/mine/lib".into())
        );
        assert_eq!(
            super::host_library_path(v, None),
            Some("/opt/mine/lib".into())
        );
        assert_eq!(
            super::host_library_path(
                "/tmp/.mount_is-aliXYZ/usr/lib",
                Some("/tmp/.mount_is-aliXYZ")
            ),
            None
        );
    }
}
