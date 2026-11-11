use crate::bundle::{self, Bundle, Part};
use crate::job::{Job, pump};
use crate::log::{Format, Log};
use crate::net::hostinfo::HostInfo;
use crate::tools::export::{self, ExportTool};
use crate::tools::notes::NotesTool;
use crate::tools::sweep::SweepTool;
use crate::unifi::{self, Clients};
use crate::{meraki, sonicwall, widgets};
use eframe::egui::{self, RichText, Ui};
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// What the bundle can draw on, borrowed from the other tabs each frame.
pub struct Sources<'a> {
    pub export: &'a ExportTool,
    pub sweep: &'a SweepTool,
    pub log: &'a Log,
    pub info: &'a HostInfo,
    pub notes: &'a NotesTool,
    /// The live map, for the notes on its cards.
    pub map: &'a crate::map::view::MapTabs,
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
    /// Backups and exports open in Config Export that were unticked: each
    /// goes in unless it's here.
    left_out: HashSet<PathBuf>,
    use_inventory: bool,
    use_results: bool,
    use_notes: bool,
    use_card_notes: bool,
    use_map: bool,
    /// Carry the /unifidump and /mapnetwork skills (`skills/` in the zip).
    use_skills: bool,
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
            left_out: HashSet::new(),
            use_inventory: true,
            use_results: false,
            use_notes: false,
            use_card_notes: true,
            use_map: true,
            use_skills: true,
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
// The second and later of a kind (`unifi-2.unifidump.json`, …).
const UNIFI_MORE_WHAT: &str = "Another UniFi controller's export from this visit, in the same form \
as unifi.unifidump.json: its own sites, devices, networks, switch port profiles and WLANs (plus clients \
as the manifest says), secrets removed. Run /unifidump on it too; its sites are its own.";
const SONICWALL_MORE_WHAT: &str = "Another SonicWall's config from this visit, in the same form as \
sonicwall.json: a separate firewall, a gateway of its own, with its own interfaces, zones, objects and \
rules. Enforced config, not observation.";
const MERAKI_MORE_WHAT: &str = "Another Meraki export from this visit, in the same form as \
meraki.json: its own networks and MX appliances, each a gateway of its own. Enforced config, not \
observation.";
const INVENTORY_WHAT: &str = "Ping sweep from the scanner: live hosts with MAC, IEEE vendor, name (each \
with where it came from: mac_from, name_from), TTL and open management ports. Observed state.";
const RESULTS_WHAT: &str = "is-alive test runs (ping, port check, traceroute, DNS, mail health) with \
start and end times. A site keeps its runs from one visit to the next, so they can span visits: judge each \
check by its newest run.";
const PROBLEMS_ID: &str = "bundle_known_problems";

