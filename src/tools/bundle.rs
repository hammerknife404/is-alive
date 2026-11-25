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
use std::collections::{HashMap, HashSet};
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

/// What the wizard asks the app to do: it only reads the tools.
pub enum Ask {
    /// Open Config Export's file dialog.
    OpenConfigs,
    /// Sweep this range.
    Sweep(String),
}

/// The wizard's steps, in order.
const STEPS: [&str; 5] = ["Site", "Configs", "Sweep", "Notes", "Save"];

pub struct BundleTool {
    pub open: bool,
    /// The wizard's step (index into STEPS).
    step: usize,
    /// The range step 3 sweeps; filled from this computer's LAN.
    range: String,
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
    /// The last look for unredacted secrets in each part: (hash of its
    /// texts, kinds found). The rules run over up to a million
    /// characters, so not every frame.
    looked: std::cell::RefCell<HashMap<&'static str, (u64, Option<String>)>>,
}

impl Default for BundleTool {
    fn default() -> Self {
        Self {
            open: false,
            step: 0,
            range: String::new(),
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
            looked: Default::default(),
        }
    }
}

const UNIFI_WHAT: &str = "UniFi controller export: sites, devices, networks, switch port profiles and \
WLANs (plus clients as the manifest says), secrets removed. mongoexport-style array for /unifidump's extract.py.";
const SONICWALL_WHAT: &str = "SonicWall config: interfaces (zone, VLAN, IP, MAC, management), zones, \
address and service objects, access and NAT rules. Enforced config, not observation.";
const MERAKI_WHAT: &str = "Meraki MX config from the Dashboard API: networks, devices, VLANs (with \
DHCP reservations: the names of phones', laptops' and PCs' follow client_names in its summary), \
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
const INVENTORY_WHAT: &str = "Ping sweep from the scanner: live hosts with how each was found (found_by: \
ping, arp, mdns, ssdp; a host with only mdns or ssdp answered nothing but the multicast browse and is listed because \
it's in the swept range on the scanner's own segment), MAC, IEEE vendor, name (each \
with where it came from: mac_from, name_from), whether a locally administered MAC is a client's private \
one (private_mac) or the gateway's or network gear's virtual one (virtual_mac, vendor then from the MAC with that bit \
cleared; the gateway is never private), TTL, open management ports (null where a host's ports weren't checked), what it said over DNS-SD \
(service types, a printer's model, queues and page languages) and over SSDP (UPnP server, device types; the \
description URL as text, never fetched). DNS-SD and SSDP are multicast: the scanner's own segment only. \
Names of phones, laptops and PCs follow client_names (in its summary and the file). scanner.switch_port is the \
switch port the scanner was plugged into (switch, port, VLANs, management address), as its newest Switch port run \
found it, with when (heard_at), null when not known; its port_description can name a person. Observed state; complete false \
when the sweep was stopped before it finished. Or, with imported set, an Advanced IP Scanner XML export the tech \
imported (found_by ais, scanned on another computer at imported.scanned_at): its alive hosts with the scanner's \
MAC, maker, name, NetBIOS name and workgroup, OS, web title, the FTP, HTTP, HTTPS and RDP it found (TLS version \
and banners) and how many shares and printers each has (never their names); dead and unknown rows only counted, \
logged-on users never imported.";
const RESULTS_WHAT: &str = "is-alive test runs (ping, port check, traceroute, DNS, mail health) with \
start and end times. A Subnet Sweep run may be an imported Advanced IP Scanner scan (its text starts \
\"Advanced IP Scanner scan of\"): its start time is when it was imported, not when that scan ran, which its \
text gives. A site keeps its runs from one visit to the next, so they can span visits: judge each \
check by its newest run. Host info and Domain join runs describe the computer is-alive ran on (its LAN, \
and how it's joined: Active Directory, Entra ID, hybrid or a workgroup), not a device on the network; \
Host info's dhcp_server, lease_obtained, lease_expires, dns_servers and dns_suffix are what that computer was \
given, recorded, not judged. Network checks: a Default gateways warning means wired and Wi-Fi led to \
different gateways (Wi-Fi may skip the firewall); a failing Captive portal row means a portal answered in the \
internet's place, so that run's internet-side checks judged the portal; a Path MTU warning is an MTU black hole; \
the Wi-Fi row is one snapshot of the access point the scanner was on (its BSSID), with channel width and link \
rates, not roaming; Nearby Wi-Fi lists other access points heard there (BSSIDs), those on its channel first, \
reported, not judged. A Wi-Fi scan run is the same list from an active scan, one row per access point \
(connected = yes on the scanner's own). A Domain join (others) run is about other computers: one row each, saying whether it named an Active \
Directory domain when asked without credentials (an Entra ID join doesn't show that way). A Switch port run says \
which switch port the scanner was plugged into: switch, port, VLANs and management address from the switch's own \
LLDP or CDP announcement heard on the wired adapter (nothing sent), or, with heard_via SNMP, the port a switch's \
forwarding table learned the scanner's MAC on (on an uplink, another switch's); one that found nothing has result \
and detail rows, not a finding. An SNMP run is one \
device's own answer over SNMP, read when the technician asked (never by a sweep): rows of section, field and \
value for its system group (sysContact left out), ENTITY-MIB chassis (model, firmware or else software revision, \
serial), a printer's status, errors and supplies, and its LLDP neighbours (its own port, then the switch and \
port at the far end). An agent row (Net-SNMP, Broadcom FASTPATH, Microsoft's SNMP service, VMware) names \
software, not the device's maker. A note row says what was cut short and what wasn't read; \"no answer\" doesn't say why. The \
community is never in it.";
const PROBLEMS_ID: &str = "bundle_known_problems";

