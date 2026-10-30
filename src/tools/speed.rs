use crate::job::{Job, Tx, pump};
use crate::log::{self, Table};
use crate::net::speed::{self, Dir, Latency, Meta, Transfer};
use crate::net::{self, TcpState, icmp};
use crate::widgets::{self, Action};
use eframe::egui::{self, Color32, RichText, Ui};
use std::net::{IpAddr, SocketAddr};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Probes at idle, and the spacing of every probe: 10 a second, enough
/// samples for jitter without looking like a flood.
const IDLE_PROBES: usize = 40;
const PROBE_EVERY: Duration = Duration::from_millis(100);
/// ICMP probes with no reply at all before switching to TCP.
const ICMP_TRIES: usize = 4;
/// Under load a reply can sit in a bloated queue for a long time: count
/// it as latency, not loss.
const IDLE_TIMEOUT_MS: u32 = 1000;
const LOADED_TIMEOUT_MS: u32 = 2000;
const BLOCKED_HINT: &str =
    "Web filters (SonicWall CFS, DNS filtering) often block speed test sites.";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Idle,
    Down,
    Up,
}

impl Phase {
    const ALL: [Phase; 3] = [Phase::Idle, Phase::Down, Phase::Up];

    fn name(self) -> &'static str {
        match self {
            Phase::Idle => "Idle",
            Phase::Down => "Download",
            Phase::Up => "Upload",
        }
    }

    fn ing(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Down => "downloading",
            Phase::Up => "uploading",
        }
    }

    fn i(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Method {
    Icmp,
    Tcp,
}

impl Method {
    fn name(self) -> &'static str {
        match self {
            Method::Icmp => "ICMP",
            Method::Tcp => "TCP 443",
        }
    }
}

#[derive(Clone, Copy)]
struct Throughput {
    mbps: f64,
    bytes: u64,
}

enum Ev {
    Resolved(IpAddr),
    Meta(Meta),
    /// ICMP got nothing back: latency is by TCP connect from here on.
    Method(Method),
    Phase(Phase),
    Probe(Phase, Option<f64>),
    /// Mbps over the last second, for the live chart and figure; from the
    /// end of the warm-up, so the chart shows what the result measures.
    Rate(Phase, f64),
    Done(Phase, Throughput),
    /// Nothing got through: its probes weren't under load, so they're dropped.
    Failed(Phase, String),
    Error(String),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Level {
    Ok,
    Warn,
    Bad,
}

impl Level {
    fn color(self) -> Color32 {
        match self {
            Level::Ok => widgets::ok(),
            Level::Warn => widgets::warn(),
            Level::Bad => widgets::bad(),
        }
    }

    /// Within the target, within twice it, or beyond.
    fn of(v: f64, target: f64) -> Self {
        if v <= target {
            Level::Ok
        } else if v <= 2.0 * target {
            Level::Warn
        } else {
            Level::Bad
        }
    }

    fn of_mos(m: f64) -> Self {
        if m >= 4.03 {
            Level::Ok
        } else if m >= 3.6 {
            Level::Warn
        } else {
            Level::Bad
        }
    }

