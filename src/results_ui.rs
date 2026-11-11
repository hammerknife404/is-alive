//! The Results tab: every run in a list down the left, newest first and
//! grouped by day (what's running at the top), and the one picked in full
//! beside it: what it was, Run again, the output, the table it made, and
//! what changed since another run of the same tool (Compare). Built from
//! the inspector's pieces, so a run reads like a device does.

use crate::{App, Tab, input, inspector, log, theme, widgets};
use eframe::egui::{self, RichText, Sense, Ui, vec2};

/// Which run is open: a finished one by tool and start time (indices move
/// as old runs drop off), or a running one by tool.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum RunKey {
    Done { tool: &'static str, started: String },
    Running(&'static str),
}

/// What the open run shows: its output as it printed, its table, or what
/// changed between it and another run of its tool.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub(crate) enum RunView {
    #[default]
    Output,
    Table,
    Compare,
}

/// "11-10 20:02" from "2026-11-10 20:02:09": a run's start, short.
fn when(started: &str) -> &str {
    started.get(5..16).unwrap_or(started)
}

/// The run list's width for a window `width` points wide: it always docks
/// (the list is how the tab is used), narrower on a small window.
pub fn list_width(width: f32) -> f32 {
    if width >= 1440.0 {
        300.0
    } else if width >= 1100.0 {
        268.0
    } else {
        232.0
    }
}

/// A finished run's one-line summary for the list: what each tool's
/// table says in a few words (ports open, hosts alive, the speeds,
/// checks failed), or the output's last line with words on it.
pub fn summarise(e: &log::Entry) -> String {
    let Some(t) = e.table.as_ref().filter(|t| !t.rows.is_empty()) else {
        return summary(&e.text);
    };
    let col = |name: &str| t.columns.iter().position(|c| *c == name);
    let count = |name: &str, want: &str| {
        col(name).map_or(0, |k| {
            t.rows
                .iter()
                .filter(|r| r.get(k).is_some_and(|v| v.eq_ignore_ascii_case(want)))
                .count()
        })
    };
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match e.tool {
        "Port Check" => format!(
            "{} open of {} checked",
            count("state", "open"),
            t.rows.len()
        ),
        "Traceroute" => plural(t.rows.len(), "hop", "hops"),
        "Subnet Sweep" => format!("{} alive", t.rows.len()),
        "Lookup" => {
            let notes = count("type", "note");
            plural(t.rows.len() - notes, "record", "records")
        }
        "Speed Test" => {
            let speed = |phase: &str| {
                let (p, m) = (col("phase")?, col("mbps")?);
                let r = t
                    .rows
                    .iter()
                    .find(|r| r.get(p).is_some_and(|v| v == phase))?;
                let mbps: f64 = r.get(m)?.parse().ok()?;
                Some(format!("{mbps:.0}"))
            };
            match (speed("download"), speed("upload")) {
                (Some(d), Some(u)) => format!("Down {d} · up {u} Mbps"),
                (Some(d), None) => format!("Down {d} Mbps"),
                _ => summary(&e.text),
            }
        }
        "Domain Health" => {
            // Each check once, by its overall status.
            let (Some(c), Some(st)) = (col("check"), col("check_status")) else {
                return summary(&e.text);
            };
            let mut seen = std::collections::BTreeMap::new();
            for r in &t.rows {
                if let (Some(k), Some(v)) = (r.get(c), r.get(st)) {
                    seen.insert(k.as_str(), v.as_str());
                }
            }
            let n = |s: &str| seen.values().filter(|v| **v == s).count();
            match (n("FAIL"), n("WARN")) {
                (0, 0) => format!("{} checks pass", seen.len()),
                (f, 0) => format!("{f} failed"),
                (0, w) => plural(w, "warning", "warnings"),
                (f, w) => format!("{f} failed · {}", plural(w, "warning", "warnings")),
            }
        }
        _ => summary(&e.text),
    }
}

