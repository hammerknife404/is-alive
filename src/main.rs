#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backdrop;
mod bundle;
mod chrome;
mod connect;
mod fonts;
mod help;
#[cfg(test)]
mod hostile;
mod icons;
mod inferred;
mod input;
mod inspector;
mod job;
mod log;
mod manual;
mod map;
mod mapnetwork;
mod markdown;
mod markdown_out;
mod meraki;
mod nav;
mod net;
mod photos;
mod power;
mod redact;
mod report;
mod results_ui;
mod runs_compare;
mod shots;
mod site;
mod site_ui;
mod skills;
mod sonicwall;
mod startup;
mod theme;
mod tools;
mod unifi;
mod update;
#[cfg(target_os = "linux")]
mod wayland_icon;
mod widgets;

use eframe::egui;
use job::{Job, pump};
use net::hostinfo::HostInfo;

/// Internet target for the full scan's traceroute (and ping when there's no gateway).
const WAN_TARGET: &str = "1.1.1.1";
const DNS_TEST_NAME: &str = "cloudflare.com";
/// Shown in the window title, the status bar and the full scan's Host info entry,
/// so a screenshot or pasted log always says which build produced it.
const VERSION: &str = env!("CARGO_PKG_VERSION");
const FULL_SCAN_TIP: &str = "Full scan: ping and port-check the gateway, trace to the internet, test DNS, sweep the LAN, and check whether HTTPS is inspected, this computer's clock is right and how it's joined to a domain. Results opens to show them. (Speed Test runs from its own tab.)";

#[derive(PartialEq, Clone, Copy, Debug)]
enum Tab {
    Ping,
    Ports,
    Trace,
    Sweep,
    Dns,
    Advanced,
    Export,
    Map,
    Notes,
    Speed,
    Calc,
    Join,
    Results,
}

/// (tab, label, hover text).
const TABS: &[(Tab, &str, &str)] = &[
    (
        Tab::Ping,
        "Ping",
        "ICMP or TCP ping with a live latency graph",
    ),
    (Tab::Ports, "Ports", "Find open TCP ports on a host"),
    (Tab::Trace, "Traceroute", "Show each router hop to a host"),
    (
        Tab::Sweep,
        "Sweep",
        "Find live hosts on a subnet, with name, MAC and vendor",
    ),
    (
        Tab::Dns,
        "Lookup",
        "Look up DNS records; Compare all spots split DNS and stale caches",
    ),
    (
        Tab::Advanced,
        "Domains",
        "Mail and domain health: MX, SPF, DMARC, DKIM, blacklists and more",
    ),
    (
        Tab::Export,
        "Configs",
        "Convert UniFi, SonicWall and Meraki configs to JSON for the skills; open /mapnetwork and saved maps",
    ),
    (
        Tab::Map,
        "Map",
        "Network map from the sweep, configs or a /mapnetwork map. Drag to pan, scroll to zoom",
    ),
    (Tab::Notes, "Notes", "Site notes in Markdown, autosaved"),
    (
        Tab::Speed,
        "Speed",
        "Speed, latency, bufferbloat and VoIP quality",
    ),
    (
        Tab::Calc,
        "Subnet Calc",
        "Subnet calculator: masks, host ranges, splitting a block, range to CIDR",
    ),
    (
        Tab::Join,
        "Domain join",
        "How this computer is joined: Active Directory, Entra ID, hybrid or a workgroup, and to which domain",
    ),
    (
        Tab::Results,
        "Results",
        "Every finished run. Filter, copy or export",
    ),
];

struct App {
    tab: Tab,
    /// The tab on screen and when it appeared, for the fade-in.
    shown_tab: Tab,
    tab_since: f64,
    /// Window size last frame as a normal window, remembered for the next
    /// launch, and whether it's maximised.
    size: Option<egui::Vec2>,
    maximized: bool,
    /// The first window has been fitted to the screen (see `fit_first`).
    fitted: bool,
    ping: tools::ping::PingTool,
    ports: tools::ports::PortTool,
    trace: tools::trace::TraceTool,
    speed: tools::speed::SpeedTool,
    sweep: tools::sweep::SweepTool,
    calc: tools::calc::CalcTool,
    dns: tools::dns::DnsTool,
    advanced: tools::advanced::AdvancedTool,
    /// Full scan's network checks (HTTPS inspection, the clock).
    netcheck: tools::netcheck::NetCheck,
    /// How this computer is joined, for its tab and the status bar.
    join: tools::join::DomainJoin,
    export: tools::export::ExportTool,
    bundle: tools::bundle::BundleTool,
    report: tools::report::ReportTool,
    map: map::view::MapTabs,
    notes: tools::notes::NotesTool,
    log: log::Log,
    /// The log as last written to the open site's folder (its stamp).
    log_saved: u64,
    info: HostInfo,
    info_job: Option<Job<HostInfo>>,
    /// The public address has been asked for: on start (unless Settings
    /// turns that off), on Refresh or by a full scan.
    wan_asked: bool,
    /// An update check under way, and what the last one found.
    update_job: Option<Job<update::Found>>,
    update: Option<update::Found>,
    /// The running check came from Check now.
    update_asked: bool,
    /// The sweep was running last frame (to see one start).
    was_sweeping: bool,
    /// The tabs whose full-scan run is still going: when the last ends,
    /// the Map tab calls for attention. A run started there by hand after
    /// the scan's own ended doesn't count.
    scanning: Vec<Tab>,
    map_call: chrome::MapCall,
    /// Results tab filter; matches tool name or output, case-insensitive.
    filter: String,
    /// ((filter, log stamp), matching entries) for the last frame.
    #[allow(clippy::type_complexity)]
    filter_cache: Option<((String, u64), Vec<usize>)>,
    /// Open the full scan's first run in Results on the next frame.
    jump_to_new: bool,
    /// The run open in Results (None: the newest), and how it's shown.
    result_pick: Option<results_ui::RunKey>,
    /// Clear or Remove in Results, made at the start of its next frame.
    results_edit: Option<results_ui::Edit>,
    result_view: results_ui::RunView,
    /// The run the open one is compared with (Results › Compare): its start.
    result_compare: Option<String>,
    look: theme::Settings,
    backdrop: backdrop::Backdrop,
    /// File dialog for the Miami backdrop image.
    pick_image: Option<Job<Option<std::path::PathBuf>>>,
    /// Settings › Files › Choose folder…, while its dialog is open.
    pick_save_dir: Option<Job<Option<std::path::PathBuf>>>,
    /// The map's files are being read again (Reload): say how it went.
    reloading: bool,
    /// The user manual, built in: Help and F1 open it.
    manual: manual::Manual,
    shots: Option<shots::Shots>,
    /// The screenshot tour's last steps: Settings open, then Help.
    shots_settings: bool,
    shots_help: bool,
    /// Closing waits on "Save the notes?" (autosave off, changes unsaved).
    close_asked: bool,
    /// Settings › Remove everything is asking: what's there, and how
    /// many files; then why it couldn't all be removed, if so.
    remove_ask: Option<(Vec<String>, usize)>,
    remove_failed: Option<String>,
    /// The user answered it: close without asking again.
    close_anyway: bool,
    /// The open site's name, for the header ("Scratch": none).
    site_name: String,
    /// The sites, read when the switcher opens.
    sites: Vec<site::Info>,
    /// What the site dialog is asking.
    site_ask: Option<site_ui::SiteAsk>,
    /// The configs the open site remembers, as last written.
    site_configs: Vec<std::path::PathBuf>,
    /// Configs it remembers that weren't found when it opened: kept on its
    /// list until Clear map.
    site_missing: Vec<std::path::PathBuf>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        startup::running(ui.ctx(), &mut self.look);
        theme::set_moves(ui.ctx(), self.look.moves());
        if ui.input(|i| i.viewport().close_requested())
            && self.notes.unsaved()
            && !self.close_anyway
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_asked = true;
        }
        self.tab_keys(ui);
        self.shots_step(ui.ctx());
        let now = ui.input(|i| i.time);
        if self.tab != self.shown_tab {
            self.shown_tab = self.tab;
            self.tab_since = now;
        }
        if self.tab == Tab::Map {
            self.map_call.seen();
        }
        self.fit_first(ui.ctx());
        self.poll_all(ui.ctx(), now);
        // Under every panel; Miami's panels are translucent over it.
        if self.look.theme == theme::Theme::Miami {
            self.backdrop.paint(
                ui.ctx(),
                &self.look.backdrop,
                self.look.moves(),
                self.look.glow,
                self.look.vhs,
            );
        }
        self.title_bar(ui);
        self.footer(ui);
        self.nav(ui);
        self.central(ui, now);
        widgets::show_toasts(ui.ctx());
        self.bundle_window(ui.ctx());
        self.report_window(ui.ctx());
        if let Some(own) = self.manual.window(ui.ctx()) {
            self.look.manual_window = own;
            theme::save(&self.look);
        }
        self.close_dialog(ui.ctx());
        self.remove_dialog(ui.ctx());
        self.site_dialog(ui.ctx());
        // Links (Help, the update notice, a card's Web) open through the
        // app's own launcher: egui's needs a feature this build leaves out.
        widgets::open_requested_links(ui.ctx());
        widgets::route_copies(ui.ctx());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Everything was removed: closing puts none of it back.
        if theme::removed() {
            widgets::clear_own_clipboard();
            return;
        }
        self.notes.set_elsewhere(self.map.note_texts());
        self.notes.flush();
        widgets::clear_own_clipboard();
        let size = self
            .size
            .map(|s| (s.x.round() as u32, s.y.round() as u32))
            .filter(|&(w, h)| (620..=8000).contains(&w) && (400..=8000).contains(&h))
            .or(self.look.size);
        if size != self.look.size || self.maximized != self.look.maximized {
            self.look.size = size;
            self.look.maximized = self.maximized;
            theme::save(&self.look);
        }
    }
}

