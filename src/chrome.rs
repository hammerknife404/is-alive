//! The window's frame around the tabs: the header (the app's name, which
//! opens Settings, the tabs and Site Bundle), the footer (this machine's
//! addresses, Full scan, Refresh), and Settings itself.

use crate::job::Job;
use crate::{App, FULL_SCAN_TIP, TABS, Tab, VERSION, tab_key, theme, widgets};
use eframe::egui;

/// The wiki's front page: Help and F1 open it.
pub const WIKI: &str = "https://github.com/hammerknife404/is-alive/wiki";

impl App {
    /// The tab bar.
    pub(crate) fn header(&mut self, ui: &mut egui::Ui) {
        let tabs = egui::Panel::top("tabs").show(ui, |ui| {
            ui.add_space(8.0);
            // Wraps to a second row when the window (or display scaling)
            // leaves too little width, so every tab stays reachable.
            ui.horizontal_wrapped(|ui| {
                // The app's name, drawn as a title; it opens Settings.
                let name = ui
                    .add(
                        egui::Button::new(egui::RichText::new("is-alive ▾").heading().strong())
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
                let call = self.map_call(ui);
                for (n, (tab, name, tip)) in TABS.iter().enumerate() {
                    if matches!(tab, Tab::Dns | Tab::Export | Tab::Notes | Tab::Results) {
                        ui.separator();
                    }
                    let on = self.tab == *tab;
                    let mut text = egui::RichText::new(*name);
                    if on {
                        text = text.color(theme::accent());
                    }
                    // Behind the Map tab's label, for its glow after a full scan.
                    let behind = (*tab == Tab::Map).then(|| ui.painter().add(egui::Shape::Noop));
                    let glow = match call {
                        Call::Glow { glow, .. } if *tab == Tab::Map => glow,
                        _ => 0.0,
                    };
                    if glow > 0.0 {
                        let plain = ui.visuals().text_color();
                        text = text.color(plain.lerp_to_gamma(theme::accent(), glow));
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
                    if let Some(at) = behind {
                        map_glow(ui, at, r.rect, glow);
                        if call != Call::Off {
                            map_ring(ui, r.rect);
                        }
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

    /// How the Map tab calls for attention this frame, asking for the
    /// frames its pulse needs. While the window isn't focused the pulse
    /// waits, so it plays when the user comes back.
    fn map_call(&mut self, ui: &egui::Ui) -> Call {
        let (now, focused) = ui.input(|i| (i.time, i.viewport().focused != Some(false)));
        let animate = self.look.animate;
        let call = self.map_call.frame(now, focused, animate);
        if let Call::Glow { left, .. } = call
            && focused
        {
            if animate {
                ui.ctx().request_repaint();
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs_f64(left));
            }
        }
        call
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
                    ui.separator();
                    if ui
                        .add(egui::Button::new("Help").small())
                        .on_hover_text("Open the is-alive wiki in the browser (F1)")
                        .clicked()
                    {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(WIKI));
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
    }

    /// Settings, opened from the app's name: appearance, the Map, keyboard
    /// shortcuts and where the app keeps its files.
    pub(crate) fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(320.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("is-alive").heading().strong());
            ui.weak(format!("v{VERSION}"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(framed(ui, egui::Button::new("Help wiki")))
                    .on_hover_text("Open the is-alive wiki in the browser: a page per tab (F1)")
                    .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(WIKI));
                }
            });
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
                    framed(ui, egui::Button::new("Check now").small()),
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
/// A button inside a menu drawn as the app's other buttons are: menus make
/// buttons frameless and see-through, so a real button there reads as text.
fn framed<'a>(ui: &egui::Ui, b: egui::Button<'a>) -> egui::Button<'a> {
    let fill = ui
        .ctx()
        .global_style()
        .visuals
        .widgets
        .inactive
        .weak_bg_fill;
    b.frame(true).fill(fill)
}

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
                ("F1", "Open the is-alive wiki (Help)"),
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

/// Seconds the Map tab pulses for after a full scan (three slow pulses);
/// with Animate off it lights up, still, for a moment instead.
const PULSE_SECS: f64 = 3.6;
const PULSES: f64 = 3.0;
const STILL_SECS: f64 = 1.5;

/// The Map tab calling for attention once a full scan is done: it glows in
/// the accent colour for a few seconds, then keeps a ring until it's opened.
#[derive(Default)]
pub(crate) struct MapCall {
    since: Option<f64>,
    /// The glow is waiting for the window to be focused again.
    held: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Call {
    Off,
    /// `glow` 0–1 now, for `left` seconds more.
    Glow {
        glow: f32,
        left: f64,
    },
    Ring,
}

impl MapCall {
    /// A full scan finished at `now`; nothing to show if the Map is open.
    pub(crate) fn start(&mut self, now: f64, on_map: bool) {
        self.since = (!on_map).then_some(now);
        self.held = false;
    }

    /// The Map tab is open (or another scan began): the call is answered.
    pub(crate) fn seen(&mut self) {
        self.since = None;
        self.held = false;
    }

    /// This frame's call. While the window isn't focused the glow waits,
    /// however long that is, and plays from the top on the first frame back.
    fn frame(&mut self, now: f64, focused: bool, animate: bool) -> Call {
        if let Some(s) = &mut self.since
            && (self.held || !focused && now - *s < glow_secs(animate))
        {
            *s = now;
            self.held = !focused;
        }
        self.call(now, animate)
    }

    fn call(&self, now: f64, animate: bool) -> Call {
        let Some(since) = self.since else {
            return Call::Off;
        };
        let t = (now - since).max(0.0);
        let left = glow_secs(animate) - t;
        if left <= 0.0 {
            return Call::Ring;
        }
        let glow = if animate {
            let turns = t / PULSE_SECS * PULSES;
            (0.5 - 0.5 * (turns * std::f64::consts::TAU).cos()) as f32
        } else {
            1.0
        };
        Call::Glow { glow, left }
    }
}

fn glow_secs(animate: bool) -> f64 {
    if animate { PULSE_SECS } else { STILL_SECS }
}

/// A soft accent fill behind the Map tab, `glow` 0–1, in the slot `at`
/// reserved under its label.
fn map_glow(ui: &egui::Ui, at: egui::layers::ShapeIdx, r: egui::Rect, glow: f32) {
    if glow <= 0.0 {
        return;
    }
    let accent = theme::accent();
    ui.painter().set(
        at,
        egui::epaint::RectShape::new(
            r.expand2(egui::vec2(2.0, 1.0)),
            4.0,
            accent.gamma_multiply(0.3 * glow),
            egui::Stroke::new(1.0, accent.gamma_multiply(0.7 * glow)),
            egui::StrokeKind::Inside,
        ),
    );
}

/// A small ring in the Map tab's top-right corner until it's opened: hollow,
/// so it isn't read as a running job's dot.
fn map_ring(ui: &egui::Ui, r: egui::Rect) {
    ui.painter().circle_stroke(
        egui::pos2(r.right() - 2.5, r.top() + 3.5),
        2.75,
        egui::Stroke::new(1.5, theme::accent()),
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_tab_glows_then_keeps_a_ring_until_opened() {
        let mut m = MapCall::default();
        assert_eq!(m.call(0.0, true), Call::Off);
        m.start(10.0, false);
        // Three pulses: dark at the start and between, bright in between.
        let glow = |m: &MapCall, t: f64| match m.call(10.0 + t, true) {
            Call::Glow { glow, .. } => glow,
            c => panic!("{c:?} at {t}"),
        };
        assert!(glow(&m, 0.0) < 0.01);
        assert!(glow(&m, 0.6) > 0.99);
        assert!(glow(&m, 1.2) < 0.01);
        assert!(glow(&m, 3.0) > 0.99);
        assert_eq!(m.call(10.01 + PULSE_SECS, true), Call::Ring);
        assert_eq!(m.call(500.0, true), Call::Ring);
        m.seen();
        assert_eq!(m.call(500.0, true), Call::Off);
    }

    #[test]
    fn map_tab_still_and_short_without_animation() {
        let mut m = MapCall::default();
        m.start(0.0, false);
        assert!(matches!(m.call(0.2, false), Call::Glow { glow: 1.0, .. }));
        assert_eq!(m.call(STILL_SECS + 0.01, false), Call::Ring);
    }

    #[test]
    fn map_tab_open_or_unwatched() {
        let mut m = MapCall::default();
        // Already on the Map: nothing to point at.
        m.start(0.0, true);
        assert_eq!(m.call(0.5, true), Call::Off);
        // Window in the background: the glow waits for the user.
        m.start(0.0, false);
        m.frame(0.0, false, true);
        m.frame(3.0, false, true);
        assert!(matches!(m.frame(3.6, true, true), Call::Glow { .. }));
        assert!(matches!(m.frame(4.2, true, true), Call::Glow { glow, .. } if glow > 0.99));
        // Once it's played, the ring just stays.
        m.frame(40.0, false, true);
        assert_eq!(m.frame(40.0, true, true), Call::Ring);
    }

    #[test]
    fn map_tab_glows_when_the_user_is_back_however_long_away() {
        // Nothing draws while the window idles in the background: one frame
        // as the scan ends, the next when the user comes back.
        for animate in [true, false] {
            let mut m = MapCall::default();
            m.start(100.0, false);
            m.frame(100.0, false, animate);
            assert!(matches!(m.frame(160.0, true, animate), Call::Glow { .. }));
            assert!(matches!(m.frame(161.0, true, animate), Call::Glow { .. }));
            assert_eq!(m.frame(170.0, true, animate), Call::Ring);
        }
        // Back after the call had played: just the ring.
        let mut m = MapCall::default();
        m.start(0.0, false);
        m.frame(0.0, true, true);
        m.frame(10.0, false, true);
        assert_eq!(m.frame(60.0, true, true), Call::Ring);
    }
}
