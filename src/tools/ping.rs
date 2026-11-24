use crate::input::Field;
use crate::job::{Job, pump};
use crate::log::{self, Table};
use crate::net::icmp::{self, EchoStatus};
use crate::net::mac;
use crate::net::{self, TcpState, wol};
use crate::widgets::{self, Action};
use eframe::egui::{self, RichText, Ui};
use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

/// Results (CSV export, the saved log): one row per result, these columns.
pub const COLUMNS: &[&str] = &[
    "target", "mode", "port", "seq", "from", "status", "rtt_ms", "ttl", "bytes",
];

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Icmp,
    Tcp,
}

/// Wake & wait: a ping this often after the Wake-on-LAN packet...
const WAKE_EVERY: Duration = Duration::from_secs(5);
/// ...for this long. A PC from shutdown takes a minute or two to boot.
const WAKE_FOR: Duration = Duration::from_secs(180);

/// A device the site has seen with a MAC Wake can use: its name (when
/// it has one) and what to wake. From the last sweep, then the Map.
pub type Known = (Option<String>, wol::Target);

/// What the box asks to wake, if it names a MAC rather than a host: a
/// MAC typed (optionally with its IP after it: "00:11:32:ab:cd:ef
/// 192.168.1.50"), or a name the site has already seen with a MAC. The
/// IP, when not typed, is where the site last saw that MAC.
/// `nets` are the ranges the last sweep covered whole: an address inside
/// one also gets its broadcast, whether or not it answered the sweep.
fn wake_of(text: &str, known: &[Known], nets: &[(Ipv4Addr, u32)]) -> Option<wol::Target> {
    let net = |ip: Option<Ipv4Addr>| ip.and_then(|ip| crate::net::targets::network_of(nets, ip));
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|w| !w.is_empty())
        .collect();
    let first = *words.first()?;
    if let Some(m) = mac::parse_mac(first) {
        let seen = known.iter().find(|k| k.1.mac == m).map(|k| k.1);
        let typed = match words.get(1) {
            Some(w) => Some(w.parse::<Ipv4Addr>().ok()?),
            None => None,
        };
        if words.len() > 2 {
            return None;
        }
        let ip = typed.or(seen.and_then(|s| s.ip));
        return Some(wol::Target {
            mac: m,
            ip,
            net: net(ip),
        });
    }
    if words.len() != 1 || first.parse::<IpAddr>().is_ok() {
        return None;
    }
    let bare = |n: &str| {
        n.trim_end_matches('.')
            .trim_end_matches(".local")
            .to_ascii_lowercase()
    };
    let want = bare(first);
    known
        .iter()
        .find(|(n, _)| n.as_deref().is_some_and(|n| bare(n) == want))
        .map(|k| wol::Target {
            net: net(k.1.ip),
            ..k.1
        })
}