const MAP_WHAT: &str = "The /mapnetwork map from an earlier visit, as it was opened here. \
Update it rather than starting over: keep what still holds, change what the other files show. \
is-alive reads it back by the skill's rules: a role word in each kind, a port in each edge label.";
const CARD_NOTES_WHAT: &str = "The technician's notes on cards of the Map tab's live map: each \
card's name, role, IP and MAC with its note, and an `inferred` list naming what is-alive worked out \
and didn't read or measure (role, uplink; empty when neither). Redacted like notes.md. Observations \
about those devices; carry what bears on the map into its device notes and findings.";
const NOTES_WHAT: &str = "The technician's notes from the Notes tab (Markdown). Redacted on their \
machine where they chose: values they marked and the passwords and keys of the configs they loaded \
are [🔑 REDACTED: …] markers. Nothing else is taken out, so anything that still looks like a \
credential, or a person's or patient's details, may be real: don't repeat it in the map or its \
findings. Context and observations; use what bears on the map and its findings.";

impl BundleTool {
    /// What the pattern rules would still redact in text the technician
    /// wrote, after their own redaction ("2 passwords, 1 email"), or None.
    /// Notes redact only what's marked, by design; this says what may be
    /// left before it goes to Claude. Nothing is changed.
    fn looks_unredacted(&self, part: &'static str, texts: &[&str]) -> Option<String> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        texts.hash(&mut h);
        let h = h.finish();
        let mut looked = self.looked.borrow_mut();
        match looked.get(part) {
            Some((seen, kinds)) if *seen == h => kinds.clone(),
            _ => {
                let kinds = unredacted(texts);
                looked.insert(part, (h, kinds.clone()));
                kinds
            }
        }
    }

    /// The ticked parts the technician wrote that still look like they hold
    /// secrets: (part, kinds).
    fn still_there(&self, src: &Sources) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if self.use_notes && !src.notes.is_empty() {
            // The words, not the photos: they're what a secret could be in.
            let text = src.notes.sanitized_words();
            if let Some(k) = self.looks_unredacted("Tech notes", &[&text]) {
                out.push(("Tech notes", k));
            }
        }
        if self.use_card_notes
            && let Some((_, json)) = src.map.bundle_notes(src.notes.overrides())
        {
            let notes = card_note_texts(&json);
            let notes: Vec<&str> = notes.iter().map(String::as_str).collect();
            if let Some(k) = self.looks_unredacted("Card notes", &notes) {
                out.push(("Card notes", k));
            }
        }
        out
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    /// The scene tour's picture of each step.
    pub(crate) fn step_for_shots(&mut self, step: usize) {
        self.step = step.min(STEPS.len() - 1);
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
                widgets::toast("Not saved", false);
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
        self.step = 0;
    }

    pub fn window(&mut self, ctx: &egui::Context, src: Sources) -> Option<Ask> {
        let running = self.poll(ctx);
        if !self.open {
            return None;
        }
        if self.site.trim().is_empty()
            && let Some((_, r)) = src.export.unifi()
            && let Some(name) = r.sites.first()
        {
            // From the backup: cleaned and cut like a typed name.
            self.site = crate::input::line(name, crate::input::Field::Text.max());
        }
        if self.range.trim().is_empty()
            && let Some((ip, bits)) = src.info.lan
        {
            self.range = crate::net::targets::subnet_cidr(ip, bits);
        }
        let mut open = self.open;
        let mut ask = None;
        widgets::compact_title(ctx, || {
            egui::Window::new("Claude Handoff")
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(600.0)
                .show(ctx, |ui| {
                    ui.set_width(600.0);
                    ask = self.wizard(ui, &src, running);
                });
        });
        self.open = open;
        ask
    }