    fn of_grade(g: &str) -> Self {
        match g {
            "A+" | "A" => Level::Ok,
            "B" | "C" => Level::Warn,
            _ => Level::Bad,
        }
    }
}

pub struct SpeedTool {
    secs: u32,
    streams: usize,
    down: bool,
    up: bool,
    job: Option<Job<Ev>>,
    run_started: Option<String>,
    /// Duration and streams of the run on screen.
    ran: (u32, usize),
    ip: Option<IpAddr>,
    meta: Option<Meta>,
    method: Method,
    phase: Option<(Phase, Instant)>,
    /// Per phase, indexed by `Phase::i`.
    probes: [Vec<Option<f64>>; 3],
    rates: [Vec<f64>; 3],
    results: [Option<Throughput>; 3],
    warns: Vec<String>,
    err: Option<String>,
}

impl Default for SpeedTool {
    fn default() -> Self {
        Self {
            secs: 10,
            streams: 4,
            down: true,
            up: true,
            job: None,
            run_started: None,
            ran: (10, 4),
            ip: None,
            meta: None,
            method: Method::Icmp,
            phase: None,
            probes: Default::default(),
            rates: Default::default(),
            results: [None; 3],
            warns: Vec::new(),
            err: None,
        }
    }
}

impl SpeedTool {
    /// Drain worker messages. Called every frame so results stay current
    /// while another tab is shown.
    pub fn poll(&mut self) -> bool {
        let running = pump(&mut self.job, |ev| match ev {
            Ev::Resolved(ip) => self.ip = Some(ip),
            Ev::Meta(m) => self.meta = Some(m),
            Ev::Method(m) => {
                self.method = m;
                self.probes[Phase::Idle.i()].clear();
            }
            Ev::Phase(p) => self.phase = Some((p, Instant::now())),
            Ev::Probe(p, s) => self.probes[p.i()].push(s),
            Ev::Rate(p, r) => self.rates[p.i()].push(r),
            Ev::Done(p, t) => self.results[p.i()] = Some(t),
            Ev::Failed(p, w) => {
                self.probes[p.i()].clear();
                self.rates[p.i()].clear();
                self.warns.push(w);
            }
            Ev::Error(e) => self.err = Some(e),
        });
        if !running {
            self.phase = None;
        }
        running
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let running = self.poll();

        ui.horizontal(|ui| {
            ui.label("Server");
            ui.label(RichText::new(speed::HOST).strong()).on_hover_text(
                "Cloudflare's speed test service. It's anycast: the nearest Cloudflare data \
centre answers, usually inside your ISP's region.",
            );
            ui.separator();
            ui.add_enabled_ui(!running, |ui| {
                ui.label("Duration");
                ui.add(
                    egui::DragValue::new(&mut self.secs)
                        .range(5..=60)
                        .suffix(" s"),
                )
                .on_hover_text("Per direction. Latency at idle is measured first, about 4 s.");
                ui.label("Streams");
                ui.add(egui::DragValue::new(&mut self.streams).range(1..=16))
                    .on_hover_text(
                        "Parallel connections per direction. 4 fills most lines; raise it for \
multi-gigabit or long-distance links.",
                    );
                ui.checkbox(&mut self.down, "Download");
                ui.checkbox(&mut self.up, "Upload");
            });
            ui.separator();
            match widgets::run_button(ui, running, false) {
                Action::Start => self.start(ui.ctx()),
                Action::Stop => self.job.as_ref().map_or((), Job::stop),
                Action::None => {}
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(self.has_results(), |ui| {
                    widgets::save_button(ui, "speed", || self.report());
                    widgets::copy_button(ui, || self.report());
                });
            });
        });
        let place = self.place();
        if !place.is_empty() {
            ui.weak(place.join("  ·  "));
        }
        for w in &self.warns {
            widgets::warning(ui, w);
        }
        widgets::error(ui, &self.err);
        ui.separator();

