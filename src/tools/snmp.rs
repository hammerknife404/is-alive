//! The inspector's SNMP: one read of one device the technician picks, from
//! the Sweep inspector or a Map card (src/net/snmp.rs does the reading).
//! Never part of a sweep or Full scan.
//!
//! The community is a secret. It's typed here, kept in memory for this
//! session only and gone when the app closes: never in settings, Results,
//! the report or a bundle. Both inspectors share it and the readings, so a
//! device read from the Map shows the same in Sweep. Each finished read is
//! logged as an "SNMP" run in Results, without the community or sysContact
//! (a person, often: on screen only).
//!
//! Readings belong to the open site: a site switch clears them (`reset`),
//! and a read under way counts as a running tool, so the switch waits for
//! it. Carrying what's open into a new site keeps them.

use crate::inspector as insp;
use crate::job::Job;
use crate::log;
use crate::net::snmp::{self, Outcome};
use crate::{theme, widgets};
use eframe::egui::{self, Ui, vec2};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{LazyLock, Mutex};

/// The name its runs are logged under.
pub const TOOL: &str = "SNMP";
pub const COLUMNS: &[&str] = &["target", "section", "field", "value"];

/// What the button says it does.
pub const TIP: &str = "Ask this device over SNMP (UDP 161, v2c then v1) what it is: model, \
firmware (or software) version, serial, a printer's toner, the switch port it's plugged into. Uses the community in its \
SNMP section (public unless you change it). Only this device is asked, only when you press it.";

const COMMUNITY_TIP: &str = "The SNMP community (read-only). Kept in memory for this session \
only: never saved, logged, reported or put in a Site Bundle.";

struct State {
    community: String,
    /// The read under way: the device, when it started, the worker.
    running: Option<(IpAddr, String, Job<Outcome>)>,
    readings: HashMap<IpAddr, Outcome>,
    /// Devices whose SNMP section shows (asked once this session).
    open: HashSet<IpAddr>,
    /// Finished reads for Results: start, text, table.
    finished: Vec<(String, String, Option<log::Table>)>,
}

impl State {
    fn new() -> Self {
        State {
            community: "public".into(),
            running: None,
            readings: HashMap::new(),
            open: HashSet::new(),
            finished: Vec::new(),
        }
    }
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State::new()));

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    poll(&mut s);
    f(&mut s)
}

/// Take a finished read's outcome into the readings and the log.
fn poll(s: &mut State) {
    let Some((_, _, job)) = &s.running else {
        return;
    };
    let done = !job.running();
    let got: Vec<Outcome> = job.drain().collect();
    if !done && got.is_empty() {
        return;
    }
    let (ip, started, _) = s.running.take().expect("checked above");
    let out = got
        .into_iter()
        .last()
        .unwrap_or_else(|| Outcome::Failed(ip, "the read ended without a result".into()));
    s.finished.push((started, out.report(), Some(table(&out))));
    s.readings.insert(ip, out);
}

/// Results' table for one read: one row per value.
pub fn table(out: &Outcome) -> log::Table {
    let target = out.target().to_string();
    log::Table {
        columns: COLUMNS,
        rows: out
            .rows()
            .into_iter()
            .map(|(sec, field, value)| vec![target.clone(), sec.to_string(), field, value])
            .collect(),
    }
}

/// Results' one-line summary of a logged read, from its rows.
pub fn summary(rows: &[Vec<String>]) -> Option<String> {
    let get = |sec: &str, field: &str| {
        rows.iter()
            .find(|r| r.get(1).is_some_and(|s| s == sec) && r.get(2).is_some_and(|f| f == field))
            .and_then(|r| r.get(3))
            .filter(|v| !v.is_empty())
            .cloned()
    };
    if let Some(a) = get("snmp", "answer") {
        return Some(a);
    }
    let model = get("chassis", "model").or_else(|| get("system", "description"));
    let s = [get("system", "name"), model]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    (!s.is_empty()).then_some(s)
}