    /// The steps across the top: done ones ticked, this one lit; any can
    /// be clicked.
    fn steps(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (i, name) in STEPS.iter().enumerate() {
                if i > 0 {
                    ui.weak("›");
                }
                let text = RichText::new(format!("{}  {name}", i + 1));
                // Done steps in the OK colour, this one lit, the rest dim.
                let text = if i == self.step {
                    text.strong().color(crate::theme::accent())
                } else if i < self.step {
                    text.color(widgets::ok())
                } else {
                    text.weak()
                };
                if ui
                    .add(
                        egui::Button::new(text)
                            .frame(i == self.step)
                            .min_size(egui::vec2(0.0, ROW_H)),
                    )
                    .clicked()
                {
                    self.step = i;
                }
            }
        });
    }

    /// The wizard: the step bar, what this step needs, Back and Next.
    fn wizard(&mut self, ui: &mut Ui, src: &Sources, running: bool) -> Option<Ask> {
        self.steps(ui);
        ui.separator();
        let mut ask = None;
        // Each step the same height, so Back and Next don't move.
        let room = (ui.ctx().content_rect().height() - 230.0).clamp(220.0, 400.0);
        egui::ScrollArea::vertical()
            .id_salt("bundle_step")
            .min_scrolled_height(room)
            .max_height(room)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().interact_size.y = ROW_H;
                match self.step {
                    0 => self.site_step(ui, src),
                    1 => ask = self.configs_step(ui, src),
                    2 => ask = self.sweep_step(ui, src),
                    3 => self.notes_step(ui, src),
                    _ => self.save_step(ui, src, running),
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.step > 0,
                    egui::Button::new("‹ Back").min_size(egui::vec2(72.0, ROW_H)),
                )
                .clicked()
            {
                self.step -= 1;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.step + 1 < STEPS.len() {
                    let next = format!("Next: {} ›", STEPS[self.step + 1]);
                    if ui
                        .add(widgets::primary(&next).min_size(egui::vec2(0.0, ROW_H)))
                        .clicked()
                    {
                        self.step += 1;
                    }
                } else {
                    self.save_button(ui, src, running);
                }
                ui.weak(format!("Step {} of {}", self.step + 1, STEPS.len()));
            });
        });
        ask
    }

    fn site_step(&mut self, ui: &mut Ui, src: &Sources) {
        ui.heading("Hand this visit to Claude");
        ui.label("Claude turns what you gather here into a full network map with findings.");
        ui.add_space(4.0);
        egui::Grid::new("handoff_how")
            .num_columns(2)
            .spacing([8.0, 2.0])
            .show(ui, |ui| {
                for (n, line) in [
                    "Answer these five steps. Skip any you have nothing for.",
                    "Save: you get one .zip, and a prompt is copied for you.",
                    "In a Claude chat, attach the .zip and paste the prompt.",
                    "Claude sends back a map (.dc.html). Drop it on the Map tab to see it here.",
                ]
                .iter()
                .enumerate()
                {
                    ui.label(RichText::new(format!("{}.", n + 1)).strong());
                    ui.label(*line);
                    ui.end_row();
                }
            });
        ui.add_space(10.0);
        ui.label("Site name");
        widgets::line(
            ui,
            crate::input::Field::Text,
            &mut self.site,
            560.0,
            "Client and building, e.g. Harbor Dental front office",
        );
        ui.add_space(8.0);
        ui.label("Known problems (optional)");
        self.problems(ui, src);
    }

    fn configs_step(&mut self, ui: &mut Ui, src: &Sources) -> Option<Ask> {
        let mut ask = None;
        ui.heading("Firewall and controller backups");
        ui.weak(
            "These tell Claude how the network is set up: VLANs, rules, ports, Wi-Fi. \
             Passwords and keys are stripped on this laptop before anything goes in the zip. \
             Have the map Claude made last visit? Open it here too, so it's updated, not redrawn.",
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let picking = src.export.picking();
            if ui
                .add_enabled(!picking, widgets::primary("Open files…").min_size(egui::vec2(0.0, ROW_H)))
                .on_hover_text("Pick backups, exports or last visit's map (.dc.html); they open in Configs and on the Map too")
                .clicked()
            {
                ask = Some(Ask::OpenConfigs);
            }
            if let Some(f) = src.export.reading() {
                widgets::spinner(ui);
                ui.weak(format!("Reading {f}…"));
            } else if picking {
                ui.weak("Pick the files in the dialog…");
            }
        });
        ui.add_space(6.0);
        let configs = configs(src.export);
        egui::Grid::new("bundle_configs")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                self.config_rows(ui, &configs);
                let map = src.export.mapnetwork();
                part_row(
                    ui,
                    &mut self.use_map,
                    "Last visit's map",
                    map.map(|(p, d)| {
                        format!(
                            "{}{} device(s) · {} · updated, not rebuilt",
                            map_check(&d.json, src.notes.overrides(), self.clients),
                            d.nodes.len(),
                            export::file_name(p),
                        )
                    }),
                    "Open the site's /mapnetwork .dc.html to have it updated",
                );
            });
        ui.add_space(8.0);
        ui.label(RichText::new("Where to get them").strong());
        egui::Grid::new("bundle_where")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label("UniFi");
                ui.weak("Settings › System › Backups › Download (.unf)");
                ui.end_row();
                ui.label("SonicWall");
                ui.weak("Device › Settings › Firmware & Settings › Export Configuration (.exp)");
                ui.end_row();
                ui.label("Meraki");
                ui.weak("An API export (.json) of the organization");
                ui.end_row();
            });
        ask
    }

    fn sweep_step(&mut self, ui: &mut Ui, src: &Sources) -> Option<Ask> {
        let mut ask = None;
        ui.heading("Find what's on the network");
        ui.weak(
            "A sweep lists every device that answers: name, MAC, vendor and open \
             management ports. Your own network is filled in; add others with spaces between.",
        );
        ui.label(
            RichText::new("It probes other devices: tell the site's IT contact before you start.")
                .color(widgets::warn()),
        );
        ui.add_space(8.0);
        ui.label("Subnets or ranges");
        ui.horizontal(|ui| {
            let busy = src.sweep.is_running();
            let enter = widgets::line(
                ui,
                crate::input::Field::Range,
                &mut self.range,
                420.0,
                "192.168.1.0/24 10.0.0.1-50",
            )
            .lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let label = if src.sweep.finished().is_some() {
                "Sweep again"
            } else {
                "Sweep"
            };
            if (ui
                .add_enabled(
                    !busy && !self.range.trim().is_empty(),
                    widgets::primary(label).min_size(egui::vec2(0.0, ROW_H)),
                )
                .clicked()
                || (enter && !busy))
                && !self.range.trim().is_empty()
            {
                ask = Some(Ask::Sweep(self.range.trim().to_string()));
            }
        });
        if let Some((done, total, since, _)) = src.sweep.progress() {
            widgets::progress(ui, done, total, since);
        }
        ui.add_space(8.0);
        let runs = src.log.entries.len();
        let targets = log_targets(src.log);
        egui::Grid::new("bundle_sweep")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                part_row(
                    ui,
                    &mut self.use_inventory,
                    "Sweep inventory",
                    src.sweep
                        .finished()
                        .map(|(range, n)| format!("{n} live host(s) · {range}")),
                    "Not swept yet",
                );
                part_row(
                    ui,
                    &mut self.use_results,
                    "Results log",
                    (runs > 0).then(|| match targets.is_empty() {
                        true => format!("{runs} run(s)"),
                        false => format!("{runs} run(s): {}", targets.join(", ")),
                    }),
                    "Nothing in Results yet (Full scan fills it)",
                );
            });
        ask
    }

    fn notes_step(&mut self, ui: &mut Ui, src: &Sources) {
        ui.heading("Your notes");
        ui.weak(
            "Notes go in as Notes redacts them: what you marked with Redact, and the \
             passwords and keys of the configs you loaded. Nothing else is taken out.",
        );
        ui.add_space(8.0);
        egui::Grid::new("bundle_notes")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                let hits = src.notes.hits();
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
            });
        for (part, kinds) in self.still_there(src) {
            ui.colored_label(
                widgets::warn(),
                format!(
                    "{part} still hold what looks like {kinds}. Notes redacts only what you mark: \
                     select each one there and press Redact, or untick {part}."
                ),
            );
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Client devices (phones, laptops)").strong());
        ui.horizontal(|ui| {
            export::clients_choice(ui, "bundle_clients", &mut self.clients);
            ui.separator();
            ui.checkbox(&mut self.map_clients, "Draw them on the map")
                .on_hover_text("Put client devices on the map (off: infrastructure only)");
        });
    }

    fn save_step(&mut self, ui: &mut Ui, src: &Sources, running: bool) {
        ui.heading("Check and save");
        ui.weak("Everything ticked goes in the zip. Untick anything you'd rather leave out.");
        ui.add_space(6.0);
        let configs = configs(src.export);
        let targets = log_targets(src.log);
        egui::Grid::new("bundle_parts")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| self.rows(ui, src, &configs, &targets));
        widgets::error(ui, &self.err);
        ui.add_space(8.0);
        if running {
            ui.horizontal(|ui| {
                widgets::spinner(ui);
                ui.weak("Saving…");
            });
        }
        if let Some(s) = &self.saved {
            ui.label(
                RichText::new(format!(
                    "Saved {}. The kickoff prompt is on the clipboard.",
                    export::file_name(&s.path)
                ))
                .color(widgets::ok()),
            )
            .on_hover_text(s.path.display().to_string());
            ui.add_space(4.0);
            ui.label(RichText::new("Now, in Claude").strong());
            ui.label("1.  Open a new chat at claude.ai (or Claude on the desktop).");
            ui.label(format!("2.  Attach {}.", export::file_name(&s.path)));
            ui.label("3.  Paste (Ctrl+V) and send. The prompt tells Claude what to do.");
            ui.label("4.  Save the map it sends back, then drop it on the Map tab.");
            if ui
                .button("Copy the prompt again")
                .on_hover_text(s.kickoff.as_str())
                .clicked()
            {
                widgets::copy(ui.ctx(), s.kickoff.clone());
                widgets::toast("Kickoff prompt copied", true);
            }
        } else {
            ui.weak("Save for Claude… writes the zip and copies the prompt to paste with it.");
        }
    }

    /// Something is ticked that has something in it.
    fn chosen(&self, src: &Sources) -> bool {
        let configs = configs(src.export);
        (self.use_map && src.export.mapnetwork().is_some())
            || configs
                .iter()
                .any(|c| c.files.iter().any(|f| !self.left_out.contains(f.path)))
            || (self.use_inventory && src.sweep.finished().is_some())
            || (self.use_results && !src.log.entries.is_empty())
            || (self.use_notes && !src.notes.is_empty())
            || (self.use_card_notes && src.map.bundle_notes(src.notes.overrides()).is_some())
    }

    fn save_button(&mut self, ui: &mut Ui, src: &Sources, running: bool) {
        let chosen = self.chosen(src);
        if ui
            .add_enabled(
                chosen && !running,
                widgets::primary("Save for Claude…").min_size(egui::vec2(0.0, ROW_H)),
            )
            .on_hover_text("Save the zip and copy the prompt")
            .on_disabled_hover_text(
                "Nothing to hand over yet: open a backup (step 2) or run a sweep (step 3)",
            )
            .clicked()
        {
            self.save(ui.ctx(), src);
        }
    }

    /// Known problems: a box that goes straight into /mapnetwork's
    /// findings, redacted as the notes are.
    fn problems(&mut self, ui: &mut Ui, src: &Sources) {
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
                &eframe::egui::FontId::monospace(crate::theme::MONO),
            );
            job.wrap.max_width = wrap;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let keep = widgets::selection_before_right_click(ui.ctx(), id);
        let edit = egui::TextEdit::multiline(&mut self.notes)
            .id(id)
            .char_limit(crate::input::Field::Problems.room())
            .desired_rows(4)
            .desired_width(560.0)
            .hint_text("Optional. What the client reported, or what you saw. Goes straight into /mapnetwork's findings.")
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
            if widgets::text_edit_menu(ui, id, crate::clip::EDITABLE, self.notes.is_empty()) {
                ui.close();
            }
        });
        let clean = crate::redact::sanitize_with(&self.notes, &ov).0;
        if let Some(kinds) = self.looks_unredacted("Known problems", &[&clean]) {
            ui.colored_label(
                widgets::warn(),
                format!("This looks like it holds {kinds}: select it and press Redact."),
            );
        }
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
                .add_enabled(pick.is_some(), egui::Button::new(label))
                .on_hover_text("Hide the selected text (Ctrl+R). Notes' redactions apply here too")
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
    }

    /// A row for each backup and export open (or, with none of a kind,
    /// what to open).
    fn config_rows(&mut self, ui: &mut Ui, configs: &[Configs]) {
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
    }

    /// The Includes list: a row for each backup and export open (or, with
    /// none of a kind, what to open), then the rest.
    fn rows(&mut self, ui: &mut Ui, src: &Sources, configs: &[Configs], targets: &[String]) {
        self.config_rows(ui, configs);
        let map = src.export.mapnetwork();
        let sweep = src.sweep.finished();
        let runs = src.log.entries.len();
        part_row(
            ui,
            &mut self.use_map,
            "Current map",
            map.map(|(p, d)| {
                format!(
                    "{}{} device(s) · {} · updated, not rebuilt",
                    map_check(&d.json, src.notes.overrides(), self.clients),
                    d.nodes.len(),
                    export::file_name(p),
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
        let hits = src.notes.hits();
        // What still looks secret in a ticked part, said on its row too: the
        // last look before the zip is made.
        let left: HashMap<_, _> = self.still_there(src).into_iter().collect();
        let check = |part: &str| {
            left.get(part)
                .map(|k| format!(" · check: looks like {k}"))
                .unwrap_or_default()
        };
        part_row(
            ui,
            &mut self.use_notes,
            "Tech notes",
            (!src.notes.is_empty()).then(|| {
                let done = match hits.len() {
                    0 => "notes.md · nothing redacted".to_string(),
                    _ => format!("notes.md · {}", crate::redact::summary(&hits)),
                };
                done + &check("Tech notes")
            }),
            "Nothing in Notes yet",
        );
        let card_notes = src.map.bundle_notes(src.notes.overrides());
        part_row(
            ui,
            &mut self.use_card_notes,
            "Card notes",
            card_notes.as_ref().map(|(n, _)| {
                format!(
                    "{n} card(s) on the live map · redacted{}",
                    check("Card notes")
                )
            }),
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
            parts.push(meraki_part(n + 1, p, r, self.clients));
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
                    "client_names": self.clients.label(),
                }),
                data: src.sweep.inventory(src.info, self.clients),
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
        .set_title("Save the Claude Handoff")
        .set_file_name(format!("{}.bundle.zip", bundle::slug(&b.site)))
        .add_filter("Zip", &["zip"])
        .save_file()
    else {
        widgets::tidy_save_folder();
        return Ok(None);
    };
    let path = widgets::safe_path(path, &["zip"]);
    let name = export::file_name(&path);
    let written = b
        .zip(&name)
        .map_err(|e| e.to_string())
        .and_then(|zip| write_whole(&path, &zip).map_err(|e| format!("{}: {e}", path.display())));
    // Saved elsewhere, or not at all: the site folder made for the dialog
    // goes if it's still empty.
    widgets::tidy_save_folder();
    written?;
    let kickoff = b.kickoff(&name);
    Ok(Some(Saved { path, kickoff }))
}

/// What to check before last visit's map goes in: it goes as it was
/// opened (Claude updates that page), so anything an older or hand-edited
/// map holds goes too. "Check it: 1 email in it · ", to start its row, when its
/// data holds a secret from the configs open now, a value marked in Notes,
/// a key, token, email address, card or patient detail; empty when not.
/// A password found by the word alone isn't counted: a map's findings say
/// "password policy" all the time. Cached: read once, not every frame.
fn map_check(json: &str, ov: &crate::redact::Overrides, clients: Clients) -> String {
    use crate::redact::Kind;
    use std::hash::{Hash, Hasher};
    static SEEN: std::sync::Mutex<Option<(u64, String)>> = std::sync::Mutex::new(None);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (json, ov, clients.label()).hash(&mut h);
    let key = h.finish();
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, s)) = seen.as_ref()
        && *k == key
    {
        return s.clone();
    }
    // The pattern rules (less the noisy ones), then what the technician
    // marked and the open configs' secrets.
    let hits: Vec<_> = crate::redact::find(json)
        .into_iter()
        .filter(|h| !matches!(h.kind, Kind::Password | Kind::Address))
        .chain(crate::redact::find_with(json, ov))
        .collect();
    // Client devices named as they were (people's names, often), where
    // Client names says they shouldn't go: the map goes as it was opened,
    // so it would carry them anyway.
    let named = if clients == Clients::Full {
        0
    } else {
        named_clients(json)
    };
    let mut what = Vec::new();
    if !hits.is_empty() {
        what.push(crate::redact::summary(&hits));
    }
    if named > 0 {
        what.push(format!(
            "{named} client device name{} (Client names is {})",
            if named == 1 { "" } else { "s" },
            clients.label()
        ));
    }
    let s = if what.is_empty() {
        String::new()
    } else {
        format!("Check it: {} in it · ", what.join(", "))
    };
    *seen = Some((key, s.clone()));
    s
}

/// How many cards of a /mapnetwork page's data are client devices (a
/// client zone, or a phone, laptop or workstation) named other than by a
/// token (`client-1a2b3c4d`) or an address.
fn named_clients(json: &str) -> usize {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return 0;
    };
    let Some(nodes) = v["nodes"].as_array() else {
        return 0;
    };
    nodes
        .iter()
        .filter(|n| {
            let zone = n["zone"].as_str().unwrap_or("").to_ascii_lowercase();
            let kind = n["kind"].as_str().unwrap_or("").to_ascii_uppercase();
            zone.contains("client")
                || ["PHONE", "LAPTOP", "WORKSTATION", "TABLET"]
                    .iter()
                    .any(|k| kind.contains(k))
        })
        .filter(|n| {
            let label = n["label"].as_str().unwrap_or("").trim();
            !label.is_empty()
                && !label.starts_with("client-")
                && label.parse::<std::net::IpAddr>().is_err()
                && crate::net::mac::parse_mac(label).is_none()
        })
        .count()
}