        if !self.has_results() && !running {
            widgets::empty_hint(
                ui,
                "Press Start: latency and jitter at idle, then download and upload with latency \
probed under load (bufferbloat), and a VoIP call-quality estimate.\nIt saturates the line while it \
runs, and a gigabit line moves over 1 GB each way.",
            );
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                self.tiles(ui);
                if running {
                    self.progress(ui);
                }
                if self.rates.iter().any(|r| !r.is_empty()) {
                    ui.add_space(4.0);
                    rate_chart(ui, &self.rates, 70.0);
                }
                let all: Vec<Option<f64>> = self.probes.concat();
                if !all.is_empty() {
                    ui.add_space(4.0);
                    widgets::latency_chart(ui, &all, 60.0);
                }
                ui.add_space(6.0);
                self.latency_grid(ui);
                ui.add_space(6.0);
                for (level, text) in self.findings() {
                    ui.label(RichText::new(text).color(level.color()));
                }
                ui.add_space(4.0);
                ui.weak(targets_note());
            });
    }

    /// Cloudflare IAD (Ashburn), ISP …, latency by ICMP to …
    fn place(&self) -> Vec<String> {
        let mut parts = Vec::new();
        let Some(ip) = self.ip else { return parts };
        if let Some(m) = &self.meta {
            if let Some(c) = &m.colo {
                parts.push(match &m.city {
                    Some(city) => format!("Cloudflare {c} ({city})"),
                    None => format!("Cloudflare {c}"),
                });
            }
            if let Some(isp) = &m.isp {
                parts.push(match m.asn {
                    Some(n) => format!("ISP {isp} (AS{n})"),
                    None => format!("ISP {isp}"),
                });
            }
        }
        parts.push(format!("latency by {} to {ip}", self.method.name()));
        parts
    }

    fn latency(&self, p: Phase) -> Option<Latency> {
        Latency::of(&self.probes[p.i()])
    }

    /// The loaded phase with the most added latency, and how much.
    fn worst_loaded(&self) -> Option<(Phase, Latency, f64)> {
        let idle = self.latency(Phase::Idle)?;
        [Phase::Down, Phase::Up]
            .into_iter()
            .filter_map(|p| self.latency(p).map(|l| (p, l, l.avg - idle.avg)))
            .max_by(|a, b| a.2.total_cmp(&b.2))
    }

    fn tiles(&self, ui: &mut Ui) {
        let idle = self.latency(Phase::Idle);
        let worst = self.worst_loaded();
        // The loaded phase with the worst value of each figure.
        let loaded = |f: fn(&Latency) -> f64| {
            [Phase::Down, Phase::Up]
                .into_iter()
                .filter_map(|p| self.latency(p).as_ref().map(f))
                .reduce(f64::max)
        };
        ui.horizontal_wrapped(|ui| {
            for p in [Phase::Down, Phase::Up] {
                let (value, sub) = match self.results[p.i()] {
                    Some(t) => (speed::fmt_rate(t.mbps), speed::fmt_bytes(t.bytes)),
                    None if self.phase.is_some_and(|(q, _)| q == p) => {
                        match self.rates[p.i()].last() {
                            Some(&now) => (speed::fmt_rate(now), "measuring…".into()),
                            None => ("—".into(), "warming up…".into()),
                        }
                    }
                    None => ("—".into(), String::new()),
                };
                tile(ui, p.name(), value, sub, None);
            }
            let fig = |v: Option<f64>, fmt: fn(f64) -> String| v.map_or_else(|| "—".into(), fmt);
            let sub = |v: Option<f64>, fmt: fn(f64) -> String| {
                v.map(|v| format!("loaded {}", fmt(v))).unwrap_or_default()
            };
            let pct = |v: f64| format!("{v:.1}%");
            let lat = idle.map(|l| l.avg);
            tile(
                ui,
                "Latency",
                fig(lat, widgets::fmt_ms),
                sub(loaded(|l| l.avg), widgets::fmt_ms),
                lat.map(|v| Level::of(v, speed::VOIP_LATENCY_MS)),
            );
            let jit = idle.map(|l| l.jitter);
            tile(
                ui,
                "Jitter",
                fig(jit, widgets::fmt_ms),
                sub(loaded(|l| l.jitter), widgets::fmt_ms),
                jit.map(|v| Level::of(v, speed::VOIP_JITTER_MS)),
            );
            let loss = idle.map(|l| l.loss_pct());
            tile(
                ui,
                "Loss",
                fig(loss, pct),
                sub(loaded(Latency::loss_pct), pct),
                loss.map(|v| Level::of(v, speed::VOIP_LOSS_PCT)),
            );
            let mos = idle.map(|l| l.mos());
            tile(
                ui,
                "VoIP MOS",
                fig(mos, |m| format!("{m:.2}")),
                mos.map(|m| match worst {
                    Some((_, l, _)) => format!("{}, loaded {:.2}", speed::mos_label(m), l.mos()),
                    None => speed::mos_label(m).into(),
                })
                .unwrap_or_default(),
                mos.map(Level::of_mos),
            );
            if let Some((_, _, added)) = worst {
                let g = speed::bloat_grade(added);
                tile(
                    ui,
                    "Bufferbloat",
                    g.into(),
                    format!("+{} under load", widgets::fmt_ms(added.max(0.0))),
                    Some(Level::of_grade(g)),
                );
            }
        });
    }

    fn progress(&self, ui: &mut Ui) {
        let Some((p, since)) = self.phase else { return };
        let (frac, text) = match p {
            Phase::Idle => {
                let n = self.probes[p.i()].len();
                (
                    n as f32 / IDLE_PROBES as f32,
                    format!("Latency at idle  {n} / {IDLE_PROBES}"),
                )
            }
            _ => {
                let t = since.elapsed().as_secs_f32();
                let total = self.ran.0 as f32;
                (
                    t / total,
                    format!("{}  {:.0} / {total:.0} s", p.name(), t.min(total)),
                )
            }
        };
        ui.add_space(4.0);
        ui.add(egui::ProgressBar::new(frac.min(1.0)).text(text));
    }

    fn latency_grid(&self, ui: &mut Ui) {
        let idle = self.latency(Phase::Idle);
        egui::Grid::new("speed_latency")
            .num_columns(8)
            .striped(true)
            .spacing([18.0, 4.0])
            .show(ui, |ui| {
                for h in [
                    self.method.name(),
                    "Avg",
                    "Min",
                    "Max",
                    "Jitter",
                    "Loss",
                    "Added",
                    "MOS",
                ] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for p in Phase::ALL {
                    let Some(l) = self.latency(p) else { continue };
                    let ms = |v: f64| RichText::new(widgets::fmt_ms(v)).monospace();
                    ui.label(p.name());
                    ui.label(ms(l.avg).color(Level::of(l.avg, speed::VOIP_LATENCY_MS).color()));
                    ui.label(ms(l.min));
                    ui.label(ms(l.max));
                    ui.label(
                        ms(l.jitter).color(Level::of(l.jitter, speed::VOIP_JITTER_MS).color()),
                    );
                    ui.label(
                        RichText::new(format!("{:.1}% ({}/{})", l.loss_pct(), l.lost, l.sent))
                            .monospace()
                            .color(Level::of(l.loss_pct(), speed::VOIP_LOSS_PCT).color()),
                    );
                    match idle.filter(|_| p != Phase::Idle) {
                        Some(i) => ui.label(
                            RichText::new(format!(
                                "+{}",
                                widgets::fmt_ms((l.avg - i.avg).max(0.0))
                            ))
                            .monospace(),
                        ),
                        None => ui.label(""),
                    };
                    let m = l.mos();
                    ui.label(
                        RichText::new(format!("{m:.2} {}", speed::mos_label(m)))
                            .color(Level::of_mos(m).color()),
                    );
                    ui.end_row();
                }
            });
    }

    /// What the numbers mean, worst first.
    fn findings(&self) -> Vec<(Level, String)> {
        let mut out = Vec::new();
        let Some(idle) = self.latency(Phase::Idle) else {
            return out;
        };
        if let Some((p, l, added)) = self.worst_loaded() {
            let g = speed::bloat_grade(added);
            let level = Level::of_grade(g);
            if level != Level::Ok {
                out.push((
                    level,
                    format!(
                        "Bufferbloat {g}: latency rises {} while {}. Calls break up whenever \
someone fills the line; fix with smart queue management (SQM: fq_codel or CAKE) or bandwidth \
management on the firewall, shaped just below the line rate.",
                        widgets::fmt_ms(added),
                        p.ing()
                    ),
                ));
            }
            let mos = l.mos();
            out.push((
                Level::of_mos(mos),
                format!(
                    "VoIP: MOS {:.2} ({}) at idle, {mos:.2} ({}) while {}.",
                    idle.mos(),
                    speed::mos_label(idle.mos()),
                    speed::mos_label(mos),
                    p.ing()
                ),
            ));
        } else {
            out.push((
                Level::of_mos(idle.mos()),
                format!(
                    "VoIP: MOS {:.2} ({}) at idle.",
                    idle.mos(),
                    speed::mos_label(idle.mos())
                ),
            ));
        }
        for p in Phase::ALL {
            let Some(l) = self.latency(p) else { continue };
            let over: Vec<String> = [
                (
                    "latency",
                    l.avg,
                    speed::VOIP_LATENCY_MS,
                    widgets::fmt_ms(l.avg),
                ),
                (
                    "jitter",
                    l.jitter,
                    speed::VOIP_JITTER_MS,
                    widgets::fmt_ms(l.jitter),
                ),
                (
                    "loss",
                    l.loss_pct(),
                    speed::VOIP_LOSS_PCT,
                    format!("{:.1}%", l.loss_pct()),
                ),
            ]
            .into_iter()
            .filter(|(_, v, target, _)| v > target)
            .map(|(what, _, _, shown)| format!("{what} {shown}"))
            .collect();
            if !over.is_empty() {
                out.push((
                    Level::Bad,
                    format!(
                        "Over the VoIP targets while {}: {}.",
                        p.ing(),
                        over.join(", ")
                    ),
                ));
            }
        }
        if self.method == Method::Tcp {
            out.push((
                Level::Warn,
                "ICMP got no replies, so latency was measured with TCP connects to port 443; \
loss is less reliable that way."
                    .into(),
            ));
        }
        out.sort_by_key(|(l, _)| std::cmp::Reverse(*l as u8));
        out
    }

    fn has_results(&self) -> bool {
        self.probes.iter().any(|p| !p.is_empty()) || self.results.iter().any(Option::is_some)
    }

    /// Start time and logged text of a run that has just ended, once.
    pub fn take_finished(&mut self) -> Option<(String, String, Option<Table>)> {
        if self.job.is_some() {
            return None;
        }
        let started = self.run_started.take()?;
        let out = self.output();
        let table = out.as_ref().map(|_| self.table());
        Some((started, log::body(out, &self.err), table))
    }

    /// Start time of the run in progress.
    pub fn running_since(&self) -> Option<&str> {
        self.job.as_ref().and(self.run_started.as_deref())
    }

    /// Report text for the Results tab; None until something has run.
    pub fn output(&self) -> Option<String> {
        self.has_results().then(|| self.report())
    }

    fn report(&self) -> String {
        let mut head = vec![speed::HOST.to_string()];
        head.extend(self.place());
        let mut s = format!("Speed test: {}\n", head.join(", "));
        let (secs, streams) = self.ran;
        for p in [Phase::Down, Phase::Up] {
            if let Some(t) = self.results[p.i()] {
                s += &format!(
                    "{:<10}{:>12}  ({}, {streams} streams, {secs} s)\n",
                    p.name(),
                    speed::fmt_rate(t.mbps),
                    speed::fmt_bytes(t.bytes)
                );
            }
        }
        let idle = self.latency(Phase::Idle);
        let mut rows = String::new();
        for p in Phase::ALL {
            let Some(l) = self.latency(p) else { continue };
            let added = match idle.filter(|_| p != Phase::Idle) {
                Some(i) => format!("+{}", widgets::fmt_ms((l.avg - i.avg).max(0.0))),
                None => String::new(),
            };
            rows += &format!(
                "{:<10}{:>10}{:>10}{:>10}{:>10}  {:<15}{:>10}  {:.2} {}\n",
                p.name(),
                widgets::fmt_ms(l.avg),
                widgets::fmt_ms(l.min),
                widgets::fmt_ms(l.max),
                widgets::fmt_ms(l.jitter),
                format!("{:.1}% ({}/{})", l.loss_pct(), l.lost, l.sent),
                added,
                l.mos(),
                speed::mos_label(l.mos())
            );
        }
        if !rows.is_empty() {
            s += &format!(
                "\n{:<10}{:>10}{:>10}{:>10}{:>10}  {:<15}{:>10}  MOS\n{rows}",
                self.method.name(),
                "Avg",
                "Min",
                "Max",
                "Jitter",
                "Loss",
                "Added"
            );
        }
        let findings = self.findings();
        if !findings.is_empty() {
            s += "\n";
            for (_, f) in findings {
                s += &format!("{f}\n");
            }
        }
        for w in &self.warns {
            s += &format!("warning: {w}\n");
        }
        s + "\n" + &targets_note()
    }

    fn table(&self) -> Table {
        let idle = self.latency(Phase::Idle);
        let rows = Phase::ALL
            .into_iter()
            .filter_map(|p| {
                let l = self.latency(p);
                let t = self.results[p.i()];
                if l.is_none() && t.is_none() {
                    return None;
                }
                let lat =
                    |f: fn(&Latency) -> f64| l.as_ref().map(|l| log::ms(f(l))).unwrap_or_default();
                let added = match (l, idle) {
                    (Some(l), Some(i)) if p != Phase::Idle => log::ms(l.avg - i.avg),
                    _ => String::new(),
                };
                Some(vec![
                    speed::HOST.to_string(),
                    log::cell(self.ip),
                    p.name().to_lowercase(),
                    t.map(|t| format!("{:.2}", t.mbps)).unwrap_or_default(),
                    t.map(|t| format!("{:.1}", t.bytes as f64 / 1e6))
                        .unwrap_or_default(),
                    if p == Phase::Idle {
                        String::new()
                    } else {
                        self.ran.0.to_string()
                    },
                    if p == Phase::Idle {
                        String::new()
                    } else {
                        self.ran.1.to_string()
                    },
                    self.method.name().into(),
                    log::cell(l.map(|l| l.sent)),
                    log::cell(l.map(|l| l.lost)),
                    lat(|l| l.avg),
                    lat(|l| l.min),
                    lat(|l| l.max),
                    lat(|l| l.jitter),
                    l.map(|l| format!("{:.1}", l.loss_pct()))
                        .unwrap_or_default(),
                    added,
                    l.map(|l| format!("{:.2}", l.mos())).unwrap_or_default(),
                ])
            })
            .collect();
        Table {
            columns: &[
                "server",
                "ip",
                "phase",
                "mbps",
                "mb",
                "seconds",
                "streams",
                "probe",
                "probes",
                "lost",
                "latency_avg_ms",
                "latency_min_ms",
                "latency_max_ms",
                "jitter_ms",
                "loss_pct",
                "added_ms",
                "mos",
            ],
            rows,
        }
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    fn start(&mut self, ctx: &egui::Context) {
        let (secs, streams, down, up) = (self.secs, self.streams, self.down, self.up);
        *self = Self {
            secs,
            streams,
            down,
            up,
            ran: (secs, streams),
            ..Default::default()
        };
        self.run_started = Some(log::now());
        self.job = Some(Job::spawn(ctx, move |tx| run(&tx, secs, streams, down, up)));
    }
}