/// A screen the first window (1120 × 720 points and a title bar) doesn't
/// fit on with a taskbar: a 1080p laptop at 150% (1280 × 720 points), or
/// 1366 × 768. The window opens maximised there.
fn small_screen(monitor: egui::Vec2) -> bool {
    monitor.y < 800.0 || monitor.x < 1200.0
}

impl App {
    /// The app as it starts, before anything is read from disk or asked
    /// of the network: every tab empty.
    fn fresh(
        look: theme::Settings,
        gl: Option<&std::sync::Arc<eframe::glow::Context>>,
        manual_window: bool,
    ) -> Self {
        App {
            tab: Tab::Ping,
            shown_tab: Tab::Ping,
            tab_since: f64::NEG_INFINITY,
            size: None,
            maximized: look.maximized,
            fitted: false,
            ping: Default::default(),
            ports: Default::default(),
            trace: Default::default(),
            speed: Default::default(),
            sweep: Default::default(),
            calc: Default::default(),
            dns: Default::default(),
            advanced: Default::default(),
            netcheck: Default::default(),
            join: Default::default(),
            export: Default::default(),
            bundle: Default::default(),
            report: Default::default(),
            map: map::view::MapTabs::new(look.map_layout),
            notes: Default::default(),
            log: Default::default(),
            log_saved: 0,
            info: HostInfo::local(),
            info_job: None,
            wan_asked: false,
            was_sweeping: false,
            scanning: Vec::new(),
            map_call: Default::default(),
            update_job: None,
            update: None,
            update_asked: false,
            filter: String::new(),
            filter_cache: None,
            jump_to_new: false,
            result_pick: None,
            results_edit: None,
            result_view: results_ui::RunView::Output,
            result_compare: None,
            look,
            backdrop: backdrop::Backdrop::new(gl),
            pick_image: None,
            pick_save_dir: None,
            reloading: false,
            manual: manual::Manual::new(manual_window),
            shots: shots::Shots::from_env(),
            shots_settings: false,
            shots_help: false,
            close_asked: false,
            remove_ask: None,
            remove_failed: None,
            close_anyway: false,
            site_name: site::name(),
            sites: Vec::new(),
            site_ask: None,
            site_configs: site::meta().map(|m| m.configs).unwrap_or_default(),
            site_missing: Vec::new(),
        }
    }

    /// Note the window's size (as a normal window) and whether it's
    /// maximised. The first time is-alive opens (no size saved), once the
    /// screen's size is known, a small screen gets the window maximised:
    /// the system only shrinks a window to the screen, not to what the
    /// taskbar leaves, so the status bar went under it.
    fn fit_first(&mut self, ctx: &egui::Context) {
        let (inner, maximized, monitor) = ctx.input(|i| {
            let v = i.viewport();
            (v.inner_rect.map(|r| r.size()), v.maximized, v.monitor_size)
        });
        if let Some(m) = maximized {
            self.maximized = m;
        }
        if maximized == Some(false) {
            self.size = inner;
        }
        if self.fitted || self.shots.is_some() {
            return;
        }
        // Maximised last time: asked again on the first frame, since some
        // window managers ignore it on a window that isn't shown yet.
        if self.look.maximized {
            self.fitted = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        } else if self.look.size.is_some() {
            self.fitted = true;
        } else if let Some(m) = monitor {
            self.fitted = true;
            if small_screen(m) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
        }
    }

    /// Ctrl+1 … Ctrl+9 for the first nine tabs, Ctrl+0 for Results; F1
    /// opens the manual at this tab's chapter.
    fn tab_keys(&mut self, ui: &mut egui::Ui) {
        use egui::Key::*;
        if ui.input_mut(|inp| inp.consume_key(egui::Modifiers::NONE, F1)) {
            self.manual.show(manual::chapter_of(self.tab));
        }
        let keys = [Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9];
        for (n, (tab, ..)) in TABS.iter().enumerate() {
            // Ctrl+Alt+digit is Notes' headings: not a tab switch.
            if let Some(d) = tab_key(n)
                && !ui.input(|inp| inp.modifiers.alt)
                && ui.input_mut(|inp| inp.consume_key(egui::Modifiers::COMMAND, keys[d]))
            {
                self.tab = *tab;
            }
        }
    }

