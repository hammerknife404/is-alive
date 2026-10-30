use crate::job::{Job, pump};
use crate::widgets;
use crate::{sonicwall, unifi};
use eframe::egui::{self, Ui};
use std::path::PathBuf;

/// A converted config, by what the file turned out to be.
enum Export {
    Unifi(unifi::Report),
    SonicWall(sonicwall::Report),
}

impl Export {
    fn headline(&self) -> String {
        match self {
            Export::Unifi(r) => format!(
                "UniFi: {} site(s), {} device(s), {} network(s), {} WLAN(s){}",
                r.sites.len(),
                r.count("device"),
                r.count("networkconf"),
                r.count("wlanconf"),
                if r.clients {
                    format!(", {} client(s)", r.count(unifi::CLIENTS))
                } else {
                    String::new()
                }
            ),
            Export::SonicWall(r) => format!(
                "SonicWall: {} interface(s), {} address object(s), {} access rule(s), {} NAT rule(s)",
                r.count("interfaces"),
                r.count("address objects"),
                r.count("access rules"),
                r.count("NAT rules"),
            ),
        }
    }

    fn summary(&self) -> String {
        match self {
            Export::Unifi(r) => r.summary(),
            Export::SonicWall(r) => r.summary(),
        }
    }

    fn json(&self) -> &str {
        match self {
            Export::Unifi(r) => &r.json,
            Export::SonicWall(r) => &r.json,
        }
    }

    /// Output file suffix, naming the skill it's for.
    fn suffix(&self) -> &'static str {
        match self {
            Export::Unifi(_) => "unifidump",
            Export::SonicWall(_) => "sonicwall",
        }
    }

    fn hint(&self) -> &'static str {
        match self {
            Export::Unifi(_) => "Save the sanitized export to hand to /unifidump",
            Export::SonicWall(_) => "Save the sanitized config to hand to /mapnetwork",
        }
    }
}

enum Ev {
    Picked(PathBuf),
    Done(Result<Export, String>),
}

#[derive(Default)]
pub struct ExportTool {
    path: Option<PathBuf>,
    /// Off by default: client hostnames are often people's names.
    clients: bool,
    job: Option<Job<Ev>>,
    export: Option<Export>,
    err: Option<String>,
}

fn convert(data: Vec<u8>, clients: bool) -> Result<Export, String> {
    if sonicwall::decode(&data).is_some() {
        sonicwall::convert(&data).map(Export::SonicWall)
    } else {
        unifi::convert(data, clients).map(Export::Unifi)
    }
}

impl ExportTool {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn poll(&mut self) -> bool {
        pump(&mut self.job, |ev| match ev {
            Ev::Picked(p) => {
                self.path = Some(p);
                self.export = None;
                self.err = None;
            }
            Ev::Done(Ok(r)) => self.export = Some(r),
            Ev::Done(Err(e)) => self.err = Some(e),
        })
    }

    /// Convert `path`, or ask for a file first when it's None.
    fn start(&mut self, ctx: &egui::Context, path: Option<PathBuf>) {
        let clients = self.clients;
        self.job = Some(Job::spawn(ctx, move |tx| {
            let Some(path) = path.or_else(|| {
                rfd::FileDialog::new()
                    .set_title("Open config export")
                    .add_filter(
                        "UniFi backup or SonicWall settings",
                        &["unf", "unifi", "exp", "zip", "gz", "txt"],
                    )
                    .pick_file()
            }) else {
                return;
            };
            tx.send(Ev::Picked(path.clone()));
            let r = std::fs::read(&path)
                .map_err(|e| format!("{}: {e}", path.display()))
                .and_then(|data| convert(data, clients));
            tx.send(Ev::Done(r));
        }));
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let running = self.poll();

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!running, egui::Button::new("Open file…"))
                .on_hover_text(
                    "UniFi: .unf (site export or autobackup), .unifi (UniFi OS console backup), \
a .zip holding one, or db.gz. SonicWall: .exp settings export.",
                )
                .clicked()
            {
                self.start(ui.ctx(), None);
            }
            let tip = "UniFi only. Client table: hostnames (often people's names), MACs and last IPs. \
unifidump uses it for client counts and to spot unadopted gear plugged in as a client.";
            let toggled = ui
                .add_enabled(!running, egui::Checkbox::new(&mut self.clients, "Include UniFi clients"))
                .on_hover_text(tip)
                .changed();
            if toggled
                && matches!(self.export, Some(Export::Unifi(_)))
                && let Some(p) = self.path.clone()
            {
                self.start(ui.ctx(), Some(p));
            }
            if running {
                ui.spinner();
            }
            if let Some(p) = &self.path {
                let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
                ui.add(egui::Label::new(egui::RichText::new(name).weak()).truncate())
                    .on_hover_text(p.display().to_string());
            }
        });
        ui.weak(
            "Decodes a UniFi backup or SonicWall settings export on this machine and saves JSON with \
the topology and config the skills need. Passwords, keys and secrets are left out. Nothing is sent anywhere.",
        );
        widgets::error(ui, &self.err);
        ui.separator();

        let Some(r) = &self.export else {
            if !running {
                widgets::empty_hint(
                    ui,
                    "Open a .unf / .unifi UniFi backup (for /unifidump) or a SonicWall .exp (for /mapnetwork).",
                );
            }
            return;
        };
        ui.horizontal(|ui| {
            ui.label(r.headline());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Save JSON…").on_hover_text(r.hint()).clicked() {
                    let stem = self
                        .path
                        .as_ref()
                        .and_then(|p| p.file_stem())
                        .map_or("config".into(), |s| s.to_string_lossy().into_owned());
                    widgets::save_file(
                        format!("{stem}.{}.json", r.suffix()),
                        "JSON",
                        vec!["json"],
                        r.json().to_string(),
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