/// Probes spaced `PROBE_EVERY` apart, start to start.
struct Prober {
    ip: IpAddr,
    method: Method,
    next: Instant,
}

impl Prober {
    /// Round-trip time, or None for no reply.
    fn probe(&mut self, tx: &Tx<Ev>, timeout_ms: u32) -> Option<f64> {
        tx.sleep(self.next.saturating_duration_since(Instant::now()));
        self.next = Instant::now() + PROBE_EVERY;
        match self.method {
            Method::Icmp => {
                let r = icmp::echo(self.ip, 128, timeout_ms, 32);
                r.is_reply().then_some(r.rtt_ms)
            }
            Method::Tcp => {
                let (state, ms) = net::tcp_probe(
                    SocketAddr::new(self.ip, 443),
                    Duration::from_millis(timeout_ms as u64),
                );
                matches!(state, TcpState::Open | TcpState::Closed).then_some(ms)
            }
        }
    }
}

fn run(tx: &Tx<Ev>, secs: u32, streams: usize, down: bool, up: bool) {
    let ip = match net::resolve(speed::HOST, false) {
        Ok(ip) => ip,
        Err(e) => return tx.send(Ev::Error(format!("could not resolve {}: {e}", speed::HOST))),
    };
    tx.send(Ev::Resolved(ip));
    let agent = speed::agent(streams);
    let mut meta = Some({
        let agent = agent.clone();
        std::thread::spawn(move || speed::meta(&agent, speed::BASE))
    });

    let mut p = Prober {
        ip,
        method: Method::Icmp,
        next: Instant::now(),
    };
    tx.send(Ev::Phase(Phase::Idle));
    let (mut sent, mut replies) = (0, 0);
    while sent < IDLE_PROBES && !tx.cancelled() {
        let rtt = p.probe(tx, IDLE_TIMEOUT_MS);
        sent += 1;
        replies += rtt.is_some() as usize;
        tx.send(Ev::Probe(Phase::Idle, rtt));
        if p.method == Method::Icmp && replies == 0 && sent == ICMP_TRIES {
            p.method = Method::Tcp;
            sent = 0;
            tx.send(Ev::Method(Method::Tcp));
        }
    }
    send_meta(tx, &mut meta, false);

    for (on, phase, dir) in [(down, Phase::Down, Dir::Down), (up, Phase::Up, Dir::Up)] {
        if on && !tx.cancelled() {
            transfer(tx, &agent, &mut p, phase, dir, secs, streams);
            send_meta(tx, &mut meta, false);
        }
    }
    send_meta(tx, &mut meta, !tx.cancelled());
}

