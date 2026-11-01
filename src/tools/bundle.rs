use crate::bundle::{self, Bundle, Part};
use crate::job::{Job, pump};
use crate::log::{Format, Log};
use crate::net::hostinfo::HostInfo;
use crate::tools::export::{self, ExportTool};
use crate::tools::notes::NotesTool;
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
    pub notes: &'a NotesTool,
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
    use_meraki: bool,
    use_inventory: bool,
    use_results: bool,
    use_notes: bool,
    use_map: bool,
    /// Redact and keep calls made in Known problems, on top of the Notes
    /// tab's. In memory only.
    ov: crate::redact::Overrides,
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
            use_meraki: true,
            use_inventory: true,
            use_results: false,
            use_notes: false,
            use_map: true,
            ov: Default::default(),
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
const MERAKI_WHAT: &str = "Meraki MX config from the Dashboard API: networks, devices, VLANs, \
appliance ports, firewall, port forwarding and NAT rules, static routes, VPN and uplinks. Enforced config, \
not observation.";
const INVENTORY_WHAT: &str = "Ping sweep from the scanner: live hosts with MAC, IEEE vendor, name, TTL \
and open management ports. Observed state.";
const RESULTS_WHAT: &str =
    "is-alive test runs (ping, port check, traceroute, DNS, mail health) with start and end times.";
const PROBLEMS_ID: &str = "bundle_known_problems";

const MAP_WHAT: &str = "The /mapnetwork map from an earlier visit, as it was opened here. \
Update it rather than starting over: keep what still holds, change what the other files show.";
const NOTES_WHAT: &str = "The technician's notes from the Notes tab (Markdown). Sanitized on their \
machine: passwords, keys, tokens and patient details are [🔑 REDACTED: …] markers. Context and \
observations; use what bears on the map and its findings.";