/// A run's last line with words on it (a ping's totals), cut to fit a
/// list row.
pub fn summary(text: &str) -> String {
    let line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.chars().any(char::is_alphanumeric))
        .unwrap_or("");
    let mut s: String = line.chars().take(90).collect();
    if s.len() < line.len() {
        s.push('…');
    }
    s
}

/// "2026-11-10 14:05:12" as its day and its hour and minute.
pub fn day_and_time(started: &str) -> (&str, &str) {
    let day = started.get(..10).unwrap_or(started);
    let time = started.get(11..16).unwrap_or("");
    (day, time)
}

/// How long a run took, from its start and end ("2026-11-10 14:05:12").
pub fn took(started: &str, ended: &str) -> Option<String> {
    let parse = |s: &str| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok();
    let secs = (parse(ended)? - parse(started)?).num_seconds();
    Some(match secs {
        s if s < 0 => return None,
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{} min {} s", s / 60, s % 60),
        s => format!("{} h {} min", s / 3600, s % 3600 / 60),
    })
}

/// The tab a tool's runs come from, for Run again. Not Lookup: its target
/// carries the record type and resolver ("example.com (MX) via 8.8.8.8"),
/// which its quick run can't repeat, so a rerun wouldn't be the same lookup.
fn tab_of(tool: &str) -> Option<Tab> {
    Some(match tool {
        "Ping" => Tab::Ping,
        "Port Check" => Tab::Ports,
        "Traceroute" => Tab::Trace,
        "Speed Test" => Tab::Speed,
        "Subnet Sweep" => Tab::Sweep,
        "Domain Health" => Tab::Advanced,
        _ => return None,
    })
}

/// Where a run's list row sends a click.
enum Row {
    Done(usize),
    Running(usize),
}

