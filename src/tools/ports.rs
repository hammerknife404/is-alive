use crate::input::Field;
use crate::job::{Job, pump};
use crate::log::{self, Table};
use crate::net::ports::{self, PRESETS, parse_ports, service_name};
use crate::net::tls::{self, Level};
use crate::net::{self, TcpState};
use crate::widgets::{self, Action};
use eframe::egui::{self, RichText, Ui};
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

/// Results (CSV export, the saved log): one row per result, these columns.
pub const COLUMNS: &[&str] = &[
    "host",
    "ip",
    "port",
    "state",
    "service",
    "rtt_ms",
    "error",
    "tls_status",
    "tls_issued_to",
    "tls_issued_by",
    "tls_expires",
];

enum Ev {
    Resolved(IpAddr, usize),
    Result(u16, TcpState, f64),
    Cert(u16, Result<tls::Info, String>),
    Error(String),
}

/// How long a certificate read waits for the server.
const TLS_WAIT: Duration = Duration::from_secs(5);

pub struct PortTool {
    host: String,
    ports: String,
    timeout_ms: u32,
    threads: usize,
    rate: u32,
    show_closed: bool,
    prefer_v6: bool,
    /// Read the TLS certificate on open ports that speak TLS.
    certs: bool,
    job: Option<Job<Ev>>,
    run_started: Option<String>,
    started_at: Instant,
    target: Option<IpAddr>,
    total: usize,
    results: Vec<(u16, TcpState, f64)>,
    /// What each open TLS port presented, or why it couldn't be read.
    tls: BTreeMap<u16, Result<tls::Info, String>>,
    /// The port whose certificate window is open.
    cert_view: Option<u16>,
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
            certs: true,
            job: None,
            run_started: None,
            started_at: Instant::now(),
            target: None,
            total: 0,
            results: Vec::new(),
            tls: BTreeMap::new(),
            cert_view: None,
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
            Ev::Cert(p, r) => {
                self.tls.insert(p, r);
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
                Field::Host,
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
                Field::Ports,
                &mut self.ports,
                ui.available_width() - widgets::LEAD_W - 90.0,
                "22,80,443,8000-8100",
            );
            ui.menu_button("Presets ▾", |ui| {
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
        let mut summary = vec![
            widgets::speed_name(self.threads, self.rate).to_string(),
            format!("timeout {} ms", self.timeout_ms),
        ];
        if self.show_closed {
            summary.push("closed shown".into());
        }
        if self.certs {
            summary.push("certificates".into());
        }
        if self.prefer_v6 {
            summary.push("IPv6 first".into());
        }
        widgets::options(ui, "ports", &summary, |ui| {
            ui.horizontal_wrapped(|ui| {
                widgets::num(ui, "Timeout", &mut self.timeout_ms, 100..=30_000, " ms");
                widgets::speed(ui, "port_speed", &mut self.threads, &mut self.rate);
                ui.checkbox(&mut self.show_closed, "Show closed/filtered");
                ui.checkbox(&mut self.prefer_v6, "Prefer IPv6");
                ui.checkbox(&mut self.certs, "Certificates").on_hover_text(
                    "Read the TLS certificate on each open port that speaks TLS (HTTPS, SSL-VPN, \
                     LDAPS...): who it's for, who issued it, when it expires, and whether this \
                     computer trusts it",
                );
            });
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

        self.cert_window(ui.ctx());
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
        let any_tls = !self.tls.is_empty();
        ui.horizontal(|ui| {
            for (h, w) in ["Port", "State", "Service", "Time"].iter().zip(cols) {
                ui.add_sized([w, 18.0], egui::Label::new(RichText::new(*h).strong()));
            }
            if any_tls {
                ui.label(RichText::new("Certificate").strong());
            }
        });
        let now = chrono::Utc::now();
        let mut view = None;
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
                        if let Some(r) = self.tls.get(port) {
                            view = view.or(cert_cell(ui, r, now).then_some(*port));
                        }
                    });
                }
            },
        );
        if view.is_some() {
            self.cert_view = view;
        }
    }

    /// The certificate window, for the port picked in the table.
    fn cert_window(&mut self, ctx: &egui::Context) {
        let Some(port) = self.cert_view else { return };
        let Some(Ok(info)) = self.tls.get(&port) else {
            self.cert_view = None;
            return;
        };
        let now = chrono::Utc::now();
        let (level, _) = info.verdict(now);
        let rows = info.details(now);
        let mut open = true;
        egui::Window::new(format!("Certificate · {}:{port}", info.name))
            .id(egui::Id::new("ports_cert"))
            .open(&mut open)
            .collapsible(false)
            .default_width(480.0)
            // Over the middle of the window, clear of the row clicked.
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                egui::Grid::new("ports_cert_grid")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        for (i, (k, v)) in rows.iter().enumerate() {
                            ui.weak(*k);
                            let t = RichText::new(v);
                            let t = if i == 0 {
                                t.color(level_colour(level)).strong()
                            } else {
                                t
                            };
                            ui.add(egui::Label::new(t).wrap().selectable(true));
                            ui.end_row();
                        }
                    });
                ui.add_space(6.0);
                if ui.button("Copy").clicked() {
                    widgets::copy(ui.ctx(), cert_text(info, port, now));
                    widgets::toast("Certificate details copied", true);
                }
            });
        if !open {
            self.cert_view = None;
        }
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
        let now = chrono::Utc::now();
        for (port, state, _) in &self.results {
            if self.show_closed || !matches!(state, TcpState::Closed | TcpState::Filtered) {
                s += &format!(
                    "{port:>5}  {:<9} {}\n",
                    state_label(state).0,
                    service_name(*port)
                );
                match self.tls.get(port) {
                    Some(Ok(info)) => {
                        for (k, v) in info.details(now) {
                            s += &format!("         {k}: {v}\n");
                        }
                    }
                    Some(Err(e)) => s += &format!("         Certificate: not read ({e})\n"),
                    None => {}
                }
            }
        }
        s
    }

    /// Every result, closed and filtered included: filter in the spreadsheet.
    fn table(&self) -> Option<Table> {
        let ip = log::cell(self.target);
        let now = chrono::Utc::now();
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
                .into_iter()
                .chain(tls_cells(self.tls.get(port), now))
                .collect()
            })
            .collect();
        Some(Table {
            columns: COLUMNS,
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

    /// Why the last run stopped short, if it did.
    pub fn error(&self) -> Option<&str> {
        self.err.as_deref()
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.err = None;
        self.target = None;
        self.results.clear();
        self.tls.clear();
        self.cert_view = None;
        self.total = 0;
        let host = match crate::input::host(&self.host) {
            Ok(t) => {
                // A port typed with the host (nas:5001, a link) is the one checked.
                if let Some(p) = t.port {
                    self.ports = p.to_string();
                }
                t.host
            }
            Err(e) => return self.err = Some(e),
        };
        let ports = match parse_ports(&self.ports) {
            Ok(p) => p,
            Err(e) => return self.err = Some(e),
        };
        let timeout = Duration::from_millis(self.timeout_ms as u64);
        let (threads, rate, v6, certs) = (self.threads, self.rate, self.prefer_v6, self.certs);

        self.run_started = Some(log::now());
        self.started_at = Instant::now();
        self.job = Some(Job::spawn(ctx, move |tx| {
            let ip = match net::resolve(&host, v6) {
                Ok(ip) => ip,
                Err(e) => return tx.send(Ev::Error(format!("could not resolve {host}: {e}"))),
            };
            tx.send(Ev::Resolved(ip, ports.len()));
            // The name as typed is what the certificate must be valid for.
            let name = host.trim_matches(['[', ']']).to_string();
            net::par_for(&ports, threads, rate, tx.cancel_flag(), |&p| {
                let addr = SocketAddr::new(ip, p);
                let (state, ms) = net::tcp_check(addr, timeout);
                let open = state == TcpState::Open;
                tx.send(Ev::Result(p, state, ms));
                let want = ports::tls(p);
                if !certs || !open || want == ports::Tls::No {
                    return;
                }
                match tls::probe(&name, addr, TLS_WAIT) {
                    Ok(info) => tx.send(Ev::Cert(p, Ok(info))),
                    // An unknown port that isn't TLS: nothing to say.
                    Err(_) if want == ports::Tls::Maybe => {}
                    Err(e) => tx.send(Ev::Cert(p, Err(e.to_string()))),
                }
            });
        }));
    }
}

fn level_colour(l: Level) -> egui::Color32 {
    match l {
        Level::Good => widgets::ok(),
        Level::Warn => widgets::warn(),
        Level::Bad => widgets::bad(),
    }
}

/// A port's Certificate cell: the verdict and whom it's for; or why it
/// couldn't be read. True when it was clicked.
fn cert_cell(
    ui: &mut Ui,
    r: &Result<tls::Info, String>,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    match r {
        Ok(info) => {
            let (level, line) = info.summary(now);
            let icon = match level {
                Level::Good => "🔒",
                Level::Warn => "⚠",
                Level::Bad => "✖",
            };
            ui.add(
                egui::Label::new(
                    RichText::new(format!("{icon} {line}")).color(level_colour(level)),
                )
                .truncate()
                .sense(egui::Sense::click()),
            )
            .on_hover_text("Click for the certificate's details")
            .clicked()
        }
        Err(e) => {
            ui.add(egui::Label::new(RichText::new("No TLS reply").weak()).truncate())
                .on_hover_text(e);
            false
        }
    }
}

/// The certificate window's text, for Copy.
fn cert_text(info: &tls::Info, port: u16, now: chrono::DateTime<chrono::Utc>) -> String {
    let mut s = format!("Certificate of {}:{port}\n", info.name);
    for (k, v) in info.details(now) {
        s += &format!("{k}: {v}\n");
    }
    s
}

/// The CSV's certificate columns for one port.
fn tls_cells(
    r: Option<&Result<tls::Info, String>>,
    now: chrono::DateTime<chrono::Utc>,
) -> [String; 4] {
    match r {
        Some(Ok(info)) => {
            let leaf = info.leaf();
            [
                info.verdict(now).1.join("; "),
                leaf.map_or(String::new(), |c| c.subject.clone()),
                leaf.map_or(String::new(), |c| c.issuer.clone()),
                leaf.and_then(|c| c.not_after)
                    .map_or(String::new(), |t| t.format("%Y-%m-%d").to_string()),
            ]
        }
        Some(Err(e)) => [
            format!("not read: {e}"),
            String::new(),
            String::new(),
            String::new(),
        ],
        None => Default::default(),
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
