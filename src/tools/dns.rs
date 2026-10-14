use crate::job::{Job, pump};
use crate::net;
use crate::widgets::{self, Action};
use eframe::egui::{self, RichText, Ui};
use std::net::IpAddr;
use std::time::Instant;

enum Ev {
    Record(String, String),
    Done(f64),
    Error(String),
}

#[derive(Default)]
pub struct DnsTool {
    query: String,
    job: Option<Job<Ev>>,
    records: Vec<(String, String)>,
    took: Option<f64>,
    err: Option<String>,
}

impl DnsTool {
    pub fn ui(&mut self, ui: &mut Ui) {
        let running = pump(&mut self.job, |ev| match ev {
            Ev::Record(k, v) => self.records.push((k, v)),
            Ev::Done(ms) => self.took = Some(ms),
            Ev::Error(e) => self.err = Some(e),
        });

        let mut enter = false;
        ui.horizontal(|ui| {
            enter |= widgets::field(
                ui,
                "Name or IP",
                &mut self.query,
                260.0,
                "example.com or 8.8.8.8",
            );
            ui.separator();
            match widgets::run_button(ui, running, enter) {
                Action::Start => self.start(ui.ctx()),
                Action::Stop => self.job.as_ref().map_or((), Job::stop),
                Action::None => {}
            }
        });
        ui.weak("Uses the system resolver (same answers Windows apps get, including hosts file and DNS suffixes).");
        widgets::error(ui, &self.err);
        ui.separator();

        ui.horizontal(|ui| {
            if let Some(ms) = self.took {
                ui.label(format!(
                    "{} record(s) in {}",
                    self.records.len(),
                    widgets::fmt_ms(ms)
                ));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                widgets::copy_button(ui, || {
                    self.records
                        .iter()
                        .map(|(k, v)| format!("{k:<5} {v}\n"))
                        .collect()
                });
            });
        });
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Grid::new("dns")
                    .striped(true)
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        for (k, v) in &self.records {
                            ui.label(RichText::new(k).strong());
                            ui.monospace(v);
                            ui.end_row();
                        }
                    });
            });
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.err = None;
        self.took = None;
        self.records.clear();
        let q = self.query.trim().to_string();
        if q.is_empty() {
            self.err = Some("enter a name or IP".into());
            return;
        }
        self.job = Some(Job::spawn(ctx, move |tx| {
            let t0 = Instant::now();
            if let Ok(ip) = q.parse::<IpAddr>() {
                match net::reverse(ip) {
                    Some(n) => tx.send(Ev::Record("PTR".into(), n)),
                    None => tx.send(Ev::Error(format!("no PTR record for {ip}"))),
                }
            } else {
                match net::lookup_all(&q) {
                    Ok(ips) => {
                        for ip in &ips {
                            let kind = if ip.is_ipv4() { "A" } else { "AAAA" };
                            tx.send(Ev::Record(kind.into(), ip.to_string()));
                        }
                        // Reverse each address: shows where a name really lands.
                        for ip in ips {
                            if let Some(n) = net::reverse(ip) {
                                tx.send(Ev::Record("PTR".into(), format!("{ip} → {n}")));
                            }
                        }
                    }
                    Err(e) => tx.send(Ev::Error(format!("lookup failed: {e}"))),
                }
            }
            tx.send(Ev::Done(t0.elapsed().as_secs_f64() * 1000.0));
        }));
    }
}