impl App {
    pub(crate) fn results_ui(&mut self, ui: &mut Ui) {
        let jump = std::mem::take(&mut self.jump_to_new);
        let running = self.running();
        let needle = self.filter.trim().to_lowercase();
        let hit = |tool: &str, text: &str| {
            needle.is_empty()
                || tool.to_lowercase().contains(&needle)
                || text.to_lowercase().contains(&needle)
        };
        // Matching the log is the costly part (it can be megabytes): only
        // when the filter or the log changes, not every frame.
        let key = (needle.clone(), self.log.stamp());
        if self.filter_cache.as_ref().is_none_or(|c| c.0 != key) {
            let idx = (0..self.log.entries.len())
                .filter(|&i| hit(self.log.entries[i].tool, &self.log.entries[i].text))
                .collect();
            self.filter_cache = Some((key, idx));
        }
        let shown: Vec<usize> = self
            .filter_cache
            .as_ref()
            .map(|c| c.1.clone())
            .unwrap_or_default();
        let running: Vec<(&'static str, String, String)> =
            running.into_iter().filter(|r| hit(r.0, &r.2)).collect();
        // After a full scan: its first run, still going.
        if jump {
            self.result_pick = running.first().map(|r| RunKey::Running(r.0));
        }
        let open = self.open_run(&shown, &running);
        if self.log.entries.is_empty() && running.is_empty() {
            widgets::empty_state(
                ui,
                "No results yet",
                "Finished runs from every tab are listed here with start and end times, to read again, copy, save or export.",
                &[],
            );
            return;
        }
        let width = ui.ctx().content_rect().width();
        let mut clicked = None;
        egui::Panel::left("results_list")
            .exact_size(list_width(width))
            .resizable(false)
            .show(ui, |ui| {
                clicked = self.run_list(ui, &shown, &running, open.as_ref())
            });
        match clicked {
            Some(Row::Done(i)) => {
                let e = &self.log.entries[i];
                self.result_pick = Some(RunKey::Done {
                    tool: e.tool,
                    started: e.started.clone(),
                });
            }
            Some(Row::Running(j)) => self.result_pick = Some(RunKey::Running(running[j].0)),
            None => {}
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 14,
                right: 0,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| match open {
                Some(Row::Done(i)) => self.run_detail(ui, Some(i), None),
                Some(Row::Running(j)) => self.run_detail(ui, None, Some(&running[j])),
                None => widgets::empty_hint(ui, "No runs match the filter."),
            });
    }

    /// The run to show: the one picked if it's still listed, else the
    /// newest finished one, else the first still running.
    fn open_run(&self, shown: &[usize], running: &[(&'static str, String, String)]) -> Option<Row> {
        match &self.result_pick {
            Some(RunKey::Running(t)) => {
                if let Some(j) = running.iter().position(|r| r.0 == *t) {
                    return Some(Row::Running(j));
                }
            }
            Some(RunKey::Done { tool, started }) => {
                if let Some(&i) = shown.iter().find(|&&i| {
                    let e = &self.log.entries[i];
                    e.tool == *tool && e.started == *started
                }) {
                    return Some(Row::Done(i));
                }
            }
            None => {}
        }
        match shown.last() {
            Some(&i) => Some(Row::Done(i)),
            None => (!running.is_empty()).then_some(Row::Running(0)),
        }
    }

    /// The list down the left: its header (filter, the runs' menu), then
    /// what's running, then the finished runs by day, newest first.
    fn run_list(
        &mut self,
        ui: &mut Ui,
        shown: &[usize],
        running: &[(&'static str, String, String)],
        open: Option<&Row>,
    ) -> Option<Row> {
        let mut clicked = None;
        let filtered = !self.filter.trim().is_empty();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.strong("Runs");
            let count = if filtered {
                format!("{} of {}", shown.len(), self.log.entries.len())
            } else {
                self.log.entries.len().to_string()
            };
            let busy = if running.is_empty() {
                String::new()
            } else {
                format!(" · {} running", running.len())
            };
            ui.weak(format!("{count}{busy}"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.runs_menu(ui, shown);
            });
        });
        ui.horizontal(|ui| {
            let w = ui.available_width() - if filtered { 28.0 } else { 0.0 };
            widgets::line(
                ui,
                input::Field::Text,
                &mut self.filter,
                w,
                "Filter: tool or text (FAIL, 443…)",
            )
            .on_hover_text(
                "Show runs containing this. Copy, Save and Export in All runs follow it",
            );
            if filtered && ui.small_button("×").on_hover_text("Clear filter").clicked() {
                self.filter.clear();
            }
        });
        if self.log.dropped > 0 {
            ui.weak(format!("{} oldest dropped", self.log.dropped));
        }
        ui.add_space(4.0);
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                if !running.is_empty() {
                    group(ui, "Running");
                    for (j, (tool, since, text)) in running.iter().enumerate() {
                        let on = matches!(open, Some(Row::Running(k)) if *k == j);
                        let (_, time) = day_and_time(since);
                        let line = format!("running… {}", summary(text));
                        if run_row(ui, tool, "", time, &line, on, true).clicked() {
                            clicked = Some(Row::Running(j));
                        }
                    }
                }
                let mut day = String::new();
                for &i in shown.iter().rev() {
                    let e = &self.log.entries[i];
                    let (d, time) = day_and_time(&e.started);
                    if d != day {
                        day = d.to_string();
                        group(ui, if d == today { "Today" } else { d });
                    }
                    let on = matches!(open, Some(Row::Done(k)) if *k == i);
                    let target = e.target().unwrap_or("");
                    if run_row(ui, e.tool, target, time, &summarise(e), on, false).clicked() {
                        clicked = Some(Row::Done(i));
                    }
                }
                if shown.is_empty() && running.is_empty() {
                    ui.weak("No runs match the filter.");
                }
            });
        clicked
    }

    /// All runs ▾ at the top of the list: every run listed (the filter's),
    /// and keeping them with the site.
    fn runs_menu(&mut self, ui: &mut Ui, shown: &[usize]) {
        let mut clear = false;
        let mut keep_to = None;
        widgets::named_menu(ui, "All runs", |ui| {
            ui.set_min_width(220.0);
            let filtered = !self.filter.trim().is_empty();
            let what = if filtered {
                "the runs listed"
            } else {
                "every run"
            };
            let render = || -> String {
                if filtered {
                    shown
                        .iter()
                        .map(|&i| self.log.entries[i].render() + "\n")
                        .collect()
                } else {
                    self.log.render()
                }
            };
            ui.add_enabled_ui(!shown.is_empty(), |ui| {
                if ui
                    .button("Copy all")
                    .on_hover_text(format!("Copy {what} as text"))
                    .clicked()
                {
                    widgets::copy(ui.ctx(), render());
                    widgets::toast("Results copied", true);
                    ui.close();
                }
                if ui
                    .button("Save all…")
                    .on_hover_text(format!("Save {what} to a text file"))
                    .clicked()
                {
                    widgets::save_text("is-alive-log.txt".into(), render());
                    ui.close();
                }
                widgets::export_menu(ui, "log", || {
                    shown.iter().map(|&i| &self.log.entries[i]).collect()
                });
            });
            ui.separator();
            // One setting: a site keeps its runs, Scratch for a day.
            let scratch = crate::site::current().is_none();
            let mut keep = self.look.results_keep;
            let (label, tip) = if scratch {
                (
                    "Keep for a day",
                    "Save Scratch's Results in your profile folder, readable by you only, for 24 \
                     hours from each run: they come through a crash, a flat battery or closing by \
                     mistake, but not into the next client's visit (Scratch's runs can be any \
                     client's). Open a site (top left) to keep them with it. Off: in memory only, \
                     gone when is-alive closes. The same setting as Keep with site at a site."
                        .to_string(),
                )
            } else {
                (
                    "Keep with site",
                    format!(
                        "Save Results in {}'s folder, readable by you only, so they're here the \
                         next time it opens (and for its Site report). Off: in memory only, gone \
                         when is-alive closes; the site's saved runs are taken off disk.",
                        self.site_name
                    ),
                )
            };
            if ui.checkbox(&mut keep, label).on_hover_text(tip).changed() {
                keep_to = Some(keep);
            }
            ui.separator();
            ui.add_enabled_ui(!self.log.entries.is_empty(), |ui| {
                clear =
                    widgets::confirm_button(ui, "log_clear", true, "Clear all", "Clear the log");
            });
        })
        .response
        .on_hover_text("Copy, save or export the runs; Keep with site; Clear");
        if clear {
            self.log.clear();
            self.result_pick = None;
        }
        if let Some(on) = keep_to {
            self.set_results_keep(on);
        }
    }

    /// The open run: what it was and when, its buttons, then its output or
    /// its table. `done` is a finished run's index; `running` a live one.
    fn run_detail(
        &mut self,
        ui: &mut Ui,
        done: Option<usize>,
        running: Option<&(&'static str, String, String)>,
    ) {
        let (tool, target, meta, text) = match (done, running) {
            (Some(i), _) => {
                let e = &self.log.entries[i];
                let took = took(&e.started, &e.ended)
                    .map(|t| format!(" · took {t}"))
                    .unwrap_or_default();
                (
                    e.tool,
                    e.target().unwrap_or("").to_string(),
                    format!("{} to {}{took}", e.started, day_and_time(&e.ended).1),
                    e.text.clone(),
                )
            }
            (None, Some((tool, since, text))) => (
                *tool,
                String::new(),
                format!("started {since}, running…"),
                text.clone(),
            ),
            (None, None) => return,
        };
        ui.add_space(6.0);
        let title = if target.is_empty() {
            tool
        } else {
            target.as_str()
        };
        let kind = if target.is_empty() { "Run" } else { tool };
        inspector::header(ui, kind, title, None, None);
        ui.weak(&meta);
        ui.add_space(6.0);
        let tab = tab_of(tool);
        let can_again = done.is_some()
            && tab.is_some_and(|t| !self.tab_running(t))
            && (tool == "Speed Test" || !target.is_empty());
        let ip: Option<std::net::Ipv4Addr> = target.parse().ok();
        // The run it's compared with: same tool, another run, still listed.
        let other = done.and_then(|i| {
            let key = self.result_compare.as_deref()?;
            self.log
                .entries
                .iter()
                .position(|e| e.tool == tool && e.started == key)
                .filter(|&j| j != i)
        });
        let comparing = self.result_view == RunView::Compare && other.is_some();
        let diff = match (done, other) {
            (Some(i), Some(j)) if comparing => {
                let (a, b) = (&self.log.entries[i], &self.log.entries[j]);
                // The earlier run is "before", whichever is open.
                let (before, after) = if a.started <= b.started {
                    (a, b)
                } else {
                    (b, a)
                };
                let d = crate::runs_compare::compare(before, after);
                let title = if target.is_empty() {
                    tool.to_string()
                } else {
                    format!("{tool} {target}")
                };
                let text = d.render(&title, &before.started, &after.started);
                Some((
                    d,
                    when(&before.started).to_string(),
                    when(&after.started).to_string(),
                    text,
                ))
            }
            _ => None,
        };
        let full = || match &diff {
            Some((.., text)) => text.clone(),
            None => format!("=== {tool} ===\n{}\n", text.trim_end()),
        };
        let acts = [
            inspector::Act {
                label: "Run again",
                tip: "Run it again on the same target, with its tab's settings as they are now \
                      (a ping sends 4); the new run opens here",
                enabled: can_again,
            },
            inspector::Act {
                label: "On map",
                tip: "Select its target on the Map",
                enabled: ip.is_some(),
            },
            inspector::Act {
                label: "Copy",
                tip: if diff.is_some() {
                    "Copy the comparison as text"
                } else {
                    "Copy this run as text"
                },
                enabled: true,
            },
            inspector::Act {
                label: "Save…",
                tip: "Save this run to a text file",
                enabled: true,
            },
        ];
        ui.horizontal(|ui| {
            match inspector::actions(ui, &acts) {
                Some(0) => {
                    if let Some(t) = tab {
                        self.run_again(ui.ctx(), t, &target);
                        self.result_pick = Some(RunKey::Running(tool));
                    }
                }
                Some(1) => {
                    if let Some(ip) = ip {
                        self.map.find(ip);
                        self.tab = Tab::Map;
                    }
                }
                Some(2) => {
                    widgets::copy(ui.ctx(), full());
                    widgets::toast("Run copied", true);
                }
                Some(3) => widgets::save_text(
                    format!("is-alive-{}.txt", tool.to_lowercase().replace(' ', "-")),
                    full(),
                ),
                _ => {}
            }
            if let Some(i) = done {
                let e = &self.log.entries[i];
                widgets::export_menu(ui, "result", || vec![e]);
                self.compare_menu(ui, i);
            }
        });
        let has_table = done.is_some_and(|i| {
            self.log.entries[i]
                .table
                .as_ref()
                .is_some_and(|t| !t.rows.is_empty())
        });
        if has_table || other.is_some() {
            ui.horizontal(|ui| {
                let views = [
                    (RunView::Output, "Output", true),
                    (RunView::Table, "Table", has_table),
                    (RunView::Compare, "Compare", other.is_some()),
                ];
                for (v, label, on) in views {
                    if on && ui.selectable_label(self.result_view == v, label).clicked() {
                        self.result_view = v;
                    }
                }
            });
        }
        ui.separator();
        if let Some((d, before, after, _)) = &diff {
            if compare_view(ui, d, before, after) {
                self.result_compare = None;
                self.result_view = RunView::Output;
            }
            return;
        }
        match (self.result_view, done) {
            (RunView::Table, Some(i)) if has_table => {
                if let Some(t) = &self.log.entries[i].table {
                    table_view(ui, t);
                }
            }
            _ => {
                egui::ScrollArea::both()
                    .auto_shrink(false)
                    .stick_to_bottom(running.is_some())
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(text.trim_end()).monospace())
                                .selectable(true)
                                .extend(),
                        );
                    });
            }
        }
    }

    /// Compare ▾: the other runs of this run's tool, newest first, those
    /// of the same target first; picking one shows what changed.
    fn compare_menu(&mut self, ui: &mut Ui, i: usize) {
        let e = &self.log.entries[i];
        let (tool, target) = (e.tool, e.target().unwrap_or("").to_string());
        let mut others: Vec<usize> = (0..self.log.entries.len())
            .filter(|&j| j != i && self.log.entries[j].tool == tool)
            .collect();
        others.sort_by_key(|&j| {
            let o = &self.log.entries[j];
            (
                o.target().unwrap_or("") != target,
                std::cmp::Reverse(o.started.clone()),
            )
        });
        let mut picked = None;
        let r = ui.add_enabled_ui(!others.is_empty(), |ui| {
            widgets::named_menu(ui, "Compare", |ui| {
                ui.set_min_width(300.0);
                ui.weak("What changed between this run and:");
                for &j in others.iter().take(40) {
                    let o = &self.log.entries[j];
                    let label = format!(
                        "{} · {} · {}",
                        when(&o.started),
                        o.target().unwrap_or(tool),
                        summarise(o)
                    );
                    let on = self.result_compare.as_deref() == Some(o.started.as_str());
                    if ui.add(egui::Button::selectable(on, label)).clicked() {
                        picked = Some(o.started.clone());
                        ui.close();
                    }
                }
            })
        });
        r.response
            .on_hover_text("What's new, gone or changed since another run of this tool")
            .on_disabled_hover_text("No other run of this tool to compare with");
        if let Some(k) = picked {
            self.result_compare = Some(k);
            self.result_view = RunView::Compare;
        }
    }

    /// Run `tab`'s tool again on `target` (the Speed Test has none).
    fn run_again(&mut self, ctx: &egui::Context, tab: Tab, target: &str) {
        match tab {
            Tab::Ping => self.ping.run(ctx, target, 4),
            Tab::Ports => self.ports.run(ctx, target),
            Tab::Trace => self.trace.run(ctx, target),
            Tab::Speed => self.speed.run(ctx),
            Tab::Sweep => self.sweep.run(ctx, target),
            Tab::Advanced => self.advanced.run(ctx, target),
            _ => {}
        }
    }
}

