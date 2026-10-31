use crate::job::{Job, pump};
use crate::widgets;
use crate::{mapnetwork, sonicwall, unifi};
use eframe::egui::{self, Ui};
use std::path::PathBuf;

/// A converted config, by what the file turned out to be.
enum Export {
    Unifi(unifi::Report),
    SonicWall(sonicwall::Report),
    Map(Box<mapnetwork::Doc>),
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
                match r.clients {
                    unifi::Clients::None => String::new(),
                    c => format!(", {} client(s), {}", r.count(unifi::CLIENTS), c.label()),
                }
            ),
            Export::SonicWall(r) => format!(
                "SonicWall: {} interface(s), {} address object(s), {} access rule(s), {} NAT rule(s)",
                r.count("interfaces"),
                r.count("address objects"),
                r.count("access rules"),
                r.count("NAT rules"),
            ),
            Export::Map(d) => format!(
                "/mapnetwork map: {}: {} device(s), {} link(s), {} finding(s)",
                if d.title.is_empty() {
                    "untitled"
                } else {
                    &d.title
                },
                d.nodes.len(),
                d.edges.len(),
                d.actions.len(),
            ),
        }
    }

    fn summary(&self) -> String {
        match self {
            Export::Unifi(r) => r.summary(),
            Export::SonicWall(r) => r.summary(),
            Export::Map(d) => d.summary(),
        }
    }

    /// Secret values the conversion stripped, held in memory only.
    fn secrets(&self) -> &[String] {
        match self {
            Export::Unifi(r) => &r.secrets,
            Export::SonicWall(r) => &r.secrets,
            Export::Map(_) => &[],
        }
    }

    fn json(&self) -> &str {
        match self {
            Export::Unifi(r) => &r.json,
            Export::SonicWall(r) => &r.json,
            Export::Map(d) => &d.json,
        }
    }

    /// Its sub-tab's label.
    fn tab(&self) -> &'static str {
        match self {
            Export::Unifi(_) => "UniFi",
            Export::SonicWall(r) if r.format.contains(".wri") => "SonicWall report",
            Export::SonicWall(_) => "SonicWall",
            Export::Map(_) => "/mapnetwork map",
        }
    }

    /// Output file suffix, naming the skill it's for.
    fn suffix(&self) -> &'static str {
        match self {
            Export::Unifi(_) => "unifidump",
            Export::SonicWall(_) => "sonicwall",
            Export::Map(_) => "mapnetwork",
        }
    }

    fn hint(&self) -> &'static str {
        match self {
            Export::Unifi(_) => "Save the sanitized export to hand to /unifidump",
            Export::SonicWall(_) => "Save the sanitized config to hand to /mapnetwork",
            Export::Map(_) => "Save the map's devices, links and findings as JSON",
        }
    }

    /// Which slot it fills: one result per kind, so opening a SonicWall
    /// export never discards an unsaved UniFi one, and vice versa.
    fn slot(&self) -> usize {
        match self {
            Export::Unifi(_) => 0,
            Export::SonicWall(_) => 1,
            Export::Map(_) => 2,
        }
    }
}

struct Slot {
    path: PathBuf,
    export: Export,
}

enum Ev {
    Picked(PathBuf),
    Done(PathBuf, Box<Result<Export, String>>),
}

#[derive(Default)]
pub struct ExportTool {
    /// None by default here: client hostnames are often people's names.
    clients: unifi::Clients,
    job: Option<Job<Ev>>,
    /// File being read while the job runs.
    pending: Option<PathBuf>,
    /// UniFi, SonicWall, /mapnetwork map.
    slots: [Option<Slot>; 3],
    /// Every loaded file's stripped secret values, for Notes to redact.
    secrets: Vec<String>,
    /// The slot whose sub-tab is showing: the one opened last, until
    /// another tab is picked.
    shown: usize,
    err: Option<String>,
}

