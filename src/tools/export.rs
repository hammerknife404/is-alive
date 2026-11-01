use crate::job::{Job, pump};
use crate::widgets;
use crate::{mapnetwork, meraki, sonicwall, unifi};
use eframe::egui::{self, Ui};
use std::path::PathBuf;

/// A converted config, by what the file turned out to be.
enum Export {
    Unifi(unifi::Report),
    SonicWall(sonicwall::Report),
    Map(Box<mapnetwork::Doc>),
    Meraki(meraki::Report),
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
            Export::Meraki(r) => format!(
                "Meraki: {} network(s), {} device(s), {} VLAN(s), {} firewall rule(s), {} port forward(s)",
                r.count("networks"),
                r.count("devices"),
                r.count("VLANs"),
                r.count("firewall rules"),
                r.count("port forwards"),
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
            Export::Meraki(r) => r.summary(),
            Export::Map(d) => d.summary(),
        }
    }

    /// Secret values the conversion stripped, held in memory only.
    fn secrets(&self) -> &[String] {
        match self {
            Export::Unifi(r) => &r.secrets,
            Export::SonicWall(r) => &r.secrets,
            Export::Meraki(r) => &r.secrets,
            Export::Map(_) => &[],
        }
    }

    fn json(&self) -> &str {
        match self {
            Export::Unifi(r) => &r.json,
            Export::SonicWall(r) => &r.json,
            Export::Meraki(r) => &r.json,
            Export::Map(d) => &d.json,
        }
    }

    /// Its sub-tab's label.
    fn tab(&self) -> &'static str {
        match self {
            Export::Unifi(_) => "UniFi",
            Export::SonicWall(r) if r.format.contains(".wri") => "SonicWall report",
            Export::SonicWall(_) => "SonicWall",
            Export::Meraki(_) => "Meraki",
            Export::Map(_) => "/mapnetwork map",
        }
    }

    /// Output file suffix, naming the skill it's for.
    fn suffix(&self) -> &'static str {
        match self {
            Export::Unifi(_) => "unifidump",
            Export::SonicWall(_) => "sonicwall",
            Export::Meraki(_) => "meraki",
            Export::Map(_) => "mapnetwork",
        }
    }

    fn hint(&self) -> &'static str {
        match self {
            Export::Unifi(_) => "Save the sanitized export to hand to /unifidump",
            Export::SonicWall(_) | Export::Meraki(_) => {
                "Save the sanitized config to hand to /mapnetwork"
            }
            Export::Map(_) => "Save the map's devices, links and findings as JSON",
        }
    }
}

struct Slot {
    path: PathBuf,
    export: Export,
}

/// Sub-tabs shown side by side; older files fold into a menu.
const TABS: usize = 6;

