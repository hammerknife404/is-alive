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

    /// Which slot it fills: one result per kind, so opening a SonicWall
    /// export never discards an unsaved UniFi one, and vice versa.
    fn slot(&self) -> usize {
        match self {
            Export::Unifi(_) => 0,
            Export::SonicWall(_) => 1,
        }
    }
}

struct Slot {
    path: PathBuf,
    export: Export,
}

enum Ev {
    Picked(PathBuf),
    Done(PathBuf, Result<Export, String>),
}

#[derive(Default)]
pub struct ExportTool {
    /// Off by default: client hostnames are often people's names.
    clients: bool,
    job: Option<Job<Ev>>,
    /// File being read while the job runs.
    pending: Option<PathBuf>,
    /// UniFi, SonicWall.
    slots: [Option<Slot>; 2],
    err: Option<String>,
}

fn convert(data: Vec<u8>, clients: bool) -> Result<Export, String> {
    if sonicwall::decode(&data).is_some() {
        sonicwall::convert(&data).map(Export::SonicWall)
    } else {
        unifi::convert(data, clients)
            .map(Export::Unifi)
            .map_err(|e| {
                if e.starts_with("not a UniFi backup") {
                    "not a UniFi backup (.unf, .unifi, db.gz) or SonicWall settings export (.exp)"
                        .into()
                } else {
                    e
                }
            })
    }
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl ExportTool {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn poll(&mut self) -> bool {
        let running = pump(&mut self.job, |ev| match ev {
            Ev::Picked(p) => self.pending = Some(p),
            Ev::Done(path, Ok(export)) => {
                let i = export.slot();
                self.slots[i] = Some(Slot { path, export });
            }
            Ev::Done(path, Err(e)) => {
                let e = format!("{}: {e}", file_name(&path));
                self.err = Some(match self.err.take() {
                    Some(prev) => format!("{prev}\n{e}"),
                    None => e,
                });
            }
        });
        if !running {
            self.pending = None;
        }
        running
    }

    /// Convert files dropped on the window or picked elsewhere.
    pub fn open(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        if !self.is_running() && !paths.is_empty() {
            self.start(ctx, Some(paths));
        }
    }

    /// Convert `paths` one after another, or ask for files first when None.
    /// Each lands in its kind's slot, so a UniFi backup and a SonicWall
    /// export picked together both show.
    fn start(&mut self, ctx: &egui::Context, paths: Option<Vec<PathBuf>>) {
        self.err = None;
        let clients = self.clients;
        self.job = Some(Job::spawn(ctx, move |tx| {
            let Some(paths) = paths.or_else(|| {
                rfd::FileDialog::new()
                    .set_title("Open config export")
                    .add_filter(
                        "UniFi backup or SonicWall settings",
                        &["unf", "unifi", "exp", "zip", "gz", "txt"],
                    )
                    .pick_files()
            }) else {
                return;
            };
            for path in paths {
                tx.send(Ev::Picked(path.clone()));
                let r = std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|data| convert(data, clients));
                tx.send(Ev::Done(path, r));
            }
        }));
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let running = self.poll();

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!running, egui::Button::new("Open file…"))
                .on_hover_text(
                    "UniFi: .unf (site export or autobackup), .unifi (UniFi OS console backup), \
a .zip holding one, or db.gz. SonicWall: .exp settings export. Pick both at once, or drop them \
on the window. Each kind keeps its own result; opening another file of the same kind replaces it.",
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
            if toggled && let Some(s) = &self.slots[0] {
                self.start(ui.ctx(), Some(vec![s.path.clone()]));
            }
            if let Some(p) = &self.pending {
                ui.spinner();
                ui.add(egui::Label::new(egui::RichText::new(file_name(p)).weak()).truncate());
            }
        });
        ui.weak(
            "Decodes a UniFi backup or SonicWall settings export on this machine and saves JSON with \
the topology and config the skills need. Passwords, keys and secrets are left out. Nothing is sent anywhere.",
        );
        widgets::error(ui, &self.err);
        ui.separator();

        if self.slots.iter().all(Option::is_none) {
            if !running {
                widgets::empty_hint(
                    ui,
                    "Open or drop a .unf / .unifi UniFi backup (for /unifidump) or a SonicWall .exp (for /mapnetwork).",
                );
            }
            return;
        }
        let mut clear = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (i, slot) in self.slots.iter().enumerate() {
                    let Some(Slot { path, export: r }) = slot else {
                        continue;
                    };
                    egui::CollapsingHeader::new(egui::RichText::new(r.headline()).strong())
                        .id_salt(("export", i))
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if ui.button("Save JSON…").on_hover_text(r.hint()).clicked() {
                                    let stem = path.file_stem().map_or("config".into(), |s| {
                                        s.to_string_lossy().into_owned()
                                    });
                                    widgets::save_file(
                                        format!("{stem}.{}.json", r.suffix()),
                                        "JSON",
                                        vec!["json"],
                                        r.json().to_string(),
                                    );
                                }
                                widgets::copy_button(ui, || r.summary());
                                if ui
                                    .button("Clear")
                                    .on_hover_text("Close this result")
                                    .clicked()
                                {
                                    clear = Some(i);
                                }
                                ui.add(
                                    egui::Label::new(egui::RichText::new(file_name(path)).weak())
                                        .truncate(),
                                )
                                .on_hover_text(path.display().to_string());
                            });
                            ui.add(
                                egui::Label::new(egui::RichText::new(r.summary()).monospace())
                                    .selectable(true),
                            );
                        });
                }
            });
        if let Some(i) = clear {
            self.slots[i] = None;
        }
    }
}
