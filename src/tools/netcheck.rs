//! Network checks, run by Full scan: whether a captive portal answers in
//! the internet's place, whether HTTPS is inspected on the way out,
//! whether this computer's clock is right, the path MTU to the internet,
//! and a Wi-Fi snapshot with the access points nearby when the way out is
//! Wi-Fi. No tab of its own: the run is logged to Results like the others.
//!
//! And Scan Wi-Fi, from a Network checks run in Results: an active scan,
//! logged as a Wi-Fi scan run of its own. Never part of Full scan: it makes
//! the radio hop channels, which would disturb the probes running then.

use crate::job::{Job, pump};
use crate::log::{self, Table};
use crate::net::checks::{self, Item, Portal, Status};
use crate::net::{netconf, pmtu, wifi};
use eframe::egui;
use std::time::Duration;

/// Results table columns.
pub const COLUMNS: &[&str] = &["check", "status", "detail"];

/// What Scan Wi-Fi logs under, and its table's columns: one row per
/// access point, strongest first; `connected` is "yes" on the one this
/// computer is on.
pub const SCAN: &str = "Wi-Fi scan";
pub const SCAN_COLUMNS: &[&str] = &[
    "ssid",
    "bssid",
    "band",
    "channel",
    "signal_dbm",
    "signal_pct",
    "connected",
];

/// A finished Wi-Fi scan: its text and table rows.
struct Scanned {
    text: String,
    rows: Vec<Vec<String>>,
}

/// How long each site or time server gets.
const WAIT: Duration = Duration::from_secs(4);

#[derive(Default)]
pub struct NetCheck {
    job: Option<Job<Item>>,
    run_started: Option<String>,
    items: Vec<Item>,
    scan: Option<Job<Scanned>>,
    scan_started: Option<String>,
    scanned: Option<Scanned>,
}

impl NetCheck {
    /// Start the checks; a run already going is replaced.
    pub fn run(&mut self, ctx: &egui::Context) {
        self.items.clear();
        self.run_started = Some(log::now());
        self.job = Some(Job::spawn(ctx, move |tx| {
            // The portal first-hand, alongside the sites: HTTPS is judged
            // knowing whether a portal answered.
            let (portal, seen) = std::thread::scope(|s| {
                let p = s.spawn(|| checks::portal(WAIT));
                let hs: Vec<_> = checks::SITES
                    .iter()
                    .map(|site| s.spawn(move || checks::present(site, WAIT)))
                    .collect();
                let seen: Vec<Result<checks::Seen, String>> = hs
                    .into_iter()
                    .map(|h| h.join().unwrap_or_else(|_| Err("check failed".into())))
                    .collect();
                let portal = p
                    .join()
                    .unwrap_or_else(|_| Portal::Unreached("check failed".into()));
                (portal, seen)
            });
            tx.send(checks::judge_portal(&portal));
            tx.send(checks::judge_https(&seen, &portal));
            if tx.cancelled() {
                return;
            }
            // The clock, the path MTU (a few seconds: retries) and the
            // Wi-Fi snapshot at once, reported in that order.
            let route = netconf::adapter_to(pmtu::TARGET);
            std::thread::scope(|s| {
                let stop = || tx.cancelled();
                let mtu = s.spawn(move || pmtu::measure(&stop));
                // The list of access points the system already has: no scan.
                let air = route
                    .as_ref()
                    .filter(|a| a.wifi)
                    .map(|a| s.spawn(move || (wifi::snapshot(a), wifi::nearby(a, false))));
                let off = checks::offset(WAIT);
                let source = crate::net::clock::synced_to();
                tx.send(checks::judge_clock(off.as_ref(), source.as_deref()));
                let vpn = route.as_ref().filter(|a| a.vpn).map(|a| a.name.as_str());
                if let Ok((base, sizes)) = mtu.join() {
                    tx.send(pmtu::judge(&base, &sizes, vpn));
                }
                if let (Some(h), Some(a)) = (air, &route)
                    && let Ok((w, near)) = h.join()
                {
                    tx.send(wifi::judge(&w, &a.name));
                    tx.send(wifi::judge_nearby(&near, linked(&w)));
                }
            });
        }));
    }