fn convert(data: Vec<u8>, clients: unifi::Clients) -> Result<Export, String> {
    if mapnetwork::sniff(&data) {
        return mapnetwork::parse(&data).map(|d| Export::Map(Box::new(d)));
    }
    // A site bundle (or any zip) holding the map /mapnetwork made from it.
    let in_zip = |ext: &str| {
        unifi::zip_find(&data, |n| n.to_lowercase().ends_with(ext))
            .ok()
            .flatten()
            .filter(|(_, html)| mapnetwork::sniff(html))
    };
    if let Some((name, html)) = in_zip(".dc.html").or_else(|| in_zip(".html")) {
        return mapnetwork::parse(&html)
            .map(|d| Export::Map(Box::new(d)))
            .map_err(|e| format!("{name}: {e}"));
    }
    if unifi::zip_find(&data, |n| n == "manifest.json")
        .ok()
        .flatten()
        .is_some_and(|(_, m)| String::from_utf8_lossy(&m).contains("is-alive site bundle"))
    {
        return Err(
            "this site bundle has no /mapnetwork map in it yet: open the .dc.html \
Claude made from it, or a zip you saved it into"
                .into(),
        );
    }
    if sonicwall::sniff(&data) {
        sonicwall::convert(&data).map(Export::SonicWall)
    } else {
        unifi::convert(data, clients)
            .map(Export::Unifi)
            .map_err(|e| {
                if e.starts_with("not a UniFi backup") {
                    "not a UniFi backup (.unf, .unifi, db.gz), SonicWall settings export (.exp) or \
tech support report (.wri), or /mapnetwork map (.dc.html)"
                        .into()
                } else {
                    e
                }
            })
    }
}

/// "UniFi clients: none / anonymized / full" picker; true when changed.
pub fn clients_choice(ui: &mut Ui, id: &str, clients: &mut unifi::Clients) -> bool {
    ui.label("UniFi clients")
        .on_hover_text("Include the UniFi client table (every device that has connected)");
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(clients.label())
        .show_ui(ui, |ui| {
            for c in unifi::Clients::ALL {
                changed |= ui
                    .selectable_value(clients, c, c.label())
                    .on_hover_text(c.hint())
                    .changed();
            }
        });
    changed
}

/// Read a file to convert: a regular file no bigger than any backup can
/// be, so a device, a pipe or a huge file is refused rather than read.
pub fn read_input(p: &std::path::Path) -> Result<Vec<u8>, String> {
    let name = file_name(p);
    let meta = std::fs::metadata(p).map_err(|e| format!("{name}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{name}: not a regular file"));
    }
    if meta.len() > crate::unifi::MAX_INPUT as u64 {
        return Err(format!(
            "{name}: {} MB is larger than any backup or export this reads",
            meta.len() >> 20
        ));
    }
    std::fs::read(p).map_err(|e| format!("{name}: {e}"))
}