    /// The screenshot tour (IS_ALIVE_SHOTS): every tab, then the Map with a
    /// card selected, then Settings, Help and the manual; or the scene tour
    /// (IS_ALIVE_SHOTS_SCENES).
    fn shots_step(&mut self, ctx: &egui::Context) {
        if self.shots.as_ref().is_some_and(shots::Shots::scenes) {
            return self.scene_shots_step(ctx);
        }
        if let Some(s) = &mut self.shots {
            // Every tab, then the Map again with a card selected.
            let mut names: Vec<&str> = TABS.iter().map(|t| t.1).collect();
            names.extend(["Map panel", "Settings", "Help", "Manual", "Manual keys"]);
            match s.step(ctx, &names, true) {
                Some(i) if i < TABS.len() => self.tab = TABS[i].0,
                Some(i) if i == TABS.len() => {
                    self.tab = crate::Tab::Map;
                    self.map.select_for_shots();
                }
                Some(i) if i == TABS.len() + 1 => self.shots_settings = true,
                Some(i) if i == TABS.len() + 2 => {
                    self.shots_settings = false;
                    self.shots_help = true;
                }
                Some(i) if i == TABS.len() + 3 => {
                    if self.shots_help {
                        self.shots_help = false;
                        egui::Popup::close_all(ctx);
                        self.tab = crate::Tab::Ping;
                        self.manual.show(manual::chapter_of(self.tab));
                    }
                }
                Some(_) => {
                    let keys = manual::chapter_named("Keyboard and mouse");
                    if self.manual.chapter() != keys {
                        self.manual.show(keys);
                    }
                }
                None => {}
            }
        }
    }

    /// Once a frame: every tool's job, the Map's runs, notes autosave,
    /// finished runs into Results, dropped files, and background lookups.
    fn poll_all(&mut self, ctx: &egui::Context, now: f64) {
        self.ping.poll();
        self.ports.poll();
        self.trace.poll();
        self.map.poll(ctx);
        self.speed.poll();
        self.sweep.poll();
        // A sweep of a real network starting (from the Map, Subnet Sweep or
        // a full scan): an example map would hide what it finds.
        let sweeping = self.sweep.is_running();
        if sweeping && !self.was_sweeping && self.export.close_examples() {
            widgets::toast("Example map closed: the map shows this network now", true);
        }
        self.was_sweeping = sweeping;
        self.dns.poll();
        self.advanced.poll();
        self.netcheck.poll();
        self.join.poll();
        self.export.poll(ctx);
        // Files the user opened are in: a saved map on show gives way to
        // the live map, which has them.
        if self.export.take_opened() && self.map.show_live() {
            widgets::toast(
                "The Map is back on Live to show what you opened (saved maps stay in the bar along the bottom)",
                true,
            );
        }
        // Read from another tab (the Map's Open…, a drop on it, a site's
        // configs): what Config Export would say, said here. Reload says
        // its own.
        let notices = self.export.take_notices();
        if self.tab != Tab::Export && !self.reloading {
            for (text, good) in notice_toasts(notices) {
                widgets::toast(text, good);
            }
        }
        // A saved map opened in Config Export or dropped: the Map shows it.
        for (path, data) in self.export.take_saved_maps() {
            match self.map.open_file(&path, &data) {
                Ok(()) => self.tab = Tab::Map,
                Err(e) => self.export.refused(&path, &e),
            }
        }
        if self.reloading && !self.export.is_running() {
            self.reloading = false;
            let failed = self.export.failed();
            if failed.is_empty() {
                widgets::toast("Map reloaded", true);
            } else {
                widgets::toast(
                    format!(
                        "Couldn't read {} again: the map still shows what was loaded before \
                         (Config Export says why)",
                        failed.join(", ")
                    ),
                    false,
                );
            }
        }
        // The full scan is done: what its sweep found is on the Map now
        // (not when the map's files leave it off as another network's).
        if !self.scanning.is_empty() {
            let mut left = std::mem::take(&mut self.scanning);
            left.retain(|t| self.tab_running(*t));
            self.scanning = left;
            if self.scanning.is_empty()
                && self.sweep.finished().is_some_and(|(_, n)| n > 0)
                && self.map.shows_sweep(&map::view::Sources {
                    sweep: &self.sweep,
                    export: &self.export,
                    info: &self.info,
                    busy: false,
                    note: self.look.map_note,
                    legend: self.look.map_legend,
                    redact: self.notes.overrides(),
                    animate: self.look.moves(),
                })
            {
                self.map_call.start(now, self.tab == Tab::Map);
            }
        }
        self.notes.set_known(self.export.secrets(), now);
        self.map.redact(self.notes.overrides());
        // Before an autosave trims the keep list to values still in use.
        if self.notes.saving() {
            self.notes.set_elsewhere(self.map.note_texts());
        }
        self.notes.poll(ctx);
        self.remember_site_configs();
        self.collect();
        self.keep_results();
        self.dropped_files(ctx);
        pump(&mut self.info_job, |i| self.info = i);
        let mut found = None;
        pump(&mut self.update_job, |f| found = Some(f));
        if let Some(f) = found {
            match &f {
                update::Found::Newer(r) => widgets::toast(
                    format!("Version {} is out: see the status bar", r.version),
                    true,
                ),
                update::Found::UpToDate if self.update_asked => {
                    widgets::toast("is-alive is up to date", true)
                }
                update::Found::Failed(_) if self.update_asked => {
                    widgets::toast("Couldn't check for updates (Settings says why)", false)
                }
                _ => {}
            }
            self.update = Some(f);
        }
        let mut picked = None;
        pump(&mut self.pick_image, |p| picked = p);
        if let Some(p) = picked {
            self.look.backdrop = backdrop::Source::Image(p);
            theme::save(&self.look);
        }
        let mut folder = widgets::take_picked_save_dir();
        pump(&mut self.pick_save_dir, |p| folder = p);
        if let Some(d) = folder {
            self.look.save_dir = Some(d);
            self.save_dir_changed();
        }
    }

