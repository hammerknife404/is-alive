use crate::job::{Job, pump};
use crate::log::{self, Table};
use crate::net::icmp::{self, EchoStatus};
use crate::net::{self, TcpState};
use crate::widgets::{self, Action};
use eframe::egui::{self, RichText, Ui};
use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Icmp,
    Tcp,
}

/// Log lines kept for display/copy; stats cover the whole run regardless.
const MAX_LINES: usize = 10_000;
const CHART_POINTS: usize = 120;

#[derive(Default)]
struct Stats {
    sent: usize,
    recv: usize,
    sum: f64,
    min: f64,
    max: f64,
}

impl Stats {
    fn add(&mut self, rtt: Option<f64>) {
        self.sent += 1;
        if let Some(r) = rtt {
            if self.recv == 0 || r < self.min {
                self.min = r;
            }
            self.max = self.max.max(r);
            self.sum += r;
            self.recv += 1;
        }
    }

    fn min_avg_max(&self) -> Option<(f64, f64, f64)> {
        (self.recv > 0).then(|| (self.min, self.sum / self.recv as f64, self.max))
    }
}

enum Ev {
    Resolved(IpAddr),
    /// `row` holds the TABLE columns.
    Reply {
        ok: bool,
        rtt: f64,
        text: String,
        row: Vec<String>,
    },
    Error(String),
}

pub struct PingTool {
    host: String,
    mode: Mode,
    port: u16,
    count: u32,
    interval_ms: u32,
    timeout_ms: u32,
    size: u16,
    prefer_v6: bool,
    job: Option<Job<Ev>>,
    run_started: Option<String>,
    target: Option<IpAddr>,
    lines: VecDeque<(bool, String)>,
    /// Same replies as `lines`, as table rows.
    rows: VecDeque<Vec<String>>,
    samples: VecDeque<Option<f64>>,
    stats: Stats,
    err: Option<String>,
}

impl Default for PingTool {
    fn default() -> Self {
        Self {
            host: String::new(),
            mode: Mode::Icmp,
            port: 443,
            count: 0,
            interval_ms: 1000,
            timeout_ms: 1000,
            size: 32,
            prefer_v6: false,
            job: None,
            run_started: None,
            target: None,
            lines: VecDeque::new(),
            rows: VecDeque::new(),
            samples: VecDeque::new(),
            stats: Stats::default(),
            err: None,
        }
    }
}

impl PingTool {
    /// Drain worker messages. Called every frame so results stay current
    /// while another tab is shown.
    pub fn poll(&mut self) -> bool {
        pump(&mut self.job, |ev| match ev {
            Ev::Resolved(ip) => self.target = Some(ip),
            Ev::Reply { ok, rtt, text, row } => {
                let rtt = ok.then_some(rtt);
                self.stats.add(rtt);
                push_capped(&mut self.samples, rtt, CHART_POINTS);
                push_capped(&mut self.lines, (ok, text), MAX_LINES);
                push_capped(&mut self.rows, row, MAX_LINES);
            }
            Ev::Error(e) => self.err = Some(e),
        })
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let running = self.poll();

        let mut enter = false;
        ui.horizontal(|ui| {
            enter |= widgets::field(ui, "Host", &mut self.host, 220.0, "hostname or IP");
            ui.separator();
            ui.selectable_value(&mut self.mode, Mode::Icmp, "ICMP");
            ui.selectable_value(&mut self.mode, Mode::Tcp, "TCP");
            if self.mode == Mode::Tcp {
                widgets::num(ui, "Port", &mut self.port, 1..=65535, "");
            }
            ui.separator();
            match widgets::run_button(ui, running, enter) {
                Action::Start => self.start(ui.ctx()),
                Action::Stop => self.job.as_ref().map_or((), Job::stop),
                Action::None => {}
            }
        });
        ui.horizontal(|ui| {
            widgets::num(ui, "Count", &mut self.count, 0..=100_000, "");
            ui.weak("(0 = continuous)");
            widgets::num(ui, "Interval", &mut self.interval_ms, 100..=60_000, " ms");
            widgets::num(ui, "Timeout", &mut self.timeout_ms, 100..=30_000, " ms");
            if self.mode == Mode::Icmp {
                widgets::num(ui, "Size", &mut self.size, 0..=65_500, " B");
            }
            ui.checkbox(&mut self.prefer_v6, "Prefer IPv6");
        });
        widgets::error(ui, &self.err);
        ui.separator();

        self.stats_row(ui);
        // No empty graph box before the first reply.
        if !self.samples.is_empty() {
            widgets::latency_chart(ui, self.samples.make_contiguous(), 70.0);
            ui.add_space(4.0);
        }

        if self.lines.is_empty() && !running {
            widgets::empty_hint(
                ui,
                "Enter a host and press Enter. Count 0 pings until stopped.",
            );
            return;
        }
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + ui.spacing().item_spacing.y;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show_rows(ui, row_h, self.lines.len(), |ui, range| {
                for (ok, line) in self.lines.range(range) {
                    let c = if *ok {
                        ui.visuals().text_color()
                    } else {
                        widgets::bad()
                    };
                    // No wrapping: show_rows needs a fixed row height.
                    ui.add(egui::Label::new(RichText::new(line).monospace().color(c)).extend());
                }
            });
    }

