//! Network checks, run by Full scan: whether HTTPS is inspected on the
//! way out, and whether this computer's clock is right. No tab of its own:
//! the run is logged to Results like the others.

use crate::job::{Job, pump};
use crate::log::{self, Table};
use crate::net::checks::{self, Item, Status};
use eframe::egui;
use std::time::Duration;

/// Results table columns.
pub const COLUMNS: &[&str] = &["check", "status", "detail"];

/// How long each site or time server gets.
const WAIT: Duration = Duration::from_secs(4);

#[derive(Default)]
pub struct NetCheck {
    job: Option<Job<Item>>,
    run_started: Option<String>,
    items: Vec<Item>,
}

impl NetCheck {
    /// Start both checks; a run already going is replaced.
    pub fn run(&mut self, ctx: &egui::Context) {
        self.items.clear();
        self.run_started = Some(log::now());
        self.job = Some(Job::spawn(ctx, move |tx| {
            let seen: Vec<Result<checks::Seen, String>> = std::thread::scope(|s| {
                let hs: Vec<_> = checks::SITES
                    .iter()
                    .map(|site| s.spawn(move || checks::present(site, WAIT)))
                    .collect();
                hs.into_iter()
                    .map(|h| h.join().unwrap_or_else(|_| Err("check failed".into())))
                    .collect()
            });
            tx.send(checks::judge_https(&seen));
            if tx.cancelled() {
                return;
            }
            let off = checks::offset(WAIT);
            let source = crate::net::clock::synced_to();
            tx.send(checks::judge_clock(off.as_ref(), source.as_deref()));
        }));
    }

    /// Drain the worker's verdicts; true while it runs.
    pub fn poll(&mut self) -> bool {
        pump(&mut self.job, |item| self.items.push(item))
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
            ("HTTPS inspection", _) => "HTTPS inspected".to_string(),
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
            0 => "HTTPS and clock ok".into(),
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
            "HTTPS and clock ok"
        );
        assert_eq!(summary(&[row("Clock", "fail")]), "clock way off");
        assert_eq!(
            summary(&[row("HTTPS inspection", "unknown"), row("Clock", "unknown")]),
            "couldn't check"
        );
    }
}