impl BundleTool {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    fn poll(&mut self, ctx: &egui::Context) -> bool {
        pump(&mut self.job, |r| match r {
            Ok(Some(saved)) => {
                widgets::copy(ctx, saved.kickoff.clone());
                widgets::toast(
                    format!(
                        "Saved {}; kickoff prompt copied",
                        export::file_name(&saved.path)
                    ),
                    true,
                );
                self.saved = Some(saved);
            }
            Ok(None) => {}
            Err(e) => {
                widgets::toast("Bundle not saved", false);
                self.err = Some(e);
            }
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
        ui.weak("One zip of everything so far, for /unifidump and /mapnetwork.");
        ui.add_space(4.0);
        egui::Grid::new("bundle_form")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Site name");
                ui.add(
                    egui::TextEdit::singleline(&mut self.site)
                        .char_limit(widgets::FIELD_MAX)
                        .desired_width(380.0)
                        .hint_text("Client or site name"),
                );
                ui.end_row();
                ui.label("Known problems");
                ui.vertical(|ui| {
                    let ov = src.notes.overrides().merged(&self.ov);
                    let id = egui::Id::new(PROBLEMS_ID);
                    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                        let text = buf.as_str();
                        let spans: Vec<_> = crate::redact::find_with(text, &ov)
                            .into_iter()
                            .map(|h| h.span)
                            .collect();
                        let mut job = crate::tools::notes::highlight(
                            ui,
                            text,
                            &spans,
                            &eframe::egui::FontId::monospace(13.0),
                        );
                        job.wrap.max_width = wrap;
                        ui.fonts_mut(|f| f.layout_job(job))
                    };
                    let edit = egui::TextEdit::multiline(&mut self.notes)
                        .id(id)
                        .char_limit(widgets::PROBLEMS_MAX)
                        .desired_rows(2)
                        .desired_width(380.0)
                        .hint_text("Optional. Goes straight into /mapnetwork's findings.")
                        .layouter(&mut layouter)
                        .show(ui);
                    let hits = crate::redact::find_with(&self.notes, &ov);
                    let pick = widgets::text_selection(ui.ctx(), id, &self.notes)
                        .and_then(|sel| crate::redact::pick(&self.notes, sel, &hits));
                    let shortcut = edit.response.has_focus()
                        && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::R));
                    ui.horizontal(|ui| {
                        let label = match &pick {
                            Some(crate::redact::Pick::Unredact(_)) => "Unredact",
                            _ => "Redact",
                        };
                        let clicked = ui
                            .add_enabled(pick.is_some(), egui::Button::new(label).small())
                            .on_hover_text(
                                "Hide the selected text (Ctrl+R). Notes' redactions apply here too",
                            )
                            .clicked();
                        if (clicked || shortcut)
                            && let Some(p) = pick
                        {
                            match self.ov.apply(&self.notes, p) {
                                Ok(m) => widgets::toast(m, true),
                                Err(m) => widgets::toast(m, false),
                            }
                        }
                        if !hits.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "⚠ {} before bundling",
                                    crate::redact::summary(&hits)
                                ))
                                .small()
                                .color(widgets::warn()),
                            );
                        }
                    });
                });
                ui.end_row();
            });
        ui.horizontal(|ui| {
            export::clients_choice(ui, "bundle_clients", &mut self.clients);
            ui.separator();
            ui.checkbox(&mut self.map_clients, "Clients on the map")
                .on_hover_text("Put client devices on the map (off: infrastructure only)");
        });
        ui.separator();

        let unifi = src.export.unifi();
        let sonicwall = src.export.sonicwall();
        let meraki = src.export.meraki();
        let map = src.export.mapnetwork();
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
                    "Open a SonicWall .exp or .wri in Config Export",
                );
                part_row(
                    ui,
                    &mut self.use_meraki,
                    "Meraki export",
                    meraki.map(|(p, r)| {
                        format!(
                            "{} network(s) · {} VLAN(s) · {}",
                            r.count("networks"),
                            r.count("VLANs"),
                            export::file_name(p)
                        )
                    }),
                    "Open a Meraki API export (.json) in Config Export",
                );
                part_row(
                    ui,
                    &mut self.use_map,
                    "Current map",
                    map.map(|(p, d)| {
                        format!(
                            "{} device(s) · {} · updated, not rebuilt",
                            d.nodes.len(),
                            export::file_name(p)
                        )
                    }),
                    "Open the site's /mapnetwork .dc.html in Config Export to have it updated",
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
                let (_, hits) = src.notes.sanitized();
                part_row(
                    ui,
                    &mut self.use_notes,
                    "Tech notes",
                    (!src.notes.is_empty()).then(|| match hits.len() {
                        0 => "notes.md · nothing redacted".to_string(),
                        _ => format!("notes.md · {}", crate::redact::summary(&hits)),
                    }),
                    "Nothing in Notes yet",
                );
            });
        let chosen = (self.use_unifi && unifi.is_some())
            || (self.use_sonicwall && sonicwall.is_some())
            || (self.use_meraki && meraki.is_some())
            || (self.use_inventory && sweep.is_some())
            || (self.use_results && runs > 0)
            || (self.use_notes && !src.notes.is_empty());
        widgets::error(ui, &self.err);
        ui.separator();

        ui.horizontal(|ui| {
            let save = ui
                .add_enabled(chosen && !running, egui::Button::new("Save bundle…"))
                .on_hover_text("Save the zip and copy the prompt")
                .on_disabled_hover_text("Nothing to bundle yet");
            if save.clicked() {
                self.save(ui.ctx(), src);
            }
            if running {
                widgets::spinner(ui);
            }
            if let Some(s) = &self.saved {
                if ui
                    .button("Copy kickoff prompt")
                    .on_hover_text(s.kickoff.as_str())
                    .clicked()
                {
                    widgets::copy(ui.ctx(), s.kickoff.clone());
                    widgets::toast("Kickoff prompt copied", true);
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "Saved {}; prompt copied",
                            export::file_name(&s.path)
                        ))
                        .color(widgets::ok()),
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
        if self.use_meraki
            && let Some((p, r)) = src.export.meraki()
        {
            parts.push(Part {
                file: bundle::MERAKI,
                source: export::file_name(p),
                skill: "/mapnetwork",
                what: MERAKI_WHAT,
                summary: json!({
                    "system": r.system.iter().map(|(k, v)| (k.clone(), json!(v)))
                        .collect::<serde_json::Map<_, _>>(),
                    "networks": r.count("networks"),
                    "vlans": r.count("VLANs"),
                    "firewall_rules": r.count("firewall rules"),
                    "port_forwards": r.count("port forwards"),
                }),
                data: r.json.clone(),
            });
        }
        if self.use_map
            && let Some((p, d)) = src.export.mapnetwork()
        {
            parts.push(Part {
                file: bundle::MAP,
                source: export::file_name(p),
                skill: "/mapnetwork",
                what: MAP_WHAT,
                summary: json!({
                    "title": d.title,
                    "devices": d.nodes.len(),
                    "links": d.edges.len(),
                    "findings": d.actions.len(),
                }),
                data: d.html.clone(),
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
        if self.use_notes && !src.notes.is_empty() {
            let (clean, hits) = src.notes.sanitized();
            parts.push(Part {
                file: bundle::NOTES,
                source: "Notes tab".into(),
                skill: "/mapnetwork",
                what: NOTES_WHAT,
                summary: json!({ "lines": clean.lines().count(), "redacted": hits.len() }),
                data: clean,
            });
        }
        let b = Bundle {
            site: self.site.trim().to_string(),
            // Same rules as the notes: nothing redactable goes in the zip.
            notes: crate::redact::sanitize_with(
                &self.notes,
                &src.notes.overrides().merged(&self.ov),
            )
            .0,
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
        let data = export::read_input(&path)?;
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
    let path = widgets::safe_path(path, &["zip"]);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::{Entry, Table};

    fn run(tool: &'static str, first: &'static str, target: &str) -> Entry {
        Entry {
            tool,
            started: "s".into(),
            ended: "e".into(),
            text: String::new(),
            table: Some(Table {
                columns: match first {
                    "host" => &["host", "state"],
                    "range" => &["range", "ip"],
                    _ => &["field", "value"],
                },
                rows: vec![vec![target.into(), "x".into()]],
            }),
        }
    }

    #[test]
    fn log_targets_are_distinct_in_run_order() {
        let mut log = Log::default();
        for e in [
            run("Ping", "host", "10.0.0.1"),
            run("Subnet Sweep", "range", "10.0.0.0/24"),
            run("Port Check", "host", "10.0.0.1"),
            run("Subnet Calc", "field", "ignored"),
            run("Ping", "host", "192.168.1.1"),
        ] {
            log.push(e);
        }
        assert_eq!(
            log_targets(&log),
            ["10.0.0.1", "10.0.0.0/24", "192.168.1.1"]
        );
    }
}