    /// Drain the worker's verdicts; true while it runs.
    pub fn poll(&mut self) -> bool {
        let scanning = pump(&mut self.scan, |s| self.scanned = Some(s));
        pump(&mut self.job, |item| self.items.push(item)) || scanning
    }

    /// Scan Wi-Fi: ask the adapter to scan, then list what it heard. A
    /// scan already going is replaced.
    pub fn scan_wifi(&mut self, ctx: &egui::Context) {
        self.scanned = None;
        self.scan_started = Some(log::now());
        self.scan = Some(Job::spawn(ctx, move |tx| {
            let route = netconf::adapter_to(pmtu::TARGET);
            let Some(a) = route.filter(|a| a.wifi) else {
                tx.send(Scanned {
                    text: "Not scanned: the way out isn't over Wi-Fi now.\n".into(),
                    rows: Vec::new(),
                });
                return;
            };
            let w = wifi::snapshot(&a);
            let near = wifi::nearby(&a, true);
            if tx.cancelled() {
                return;
            }
            let here = linked(&w);
            let mut text = self::text(&[wifi::judge_nearby(&near, here)]);
            let rows = match &near {
                wifi::Neighbours::Listed(n) => wifi::rows(n, here),
                _ => Vec::new(),
            };
            for r in &rows {
                let signal = match (r[4].as_str(), r[5].as_str()) {
                    ("", "") => String::new(),
                    ("", pct) => format!("{pct}%"),
                    (dbm, _) => format!("{dbm} dBm"),
                };
                let ssid = if r[0].is_empty() { "(hidden)" } else { &r[0] };
                let here = if r[6] == "yes" { "  ← connected" } else { "" };
                text += &format!(
                    "{ssid:<32} {}  {:>7} ch {:>3}  {signal:>8}{here}\n",
                    r[1], r[2], r[3]
                );
            }
            tx.send(Scanned { text, rows });
        }));
    }

    /// Start time of the Wi-Fi scan in progress.
    pub fn scan_running_since(&self) -> Option<&str> {
        self.scan.as_ref().and(self.scan_started.as_deref())
    }

    /// Start time, logged text and table of a Wi-Fi scan that has just
    /// ended, once.
    pub fn take_finished_scan(&mut self) -> Option<(String, String, Option<Table>)> {
        if self.scan.is_some() {
            return None;
        }
        let started = self.scan_started.take()?;
        let done = self.scanned.take();
        let (text, rows) = done.map_or((None, Vec::new()), |d| (Some(d.text), d.rows));
        Some((
            started,
            log::body(text, &None),
            Some(Table {
                columns: SCAN_COLUMNS,
                rows,
            }),
        ))
    }

    /// Start time of the run in progress.
    pub fn running_since(&self) -> Option<&str> {
        self.job.as_ref().and(self.run_started.as_deref())
    }

    /// The verdicts so far, one line each.
    pub fn output(&self) -> Option<String> {
        (!self.items.is_empty()).then(|| text(&self.items))
    }

    /// Start time, logged text and table of a run that has just ended, once.
    pub fn take_finished(&mut self) -> Option<(String, String, Option<Table>)> {
        if self.job.is_some() {
            return None;
        }
        let started = self.run_started.take()?;
        let rows = self
            .items
            .iter()
            .map(|i| {
                vec![
                    i.check.to_string(),
                    i.status.tag().to_string(),
                    i.detail.clone(),
                ]
            })
            .collect();
        Some((
            started,
            log::body(self.output(), &None),
            Some(Table {
                columns: COLUMNS,
                rows,
            }),
        ))
    }
}

