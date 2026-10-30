use crate::job::{Job, pump};
use crate::unifi::{self, Report};
use crate::widgets;
use eframe::egui::{self, Ui};
use std::path::PathBuf;

enum Ev {
    Picked(PathBuf),
    Done(Result<Report, String>),
}

#[derive(Default)]
pub struct UnifiTool {
    path: Option<PathBuf>,
    /// Off by default: client hostnames are often people's names.
    clients: bool,
    job: Option<Job<Ev>>,
    report: Option<Report>,
    err: Option<String>,
}

impl UnifiTool {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn poll(&mut self) -> bool {
        pump(&mut self.job, |ev| match ev {
            Ev::Picked(p) => {
                self.path = Some(p);
                self.report = None;
                self.err = None;
            }
            Ev::Done(Ok(r)) => self.report = Some(r),
            Ev::Done(Err(e)) => self.err = Some(e),
        })
    }

    /// Convert `path`, or ask for a file first when it's None.
    fn start(&mut self, ctx: &egui::Context, path: Option<PathBuf>) {
        let clients = self.clients;
        self.job = Some(Job::spawn(ctx, move |tx| {
            let Some(path) = path.or_else(|| {
                rfd::FileDialog::new()
                    .set_title("Open UniFi backup")
                    .add_filter("UniFi backup", &["unf", "unifi", "zip", "gz"])
                    .pick_file()
            }) else {
                return;
            };
            tx.send(Ev::Picked(path.clone()));
            let r = std::fs::read(&path)
                .map_err(|e| format!("{}: {e}", path.display()))
                .and_then(|data| unifi::convert(data, clients));
            tx.send(Ev::Done(r));
        }));
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let running = self.poll();

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!running, egui::Button::new("Open backup…"))
                .on_hover_text(".unf (site export or autobackup), .unifi (UniFi OS console backup), a .zip holding one, or db.gz")
                .clicked()
            {
                self.start(ui.ctx(), None);
            }
            let tip = "Client table: hostnames (often people's names), MACs and last IPs. \
unifidump uses it for client counts and to spot unadopted gear plugged in as a client.";
            let toggled = ui
                .add_enabled(!running, egui::Checkbox::new(&mut self.clients, "Include clients"))
                .on_hover_text(tip)
                .changed();
            if toggled && let Some(p) = self.path.clone() {
                self.start(ui.ctx(), Some(p));
            }
            if running {
                ui.spinner();
            }
            if let Some(p) = &self.path {
                ui.weak(p.display().to_string());
            }
        });
        ui.weak(
            "Decrypts the backup on this machine and saves JSON for the unifidump skill: sites, devices, \
networks, port profiles and WLANs, with passwords, keys and secrets removed. Nothing is sent anywhere.",
        );
        widgets::error(ui, &self.err);
        ui.separator();

        let Some(r) = &self.report else {
            if !running {
                widgets::empty_hint(ui, "Open a .unf or .unifi backup to convert it.");
            }
            return;
        };
        ui.horizontal(|ui| {
            ui.label(format!(
                "{} site(s), {} device(s), {} network(s), {} WLAN(s){}",
                r.sites.len(),
                r.count("device"),
                r.count("networkconf"),
                r.count("wlanconf"),
                if r.clients {
                    format!(", {} client(s)", r.count(unifi::CLIENTS))
                } else {
                    String::new()
                }
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("Save JSON…")
                    .on_hover_text("Save the sanitized export to hand to /unifidump")
                    .clicked()
                {
                    let stem = self
                        .path
                        .as_ref()
                        .and_then(|p| p.file_stem())
                        .map_or("unifi".into(), |s| s.to_string_lossy().into_owned());
                    widgets::save_file(
                        format!("{stem}.unifidump.json"),
                        "JSON",
                        vec!["json"],
                        r.json.clone(),
                    );
                }
                widgets::copy_button(ui, || r.summary());
            });
        });
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(r.summary()).monospace()).selectable(true),
                );
            });
    }
}
