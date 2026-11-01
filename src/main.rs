#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backdrop;
mod bundle;
mod chrome;
mod fonts;
mod job;
mod log;
mod map;
mod mapnetwork;
mod markdown;
mod meraki;
mod net;
mod redact;
mod shots;
mod skills;
mod sonicwall;
mod theme;
mod tools;
mod unifi;
mod widgets;

use eframe::egui;
use job::{Job, pump};
use net::hostinfo::HostInfo;

/// Internet target for the full scan's traceroute (and ping when there's no gateway).
const WAN_TARGET: &str = "1.1.1.1";
const DNS_TEST_NAME: &str = "cloudflare.com";
/// Shown in the title bar, the footer and the full scan's Host info entry,
/// so a screenshot or pasted log always says which build produced it.
const VERSION: &str = env!("CARGO_PKG_VERSION");
const FULL_SCAN_TIP: &str = "Full scan: ping and port-check the gateway, trace to the internet, test DNS, sweep the LAN. Results opens to show them. (Speed Test runs from its own tab.)";

#[derive(PartialEq, Clone, Copy)]
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
    Results,
}

/// (tab, label, hover text).
const TABS: &[(Tab, &str, &str)] = &[
    (
        Tab::Ping,
        "Ping",
        "ICMP or TCP ping with a live latency graph",
    ),
    (Tab::Ports, "Port Check", "Find open TCP ports on a host"),
    (Tab::Trace, "Traceroute", "Show each router hop to a host"),
    (
        Tab::Sweep,
        "Subnet Sweep",
        "Find live hosts on a subnet, with name, MAC and vendor",
    ),
    (
        Tab::Dns,
        "DNS Lookup",
        "Look up DNS records; Compare all spots split DNS and stale caches",
    ),
    (
        Tab::Advanced,
        "Advanced",
        "Mail and domain health: MX, SPF, DMARC, DKIM, blacklists and more",
    ),
    (
        Tab::Export,
        "Config Export",
        "Convert UniFi and SonicWall configs to JSON for the skills; open /mapnetwork maps",
    ),
    (
        Tab::Map,
        "Map",
        "Network map from the sweep, configs or a /mapnetwork map. Drag to pan, scroll to zoom",
    ),
    (Tab::Notes, "Notes", "Site notes in Markdown, autosaved"),
    (
        Tab::Speed,
        "Speed Test",
        "Speed, latency, bufferbloat and VoIP quality",
    ),
    (
        Tab::Calc,
        "Subnet Calc",
        "Subnet calculator: masks, host ranges, splitting a block, range to CIDR",
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
    /// Window size last frame, remembered for the next launch.
    size: Option<egui::Vec2>,
    ping: tools::ping::PingTool,
    ports: tools::ports::PortTool,
    trace: tools::trace::TraceTool,
    speed: tools::speed::SpeedTool,
    sweep: tools::sweep::SweepTool,
    calc: tools::calc::CalcTool,
    dns: tools::dns::DnsTool,
    advanced: tools::advanced::AdvancedTool,
    export: tools::export::ExportTool,
    bundle: tools::bundle::BundleTool,
    map: map::view::MapView,
    notes: tools::notes::NotesTool,
    log: log::Log,
    info: HostInfo,
    info_job: Option<Job<HostInfo>>,
    /// The public address has been asked for: on start (unless Settings
    /// turns that off), on Refresh or by a full scan.
    wan_asked: bool,
    /// The sweep was running last frame (to see one start).
    was_sweeping: bool,
    /// Results tab filter; matches tool name or output, case-insensitive.
    filter: String,
    /// ((filter, log stamp), matching entries) for the last frame.
    #[allow(clippy::type_complexity)]
    filter_cache: Option<((String, (usize, usize, usize)), Vec<usize>)>,
    /// Scroll Results to its newest runs on the next frame (after a full scan).
    jump_to_new: bool,
    look: theme::Settings,
    backdrop: backdrop::Backdrop,
    /// File dialog for the Miami backdrop image.
    pick_image: Option<Job<Option<std::path::PathBuf>>>,
    /// The map's files are being read again (Reload): say how it went.
    reloading: bool,
    shots: Option<shots::Shots>,
    /// The screenshot tour's last step: Settings open.
    shots_settings: bool,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.tab_keys(ui);
        self.shots_step(ui.ctx());
        let now = ui.input(|i| i.time);
        if self.tab != self.shown_tab {
            self.shown_tab = self.tab;
            self.tab_since = now;
        }
        self.size = ui.input(|i| i.viewport().inner_rect.map(|r| r.size()));
        self.poll_all(ui.ctx(), now);
        // Under every panel; Miami's panels are translucent over it.
        if self.look.theme == theme::Theme::Miami {
            self.backdrop.paint(
                ui.ctx(),
                &self.look.backdrop,
                self.look.animate,
                self.look.glow,
                self.look.vhs,
            );
        }
        self.header(ui);
        self.footer(ui);
        self.central(ui, now);
        widgets::show_toasts(ui.ctx());
        self.bundle_window(ui.ctx());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.notes.flush();
        widgets::clear_own_clipboard();
        let size = self
            .size
            .map(|s| (s.x.round() as u32, s.y.round() as u32))
            .filter(|&(w, h)| (620..=8000).contains(&w) && (400..=8000).contains(&h));
        if size.is_some() && size != self.look.size {
            self.look.size = size;
            theme::save(&self.look);
        }
    }
}