    /// The tab's own content, faded in for a moment after a switch.
    fn central(&mut self, ui: &mut egui::Ui, now: f64) {
        // A quick fade as a tab comes in.
        let _ = now;
        let fade = theme::since(ui.ctx(), self.tab_since, theme::QUICK);
        // Six points over the run row, not eight: the run bar's share of
        // the 130-point budget (chrome.rs).
        let frame = egui::Frame::central_panel(ui.style()).inner_margin(egui::Margin {
            top: 6,
            ..egui::Margin::same(8)
        });
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.multiply_opacity(fade);
            self.tab_ui(ui)
        });
    }

    /// The Site Bundle window, over everything when it's open.
    /// Closing with autosave off and the notes changed: save them first?
    fn close_dialog(&mut self, ctx: &egui::Context) {
        if !self.close_asked {
            return;
        }
        let mut choice = None;
        let r = egui::Modal::new(egui::Id::new("close_unsaved")).show(ctx, |ui| {
            ui.set_max_width(340.0);
            ui.strong("Save the notes before closing?");
            ui.label("Autosave is off, and the notes have changes that aren't saved.");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(true);
                }
                if ui
                    .button(egui::RichText::new("Don't save").color(widgets::bad()))
                    .clicked()
                {
                    choice = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    ui.close();
                }
            });
        });
        match choice {
            Some(save) => {
                if save {
                    self.notes.set_elsewhere(self.map.note_texts());
                    if !self.notes.save_now() {
                        // The error shows in Notes; the window stays open.
                        self.close_asked = false;
                        self.tab = Tab::Notes;
                        return;
                    }
                }
                self.close_anyway = true;
                self.close_asked = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            None if r.should_close() => self.close_asked = false,
            None => {}
        }
    }

    /// Settings › Remove everything is-alive saved: says what's there
    /// and asks. On yes it's removed and the app closes, writing nothing
    /// on the way out.
    fn remove_dialog(&mut self, ctx: &egui::Context) {
        let Some((what, files)) = self.remove_ask.clone() else {
            return;
        };
        let Some(dir) = theme::data_dir() else {
            // Removed already (or nowhere to keep anything): say how it
            // went until the window closes.
            if let Some(e) = &self.remove_failed {
                egui::Modal::new(egui::Id::new("remove_failed")).show(ctx, |ui| {
                    ui.set_max_width(420.0);
                    ui.strong("Not all of it could be removed");
                    ui.label(e.as_str());
                    if ui.button("Close is-alive").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            }
            return;
        };
        let portable = theme::is_portable();
        let mut go = false;
        let r = egui::Modal::new(egui::Id::new("remove_everything")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.strong("Remove everything is-alive saved on this computer?");
            ui.label(
                egui::RichText::new(dir.display().to_string())
                    .monospace()
                    .small(),
            );
            ui.add_space(4.0);
            if what.is_empty() {
                ui.label("Nothing is saved there yet.");
            } else {
                for w in &what {
                    ui.label(format!("•  {w}"));
                }
                ui.weak(format!(
                    "{files} file{} in all. This can't be undone.",
                    if files == 1 { "" } else { "s" }
                ));
            }
            ui.add_space(4.0);
            ui.weak(if portable {
                "The folder itself stays, empty, so is-alive is still portable here. Files \
                 you saved elsewhere with Save… or Export stay too. is-alive closes afterwards."
            } else {
                "Files you saved elsewhere with Save… or Export stay: they're yours. \
                 is-alive closes afterwards, and starts as new next time."
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .button(egui::RichText::new("Remove it all and close").color(widgets::bad()))
                    .clicked()
                {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    ui.close();
                }
            });
        });
        if go {
            // Runs under way would write as they finish: stopped first.
            self.notes.set_autosave(false);
            match theme::remove_everything(&dir, portable) {
                Ok(()) => {
                    self.remove_ask = None;
                    self.close_anyway = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(e) => self.remove_failed = Some(e),
            }
        } else if r.should_close() {
            self.remove_ask = None;
        }
    }

    fn bundle_window(&mut self, ctx: &egui::Context) {
        let App {
            bundle,
            export,
            sweep,
            log,
            info,
            notes,
            map,
            ..
        } = self;
        bundle.window(
            ctx,
            tools::bundle::Sources {
                export,
                sweep,
                log,
                info,
                notes,
                map,
            },
        );
    }

    fn report_window(&mut self, ctx: &egui::Context) {
        self.report.window(
            ctx,
            tools::report::Sources {
                map: &self.map,
                map_src: map::view::Sources {
                    sweep: &self.sweep,
                    export: &self.export,
                    info: &self.info,
                    busy: false,
                    note: self.look.map_note,
                    legend: self.look.map_legend,
                    redact: self.notes.overrides(),
                    animate: self.look.moves(),
                },
                log: &self.log,
                info: &self.info,
                notes: &self.notes,
            },
        );
    }

    fn tab_ui(&mut self, ui: &mut egui::Ui) {
        match self.tab {
            Tab::Ping => self.ping.ui(ui),
            Tab::Ports => self.ports.ui(ui),
            Tab::Trace => self.trace.ui(ui),
            Tab::Speed => self.speed.ui(ui),
            Tab::Sweep => {
                let ask = self.sweep.ui(ui, &self.info);
                self.inspector_ask(ui.ctx(), ask);
            }
            Tab::Dns => self.dns.ui(ui),
            Tab::Advanced => self.advanced.ui(ui),
            Tab::Export => {
                self.export.ui(ui);
                // Its UniFi clients choice is kept, so a site's backup
                // reopens with the clients the Map shows.
                if self.export.clients() != self.look.unifi_clients {
                    self.look.unifi_clients = self.export.clients();
                    theme::save(&self.look);
                }
            }
            Tab::Map => {
                let busy =
                    self.ping.is_running() || self.ports.is_running() || self.trace.is_running();
                let act = self.map.ui(
                    ui,
                    map::view::Sources {
                        sweep: &self.sweep,
                        export: &self.export,
                        info: &self.info,
                        busy,
                        note: self.look.map_note,
                        legend: self.look.map_legend,
                        redact: self.notes.overrides(),
                        animate: self.look.moves(),
                    },
                );
                self.map_action(ui.ctx(), act);
            }
            Tab::Notes => self.notes.ui(ui, &self.info, &mut self.look),
            Tab::Calc => {
                if let Some(range) = self.calc.ui(ui, &self.info, self.sweep.is_running()) {
                    self.sweep.run(ui.ctx(), &range);
                    self.tab = Tab::Sweep;
                }
            }
            Tab::Join => self.join.ui(ui),
            Tab::Results => self.results_ui(ui),
        }
    }

    /// Files dropped on the window go where they make sense on this tab
    /// (`route_drop`); while they hover, say where.
    fn dropped_files(&mut self, ctx: &egui::Context) {
        let (hovering, hovered, dropped) = ctx.input(|i| {
            (
                !i.raw.hovered_files.is_empty(),
                i.raw
                    .hovered_files
                    .iter()
                    .filter_map(|f| f.path.clone())
                    .collect::<Vec<_>>(),
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .collect::<Vec<_>>(),
            )
        });
        if !dropped.is_empty() {
            let (notes, export, show) = route_drop(self.tab, dropped);
            if !notes.is_empty() && !export.is_empty() {
                widgets::toast(
                    format!(
                        "Photos and notes go into Notes; {} other file{} to Config Export",
                        export.len(),
                        if export.len() == 1 { "" } else { "s" }
                    ),
                    true,
                );
            }
            self.notes.open_files(ctx, notes);
            // Read now, or right after the read under way.
            self.export.open(ctx, export);
            self.tab = show;
        }
        let fade = theme::appear(ctx, egui::Id::new("drop_fade"), hovering);
        if fade > 0.0 {
            let rect = ctx.content_rect();
            let p = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("drop"),
            ));
            p.rect_filled(
                rect,
                0.0,
                egui::Color32::from_black_alpha((180.0 * fade) as u8),
            );
            let (what, to) = drop_hint(self.tab, &hovered);
            let white = egui::Color32::WHITE.gamma_multiply(fade);
            let wrap = (rect.width() - 64.0).max(200.0);
            let line = |text: &str, size: f32, colour: egui::Color32| {
                let mut job = egui::text::LayoutJob::simple(
                    text.to_string(),
                    egui::FontId::proportional(size),
                    colour,
                    wrap,
                );
                job.halign = egui::Align::Center;
                p.layout_job(job)
            };
            let what = line(what, 18.0, white);
            let to = line(to, 15.0, white.gamma_multiply(0.8));
            let h = what.rect.height() + 8.0 + to.rect.height();
            let mut y = rect.center().y - h / 2.0;
            for g in [what, to] {
                let x = rect.center().x - g.rect.center().x;
                let next = y + g.rect.height() + 8.0;
                p.galley(egui::pos2(x, y), g, white);
                y = next;
            }
        }
    }

    /// What an inspector's button asked for: a test in its own tab, or the
    /// device on the Map.
    fn inspector_ask(&mut self, ctx: &egui::Context, ask: Option<inspector::Ask>) {
        match ask {
            Some(inspector::Ask::Ping(ip)) => {
                self.ping.run(ctx, &ip, 4);
                self.tab = Tab::Ping;
            }
            Some(inspector::Ask::Ports(ip)) => {
                self.ports.run(ctx, &ip);
                self.tab = Tab::Ports;
            }
            Some(inspector::Ask::Trace(ip)) => {
                self.trace.run(ctx, &ip);
                self.tab = Tab::Trace;
            }
            Some(inspector::Ask::OnMap(ip)) => {
                self.map.find(ip);
                self.tab = Tab::Map;
            }
            // Fills the box; Start is the technician's.
            Some(inspector::Ask::Join(host)) => {
                self.join.ask_about(&host);
                self.tab = Tab::Join;
            }
            None => {}
        }
    }

    fn map_action(&mut self, ctx: &egui::Context, act: Option<map::view::Action>) {
        use map::view::Action;
        match act {
            Some(Action::Sweep) => {
                if let Some((ip, bits)) = self.info.lan
                    && !self.sweep.is_running()
                {
                    self.sweep.run(ctx, &net::targets::subnet_cidr(ip, bits));
                }
            }
            // Config Export's dialog; the Map stays on screen and redraws.
            Some(Action::Open) => self.export.open_dialog(ctx),
            Some(Action::Ping(ip)) => {
                self.ping.run(ctx, &ip, 4);
                self.tab = Tab::Ping;
            }
            Some(Action::Ports(ip)) => {
                self.ports.run(ctx, &ip);
                self.tab = Tab::Ports;
            }
            Some(Action::Trace(ip)) => {
                self.trace.run(ctx, &ip);
                self.tab = Tab::Trace;
            }
            Some(Action::Join(host)) => {
                self.join.ask_about(&host);
                self.tab = Tab::Join;
            }
            Some(Action::Bundle) => self.bundle.open = true,
            Some(Action::Example(i)) => self.export.open_example(ctx, i),
            Some(Action::ClearLive) => {
                // The site forgets its configs too, found or not.
                self.site_missing.clear();
                self.export.clear();
                self.sweep.clear_results();
                self.map.clear_live();
            }
            Some(Action::CloseExample) => {
                self.export.close_examples();
            }
            Some(Action::HideNote) => {
                self.look.map_note = false;
                theme::save(&self.look);
            }
            Some(Action::Layout(m)) => {
                self.look.map_layout = Some(m);
                theme::save(&self.look);
            }
            Some(Action::Legend(on)) => {
                self.look.map_legend = on;
                theme::save(&self.look);
            }
            Some(Action::NotePick(text, pick)) => {
                let now = ctx.input(|i| i.time);
                self.notes.set_elsewhere(self.map.note_texts());
                self.notes.apply_pick(&text, pick, now);
            }
            Some(Action::Reload) => {
                let files = self.export.map_sources();
                self.map.reload();
                if files.is_empty() {
                    widgets::toast("Map rebuilt", true);
                } else if self.export.is_running() {
                    widgets::toast(
                        "Config Export is still loading a file: rebuilt from what's loaded. \
                         Reload again when it's done to re-read the map's files.",
                        false,
                    );
                } else {
                    self.export.reopen(ctx, files);
                    self.reloading = true;
                }
            }
            None => {}
        }
    }

    /// Ask GitHub for the latest release. `asked`: from Check now, so say
    /// how it went; on start, only a newer release is worth a word.
    fn check_updates(&mut self, ctx: &egui::Context, asked: bool) {
        self.update_asked = asked;
        self.update_job = Some(Job::spawn(ctx, |tx| tx.send(update::check(VERSION))));
    }

    fn refresh_info(&mut self, ctx: &egui::Context) {
        self.wan_asked = true;
        self.info_job = Some(Job::spawn(ctx, |tx| tx.send(HostInfo::fetch())));
    }

    fn tab_running(&self, tab: Tab) -> bool {
        match tab {
            Tab::Ping => self.ping.is_running(),
            Tab::Ports => self.ports.is_running(),
            Tab::Trace => self.trace.is_running(),
            Tab::Speed => self.speed.is_running(),
            Tab::Sweep => self.sweep.is_running(),
            Tab::Dns => self.dns.is_running(),
            Tab::Advanced => self.advanced.is_running(),
            Tab::Export => self.export.is_running(),
            Tab::Join => {
                self.join.running_since().is_some() || self.join.others_running_since().is_some()
            }
            Tab::Map | Tab::Notes | Tab::Calc | Tab::Results => false,
        }
    }

    fn any_running(&self) -> bool {
        self.ping.is_running()
            || self.ports.is_running()
            || self.trace.is_running()
            || self.speed.is_running()
            || self.sweep.is_running()
            || self.dns.is_running()
            || self.advanced.is_running()
            || self.netcheck.running_since().is_some()
    }

    /// Run every tool against this machine's network and switch to Results.
    fn full_scan(&mut self, ctx: &egui::Context) {
        // LAN and gateway are re-read now; the WAN address refreshes alongside.
        let local = HostInfo::local();
        self.info.lan = local.lan;
        self.info.gateway = local.gateway;
        self.refresh_info(ctx);

        let now = log::now();
        self.log.push(log::Entry {
            tool: "Host info",
            started: now.clone(),
            ended: now,
            text: format!("is-alive {VERSION}\n{}", self.info.summary()),
            table: Some(self.info.table(VERSION)),
        });

        let gw = local.gateway.map(|g| g.to_string());
        self.ping.run(ctx, gw.as_deref().unwrap_or(WAN_TARGET), 4);
        self.scanning = vec![Tab::Ping, Tab::Trace, Tab::Dns];
        if let Some(gw) = &gw {
            self.ports.run(ctx, gw);
            self.scanning.push(Tab::Ports);
        }
        self.trace.run(ctx, WAN_TARGET);
        self.dns.run(ctx, DNS_TEST_NAME);
        self.netcheck.run(ctx);
        self.join.check(ctx, true);
        if let Some((ip, bits)) = local.lan {
            self.sweep.run(ctx, &net::targets::subnet_cidr(ip, bits));
            self.scanning.push(Tab::Sweep);
        } else {
            // Only the sweep puts anything on the Map.
            self.scanning.clear();
        }
        self.map_call.seen();
        self.tab = Tab::Results;
        // Show the new runs: a leftover filter could hide them, and the
        // view may be scrolled up on older ones.
        self.filter.clear();
        self.jump_to_new = true;
    }

    /// Move finished runs into the log.
    fn collect(&mut self) {
        let done = [
            ("Ping", self.ping.take_finished()),
            ("Port Check", self.ports.take_finished()),
            ("Traceroute", self.trace.take_finished()),
            ("Speed Test", self.speed.take_finished()),
            ("Subnet Sweep", self.sweep.take_finished()),
            ("Lookup", self.dns.take_finished()),
            ("Domain Health", self.advanced.take_finished()),
            ("Network checks", self.netcheck.take_finished()),
            ("Domain join", self.join.take_finished()),
            (tools::join::OTHERS, self.join.take_finished_others()),
        ];
        for (tool, run) in done {
            if let Some((started, text, table)) = run {
                self.log.push(log::Entry {
                    tool,
                    started,
                    ended: log::now(),
                    text,
                    table,
                });
            }
        }
    }

    /// The open site's saved runs, as Results (none with Keep with site
    /// off); Scratch's from the last day only.
    pub(crate) fn load_results(&mut self) {
        self.log = match log::site_file().filter(|_| self.look.results_keep) {
            Some(p) => log::Log::load(&p),
            None => log::Log::default(),
        };
        // As read: runs that expired make it differ, so the file is written
        // again without them.
        self.log_saved = self.log.stamp();
        if site::current().is_none() {
            self.log
                .keep_since(chrono::Local::now().naive_local(), log::SCRATCH_HOURS);
        }
    }

    /// Write Results to the open site's folder when they've changed: a run
    /// finished, the log was cleared. Not in Scratch, nor with Keep with
    /// site off.
    pub(crate) fn keep_results(&mut self) {
        let stamp = self.log.stamp();
        if stamp == self.log_saved || !self.look.results_keep || self.shots.is_some() {
            return;
        }
        let Some(path) = log::site_file() else { return };
        match self.log.save(&path) {
            Ok(()) => self.log_saved = stamp,
            Err(e) => {
                // Said once; tried again at the next change.
                self.log_saved = stamp;
                widgets::toast(format!("Results not saved: {e}"), false);
            }
        }
    }

    /// Keep with site turned on or off. On: the site's saved runs come back
    /// ahead of this session's, and all of them are saved. Off: the open
    /// site's file goes (its runs stay in Results until is-alive closes).
    fn set_results_keep(&mut self, on: bool) {
        self.look.results_keep = on;
        theme::save(&self.look);
        if on {
            let now = std::mem::take(&mut self.log);
            self.load_results();
            for e in now.entries {
                self.log.push(e);
            }
            self.log.add_dropped(now.dropped);
        } else if let Some(p) = log::site_file()
            && let Err(e) = log::Log::default().save(&p)
        {
            widgets::toast(format!("Couldn't remove the saved Results: {e}"), false);
        }
    }

    /// Runs still in progress, with their live output.
    fn running(&self) -> Vec<(&'static str, String, String)> {
        [
            ("Ping", self.ping.running_since(), self.ping.output()),
            (
                "Port Check",
                self.ports.running_since(),
                self.ports.output(),
            ),
            (
                "Traceroute",
                self.trace.running_since(),
                self.trace.output(),
            ),
            (
                "Speed Test",
                self.speed.running_since(),
                self.speed.output(),
            ),
            (
                "Subnet Sweep",
                self.sweep.running_since(),
                self.sweep.output(),
            ),
            ("Lookup", self.dns.running_since(), self.dns.output()),
            (
                "Domain Health",
                self.advanced.running_since(),
                self.advanced.output(),
            ),
            (
                "Network checks",
                self.netcheck.running_since(),
                self.netcheck.output(),
            ),
            ("Domain join", self.join.running_since(), None),
            (
                tools::join::OTHERS,
                self.join.others_running_since(),
                self.join.others_output(),
            ),
        ]
        .into_iter()
        .filter_map(|(tool, since, out)| Some((tool, since?.to_owned(), out.unwrap_or_default())))
        .collect()
    }
}

