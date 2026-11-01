use crate::job::{Job, pump};
use crate::log::{self, Table};
use crate::net::ports::{PRESETS, parse_ports, service_name};
use crate::net::{self, TcpState};
use crate::widgets::{self, Action};
use eframe::egui::{self, RichText, Ui};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

enum Ev {
    Resolved(IpAddr, usize),
    Result(u16, TcpState, f64),
    Error(String),
}

pub struct PortTool {
    host: String,
    ports: String,
    timeout_ms: u32,
    threads: usize,
    rate: u32,
    show_closed: bool,
    prefer_v6: bool,
    job: Option<Job<Ev>>,
    run_started: Option<String>,
    started_at: Instant,
    target: Option<IpAddr>,
    total: usize,
    results: Vec<(u16, TcpState, f64)>,
    err: Option<String>,
}

impl Default for PortTool {
    fn default() -> Self {
        Self {
            host: String::new(),
            ports: PRESETS[0].1.into(),
            timeout_ms: 1500,
            threads: widgets::SPEEDS[widgets::DEFAULT_SPEED].1,
            rate: widgets::SPEEDS[widgets::DEFAULT_SPEED].2,
            show_closed: false,
            prefer_v6: false,
            job: None,
            run_started: None,
            started_at: Instant::now(),
            target: None,
            total: 0,
            results: Vec::new(),
            err: None,
        }
    }
}

