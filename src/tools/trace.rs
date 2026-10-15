use crate::job::{Job, pump};
use crate::net;
use crate::net::icmp::{self, EchoStatus};
use crate::widgets::{self, Action, OK, WARN};
use eframe::egui::{self, RichText, Ui};
use std::net::IpAddr;

const PROBES: usize = 3;

struct Hop {
    ttl: u8,
    rtts: [Option<f64>; PROBES],
    addr: Option<IpAddr>,
    name: Option<String>,
    note: String,
}

enum Ev {
    Resolved(IpAddr),
    Hop(Hop),
    Done(bool),
    Error(String),
}

pub struct TraceTool {
    host: String,
    max_hops: u8,
    timeout_ms: u32,
    resolve_names: bool,
    prefer_v6: bool,
    job: Option<Job<Ev>>,
    run_started: Option<String>,
    target: Option<IpAddr>,
    hops: Vec<Hop>,
    reached: Option<bool>,
    err: Option<String>,
}

impl Default for TraceTool {
    fn default() -> Self {
        Self {
            host: String::new(),
            max_hops: 30,
            timeout_ms: 1000,
            resolve_names: true,
            prefer_v6: false,
            job: None,
            run_started: None,
            target: None,
            hops: Vec::new(),
            reached: None,
            err: None,
        }
    }
}

impl TraceTool {
    /// Drain worker messages. Called every frame so results stay current
    /// while another tab is shown.
    pub fn poll(&mut self) -> bool {
        pump(&mut self.job, |ev| match ev {
            Ev::Resolved(ip) => self.target = Some(ip),
            Ev::Hop(h) => self.hops.push(h),
            Ev::Done(r) => self.reached = Some(r),
            Ev::Error(e) => self.err = Some(e),
        })
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let running = self.poll();

        let mut enter = false;
        ui.horizontal(|ui| {
            enter |= widgets::field(ui, "Host", &mut self.host, 220.0, "hostname or IP");
            ui.separator();
            match widgets::run_button(ui, running, enter) {
                Action::Start => self.start(ui.ctx()),
                Action::Stop => self.job.as_ref().map_or((), Job::stop),
                Action::None => {}
            }
        });
        ui.horizontal(|ui| {
            widgets::num(ui, "Max hops", &mut self.max_hops, 1..=64, "");
            widgets::num(ui, "Timeout", &mut self.timeout_ms, 100..=10_000, " ms");
            ui.checkbox(&mut self.resolve_names, "Resolve names");
            ui.checkbox(&mut self.prefer_v6, "Prefer IPv6");
        });
        widgets::error(ui, &self.err);
        ui.separator();

        ui.horizontal(|ui| {
            if let Some(ip) = self.target {
                ui.label(RichText::new(format!("Tracing {ip}")).strong());
            }
            match self.reached {
                Some(true) => _ = ui.label(RichText::new("✔ destination reached").color(OK)),
                Some(false) => _ = ui.label(RichText::new("destination not reached").color(WARN)),
                None => {}
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                widgets::save_button(ui, "traceroute", || self.report());
                widgets::copy_button(ui, || self.report());
            });
        });

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Grid::new("trace")
                    .striped(true)
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        for h in ["Hop", "RTT 1", "RTT 2", "RTT 3", "Address", "Name"] {
                            ui.strong(h);
                        }
                        ui.end_row();
                        for h in &self.hops {
                            ui.monospace(h.ttl.to_string());
                            for r in h.rtts {
                                ui.monospace(r.map_or("*".into(), widgets::fmt_ms));
                            }
                            match h.addr {
                                Some(a) => ui.monospace(a.to_string()),
                                None => ui.weak("request timed out"),
                            };
                            ui.label(h.name.as_deref().unwrap_or(&h.note));
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
        (!self.hops.is_empty()).then(|| self.report())
    }

    fn report(&self) -> String {
        let mut s = format!("Traceroute to {}\n", self.host.trim());
        for h in &self.hops {
            let rtts: Vec<String> = h
                .rtts
                .iter()
                .map(|r| r.map_or("*".into(), |v| format!("{v:.0} ms")))
                .collect();
            let addr = h.addr.map_or("request timed out".into(), |a| a.to_string());
            let name = h.name.as_deref().unwrap_or(&h.note);
            s += &format!(
                "{:>3}  {:>7} {:>7} {:>7}  {addr}  {name}\n",
                h.ttl, rtts[0], rtts[1], rtts[2]
            );
        }
        s
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.err = None;
        self.target = None;
        self.reached = None;
        self.hops.clear();
        let host = self.host.trim().to_string();
        if host.is_empty() {
            self.err = Some("enter a host".into());
            return;
        }
        let (max, timeout, names, v6) = (
            self.max_hops,
            self.timeout_ms,
            self.resolve_names,
            self.prefer_v6,
        );

        self.run_started = Some(crate::log::now());
        self.job = Some(Job::spawn(ctx, move |tx| {
            let ip = match net::resolve(&host, v6) {
                Ok(ip) => ip,
                Err(e) => return tx.send(Ev::Error(format!("could not resolve {host}: {e}"))),
            };
            tx.send(Ev::Resolved(ip));
            for ttl in 1..=max {
                let mut hop = Hop {
                    ttl,
                    rtts: [None; PROBES],
                    addr: None,
                    name: None,
                    note: String::new(),
                };
                let mut reached = false;
                for slot in hop.rtts.iter_mut() {
                    if tx.cancelled() {
                        return;
                    }
                    let r = icmp::echo(ip, ttl, timeout, 32);
                    if r.from.is_some() {
                        hop.addr = hop.addr.or(r.from);
                        *slot = Some(r.rtt_ms);
                    }
                    match r.status {
                        EchoStatus::Reply => reached = true,
                        EchoStatus::Unreachable(m) => hop.note = m,
                        _ => {}
                    }
                }
                if names {
                    hop.name = hop.addr.and_then(net::reverse);
                }
                let stop = reached || !hop.note.is_empty();
                tx.send(Ev::Hop(hop));
                if stop {
                    return tx.send(Ev::Done(reached));
                }
            }
            tx.send(Ev::Done(false));
        }));
    }
}
