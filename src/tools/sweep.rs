use crate::job::{Job, pump};
use crate::net::icmp;
use crate::net::{self, targets};
use crate::widgets::{self, Action, OK};
use eframe::egui::{self, RichText, Ui};
use std::net::{IpAddr, Ipv4Addr};
use std::time::Instant;

struct Host {
    ip: Ipv4Addr,
    rtt: f64,
    ttl: Option<u8>,
    name: Option<String>,
}

enum Ev {
    Probed,
    Alive(Host),
}

pub struct SweepTool {
    range: String,
    detected: bool,
    timeout_ms: u32,
    threads: usize,
    rate: u32,
    resolve_names: bool,
    job: Option<Job<Ev>>,
    run_started: Option<String>,
    started_at: Instant,
    total: usize,
    probed: usize,
    alive: Vec<Host>,
    err: Option<String>,
}

impl Default for SweepTool {
    fn default() -> Self {
        Self {
            range: "192.168.1.0/24".into(),
            detected: false,
            timeout_ms: 750,
            threads: widgets::SPEEDS[widgets::DEFAULT_SPEED].1,
            rate: widgets::SPEEDS[widgets::DEFAULT_SPEED].2,
            resolve_names: true,
            job: None,
            run_started: None,
            started_at: Instant::now(),
            total: 0,
            probed: 0,
            alive: Vec::new(),
            err: None,
        }
    }
}

impl SweepTool {
    /// Drain worker messages. Called every frame so results stay current
    /// while another tab is shown.
    pub fn poll(&mut self) -> bool {
        let mut got = false;
        let running = pump(&mut self.job, |ev| match ev {
            Ev::Probed => self.probed += 1,
            Ev::Alive(h) => {
                self.alive.push(h);
                got = true;
            }
        });
        if got {
            self.alive.sort_by_key(|h| h.ip);
        }
        running
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        // First time the tab is shown: prefill with the host's own subnet.
        if !self.detected {
            self.detected = true;
            if let Some(r) = targets::local_subnet() {
                self.range = r;
            }
        }
        let running = self.poll();

        let mut enter = false;
        ui.horizontal(|ui| {
            enter |= widgets::field(
                ui,
                "Range",
                &mut self.range,
                220.0,
                "10.0.0.0/24 or 10.0.0.1-254",
            );
            ui.separator();
            match widgets::run_button(ui, running, enter) {
                Action::Start => self.start(ui.ctx()),
                Action::Stop => self.job.as_ref().map_or((), Job::stop),
                Action::None => {}
            }
        });
        ui.horizontal(|ui| {
            widgets::num(ui, "Timeout", &mut self.timeout_ms, 100..=10_000, " ms");
            widgets::speed(ui, "sweep_speed", &mut self.threads, &mut self.rate);
            ui.checkbox(&mut self.resolve_names, "Resolve names");
        });
        widgets::error(ui, &self.err);
        ui.separator();

        ui.horizontal(|ui| {
            if self.total > 0 {
                ui.label(
                    RichText::new(format!("{} alive", self.alive.len()))
                        .color(OK)
                        .strong(),
                );
                ui.label(format!("· {} / {} probed", self.probed, self.total));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(self.total > 0, |ui| {
                    widgets::save_button(ui, "sweep", || self.report());
                    widgets::copy_button(ui, || self.report());
                });
            });
        });
        if running && self.total > 0 {
            widgets::progress(ui, self.probed, self.total, self.started_at);
        }
        if self.alive.is_empty() && !running {
            let hint = if self.total > 0 {
                "No hosts answered. Firewalls often drop ping; try Port Check on a known host."
            } else {
                "Enter a CIDR or range and press Enter."
            };
            widgets::empty_hint(ui, hint);
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Grid::new("sweep")
                    .striped(true)
                    .min_col_width(70.0)
                    .show(ui, |ui| {
                        for h in ["Address", "Time", "TTL", "Name"] {
                            ui.strong(h);
                        }
                        ui.end_row();
                        for h in &self.alive {
                            ui.monospace(h.ip.to_string());
                            ui.monospace(widgets::fmt_ms(h.rtt));
                            ui.monospace(h.ttl.map(|t| t.to_string()).unwrap_or_default());
                            ui.label(h.name.as_deref().unwrap_or(""));
                            ui.end_row();
                        }
                    });
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
        (self.total > 0).then(|| self.report())
    }

    fn report(&self) -> String {
        let mut s = format!(
            "Sweep of {}: {} alive\n",
            self.range.trim(),
            self.alive.len()
        );
        for h in &self.alive {
            s += &format!(
                "{:<15}  {:>6.1} ms  {}\n",
                h.ip,
                h.rtt,
                h.name.as_deref().unwrap_or("")
            );
        }
        s
    }

    /// Start a run with the given target (used by the full scan).
    pub fn run(&mut self, ctx: &egui::Context, range: &str) {
        self.range = range.into();
        self.detected = true;
        self.start(ctx);
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.err = None;
        self.alive.clear();
        self.probed = 0;
        let ips = match targets::parse_range(&self.range) {
            Ok(v) => v,
            Err(e) => return self.err = Some(e),
        };
        self.total = ips.len();
        let (timeout, threads, rate, names) =
            (self.timeout_ms, self.threads, self.rate, self.resolve_names);

        self.run_started = Some(crate::log::now());
        self.started_at = Instant::now();
        self.job = Some(Job::spawn(ctx, move |tx| {
            net::par_for(&ips, threads, rate, tx.cancel_flag(), |&ip| {
                let r = icmp::echo(IpAddr::V4(ip), 128, timeout, 32);
                if r.is_reply() {
                    let name = if names {
                        net::reverse(IpAddr::V4(ip))
                    } else {
                        None
                    };
                    tx.send(Ev::Alive(Host {
                        ip,
                        rtt: r.rtt_ms,
                        ttl: r.ttl,
                        name,
                    }));
                }
                tx.send(Ev::Probed);
            });
        }));
    }
}
