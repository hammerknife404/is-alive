//! Domain join: how this computer is joined (Active Directory, Entra ID,
//! hybrid, or not at all) and to which domain, from `dsregcmd /status` on
//! Windows or `realm list` on Linux. Checked once on start for the status
//! bar; a check from this tab or a Full scan is logged to Results.
//!
//! **Other computers**: whether the computers you name are joined to
//! Active Directory, asked without credentials (`net::joined`). Never
//! part of a sweep or a Full scan: it asks only what's typed in its box.

use crate::job::{Job, pump};
use crate::log::{self, Table};
use crate::net::join::{self, Join, Kind};
use crate::net::joined::{self, Answer, Verdict};
use crate::widgets;
use eframe::egui::{self, RichText, Ui};

#[derive(Default)]
pub struct DomainJoin {
    job: Option<Job<Join>>,
    /// The last answer.
    pub last: Option<Join>,
    /// When the check under way started, if it's to be logged.
    log_from: Option<String>,
    /// A finished check waiting to go to Results.
    done: Option<(String, Join)>,
    /// Show every field the tool printed, not just the ones that matter.
    all: bool,
    /// Which half of the tab shows.
    view: View,
    others: Others,
}

/// The run of other computers that Results logs.
pub const OTHERS: &str = "Domain join (others)";

#[derive(Default, Clone, Copy, PartialEq)]
enum View {
    #[default]
    This,
    Others,
}

/// Other computers: what's typed, the run under way and its answers.
#[derive(Default)]
struct Others {
    input: String,
    job: Option<Job<Answer>>,
    /// The answers so far, in the order asked.
    rows: Vec<Answer>,
    /// How many were asked.
    total: usize,
    started: Option<String>,
    /// A finished run waiting to go to Results.
    done: Option<(String, Vec<Answer>)>,
    err: Option<String>,
}

impl DomainJoin {
    /// Ask again. `log`: the check goes to Results when it's done (from
    /// this tab or a Full scan; not the quiet one on start).
    pub fn check(&mut self, ctx: &egui::Context, log: bool) {
        if self.job.is_some() {
            // One already going: log it if this one would have been.
            if log && self.log_from.is_none() {
                self.log_from = Some(log::now());
            }
            return;
        }
        self.log_from = log.then(log::now);
        self.job = Some(Job::spawn(ctx, |tx| tx.send(join::status())));
    }