impl PortTool {
    /// Drain worker messages. Called every frame so results stay current
    /// while another tab is shown.
    pub fn poll(&mut self) -> bool {
        pump(&mut self.job, |ev| match ev {
            Ev::Resolved(ip, n) => (self.target, self.total) = (Some(ip), n),
            // Keep port order without re-sorting the whole list each frame.
            Ev::Result(p, s, ms) => {
                let at = self.results.partition_point(|r| r.0 < p);
                self.results.insert(at, (p, s, ms));
            }
            Ev::Error(e) => self.err = Some(e),
        })
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let running = self.poll();

        let mut enter = false;
        ui.horizontal(|ui| {
            enter |= widgets::field(
                ui,
                "Host",
                &mut self.host,
                widgets::MAIN_W,
                "hostname or IP",
            );
            match widgets::run_button(ui, running, enter) {
                Action::Start => self.start(ui.ctx()),
                Action::Stop => self.job.as_ref().map_or((), Job::stop),
                Action::None => {}
            }
        });
        ui.horizontal(|ui| {
            enter = widgets::field(
                ui,
                "Ports",
                &mut self.ports,
                ui.available_width() - widgets::LEAD_W - 90.0,
                "22,80,443,8000-8100",
            );
            ui.menu_button("Presets ⏷", |ui| {
                for (name, spec) in PRESETS {
                    if ui.button(*name).on_hover_text(*spec).clicked() {
                        self.ports = spec.to_string();
                        ui.close();
                    }
                }
            });
        });
        if enter && !running {
            self.start(ui.ctx());
        }
        ui.horizontal(|ui| {
            widgets::num(ui, "Timeout", &mut self.timeout_ms, 100..=30_000, " ms");
            widgets::speed(ui, "port_speed", &mut self.threads, &mut self.rate);
            ui.checkbox(&mut self.show_closed, "Show closed/filtered");
            ui.checkbox(&mut self.prefer_v6, "Prefer IPv6");
        });
        widgets::error(ui, &self.err);
        ui.separator();

        let open = self
            .results
            .iter()
            .filter(|r| r.1 == TcpState::Open)
            .count();
        let first_err = self.results.iter().find_map(|r| match &r.1 {
            TcpState::Error(e) => Some(e.as_str()),
            _ => None,
        });
        widgets::status_row(
            ui,
            |ui| {
                if let Some(ip) = self.target {
                    ui.label(RichText::new(ip.to_string()).strong());
                    ui.separator();
                }
                if self.total > 0 {
                    let c = if open > 0 {
                        widgets::ok()
                    } else {
                        ui.visuals().text_color()
                    };
                    ui.label(RichText::new(format!("{open} open")).color(c).strong());
                    ui.label(format!("· {} / {} scanned", self.results.len(), self.total));
                }
            },
            |ui| {
                ui.add_enabled_ui(!self.results.is_empty(), |ui| {
                    widgets::save_button(ui, "ports", || self.report());
                    widgets::copy_button(ui, || self.report());
                });
            },
        );
        if let Some(e) = first_err {
            // Errors (no route, bad address, out of sockets) are always shown:
            // hiding them would make a broken scan look like "0 open".
            widgets::error_text(ui, &format!("Some probes failed: {e}"));
        }
        if running && self.total > 0 {
            widgets::progress(ui, self.results.len(), self.total, self.started_at);
        }
        if self.results.is_empty() {
            if !running {
                widgets::empty_state(
                    ui,
                    "Check a host's ports",
                    "Enter a host and press Enter. Ports takes lists and ranges, or pick a preset.",
                    &[],
                );
            }
            return;
        }

        // Virtualized: a full 1-65535 scan with closed ports shown stays fast.
        let rows: Vec<&(u16, TcpState, f64)> = self
            .results
            .iter()
            .filter(|r| self.show_closed || !matches!(r.1, TcpState::Closed | TcpState::Filtered))
            .collect();
        if rows.is_empty() && !running {
            widgets::empty_hint(
                ui,
                "No open ports. Tick \"Show closed/filtered\" to see every result.",
            );
            return;
        }
        let cols = [70.0, 80.0, 120.0, 80.0];
        ui.horizontal(|ui| {
            for (h, w) in ["Port", "State", "Service", "Time"].iter().zip(cols) {
                ui.add_sized([w, 18.0], egui::Label::new(RichText::new(*h).strong()));
            }
        });
        let row_h = 18.0 + ui.spacing().item_spacing.y;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            row_h,
            rows.len(),
            |ui, range| {
                for (i, (port, state, ms)) in range.clone().zip(&rows[range]) {
                    let (txt, c) = state_label(state);
                    let time = if *state == TcpState::Open {
                        widgets::fmt_ms(*ms)
                    } else {
                        String::new()
                    };
                    if i % 2 == 1 {
                        // Painted first so it sits behind the row: matches the striped grids.
                        let top = ui.cursor().top();
                        let rect =
                            egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=top + 18.0);
                        ui.painter()
                            .rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
                    }
                    ui.horizontal(|ui| {
                        let cell = |ui: &mut Ui, w: f32, t: RichText| {
                            ui.add_sized([w, 18.0], egui::Label::new(t));
                        };
                        cell(ui, cols[0], RichText::new(port.to_string()).monospace());
                        cell(ui, cols[1], RichText::new(txt).color(c));
                        cell(ui, cols[2], RichText::new(service_name(*port)));
                        cell(ui, cols[3], RichText::new(time));
                    });
                }
            },
        );
    }

    /// Start time and logged text of a run that has just ended, once.
    pub fn take_finished(&mut self) -> Option<(String, String, Option<Table>)> {
        if self.job.is_some() {
            return None;
        }
        let started = self.run_started.take()?;
        let out = self.output();
        let table = out.as_ref().and_then(|_| self.table());
        Some((started, log::body(out, &self.err), table))
    }

    /// Start time of the run in progress.
    pub fn running_since(&self) -> Option<&str> {
        self.job.as_ref().and(self.run_started.as_deref())
    }

    /// Report text for the Results tab; None until something has run.
    pub fn output(&self) -> Option<String> {
        (!self.results.is_empty()).then(|| self.report())
    }

    fn report(&self) -> String {
        let host = self.target.map(|i| i.to_string()).unwrap_or_default();
        let mut s = format!("Port scan of {} ({host})\n", self.host.trim());
        for (port, state, _) in &self.results {
            if self.show_closed || !matches!(state, TcpState::Closed | TcpState::Filtered) {
                s += &format!(
                    "{port:>5}  {:<9} {}\n",
                    state_label(state).0,
                    service_name(*port)
                );
            }
        }
        s
    }

    /// Every result, closed and filtered included: filter in the spreadsheet.
    fn table(&self) -> Option<Table> {
        let ip = log::cell(self.target);
        let rows = self
            .results
            .iter()
            .map(|(port, state, ms)| {
                let (st, err) = match state {
                    TcpState::Error(e) => ("error", e.as_str()),
                    s => (state_label(s).0, ""),
                };
                vec![
                    self.host.trim().to_string(),
                    ip.clone(),
                    port.to_string(),
                    st.to_string(),
                    service_name(*port).to_string(),
                    if *state == TcpState::Open {
                        log::ms(*ms)
                    } else {
                        String::new()
                    },
                    err.to_string(),
                ]
            })
            .collect();
        Some(Table {
            columns: &["host", "ip", "port", "state", "service", "rtt_ms", "error"],
            rows,
        })
    }

    /// Start a run with the given target (used by the full scan).
    pub fn run(&mut self, ctx: &egui::Context, host: &str) {
        self.host = host.into();
        self.ports = PRESETS[0].1.into();
        self.start(ctx);
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.err = None;
        self.target = None;
        self.results.clear();
        self.total = 0;
        let host = self.host.trim().to_string();
        if host.is_empty() {
            self.err = Some("enter a host".into());
            return;
        }
        let ports = match parse_ports(&self.ports) {
            Ok(p) => p,
            Err(e) => return self.err = Some(e),
        };
        let timeout = Duration::from_millis(self.timeout_ms as u64);
        let (threads, rate, v6) = (self.threads, self.rate, self.prefer_v6);

        self.run_started = Some(log::now());
        self.started_at = Instant::now();
        self.job = Some(Job::spawn(ctx, move |tx| {
            let ip = match net::resolve(&host, v6) {
                Ok(ip) => ip,
                Err(e) => return tx.send(Ev::Error(format!("could not resolve {host}: {e}"))),
            };
            tx.send(Ev::Resolved(ip, ports.len()));
            net::par_for(&ports, threads, rate, tx.cancel_flag(), |&p| {
                let (state, ms) = net::tcp_check(SocketAddr::new(ip, p), timeout);
                tx.send(Ev::Result(p, state, ms));
            });
        }));
    }
}

fn state_label(s: &TcpState) -> (&'static str, egui::Color32) {
    match s {
        TcpState::Open => ("open", widgets::ok()),
        TcpState::Closed => ("closed", widgets::bad()),
        TcpState::Filtered => ("filtered", widgets::warn()),
        TcpState::Error(_) => ("error", widgets::bad()),
    }
}
