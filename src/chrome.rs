//! The window's frame around the tabs: the header (the app's name, which
//! opens Settings, the tabs and Site Bundle), the footer (this machine's
//! addresses, Full scan, Refresh), and Settings itself.

use crate::job::Job;
use crate::{App, FULL_SCAN_TIP, TABS, VERSION, tab_key, theme, widgets};
use eframe::egui;

impl App {
    /// The tab bar.
    pub(crate) fn header(&mut self, ui: &mut egui::Ui) {
        let tabs = egui::Panel::top("tabs").show(ui, |ui| {
            ui.add_space(4.0);
            // Wraps to a second row when the window (or display scaling)
            // leaves too little width, so every tab stays reachable.
            ui.horizontal_wrapped(|ui| {
                // The app's name, drawn as a title; it opens Settings.
                let name = ui
                    .add(
                        egui::Button::new(egui::RichText::new("is-alive").heading().strong())
                            .frame(false),
                    )
                    .on_hover_text(format!("Settings · {} theme", self.look.theme.name()));
                if self.shots_settings {
                    egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&name));
                }
                egui::Popup::menu(&name)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| self.settings_ui(ui));
                ui.separator();
                // The selected tab is marked by a sliding underline rather
                // than a filled box; running jobs get a pulsing dot that
                // doesn't change the tab's width.
                let fill = ui.visuals().selection.bg_fill;
                ui.visuals_mut().selection.bg_fill = egui::Color32::TRANSPARENT;
                // Tabs sit closer than other controls: twelve of them and
                // Site Bundle fit one row at the default width in any theme.
                let gap = ui.spacing().item_spacing.x;
                ui.spacing_mut().item_spacing.x = 4.0;
                let mut selected = None;
                let mut dots = Vec::new();
                for (n, (tab, name, tip)) in TABS.iter().enumerate() {
                    let on = self.tab == *tab;
                    let mut text = egui::RichText::new(*name);
                    if on {
                        text = text.color(theme::accent());
                    }
                    let r = ui
                        .selectable_label(on, text)
                        .on_hover_text(match tab_key(n) {
                            Some(d) => format!("{tip}\n\nCtrl+{d}"),
                            None => tip.to_string(),
                        });
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
                ui.spacing_mut().item_spacing.x = gap;
                if let Some(r) = selected {
                    tab_underline(ui, r);
                }
                let bundle = |ui: &mut egui::Ui| {
                    ui.button("Site Bundle…")
                        .on_hover_text("Zip everything for /unifidump and /mapnetwork")
                };
                // Right-aligned when the row has room; otherwise it wraps with the tabs.
                let need = egui::WidgetText::from("Site Bundle…")
                    .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Button)
                    .size()
                    .x
                    + 2.0 * ui.spacing().button_padding.x
                    + ui.spacing().item_spacing.x
                    + 8.0;
                let button = if ui.available_size_before_wrap().x > need {
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
    }

    /// LAN, gateway and WAN on the left; Full scan, Refresh and the version
    /// on the right.
    pub(crate) fn footer(&mut self, ui: &mut egui::Ui) {
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
                let wan = match (i.wan, self.info_job.is_some(), self.wan_asked) {
                    (Some(w), ..) => w.to_string(),
                    (None, true, _) => "looking up…".into(),
                    (None, false, true) => na(),
                    (None, false, false) => "not looked up".into(),
                };
                ui.label(format!("WAN {wan}")).on_hover_text(
                    "Public IP, looked up via OpenDNS on start (unless Settings turns it off), \
                         on Refresh or a full scan",
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(crate::update::Found::Newer(r)) = &self.update {
                        ui.hyperlink_to(
                            egui::RichText::new(format!("Update: v{}", r.version))
                                .color(widgets::ok())
                                .small(),
                            &r.url,
                        )
                        .on_hover_text(format!(
                            "Version {} is out (this is v{VERSION}). Opens its release page; \
                             nothing is downloaded.",
                            r.version
                        ));
                        ui.separator();
                    }
                    if ui
                        .add_enabled(
                            self.info_job.is_none(),
                            egui::Button::new("Refresh").small(),
                        )
                        .on_hover_text("Re-read LAN and gateway, look up the public IP")
                        .clicked()
                    {
                        self.refresh_info(ui.ctx());
                    }
                    let busy = self.any_running();
                    let tip = if busy {
                        "Wait for running tools to finish (or stop them) to run a full scan."
                    } else {
                        FULL_SCAN_TIP
                    };
                    if ui
                        .add_enabled(!busy, egui::Button::new("Full scan").small())
                        .on_hover_text(tip)
                        .on_disabled_hover_text(tip)
                        .clicked()
                    {
                        self.full_scan(ui.ctx());
                    }
                    ui.separator();
                    ui.weak(format!("v{VERSION}"));
                    if self.look.theme == theme::Theme::Miami
                        && let Some(e) = &self.backdrop.err
                    {
                        ui.label(egui::RichText::new("⚠ backdrop").color(widgets::warn()))
                            .on_hover_text(e.as_str());
                    }
                });
            });
        });
    }

    /// Settings, opened from the app's name: appearance, the Map, keyboard
    /// shortcuts and where the app keeps its files.
    pub(crate) fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(320.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("is-alive").heading().strong());
            ui.weak(format!("v{VERSION}"));
        });
        ui.separator();
        match theme::appearance(
            ui,
            &mut self.look,
            &self.backdrop.renderer,
            self.backdrop.on_gpu(),
        ) {
            Some(theme::MenuAction::Changed) => {
                theme::apply(ui.ctx(), self.look.theme);
                theme::save(&self.look);
            }
            Some(theme::MenuAction::PickImage) if self.pick_image.is_none() => {
                self.pick_image = Some(Job::spawn(ui.ctx(), |tx| {
                    tx.send(
                        rfd::FileDialog::new()
                            .set_title("Miami backdrop")
                            .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
                            .pick_file(),
                    )
                }));
            }
            _ => {}
        }
        ui.separator();
        ui.label(egui::RichText::new("Network").strong());
        if ui
            .checkbox(
                &mut self.look.wan_on_start,
                "Look up the public IP on start",
            )
            .on_hover_text(
                "One DNS query to OpenDNS when the app opens, for the WAN address in the \
                 footer. Off, nothing leaves this machine until you run something (Refresh \
                 or a full scan look it up).",
            )
            .changed()
        {
            theme::save(&self.look);
        }
        ui.horizontal(|ui| {
            if ui
                .checkbox(
                    &mut self.look.updates_on_start,
                    "Check for updates on start",
                )
                .on_hover_text(
                    "Asks GitHub for the latest release when the app opens (one HTTPS \
                     request). A newer one shows in the footer with a link; nothing is \
                     downloaded.",
                )
                .changed()
            {
                theme::save(&self.look);
            }
            if ui
                .add_enabled(
                    self.update_job.is_none(),
                    egui::Button::new("Check now").small(),
                )
                .clicked()
            {
                self.check_updates(ui.ctx(), true);
            }
        });
        match (&self.update, self.update_job.is_some()) {
            (_, true) => {
                ui.weak("Checking…");
            }
            (Some(crate::update::Found::Newer(r)), _) => {
                ui.hyperlink_to(
                    format!("Version {} is out: open the release page", r.version),
                    &r.url,
                );
            }
            (Some(crate::update::Found::UpToDate), _) => {
                ui.weak(format!("Up to date (v{VERSION})"));
            }
            (Some(crate::update::Found::Failed(e)), _) => {
                ui.weak(format!("Couldn't check: {e}"));
            }
            (None, false) => {}
        }
        ui.separator();
        ui.label(egui::RichText::new("Map").strong());
        if ui
            .checkbox(&mut self.look.map_note, "Show the accuracy note")
            .on_hover_text("The note over the Map on how to get an accurate one")
            .changed()
        {
            theme::save(&self.look);
        }
        ui.separator();
        ui.collapsing("Keyboard and mouse", shortcuts);
        ui.separator();
        if let Some(dir) = theme::data_dir() {
            ui.weak("Settings and autosaved notes are kept in");
            ui.label(
                egui::RichText::new(dir.display().to_string())
                    .monospace()
                    .small(),
            );
        }
    }
}

