//! Spot checks run from a Map card: Ping, Port Check and Traceroute against
//! one device, with a condensed result kept in the card. They're for a quick
//! look only, so they don't reach the Results tab or exports, and they're
//! kept per card for the session (a map rebuild that changes a card's id,
//! or a restart, lets them go). A full run is still one click away in the
//! tool's own tab.

use crate::job::Job;
use crate::net::icmp::{self, EchoStatus};
use crate::net::ports::{PRESETS, parse_ports};
use crate::net::{self, TcpState};
use crate::widgets;
use eframe::egui;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

/// Most card runs probing at once; the rest wait their turn, so a big
/// selection can't flood the LAN.
pub const MAX_RUNNING: usize = 3;
pub const PINGS: u32 = 4;
const PING_EVERY: Duration = Duration::from_secs(1);
const PING_TIMEOUT_MS: u32 = 1000;
const PORT_TIMEOUT: Duration = Duration::from_millis(1500);
const MAX_HOPS: u8 = 30;
const HOP_TIMEOUT_MS: u32 = 1000;
const HOP_PROBES: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Tool {
    Ping,
    Ports,
    Trace,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Ping => "Ping",
            Tool::Ports => "Port Check",
            Tool::Trace => "Traceroute",
        }
    }
}

pub struct Reply {
    pub seq: u32,
    /// Round trip, when it answered.
    pub rtt: Option<f64>,
    pub ttl: Option<u8>,
    /// Why it didn't answer.
    pub note: String,
}

pub struct Hop {
    pub ttl: u8,
    pub addr: Option<IpAddr>,
    pub name: Option<String>,
    /// Fastest of its probes.
    pub best: Option<f64>,
}

pub enum Out {
    Ping(Vec<Reply>),
    Ports {
        total: usize,
        /// In port order.
        found: Vec<(u16, TcpState, f64)>,
    },
    Trace {
        hops: Vec<Hop>,
        reached: Option<bool>,
    },
}

#[derive(Clone, PartialEq, Debug)]
pub enum State {
    Queued,
    Running,
    Done,
    Stopped,
    Failed(String),
}

enum Ev {
    Reply(Reply),
    Total(usize),
    Port(u16, TcpState, f64),
    Hop(Hop),
    Done(bool),
    Error(String),
}

pub struct Run {
    pub tool: Tool,
    pub ip: IpAddr,
    pub state: State,
    pub out: Out,
    /// Queue position: lower starts first.
    seq: u64,
    job: Option<Job<Ev>>,
}

impl Run {
    fn new(tool: Tool, ip: IpAddr, seq: u64) -> Self {
        let out = match tool {
            Tool::Ping => Out::Ping(Vec::new()),
            Tool::Ports => Out::Ports {
                total: 0,
                found: Vec::new(),
            },
            Tool::Trace => Out::Trace {
                hops: Vec::new(),
                reached: None,
            },
        };
        Self {
            tool,
            ip,
            state: State::Queued,
            out,
            seq,
            job: None,
        }
    }

    /// Queued or running.
    pub fn active(&self) -> bool {
        matches!(self.state, State::Queued | State::Running)
    }

    /// The one-line result shown while the section is collapsed.
    pub fn summary(&self) -> String {
        let live = self.state == State::Running;
        let line = match &self.state {
            State::Queued => return "Waiting for another run to finish".into(),
            State::Failed(e) => return e.clone(),
            _ => match &self.out {
                Out::Ping(r) => ping_line(r, live),
                Out::Ports { total, found } => ports_line(*total, found, live),
                Out::Trace { hops, reached } => trace_line(hops, *reached, self.ip, live),
            },
        };
        if self.state == State::Stopped {
            format!("{line} · stopped")
        } else {
            line
        }
    }

