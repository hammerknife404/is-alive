#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backdrop;
mod bundle;
mod job;
mod log;
mod net;
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
const FULL_SCAN_TIP: &str = "Full scan: pings your gateway, checks its common ports, traces the route to the \
internet, tests DNS and sweeps your LAN, all at once. Opens Results to watch them run.";

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Ping,
    Ports,
    Trace,
    Sweep,
    Dns,
    Advanced,
    Export,
    Results,
}

/// (tab, label, hover text).
const TABS: &[(Tab, &str, &str)] = &[
    (
        Tab::Ping,
        "Ping",
        "ICMP or TCP ping, continuous or a set count, with a live latency graph and loss/min/avg/max.",
    ),
    (
        Tab::Ports,
        "Port Check",
        "Check which TCP ports are open on a host: lists, ranges or presets, with service names.",
    ),
    (
        Tab::Trace,
        "Traceroute",
        "Show each router hop on the way to a host, 3 probes per hop.",
    ),
    (
        Tab::Sweep,
        "Subnet Sweep",
        "Find live hosts on a subnet or range, with name, MAC and vendor.",
    ),
    (
        Tab::Dns,
        "DNS Lookup",
        "Look up any record type, or all of them, against the system resolver or a public one; \
Compare all spots split DNS and stale caches.",
    ),
    (
        Tab::Advanced,
        "Advanced",
        "Mail and domain health for a domain: MX, SPF, DMARC, DKIM, nameservers, SMTP, blacklists and more.",
    ),
    (
        Tab::Export,
        "Config Export",
        "Decode a UniFi backup (.unf / .unifi) or SonicWall settings export (.exp) on this machine \
into sanitized JSON for /unifidump and /mapnetwork. Secrets are left out.",
    ),
    (
        Tab::Results,
        "Results",
        "Every finished run with start and end times. Filter, copy, save, or export as CSV, JSON, \
Markdown or HTML.",
    ),
];

