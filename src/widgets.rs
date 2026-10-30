//! Small shared UI pieces.

use eframe::egui::{self, Color32, RichText, Ui};

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
        ui.spinner();
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
    (
        "Gentle",
        4,
        10,
        "10 probes/s, 4 in flight. Slowest; least load and least scan-like to IDS/EDR.",
    ),
    ("Normal", 16, 50, "50 probes/s, 16 in flight."),
    (
        "Fast",
        64,
        500,
        "500 probes/s, 64 in flight. Expect IDS/EDR scan alerts.",
    ),
    (
        "Unthrottled",
        256,
        0,
        "No rate limit, 256 in flight. Tens of thousands of probes/s on a LAN; \
will be flagged as a scan.",
    ),
];
pub const DEFAULT_SPEED: usize = 1;

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
        .on_hover_text("Max probes started per second across all threads (0 = unlimited)");
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

/// Single-line text field; returns true if Enter was pressed in it.
pub fn field(ui: &mut Ui, label: &str, value: &mut String, width: f32, hint: &str) -> bool {
    ui.label(label);
    let r = ui.add(
        egui::TextEdit::singleline(value)
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
        ui.ctx().copy_text(text());
    }
}

/// Save button: opens a native save dialog off the UI thread and writes `text`.
pub fn save_button(ui: &mut Ui, stem: &str, text: impl FnOnce() -> String) {
    if ui
        .button("Save")
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

pub fn save_file(file_name: String, filter: &'static str, exts: Vec<&'static str>, text: String) {
    std::thread::spawn(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(&file_name)
            .add_filter(filter, &exts)
            .save_file()
        else {
            return;
        };
        if let Err(e) = std::fs::write(&path, text) {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Save failed")
                .set_description(format!("{}: {e}", path.display()))
                .show();
        }
    });
}

pub fn error(ui: &mut Ui, err: &Option<String>) {
    if let Some(e) = err {
        ui.label(RichText::new(format!("⚠ {e}")).color(bad()));
    }
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