const MAP_WHAT: &str = "The /mapnetwork map from an earlier visit, as it was opened here. \
Update it rather than starting over: keep what still holds, change what the other files show.";
const CARD_NOTES_WHAT: &str = "The technician's notes on cards of the Map tab's live map: each \
card's name, role, IP and MAC with its note. Sanitized on their machine like notes.md. Observations \
about those devices; carry what bears on the map into its device notes and findings.";
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
                let name = export::file_name(&saved.path);
                widgets::toast(format!("Saved {name}; kickoff prompt copied"), true);
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
    /// Another site opened: its name, and nothing typed for the last one.
    pub fn for_site(&mut self, name: Option<&str>) {
        self.site = name
            .map(|n| crate::input::line(n, crate::input::Field::Text.max()))
            .unwrap_or_default();
        self.notes.clear();
        self.ov = Default::default();
        self.left_out.clear();
        self.saved = None;
        self.err = None;
    }

    pub fn window(&mut self, ctx: &egui::Context, src: Sources) {
        let running = self.poll(ctx);
        if !self.open {
            return;
        }
        if self.site.trim().is_empty()
            && let Some((_, r)) = src.export.unifi()
            && let Some(name) = r.sites.first()
        {
            // From the backup: cleaned and cut like a typed name.
            self.site = crate::input::line(name, crate::input::Field::Text.max());
        }
        let mut open = self.open;
        widgets::compact_title(ctx, || {
            egui::Window::new("Site Bundle")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(560.0)
                .show(ctx, |ui| self.body(ui, &src, running));
        });
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
                widgets::line(
                    ui,
                    crate::input::Field::Text,
                    &mut self.site,
                    380.0,
                    "Client or site name",
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
                    let keep = widgets::selection_before_right_click(ui.ctx(), id);
                    let edit = egui::TextEdit::multiline(&mut self.notes)
                        .id(id)
                        .char_limit(crate::input::Field::Problems.room())
                        .desired_rows(2)
                        .desired_width(380.0)
                        .hint_text("Optional. Goes straight into /mapnetwork's findings.")
                        .layouter(&mut layouter)
                        .show(ui);
                    widgets::tidy(
                        ui.ctx(),
                        &edit.response,
                        crate::input::Field::Problems,
                        &mut self.notes,
                    );
                    widgets::restore_selection(ui.ctx(), id, &edit.response, keep);
                    edit.response.context_menu(|ui| {
                        if widgets::text_edit_menu(ui, id, true) {
                            ui.close();
                        }
                    });
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

        let configs = configs(src.export);
        let map = src.export.mapnetwork();
        let sweep = src.sweep.finished();
        let runs = src.log.entries.len();
        let targets = log_targets(src.log);
        ui.label(RichText::new("Includes").strong());
        // Every backup and export open is a row: the list scrolls before the
        // window outgrows a laptop's (at 720 high, about eleven rows show).
        let room = (ui.ctx().content_rect().height() - 370.0).max(180.0);
        egui::ScrollArea::vertical()
            .id_salt("bundle_parts_scroll")
            .max_height(room)
            .show(ui, |ui| {
                // Rows a trackpad can hit.
                ui.spacing_mut().interact_size.y = ROW_H;
                egui::Grid::new("bundle_parts")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| self.rows(ui, src, &configs, &targets));
            });
        // The map alone is enough: Claude reviews and updates it.
        let chosen = (self.use_map && map.is_some())
            || configs
                .iter()
                .any(|c| c.files.iter().any(|f| !self.left_out.contains(f.path)))
            || (self.use_inventory && sweep.is_some())
            || (self.use_results && runs > 0)
            || (self.use_notes && !src.notes.is_empty())
            || (self.use_card_notes && src.map.bundle_notes(src.notes.overrides()).is_some());
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
        ui.weak("Attach the zip in a Claude chat and paste the prompt.");
    }

    /// The Includes list: a row for each backup and export open (or, with
    /// none of a kind, what to open), then the rest.
    fn rows(&mut self, ui: &mut Ui, src: &Sources, configs: &[Configs], targets: &[String]) {
        for c in configs {
            if c.files.is_empty() {
                part_row(ui, &mut false, c.name, None, c.missing);
            }
            let mut n = 0;
            for f in &c.files {
                let on = !self.left_out.contains(f.path);
                let file = on.then(|| {
                    n += 1;
                    bundle::numbered(c.file, n)
                });
                if file_row(ui, on, f.path, c.name, &f.detail, file.as_deref()) {
                    if on {
                        self.left_out.insert(f.path.clone());
                    } else {
                        self.left_out.remove(f.path);
                    }
                }
            }
        }
        let map = src.export.mapnetwork();
        let sweep = src.sweep.finished();
        let runs = src.log.entries.len();
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
        let card_notes = src.map.bundle_notes(src.notes.overrides());
        part_row(
            ui,
            &mut self.use_card_notes,
            "Card notes",
            card_notes
                .as_ref()
                .map(|(n, _)| format!("{n} card(s) on the live map · redacted")),
            "No notes on the live map's cards",
        );
        part_row(
            ui,
            &mut self.use_skills,
            "Claude Skills",
            Some(format!(
                "/unifidump and /mapnetwork · {} KB, for a Claude without them installed",
                crate::skills::bytes() / 1024
            )),
            "",
        );
    }

    /// Gather the parts and save off the UI thread. The UniFi backups are
    /// converted again so the bundle's client setting applies.
    fn save(&mut self, ctx: &egui::Context, src: &Sources) {
        self.err = None;
        self.saved = None;
        let included = |path: &&PathBuf| !self.left_out.contains(*path);
        let unifi: Vec<PathBuf> = src
            .export
            .unifis()
            .map(|(p, _)| p)
            .filter(included)
            .cloned()
            .collect();
        let mut parts = Vec::new();
        let sonicwalls = src.export.sonicwalls().filter(|(p, _)| included(p));
        for (n, (p, r)) in sonicwalls.enumerate() {
            parts.push(sonicwall_part(n + 1, p, r));
        }
        let merakis = src.export.merakis().filter(|(p, _)| included(p));
        for (n, (p, r)) in merakis.enumerate() {
            parts.push(meraki_part(n + 1, p, r));
        }
        if self.use_map
            && let Some((p, d)) = src.export.mapnetwork()
        {
            parts.push(Part {
                file: bundle::MAP.into(),
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
                file: bundle::INVENTORY.into(),
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
                file: bundle::RESULTS.into(),
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
                file: bundle::NOTES.into(),
                source: "Notes tab".into(),
                skill: "/mapnetwork",
                what: NOTES_WHAT,
                summary: json!({ "lines": clean.lines().count(), "redacted": hits.len() }),
                // Without the emoji style marks, like Copy and Save.
                data: crate::theme::strip_emoji_styles(&clean),
            });
        }
        if self.use_card_notes
            && let Some((n, data)) = src.map.bundle_notes(src.notes.overrides())
        {
            parts.push(Part {
                file: bundle::CARD_NOTES.into(),
                source: "Map tab".into(),
                skill: "/mapnetwork",
                what: CARD_NOTES_WHAT,
                summary: json!({ "cards": n }),
                data,
            });
        }
        let b = Bundle {
            site: self.site.trim().to_string(),
            // Same rules as the notes: nothing redactable goes in the zip,
            // nor emoji style marks pasted in from them.
            notes: crate::theme::strip_emoji_styles(
                &crate::redact::sanitize_with(&self.notes, &src.notes.overrides().merged(&self.ov))
                    .0,
            ),
            clients: self.clients,
            map_clients: self.map_clients,
            scanner: src.info.clone(),
            parts,
            skills: self.use_skills,
        };
        self.job = Some(Job::spawn(ctx, move |tx| {
            tx.send(build_and_save(unifi, b));
        }));
    }
}