    /// Take the answer in; true while a check runs. Called every frame,
    /// whichever tab is on screen, for the status bar.
    pub fn poll(&mut self) -> bool {
        self.poll_others();
        let mut got = None;
        let running = pump(&mut self.job, |j| got = Some(j));
        if let Some(j) = got {
            if let Some(started) = self.log_from.take() {
                self.done = Some((started, j.clone()));
            }
            self.last = Some(j);
        } else if !running {
            // Ended without an answer (it panicked): nothing to log.
            self.log_from = None;
        }
        running
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    /// A card or a row asks about its computer: its address goes in the
    /// Other computers box (added to what's there), and that view shows.
    /// Nothing runs until Start: asking is the technician's call.
    pub fn ask_about(&mut self, host: &str) {
        self.view = View::Others;
        let o = &mut self.others;
        let there = joined::hosts(&o.input).unwrap_or_default();
        if !there.iter().any(|h| h.eq_ignore_ascii_case(host)) {
            if !o.input.trim().is_empty() {
                o.input = format!("{}, ", o.input.trim().trim_end_matches(','));
            }
            o.input += host;
        }
        o.err = None;
    }

    /// Ask every computer named, a few at a time.
    fn ask_others(&mut self, ctx: &egui::Context) {
        let o = &mut self.others;
        let hosts = match joined::hosts(&o.input) {
            Ok(h) if h.is_empty() => {
                o.err = Some("Name a computer: its name or its address".into());
                return;
            }
            Ok(h) => h,
            Err(e) => {
                o.err = Some(e);
                return;
            }
        };
        o.err = None;
        o.rows.clear();
        o.total = hosts.len();
        o.started = Some(log::now());
        o.job = Some(Job::spawn(ctx, move |tx| {
            crate::net::par_for(&hosts, 8, 0, tx.cancel_flag(), |h| {
                if !tx.cancelled() {
                    tx.send(joined::ask(h));
                }
            });
        }));
    }

    /// Take the others' answers in; true while that run goes.
    fn poll_others(&mut self) -> bool {
        let o = &mut self.others;
        let had = o.job.is_some();
        let mut got = Vec::new();
        let running = pump(&mut o.job, |a| got.push(a));
        o.rows.extend(got);
        if had && !running {
            // As typed, not as they happened to answer.
            let order = joined::hosts(&o.input).unwrap_or_default();
            o.rows.sort_by_key(|r| {
                order
                    .iter()
                    .position(|h| h.eq_ignore_ascii_case(&r.host))
                    .unwrap_or(usize::MAX)
            });
            if let Some(started) = o.started.take()
                && !o.rows.is_empty()
            {
                o.done = Some((started, o.rows.clone()));
            }
        }
        running
    }

    pub fn others_running_since(&self) -> Option<&str> {
        self.others.job.as_ref().and(self.others.started.as_deref())
    }

    /// The others' answers so far, for Results while the run goes.
    pub fn others_output(&self) -> Option<String> {
        self.others.job.as_ref()?;
        Some(others_text(&self.others.rows))
    }

    /// Start time, logged text and table of a run of other computers that
    /// has just ended, once.
    pub fn take_finished_others(&mut self) -> Option<(String, String, Option<Table>)> {
        let (started, rows) = self.others.done.take()?;
        let table = Table {
            columns: joined::COLUMNS,
            rows: rows.iter().map(Answer::row).collect(),
        };
        Some((started, others_text(&rows), Some(table)))
    }

    pub fn running_since(&self) -> Option<&str> {
        self.job.as_ref().and(self.log_from.as_deref())
    }

    /// Start time, logged text and table of a logged check that has just
    /// ended, once.
    pub fn take_finished(&mut self) -> Option<(String, String, Option<Table>)> {
        let (started, j) = self.done.take()?;
        Some((started, j.summary(), Some(j.table())))
    }

    /// For the status bar: "corp.example.com", "checking…".
    pub fn short(&self) -> String {
        match (&self.last, self.job.is_some()) {
            (Some(j), _) => j.short(),
            (None, true) => "checking…".into(),
            (None, false) => "not checked".into(),
        }
    }

    /// For the status bar's hover: the verdict and the names behind it.
    pub fn tip(&self) -> String {
        let Some(j) = &self.last else {
            return "How this computer is joined: Active Directory, Entra ID, both or none. \
                    Click for the Domain join tab."
                .into();
        };
        let mut s = j.kind.name().to_string();
        if let Some(d) = &j.dns {
            s += &format!("\nAD domain: {d}");
        }
        if let Some(n) = &j.netbios {
            s += &format!("\nShort name: {n}");
        }
        if let Some(t) = &j.tenant {
            s += &format!("\nEntra tenant: {t}");
        }
        if let Some(e) = &j.error {
            s += &format!("\n{e}");
        }
        s + &format!("\nFrom {}. Click for the Domain join tab.", j.source)
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            widgets::segmented(ui, |ui| {
                ui.selectable_value(&mut self.view, View::This, "This computer")
                    .on_hover_text("How the computer is-alive runs on is joined");
                ui.selectable_value(&mut self.view, View::Others, "Other computers")
                    .on_hover_text(
                        "Whether computers you name are joined to Active Directory, asked \
                         without signing in",
                    );
            });
        });
        match self.view {
            View::This => self.this_ui(ui),
            View::Others => self.others_ui(ui),
        }
    }

    fn others_ui(&mut self, ui: &mut Ui) {
        let running = self.others.job.is_some();
        let mut enter = false;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!running, |ui| {
                let room = (ui.available_width() - widgets::LEAD_W - 110.0).clamp(160.0, 560.0);
                enter = widgets::field(
                    ui,
                    "Computers",
                    crate::input::Field::Range,
                    &mut self.others.input,
                    room,
                    "names or addresses: pc-01, 10.0.0.7",
                );
            });
            match widgets::run_button(ui, running, enter) {
                widgets::Action::Start => self.ask_others(ui.ctx()),
                widgets::Action::Stop => self.others.job.as_ref().map_or((), Job::stop),
                widgets::Action::None => {}
            }
        });
        widgets::toolbar_note(
            ui,
            "Starts a sign-in on SMB (445), or RDP (3389) when that's shut, and stops at the \
             server's first reply, which names the computer and its domain. No user name or \
             password is sent. An Entra ID join doesn't show this way.",
        );
        widgets::error(ui, &self.others.err);
        let rows = &self.others.rows;
        let text = others_text(rows);
        widgets::status_row(
            ui,
            |ui| {
                if running {
                    ui.label(format!("{} of {} asked", rows.len(), self.others.total));
                } else if !rows.is_empty() {
                    let ad = rows
                        .iter()
                        .filter(|r| matches!(r.verdict, Verdict::Ad(_)))
                        .count();
                    ui.label(format!("{ad} of {} in Active Directory", rows.len()));
                }
            },
            |ui| {
                ui.add_enabled_ui(!rows.is_empty(), |ui| {
                    widgets::save_button(ui, "domain-join-others", || text.clone());
                    widgets::copy_button(ui, || text.clone());
                });
            },
        );
        ui.separator();
        if rows.is_empty() {
            if !running {
                widgets::empty_state(
                    ui,
                    "Is that computer in the domain?",
                    "Name one computer or several (32 at most) and press Enter. Each says its \
                     own name and its domain's; a Map card or a Sweep row can fill this box.",
                    &[],
                );
            }
            return;
        }
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            let weak = ui.visuals().weak_text_color();
            egui::Grid::new("join_others")
                .num_columns(6)
                .spacing(egui::vec2(18.0, 4.0))
                .striped(true)
                .show(ui, |ui| {
                    for h in [
                        "Asked",
                        "Computer",
                        "Domain",
                        "DNS domain",
                        "Verdict",
                        "Port",
                    ] {
                        ui.label(RichText::new(h).color(weak));
                    }
                    ui.end_row();
                    for r in rows {
                        let cell = |ui: &mut Ui, v: &Option<String>| {
                            ui.add(egui::Label::new(v.as_deref().unwrap_or("")).selectable(true));
                        };
                        ui.add(
                            egui::Label::new(RichText::new(&r.host).monospace()).selectable(true),
                        );
                        cell(ui, &r.names.computer);
                        cell(ui, &r.names.domain);
                        cell(ui, &r.names.dns_domain);
                        let (colour, tip) = match &r.verdict {
                            Verdict::Ad(_) => (
                                widgets::ok(),
                                "Its reply names a domain that isn't itself, with a DNS name: a \
                                 member of that Active Directory domain (or one of its domain \
                                 controllers)",
                            ),
                            Verdict::NotAd => (
                                ui.visuals().text_color(),
                                "Its reply names no domain but itself. It may still be joined \
                                 to Entra ID: that shows only to someone signed in",
                            ),
                            Verdict::CantTell => {
                                (widgets::warn(), "A port answered, but not with its names")
                            }
                            Verdict::NoAnswer => (
                                weak,
                                "Neither port answered: off, firewalled, or not a Windows \
                                 computer",
                            ),
                        };
                        let tip = if r.detail.is_empty() {
                            tip.to_string()
                        } else {
                            format!("{tip}\n{}", r.detail)
                        };
                        ui.label(RichText::new(r.verdict.text()).color(colour))
                            .on_hover_text(tip);
                        ui.label(r.port.map(|p| p.to_string()).unwrap_or_default());
                        ui.end_row();
                    }
                });
        });
    }

    fn this_ui(&mut self, ui: &mut Ui) {
        let running = self.is_running();
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!running, |ui| {
                if ui
                    .add(widgets::primary("▶ Check"))
                    .on_hover_text("Ask again, and log the answer to Results")
                    .clicked()
                {
                    self.check(ui.ctx(), true);
                }
            });
            if running {
                widgets::spinner(ui);
            }
            ui.checkbox(&mut self.all, "Everything")
                .on_hover_text(if cfg!(windows) {
                    "Every field dsregcmd printed, the signed-in account and diagnostics among \
                     them. Off: the fields that say how it's joined and whether that works \
                     (what Results keeps)"
                } else {
                    "Every field realm list printed. Off: the ones that say how it's joined"
                });
        });
        widgets::toolbar_note(
            ui,
            if cfg!(windows) {
                "Asks dsregcmd /status, as you would at a prompt: Active Directory, Entra ID \
                 (Azure AD), hybrid, or a workgroup. The domain also shows in the status bar."
            } else {
                "Asks realmd (realm list) whether this computer is a member of a domain. The \
                 domain also shows in the status bar."
            },
        );
        // What Copy and Save… take: as printed with Everything on, else
        // what Results keeps.
        let text = self.last.as_ref().map_or_else(String::new, |j| {
            if self.all && !j.raw.trim().is_empty() {
                j.raw.clone()
            } else {
                j.summary()
            }
        });
        widgets::status_row(
            ui,
            |ui| {
                if let Some(j) = &self.last {
                    let colour = match j.kind {
                        k if k.joined() => widgets::ok(),
                        Kind::Unknown => widgets::warn(),
                        _ => ui.visuals().text_color(),
                    };
                    ui.label(RichText::new(j.kind.name()).strong().color(colour));
                }
            },
            |ui| {
                ui.add_enabled_ui(self.last.is_some(), |ui| {
                    widgets::save_button(ui, "domain-join", || text.clone());
                    widgets::copy_button(ui, || text.clone());
                });
            },
        );
        ui.separator();
        let Some(j) = &self.last else {
            widgets::empty_hint(
                ui,
                if running {
                    "Checking…"
                } else {
                    "Not checked yet: ▶ Check"
                },
            );
            return;
        };
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| facts(ui, j, self.all));
    }
}

