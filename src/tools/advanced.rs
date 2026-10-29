use crate::job::{Job, pump};
use crate::net::dnsq::{self, Client};
use crate::net::domain::{self, CORE, Check, NAMES, Status};
use crate::widgets::{self, Action};
use eframe::egui::{self, Color32, RichText, Ui};
use std::time::Instant;

const DEFAULT_RESOLVER: &str = "1.1.1.1";

enum Ev {
    Check(usize, Check),
    Done(f64),
}

pub struct AdvancedTool {
    domain: String,
    resolver: String,
    selectors: String,
    job: Option<Job<Ev>>,
    run_started: Option<String>,
    /// Domain and resolver of the last run, for the report header.
    ran: (String, String),
    checks: Vec<Option<Check>>,
    took: Option<f64>,
    err: Option<String>,
}

impl Default for AdvancedTool {
    fn default() -> Self {
        Self {
            domain: String::new(),
            resolver: DEFAULT_RESOLVER.into(),
            selectors: String::new(),
            job: None,
            run_started: None,
            ran: Default::default(),
            checks: Vec::new(),
            took: None,
            err: None,
        }
    }
}

fn color(s: Status) -> Color32 {
    match s {
        Status::Info => Color32::GRAY,
        Status::Pass => widgets::OK,
        Status::Warn => widgets::WARN,
        Status::Fail => widgets::BAD,
    }
}

fn icon(s: Status) -> &'static str {
    match s {
        Status::Info => "ℹ",
        Status::Pass => "✔",
        Status::Warn => "⚠",
        Status::Fail => "✖",
    }
}