/// Add the UniFi parts (each converted with the bundle's client setting, in
/// the order opened), then ask where to save and write the zip.
fn build_and_save(unifi: Vec<PathBuf>, mut b: Bundle) -> Result<Option<Saved>, String> {
    for (n, path) in unifi.iter().enumerate() {
        let data = export::read_input(path)?;
        let r = unifi::convert(data, b.clients)?;
        b.parts.insert(n, unifi_part(n + 1, path, r));
    }
    // Ask first: the prompt inside the zip names the file as saved.
    let Some(path) = widgets::save_dialog()
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

/// The `n`th UniFi backup's part (counting from 1), `r` its conversion
/// with the bundle's client setting.
fn unifi_part(n: usize, path: &Path, r: unifi::Report) -> Part {
    Part {
        file: bundle::numbered(bundle::UNIFI, n),
        source: export::file_name(path),
        skill: "/unifidump",
        what: if n == 1 { UNIFI_WHAT } else { UNIFI_MORE_WHAT },
        summary: json!({
            "unifi_network": r.version,
            "sites": r.sites,
            "devices": r.count("device"),
            "networks": r.count("networkconf"),
            "wlans": r.count("wlanconf"),
            "clients": r.count(unifi::CLIENTS),
            "client_mode": r.clients.label(),
        }),
        data: r.json,
    }
}

/// The `n`th SonicWall export's part, counting from 1.
fn sonicwall_part(n: usize, path: &Path, r: &sonicwall::Report) -> Part {
    Part {
        file: bundle::numbered(bundle::SONICWALL, n),
        source: export::file_name(path),
        skill: "/mapnetwork",
        what: if n == 1 {
            SONICWALL_WHAT
        } else {
            SONICWALL_MORE_WHAT
        },
        summary: json!({
            "system": r.system.iter().map(|(k, v)| (k.clone(), json!(v)))
                .collect::<serde_json::Map<_, _>>(),
            "interfaces": r.count("interfaces"),
            "address_objects": r.count("address objects"),
            "access_rules": r.count("access rules"),
            "nat_rules": r.count("NAT rules"),
        }),
        data: r.json.clone(),
    }
}

/// The `n`th Meraki export's part, counting from 1.
fn meraki_part(n: usize, path: &Path, r: &meraki::Report) -> Part {
    Part {
        file: bundle::numbered(bundle::MERAKI, n),
        source: export::file_name(path),
        skill: "/mapnetwork",
        what: if n == 1 {
            MERAKI_WHAT
        } else {
            MERAKI_MORE_WHAT
        },
        summary: json!({
            "system": r.system.iter().map(|(k, v)| (k.clone(), json!(v)))
                .collect::<serde_json::Map<_, _>>(),
            "networks": r.count("networks"),
            "vlans": r.count("VLANs"),
            "firewall_rules": r.count("firewall rules"),
            "port_forwards": r.count("port forwards"),
        }),
        data: r.json.clone(),
    }
}

/// A backup or export open in Config Export, as the bundle lists it.
struct Config<'a> {
    path: &'a PathBuf,
    /// What it holds: "12 device(s)".
    detail: String,
}

