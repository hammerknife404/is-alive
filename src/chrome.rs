//! The window's frame around the tabs: the title bar (the app's name, the
//! site switcher, Report and Full scan), the status bar (this machine's
//! addresses, Refresh, what's running) and Settings. The nav is in nav.rs.

use crate::job::Job;
use crate::{App, FULL_SCAN_TIP, TABS, VERSION, theme, widgets};
use eframe::egui;

impl App {
    /// The title bar: the app's name and the site switcher on the left,
    /// Report and Full scan on the right.
    pub(crate) fn title_bar(&mut self, ui: &mut egui::Ui) {
        let bar = egui::Panel::top("title").exact_size(40.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new("is-alive").heading().strong());
                ui.add_space(6.0);
                self.site_chip(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    let busy = self.any_running();
                    let tip = if busy {
                        "Wait for running tools to finish (or stop them) to run a full scan."
                    } else {
                        FULL_SCAN_TIP
                    };
                    let accent = theme::accent();
                    if ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                egui::RichText::new("Full scan").strong().color(on(accent)),
                            )
                            .fill(accent)
                            .min_size(egui::vec2(0.0, 28.0)),
                        )
                        .on_hover_text(tip)
                        .on_disabled_hover_text(tip)
                        .clicked()
                    {
                        self.full_scan(ui.ctx());
                    }
                    if ui
                        .add(
                            egui::Button::new("Report…")
                                .selected(self.report.open)
                                .min_size(egui::vec2(0.0, 28.0)),
                        )
                        .on_hover_text(
                            "One web page documenting the visit: the map, findings, every \
                             device and the notes. Opens in any browser; prints to PDF",
                        )
                        .clicked()
                    {
                        self.report.open = !self.report.open;
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
        theme::top_rule(ui.ctx(), bar.response.rect.bottom(), self.look.animate);
    }

    /// How the Map tab calls for attention this frame, asking for the
    /// frames its pulse needs. While the window isn't focused the pulse
    /// waits, so it plays when the user comes back.
    pub(crate) fn map_call(&mut self, ui: &egui::Ui) -> Call {
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

    /// The status bar: LAN, gateway and WAN on the left; what's running,
    /// a newer release and the version on the right.
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
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(format!("v{VERSION}"));
                    if let Some(crate::update::Found::Newer(r)) = &self.update {
                        ui.separator();
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
                    }
                    let running = self.running_names();
                    if !running.is_empty() {
                        ui.separator();
                        ui.label(format!("{} running", running.join(", ")));
                        let (dot, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot.center(), 3.0, widgets::ok());
                    }
                });
            });
        });
    }

    /// The names of the tools running now, for the status bar.
    fn running_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = TABS
            .iter()
            .filter(|(tab, ..)| self.tab_running(*tab))
            .map(|t| t.1)
            .collect();
        if self.bundle.is_running() {
            names.push("Site Bundle");
        }
        names
    }

    /// Settings, opened from the nav: appearance, the network, the Map and
    /// where the app keeps its files.
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
                 status bar. Off, nothing leaves this machine until you run something (Refresh \
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
                     request). A newer one shows in the status bar with a link; nothing is \
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

/// A soft accent fill behind the Map's nav row, `glow` 0–1.
pub(crate) fn map_glow(p: &egui::Painter, r: egui::Rect, glow: f32) {
    if glow <= 0.0 {
        return;
    }
    let accent = theme::accent();
    p.add(egui::epaint::RectShape::new(
        r,
        6.0,
        accent.gamma_multiply(0.3 * glow),
        egui::Stroke::new(1.0, accent.gamma_multiply(0.7 * glow)),
        egui::StrokeKind::Inside,
    ));
}

/// A small ring at the top-right corner of `r` (the Map's icon) until the
/// Map is opened: hollow, so it isn't read as a running job's dot.
pub(crate) fn map_ring(p: &egui::Painter, r: egui::Rect) {
    p.circle_stroke(
        egui::pos2(r.right() - 1.0, r.top() + 1.0),
        3.0,
        egui::Stroke::new(1.5, theme::accent()),
    );
}

/// Text that reads on a fill of `colour`: near-black or white, whichever
/// contrasts more (WCAG relative luminance).
pub(crate) fn on(colour: egui::Color32) -> egui::Color32 {
    let dark = egui::Color32::from_rgb(0x10, 0x12, 0x16);
    if contrast(colour, dark) >= contrast(colour, egui::Color32::WHITE) {
        dark
    } else {
        egui::Color32::WHITE
    }
}

fn luminance(c: egui::Color32) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG contrast ratio between two colours, 1 to 21.
pub(crate) fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scan_text_reads_on_every_themes_accent() {
        for t in theme::Theme::ALL {
            let a = theme::accent_for(t);
            assert!(
                contrast(a, on(a)) >= 4.5,
                "{t:?}: {:.2}",
                contrast(a, on(a))
            );
        }
    }

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
