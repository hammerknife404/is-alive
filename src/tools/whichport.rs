//! "Which port am I on?", from the status bar's Port item: listen on the
//! wired adapter for the switch's LLDP or CDP announcement (src/net/listen.rs),
//! or ask a switch over SNMP which of its ports has this computer's MAC.
//! Listening sends nothing; it waits up to `listen::LISTEN` and stops at
//! the first announcement. Each attempt is logged as a Switch port run in
//! Results, and what was heard becomes the scanner's record in a bundle.

use crate::inspector as insp;
use crate::job::Job;
use crate::log::{self, Table};
use crate::net::lldp::{Heard, Neighbor};
use crate::net::{listen, netconf, snmp};
use crate::{theme, widgets};
use eframe::egui::{self, Ui, vec2};
use std::net::{IpAddr, Ipv4Addr};
use std::time::Instant;

/// The name its runs are logged under, and their columns.
pub const TOOL: &str = "Switch port";
pub const COLUMNS: &[&str] = &["field", "value"];

pub const TIP: &str = "Which port am I on? Listens on the wired adapter for the switch's own \
announcement (LLDP or CDP, every 30 to 60 s): its name, the port, VLANs and management address. \
Sends nothing.";

enum Msg {
    Status(String),
    Listened(listen::Outcome),
    Asked(snmp::Lookup),
}

/// The last attempt, as shown.
#[derive(Clone, Debug, PartialEq)]
pub enum Last {
    Listened(listen::Outcome),
    Asked(snmp::Lookup),
}

#[derive(Default)]
pub struct WhichPort {
    job: Option<Job<Msg>>,
    /// When the attempt under way started: for Results, and the countdown.
    started: Option<(String, Instant)>,
    /// Listening (true) or asking over SNMP.
    listening: bool,
    status: String,
    last: Option<Last>,
    /// A run waiting for Results: start, text, table.
    finished: Option<(String, String, Option<Table>)>,
    /// Something new was heard, for the scanner's record.
    heard: Option<Heard>,
    /// The switch to ask over SNMP.
    pub switch_ip: String,
    /// The scene tour opens the popup.
    pub shots_open: bool,
}

impl WhichPort {
    pub fn listen(&mut self, ctx: &egui::Context, lan: Option<Ipv4Addr>) {
        if self.job.is_some() {
            return;
        }
        self.start(true);
        self.job = Some(Job::spawn(ctx, move |tx| {
            let status = |s: String| tx.send(Msg::Status(s));
            let out = listen::listen(lan, tx.cancel_flag(), &status);
            tx.send(Msg::Listened(out));
        }));
    }

    fn start(&mut self, listening: bool) {
        self.listening = listening;
        self.status.clear();
        self.started = Some((log::now(), Instant::now()));
    }

    /// Ask `ip` over SNMP which port has the wired adapter's MAC.
    fn ask(&mut self, ctx: &egui::Context, ip: IpAddr, mac: [u8; 6]) {
        let Some(community) = crate::tools::snmp::community().filter(|c| !c.is_empty()) else {
            self.last = Some(Last::Asked(snmp::Lookup::Failed(
                "type a community first".into(),
            )));
            return;
        };
        self.start(false);
        self.job = Some(Job::spawn(ctx, move |tx| {
            tx.send(Msg::Asked(snmp::find_mac(
                ip,
                community.as_bytes(),
                mac,
                tx.cancel_flag(),
            )));
        }));
    }

    /// Drain the worker; true while it runs.
    pub fn poll(&mut self) -> bool {
        let Some(job) = &self.job else {
            return false;
        };
        let running = job.running();
        let got: Vec<Msg> = job.drain().collect();
        for m in got {
            match m {
                Msg::Status(s) => self.status = s,
                Msg::Listened(o) => self.done(Last::Listened(o)),
                Msg::Asked(l) => self.done(Last::Asked(l)),
            }
        }
        if !running && self.job.is_some() {
            self.job = None;
            // Ended without a word (it can't, but say so rather than spin).
            if let Some((started, _)) = self.started.take() {
                self.finished = Some((started, "(no results)".into(), None));
            }
        }
        self.job.is_some()
    }