/// Digit of the tab's Ctrl shortcut: 1-9 for the first nine, 0 for Results
/// (the last); any tab in between has none.
fn tab_key(n: usize) -> Option<usize> {
    if n < 9 {
        Some(n + 1)
    } else if n == TABS.len() - 1 {
        Some(0)
    } else {
        None
    }
}

/// Where a file dropped on the window goes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    /// Into Notes, as its own Open… takes it: a photo, or notes.
    Notes,
    /// Config Export reads it while the Map stays on screen and redraws
    /// (a saved map opens on the Map).
    Map,
    /// Config Export reads it, and comes on screen to show it.
    Export,
}

/// Where `path`, dropped on `tab`, goes: on Notes a photo or notes stay
/// there; on the Map configs and maps open and the Map stays; anything
/// else is Config Export's, which shows it.
fn drop_target(tab: Tab, path: &std::path::Path) -> Target {
    match tab {
        Tab::Notes if tools::notes::takes_drop(path) => Target::Notes,
        Tab::Map => Target::Map,
        _ => Target::Export,
    }
}

/// Files dropped on `tab`, split by where they go: (Notes', Config
/// Export's, the tab to show then). Config Export comes on screen to show
/// what it reads, unless the Map redraws with it, or Notes took some of the
/// files.
fn route_drop(
    tab: Tab,
    paths: Vec<std::path::PathBuf>,
) -> (Vec<std::path::PathBuf>, Vec<std::path::PathBuf>, Tab) {
    let (notes, export): (Vec<_>, Vec<_>) = paths
        .into_iter()
        .partition(|p| drop_target(tab, p) == Target::Notes);
    let shows = !export.is_empty()
        && notes.is_empty()
        && export.iter().all(|p| drop_target(tab, p) == Target::Export);
    (notes, export, if shows { Tab::Export } else { tab })
}