struct App {
    tab: Tab,
    ping: tools::ping::PingTool,
    ports: tools::ports::PortTool,
    trace: tools::trace::TraceTool,
    sweep: tools::sweep::SweepTool,
    dns: tools::dns::DnsTool,
    advanced: tools::advanced::AdvancedTool,
    export: tools::export::ExportTool,
    bundle: tools::bundle::BundleTool,
    log: log::Log,
    info: HostInfo,
    info_job: Option<Job<HostInfo>>,
    /// Results tab filter; matches tool name or output, case-insensitive.
    filter: String,
    /// Scroll Results to its newest runs on the next frame (after a full scan).
    jump_to_new: bool,
    look: theme::Settings,
    backdrop: backdrop::Backdrop,
    /// File dialog for the Miami Vice backdrop image.
    pick_image: Option<Job<Option<std::path::PathBuf>>>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ping.poll();
        self.ports.poll();
        self.trace.poll();
        self.sweep.poll();
        self.dns.poll();
        self.advanced.poll();
        self.export.poll();
        self.collect();
        self.dropped_files(ui.ctx());
        pump(&mut self.info_job, |i| self.info = i);
        let mut picked = None;
        pump(&mut self.pick_image, |p| picked = p);
        if let Some(p) = picked {
            self.look.backdrop = backdrop::Source::Image(p);
            theme::save(&self.look);
        }
        // Under every panel; Miami Vice's panels are translucent over it.
        if self.look.theme == theme::Theme::Miami {
            self.backdrop
                .paint(ui.ctx(), &self.look.backdrop, self.look.animate);
        }
        let tabs = egui::Panel::top("tabs").show(ui, |ui| {
            ui.add_space(4.0);
            // Wraps to a second row when the window (or display scaling)
            // leaves too little width, so every tab stays reachable.
            ui.horizontal_wrapped(|ui| {
                let busy = self.any_running();
                let tip = if busy {
                    "Wait for running tools to finish (or stop them) to run a full scan."
                } else {
                    FULL_SCAN_TIP
                };
                let b = ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(egui::RichText::new("is-alive").heading()),
                    )
                    .on_hover_text(tip)
                    .on_disabled_hover_text(tip);
                if b.clicked() {
                    self.full_scan(ui.ctx());
                }
                ui.separator();
                // The selected tab is marked by a sliding underline rather
                // than a filled box; running jobs get a pulsing dot that
                // doesn't change the tab's width.
                let fill = ui.visuals().selection.bg_fill;
                ui.visuals_mut().selection.bg_fill = egui::Color32::TRANSPARENT;
                let mut selected = None;
                let mut dots = Vec::new();
                for (tab, name, tip) in TABS {
                    let on = self.tab == *tab;
                    let mut text = egui::RichText::new(*name);
                    if on {
                        text = text.color(theme::accent());
                    }
                    let r = ui.selectable_label(on, text).on_hover_text(*tip);
                    if r.clicked() {
                        self.tab = *tab;
                    }
                    if on {
                        selected = Some(r.rect);
                    }
                    if self.tab_running(*tab) {
                        dots.push(r.rect);
                    }
                }
                ui.visuals_mut().selection.bg_fill = fill;
                if let Some(r) = selected {
                    tab_underline(ui, r);
                }
                let bundle = |ui: &mut egui::Ui| {
                    ui.button("Site Bundle…").on_hover_text(
                        "Wrap the UniFi and SonicWall exports, the sweep inventory and \
(optionally) the Results log into one zip with a manifest for /unifidump and /mapnetwork.",
                    )
                };
                // Right-aligned when the row has room; otherwise it wraps with the tabs.
                let need = egui::WidgetText::from("Site Bundle…")
                    .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Button)
                    .size()
                    .x
                    + 2.0 * ui.spacing().button_padding.x
                    + ui.spacing().item_spacing.x
                    + 8.0;
                let button = if ui.available_width() > need {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), bundle)
                        .inner
                } else {
                    bundle(ui)
                };
                if button.clicked() {
                    self.bundle.open = !self.bundle.open;
                }
                if self.bundle.is_running() {
                    dots.push(button.rect);
                }
                running_dots(ui, &dots);
            });
            ui.add_space(2.0);
        });
        theme::top_rule(ui.ctx(), tabs.response.rect.bottom(), self.look.animate);
        egui::Panel::bottom("footer").show(ui, |ui| {
            ui.horizontal(|ui| {
                let i = &self.info;
                let na = || "unknown".to_string();
                ui.label(format!(
                    "LAN {}",
                    i.lan.map_or_else(na, |(ip, bits)| format!("{ip}/{bits}"))
                ));
                ui.separator();
                ui.label(format!(
                    "Gateway {}",
                    i.gateway.map_or_else(na, |g| g.to_string())
                ));
                ui.separator();
                let wan = match (i.wan, self.info_job.is_some()) {
                    (Some(w), _) => w.to_string(),
                    (None, true) => "looking up…".into(),
                    (None, false) => na(),
                };
                ui.label(format!("WAN {wan}"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            self.info_job.is_none(),
                            egui::Button::new("Refresh").small(),
                        )
                        .on_hover_text("Re-read LAN, gateway and public IP")
                        .clicked()
                    {
                        self.refresh_info(ui.ctx());
                    }
                    ui.weak(format!("v{VERSION}"));
                    ui.separator();
                    match theme::menu(ui, &mut self.look) {
                        Some(theme::MenuAction::Changed) => {
                            theme::apply(ui.ctx(), self.look.theme);
                            theme::save(&self.look);
                        }
                        Some(theme::MenuAction::PickImage) if self.pick_image.is_none() => {
                            self.pick_image = Some(Job::spawn(ui.ctx(), |tx| {
                                tx.send(
                                    rfd::FileDialog::new()
                                        .set_title("Miami Vice backdrop")
                                        .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
                                        .pick_file(),
                                )
                            }));
                        }
                        _ => {}
                    }
                    if self.look.theme == theme::Theme::Miami
                        && let Some(e) = &self.backdrop.err
                    {
                        ui.label(egui::RichText::new("⚠ backdrop").color(widgets::warn()))
                            .on_hover_text(e.as_str());
                    }
                });
            });
        });
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Ping => self.ping.ui(ui),
            Tab::Ports => self.ports.ui(ui),
            Tab::Trace => self.trace.ui(ui),
            Tab::Sweep => self.sweep.ui(ui, &self.info),
            Tab::Dns => self.dns.ui(ui),
            Tab::Advanced => self.advanced.ui(ui),
            Tab::Export => self.export.ui(ui),
            Tab::Results => self.results_ui(ui),
        });
        let App {
            bundle,
            export,
            sweep,
            log,
            info,
            ..
        } = self;
        bundle.window(
            ui.ctx(),
            tools::bundle::Sources {
                export,
                sweep,
                log,
                info,
            },
        );
    }
}

