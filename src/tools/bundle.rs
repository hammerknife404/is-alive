use crate::bundle::{self, Bundle, Part};
use crate::job::{Job, pump};
use crate::log::{Format, Log};
use crate::net::hostinfo::HostInfo;
use crate::tools::export::{self, ExportTool};
use crate::tools::sweep::SweepTool;
use crate::unifi::{self, Clients};
use crate::widgets;
use eframe::egui::{self, RichText, Ui};
use serde_json::json;
use std::path::PathBuf;

/// What the bundle can draw on, borrowed from the other tabs each frame.
pub struct Sources<'a> {
    pub export: &'a ExportTool,
    pub sweep: &'a SweepTool,
    pub log: &'a Log,
    pub info: &'a HostInfo,
}

struct Saved {
    path: PathBuf,
    kickoff: String,
}

pub struct BundleTool {
    pub open: bool,
    site: String,
    notes: String,
    clients: Clients,
    /// Answer to /mapnetwork's "include clients?"; off means infrastructure only.
    map_clients: bool,
    use_unifi: bool,
    use_sonicwall: bool,
    use_inventory: bool,
    use_results: bool,
    job: Option<Job<Result<Option<Saved>, String>>>,
    saved: Option<Saved>,
    err: Option<String>,
}

impl Default for BundleTool {
    fn default() -> Self {
        Self {
            open: false,
            site: String::new(),
            notes: String::new(),
            clients: Clients::Anonymized,
            map_clients: false,
            use_unifi: true,
            use_sonicwall: true,
            use_inventory: true,
            use_results: false,
            job: None,
            saved: None,
            err: None,
        }
    }
}

const UNIFI_WHAT: &str = "UniFi controller export: sites, devices, networks, switch port profiles and \
WLANs (plus clients as the manifest says), secrets removed. mongoexport-style array for /unifidump's extract.py.";
const SONICWALL_WHAT: &str = "SonicWall config: interfaces (zone, VLAN, IP, MAC, management), zones, \
address and service objects, access and NAT rules. Enforced config, not observation.";
const INVENTORY_WHAT: &str = "Ping sweep from the scanner: live hosts with MAC, IEEE vendor, name, TTL \
and open management ports. Observed state.";
const RESULTS_WHAT: &str =
    "is-alive test runs (ping, port check, traceroute, DNS, mail health) with start and end times.";

impl BundleTool {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    fn poll(&mut self, ctx: &egui::Context) -> bool {
        pump(&mut self.job, |r| match r {
            Ok(Some(saved)) => {
                ctx.copy_text(saved.kickoff.clone());
                self.saved = Some(saved);
            }
            Ok(None) => {}
            Err(e) => self.err = Some(e),
        })
    }

    /// The Site Bundle window; call every frame, open or not, so a save
    /// started before closing it still finishes.
    pub fn window(&mut self, ctx: &egui::Context, src: Sources) {
        let running = self.poll(ctx);
        if !self.open {
            return;
        }
        if self.site.trim().is_empty()
            && let Some((_, r)) = src.export.unifi()
            && let Some(name) = r.sites.first()
        {
            self.site = name.clone();
        }
        let mut open = self.open;
        egui::Window::new("Site Bundle")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .show(ctx, |ui| self.body(ui, &src, running));
        self.open = open;
    }