impl Slot {
    /// "SonicWall · fw-main", short enough for a sub-tab.
    fn label(&self) -> String {
        // "fw-main.dc.html" → "fw-main".
        let name = file_name(&self.path);
        let stem = match name.split_once('.') {
            Some((s, _)) if !s.is_empty() => s.to_string(),
            _ => name,
        };
        let stem: String = if stem.chars().count() > 22 {
            stem.chars().take(21).collect::<String>() + "…"
        } else {
            stem
        };
        let kind = match self.export {
            Export::Map(_) => "Map",
            _ => self.export.tab(),
        };
        format!("{kind} · {stem}")
    }
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
    /// Every file converted, in the order opened; each has its own sub-tab.
    /// Opening a file again replaces its result.
    slots: Vec<Slot>,
    /// Every loaded file's stripped secret values, for Notes to redact.
    secrets: Vec<String>,
    /// The slot whose sub-tab is showing: the one opened last, until
    /// another tab is picked.
    shown: usize,
    err: Option<String>,
    /// Counts loads, so the map can tell a file read again from the same
    /// file unchanged.
    generation: u64,
    /// Files the last load couldn't read or convert.
    failed: Vec<String>,
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
    } else if meraki::sniff(&data) {
        meraki::convert(&data).map(Export::Meraki)
    } else {
        unifi::convert(data, clients)
            .map(Export::Unifi)
            .map_err(|e| {
                if e.starts_with("not a UniFi backup") {
                    "not a UniFi backup (.unf, .unifi, db.gz), SonicWall settings export (.exp) or \
tech support report (.wri), Meraki API export (.json), or /mapnetwork map (.dc.html)"
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
    if let Some(data) = crate::skills::example(p) {
        return Ok(data.to_vec());
    }
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

    /// The UniFi backup opened last, and its conversion.
    pub fn unifi(&self) -> Option<(&PathBuf, &unifi::Report)> {
        self.slots.iter().rev().find_map(|s| match &s.export {
            Export::Unifi(r) => Some((&s.path, r)),
            _ => None,
        })
    }

    /// Secret values from every loaded backup and export (memory only).
    pub fn secrets(&self) -> &[String] {
        &self.secrets
    }

    fn gather_secrets(&mut self) {
        let mut all: Vec<String> = self
            .slots
            .iter()
            .flat_map(|s| s.export.secrets().iter().cloned())
            .collect();
        all.sort();
        all.dedup();
        self.secrets = all;
    }

    /// Files the last load couldn't read or convert.
    pub fn failed(&self) -> &[String] {
        &self.failed
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The files the Map is drawn from: the UniFi backup, SonicWall export
    /// and /mapnetwork map opened last of each.
    pub fn map_sources(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = [
            self.unifi().map(|u| u.0),
            self.sonicwall().map(|s| s.0),
            self.mapnetwork().map(|m| m.0),
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
        v.dedup();
        v
    }

    /// The /mapnetwork map opened last.
    pub fn mapnetwork(&self) -> Option<(&PathBuf, &mapnetwork::Doc)> {
        self.slots.iter().rev().find_map(|s| match &s.export {
            Export::Map(d) => Some((&s.path, &**d)),
            _ => None,
        })
    }

    /// The Meraki export opened last, and its conversion.
    pub fn meraki(&self) -> Option<(&PathBuf, &meraki::Report)> {
        self.slots.iter().rev().find_map(|s| match &s.export {
            Export::Meraki(r) => Some((&s.path, r)),
            _ => None,
        })
    }

    /// The SonicWall export opened last, and its conversion.
    pub fn sonicwall(&self) -> Option<(&PathBuf, &sonicwall::Report)> {
        self.slots.iter().rev().find_map(|s| match &s.export {
            Export::SonicWall(r) => Some((&s.path, r)),
            _ => None,
        })
    }

    pub fn poll(&mut self) -> bool {
        let mut loaded = false;
        let running = pump(&mut self.job, |ev| match ev {
            Ev::Picked(p) => self.pending = Some(p),
            Ev::Done(path, r) => match *r {
                Ok(export) => {
                    let slot = Slot { path, export };
                    self.shown = match self.slots.iter().position(|s| s.path == slot.path) {
                        Some(i) => {
                            self.slots[i] = slot;
                            i
                        }
                        None => {
                            self.slots.push(slot);
                            self.slots.len() - 1
                        }
                    };
                    loaded = true;
                }
                Err(e) => {
                    self.failed.push(file_name(&path));
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
            self.generation += 1;
        }
        if !running {
            self.pending = None;
        }
        running
    }

    /// Open one of the example maps (see `skills::EXAMPLES`).
    pub fn open_example(&mut self, ctx: &egui::Context, i: usize) {
        self.open(ctx, vec![crate::skills::example_path(i)]);
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
        self.failed.clear();
        let clients = self.clients;
        self.job = Some(Job::spawn(ctx, move |tx| {
            let Some(paths) = paths.or_else(|| {
                rfd::FileDialog::new()
                    .set_title("Open config export")
                    .add_filter(
                        "UniFi backup, SonicWall or Meraki config, or /mapnetwork map",
                        &[
                            "unf", "unifi", "exp", "wri", "json", "html", "htm", "zip", "gz", "txt",
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
                    "UniFi .unf / .unifi / db.gz, SonicWall .exp / .wri, Meraki API .json (or a zip of them), or a /mapnetwork .dc.html. Several at once, or drop them on the window",
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
            if toggled && let Some((p, _)) = self.unifi() {
                self.start(ui.ctx(), Some(vec![p.clone()]));
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

        if self.slots.is_empty() {
            if !running
                && widgets::empty_state(
                    ui,
                    "Convert a config",
                    "Open or drop a UniFi backup, SonicWall .exp / .wri, Meraki .json, or a /mapnetwork map. \
                     The result is sanitized JSON for the skills, and it feeds the Map.",
                    &[("Open file…", "Pick one or more files", true)],
                )
                .is_some()
            {
                self.start(ui.ctx(), None);
            }
            return;
        }
        self.shown = self.shown.min(self.slots.len() - 1);
        // One sub-tab per file; past TABS the oldest fold into a menu.
        ui.horizontal_wrapped(|ui| {
            let n = self.slots.len();
            let folded = n.saturating_sub(TABS);
            let mut pick = None;
            if folded > 0 {
                let shown_folded = self.shown < folded;
                let label = if shown_folded {
                    format!("{} ⏷", self.slots[self.shown].label())
                } else {
                    format!("{folded} more ⏷")
                };
                ui.menu_button(label, |ui| {
                    for (i, s) in self.slots[..folded].iter().enumerate() {
                        if ui
                            .selectable_label(self.shown == i, s.label())
                            .on_hover_text(format!(
                                "{}\n{}",
                                s.export.headline(),
                                file_name(&s.path)
                            ))
                            .clicked()
                        {
                            pick = Some(i);
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Files opened earlier");
                ui.separator();
            }
            for (i, s) in self.slots.iter().enumerate().skip(folded) {
                let on = self.shown == i;
                let mut text = egui::RichText::new(s.label());
                if on {
                    text = text.color(crate::theme::accent()).strong();
                }
                if ui
                    .selectable_label(on, text)
                    .on_hover_text(format!("{}\n{}", s.export.headline(), file_name(&s.path)))
                    .clicked()
                {
                    pick = Some(i);
                }
            }
            if let Some(i) = pick {
                self.shown = i;
            }
        });
        ui.separator();
        let mut clear = None;
        if let Some(Slot { path, export: r }) = self.slots.get(self.shown) {
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
            self.slots.remove(i);
            self.shown = self.shown.min(self.slots.len().saturating_sub(1));
            self.gather_secrets();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: &str = "<html>/* ===== SITE DATA ===== */ this.META = { title:'Acme' };\
        this.NODES = [ {id:'gw', kind:'GATEWAY', label:'GW'} ]; this.EDGES = [];\
        /* ===== END SITE DATA ===== */</html>";

    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        crate::bundle::zip(files).unwrap()
    }

    fn kind(r: &Result<Export, String>) -> &'static str {
        match r {
            Ok(e) => e.tab(),
            Err(_) => "error",
        }
    }

    #[test]
    fn a_mapnetwork_page_is_a_map() {
        let r = convert(MAP.as_bytes().to_vec(), unifi::Clients::None);
        assert_eq!(kind(&r), "/mapnetwork map");
        assert!(r.unwrap().headline().contains("Acme: 1 device(s)"));
    }

    #[test]
    fn a_zip_holding_the_map_opens_the_map() {
        let z = zip(&[("manifest.json", b"{}"), ("acme.dc.html", MAP.as_bytes())]);
        assert_eq!(kind(&convert(z, unifi::Clients::None)), "/mapnetwork map");
    }

    #[test]
    fn a_site_bundle_without_a_map_says_what_to_open() {
        let z = zip(&[("manifest.json", br#"{"kind":"is-alive site bundle"}"#)]);
        let e = convert(z, unifi::Clients::None).err().unwrap();
        assert!(e.contains("no /mapnetwork map in it yet"), "{e}");
    }

    #[test]
    fn a_sonicwall_export_is_read_as_one() {
        let exp: String = (0..12)
            .map(|i| format!("iface_name_{i}=X{i}&interface_Zone_{i}=LAN&"))
            .collect::<String>()
            + "shortProdName=TZ%20470&firmwareVersion=SonicOS%207.0.1";
        let r = convert(exp.into_bytes(), unifi::Clients::None);
        assert_eq!(kind(&r), "SonicWall");
        let e = r.unwrap();
        assert_eq!(e.suffix(), "sonicwall");
        assert!(e.headline().starts_with("SonicWall: "));
    }

    #[test]
    fn anything_else_names_every_format_it_takes() {
        let e = convert(b"just some text".to_vec(), unifi::Clients::None)
            .err()
            .unwrap();
        for f in [".unf", ".exp", ".wri", ".json", ".dc.html"] {
            assert!(e.contains(f), "{f} missing from: {e}");
        }
    }

    #[test]
    fn sub_tab_labels_are_short() {
        let map = |path: &str| Slot {
            path: PathBuf::from(path),
            export: convert(MAP.as_bytes().to_vec(), unifi::Clients::None).unwrap(),
        };
        assert_eq!(map("/x/fw-main.dc.html").label(), "Map · fw-main");
        let long = map("/x/a-very-long-site-name-for-a-map.dc.html").label();
        // 21 characters and an ellipsis, so the sub-tab stays narrow.
        assert_eq!(long, "Map · a-very-long-site-name…");
    }

    #[test]
    fn read_input_refuses_what_isnt_a_file() {
        let dir = std::env::temp_dir();
        let e = read_input(&dir).err().unwrap();
        assert!(e.ends_with("not a regular file"), "{e}");
        let e = read_input(&dir.join("is-alive-no-such-file"))
            .err()
            .unwrap();
        assert!(e.starts_with("is-alive-no-such-file: "), "{e}");
    }
}