    fn apply(&mut self, ev: Ev) {
        match (ev, &mut self.out) {
            (Ev::Reply(r), Out::Ping(v)) => v.push(r),
            (Ev::Total(n), Out::Ports { total, .. }) => *total = n,
            (Ev::Port(p, s, ms), Out::Ports { found, .. }) => {
                let at = found.partition_point(|r| r.0 < p);
                found.insert(at, (p, s, ms));
            }
            (Ev::Hop(h), Out::Trace { hops, .. }) => hops.push(h),
            (Ev::Done(r), Out::Trace { reached, .. }) => *reached = Some(r),
            (Ev::Error(e), _) => self.state = State::Failed(e),
            _ => {}
        }
    }

    fn launch(&mut self, ctx: &egui::Context) {
        let ip = self.ip;
        self.state = State::Running;
        self.job = Some(match self.tool {
            Tool::Ping => Job::spawn(ctx, move |tx| {
                for seq in 1..=PINGS {
                    if tx.cancelled() {
                        return;
                    }
                    let t0 = Instant::now();
                    let r = icmp::echo(ip, 128, PING_TIMEOUT_MS, 32);
                    let (rtt, note) = match r.status {
                        EchoStatus::Reply => (Some(r.rtt_ms), String::new()),
                        EchoStatus::Timeout => (None, "timed out".into()),
                        EchoStatus::TtlExpired => (None, "TTL expired in transit".into()),
                        EchoStatus::Unreachable(m) | EchoStatus::Error(m) => (None, m),
                    };
                    tx.send(Ev::Reply(Reply {
                        seq,
                        rtt,
                        ttl: r.ttl,
                        note,
                    }));
                    if seq < PINGS {
                        tx.sleep(PING_EVERY.saturating_sub(t0.elapsed()));
                    }
                }
            }),
            Tool::Ports => Job::spawn(ctx, move |tx| {
                let ports = match parse_ports(PRESETS[0].1) {
                    Ok(p) => p,
                    Err(e) => return tx.send(Ev::Error(e)),
                };
                tx.send(Ev::Total(ports.len()));
                let (_, threads, rate, _) = widgets::SPEEDS[widgets::DEFAULT_SPEED];
                net::par_for(&ports, threads, rate, tx.cancel_flag(), |&p| {
                    let (state, ms) = net::tcp_check(SocketAddr::new(ip, p), PORT_TIMEOUT);
                    tx.send(Ev::Port(p, state, ms));
                });
            }),
            Tool::Trace => Job::spawn(ctx, move |tx| {
                for ttl in 1..=MAX_HOPS {
                    let mut hop = Hop {
                        ttl,
                        addr: None,
                        name: None,
                        best: None,
                    };
                    let (mut reached, mut stop) = (false, false);
                    for _ in 0..HOP_PROBES {
                        if tx.cancelled() {
                            return;
                        }
                        let r = icmp::echo(ip, ttl, HOP_TIMEOUT_MS, 32);
                        if r.from.is_some() {
                            hop.addr = hop.addr.or(r.from);
                            hop.best = Some(hop.best.map_or(r.rtt_ms, |b| b.min(r.rtt_ms)));
                        }
                        match r.status {
                            EchoStatus::Reply => reached = true,
                            EchoStatus::Unreachable(_) => stop = true,
                            _ => {}
                        }
                    }
                    hop.name = hop.addr.and_then(net::reverse);
                    tx.send(Ev::Hop(hop));
                    if reached || stop {
                        return tx.send(Ev::Done(reached));
                    }
                }
                tx.send(Ev::Done(false));
            }),
        });
    }
}

/// Every card's runs, by node id, newest first within a card.
#[derive(Default)]
pub struct CardRuns {
    cards: HashMap<String, Vec<Run>>,
    next: u64,
}

impl CardRuns {
    pub fn runs(&self, card: &str) -> &[Run] {
        self.cards.get(card).map_or(&[], Vec::as_slice)
    }

    pub fn active(&self, card: &str, tool: Tool) -> bool {
        self.runs(card).iter().any(|r| r.tool == tool && r.active())
    }