/// What the drop overlay says on `tab`: what can be dropped, and where it
/// goes (by the files hovering, when the system names them before the drop).
fn drop_hint(tab: Tab, hovered: &[std::path::PathBuf]) -> (&'static str, &'static str) {
    const KINDS: &str = "Drop a UniFi backup, SonicWall export or report, Meraki export, \
                         /mapnetwork map or saved map";
    let notes = hovered.iter().any(|p| drop_target(tab, p) == Target::Notes);
    let other = hovered.iter().any(|p| drop_target(tab, p) != Target::Notes);
    match tab {
        Tab::Notes if notes && !other => (
            "Drop a photo or notes",
            "Into Notes, as File › Open… adds it",
        ),
        Tab::Notes if other && !notes => (KINDS, "Config Export opens to read it"),
        Tab::Notes => (
            "Drop a photo or notes (.md, .txt), or a config or map",
            "Photos and notes go into Notes; configs and maps to Config Export",
        ),
        Tab::Map => (
            KINDS,
            "The Map stays on screen and redraws with it (Meraki configs open in Config \
             Export: the Map doesn't draw them)",
        ),
        Tab::Export => (KINDS, "Config Export reads it"),
        _ => (KINDS, "Config Export opens to read it"),
    }
}

/// Config Export's notices as toasts: good news as it is, and of the files
/// that couldn't be read the first two, then how many more.
fn notice_toasts(notices: Vec<(String, bool)>) -> Vec<(String, bool)> {
    let (good, bad): (Vec<_>, Vec<_>) = notices.into_iter().partition(|n| n.1);
    let more = bad.len().saturating_sub(2);
    let mut out = good;
    out.extend(bad.into_iter().take(2));
    if more > 0 {
        out.push((
            format!(
                "…and {more} more file{} couldn't be opened: Config Export says why",
                if more == 1 { "" } else { "s" }
            ),
            false,
        ));
    }
    out
}

/// Window and taskbar icon; the same artwork is in the .exe and the AppImage.
fn icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png"))
        .expect("valid icon PNG")
}