    fn stats_row(&self, ui: &mut Ui) {
        let (sent, recv, mm) = (self.stats.sent, self.stats.recv, self.stats.min_avg_max());
        ui.horizontal(|ui| {
            if let Some(ip) = self.target {
                ui.label(RichText::new(ip.to_string()).strong());
                ui.separator();
            }
            ui.label(format!("Sent {sent}  Received {recv}"));
            if sent > 0 {
                let loss = 100.0 * (sent - recv) as f64 / sent as f64;
                let c = if loss == 0.0 {
                    widgets::ok()
                } else if loss < 100.0 {
                    widgets::warn()
                } else {
                    widgets::bad()
                };
                ui.label(RichText::new(format!("Loss {loss:.0}%")).color(c));
            }
            if let Some((min, avg, max)) = mm {
                ui.separator();
                ui.label(format!(
                    "Min {}  Avg {}  Max {}",
                    widgets::fmt_ms(min),
                    widgets::fmt_ms(avg),
                    widgets::fmt_ms(max)
                ));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(sent > 0, |ui| {
                    widgets::save_button(ui, "ping", || self.report());
                    widgets::copy_button(ui, || self.report());
                });
            });
        });
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
        (self.stats.sent > 0).then(|| self.report())
    }

    fn report(&self) -> String {
        let mut s: String = self.lines.iter().map(|(_, l)| format!("{l}\n")).collect();
        let (sent, recv, mm) = (self.stats.sent, self.stats.recv, self.stats.min_avg_max());
        if sent > self.lines.len() {
            s = format!("(showing last {} of {sent} replies)\n", self.lines.len()) + &s;
        }
        s += &format!("\nSent {sent}, Received {recv}");
        if let Some((min, avg, max)) = mm {
            s += &format!(
                ", Min {}, Avg {}, Max {}",
                widgets::fmt_ms(min),
                widgets::fmt_ms(avg),
                widgets::fmt_ms(max)
            );
        }
        s
    }

    fn table(&self) -> Option<Table> {
        Some(Table {
            columns: &[
                "target", "mode", "port", "seq", "from", "status", "rtt_ms", "ttl", "bytes",
            ],
            rows: self.rows.iter().cloned().collect(),
        })
    }

    /// Start a run with the given target (used by the full scan).
    /// `count` ICMP pings to `host` (full scan, Map), leaving the tab's own
    /// mode and count as the user set them.
    pub fn run(&mut self, ctx: &egui::Context, host: &str, count: u32) {
        self.host = host.into();
        self.launch(ctx, Mode::Icmp, count);
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.launch(ctx, self.mode, self.count);
    }

    fn launch(&mut self, ctx: &egui::Context, mode: Mode, count: u32) {
        self.err = None;
        self.target = None;
        self.lines.clear();
        self.rows.clear();
        self.samples.clear();
        self.stats = Stats::default();
        let host = self.host.trim().to_string();
        if host.is_empty() {
            self.err = Some("enter a host".into());
            return;
        }
        let (port, size, v6) = (self.port, self.size, self.prefer_v6);
        let interval = Duration::from_millis(self.interval_ms as u64);
        let timeout = self.timeout_ms;

        self.run_started = Some(log::now());
        self.job = Some(Job::spawn(ctx, move |tx| {
            let ip = match net::resolve(&host, v6) {
                Ok(ip) => ip,
                Err(e) => return tx.send(Ev::Error(format!("could not resolve {host}: {e}"))),
            };
            tx.send(Ev::Resolved(ip));
            let mut seq = 0u32;
            while !tx.cancelled() && (count == 0 || seq < count) {
                seq += 1;
                let t0 = Instant::now();
                let ev = match mode {
                    Mode::Icmp => icmp_line(ip, seq, size, timeout),
                    Mode::Tcp => tcp_line(SocketAddr::new(ip, port), seq, timeout),
                };
                tx.send(ev);
                if count == 0 || seq < count {
                    tx.sleep(interval.saturating_sub(t0.elapsed()));
                }
            }
        }));
    }
}