    fn body(&mut self, ui: &mut Ui, src: &Sources, running: bool) {
        ui.weak(
            "One zip of everything converted so far, with a manifest that tells /unifidump and \
/mapnetwork what each file is. Built only from the sanitized exports.",
        );
        ui.add_space(4.0);
        egui::Grid::new("bundle_form")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Site name");
                ui.add(
                    egui::TextEdit::singleline(&mut self.site)
                        .desired_width(380.0)
                        .hint_text("Client or site name"),
                );
                ui.end_row();
                ui.label("Known problems");
                ui.add(
                    egui::TextEdit::multiline(&mut self.notes)
                        .desired_rows(2)
                        .desired_width(380.0)
                        .hint_text("Optional. Goes straight into /mapnetwork's findings."),
                );
                ui.end_row();
            });
        ui.horizontal(|ui| {
            export::clients_choice(ui, "bundle_clients", &mut self.clients);
            ui.separator();
            ui.checkbox(&mut self.map_clients, "Clients on the map").on_hover_text(
                "Off: /mapnetwork maps infrastructure only and uses the client table for counts \
and to spot unadopted gear. On: client devices go on the map too (it caps and collapses large \
client lists). Applies to clients from the sweep as well as UniFi.",
            );
        });
        ui.separator();

        let unifi = src.export.unifi();
        let sonicwall = src.export.sonicwall();
        let sweep = src.sweep.finished();
        let runs = src.log.entries.len();
        let targets = log_targets(src.log);
        ui.label(RichText::new("Includes").strong());
        egui::Grid::new("bundle_parts")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                part_row(
                    ui,
                    &mut self.use_unifi,
                    "UniFi backup",
                    unifi.map(|(p, r)| {
                        format!("{} device(s) · {}", r.count("device"), export::file_name(p))
                    }),
                    "Open a UniFi backup in Config Export",
                );
                part_row(
                    ui,
                    &mut self.use_sonicwall,
                    "SonicWall export",
                    sonicwall.map(|(p, r)| {
                        format!(
                            "{} interface(s) · {}",
                            r.count("interfaces"),
                            export::file_name(p)
                        )
                    }),
                    "Open a SonicWall .exp in Config Export",
                );
                part_row(
                    ui,
                    &mut self.use_inventory,
                    "Sweep inventory",
                    sweep
                        .as_ref()
                        .map(|(range, n)| format!("{n} live host(s) · {range}")),
                    "Run a Subnet Sweep",
                );
                part_row(
                    ui,
                    &mut self.use_results,
                    "Results log",
                    (runs > 0).then(|| match targets.is_empty() {
                        true => format!("{runs} run(s)"),
                        false => format!("{runs} run(s): {}", targets.join(", ")),
                    }),
                    "Nothing in Results yet",
                );
            });
        let chosen = (self.use_unifi && unifi.is_some())
            || (self.use_sonicwall && sonicwall.is_some())
            || (self.use_inventory && sweep.is_some())
            || (self.use_results && runs > 0);
        widgets::error(ui, &self.err);
        ui.separator();

        ui.horizontal(|ui| {
            let save = ui
                .add_enabled(chosen && !running, egui::Button::new("Save bundle…"))
                .on_hover_text("Save the zip, then copy the kickoff prompt to the clipboard")
                .on_disabled_hover_text("Nothing to bundle yet");
            if save.clicked() {
                self.save(ui.ctx(), src);
            }
            if running {
                ui.spinner();
            }
            if let Some(s) = &self.saved {
                if ui
                    .button("Copy kickoff prompt")
                    .on_hover_text(s.kickoff.as_str())
                    .clicked()
                {
                    ui.ctx().copy_text(s.kickoff.clone());
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "Saved {}; prompt copied",
                            export::file_name(&s.path)
                        ))
                        .color(widgets::OK),
                    )
                    .truncate(),
                )
                .on_hover_text(s.path.display().to_string());
            }
        });
        ui.weak("Attach the zip in Claude and paste the prompt.");
    }

    /// Gather the parts and save off the UI thread. The UniFi backup is
    /// converted again so the bundle's client setting applies.
    fn save(&mut self, ctx: &egui::Context, src: &Sources) {
        self.err = None;
        self.saved = None;
        let unifi = self
            .use_unifi
            .then(|| src.export.unifi().map(|(p, _)| p.clone()))
            .flatten();
        let mut parts = Vec::new();
        if self.use_sonicwall
            && let Some((p, r)) = src.export.sonicwall()
        {
            parts.push(Part {
                file: bundle::SONICWALL,
                source: export::file_name(p),
                skill: "/mapnetwork",
                what: SONICWALL_WHAT,
                summary: json!({
                    "system": r.system.iter().map(|(k, v)| (k.clone(), json!(v)))
                        .collect::<serde_json::Map<_, _>>(),
                    "interfaces": r.count("interfaces"),
                    "address_objects": r.count("address objects"),
                    "access_rules": r.count("access rules"),
                    "nat_rules": r.count("NAT rules"),
                }),
                data: r.json.clone(),
            });
        }
        if self.use_inventory
            && let Some((range, n)) = src.sweep.finished()
        {
            parts.push(Part {
                file: bundle::INVENTORY,
                source: range.clone(),
                skill: "/mapnetwork",
                what: INVENTORY_WHAT,
                summary: json!({
                    "range": range,
                    "live_hosts": n,
                    "ports_checked": src.sweep.checked_ports(),
                }),
                data: src.sweep.inventory(src.info),
            });
        }
        if self.use_results && !src.log.entries.is_empty() {
            let entries: Vec<_> = src.log.entries.iter().collect();
            let mut tools: Vec<&str> = entries.iter().map(|e| e.tool).collect();
            tools.dedup();
            parts.push(Part {
                file: bundle::RESULTS,
                source: "Results tab".into(),
                skill: "/mapnetwork",
                what: RESULTS_WHAT,
                summary: json!({ "runs": entries.len(), "tools": tools }),
                data: Format::Json.export(&entries),
            });
        }
        let b = Bundle {
            site: self.site.trim().to_string(),
            notes: self.notes.clone(),
            clients: self.clients,
            map_clients: self.map_clients,
            scanner: src.info.clone(),
            parts,
        };
        self.job = Some(Job::spawn(ctx, move |tx| {
            tx.send(build_and_save(unifi, b));
        }));
    }
}

