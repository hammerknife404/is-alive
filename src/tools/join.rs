//! Domain join: how this computer is joined (Active Directory, Entra ID,
//! hybrid, or not at all) and to which domain, from `dsregcmd /status` on
//! Windows or `realm list` on Linux. Checked once on start for the status
//! bar; a check from this tab or a Full scan is logged to Results.

use crate::job::{Job, pump};
use crate::log::{self, Table};
use crate::net::join::{self, Join, Kind};
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
        let running = self.is_running();
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!running, |ui| {
                if ui
                    .add(egui::Button::new("▶ Check").min_size(egui::vec2(0.0, widgets::TARGET_H)))
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