fn start_in(s: &mut State, ctx: &egui::Context, ip: IpAddr) {
    s.open.insert(ip);
    if s.running.is_some() || s.community.is_empty() {
        return;
    }
    // Moved into the worker; never written anywhere.
    let community = s.community.clone();
    let job = Job::spawn(ctx, move |tx| {
        tx.send(snmp::read(ip, community.as_bytes(), tx.cancel_flag()));
    });
    s.running = Some((ip, log::now(), job));
}

/// The inspector's SNMP button was pressed: read `ip` (one read at a time).
pub fn start(ctx: &egui::Context, ip: IpAddr) {
    with(|s| start_in(s, ctx, ip));
}

/// Whether the SNMP button can be pressed for `ip`, and its tip.
pub fn button(ip: IpAddr) -> (bool, String) {
    with(|s| button_in(s, ip))
}

fn button_in(s: &State, ip: IpAddr) -> (bool, String) {
    match &s.running {
        Some((on, ..)) if *on == ip => (false, "Reading it over SNMP…".to_string()),
        Some((on, ..)) => (
            false,
            format!("An SNMP read of {on} is under way: one device at a time"),
        ),
        // Enabled with an empty community too: pressing it opens the
        // section, where the community is typed (nothing is sent until then).
        None if s.community.is_empty() => (
            true,
            "Open its SNMP section: the community is empty, so type one there and press Read"
                .to_string(),
        ),
        None => (true, TIP.to_string()),
    }
}

/// Whether a read is under way (a site switch waits for it).
pub fn is_running() -> bool {
    with(|s| s.running.is_some())
}

/// A site switch: the last site's readings, open sections and unlogged
/// reads go, and a read under way is stopped. The community stays for the
/// session.
pub fn reset() {
    with(reset_in);
}

fn reset_in(s: &mut State) {
    if let Some((.., job)) = s.running.take() {
        job.stop();
    }
    s.readings.clear();
    s.open.clear();
    s.finished.clear();
}

/// A finished read waiting for Results: its start, text and table.
pub fn take_finished() -> Option<(String, String, Option<log::Table>)> {
    with(|s| (!s.finished.is_empty()).then(|| s.finished.remove(0)))
}

/// The read under way and what it has said so far, for Results' running list.
pub fn running_since() -> Option<(String, String)> {
    with(|s| {
        s.running
            .as_ref()
            .map(|(ip, started, _)| (started.clone(), format!("SNMP read of {ip}…\n")))
    })
}

/// `ip`'s SNMP section in an inspector, once its button has been pressed
/// this session: the community, Read again, and what came back.
pub fn section(ui: &mut Ui, ip: IpAddr) {
    with(|s| {
        let running_here = s.running.as_ref().is_some_and(|(on, ..)| *on == ip);
        if !s.open.contains(&ip) && !running_here && !s.readings.contains_key(&ip) {
            return;
        }
        insp::section(ui, "SNMP");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            ui.label("Community");
            ui.add(
                egui::TextEdit::singleline(&mut s.community)
                    .password(true)
                    .desired_width(120.0)
                    .min_size(vec2(0.0, theme::TARGET_H)),
            )
            .on_hover_text(COMMUNITY_TIP);
            if running_here {
                if ui
                    .add(egui::Button::new("Stop").min_size(vec2(56.0, theme::TARGET_H)))
                    .on_hover_text("Stop this read")
                    .clicked()
                    && let Some((.., job)) = &s.running
                {
                    job.stop();
                }
            } else {
                let busy = s.running.is_some();
                let label = if s.readings.contains_key(&ip) {
                    "Read again"
                } else {
                    "Read"
                };
                if ui
                    .add_enabled(
                        !busy && !s.community.is_empty(),
                        egui::Button::new(label).min_size(vec2(56.0, theme::TARGET_H)),
                    )
                    .on_hover_text("Read it again with this community")
                    .on_disabled_hover_text(if busy {
                        "Another SNMP read is under way: one device at a time"
                    } else {
                        "Type a community first"
                    })
                    .clicked()
                {
                    start_in(s, ui.ctx(), ip);
                }
            }
        });
        if s.running.as_ref().is_some_and(|(on, ..)| *on == ip) {
            ui.horizontal(|ui| {
                widgets::spinner(ui);
                ui.weak(format!(
                    "Reading over UDP 161 (up to {} s)…",
                    snmp::WHOLE.as_secs()
                ));
            });
            return;
        }
        let Some(out) = s.readings.get(&ip) else {
            return;
        };
        if let Some(p) = out.problem() {
            insp::note(ui, &p);
            return;
        }
        // With sysContact: on screen, never logged.
        let rows = out.screen_rows();
        for (sec, field, value) in rows.iter().skip(1) {
            let (k, v) = snmp::pair(sec, field, value);
            if *sec == "note" {
                insp::note(ui, &v);
            } else {
                insp::row(ui, &k, &v, *sec == "system" && field == "object_id");
            }
        }
        if let Outcome::Read(r) = out {
            insp::note(
                ui,
                &format!(
                    "Read over SNMP {} from the device itself; also in Results{}.",
                    r.version.name(),
                    if r.system.contact.is_some() {
                        ", without the contact"
                    } else {
                        ""
                    }
                ),
            );
        }
    });
}