/// Add the UniFi part (converted with the bundle's client setting), then
/// ask where to save and write the zip.
fn build_and_save(unifi: Option<PathBuf>, mut b: Bundle) -> Result<Option<Saved>, String> {
    let clients = b.clients;
    if let Some(path) = unifi {
        let data =
            std::fs::read(&path).map_err(|e| format!("{}: {e}", export::file_name(&path)))?;
        let r = unifi::convert(data, clients)?;
        b.parts.insert(
            0,
            Part {
                file: bundle::UNIFI,
                source: export::file_name(&path),
                skill: "/unifidump",
                what: UNIFI_WHAT,
                summary: json!({
                    "unifi_network": r.version,
                    "sites": r.sites,
                    "devices": r.count("device"),
                    "networks": r.count("networkconf"),
                    "wlans": r.count("wlanconf"),
                    "clients": r.count(unifi::CLIENTS),
                    "client_mode": clients.label(),
                }),
                data: r.json,
            },
        );
    }
    // Ask first: the prompt inside the zip names the file as saved.
    let Some(path) = rfd::FileDialog::new()
        .set_title("Save site bundle")
        .set_file_name(format!("{}.bundle.zip", bundle::slug(&b.site)))
        .add_filter("Zip", &["zip"])
        .save_file()
    else {
        return Ok(None);
    };
    let name = export::file_name(&path);
    let zip = b.zip(&name).map_err(|e| e.to_string())?;
    std::fs::write(&path, zip).map_err(|e| format!("{}: {e}", path.display()))?;
    let kickoff = b.kickoff(&name);
    Ok(Some(Saved { path, kickoff }))
}

/// Distinct targets in the Results log, in run order, so it's plain whether
/// every run is this site's before the log goes in.
fn log_targets(log: &Log) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in log.entries.iter().filter_map(|e| e.target()) {
        if !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    }
    out
}

/// Checkbox, name and what's loaded; greyed out with a hint when there's
/// nothing to include.
fn part_row(ui: &mut Ui, on: &mut bool, name: &str, loaded: Option<String>, missing: &str) {
    match loaded {
        Some(detail) => {
            ui.checkbox(on, name);
            ui.add(egui::Label::new(RichText::new(&detail).weak()).truncate())
                .on_hover_text(detail);
        }
        None => {
            ui.add_enabled(false, egui::Checkbox::new(&mut false, name));
            ui.weak(missing);
        }
    }
    ui.end_row();
}