impl App {
    /// Ctrl+1 … Ctrl+9 for the first nine tabs, Ctrl+0 for Results.
    fn tab_keys(&mut self, ui: &mut egui::Ui) {
        use egui::Key::*;
        let keys = [Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9];
        for (n, (tab, ..)) in TABS.iter().enumerate() {
            if let Some(d) = tab_key(n)
                && ui.input_mut(|inp| inp.consume_key(egui::Modifiers::COMMAND, keys[d]))
            {
                self.tab = *tab;
            }
        }
    }

    /// The screenshot tour (IS_ALIVE_SHOTS): every tab, then the Map with a
    /// card selected, then Settings.
    fn shots_step(&mut self, ctx: &egui::Context) {
        if let Some(s) = &mut self.shots {
            // Every tab, then the Map again with a card selected.
            let mut names: Vec<&str> = TABS.iter().map(|t| t.1).collect();
            names.extend(["Map panel", "Settings"]);
            match s.step(ctx, &names) {
                Some(i) if i < TABS.len() => self.tab = TABS[i].0,
                Some(i) if i == TABS.len() => {
                    self.tab = crate::Tab::Map;
                    self.map.select_for_shots();
                }
                Some(_) => self.shots_settings = true,
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
        self.export.poll();
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
        self.notes.set_known(self.export.secrets(), now);
        self.notes.poll(ctx);
        self.collect();
        self.dropped_files(ctx);
        pump(&mut self.info_job, |i| self.info = i);
        let mut picked = None;
        pump(&mut self.pick_image, |p| picked = p);
        if let Some(p) = picked {
            self.look.backdrop = backdrop::Source::Image(p);
            theme::save(&self.look);
        }
    }

    /// The tab's own content, faded in for a moment after a switch.
    fn central(&mut self, ui: &mut egui::Ui, now: f64) {
        // A quick fade as a tab comes in.
        let fade = ((now - self.tab_since) / 0.16).min(1.0) as f32;
        if fade < 1.0 {
            ui.ctx().request_repaint();
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.multiply_opacity(fade);
            self.tab_ui(ui)
        });
    }

    /// The Site Bundle window, over everything when it's open.
    fn bundle_window(&mut self, ctx: &egui::Context) {
        let App {
            bundle,
            export,
            sweep,
            log,
            info,
            notes,
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
            },
        );
    }

    fn tab_ui(&mut self, ui: &mut egui::Ui) {
        match self.tab {
            Tab::Ping => self.ping.ui(ui),
            Tab::Ports => self.ports.ui(ui),
            Tab::Trace => self.trace.ui(ui),
            Tab::Speed => self.speed.ui(ui),
            Tab::Sweep => self.sweep.ui(ui, &self.info),
            Tab::Dns => self.dns.ui(ui),
            Tab::Advanced => self.advanced.ui(ui),
            Tab::Export => self.export.ui(ui),
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
            Tab::Results => self.results_ui(ui),
        }
    }