fn icmp_line(ip: IpAddr, seq: u32, size: u16, timeout: u32) -> Ev {
    let r = icmp::echo(ip, 128, timeout, size);
    let from = r.from.map_or_else(|| ip.to_string(), |a| a.to_string());
    let (ok, text) = match &r.status {
        EchoStatus::Reply => {
            let ttl = r.ttl.map(|t| format!(" ttl={t}")).unwrap_or_default();
            (
                true,
                format!(
                    "#{seq:<5} reply from {from}: bytes={size} time={}{ttl}",
                    widgets::fmt_ms(r.rtt_ms)
                ),
            )
        }
        EchoStatus::Timeout => (false, format!("#{seq:<5} request timed out")),
        EchoStatus::TtlExpired => (false, format!("#{seq:<5} {from}: TTL expired in transit")),
        EchoStatus::Unreachable(m) => (false, format!("#{seq:<5} {from}: {m}")),
        EchoStatus::Error(m) => (false, format!("#{seq:<5} error: {m}")),
    };
    let status = match &r.status {
        EchoStatus::Reply => "reply".to_string(),
        EchoStatus::Timeout => "timeout".into(),
        EchoStatus::TtlExpired => "ttl expired".into(),
        EchoStatus::Unreachable(m) => format!("unreachable: {m}"),
        EchoStatus::Error(m) => format!("error: {m}"),
    };
    let row = vec![
        ip.to_string(),
        "ICMP".into(),
        String::new(),
        seq.to_string(),
        r.from.map(|a| a.to_string()).unwrap_or_default(),
        status,
        if ok { log::ms(r.rtt_ms) } else { String::new() },
        log::cell(r.ttl),
        size.to_string(),
    ];
    Ev::Reply {
        ok,
        rtt: r.rtt_ms,
        text,
        row,
    }
}

fn tcp_line(addr: SocketAddr, seq: u32, timeout: u32) -> Ev {
    let (state, ms) = net::tcp_probe(addr, Duration::from_millis(timeout as u64));
    let (ok, text) = match &state {
        TcpState::Open => (
            true,
            format!("#{seq:<5} connected to {addr} time={}", widgets::fmt_ms(ms)),
        ),
        TcpState::Closed => (false, format!("#{seq:<5} {addr} refused (port closed)")),
        TcpState::Filtered => (false, format!("#{seq:<5} {addr} timed out (filtered?)")),
        TcpState::Error(e) => (false, format!("#{seq:<5} {addr} error: {e}")),
    };
    let status = match &state {
        TcpState::Open => "open".to_string(),
        TcpState::Closed => "closed".into(),
        TcpState::Filtered => "filtered".into(),
        TcpState::Error(e) => format!("error: {e}"),
    };
    let row = vec![
        addr.ip().to_string(),
        "TCP".into(),
        addr.port().to_string(),
        seq.to_string(),
        String::new(),
        status,
        if ok { log::ms(ms) } else { String::new() },
        String::new(),
        String::new(),
    ];
    Ev::Reply {
        ok,
        rtt: ms,
        text,
        row,
    }
}

fn push_capped<T>(q: &mut VecDeque<T>, v: T, cap: usize) {
    if q.len() == cap {
        q.pop_front();
    }
    q.push_back(v);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_scan_ping_leaves_the_tab_settings() {
        let ctx = egui::Context::default();
        let mut t = PingTool {
            mode: Mode::Tcp,
            count: 0,
            ..Default::default()
        };
        t.run(&ctx, "127.0.0.1", 1);
        assert!((t.mode == Mode::Tcp, t.count) == (true, 0));
        assert_eq!(t.host, "127.0.0.1");
        if let Some(j) = &t.job {
            j.stop();
        }
    }

    #[test]
    fn stats_track_whole_run() {
        let mut s = Stats::default();
        for r in [None, Some(5.0), Some(1.0), None, Some(3.0)] {
            s.add(r);
        }
        assert_eq!((s.sent, s.recv), (5, 3));
        assert_eq!(s.min_avg_max(), Some((1.0, 3.0, 5.0)));
        assert_eq!(Stats::default().min_avg_max(), None);
    }

    #[test]
    fn rows_match_columns() {
        let cols = PingTool::default().table().unwrap().columns.len();
        let addr = SocketAddr::from(([127, 0, 0, 1], 1));
        let Ev::Reply { row, .. } = tcp_line(addr, 1, 200) else {
            panic!("expected a reply");
        };
        assert_eq!(row.len(), cols);
        let Ev::Reply { row, .. } = icmp_line(addr.ip(), 1, 32, 200) else {
            panic!("expected a reply");
        };
        assert_eq!(row.len(), cols);
    }

    #[test]
    fn capped_queue_drops_oldest() {
        let mut q = VecDeque::new();
        for i in 0..5 {
            push_capped(&mut q, i, 3);
        }
        assert_eq!(q, [2, 3, 4]);
    }
}
