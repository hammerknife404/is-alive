use crate::job::{Job, pump};
use crate::net::icmp::{self, EchoStatus};
use crate::net::{self, TcpState};
use crate::widgets::{self, Action, BAD, OK, WARN};
use eframe::egui::{self, RichText, Ui};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Icmp,
    Tcp,
}

enum Ev {
    Resolved(IpAddr),
    Reply { ok: bool, rtt: f64, text: String },
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
    target: Option<IpAddr>,
    lines: Vec<(bool, String)>,
    samples: Vec<Option<f64>>,
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
            target: None,
            lines: Vec::new(),
            samples: Vec::new(),
            err: None,
        }
    }
}

impl PingTool {
    pub fn ui(&mut self, ui: &mut Ui) {
        let running = pump(&mut self.job, |ev| match ev {
            Ev::Resolved(ip) => self.target = Some(ip),
            Ev::Reply { ok, rtt, text } => {
                self.samples.push(ok.then_some(rtt));
                self.lines.push((ok, text));
            }
            Ev::Error(e) => self.err = Some(e),
        });

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
        let tail = &self.samples[self.samples.len().saturating_sub(120)..];
        widgets::latency_chart(ui, tail, 70.0);
        ui.add_space(4.0);

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (ok, line) in &self.lines {
                    let c = if *ok { ui.visuals().text_color() } else { BAD };
                    ui.label(RichText::new(line).monospace().color(c));
                }
            });
    }

    fn stats(&self) -> (usize, usize, Option<(f64, f64, f64)>) {
        let sent = self.samples.len();
        let rtts: Vec<f64> = self.samples.iter().flatten().copied().collect();
        let mm = (!rtts.is_empty()).then(|| {
            let min = rtts.iter().copied().fold(f64::MAX, f64::min);
            let max = rtts.iter().copied().fold(0.0, f64::max);
            (min, rtts.iter().sum::<f64>() / rtts.len() as f64, max)
        });
        (sent, rtts.len(), mm)
    }

    fn stats_row(&self, ui: &mut Ui) {
        let (sent, recv, mm) = self.stats();
        ui.horizontal(|ui| {
            if let Some(ip) = self.target {
                ui.label(RichText::new(ip.to_string()).strong());
                ui.separator();
            }
            ui.label(format!("Sent {sent}  Received {recv}"));
            if sent > 0 {
                let loss = 100.0 * (sent - recv) as f64 / sent as f64;
                let c = if loss == 0.0 {
                    OK
                } else if loss < 100.0 {
                    WARN
                } else {
                    BAD
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
                widgets::copy_button(ui, || self.report());
            });
        });
    }

    fn report(&self) -> String {
        let mut s: String = self.lines.iter().map(|(_, l)| format!("{l}\n")).collect();
        let (sent, recv, mm) = self.stats();
        s += &format!("\nSent {sent}, Received {recv}");
        if let Some((min, avg, max)) = mm {
            s += &format!(", Min {min:.1} ms, Avg {avg:.1} ms, Max {max:.1} ms");
        }
        s
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.err = None;
        self.target = None;
        self.lines.clear();
        self.samples.clear();
        let host = self.host.trim().to_string();
        if host.is_empty() {
            self.err = Some("enter a host".into());
            return;
        }
        let (mode, port, count, size, v6) =
            (self.mode, self.port, self.count, self.size, self.prefer_v6);
        let interval = Duration::from_millis(self.interval_ms as u64);
        let timeout = self.timeout_ms;

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
    Ev::Reply {
        ok,
        rtt: r.rtt_ms,
        text,
    }
}

fn tcp_line(addr: SocketAddr, seq: u32, timeout: u32) -> Ev {
    let (state, ms) = net::tcp_probe(addr, Duration::from_millis(timeout as u64));
    let (ok, text) = match state {
        TcpState::Open => (
            true,
            format!("#{seq:<5} connected to {addr} time={}", widgets::fmt_ms(ms)),
        ),
        TcpState::Closed => (false, format!("#{seq:<5} {addr} refused (port closed)")),
        TcpState::Filtered => (false, format!("#{seq:<5} {addr} timed out (filtered?)")),
        TcpState::Error(e) => (false, format!("#{seq:<5} {addr} error: {e}")),
    };
    Ev::Reply { ok, rtt: ms, text }
}