    /// Files dropped anywhere on the window go to Config Export; while they
    /// hover, say so.
    fn dropped_files(&mut self, ctx: &egui::Context) {
        let (hovering, dropped) = ctx.input(|i| {
            (
                !i.raw.hovered_files.is_empty(),
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .collect::<Vec<_>>(),
            )
        });
        if !dropped.is_empty() && !self.export.is_running() {
            self.tab = Tab::Export;
            self.export.open(ctx, dropped);
        }
        let fade = ctx.animate_bool(egui::Id::new("drop_fade"), hovering);
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
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop a UniFi backup, SonicWall export or report, or /mapnetwork map",
                egui::FontId::proportional(18.0),
                egui::Color32::WHITE.gamma_multiply(fade),
            );
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
            Some(Action::Export) => self.tab = Tab::Export,
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
            Some(Action::Bundle) => self.bundle.open = true,
            Some(Action::Example(i)) => self.export.open_example(ctx, i),
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
                    self.export.open(ctx, files);
                    self.reloading = true;
                }
            }
            None => {}
        }
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
        if let Some(gw) = &gw {
            self.ports.run(ctx, gw);
        }
        self.trace.run(ctx, WAN_TARGET);
        self.dns.run(ctx, DNS_TEST_NAME);
        if let Some((ip, bits)) = local.lan {
            self.sweep.run(ctx, &net::targets::subnet_cidr(ip, bits));
        }
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
            ("DNS Lookup", self.dns.take_finished()),
            ("Advanced", self.advanced.take_finished()),
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
            ("DNS Lookup", self.dns.running_since(), self.dns.output()),
            (
                "Advanced",
                self.advanced.running_since(),
                self.advanced.output(),
            ),
        ]
        .into_iter()
        .filter_map(|(tool, since, out)| Some((tool, since?.to_owned(), out.unwrap_or_default())))
        .collect()
    }

    fn results_ui(&mut self, ui: &mut egui::Ui) {
        let jump = std::mem::take(&mut self.jump_to_new);
        let running = self.running();
        let needle = self.filter.trim().to_lowercase();
        let hit = |tool: &str, text: &str| {
            needle.is_empty()
                || tool.to_lowercase().contains(&needle)
                || text.to_lowercase().contains(&needle)
        };
        // Matching the log is the costly part (it can be megabytes): only
        // when the filter or the log changes, not every frame.
        let key = (needle.clone(), self.log.stamp());
        if self.filter_cache.as_ref().is_none_or(|c| c.0 != key) {
            let idx = (0..self.log.entries.len())
                .filter(|&i| hit(self.log.entries[i].tool, &self.log.entries[i].text))
                .collect();
            self.filter_cache = Some((key, idx));
        }
        let shown: Vec<(usize, &log::Entry)> = self
            .filter_cache
            .as_ref()
            .map(|c| c.1.iter().map(|&i| (i, &self.log.entries[i])).collect())
            .unwrap_or_default();
        let running: Vec<_> = running.iter().filter(|r| hit(r.0, &r.2)).collect();
        let filtered = !needle.is_empty();
        let mut clear = false;
        ui.horizontal(|ui| {
            widgets::lead(ui, "Filter");
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .char_limit(widgets::FIELD_MAX)
                    .desired_width(240.0)
                    .hint_text("tool or text, e.g. FAIL, 443, timeout"),
            )
            .on_hover_text("Show runs containing this. Copy, Save and Export follow it");
            if filtered && ui.small_button("×").on_hover_text("Clear filter").clicked() {
                self.filter.clear();
            }
        });
        ui.separator();
        widgets::status_row(
            ui,
            |ui| {
                ui.label(format!(
                    "{}{} logged run(s){}",
                    if filtered {
                        format!("{} of ", shown.len())
                    } else {
                        String::new()
                    },
                    self.log.entries.len(),
                    if running.is_empty() {
                        String::new()
                    } else {
                        format!(", {} running", running.len())
                    }
                ));
                if self.log.dropped > 0 {
                    ui.weak(format!("({} oldest dropped)", self.log.dropped));
                }
            },
            |ui| {
                ui.add_enabled_ui(!shown.is_empty(), |ui| {
                    clear =
                        widgets::confirm_button(ui, "log_clear", true, "Clear", "Clear the log");
                    let render = || {
                        if filtered {
                            shown.iter().map(|(_, e)| e.render() + "\n").collect()
                        } else {
                            self.log.render()
                        }
                    };
                    widgets::export_menu(ui, "log", || shown.iter().map(|(_, e)| *e).collect());
                    widgets::save_button(ui, "log", render);
                    widgets::copy_button(ui, render);
                });
            },
        );
        if self.log.entries.is_empty() && running.is_empty() {
            widgets::empty_state(
                ui,
                "No results yet",
                "Finished runs from every tab are logged here with start and end times, to copy, save or export together.",
                &[],
            );
        } else if shown.is_empty() && running.is_empty() {
            widgets::empty_hint(ui, "No runs match the filter.");
        } else {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    // The newest runs open, older ones folded: a long log
                    // stays quick to draw.
                    let open_from = shown.len().saturating_sub(12);
                    for (k, (i, e)) in shown.iter().enumerate() {
                        let open = k >= open_from;
                        entry_ui(ui, *i, &e.header(), &e.text, Some(e), open, || e.render());
                    }
                    // Ids from the top of the range so they never meet log indices.
                    for (j, (tool, since, text)) in running.iter().enumerate() {
                        let header = format!("{tool}  started {since}, running…");
                        entry_ui(ui, usize::MAX - j, &header, text, None, true, || {
                            format!("=== {header} ===\n{}\n", text.trim_end())
                        });
                    }
                    if jump {
                        ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                    }
                });
        }
        if clear {
            self.log.clear();
        }
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

