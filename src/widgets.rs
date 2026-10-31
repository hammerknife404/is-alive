//! Small shared UI pieces.

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
    if let Ok(mut t) = TOASTS.lock() {
        t.push((text.into(), ok, Instant::now()));
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

/// Muted placeholder for an empty result view.
pub fn empty_hint(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| ui.weak(text));
}

/// Most characters a field takes. A host, domain or search never needs
/// more; a paste of a whole log into one is cut off, not carried around.
pub const FIELD_MAX: usize = 255;
/// A link (Speed Test's file).
pub const URL_MAX: usize = 2048;
/// Notes: about 1 MB of text, far beyond any site's notes.
pub const NOTES_MAX: usize = 1_000_000;
/// Known problems in the site bundle.
pub const PROBLEMS_MAX: usize = 20_000;

/// Single-line text field; returns true if Enter was pressed in it.
pub fn field(ui: &mut Ui, label: &str, value: &mut String, width: f32, hint: &str) -> bool {
    ui.label(label);
    let r = ui.add(
        egui::TextEdit::singleline(value)
            .char_limit(FIELD_MAX)
            .desired_width(width)
            .hint_text(hint),
    );
    r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

pub fn num<T: egui::emath::Numeric>(
    ui: &mut Ui,
    label: &str,
    v: &mut T,
    range: std::ops::RangeInclusive<T>,
    suffix: &str,
) {
    ui.label(label);
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
}