/// A made-up run on Wi-Fi for the scene tour, through the real verdicts:
/// fictional names, documentation MACs.
pub fn sample_for_shots() -> (String, String, Option<Table>) {
    let here = wifi::Snapshot {
        ssid: Some("Example Guest".into()),
        bssid: Some("02:00:5e:10:00:01".into()),
        freq_mhz: Some(5180),
        signal_dbm: Some(-58),
        width_mhz: Some(80),
        tx_mbps: Some(780.0),
        rx_mbps: Some(866.7),
        ..Default::default()
    };
    let ap = |ssid: &str, n: u8, freq: u32, dbm: i32| wifi::Bss {
        ssid: Some(ssid.into()),
        bssid: format!("02:00:5e:10:00:{n:02x}"),
        freq_mhz: Some(freq),
        channel: wifi::band_channel(freq).map(|(_, c)| c),
        signal_dbm: Some(dbm),
        signal_pct: None,
    };
    let near = wifi::Neighbours::Listed(wifi::Nearby {
        aps: vec![
            ap("Example Guest", 1, 5180, -58),
            ap("Example Staff", 2, 5180, -64),
            ap("Neighbour", 3, 5180, -77),
            ap("Example Staff", 4, 5745, -70),
        ],
        age_secs: Some(12),
        scanned: false,
    });
    let sizes = pmtu::search(
        |s| {
            if s > 1492 {
                pmtu::Probe::TooBig {
                    mtu: Some(1492),
                    from: Some("192.0.2.1".parse().expect("an address")),
                }
            } else {
                pmtu::Probe::Through
            }
        },
        &|| false,
    );
    let mut t = NetCheck {
        run_started: Some(log::now()),
        items: vec![
            pmtu::judge(&pmtu::Probe::Through, &sizes, None),
            wifi::judge(&wifi::Wifi::Connected(here.clone()), "Wi-Fi"),
            wifi::judge_nearby(&near, Some(&here)),
        ],
        ..Default::default()
    };
    t.take_finished().expect("a finished run")
}

/// The access point a snapshot says this computer is on, when it says.
fn linked(w: &wifi::Wifi) -> Option<&wifi::Snapshot> {
    match w {
        wifi::Wifi::Connected(s) | wifi::Wifi::Denied(s) => Some(s),
        _ => None,
    }
}

/// Whether a Network checks run looked at Wi-Fi: where Scan Wi-Fi is
/// offered.
pub fn on_wifi(rows: &[Vec<String>]) -> bool {
    rows.iter().any(|r| {
        r.first()
            .is_some_and(|c| c == "Wi-Fi" || c == "Nearby Wi-Fi")
    })
}

/// Results' few words for a Wi-Fi scan: "14 access points · 3 on channel
/// 36", the channel being the connected one's.
pub fn scan_summary(rows: &[Vec<String>]) -> String {
    let n = rows.len();
    let mut s = format!("{n} access point{}", if n == 1 { "" } else { "s" });
    let mine = rows.iter().find(|r| r.get(6).is_some_and(|c| c == "yes"));
    if let Some(m) = mine.filter(|m| m.get(3).is_some_and(|c| !c.is_empty())) {
        let co = rows
            .iter()
            .filter(|r| {
                r.get(6).is_none_or(|c| c != "yes") && r.get(2) == m.get(2) && r.get(3) == m.get(3)
            })
            .count();
        s += &format!(" · {co} more on channel {}", m[3]);
    }
    s
}

/// "[WARN] HTTPS inspection: …" per check.
fn text(items: &[Item]) -> String {
    items
        .iter()
        .map(|i| {
            let tag = match i.status {
                Status::Ok => "OK",
                Status::Warn => "WARN",
                Status::Fail => "FAIL",
                Status::Unknown => "--",
            };
            format!("[{tag:>4}] {}: {}\n", i.check, i.detail)
        })
        .collect()
}