/// `entry` is the logged run, for Export; None while it's still running.
fn entry_ui(
    ui: &mut egui::Ui,
    id: usize,
    header: &str,
    text: &str,
    entry: Option<&log::Entry>,
    open: bool,
    full: impl Fn() -> String,
) {
    egui::CollapsingHeader::new(egui::RichText::new(header).strong())
        .id_salt(("log", id))
        .default_open(open)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::copy_button(ui, &full);
                widgets::save_button(ui, "result", &full);
                if let Some(e) = entry {
                    widgets::export_menu(ui, "result", || vec![e]);
                }
            });
            ui.add(
                egui::Label::new(egui::RichText::new(text.trim_end()).monospace()).selectable(true),
            );
        });
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

fn main() -> eframe::Result {
    #[cfg(target_os = "linux")]
    close_inherited_fds_on_exec();
    let look = theme::load();
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
            .with_drag_and_drop(true)
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native(
        "is-alive",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx, look.theme);
            widgets::init(&cc.egui_ctx);
            net::clock::warm();
            let mut app = App {
                tab: Tab::Ping,
                shown_tab: Tab::Ping,
                tab_since: f64::NEG_INFINITY,
                size: None,
                ping: Default::default(),
                ports: Default::default(),
                trace: Default::default(),
                speed: Default::default(),
                sweep: Default::default(),
                calc: Default::default(),
                dns: Default::default(),
                advanced: Default::default(),
                export: Default::default(),
                bundle: Default::default(),
                map: map::view::MapView::new(look.map_layout),
                notes: Default::default(),
                log: Default::default(),
                info: HostInfo::local(),
                info_job: None,
                wan_asked: false,
                was_sweeping: false,
                filter: String::new(),
                filter_cache: None,
                jump_to_new: false,
                look,
                backdrop: backdrop::Backdrop::new(cc.gl.as_ref()),
                pick_image: None,
                reloading: false,
                shots: shots::Shots::from_env(),
                shots_settings: false,
            };
            if app.look.wan_on_start && app.shots.is_none() {
                app.refresh_info(&cc.egui_ctx);
            }
            // Files named on the command line ("Open with") go to Config Export.
            let files: Vec<std::path::PathBuf> =
                std::env::args_os().skip(1).map(Into::into).collect();
            if !files.is_empty() {
                app.tab = Tab::Export;
                app.export.open(&cc.egui_ctx, files);
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
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

    /// Every character the app writes must exist in egui's bundled fonts,
    /// for both proportional and monospace text, or it shows as a box.
    #[test]
    fn every_glyph_in_the_source_renders() {
        use eframe::egui::{FontDefinitions, FontFamily};
        let defs = FontDefinitions::default();
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