impl AdvancedTool {
    /// Drain worker messages. Called every frame so results stay current
    /// while another tab is shown.
    pub fn poll(&mut self) -> bool {
        pump(&mut self.job, |ev| match ev {
            Ev::Check(i, c) => self.checks[i] = Some(c),
            Ev::Done(ms) => self.took = Some(ms),
        })
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let running = self.poll();

        let mut enter = false;
        ui.horizontal(|ui| {
            enter |= widgets::field(ui, "Domain", &mut self.domain, 200.0, "example.com");
            enter |= widgets::field(ui, "Resolver", &mut self.resolver, 110.0, DEFAULT_RESOLVER);
            ui.separator();
            match widgets::run_button(ui, running, enter) {
                Action::Start => self.start(ui.ctx()),
                Action::Stop => self.job.as_ref().map_or((), Job::stop),
                Action::None => {}
            }
        });
        ui.horizontal(|ui| {
            // Enter here starts a run too; the Start button above has already been drawn.
            let enter = widgets::field(
                ui,
                "Extra DKIM selectors",
                &mut self.selectors,
                200.0,
                "s1024, mykey",
            );
            ui.weak("(common ones are always probed)")
                .on_hover_text(domain::SELECTORS.join(", "));
            if enter && !running {
                self.start(ui.ctx());
            }
        });
        ui.weak(
            "Mail and DNS health: asks the resolver above directly, not the system one, so you see what the internet sees.",
        );
        widgets::error(ui, &self.err);
        ui.separator();

        ui.horizontal(|ui| {
            let done: Vec<Status> = self.checks.iter().flatten().map(Check::status).collect();
            if !done.is_empty() {
                for s in [Status::Fail, Status::Warn, Status::Pass] {
                    let n = done.iter().filter(|&&d| d == s).count();
                    ui.label(
                        RichText::new(format!("{n} {}", s.tag().to_lowercase())).color(color(s)),
                    );
                }
                if let Some(ms) = self.took {
                    ui.weak(format!("in {}", widgets::fmt_ms(ms)));
                } else {
                    ui.weak(format!("{}/{}", done.len(), NAMES.len()));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!done.is_empty(), |ui| {
                    widgets::save_button(ui, "domain", || self.report());
                    widgets::copy_button(ui, || self.report());
                });
            });
        });
        if self.checks.is_empty() {
            widgets::empty_hint(
                ui,
                "Enter a domain and press Enter to check MX, SPF, DMARC, DKIM, nameservers and more.",
            );
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (i, name) in NAMES.iter().enumerate() {
                    if i == 0 || i == CORE {
                        ui.add_space(4.0);
                        ui.heading(if i == 0 {
                            "Core (email)"
                        } else {
                            "Nice-to-haves"
                        });
                    }
                    check_ui(ui, name, self.checks[i].as_ref(), running);
                }
            });
    }

    /// Start time and logged text of a run that has just ended, once.
    pub fn take_finished(&mut self) -> Option<(String, String)> {
        if self.job.is_some() {
            return None;
        }
        let started = self.run_started.take()?;
        Some((started, crate::log::body(self.output(), &self.err)))
    }

    /// Start time of the run in progress.
    pub fn running_since(&self) -> Option<&str> {
        self.job.as_ref().and(self.run_started.as_deref())
    }

    /// Report text for the Results tab; None until something has run.
    pub fn output(&self) -> Option<String> {
        self.checks
            .iter()
            .any(Option::is_some)
            .then(|| self.report())
    }

    fn report(&self) -> String {
        let mut s = format!("Domain health of {} via {}\n", self.ran.0, self.ran.1);
        for (i, name) in NAMES.iter().enumerate() {
            if i == 0 || i == CORE {
                s += if i == 0 {
                    "\n-- Core (email) --\n"
                } else {
                    "\n-- Nice-to-haves --\n"
                };
            }
            let Some(Some(c)) = self.checks.get(i) else {
                s += &format!("[ -- ] {name}: not run\n");
                continue;
            };
            s += &format!("[{}] {name}\n", c.status().tag());
            for (st, f) in &c.findings {
                s += &format!("       {} {f}\n", icon(*st));
            }
            for r in &c.records {
                s += &format!("         | {r}\n");
            }
        }
        s
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.err = None;
        self.took = None;
        self.checks.clear();
        let d = domain::normalize(&self.domain);
        if d.is_empty() || !d.contains('.') {
            self.err = Some("enter a domain like example.com".into());
            return;
        }
        let res = if self.resolver.trim().is_empty() {
            DEFAULT_RESOLVER
        } else {
            self.resolver.trim()
        };
        let Some(server) = dnsq::parse_server(res) else {
            self.err = Some(format!("resolver must be an IP address, got '{res}'"));
            return;
        };
        self.domain = d.clone();
        self.ran = (d.clone(), server.ip().to_string());
        self.checks = vec![None; NAMES.len()];
        let selectors: Vec<String> = self
            .selectors
            .split([',', ' ', ';'])
            .map(|s| s.trim().trim_end_matches("._domainkey").to_string())
            .filter(|s| !s.is_empty())
            .collect();
        self.run_started = Some(crate::log::now());
        self.job = Some(Job::spawn(ctx, move |tx| {
            let t0 = Instant::now();
            let c = Client::new(server);
            domain::run(&c, &d, &selectors, tx.cancel_flag(), |i, check| {
                tx.send(Ev::Check(i, check))
            });
            tx.send(Ev::Done(t0.elapsed().as_secs_f64() * 1000.0));
        }));
    }
}

fn check_ui(ui: &mut Ui, name: &str, check: Option<&Check>, running: bool) {
    let Some(c) = check else {
        ui.horizontal(|ui| {
            if running {
                ui.spinner();
            } else {
                ui.weak("–");
            }
            ui.weak(name);
        });
        return;
    };
    let st = c.status();
    // Headline: the worst finding, so a collapsed row still says what's wrong.
    let headline = c
        .findings
        .iter()
        .find(|f| f.0 == st)
        .map_or("", |f| f.1.as_str());
    let mut header = egui::text::LayoutJob::default();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let fmt = |c: Color32| egui::TextFormat::simple(font.clone(), c);
    header.append(&format!("{} {} ", icon(st), st.tag()), 0.0, fmt(color(st)));
    header.append(name, 6.0, fmt(ui.visuals().strong_text_color()));
    header.append(headline, 12.0, fmt(ui.visuals().weak_text_color()));
    egui::CollapsingHeader::new(header)
        .id_salt(("adv", name))
        .default_open(st >= Status::Warn)
        .show(ui, |ui| {
            for (s, f) in &c.findings {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(icon(*s)).color(color(*s)));
                    ui.label(f);
                });
            }
            if !c.records.is_empty() {
                ui.add_space(2.0);
                ui.add(
                    egui::Label::new(RichText::new(c.records.join("\n")).monospace().weak())
                        .selectable(true),
                );
            }
        });
}