/// Keyboard and mouse tips, in Settings.
fn shortcuts(ui: &mut egui::Ui) {
    ui.label(egui::RichText::new("Keyboard").strong());
    egui::Grid::new("keys")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            for (k, what) in [
                ("Ctrl+1 … Ctrl+9", "Switch to the first nine tabs, in order"),
                ("Ctrl+0", "Results"),
                ("Enter / Esc", "Start / stop the run in a tool's field"),
                ("Ctrl+B / Ctrl+I", "Bold / italic in Notes"),
                ("Ctrl+R", "Redact / unredact in Notes"),
                (
                    "Enter in a list",
                    "Next item in Notes (twice ends the list)",
                ),
                ("Esc", "Clear the selection on the Map"),
            ] {
                ui.label(egui::RichText::new(k).monospace());
                ui.label(what);
                ui.end_row();
            }
        });
    ui.separator();
    ui.label(egui::RichText::new("Mouse").strong());
    for tip in [
        "Map: drag to pan, scroll to zoom, double-click to fit, drag a card to move it",
        "Drop a UniFi backup, SonicWall .exp / .wri or /mapnetwork map anywhere to open it",
        "Full scan (bottom right) pings, traces and sweeps this network in one go",
        "The Map is a quick picture: for an accurate one, make a Site Bundle and run /mapnetwork in Claude",
    ] {
        ui.label(format!("• {tip}"));
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