/// A run of other computers as text: a line each.
fn others_text(rows: &[Answer]) -> String {
    rows.iter().map(|r| r.line() + "\n").collect()
}

/// The answer: the names first, then each section's fields.
fn facts(ui: &mut Ui, j: &Join, all: bool) {
    let weak = ui.visuals().weak_text_color();
    let grid = |ui: &mut Ui, id: &str, rows: &[(&str, &str)]| {
        egui::Grid::new(id)
            .num_columns(2)
            .spacing(egui::vec2(16.0, 4.0))
            .striped(true)
            .show(ui, |ui| {
                for (k, v) in rows {
                    ui.label(RichText::new(*k).color(weak));
                    ui.add(egui::Label::new(value(ui, v)).wrap().selectable(true));
                    ui.end_row();
                }
            });
    };
    let mut names: Vec<(&str, &str)> = Vec::new();
    if let Some(d) = &j.dns {
        names.push(("AD domain", d));
    }
    if let Some(n) = &j.netbios {
        names.push(("Short name", n));
    }
    if let Some(t) = &j.tenant {
        names.push(("Entra tenant", t));
    }
    names.push(("Asked", j.source));
    ui.add_space(6.0);
    grid(ui, "join_names", &names);
    if let Some(e) = &j.error {
        ui.add_space(6.0);
        widgets::warning(ui, e);
    }
    let shown: Vec<&(String, String, String)> = j
        .fields
        .iter()
        .filter(|f| all || join::keeps(&f.1))
        .collect();
    let mut sections: Vec<&str> = Vec::new();
    for f in &shown {
        if !sections.contains(&f.0.as_str()) {
            sections.push(&f.0);
        }
    }
    for s in sections {
        crate::inspector::section(ui, if s.is_empty() { "Fields" } else { s });
        let rows: Vec<(&str, &str)> = shown
            .iter()
            .filter(|f| f.0 == s)
            .map(|f| (f.1.as_str(), f.2.as_str()))
            .collect();
        grid(ui, &format!("join_{s}"), &rows);
    }
}