    fn done(&mut self, last: Last) {
        self.job = None;
        let Some((started, _)) = self.started.take() else {
            return;
        };
        let (text, rows) = record(&last);
        if let Some(h) = heard(&last) {
            self.heard = Some(h);
        }
        self.finished = Some((
            started,
            text,
            Some(Table {
                columns: COLUMNS,
                rows,
            }),
        ));
        self.last = Some(last);
    }

    pub fn running_since(&self) -> Option<&str> {
        self.job
            .as_ref()
            .and(self.started.as_ref())
            .map(|(s, _)| s.as_str())
    }

    /// For Results' running list.
    pub fn output(&self) -> Option<String> {
        self.job.as_ref()?;
        Some(format!("{}\n", self.progress()))
    }

    /// "Listening on eth0 (raw socket): 42 s left".
    fn progress(&self) -> String {
        if !self.listening {
            return "Asking the switch over SNMP…".into();
        }
        let left = self.started.as_ref().map_or(0, |(_, at)| {
            listen::LISTEN.saturating_sub(at.elapsed()).as_secs()
        });
        let what = if self.status.is_empty() {
            "Listening"
        } else {
            &self.status
        };
        format!("{what}: {left} s left")
    }

    pub fn take_finished(&mut self) -> Option<(String, String, Option<Table>)> {
        self.finished.take()
    }

    /// Something newly heard, for the scanner's record, once.
    pub fn take_heard(&mut self) -> Option<Heard> {
        self.heard.take()
    }

    /// A site switch: what was shown belongs to the last site.
    pub fn reset(&mut self) {
        if let Some(j) = &self.job {
            j.stop();
        }
        self.last = None;
    }

    /// The status bar's words: what's known, or the question.
    pub fn bar_text(&self, known: Option<&Heard>) -> String {
        if self.job.is_some() {
            if self.listening {
                let left = self.started.as_ref().map_or(0, |(_, at)| {
                    listen::LISTEN.saturating_sub(at.elapsed()).as_secs()
                });
                return format!("Port: listening {left} s");
            }
            return "Port: asking…".into();
        }
        match known.and_then(|h| h.n.port.clone()) {
            Some(p) => format!("Port {}", cut(&p, 18)),
            None => "Which port?".into(),
        }
    }

    /// The popup's contents.
    pub fn ui(&mut self, ui: &mut Ui, lan: Option<Ipv4Addr>, known: Option<&Heard>) {
        ui.set_width(340.0);
        ui.strong("Which port am I on?");
        ui.weak(
            "Listens on the wired adapter for the switch's own announcement (LLDP or CDP, every \
             30 to 60 s). Sends nothing.",
        );
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            if self.job.is_some() {
                if ui
                    .add(egui::Button::new("Stop").min_size(vec2(56.0, theme::TARGET_H)))
                    .clicked()
                    && let Some(j) = &self.job
                {
                    j.stop();
                }
                widgets::spinner(ui);
                ui.label(self.progress());
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(500));
            } else {
                let label = if self.last.is_some() || known.is_some() {
                    "Listen again"
                } else {
                    "Listen"
                };
                if ui
                    .add(widgets::primary(label).min_size(vec2(56.0, theme::TARGET_H)))
                    .on_hover_text(format!(
                        "Listen up to {} s, stopping at the first announcement. Logged in Results.",
                        listen::LISTEN.as_secs()
                    ))
                    .clicked()
                {
                    self.listen(ui.ctx(), lan);
                }
            }
        });
        // What the last attempt here said; else what the site last heard.
        match (&self.last, known) {
            (Some(l), _) => show_last(ui, l),
            (None, Some(h)) => {
                show_heard(ui, h);
                insp::note(ui, "Heard earlier at this site (in Results).");
            }
            (None, None) => {}
        }
        if self.job.is_none() {
            self.snmp_ui(ui, lan);
        }
    }

    /// Asking a switch over SNMP: offered once listening didn't name the port.
    fn snmp_ui(&mut self, ui: &mut Ui, lan: Option<Ipv4Addr>) {
        let heard = matches!(
            self.last,
            Some(Last::Listened(listen::Outcome::Heard { .. }))
                | Some(Last::Asked(snmp::Lookup::Found(_)))
        );
        if self.last.is_none() || heard {
            return;
        }
        insp::section(ui, "Ask the switch over SNMP");
        ui.weak(
            "If you know a switch's address: asks it (UDP 161) which of its ports has this \
             computer's wired MAC. Uses the community from the SNMP section.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.switch_ip)
                    .hint_text("switch address")
                    .desired_width(130.0)
                    .min_size(vec2(0.0, theme::TARGET_H)),
            );
            crate::tools::snmp::community_edit(ui);
            let ip: Option<IpAddr> = self.switch_ip.trim().parse().ok();
            if ui
                .add_enabled(
                    ip.is_some(),
                    egui::Button::new("Ask").min_size(vec2(56.0, theme::TARGET_H)),
                )
                .on_disabled_hover_text("Type the switch's IP address")
                .clicked()
                && let Some(ip) = ip
            {
                // The MAC the listen saw, else the wired adapter's now.
                let mac = match &self.last {
                    Some(Last::Listened(o)) => o.mac(),
                    _ => None,
                }
                .or_else(|| {
                    let all = netconf::wired(lan);
                    netconf::pick_wired(&all).and_then(|w| w.mac)
                });
                match mac {
                    Some(mac) => self.ask(ui.ctx(), ip, mac),
                    None => {
                        self.last = Some(Last::Asked(snmp::Lookup::Failed(
                            "no wired adapter with a MAC to ask about".into(),
                        )));
                    }
                }
            }
        });
    }
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n - 1).collect::<String>() + "…"
    }
}