/// The comparison: the two runs, the figures side by side, then what's
/// new, gone and changed, each in its colour. True when Stop was clicked.
fn compare_view(ui: &mut Ui, d: &crate::runs_compare::Diff, before: &str, after: &str) -> bool {
    use crate::runs_compare::Change;
    let mut stop = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Before {before}  →  after {after}")).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            stop = ui
                .add(egui::Button::new("Stop comparing").min_size(vec2(0.0, widgets::TARGET_H)))
                .clicked();
        });
    });
    let (new, gone, changed) = (
        d.count(|c| matches!(c, Change::New(_))),
        d.count(|c| matches!(c, Change::Gone(_))),
        d.count(|c| matches!(c, Change::Changed(..))),
    );
    if d.figures.is_empty() {
        ui.weak(format!("{new} new · {gone} gone · {changed} changed"));
    }
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            if d.is_empty() {
                ui.weak("No differences: the two runs found the same.");
                return;
            }
            if !d.figures.is_empty() {
                egui::Grid::new("compare_figures")
                    .num_columns(4)
                    .striped(true)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for h in ["", "Before", "After", "Change"] {
                            ui.label(RichText::new(h).strong());
                        }
                        ui.end_row();
                        for f in &d.figures {
                            ui.label(f.name);
                            ui.monospace(&f.before);
                            ui.monospace(&f.after);
                            ui.monospace(&f.delta);
                            ui.end_row();
                        }
                    });
                ui.add_space(6.0);
            }
            for (title, colour, pick) in [
                ("New", widgets::ok(), 0),
                ("Gone", widgets::bad(), 1),
                ("Changed", widgets::warn(), 2),
            ] {
                let items: Vec<&Change> = d
                    .changes
                    .iter()
                    .filter(|c| match c {
                        Change::New(_) => pick == 0,
                        Change::Gone(_) => pick == 1,
                        Change::Changed(..) => pick == 2,
                    })
                    .collect();
                if items.is_empty() {
                    continue;
                }
                ui.label(
                    RichText::new(format!("{title} ({})", items.len()))
                        .strong()
                        .color(colour),
                );
                for c in items {
                    let line = match c {
                        Change::New(w) | Change::Gone(w) => w.clone(),
                        Change::Changed(w, how) => format!("{w}: {how}"),
                    };
                    ui.horizontal_top(|ui| {
                        ui.add_space(8.0);
                        ui.add(egui::Label::new(RichText::new(line).monospace()).wrap());
                    });
                }
                ui.add_space(6.0);
            }
        });
    stop
}