/// One kind of backup or export, and every one of it open.
struct Configs<'a> {
    /// The row's name: "UniFi backup".
    name: &'static str,
    /// The first one's name in the zip; the next are numbered from it.
    file: &'static str,
    /// What to do to have one.
    missing: &'static str,
    files: Vec<Config<'a>>,
}

/// The UniFi backups, SonicWall exports and Meraki exports open, each kind
/// in the order opened.
fn configs(e: &ExportTool) -> [Configs<'_>; 3] {
    [
        Configs {
            name: "UniFi backup",
            file: bundle::UNIFI,
            missing: "Open a UniFi backup in Config Export",
            files: e
                .unifis()
                .map(|(path, r)| Config {
                    path,
                    detail: format!("{} device(s)", r.count("device")),
                })
                .collect(),
        },
        Configs {
            name: "SonicWall export",
            file: bundle::SONICWALL,
            missing: "Open a SonicWall .exp or .wri in Config Export",
            files: e
                .sonicwalls()
                .map(|(path, r)| Config {
                    path,
                    detail: format!("{} interface(s)", r.count("interfaces")),
                })
                .collect(),
        },
        Configs {
            name: "Meraki export",
            file: bundle::MERAKI,
            missing: "Open a Meraki API export (.json) in Config Export",
            files: e
                .merakis()
                .map(|(path, r)| Config {
                    path,
                    detail: format!(
                        "{} network(s) · {} VLAN(s)",
                        r.count("networks"),
                        r.count("VLANs")
                    ),
                })
                .collect(),
        },
    ]
}

/// The Includes list's row height: a click target a trackpad can hit.
const ROW_H: f32 = 28.0;

/// Longest file name a row shows whole; past it, the middle goes.
const NAME_MAX: usize = 30;

/// `name` cut to `NAME_MAX` characters in the middle, so its start and its
/// extension both show.
fn short_name(name: &str) -> String {
    let n = name.chars().count();
    if n <= NAME_MAX {
        return name.to_string();
    }
    let tail = 12;
    let head: String = name.chars().take(NAME_MAX - tail - 1).collect();
    let end: String = name.chars().skip(n - tail).collect();
    format!("{head}…{end}")
}

/// A backup or export's row: a checkbox named after its file, then what it
/// is and the name it goes in the zip as (`file`; None while it's left
/// out). True when clicked.
fn file_row(
    ui: &mut Ui,
    mut on: bool,
    path: &Path,
    kind: &str,
    detail: &str,
    file: Option<&str>,
) -> bool {
    let name = export::file_name(path);
    let clicked = ui
        .checkbox(&mut on, short_name(&name))
        .on_hover_text(path.display().to_string())
        .clicked();
    let detail = match file {
        Some(f) => format!("{kind} · {detail} → {f}"),
        None => format!("{kind} · {detail} · left out"),
    };
    ui.add(egui::Label::new(RichText::new(&detail).weak()).truncate())
        .on_hover_text(detail);
    ui.end_row();
    clicked
}