/// YES in the ok colour, NO plain, the rest as printed.
fn value(ui: &Ui, v: &str) -> RichText {
    match v {
        "YES" | "SUCCESS" => RichText::new(v).color(widgets::ok()),
        "NO" => RichText::new(v).color(ui.visuals().weak_text_color()),
        _ => RichText::new(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::joined::Names;

    fn answer(host: &str, verdict: Verdict) -> Answer {
        Answer {
            host: host.into(),
            names: Names {
                computer: Some(host.to_uppercase()),
                ..Default::default()
            },
            verdict,
            port: Some(445),
            detail: String::new(),
        }
    }

    /// A Map card or a Sweep row fills the box and shows the view. It
    /// never starts the run: nothing is sent until Start.
    #[test]
    fn a_card_or_a_row_fills_the_box_and_starts_nothing() {
        let mut j = DomainJoin::default();
        j.ask_about("10.0.0.7");
        assert!(j.view == View::Others);
        assert_eq!(j.others.input, "10.0.0.7");
        assert!(j.others.job.is_none() && j.others_running_since().is_none());
        // Another is added to it; the same one again isn't.
        j.ask_about("pc-01");
        j.ask_about("10.0.0.7");
        assert_eq!(j.others.input, "10.0.0.7, pc-01");
    }

    /// A range is refused where it's typed, with why; so is an empty box.
    #[test]
    fn a_range_or_nothing_is_refused_before_anything_is_sent() {
        let ctx = egui::Context::default();
        let mut j = DomainJoin::default();
        j.ask_others(&ctx);
        assert!(j.others.err.as_deref().unwrap().contains("Name a computer"));
        j.others.input = "10.0.0.0/24".into();
        j.ask_others(&ctx);
        assert!(j.others.err.as_deref().unwrap().contains("range"));
        assert!(j.others.job.is_none());
    }

    #[test]
    fn a_finished_run_goes_to_results_once_with_a_row_a_computer() {
        let mut j = DomainJoin::default();
        j.others.done = Some((
            "2026-11-12T10:00:00+00:00".into(),
            vec![
                answer("pc-01", Verdict::Ad("harbor.example".into())),
                answer("kiosk", Verdict::NotAd),
            ],
        ));
        let (started, text, table) = j.take_finished_others().unwrap();
        assert_eq!(started, "2026-11-12T10:00:00+00:00");
        assert!(text.contains("pc-01: AD: harbor.example"));
        assert!(text.contains("kiosk: Not AD-joined (Entra not checked)"));
        let t = table.unwrap();
        assert_eq!(t.columns, joined::COLUMNS);
        assert_eq!(t.rows.len(), 2);
        assert!(t.rows.iter().all(|r| r.len() == t.columns.len()));
        assert!(j.take_finished_others().is_none());
    }

    /// Both halves of the tab draw, empty and with every verdict in it.
    #[test]
    fn both_views_draw_with_every_verdict() {
        let ctx = egui::Context::default();
        let mut j = DomainJoin::default();
        j.others.rows = vec![
            answer("pc-01", Verdict::Ad("harbor.example".into())),
            answer("kiosk", Verdict::NotAd),
            answer("nas", Verdict::CantTell),
            answer("10.0.0.99", Verdict::NoAnswer),
        ];
        for view in [View::This, View::Others, View::Others] {
            j.view = view;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| j.ui(ui));
            });
            out.textures_delta.clear();
            j.others.rows.clear();
        }
    }
}