/// What an attempt found, as the scanner's record.
fn heard(last: &Last) -> Option<Heard> {
    match last {
        Last::Listened(listen::Outcome::Heard {
            n, adapter, via, ..
        }) => Some(Heard {
            n: (**n).clone(),
            via: (*via).to_string(),
            adapter: adapter.clone(),
        }),
        Last::Asked(snmp::Lookup::Found(l)) => Some(Heard {
            n: Neighbor {
                proto: "SNMP",
                switch: l.switch.clone(),
                port: Some(l.port.clone()),
                port_desc: l.port_alias.clone(),
                mgmt: Some(l.target),
                ..Default::default()
            },
            via: "SNMP".into(),
            adapter: String::new(),
        }),
        _ => None,
    }
}

/// The words for an attempt that named no port.
fn problem(last: &Last) -> Option<String> {
    Some(match last {
        Last::Listened(o) => match o {
            listen::Outcome::Heard { .. } => return None,
            listen::Outcome::Silent { adapter, secs, .. } => listen::silent_text(adapter, *secs),
            listen::Outcome::NoWire(s) => s.clone(),
            listen::Outcome::Cannot { fix, .. } => fix.clone(),
            listen::Outcome::Stopped => "Stopped before anything was heard.".into(),
        },
        Last::Asked(l) => match l {
            snmp::Lookup::Found(_) => return None,
            snmp::Lookup::NotLearned { target, switch } => format!(
                "{} answered, and its forwarding table doesn't have this computer's MAC: it's \
                 plugged in elsewhere, or the switch hasn't seen it send yet.",
                switch
                    .as_deref()
                    .map_or(target.to_string(), |s| format!("{s} ({target})"))
            ),
            snmp::Lookup::Failed(e) => format!("SNMP: {e}."),
        },
    })
}

/// Results' text and rows for an attempt.
fn record(last: &Last) -> (String, Vec<Vec<String>>) {
    if let Some(h) = heard(last) {
        let mut text: String =
            h.n.fields()
                .iter()
                .map(|(_, l, v)| format!("{l:<11}{v}\n"))
                .collect();
        text += &match last {
            Last::Listened(listen::Outcome::Heard { secs, .. }) => format!(
                "Heard over {} on {} ({}) after {secs} s.\n",
                h.n.proto, h.adapter, h.via
            ),
            _ => "From the switch's forwarding table over SNMP: if this is its uplink to another \
                  switch, ask that one.\n"
                .into(),
        };
        return (text, h.rows());
    }
    let p = problem(last).unwrap_or_default();
    let result = match last {
        Last::Listened(listen::Outcome::Silent { .. }) => "nothing heard",
        Last::Listened(listen::Outcome::NoWire(_)) => "no wired link",
        Last::Listened(listen::Outcome::Cannot { .. }) => "can't listen",
        Last::Listened(_) => "stopped",
        Last::Asked(snmp::Lookup::NotLearned { .. }) => "not learned",
        Last::Asked(_) => "failed",
    };
    (
        format!("{p}\n"),
        vec![
            vec!["result".into(), result.into()],
            vec!["detail".into(), p.replace('\n', " ")],
        ],
    )
}