/// Results' few words for a run: what's wrong, else "all ok".
pub fn summary(rows: &[Vec<String>]) -> String {
    let bad: Vec<String> = rows
        .iter()
        .filter(|r| r.get(1).is_some_and(|s| s == "warn" || s == "fail"))
        .map(|r| match (r[0].as_str(), r[1].as_str()) {
            ("Captive portal", _) => "captive portal".to_string(),
            ("HTTPS inspection", _) => "HTTPS inspected".to_string(),
            ("Path MTU", _) => "MTU black hole".to_string(),
            ("Wi-Fi", _) if r.get(2).is_some_and(|d| d.contains("weak:")) => {
                "weak Wi-Fi".to_string()
            }
            ("Wi-Fi", _) => "fair Wi-Fi".to_string(),
            ("Clock", "fail") => "clock way off".to_string(),
            ("Clock", _) => "clock drifting".to_string(),
            (c, s) => format!("{c} {s}"),
        })
        .collect();
    if bad.is_empty() {
        let unknown = rows
            .iter()
            .filter(|r| r.get(1).is_some_and(|s| s == "unknown"));
        match unknown.count() {
            0 => "all ok".into(),
            n if n == rows.len() => "couldn't check".into(),
            _ => "ok where checked".into(),
        }
    } else {
        bad.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_reads_as_a_table_and_a_few_words() {
        let mut t = NetCheck {
            run_started: Some("2026-11-11 07:00:00".into()),
            items: vec![
                Item {
                    check: "HTTPS inspection",
                    status: Status::Warn,
                    detail: "HTTPS is inspected here".into(),
                },
                Item {
                    check: "Clock",
                    status: Status::Ok,
                    detail: "under a second".into(),
                },
            ],
            ..Default::default()
        };
        let (started, text, table) = t.take_finished().unwrap();
        assert_eq!(started, "2026-11-11 07:00:00");
        assert!(text.starts_with("[WARN] HTTPS inspection: HTTPS is inspected here\n"));
        let table = table.unwrap();
        assert_eq!(table.rows[1], ["Clock", "ok", "under a second"]);
        assert_eq!(summary(&table.rows), "HTTPS inspected");
        assert!(t.take_finished().is_none(), "once");
        let row = |c: &str, s: &str| vec![c.to_string(), s.to_string(), String::new()];
        assert_eq!(
            summary(&[row("HTTPS inspection", "ok"), row("Clock", "ok")]),
            "all ok"
        );
        assert_eq!(
            summary(&[
                row("Captive portal", "fail"),
                row("HTTPS inspection", "unknown"),
                row("Path MTU", "warn")
            ]),
            "captive portal · MTU black hole"
        );
        let wifi = |d: &str| vec!["Wi-Fi".to_string(), "warn".into(), d.to_string()];
        assert_eq!(
            summary(&[wifi("signal -79 dBm (weak: expect drops")]),
            "weak Wi-Fi"
        );
        assert_eq!(
            summary(&[wifi("signal -71 dBm (fair: fine for")]),
            "fair Wi-Fi"
        );
        assert_eq!(summary(&[row("Clock", "fail")]), "clock way off");
        // Nearby Wi-Fi is reported, never a problem in the few words.
        assert_eq!(
            summary(&[vec![
                "Nearby Wi-Fi".into(),
                "ok".into(),
                "9 access points".into()
            ]]),
            "all ok"
        );
        assert!(on_wifi(&[vec![
            "Nearby Wi-Fi".into(),
            "ok".into(),
            String::new()
        ]]));
        assert!(!on_wifi(&[row("Clock", "ok")]));
        // A Wi-Fi scan: how many, and how many share the connected one's
        // channel.
        let ap = |band: &str, ch: &str, here: &str| {
            ["x", "02:00:5e:10:00:01", band, ch, "-60", "", here]
                .map(String::from)
                .to_vec()
        };
        let rows = [
            ap("5 GHz", "36", ""),
            ap("5 GHz", "36", "yes"),
            ap("5 GHz", "149", ""),
            ap("2.4 GHz", "36", ""),
        ];
        assert_eq!(
            scan_summary(&rows),
            "4 access points · 1 more on channel 36"
        );
        assert_eq!(scan_summary(&rows[2..3]), "1 access point");
        assert_eq!(
            summary(&[row("HTTPS inspection", "unknown"), row("Clock", "unknown")]),
            "couldn't check"
        );
    }
}