pub fn file_name(p: &std::path::Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl ExportTool {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    /// The UniFi backup currently loaded, and its conversion.
    pub fn unifi(&self) -> Option<(&PathBuf, &unifi::Report)> {
        match &self.slots[0] {
            Some(Slot {
                path,
                export: Export::Unifi(r),
            }) => Some((path, r)),
            _ => None,
        }
    }

    /// Secret values from every loaded backup and export (memory only).
    pub fn secrets(&self) -> &[String] {
        &self.secrets
    }

    fn gather_secrets(&mut self) {
        let mut all: Vec<String> = self
            .slots
            .iter()
            .flatten()
            .flat_map(|s| s.export.secrets().iter().cloned())
            .collect();
        all.sort();
        all.dedup();
        self.secrets = all;
    }

    /// The /mapnetwork map currently loaded.
    pub fn mapnetwork(&self) -> Option<(&PathBuf, &mapnetwork::Doc)> {
        match &self.slots[2] {
            Some(Slot {
                path,
                export: Export::Map(d),
            }) => Some((path, &**d)),
            _ => None,
        }
    }

    /// The SonicWall export currently loaded, and its conversion.
    pub fn sonicwall(&self) -> Option<(&PathBuf, &sonicwall::Report)> {
        match &self.slots[1] {
            Some(Slot {
                path,
                export: Export::SonicWall(r),
            }) => Some((path, r)),
            _ => None,
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut loaded = false;
        let running = pump(&mut self.job, |ev| match ev {
            Ev::Picked(p) => self.pending = Some(p),
            Ev::Done(path, r) => match *r {
                Ok(export) => {
                    let i = export.slot();
                    self.slots[i] = Some(Slot { path, export });
                    self.shown = i;
                    loaded = true;
                }
                Err(e) => {
                    let e = format!("{}: {e}", file_name(&path));
                    self.err = Some(match self.err.take() {
                        Some(prev) => format!("{prev}\n{e}"),
                        None => e,
                    });
                }
            },
        });
        if loaded {
            self.gather_secrets();
        }
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
                        "UniFi backup, SonicWall settings or /mapnetwork map",
                        &[
                            "unf", "unifi", "exp", "wri", "html", "htm", "zip", "gz", "txt",
                        ],
                    )
                    .pick_files()
            }) else {
                return;
            };
            for path in paths {
                tx.send(Ev::Picked(path.clone()));
                let r = read_input(&path).and_then(|data| convert(data, clients));
                tx.send(Ev::Done(path, Box::new(r)));
            }
        }));
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let running = self.poll();

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!running, egui::Button::new("Open file…"))
                .on_hover_text(
                    "UniFi .unf / .unifi / db.gz, SonicWall .exp / .wri, or a /mapnetwork .dc.html (or a zip of one). Several at once, or drop them on the window",
                )
                .clicked()
            {
                self.start(ui.ctx(), None);
            }
            let toggled = ui
                .add_enabled_ui(!running, |ui| {
                    clients_choice(ui, "export_clients", &mut self.clients)
                })
                .inner;
            if toggled && let Some(s) = &self.slots[0] {
                self.start(ui.ctx(), Some(vec![s.path.clone()]));
            }
            if let Some(p) = &self.pending {
                widgets::spinner(ui);
                ui.add(egui::Label::new(egui::RichText::new(file_name(p)).weak()).truncate());
            }
        });
        ui.weak(
            "Converts configs on this machine into JSON for the skills. Secrets are stripped; nothing is sent.",
        );
        widgets::error(ui, &self.err);
        ui.separator();

        if self.slots.iter().all(Option::is_none) {
            if !running {
                widgets::empty_hint(
                    ui,
                    "Open or drop a UniFi backup, SonicWall .exp / .wri, or /mapnetwork map.",
                );
            }
            return;
        }
        if self.slots[self.shown].is_none() {
            self.shown = self.slots.iter().position(Option::is_some).unwrap_or(0);
        }
        // One sub-tab per decoded file.
        ui.horizontal(|ui| {
            for (i, slot) in self.slots.iter().enumerate() {
                let Some(Slot { path, export: r }) = slot else {
                    continue;
                };
                let on = self.shown == i;
                let mut text = egui::RichText::new(r.tab());
                if on {
                    text = text.color(crate::theme::accent()).strong();
                }
                if ui
                    .selectable_label(on, text)
                    .on_hover_text(format!("{}\n{}", r.headline(), file_name(path)))
                    .clicked()
                {
                    self.shown = i;
                }
            }
        });
        ui.separator();
        let mut clear = None;
        if let Some(Slot { path, export: r }) = &self.slots[self.shown] {
            ui.label(egui::RichText::new(r.headline()).strong());
            ui.horizontal(|ui| {
                if ui.button("Save JSON…").on_hover_text(r.hint()).clicked() {
                    let stem = path
                        .file_stem()
                        .map_or("config".into(), |s| s.to_string_lossy().into_owned());
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
                    clear = Some(self.shown);
                }
                ui.add(egui::Label::new(egui::RichText::new(file_name(path)).weak()).truncate())
                    .on_hover_text(path.display().to_string());
            });
            let n = r.secrets().len();
            if n > 0 {
                ui.weak(format!(
                    "🔑 {n} secret{} from this file hidden in Notes (kept in memory only)",
                    if n == 1 { "" } else { "s" },
                ));
            }
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt(("export_summary", self.shown))
                .auto_shrink(false)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(r.summary()).monospace())
                            .selectable(true),
                    );
                });
        }
        if let Some(i) = clear {
            self.slots[i] = None;
            self.gather_secrets();
        }
    }
}