fn show_heard(ui: &mut Ui, h: &Heard) {
    for (k, l, v) in h.n.fields() {
        let mono = matches!(
            k,
            "port" | "management" | "chassis_id" | "vlan" | "voice_vlan"
        );
        insp::row(ui, l, &v, mono);
    }
}

fn show_last(ui: &mut Ui, last: &Last) {
    if let Some(h) = heard(last) {
        show_heard(ui, &h);
        let how = match last {
            Last::Listened(listen::Outcome::Heard { secs, .. }) => format!(
                "Heard over {} on {} ({}) after {secs} s; also in Results.",
                h.n.proto, h.adapter, h.via
            ),
            _ => "From the switch's forwarding table over SNMP; also in Results. If this is its \
                  uplink to another switch, ask that one."
                .into(),
        };
        insp::note(ui, &how);
    } else if let Some(p) = problem(last) {
        insp::note(ui, &p);
    }
}

/// Results' few words for a run.
pub fn summary(rows: &[Vec<String>]) -> String {
    if let Some(h) = Heard::from_rows(rows) {
        return h.n.short();
    }
    rows.iter()
        .find(|r| r.first().is_some_and(|f| f == "result"))
        .and_then(|r| r.get(1).cloned())
        .unwrap_or_default()
}

/// The newest Switch port run that named a port, for the scanner's record.
pub fn from_log(l: &log::Log) -> Option<Heard> {
    l.entries
        .iter()
        .rev()
        .filter(|e| e.tool == TOOL)
        .find_map(|e| e.table.as_ref().and_then(|t| Heard::from_rows(&t.rows)))
}

/// The scene tour: a fictional switch heard on a fictional adapter.
pub fn sample_for_shots(p: &mut WhichPort) {
    let n = Neighbor {
        proto: "LLDP",
        switch: Some("sw-example-01".into()),
        chassis: Some("02:00:5e:10:00:20".into()),
        port: Some("Gi1/0/12".into()),
        port_desc: Some("Room 2.14 desk B".into()),
        model: Some("Example Switch 2400".into()),
        mgmt: Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10))),
        vlan: Some(20),
        voice_vlan: Some(30),
        vlan_names: vec![(20, "Staff".into()), (30, "Voice".into())],
        poe: Some("switch supplies power, on, class 4, 25.5 W allocated".into()),
    };
    p.start(true);
    p.done(Last::Listened(listen::Outcome::Heard {
        n: Box::new(n),
        adapter: "eth0".into(),
        via: "raw socket",
        secs: 23,
        mac: None,
    }));
    p.shots_open = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heard_switch_is_logged_and_kept() {
        let mut p = WhichPort::default();
        sample_for_shots(&mut p);
        let (_, text, table) = p.take_finished().expect("a run");
        let rows = table.unwrap().rows;
        assert!(text.contains("Gi1/0/12"), "{text}");
        assert!(text.contains("after 23 s"), "{text}");
        assert_eq!(summary(&rows), "Gi1/0/12 on sw-example-01");
        let h = p.take_heard().expect("kept for the scanner");
        assert_eq!(Heard::from_rows(&rows), Some(h));
        assert_eq!(p.take_heard(), None);
        assert_eq!(p.bar_text(None), "Which port?");
    }

    #[test]
    fn nothing_heard_is_logged_with_the_reason() {
        let mut p = WhichPort::default();
        p.start(true);
        p.done(Last::Listened(listen::Outcome::Cannot {
            adapter: "eth0".into(),
            fix: "install Npcap".into(),
            mac: None,
        }));
        let (_, text, table) = p.take_finished().unwrap();
        let rows = table.unwrap().rows;
        assert_eq!(summary(&rows), "can't listen");
        assert!(text.contains("install Npcap"));
        assert_eq!(p.take_heard(), None);
        // SNMP that found it is a port too.
        p.start(false);
        p.done(Last::Asked(snmp::Lookup::Found(snmp::Learned {
            target: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
            switch: Some("sw-example-01".into()),
            port: "Gi1/0/12".into(),
            port_alias: None,
            fdb: Some(20),
        })));
        let (_, _, table) = p.take_finished().unwrap();
        assert_eq!(summary(&table.unwrap().rows), "Gi1/0/12 on sw-example-01");
        assert_eq!(p.take_heard().unwrap().via, "SNMP");
    }
}