impl App {
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
                "Drop a UniFi backup or SonicWall export to convert it",
                egui::FontId::proportional(18.0),
                egui::Color32::WHITE.gamma_multiply(fade),
            );
        }
    }

    fn refresh_info(&mut self, ctx: &egui::Context) {
        self.info_job = Some(Job::spawn(ctx, |tx| tx.send(HostInfo::fetch())));
    }

    fn tab_running(&self, tab: Tab) -> bool {
        match tab {
            Tab::Ping => self.ping.is_running(),
            Tab::Ports => self.ports.is_running(),
            Tab::Trace => self.trace.is_running(),
            Tab::Sweep => self.sweep.is_running(),
            Tab::Dns => self.dns.is_running(),
            Tab::Advanced => self.advanced.is_running(),
            Tab::Export => self.export.is_running(),
            Tab::Results => false,
        }
    }

    fn any_running(&self) -> bool {
        self.ping.is_running()
            || self.ports.is_running()
            || self.trace.is_running()
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
        let shown: Vec<(usize, &log::Entry)> = self
            .log
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| hit(e.tool, &e.text))
            .collect();
        let running: Vec<_> = running.iter().filter(|r| hit(r.0, &r.2)).collect();
        let filtered = !needle.is_empty();
        let mut clear = false;
        ui.horizontal(|ui| {
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
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!shown.is_empty(), |ui| {
                    if ui.button("Clear").on_hover_text("Clear the log").clicked() {
                        clear = true;
                    }
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
            });
        });
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .desired_width(240.0)
                    .hint_text("tool or text, e.g. FAIL, 443, timeout"),
            )
            .on_hover_text("Show only runs whose tool or output contains this. Copy, Save and Export use the filtered set.");
            if filtered && ui.small_button("✕").on_hover_text("Clear filter").clicked() {
                self.filter.clear();
            }
        });
        ui.separator();
        if self.log.entries.is_empty() && running.is_empty() {
            ui.weak("No results yet. Finished runs from every tab are logged here with start/end times.");
        } else if shown.is_empty() && running.is_empty() {
            ui.weak("No runs match the filter.");
        } else {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (i, e) in &shown {
                        entry_ui(ui, *i, &e.header(), &e.text, Some(e), || e.render());
                    }
                    // Ids from the top of the range so they never meet log indices.
                    for (j, (tool, since, text)) in running.iter().enumerate() {
                        let header = format!("{tool}  started {since}, running…");
                        entry_ui(ui, usize::MAX - j, &header, text, None, || {
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

/// Underline under the selected tab, sliding to the new one on a switch.
fn tab_underline(ui: &egui::Ui, r: egui::Rect) {
    let ctx = ui.ctx();
    let ease =
        |k: &str, v: f32| ctx.animate_value_with_time(egui::Id::new(("tab_line", k)), v, 0.16);
    let (x0, x1, y) = (
        ease("x0", r.left() + 4.0),
        ease("x1", r.right() - 4.0),
        ease("y", r.bottom() + 1.0),
    );
    theme::glow(ui.painter(), x0..=x1, y);
    ui.painter()
        .hline(x0..=x1, y, egui::Stroke::new(2.0, theme::accent()));
}

/// Pulsing dot in the top-right corner of each tab whose job is running.
fn running_dots(ui: &egui::Ui, rects: &[egui::Rect]) {
    if rects.is_empty() {
        return;
    }
    // Slow pulse at 10 fps, held still while the window isn't focused: a
    // continuous ping can run for hours, and each frame redraws the window.
    let focused = ui.input(|i| i.viewport().focused != Some(false));
    let t = ui.input(|i| i.time) as f32;
    let a = if focused {
        0.35 + 0.65 * (0.5 + 0.5 * (t * 3.0).sin())
    } else {
        1.0
    };
    for r in rects {
        ui.painter().circle_filled(
            egui::pos2(r.right() - 2.0, r.top() + 3.0),
            2.5,
            widgets::ok().gamma_multiply(a),
        );
    }
    if focused {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// `entry` is the logged run, for Export; None while it's still running.
fn entry_ui(
    ui: &mut egui::Ui,
    id: usize,
    header: &str,
    text: &str,
    entry: Option<&log::Entry>,
    full: impl Fn() -> String,
) {
    egui::CollapsingHeader::new(egui::RichText::new(header).strong())
        .id_salt(("log", id))
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::save_button(ui, "result", &full);
                widgets::copy_button(ui, &full);
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

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("is-alive {VERSION} — network tester"))
            .with_app_id("is-alive")
            .with_inner_size([980.0, 660.0])
            .with_min_inner_size([620.0, 400.0])
            .with_drag_and_drop(true)
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native(
        "is-alive",
        options,
        Box::new(|cc| {
            let look = theme::load();
            theme::apply(&cc.egui_ctx, look.theme);
            let mut app = App {
                tab: Tab::Ping,
                ping: Default::default(),
                ports: Default::default(),
                trace: Default::default(),
                sweep: Default::default(),
                dns: Default::default(),
                advanced: Default::default(),
                export: Default::default(),
                bundle: Default::default(),
                log: Default::default(),
                info: HostInfo::local(),
                info_job: None,
                filter: String::new(),
                jump_to_new: false,
                look,
                backdrop: Default::default(),
                pick_image: None,
            };
            app.refresh_info(&cc.egui_ctx);
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
}