/// Write `data` to `path` whole or not at all: to a file beside it, then
/// renamed over it. A full disk part-way leaves no half-written zip that
/// looks like a handoff (and an older one of that name as it was).
fn write_whole(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let r = (|| {
        let mut f = std::fs::File::create(&part)?;
        f.write_all(data)?;
        f.sync_all()?;
        std::fs::rename(&part, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    r
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
fn meraki_part(n: usize, path: &Path, r: &meraki::Report, clients: Clients) -> Part {
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
            "client_names": clients.label(),
        }),
        data: r.json_for(clients),
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
            missing: "None open",
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
            missing: "None open",
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
            missing: "None open",
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
/// The pattern rules over `texts` (already redacted as the technician
/// chose): "2 passwords, 1 email", or None when nothing looks secret.
fn unredacted(texts: &[&str]) -> Option<String> {
    let hits: Vec<_> = texts.iter().flat_map(|t| crate::redact::find(t)).collect();
    (!hits.is_empty()).then(|| crate::redact::kinds(&hits))
}

/// Each card's note from `card-notes.json`.
fn card_note_texts(json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<serde_json::Value>>(json)
        .unwrap_or_default()
        .iter()
        .filter_map(|r| r["note"].as_str().map(String::from))
        .collect()
}

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

    /// Client devices named in last visit's map are pointed out when
    /// Client names says names don't go; tokens, addresses and
    /// infrastructure aren't counted, and Full asks nothing.
    #[test]
    fn a_map_naming_client_devices_says_so() {
        let ov = crate::redact::Overrides::default();
        let map = r#"{"nodes":[
            {"label":"Core switch","zone":"switch","kind":"SWITCH"},
            {"label":"Dana's iPhone","zone":"client","kind":"PHONE"},
            {"label":"RECEPTION-PC","zone":"clients","kind":"WORKSTATION"},
            {"label":"client-1a2b3c4d","zone":"client"},
            {"label":"10.0.0.50","zone":"client"}]}"#;
        let s = map_check(map, &ov, Clients::Anonymized);
        assert!(s.contains("2 client device names"), "{s}");
        assert!(s.contains("anonymized"), "{s}");
        assert_eq!(map_check(map, &ov, Clients::Full), "");
    }

    /// Last visit's map goes as it was opened: what it holds that Notes
    /// would redact is pointed out on its row, but not prose about
    /// passwords.
    #[test]
    fn a_map_holding_a_secret_says_so() {
        let ov = crate::redact::Overrides::default();
        assert_eq!(
            map_check(
                r#"{"nodes":[{"label":"Core switch"}]}"#,
                &ov,
                Clients::Anonymized
            ),
            ""
        );
        let prose = r#"{"actions":[{"text":"scope the password replication policy tightly"}]}"#;
        assert_eq!(map_check(prose, &ov, Clients::Anonymized), "");
        let s = map_check(
            r#"{"nodes":[{"details":["NOC: jane.doe@clinic-mail.com"]}]}"#,
            &ov,
            Clients::Anonymized,
        );
        assert!(s.starts_with("Check it: ") && s.contains("email"), "{s}");
        // A secret from a config open now, wherever it sits.
        let known = crate::redact::Overrides {
            known: vec!["Harbor2024!".into()],
            ..Default::default()
        };
        let s = map_check(
            r#"{"meta":{"subtitle":"wifi Harbor2024!"}}"#,
            &known,
            Clients::Anonymized,
        );
        assert!(s.starts_with("Check it: "), "{s}");
        // Both bundled examples are clean.
        for i in 0..2 {
            let p = crate::skills::example_path(i);
            let d = crate::mapnetwork::parse(crate::skills::example(&p).unwrap()).unwrap();
            // No secrets in them (their client names are fictional).
            assert_eq!(map_check(&d.json, &ov, Clients::Full), "", "example {i}");
        }
    }

    /// A handoff is written whole or not at all: a good write replaces
    /// the old zip, a failed one leaves no part file and the old one as
    /// it was.
    #[test]
    fn the_zip_is_written_whole_or_not_at_all() {
        let dir = std::env::temp_dir().join(format!("is-alive-whole-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("site.bundle.zip");
        std::fs::write(&p, b"old").unwrap();
        write_whole(&p, b"new").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        assert!(!dir.join("site.bundle.zip.part").exists());
        let gone = dir.join("no-such-folder").join("x.zip");
        assert!(write_whole(&gone, b"x").is_err());
        assert!(!dir.join("no-such-folder").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
    use crate::log::{Entry, Table};

    /// The Handoff says what's left in the technician's text after their own
    /// redaction, by the pattern rules, and changes nothing: notes are
    /// redacted only where marked, so the warning is the one safety net.
    #[test]
    fn what_still_looks_secret_is_said_not_removed() {
        let text = "Front desk PC, admin password: hunter2!\nAsk bob@clinic.example";
        assert_eq!(unredacted(&[text]).as_deref(), Some("1 password, 1 email"));
        let marked = "admin password: [🔑 REDACTED: password]\nall fine";
        assert_eq!(unredacted(&[marked]), None);
        assert_eq!(unredacted(&[]), None);
        let tool = BundleTool::default();
        // Cached per part, and a new text is looked at again.
        assert_eq!(
            tool.looks_unredacted("Tech notes", &[text]).as_deref(),
            Some("1 password, 1 email")
        );
        assert_eq!(tool.looks_unredacted("Tech notes", &[marked]), None);
        assert_eq!(
            tool.looks_unredacted("Card notes", &[text]).as_deref(),
            Some("1 password, 1 email")
        );
    }

    /// Known problems holding a password says so, and the warning fits a
    /// field laptop's window at both sizes.
    #[test]
    fn the_known_problems_warning_shows_and_fits() {
        let export = ExportTool::default();
        let sweep = SweepTool::default();
        let log = Log::default();
        let info = HostInfo::default();
        let notes = NotesTool::default();
        let map = crate::map::view::MapTabs::new(Default::default());
        for size in [(1280.0, 720.0), (1120.0, 720.0)] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::fonts::definitions());
            let mut tool = BundleTool {
                open: true,
                notes: "The front desk PC's admin password: hunter2! (client said)".into(),
                ..Default::default()
            };
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1));
            let mut texts = Vec::new();
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    let src = Sources {
                        export: &export,
                        sweep: &sweep,
                        log: &log,
                        info: &info,
                        notes: &notes,
                        map: &map,
                    };
                    tool.window(ui.ctx(), src);
                });
                out.textures_delta.clear();
                texts = out
                    .shapes
                    .iter()
                    .filter_map(|c| match &c.shape {
                        egui::Shape::Text(t) => {
                            Some((t.galley.text().to_string(), t.visual_bounding_rect()))
                        }
                        _ => None,
                    })
                    .collect();
            }
            assert!(
                texts
                    .iter()
                    .any(|(t, _)| t.contains("looks like it holds 1 password")),
                "{size:?}: {texts:?}"
            );
            // (An empty galley has no bounds to check.)
            for (t, r) in texts.iter().filter(|(t, _)| !t.trim().is_empty()) {
                assert!(
                    screen.contains_rect(*r),
                    "{t:?} at {r:?} is outside {size:?}"
                );
            }
        }
    }

    #[test]
    fn card_notes_are_read_from_their_json() {
        let json = r#"[{"card":"gw","note":"psk is Spring2026!"},{"card":"sw","note":"fine"},{"card":"x"}]"#;
        assert_eq!(card_note_texts(json), ["psk is Spring2026!", "fine"]);
        assert!(card_note_texts("not json").is_empty());
    }

    /// The manifest tells Claude what was done to the notes, no more: they
    /// were never pattern-scrubbed, and Claude is told not to repeat what
    /// may be left.
    #[test]
    fn the_manifest_doesnt_overstate_the_notes_redaction() {
        assert!(NOTES_WHAT.contains("Nothing else is taken out"));
        assert!(!NOTES_WHAT.contains("patient details are"));
        assert!(!CARD_NOTES_WHAT.contains("Sanitized"));
    }

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