/// Beside Wake: what it will do.
fn wake_hint(t: &wol::Target) -> String {
    let m = mac::fmt(&t.mac);
    match t.ip {
        Some(ip) => format!("Wake-on-LAN to {m}, then wait for {ip}"),
        None => format!("Wake-on-LAN to {m} (no IP known: add it to wait for it)"),
    }
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
    /// A line of Wake & wait's own: what went out, and how it ended.
    Note(bool, String),
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
            Ev::Note(ok, text) => push_capped(&mut self.lines, (ok, text), MAX_LINES),
        })
    }

    /// The tab. `known` is every device the site has seen with a MAC, for
    /// a name typed in the box to be woken by.
    /// `nets` are the ranges the last sweep covered whole.
    pub fn ui(&mut self, ui: &mut Ui, known: &[Known], nets: &[(Ipv4Addr, u32)]) {
        let running = self.poll();
        let wake = wake_of(&self.host, known, nets);

        let mut enter = false;
        // Wraps on a narrow window (docs/dev/ui-layout.md): TCP mode's Port box would
        // be cut off at 620 wide.
        ui.horizontal_wrapped(|ui| {
            enter |= widgets::field(
                ui,
                "Host",
                Field::Host,
                &mut self.host,
                widgets::MAIN_W,
                "hostname, IP or MAC",
            );
            let act = match &wake {
                Some(t) if !running => wake_button(ui, t, enter),
                _ => widgets::run_button(ui, running, enter),
            };
            match act {
                Action::Start => match wake {
                    Some(t) => self.wake(ui.ctx(), t),
                    None => self.start(ui.ctx()),
                },
                Action::Stop => self.job.as_ref().map_or((), Job::stop),
                Action::None => {}
            }
            ui.separator();
            if let Some(t) = &wake {
                // In the mode switch's place: no new row (docs/dev/ui-layout.md).
                let hint = wake_hint(t);
                ui.add(egui::Label::new(RichText::new(&hint).weak()).truncate())
                    .on_hover_text(format!(
                        "{hint}. A MAC, or a name the last sweep or the Map saw with a MAC, \
                         sends Wake-on-LAN instead of a ping. With an IP (typed after the MAC, \
                         or where it was last seen) it then pings every 5 s for up to 3 \
                         minutes. Type the IP to ping it without waking it."
                    ));
            } else {
                widgets::segmented(ui, |ui| {
                    ui.selectable_value(&mut self.mode, Mode::Icmp, "ICMP");
                    ui.selectable_value(&mut self.mode, Mode::Tcp, "TCP");
                });
                if self.mode == Mode::Tcp {
                    widgets::num(ui, "Port", &mut self.port, 1..=65535, "");
                }
            }
        });
        let summary = self.summary();
        widgets::options(ui, "ping", &summary, |ui| {
            ui.horizontal_wrapped(|ui| {
                widgets::num(ui, "Count", &mut self.count, 0..=100_000, "");
                ui.weak("(0 = continuous)");
                widgets::num(ui, "Interval", &mut self.interval_ms, 100..=60_000, " ms");
                widgets::num(ui, "Timeout", &mut self.timeout_ms, 100..=30_000, " ms");
                if self.mode == Mode::Icmp {
                    widgets::num(ui, "Size", &mut self.size, 0..=65_500, " B");
                }
                ui.checkbox(&mut self.prefer_v6, "Prefer IPv6");
            });
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
            widgets::empty_state(
                ui,
                "Ping a host",
                "Enter a host and press Enter. ICMP is a classic ping; TCP times a \
                 connection to a port, for hosts that drop ICMP. Count 0 pings until stopped.",
                &[],
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

    /// The options, folded: how many pings, how often, how long to wait.
    fn summary(&self) -> Vec<String> {
        let mut s = vec![
            match self.count {
                0 => "Until stopped".to_string(),
                1 => "1 ping".to_string(),
                n => format!("{n} pings"),
            },
            format!("every {} ms", self.interval_ms),
            format!("timeout {} ms", self.timeout_ms),
        ];
        if self.mode == Mode::Icmp {
            s.push(format!("{} B", self.size));
        }
        if self.prefer_v6 {
            s.push("IPv6 first".into());
        }
        s
    }

    fn stats_row(&self, ui: &mut Ui) {
        let (sent, recv, mm) = (self.stats.sent, self.stats.recv, self.stats.min_avg_max());
        widgets::status_row(
            ui,
            |ui| {
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
            },
            |ui| {
                ui.add_enabled_ui(sent > 0, |ui| {
                    widgets::save_button(ui, "ping", || self.report());
                    widgets::copy_button(ui, || self.report());
                });
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
        (self.stats.sent > 0 || !self.lines.is_empty()).then(|| self.report())
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
            columns: COLUMNS,
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

    /// `count` ICMP pings to `host` with the tab showing that count (the
    /// user manual's screenshot tour).
    pub fn run_shown(&mut self, ctx: &egui::Context, host: &str, count: u32) {
        self.mode = Mode::Icmp;
        self.count = count;
        self.run(ctx, host, count);
    }

    /// For the screenshot tour: `text` in the box, nothing sent.
    pub fn show_for_shots(&mut self, text: &str) {
        self.host = text.into();
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    /// Why the last run stopped short, if it did.
    pub fn error(&self) -> Option<&str> {
        self.err.as_deref()
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.launch(ctx, self.mode, self.count);
    }

    /// Wake: the Wake-on-LAN packet, then, with an IP known, a ping every
    /// 5 s until it answers ("up after N s") or 3 minutes pass ("no
    /// answer"). Only ever from the tab's own button: Full scan and the
    /// Map's quick runs go through `run`, which never wakes anything.
    fn wake(&mut self, ctx: &egui::Context, t: wol::Target) {
        if !wol::wakeable(&t.mac) {
            self.err = Some(format!(
                "{} is a private, multicast or empty MAC: no network card wakes to it",
                mac::fmt(&t.mac)
            ));
            return;
        }
        self.err = None;
        self.target = None;
        self.lines.clear();
        self.rows.clear();
        self.samples.clear();
        self.stats = Stats::default();
        self.run_started = Some(log::now());
        self.job = Some(Job::spawn(ctx, move |tx| {
            let sent = wol::wake(&t);
            let (text, ok) = wol::report(&t.mac, &sent);
            if !ok {
                return tx.send(Ev::Error(text));
            }
            // The report's "give it a minute, then Ping it" is this run's job.
            let text = text
                .replace(" Give it a minute, then Ping it.", "")
                .replace(" Give it a minute, then Ping it", "");
            // Its directed-broadcast sentence, if any, is already in it.
            tx.send(Ev::Note(true, text));
            let Some(ip) = t.ip else {
                return tx.send(Ev::Note(
                    true,
                    "No IP known to wait for: type it after the MAC to have Wake wait for it, \
                     or Ping it in a minute."
                        .into(),
                ));
            };
            tx.send(Ev::Resolved(IpAddr::V4(ip)));
            tx.send(Ev::Note(
                true,
                format!("Waiting for {ip}: a ping every 5 s for up to 3 minutes."),
            ));
            let t0 = Instant::now();
            let mut seq = 0;
            while !tx.cancelled() {
                seq += 1;
                let tick = Instant::now();
                let ev = icmp_line(IpAddr::V4(ip), seq, 32, 1000);
                let up = matches!(ev, Ev::Reply { ok: true, .. });
                tx.send(ev);
                let secs = t0.elapsed().as_secs();
                if up {
                    return tx.send(Ev::Note(true, format!("{ip} is up after {secs} s.")));
                }
                if t0.elapsed() + WAKE_EVERY > WAKE_FOR {
                    return tx.send(Ev::Note(false, wake_gave_up(ip, secs)));
                }
                tx.sleep(WAKE_EVERY.saturating_sub(tick.elapsed()));
            }
        }));
    }

    fn launch(&mut self, ctx: &egui::Context, mode: Mode, count: u32) {
        // A host that won't read leaves the last run as it was.
        let target = match crate::input::host(&self.host) {
            Ok(t) => t,
            Err(e) => return self.err = Some(e),
        };
        self.err = None;
        self.target = None;
        self.lines.clear();
        self.rows.clear();
        self.samples.clear();
        self.stats = Stats::default();
        // A port typed with the host (host:8443, a link) is the one TCP times.
        if let Some(p) = target.port
            && mode == Mode::Tcp
        {
            self.port = p;
        }
        let host = target.host;
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

/// Wake's button, in Start's place: "Wake & wait" when there's an IP to
/// wait for.
fn wake_button(ui: &mut Ui, t: &wol::Target, enter: bool) -> Action {
    let (label, tip) = match t.ip {
        Some(ip) => (
            "▶ Wake & wait",
            format!("Send Wake-on-LAN, then ping {ip} every 5 s for up to 3 minutes (Enter)"),
        ),
        None => ("▶ Wake", "Send Wake-on-LAN (Enter)".to_string()),
    };
    if ui.add(widgets::primary(label)).on_hover_text(tip).clicked() || enter {
        Action::Start
    } else {
        Action::None
    }
}

/// How Wake & wait ends when nothing answered.
fn wake_gave_up(ip: Ipv4Addr, secs: u64) -> String {
    format!(
        "No answer from {ip} after {secs} s. It may not have woken (Wake-on-LAN off in its \
         BIOS or network card, a Wi-Fi adapter, Windows Fast Startup), it may drop ping, or \
         the packet didn't reach its network."
    )
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
    fn the_folded_options_say_how_the_next_ping_goes() {
        let mut t = PingTool::default();
        assert_eq!(
            t.summary(),
            ["Until stopped", "every 1000 ms", "timeout 1000 ms", "32 B"]
        );
        t.count = 4;
        t.mode = Mode::Tcp;
        t.prefer_v6 = true;
        // TCP sends no payload: no size.
        assert_eq!(
            t.summary(),
            ["4 pings", "every 1000 ms", "timeout 1000 ms", "IPv6 first"]
        );
    }

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
    fn a_mac_or_a_name_seen_with_one_is_woken() {
        use crate::net::mac::Mac;
        let ip = |s: &str| s.parse::<Ipv4Addr>().unwrap();
        let m: Mac = [0x00, 0x11, 0x32, 0xAB, 0xCD, 0xEF];
        let known: Vec<Known> = vec![(
            Some("FRONTDESK-PC".into()),
            wol::Target {
                mac: m,
                ip: Some(ip("10.20.0.5")),
                net: Some((ip("10.20.0.0"), 24)),
            },
        )];
        let nets = [(ip("10.20.0.0"), 24)];
        // A MAC alone: where the site last saw it, with its swept network.
        let t = wake_of("00:11:32:ab:cd:ef", &known, &nets).unwrap();
        assert_eq!(
            (t.ip, t.net),
            (Some(ip("10.20.0.5")), Some((ip("10.20.0.0"), 24)))
        );
        // A MAC and an IP typed: that IP; outside the swept range, so no network.
        let t = wake_of("00-11-32-AB-CD-EF 192.168.1.50", &known, &nets).unwrap();
        assert_eq!((t.ip, t.net), (Some(ip("192.168.1.50")), None));
        // A sleeping PC didn't answer the sweep, but its address is in the
        // swept range: that range's broadcast too.
        let t = wake_of("00:11:32:00:00:09 10.20.0.50", &known, &nets).unwrap();
        assert_eq!(t.net, Some((ip("10.20.0.0"), 24)));
        // A MAC never seen: nothing to wait for.
        let t = wake_of("00:11:32:00:00:01", &known, &nets).unwrap();
        assert_eq!((t.mac[5], t.ip), (1, None));
        // A name the site saw with a MAC, any case, .local or not.
        assert_eq!(wake_of("frontdesk-pc.local", &known, &nets).unwrap().mac, m);
        // Everything else is a ping.
        for text in [
            "10.20.0.5",
            "example.com",
            "FRONTDESK-PC 1",
            "00:11:32:ab:cd:ef x",
            "",
        ] {
            assert_eq!(wake_of(text, &known, &nets), None, "{text}");
        }
        assert!(wake_hint(&wake_of("00:11:32:00:00:01", &known, &nets).unwrap()).contains("no IP"));
        assert!(wake_gave_up(ip("10.20.0.5"), 180).contains("Fast Startup"));
    }

    /// Full scan and the Map ping through `run`: a MAC there is a host
    /// that won't resolve, never a Wake-on-LAN packet.
    #[test]
    fn run_never_wakes() {
        let ctx = egui::Context::default();
        let mut t = PingTool::default();
        t.run(&ctx, "00:11:32:ab:cd:ef", 1);
        let end = Instant::now() + Duration::from_secs(10);
        while t.poll() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(t.lines.iter().all(|(_, l)| !l.contains("Wake-on-LAN")));
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