/// Distinct targets in the Results log, in run order, so it's plain whether
/// every run is this site's before the log goes in.
pub(crate) fn log_targets(log: &Log) -> Vec<String> {
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
pub(crate) fn part_row(
    ui: &mut Ui,
    on: &mut bool,
    name: &str,
    loaded: Option<String>,
    missing: &str,
) {
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

    fn unifi_report(site: &str) -> unifi::Report {
        unifi::Report {
            layers: vec!["unf".into()],
            version: Some("9.0.114".into()),
            sites: vec![site.into()],
            kept: vec![("device".into(), 3)],
            dropped: Default::default(),
            redacted: Default::default(),
            clients: Clients::Anonymized,
            anonymized: 0,
            json: format!("[{{\"site\":\"{site}\"}}]"),
            secrets: Vec::new(),
        }
    }

    fn sonicwall_report(name: &str) -> sonicwall::Report {
        let exp = format!(
            "{}shortProdName=TZ%20270&firewallName={name}",
            (0..12)
                .map(|i| format!("iface_name_{i}=X{i}&interface_Zone_{i}=LAN&"))
                .collect::<String>()
        );
        sonicwall::convert(exp.as_bytes()).expect("a settings export")
    }

    /// Two sites' backups and firewall reports go in as four parts, in the
    /// order opened, the first of each kind under its usual name.
    #[test]
    fn every_backup_and_export_is_a_part_of_its_own() {
        let p = |s: &str| PathBuf::from(format!("/visit/{s}"));
        let (fw_a, fw_b) = (sonicwall_report("fw-north"), sonicwall_report("fw-south"));
        let mut parts = vec![
            sonicwall_part(1, &p("fw-north.wri"), &fw_a),
            sonicwall_part(2, &p("fw-south.wri"), &fw_b),
        ];
        // As build_and_save adds them: first, in the order opened.
        for (n, (file, site)) in [("north.unf", "North"), ("south.unf", "South")]
            .into_iter()
            .enumerate()
        {
            parts.insert(n, unifi_part(n + 1, &p(file), unifi_report(site)));
        }
        let got: Vec<(&str, &str, &str)> = parts
            .iter()
            .map(|x| (x.file.as_str(), x.source.as_str(), x.skill))
            .collect();
        assert_eq!(
            got,
            [
                ("unifi.unifidump.json", "north.unf", "/unifidump"),
                ("unifi-2.unifidump.json", "south.unf", "/unifidump"),
                ("sonicwall.json", "fw-north.wri", "/mapnetwork"),
                ("sonicwall-2.json", "fw-south.wri", "/mapnetwork"),
            ]
        );
        assert!(parts[1].data.contains("South") && parts[3].data.contains("fw-south"));
        // The first of a kind reads as it always has; the next says it's
        // another controller or firewall.
        assert_eq!(parts[0].what, UNIFI_WHAT);
        assert_eq!(parts[2].what, SONICWALL_WHAT);
        assert!(parts[1].what.starts_with("Another UniFi controller"));
        assert!(parts[3].what.starts_with("Another SonicWall"));
        assert_eq!(parts[1].summary["sites"], json!(["South"]));
        let b = Bundle {
            site: "Northwind".into(),
            notes: String::new(),
            clients: Clients::Anonymized,
            map_clients: false,
            scanner: HostInfo::default(),
            parts,
            skills: false,
        };
        let k = b.kickoff("northwind.bundle.zip");
        for f in [
            "unifi.unifidump.json",
            "unifi-2.unifidump.json",
            "sonicwall.json",
            "sonicwall-2.json",
        ] {
            assert!(k.contains(f), "{f} missing from: {k}");
        }
    }

    #[test]
    fn long_file_names_keep_their_start_and_end() {
        assert_eq!(short_name("fw-north.wri"), "fw-north.wri");
        let long = "network_backup_10.11.2026_14-05_v9.0.114.unf";
        let short = short_name(long);
        assert_eq!(short.chars().count(), NAME_MAX);
        assert_eq!(short, "network_backup_10…v9.0.114.unf");
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