    /// Queue a run. One that already ran on this card is replaced where it
    /// is (stopping it if it's still going); a new one goes on top.
    pub fn start(&mut self, card: &str, tool: Tool, ip: IpAddr) {
        self.next += 1;
        let run = Run::new(tool, ip, self.next);
        let runs = self.cards.entry(card.to_string()).or_default();
        match runs.iter_mut().find(|r| r.tool == tool) {
            Some(old) => *old = run,
            None => runs.insert(0, run),
        }
    }

    /// Stop a run, keeping what it found so far.
    pub fn stop(&mut self, card: &str, tool: Tool) {
        if let Some(r) = self.find(card, tool)
            && r.active()
        {
            // Dropping the job tells its thread to stop.
            r.job = None;
            r.state = State::Stopped;
        }
    }

    /// Take a run's section out of the card (stopping it).
    pub fn clear(&mut self, card: &str, tool: Tool) {
        if let Some(runs) = self.cards.get_mut(card) {
            runs.retain(|r| r.tool != tool);
            if runs.is_empty() {
                self.cards.remove(card);
            }
        }
    }

    fn find(&mut self, card: &str, tool: Tool) -> Option<&mut Run> {
        self.cards
            .get_mut(card)?
            .iter_mut()
            .find(|r| r.tool == tool)
    }