/// Don't pass on file descriptors this process was started with. The AppImage
/// runtime leaves one open on its mount (/tmp/.mount_…) without close-on-exec,
/// so every tool we start (chronyc, ip, ping) would inherit it; SELinux then
/// denies the confined ones (chronyc_t) access and raises an alert each time.
#[cfg(target_os = "linux")]
fn close_inherited_fds_on_exec() {
    let Ok(dir) = std::fs::read_dir("/proc/self/fd") else {
        return;
    };
    let fds: Vec<libc::c_int> = dir
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .filter(|&fd| fd > 2)
        .collect();
    for fd in fds {
        // SAFETY: F_GETFD/F_SETFD only change the descriptor's flags; a stale
        // number (such as read_dir's own, now closed) just fails with EBADF.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

fn main() {
    // First, so nothing after it can end the app without a word.
    startup::install_panic_hook();
    #[cfg(target_os = "linux")]
    close_inherited_fds_on_exec();
    let look = theme::load();
    // The site open last time (none for the screenshot tours, which show
    // the app as a new user sees it).
    site::set_current(
        look.site
            .clone()
            .filter(|_| std::env::var_os("IS_ALIVE_SHOTS").is_none()),
    );
    let plan = match startup::plan(look.renderer) {
        Ok(p) => p,
        Err(e) => startup::bad_arguments(&e),
    };
    theme::set_emoji_style(look.emoji);
    let size = look
        .size
        .map_or([1120.0, 720.0], |(w, h)| [w as f32, h as f32]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("is-alive {VERSION} — network tester"))
            .with_app_id("is-alive")
            .with_inner_size(size)
            .with_min_inner_size([620.0, 400.0])
            .with_maximized(look.maximized)
            .with_drag_and_drop(true)
            .with_icon(icon()),
        renderer: plan.renderer,
        wgpu_options: startup::wgpu_options(),
        ..Default::default()
    };
    startup::begin(&plan);
    let files = plan.files;
    let run = eframe::run_native(
        "is-alive",
        options,
        Box::new(|cc| {
            startup::created(cc);
            #[cfg(target_os = "linux")]
            wayland_icon::set(cc, &icon());
            cc.egui_ctx.set_fonts(fonts::definitions());
            theme::apply(&cc.egui_ctx, look.theme);
            widgets::init(&cc.egui_ctx);
            net::clock::warm();
            // The screenshot tour only sees the app's window: the manual
            // stays over it there.
            let look_manual_window =
                look.manual_window && std::env::var_os("IS_ALIVE_SHOTS").is_none();
            let mut app = App::fresh(look, cc.gl.as_ref(), look_manual_window);
            app.notes.set_autosave(app.look.notes_autosave);
            app.export.set_clients(app.look.unifi_clients);
            widgets::set_save_dir(app.look.save_dir.clone(), app.look.save_by_site);
            app.load_results();
            // The site opens as it was left: its configs read again (and
            // then any files it was opened with, below, which join them).
            if app.shots.is_none() {
                app.reopen_site_configs(&cc.egui_ctx);
                site::touch();
            }
            app.retitle(&cc.egui_ctx);
            // Local, and quick: the status bar names the domain from the start.
            app.join.check(&cc.egui_ctx, false);
            if app.look.wan_on_start && app.shots.is_none() {
                app.refresh_info(&cc.egui_ctx);
            }
            if app.look.updates_on_start && app.shots.is_none() {
                app.check_updates(&cc.egui_ctx, false);
            }
            // Files named on the command line ("Open with") go to Config
            // Export, read after the site's own: the site's list grows.
            if !files.is_empty() {
                app.tab = Tab::Export;
                app.export.open(&cc.egui_ctx, files);
            }
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = run {
        startup::failed(&e);
    }
    // A screenshot tour with a picture that isn't what it should be.
    if shots::failed() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        App, TABS, Tab, Target, drop_hint, drop_target, egui, fonts, log, notice_toasts,
        results_ui, route_drop, small_screen, theme, tools,
    };
    use std::path::{Path, PathBuf};

    /// The screens the first window opens maximised on: a 1080p laptop at
    /// 150% and 1366 × 768; not 1280 × 800 (it fits) or bigger.
    /// One frame of the window's own drawing (bars, nav, the tab), at
    /// `size`, without the parts that read or write this computer.
    fn draw(app: &mut App, ctx: &egui::Context, size: (f32, f32)) {
        let now = ctx.input(|i| i.time) + 0.2;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size.0, size.1),
            )),
            time: Some(now),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            app.title_bar(ui);
            app.footer(ui);
            app.nav(ui);
            app.central(ui, now);
        });
        out.textures_delta.clear();
    }

    fn run(tool: &'static str, started: &str) -> log::Entry {
        log::Entry {
            tool,
            started: started.into(),
            ended: started.into(),
            text: format!("{tool}\nline two\n"),
            table: Some(log::Table {
                columns: &["host", "ms"],
                rows: vec![vec!["192.0.2.1".into(), "1.0".into()]],
            }),
        }
    }

    /// Release builds end on any panic, and v0.28.21's was an index kept
    /// past a list that had been cleared. So every tab is drawn, at three
    /// window sizes and in every theme's colours: empty, as it starts;
    /// with a map and runs in it and something picked on each; and then
    /// with all of that cleared from under whatever the tabs had picked,
    /// which is harsher than Clear all or Clear map, since they tidy up.
    #[test]
    fn every_tab_draws_empty_filled_and_just_cleared() {
        for (n, size) in [(1280.0, 720.0), (1120.0, 720.0), (620.0, 400.0)]
            .into_iter()
            .enumerate()
        {
            let ctx = egui::Context::default();
            ctx.set_fonts(fonts::definitions());
            let look = theme::Settings {
                theme: theme::Theme::ALL[n * 4],
                ..Default::default()
            };
            theme::apply(&ctx, look.theme);
            let mut app = App::fresh(look, None, false);
            let all = |app: &mut App, what: &str| {
                for (tab, name, _) in TABS {
                    app.tab = *tab;
                    for _ in 0..3 {
                        draw(app, &ctx, size);
                    }
                    let _ = (name, what);
                }
            };
            all(&mut app, "empty");

            app.export = tools::export::ExportTool::with_example(0);
            for (i, (_, name, _)) in TABS.iter().enumerate() {
                app.log
                    .push(run(name, &format!("2026-11-12T10:{i:02}:00+00:00")));
                app.log
                    .push(run(name, &format!("2026-11-12T11:{i:02}:00+00:00")));
            }
            app.result_pick = Some(results_ui::RunKey::Done {
                tool: TABS[3].1,
                started: "2026-11-12T11:03:00+00:00".into(),
            });
            app.result_compare = Some("2026-11-12T10:03:00+00:00".into());
            app.result_view = results_ui::RunView::Compare;
            app.filter = "192".into();
            all(&mut app, "filled");
            for view in [results_ui::RunView::Table, results_ui::RunView::Output] {
                app.result_view = view;
                app.tab = Tab::Results;
                draw(&mut app, &ctx, size);
            }

            // The Map opened up into the exploded view, and split; at one
            // size it stays so while everything goes from under it.
            app.map.explode_for_shots(Some(1.0), false);
            all(&mut app, "exploded");
            // And in 3D, where cards are drawn at sizes of their own.
            app.map.explode_for_shots(Some(1.0), true);
            all(&mut app, "in 3D");
            if n != 1 {
                app.map.explode_for_shots(None, false);
            }

            // Everything goes, and nothing the tabs hold on to is told.
            app.log.clear();
            app.export.clear();
            app.sweep.clear_results();
            all(&mut app, "the data gone");
            app.map.clear_live();
            app.result_view = results_ui::RunView::Compare;
            all(&mut app, "the map cleared");
        }
        theme::apply(&egui::Context::default(), theme::Theme::Dark);
    }

    #[test]
    fn small_screens_get_a_maximised_first_window() {
        let at = |w: f32, h: f32| small_screen(egui::vec2(w, h));
        assert!(at(1280.0, 720.0));
        assert!(at(1366.0, 768.0));
        assert!(at(1097.0, 617.0)); // 1080p at 175%
        assert!(!at(1280.0, 800.0));
        assert!(!at(1920.0, 1080.0));
        assert!(!at(1440.0, 900.0));
    }

    /// Where a dropped file goes, by the tab it's dropped on.
    #[test]
    fn a_drop_goes_where_it_makes_sense_on_its_tab() {
        let to = |tab, f: &str| drop_target(tab, Path::new(f));
        // Notes takes photos and notes, as its Open… would; the rest is
        // Config Export's.
        for f in ["rack.jpg", "label.PNG", "x.webp", "visit.md", "visit.txt"] {
            assert_eq!(to(Tab::Notes, f), Target::Notes, "{f}");
        }
        for f in [
            "backup.unf",
            "fw.exp",
            "acme.dc.html",
            "saved.json",
            "bundle.zip",
        ] {
            assert_eq!(to(Tab::Notes, f), Target::Export, "{f}");
        }
        // On the Map, configs and maps open there (a .dc.html too); a photo
        // isn't Notes' from the Map.
        for f in [
            "backup.unf",
            "fw.wri",
            "acme.dc.html",
            "saved.json",
            "rack.jpg",
        ] {
            assert_eq!(to(Tab::Map, f), Target::Map, "{f}");
        }
        for tab in [Tab::Export, Tab::Ping, Tab::Sweep, Tab::Results] {
            assert_eq!(to(tab, "rack.jpg"), Target::Export, "{tab:?}");
            assert_eq!(to(tab, "backup.unf"), Target::Export, "{tab:?}");
        }
    }

    /// Which tab shows after a drop: the Map stays, Notes stays when it
    /// took a file, and elsewhere Config Export comes on screen.
    #[test]
    fn a_drop_shows_where_it_went() {
        let p = |v: &[&str]| v.iter().map(PathBuf::from).collect::<Vec<_>>();
        let (n, e, show) = route_drop(Tab::Map, p(&["a.unf", "b.dc.html"]));
        assert!(n.is_empty());
        assert_eq!(e, p(&["a.unf", "b.dc.html"]));
        assert_eq!(show, Tab::Map);
        let (n, e, show) = route_drop(Tab::Notes, p(&["rack.jpg", "a.unf", "visit.md"]));
        assert_eq!(n, p(&["rack.jpg", "visit.md"]));
        assert_eq!(e, p(&["a.unf"]));
        assert_eq!(show, Tab::Notes, "Notes took some: it stays");
        let (n, e, show) = route_drop(Tab::Notes, p(&["a.unf"]));
        assert!(n.is_empty() && e.len() == 1);
        assert_eq!(show, Tab::Export);
        let (_, _, show) = route_drop(Tab::Ping, p(&["rack.jpg"]));
        assert_eq!(show, Tab::Export, "as before: Config Export says why");
        let (_, _, show) = route_drop(Tab::Export, p(&["a.unf"]));
        assert_eq!(show, Tab::Export);
    }

    /// The overlay names Meraki, and says where the drop goes on this tab.
    #[test]
    fn the_drop_overlay_says_where_it_goes() {
        let none: &[PathBuf] = &[];
        for tab in [Tab::Ping, Tab::Export, Tab::Map] {
            let (what, _) = drop_hint(tab, none);
            for kind in ["UniFi", "SonicWall", "Meraki", "/mapnetwork", "saved map"] {
                assert!(what.contains(kind), "{kind} missing on {tab:?}: {what}");
            }
        }
        assert!(
            drop_hint(Tab::Map, none)
                .1
                .starts_with("The Map stays on screen")
        );
        assert!(drop_hint(Tab::Ping, none).1.contains("Config Export opens"));
        let photo = [PathBuf::from("rack.jpg")];
        assert_eq!(
            drop_hint(Tab::Notes, &photo).1,
            "Into Notes, as File › Open… adds it"
        );
        let backup = [PathBuf::from("a.unf")];
        assert!(drop_hint(Tab::Notes, &backup).1.contains("Config Export"));
    }

    /// Many files that couldn't be read are two toasts and a count, not one
    /// each.
    #[test]
    fn many_failures_are_told_in_short() {
        let n = |s: &str, ok| (s.to_string(), ok);
        let out = notice_toasts(vec![
            n("Couldn't open a", false),
            n("x.json is open in Config Export", true),
            n("Couldn't open b", false),
            n("Couldn't open c", false),
            n("Couldn't open d", false),
        ]);
        assert_eq!(out.len(), 4);
        assert!(out[0].1);
        assert_eq!(
            out[3].0,
            "…and 2 more files couldn't be opened: Config Export says why"
        );
    }

    #[test]
    fn icon_decodes() {
        let i = super::icon();
        assert_eq!((i.width, i.height), (256, 256));
    }

    /// Code points a TrueType/OpenType font maps (cmap formats 4 and 12).
    fn cmap(d: &[u8]) -> std::collections::HashSet<u32> {
        let u16_at = |o: usize| u16::from_be_bytes([d[o], d[o + 1]]) as usize;
        let u32_at = |o: usize| u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize;
        let table = (0..u16_at(4))
            .map(|i| 12 + 16 * i)
            .find(|&t| &d[t..t + 4] == b"cmap")
            .map(|t| u32_at(t + 8))
            .expect("font has a cmap");
        let mut cps = std::collections::HashSet::new();
        for i in 0..u16_at(table + 2) {
            let o = table + u32_at(table + 8 + 8 * i);
            match u16_at(o) {
                4 => {
                    let seg = u16_at(o + 6) / 2;
                    for s in 0..seg {
                        let end = u16_at(o + 14 + 2 * s);
                        let start = u16_at(o + 16 + 2 * seg + 2 * s);
                        cps.extend(start as u32..=end as u32);
                    }
                }
                12 => {
                    for g in 0..u32_at(o + 12) {
                        let at = o + 16 + 12 * g;
                        cps.extend(u32_at(at) as u32..=u32_at(at + 4) as u32);
                    }
                }
                _ => {}
            }
        }
        cps
    }

    /// Symbols a map's text (a /mapnetwork map, a note) often carries draw
    /// in proportional text too, not as boxes: arrows, box-drawing lines,
    /// triangles, dashes.
    #[test]
    fn map_symbols_render_in_sans_text() {
        use eframe::egui::FontFamily;
        let defs = crate::fonts::definitions();
        let missing: Vec<char> = "·—–×→←↑↓↔⇄─│├└┌┐┘┬┴┼▲▼◆•…±°µ≤≥"
            .chars()
            .filter(|&c| {
                !defs.families[&FontFamily::Proportional]
                    .iter()
                    .any(|name| cmap(&defs.font_data[name].font).contains(&(c as u32)))
            })
            .collect();
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }

    /// Every character the app writes must exist in egui's bundled fonts,
    /// for both proportional and monospace text, or it shows as a box.
    #[test]
    fn every_glyph_in_the_source_renders() {
        use eframe::egui::FontFamily;
        let defs = crate::fonts::definitions();
        let covers = |fam: &FontFamily, c: char| {
            defs.families[fam]
                .iter()
                .any(|name| cmap(&defs.font_data[name].font).contains(&(c as u32)))
        };
        fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(p).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")),
            &mut files,
        );
        let mut missing = Vec::new();
        for f in files {
            for (n, line) in std::fs::read_to_string(&f).unwrap().lines().enumerate() {
                // Comments aren't drawn.
                let code = line.split("//").next().unwrap_or_default();
                for c in code.chars().filter(|c| !c.is_ascii()) {
                    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
                        if !covers(&fam, c) {
                            missing.push(format!("{c:?} ({fam:?}) at {}:{}", f.display(), n + 1));
                        }
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "renders as a box:\n{}",
            missing.join("\n")
        );
    }
}