/// A day's heading in the list.
fn group(ui: &mut Ui, label: &str) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(label.to_uppercase())
            .small()
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(2.0);
}

/// One run in the list: the tool and its target with the time on the
/// first line, its summary under them; the whole row is the button.
fn run_row(
    ui: &mut Ui,
    tool: &str,
    target: &str,
    time: &str,
    line: &str,
    on: bool,
    live: bool,
) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 44.0), Sense::click());
    let v = ui.visuals();
    let fill = if on {
        theme::accent().gamma_multiply(0.16)
    } else if resp.hovered() {
        v.widgets.hovered.weak_bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, 6.0, fill);
    let dot = if live {
        theme::accent()
    } else {
        v.weak_text_color()
    };
    p.circle_filled(egui::pos2(rect.left() + 10.0, rect.top() + 14.0), 3.5, dot);
    let x = rect.left() + 20.0;
    let room = rect.right() - x - 6.0;
    let strong = if on {
        theme::accent()
    } else {
        v.strong_text_color()
    };
    let mut first = egui::text::LayoutJob::default();
    first.append(
        tool,
        0.0,
        egui::TextFormat::simple(egui::FontId::proportional(14.0), strong),
    );
    if !target.is_empty() {
        first.append(
            target,
            8.0,
            egui::TextFormat::simple(egui::FontId::monospace(13.0), v.text_color()),
        );
    }
    let time_g = p.layout_no_wrap(
        time.to_string(),
        egui::FontId::proportional(12.0),
        v.weak_text_color(),
    );
    first.wrap = egui::text::TextWrapping {
        max_width: room - time_g.size().x - 8.0,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let g = p.layout_job(first);
    p.galley(egui::pos2(x, rect.top() + 5.0), g, strong);
    p.galley(
        egui::pos2(rect.right() - 6.0 - time_g.size().x, rect.top() + 7.0),
        time_g,
        v.weak_text_color(),
    );
    let mut second = egui::text::LayoutJob::simple_singleline(
        line.to_string(),
        egui::FontId::proportional(12.5),
        v.weak_text_color(),
    );
    second.wrap = egui::text::TextWrapping {
        max_width: room,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let g = p.layout_job(second);
    p.galley(egui::pos2(x, rect.top() + 25.0), g, v.weak_text_color());
    resp.on_hover_text(line)
}

/// A run's table: its columns and rows, the rows drawn as they scroll in.
fn table_view(ui: &mut Ui, t: &log::Table) {
    let cols = t.columns.len().max(1);
    let row_h = 22.0;
    egui::ScrollArea::horizontal()
        .id_salt("result_table_h")
        .show(ui, |ui| {
            let w = (ui.available_width() / cols as f32).max(90.0);
            let grid = |ui: &mut Ui, cells: &[String], strong: bool| {
                ui.horizontal(|ui| {
                    for c in cells {
                        let text = if strong {
                            RichText::new(c).strong()
                        } else {
                            RichText::new(c).monospace()
                        };
                        ui.add_sized(
                            vec2(w, row_h),
                            egui::Label::new(text).truncate().selectable(true),
                        );
                    }
                });
            };
            let head: Vec<String> = t.columns.iter().map(|c| c.to_string()).collect();
            grid(ui, &head, true);
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("result_table_v")
                .auto_shrink(false)
                .show_rows(ui, row_h, t.rows.len(), |ui, range| {
                    for r in &t.rows[range] {
                        grid(ui, r, false);
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runs_summary_is_its_last_line_with_words() {
        assert_eq!(
            summary("#1 reply\n#2 reply\n\nSent 2, Received 2, Avg 0.4 ms\n\n"),
            "Sent 2, Received 2, Avg 0.4 ms"
        );
        assert_eq!(summary("--- \n"), "");
        let long = "x".repeat(200);
        assert_eq!(summary(&long).chars().count(), 91);
    }

    fn entry(tool: &'static str, columns: &'static [&'static str], rows: &[&[&str]]) -> log::Entry {
        log::Entry {
            tool,
            started: "2026-11-10 14:05:12".into(),
            ended: "2026-11-10 14:05:20".into(),
            text: "the output\nlast line here".into(),
            table: Some(log::Table {
                columns,
                rows: rows
                    .iter()
                    .map(|r| r.iter().map(|c| c.to_string()).collect())
                    .collect(),
            }),
        }
    }

    #[test]
    fn each_tool_says_what_its_run_found_in_a_few_words() {
        let ports = entry(
            "Port Check",
            crate::tools::ports::COLUMNS,
            &[
                &["h", "ip", "22", "open"],
                &["h", "ip", "23", "closed"],
                &["h", "ip", "443", "open"],
            ],
        );
        assert_eq!(summarise(&ports), "2 open of 3 checked");
        let speed = entry(
            "Speed Test",
            crate::tools::speed::COLUMNS,
            &[
                &["s", "ip", "idle", ""],
                &["s", "ip", "download", "665.80"],
                &["s", "ip", "upload", "38.20"],
            ],
        );
        assert_eq!(summarise(&speed), "Down 666 · up 38 Mbps");
        let domain = entry(
            "Domain Health",
            crate::tools::advanced::COLUMNS,
            &[
                &[
                    "d", "r", "core", "SPF", "WARN", "finding", "WARN", "softfail",
                ],
                &["d", "r", "core", "SPF", "WARN", "record", "", "v=spf1"],
                &["d", "r", "core", "DMARC", "FAIL", "finding", "FAIL", "none"],
                &["d", "r", "core", "MX", "PASS", "record", "", "mx1"],
            ],
        );
        assert_eq!(summarise(&domain), "1 failed · 1 warning");
        let dns = entry(
            "Lookup",
            crate::tools::dns::COLUMNS,
            &[&["q", "System", "A", "1"], &["q", "", "note", "n"]],
        );
        assert_eq!(summarise(&dns), "1 record");
        // No table (an early failure): the output's last line.
        let mut bare = entry("Ping", &[], &[]);
        bare.table = None;
        assert_eq!(summarise(&bare), "last line here");
    }

    #[test]
    fn days_times_and_durations() {
        assert_eq!(day_and_time("2026-11-10 14:05:12"), ("2026-11-10", "14:05"));
        assert_eq!(
            took("2026-11-10 14:05:12", "2026-11-10 14:05:40").as_deref(),
            Some("28 s")
        );
        assert_eq!(
            took("2026-11-10 14:05:12", "2026-11-10 14:07:15").as_deref(),
            Some("2 min 3 s")
        );
        assert_eq!(took("nonsense", "2026-11-10 14:07:15"), None);
        assert_eq!(took("2026-11-10 14:07:15", "2026-11-10 14:05:12"), None);
    }

    #[test]
    fn the_list_is_narrower_on_a_small_window_but_always_there() {
        assert_eq!(list_width(1920.0), 300.0);
        assert_eq!(list_width(1280.0), 268.0);
        assert!(list_width(620.0) >= 200.0);
    }

    #[test]
    fn the_tools_that_can_run_again_from_their_target() {
        for tool in [
            "Ping",
            "Port Check",
            "Traceroute",
            "Speed Test",
            "Subnet Sweep",
            "Domain Health",
        ] {
            assert!(tab_of(tool).is_some(), "{tool}");
        }
        // Host info has no tab; a Lookup's target isn't a query to repeat.
        assert!(tab_of("Host info").is_none());
        assert!(tab_of("Lookup").is_none());
    }
}