/// Pass on where the test ran once the lookup is back; with `wait`, wait for it.
fn send_meta(tx: &Tx<Ev>, slot: &mut Option<JoinHandle<Option<Meta>>>, wait: bool) {
    if let Some(h) = slot.take_if(|h| wait || h.is_finished())
        && let Ok(Some(m)) = h.join()
    {
        tx.send(Ev::Meta(m));
    }
}

/// One direction for `secs`, probing latency the whole time.
fn transfer(
    tx: &Tx<Ev>,
    agent: &ureq::Agent,
    p: &mut Prober,
    phase: Phase,
    dir: Dir,
    secs: u32,
    streams: usize,
) {
    tx.send(Ev::Phase(phase));
    let t = Transfer::start(agent, speed::BASE, dir, streams);
    let secs = secs as f64;
    let warm = speed::warmup(secs);
    let start = Instant::now();
    // (time, bytes) over the last second: 100 ms windows are too jumpy to read.
    let mut recent = std::collections::VecDeque::from([(start, 0)]);
    let mut warmed = None;
    while !tx.cancelled() && start.elapsed().as_secs_f64() < secs {
        let rtt = p.probe(tx, LOADED_TIMEOUT_MS);
        tx.send(Ev::Probe(phase, rtt));
        let (now, bytes) = (Instant::now(), t.bytes());
        if warmed.is_none() && (now - start).as_secs_f64() >= warm {
            warmed = Some((now, bytes));
        }
        while recent.len() > 1 && now - recent[1].0 >= Duration::from_secs(1) {
            recent.pop_front();
        }
        let (at, base) = recent[0];
        if warmed.is_some() {
            tx.send(Ev::Rate(
                phase,
                speed::mbps(bytes - base, (now - at).as_secs_f64()),
            ));
        }
        recent.push_back((now, bytes));
    }
    let (end, total) = (Instant::now(), t.bytes());
    let err = t.error();
    t.stop(Duration::from_secs(2));
    if total == 0 {
        let why = match err {
            Some(e) => format!("{} failed: {e}. {BLOCKED_HINT}", phase.name()),
            None => format!("{} stopped before any data arrived.", phase.name()),
        };
        tx.send(Ev::Failed(phase, why));
        return;
    }
    // Slow start and filling buffers are left out; a run stopped before
    // the warm-up ended has no result.
    if let Some((from, base)) = warmed {
        let window = (end - from).as_secs_f64();
        if window >= 1.0 {
            tx.send(Ev::Done(
                phase,
                Throughput {
                    mbps: speed::mbps(total - base, window),
                    bytes: total,
                },
            ));
        }
    }
    // Let the queues drain before the next phase's probes.
    tx.sleep(Duration::from_millis(500));
}