    /// Take in what the running ones found, then start queued ones while
    /// fewer than MAX_RUNNING are going. Called every frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        for r in self.cards.values_mut().flatten() {
            let Some(job) = &r.job else { continue };
            let alive = job.running();
            let evs: Vec<Ev> = job.drain().collect();
            for ev in evs {
                r.apply(ev);
            }
            if !alive {
                r.job = None;
                if r.state == State::Running {
                    r.state = State::Done;
                }
            }
        }
        for (card, tool) in self.due() {
            if let Some(r) = self.find(&card, tool) {
                r.launch(ctx);
            }
        }
    }

    /// The queued runs to start now, oldest first, up to the free places.
    fn due(&self) -> Vec<(String, Tool)> {
        let all = || {
            self.cards
                .iter()
                .flat_map(|(c, v)| v.iter().map(move |r| (c, r)))
        };
        let running = all().filter(|(_, r)| r.state == State::Running).count();
        let mut queued: Vec<(u64, &String, Tool)> = all()
            .filter(|(_, r)| r.state == State::Queued)
            .map(|(c, r)| (r.seq, c, r.tool))
            .collect();
        queued.sort_by_key(|q| q.0);
        queued
            .into_iter()
            .take(MAX_RUNNING.saturating_sub(running))
            .map(|(_, c, t)| (c.clone(), t))
            .collect()
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `4/4 replies · avg 0.34 ms · 0% loss`
pub fn ping_line(replies: &[Reply], live: bool) -> String {
    let sent = replies.len();
    if sent == 0 {
        return if live {
            "Pinging…".into()
        } else {
            "No pings sent".into()
        };
    }
    let rtts: Vec<f64> = replies.iter().filter_map(|r| r.rtt).collect();
    let loss = ((sent - rtts.len()) as f64 * 100.0 / sent as f64).round();
    let mut s = format!("{}/{sent} replies", rtts.len());
    if !rtts.is_empty() {
        let avg = rtts.iter().sum::<f64>() / rtts.len() as f64;
        s += &format!(" · avg {}", widgets::fmt_ms(avg));
    }
    s + &format!(" · {loss}% loss")
}

/// `Open: 22, 80, 443 · 12 closed`
pub fn ports_line(total: usize, found: &[(u16, TcpState, f64)], live: bool) -> String {
    if found.is_empty() {
        return if live {
            format!("Checking {}…", plural(total, "port", "ports"))
        } else {
            "No ports checked".into()
        };
    }
    let open: Vec<String> = found
        .iter()
        .filter(|r| r.1 == TcpState::Open)
        .map(|r| r.0.to_string())
        .collect();
    let count = |want: fn(&TcpState) -> bool| found.iter().filter(|r| want(&r.1)).count();
    let closed = count(|s| *s == TcpState::Closed);
    let filtered = count(|s| *s == TcpState::Filtered);
    let errors = count(|s| matches!(s, TcpState::Error(_)));
    let mut parts = vec![if open.is_empty() {
        "None open".to_string()
    } else {
        format!("Open: {}", open.join(", "))
    }];
    if closed > 0 {
        parts.push(format!("{closed} closed"));
    }
    if filtered > 0 {
        parts.push(format!("{filtered} filtered"));
    }
    if errors > 0 {
        parts.push(plural(errors, "error", "errors"));
    }
    if live {
        parts.push(format!("{}/{total} checked", found.len()));
    }
    parts.join(" · ")
}

/// `3 hops · 1.2 ms to 10.0.0.251`
pub fn trace_line(hops: &[Hop], reached: Option<bool>, ip: IpAddr, live: bool) -> String {
    let n = plural(hops.len(), "hop", "hops");
    if live {
        return if hops.is_empty() {
            "Tracing…".into()
        } else {
            format!("{n} so far")
        };
    }
    match (reached, hops.last().and_then(|h| h.best)) {
        (Some(true), Some(ms)) => format!("{n} · {} to {ip}", widgets::fmt_ms(ms)),
        (Some(true), None) => format!("{n} to {ip}"),
        _ if hops.is_empty() => "No hops".into(),
        _ => format!("{n} · no reply from {ip}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(seq: u32, rtt: Option<f64>) -> Reply {
        Reply {
            seq,
            rtt,
            ttl: rtt.map(|_| 64),
            note: if rtt.is_some() { "" } else { "timed out" }.into(),
        }
    }

    fn hop(ttl: u8, ip: &str, best: Option<f64>) -> Hop {
        Hop {
            ttl,
            addr: Some(ip.parse().unwrap()),
            name: None,
            best,
        }
    }

    #[test]
    fn ping_line_all_some_none() {
        let all: Vec<Reply> = (1..=4).map(|s| reply(s, Some(0.34))).collect();
        assert_eq!(
            ping_line(&all, false),
            "4/4 replies · avg 0.34 ms · 0% loss"
        );
        let some = vec![
            reply(1, Some(2.0)),
            reply(2, None),
            reply(3, Some(4.0)),
            reply(4, None),
        ];
        assert_eq!(
            ping_line(&some, false),
            "2/4 replies · avg 3.0 ms · 50% loss"
        );
        let none: Vec<Reply> = (1..=4).map(|s| reply(s, None)).collect();
        assert_eq!(ping_line(&none, false), "0/4 replies · 100% loss");
        // Live, it counts what's been sent so far.
        assert_eq!(
            ping_line(&all[..1], true),
            "1/1 replies · avg 0.34 ms · 0% loss"
        );
        assert_eq!(ping_line(&[], true), "Pinging…");
    }

    #[test]
    fn ports_line_lists_open_and_counts_the_rest() {
        let mut found: Vec<(u16, TcpState, f64)> = [22, 80, 443]
            .into_iter()
            .map(|p| (p, TcpState::Open, 1.0))
            .collect();
        found.extend((1000..1012).map(|p| (p, TcpState::Closed, 1.0)));
        assert_eq!(
            ports_line(15, &found, false),
            "Open: 22, 80, 443 · 12 closed"
        );
        found.push((9999, TcpState::Filtered, 1500.0));
        assert_eq!(
            ports_line(20, &found, true),
            "Open: 22, 80, 443 · 12 closed · 1 filtered · 16/20 checked"
        );
        let shut = vec![(22, TcpState::Closed, 1.0)];
        assert_eq!(ports_line(1, &shut, false), "None open · 1 closed");
        assert_eq!(ports_line(15, &[], true), "Checking 15 ports…");
    }

    #[test]
    fn trace_line_reached_or_not() {
        let ip: IpAddr = "10.0.0.251".parse().unwrap();
        let hops = vec![
            hop(1, "10.0.0.1", Some(0.5)),
            hop(2, "10.0.1.1", Some(0.9)),
            hop(3, "10.0.0.251", Some(1.2)),
        ];
        assert_eq!(
            trace_line(&hops, Some(true), ip, false),
            "3 hops · 1.2 ms to 10.0.0.251"
        );
        assert_eq!(
            trace_line(&hops[..1], Some(true), ip, false),
            "1 hop · 0.50 ms to 10.0.0.251"
        );
        assert_eq!(
            trace_line(&hops, Some(false), ip, false),
            "3 hops · no reply from 10.0.0.251"
        );
        assert_eq!(trace_line(&hops[..2], None, ip, true), "2 hops so far");
    }

    fn ip() -> IpAddr {
        "192.0.2.7".parse().unwrap()
    }

    #[test]
    fn a_new_run_goes_on_top_and_a_rerun_replaces_only_its_own() {
        let mut c = CardRuns::default();
        c.start("a", Tool::Ping, ip());
        c.start("a", Tool::Trace, ip());
        let tools: Vec<Tool> = c.runs("a").iter().map(|r| r.tool).collect();
        assert_eq!(tools, [Tool::Trace, Tool::Ping]);
        // Pretend the ping finished with a result.
        if let Out::Ping(v) = &mut c.cards.get_mut("a").unwrap()[1].out {
            v.push(reply(1, Some(1.0)));
        }
        c.cards.get_mut("a").unwrap()[1].state = State::Done;
        // Rerunning the trace leaves the ping alone and keeps the order.
        c.start("a", Tool::Trace, ip());
        let r = c.runs("a");
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].tool, &r[0].state), (Tool::Trace, &State::Queued));
        assert_eq!(r[1].state, State::Done);
        assert!(matches!(&r[1].out, Out::Ping(v) if v.len() == 1));
        // Rerunning the ping starts it afresh.
        c.start("a", Tool::Ping, ip());
        assert!(matches!(&c.runs("a")[1].out, Out::Ping(v) if v.is_empty()));
    }

    #[test]
    fn runs_belong_to_their_card_and_clear_by_tool() {
        let mut c = CardRuns::default();
        c.start("a", Tool::Ping, ip());
        c.start("b", Tool::Ping, ip());
        assert!(c.active("a", Tool::Ping));
        assert!(!c.active("a", Tool::Ports));
        c.stop("a", Tool::Ping);
        assert_eq!(c.runs("a")[0].state, State::Stopped);
        assert!(c.active("b", Tool::Ping));
        c.clear("a", Tool::Ping);
        assert!(c.runs("a").is_empty());
        assert_eq!(c.runs("b").len(), 1);
    }

    #[test]
    fn at_most_three_run_at_once_and_the_rest_wait_in_order() {
        let mut c = CardRuns::default();
        for card in ["a", "b", "c", "d", "e"] {
            c.start(card, Tool::Ports, ip());
        }
        let due = c.due();
        let cards: Vec<&str> = due.iter().map(|d| d.0.as_str()).collect();
        assert_eq!(cards, ["a", "b", "c"]);
        for (card, tool) in due {
            c.find(&card, tool).unwrap().state = State::Running;
        }
        assert!(c.due().is_empty());
        c.find("b", Tool::Ports).unwrap().state = State::Done;
        assert_eq!(c.due(), [("d".to_string(), Tool::Ports)]);
        // A stopped one frees its place too.
        c.stop("a", Tool::Ports);
        assert_eq!(c.due().len(), 2);
    }

    #[test]
    fn summaries_say_queued_stopped_and_failed() {
        let mut r = Run::new(Tool::Ping, ip(), 1);
        assert_eq!(r.summary(), "Waiting for another run to finish");
        r.state = State::Stopped;
        r.apply(Ev::Reply(reply(1, Some(1.0))));
        assert_eq!(r.summary(), "1/1 replies · avg 1.0 ms · 0% loss · stopped");
        r.apply(Ev::Error("no permission".into()));
        assert_eq!(r.summary(), "no permission");
    }
}