/// The scene tour: a fictional printer's reading on `ip`, as if read.
pub fn sample_for_shots(ip: IpAddr) {
    with(|s| {
        s.open.insert(ip);
        s.readings
            .insert(ip, Outcome::Read(Box::new(snmp::sample(ip))));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logged_read_has_no_community_and_a_summary() {
        let ip: IpAddr = "192.0.2.40".parse().unwrap();
        let out = Outcome::Read(Box::new(snmp::sample(ip)));
        let t = table(&out);
        assert!(t.rows.iter().all(|r| r[0] == "192.0.2.40" && r.len() == 4));
        assert_eq!(
            summary(&t.rows).as_deref(),
            Some("PRN-FRONTDESK · HP LaserJet Pro M404")
        );
        let silent = table(&Outcome::Silent(ip));
        assert_eq!(summary(&silent.rows).as_deref(), Some("no answer"));
        // The community never reaches the log: it isn't in Outcome at all,
        // and the session's default doesn't leak into the text.
        assert!(!out.report().contains("public"));
    }

    #[test]
    fn an_empty_community_reads_nothing_from_either_button() {
        let ip: IpAddr = "192.0.2.41".parse().unwrap();
        let mut s = State::new();
        assert_eq!(button_in(&s, ip), (true, TIP.to_string()));
        s.community.clear();
        // Still pressable (else, with no section open anywhere, there'd be
        // nowhere to type one), and it says why nothing will be sent yet.
        let (ok, tip) = button_in(&s, ip);
        assert!(ok && tip.contains("community is empty"), "{tip}");
        // Pressed: the section opens, nothing is sent.
        start_in(&mut s, &egui::Context::default(), ip);
        assert!(s.open.contains(&ip) && s.running.is_none());
        // And after a site switch, still.
        reset_in(&mut s);
        assert!(button_in(&s, ip).0);
        start_in(&mut s, &egui::Context::default(), ip);
        assert!(s.open.contains(&ip) && s.running.is_none());
    }

    #[test]
    fn a_site_switch_forgets_the_last_sites_readings() {
        let ip: IpAddr = "192.168.1.40".parse().unwrap();
        let mut s = State::new();
        s.community = "example-ro".into();
        let out = Outcome::Read(Box::new(snmp::sample(ip)));
        s.open.insert(ip);
        s.finished
            .push((log::now(), out.report(), Some(table(&out))));
        s.readings.insert(ip, out);
        // A read under way is stopped, not logged into the next site.
        let job = Job::spawn(&egui::Context::default(), move |tx| {
            while !tx.cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            tx.send(Outcome::Stopped(ip));
        });
        s.running = Some((ip, log::now(), job));
        reset_in(&mut s);
        assert!(s.readings.is_empty() && s.open.is_empty() && s.finished.is_empty());
        assert!(s.running.is_none());
        // The community is the session's, not the site's.
        assert_eq!(s.community, "example-ro");
    }
}