fn targets_note() -> String {
    format!(
        "VoIP targets: latency < {:.0} ms, jitter < {:.0} ms, loss < {:.0}%. MOS is estimated \
with the simplified E-model (G.711): 4.34+ excellent, 4.03 good, 3.6 fair, 3.1 poor.",
        speed::VOIP_LATENCY_MS,
        speed::VOIP_JITTER_MS,
        speed::VOIP_LOSS_PCT
    )
}

/// A labelled figure with a smaller line under it.
fn tile(ui: &mut Ui, label: &str, value: String, sub: String, level: Option<Level>) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(112.0);
        ui.vertical(|ui| {
            ui.weak(label);
            let mut v = RichText::new(value).size(22.0).strong();
            if let Some(l) = level {
                v = v.color(l.color());
            }
            ui.label(v);
            ui.weak(if sub.is_empty() { " ".into() } else { sub });
        });
    });
}

/// Live throughput, download then upload along one time line.
fn rate_chart(ui: &mut Ui, rates: &[Vec<f64>; 3], height: f32) {
    // A steady rate runs near the peak: room above it for the labels.
    const HEADROOM: f64 = 1.35;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let vis = ui.visuals();
    p.rect_filled(rect, 4.0, vis.extreme_bg_color);
    let series = [
        (Phase::Down, widgets::ok()),
        (Phase::Up, crate::theme::accent()),
    ];
    let n: usize = series.iter().map(|(ph, _)| rates[ph.i()].len()).sum();
    if n == 0 {
        return;
    }
    let peak = series
        .iter()
        .flat_map(|(ph, _)| rates[ph.i()].iter())
        .fold(0.1f64, |a, &b| a.max(b))
        * HEADROOM;
    let max = ui
        .ctx()
        .animate_value_with_time(ui.id().with("rate_scale"), peak as f32, 0.3) as f64;
    let step = (rect.width() - 8.0) / (n.max(2) - 1) as f32;
    let y = |v: f64| rect.bottom() - 4.0 - (rect.height() - 8.0) * (v / max).min(1.0) as f32;
    let mut i = 0;
    let mut legend = rect.right() - 6.0;
    for (ph, colour) in series.iter().rev() {
        if !rates[ph.i()].is_empty() {
            let r = p.text(
                egui::pos2(legend, rect.top() + 4.0),
                egui::Align2::RIGHT_TOP,
                ph.name(),
                egui::FontId::proportional(11.0),
                *colour,
            );
            legend = r.left() - 10.0;
        }
    }
    for (ph, colour) in series {
        let pts: Vec<egui::Pos2> = rates[ph.i()]
            .iter()
            .map(|&v| {
                let pt = egui::pos2(rect.left() + 4.0 + step * i as f32, y(v));
                i += 1;
                pt
            })
            .collect();
        if pts.len() > 1 {
            p.add(egui::Shape::line(pts, egui::Stroke::new(1.5, colour)));
        }
    }
    p.text(
        rect.left_top() + egui::vec2(6.0, 4.0),
        egui::Align2::LEFT_TOP,
        speed::fmt_rate(max / HEADROOM),
        egui::FontId::monospace(10.0),
        vis.weak_text_color(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SpeedTool {
        let mut t = SpeedTool {
            ip: Some(IpAddr::from([192, 0, 2, 1])),
            ..Default::default()
        };
        t.probes[Phase::Idle.i()] = vec![Some(10.0), Some(12.0), Some(11.0), Some(13.0)];
        t.probes[Phase::Up.i()] = vec![Some(200.0), Some(260.0), None, Some(230.0)];
        t.results[Phase::Down.i()] = Some(Throughput {
            mbps: 412.3,
            bytes: 515_000_000,
        });
        t.results[Phase::Up.i()] = Some(Throughput {
            mbps: 38.2,
            bytes: 47_800_000,
        });
        t
    }

    #[test]
    fn rows_match_columns() {
        let t = sample().table();
        assert_eq!(t.rows.len(), 3);
        for r in &t.rows {
            assert_eq!(r.len(), t.columns.len());
        }
        // Download has a rate but no latency probes in this sample.
        assert_eq!(t.rows[1][2], "download");
        assert_eq!(t.rows[1][3], "412.30");
        assert_eq!(t.rows[1][10], "");
        assert_eq!(t.rows[2][15], "218.50");
    }

    #[test]
    fn report_explains_bufferbloat_and_voip() {
        let r = sample().report();
        assert!(r.contains("Download      412 Mbps"), "{r}");
        assert!(
            r.contains("Bufferbloat D: latency rises 218 ms while uploading"),
            "{r}"
        );
        assert!(
            r.contains("Over the VoIP targets while uploading: latency 230 ms, jitter"),
            "{r}"
        );
        // Worst first.
        let f = sample().findings();
        assert!(f.windows(2).all(|w| w[0].0 as u8 >= w[1].0 as u8));
        assert_eq!(f[0].0, Level::Bad);
    }

    #[test]
    fn worst_loaded_picks_the_most_added_latency() {
        let mut t = sample();
        t.probes[Phase::Down.i()] = vec![Some(40.0); 3];
        let (p, _, added) = t.worst_loaded().unwrap();
        assert_eq!(p, Phase::Up);
        assert_eq!(added, 218.5);
    }

    #[test]
    fn levels() {
        assert!(Level::of(0.0, 1.0) == Level::Ok);
        assert!(Level::of(1.5, 1.0) == Level::Warn);
        assert!(Level::of(31.0, 15.0) == Level::Bad);
        assert!(Level::of_grade("A+") == Level::Ok);
        assert!(Level::of_grade("F") == Level::Bad);
    }
}
