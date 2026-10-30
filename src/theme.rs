//! Look and feel: three themes (Dark, egui's stock look, is the default), status colours that
//! stay readable on each, and the choice remembered between launches.

use crate::backdrop::Source;
use eframe::egui::{self, Color32, CornerRadius, Shadow, Stroke, Style, Visuals};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    /// Miami: midnight purple, hot pink and teal, a sunset rule.
    Miami,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Dark, Theme::Light, Theme::Miami];

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::Miami => "Miami",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Miami => "miami",
        }
    }

    fn from_key(k: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.key() == k.trim())
    }
}

/// The theme in use, for code that picks colours without a `Ui` at hand.
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Theme {
    Theme::ALL[CURRENT.load(Ordering::Relaxed) as usize % 3]
}

/// Status colours: (ok, warn, bad) per theme.
pub fn status() -> (Color32, Color32, Color32) {
    match current() {
        // The colours from before themes existed.
        Theme::Dark => (
            Color32::from_rgb(80, 200, 120),
            Color32::from_rgb(230, 180, 60),
            Color32::from_rgb(235, 90, 90),
        ),
        Theme::Light => (
            Color32::from_rgb(22, 132, 70),
            Color32::from_rgb(176, 108, 0),
            Color32::from_rgb(198, 40, 40),
        ),
        Theme::Miami => (
            Color32::from_rgb(64, 232, 200),
            Color32::from_rgb(255, 172, 70),
            Color32::from_rgb(255, 78, 120),
        ),
    }
}

/// Accent for the selected tab and other highlights.
pub fn accent() -> Color32 {
    accent_for(current())
}

fn accent_for(t: Theme) -> Color32 {
    match t {
        Theme::Dark => Visuals::dark().hyperlink_color,
        Theme::Light => Color32::from_rgb(28, 104, 176),
        Theme::Miami => PINK,
    }
}

pub fn apply(ctx: &egui::Context, theme: Theme) {
    CURRENT.store(
        Theme::ALL.iter().position(|t| *t == theme).unwrap_or(0) as u8,
        Ordering::Relaxed,
    );
    let (slot, visuals) = match theme {
        // Untouched: egui's own dark style, as the app always looked.
        Theme::Dark => {
            ctx.set_theme(egui::Theme::Dark);
            ctx.set_style_of(egui::Theme::Dark, Style::default());
            return;
        }
        Theme::Light => (egui::Theme::Light, light()),
        Theme::Miami => (egui::Theme::Dark, miami()),
    };
    let mut style = Style {
        visuals,
        ..Style::default()
    };
    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 6.0);
    s.button_padding = egui::vec2(6.0, 2.0);
    s.window_margin = egui::Margin::same(12);
    s.menu_margin = egui::Margin::same(6);
    ctx.set_theme(slot);
    ctx.set_style_of(slot, style);
}

fn rounded(v: &mut Visuals, r: u8) {
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(r);
        // Hover grows nothing: no jiggle under the pointer.
        w.expansion = 0.0;
    }
    v.window_corner_radius = CornerRadius::same(r + 4);
    v.menu_corner_radius = CornerRadius::same(r + 2);
}

fn light() -> Visuals {
    let mut v = Visuals::light();
    v.panel_fill = Color32::from_rgb(246, 247, 249);
    v.window_fill = Color32::WHITE;
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = Color32::from_rgb(237, 239, 243);
    v.code_bg_color = Color32::from_rgb(234, 237, 241);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(210, 214, 220));
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(30),
    };
    v.selection.bg_fill = Color32::from_rgb(196, 222, 246);
    v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(20, 80, 140));
    v.hyperlink_color = accent_for(Theme::Light);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(228, 231, 236);
    v.widgets.inactive.bg_fill = Color32::from_rgb(228, 231, 236);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(214, 219, 226);
    v.widgets.hovered.bg_fill = Color32::from_rgb(214, 219, 226);
    rounded(&mut v, 5);
    v
}

const PINK: Color32 = Color32::from_rgb(255, 72, 170);
const TEAL: Color32 = Color32::from_rgb(40, 226, 220);
const SUNSET: Color32 = Color32::from_rgb(255, 150, 70);

/// Neon on midnight: hot pink accents, teal outlines, warm off-white text.
fn miami() -> Visuals {
    let mut v = Visuals::dark();
    v.override_text_color = Some(Color32::from_rgb(246, 236, 255));
    // Translucent so the backdrop shows through; denser where text and
    // numbers sit (fields, graph, table stripes) to keep them crisp.
    v.panel_fill = Color32::from_rgba_unmultiplied(22, 14, 44, 184);
    v.window_fill = Color32::from_rgba_unmultiplied(30, 19, 58, 244);
    v.extreme_bg_color = Color32::from_rgba_unmultiplied(13, 8, 30, 225);
    v.faint_bg_color = Color32::from_rgba_unmultiplied(32, 22, 62, 200);
    v.code_bg_color = Color32::from_rgba_unmultiplied(36, 24, 70, 210);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(120, 50, 140));
    v.window_shadow = Shadow {
        offset: [0, 0],
        blur: 22,
        spread: 0,
        color: Color32::from_rgba_unmultiplied(255, 72, 170, 40),
    };
    v.selection.bg_fill = Color32::from_rgb(120, 28, 96);
    v.selection.stroke = Stroke::new(1.0, TEAL);
    v.hyperlink_color = TEAL;
    v.warn_fg_color = SUNSET;
    v.error_fg_color = Color32::from_rgb(255, 78, 120);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(60, 36, 100));
    for (s, bg, stroke) in [
        (
            &mut w.inactive,
            (44, 28, 84),
            Color32::from_rgb(70, 110, 150),
        ),
        (&mut w.hovered, (64, 34, 112), TEAL),
        (&mut w.active, (96, 30, 110), PINK),
        (&mut w.open, (64, 34, 112), PINK),
    ] {
        s.bg_fill = Color32::from_rgb(bg.0, bg.1, bg.2);
        s.weak_bg_fill = s.bg_fill;
        s.bg_stroke = Stroke::new(1.0, stroke);
    }
    rounded(&mut v, 6);
    v
}

/// Miami's sunset rule along the bottom of the tab bar: pink, orange
/// and teal, drifting slowly from side to side and back when animating.
/// `smooth`: animate at the display's refresh rate (there's a GPU).
pub fn top_rule(ctx: &egui::Context, y: f32, animate: bool, smooth: bool) {
    if current() != Theme::Miami {
        return;
    }
    let shift = if animate {
        let t = ctx.input(|i| i.time) as f32;
        if ctx.input(|i| i.viewport().focused != Some(false)) {
            ctx.request_repaint_after(if smooth {
                crate::backdrop::SMOOTH
            } else {
                std::time::Duration::from_millis(100)
            });
        }
        // Over to one side and back every 12 seconds.
        0.5 - 0.5 * (t * std::f32::consts::TAU / 12.0).cos()
    } else {
        0.0
    };
    let r = ctx.content_rect();
    let (top, bottom) = (y - 1.0, y + 1.0);
    // Pink → orange → teal → pink, so the cycle has no seam.
    let stops = [PINK, SUNSET, TEAL, PINK];
    let colour = |u: f32| {
        let u = u.rem_euclid(1.0) * (stops.len() - 1) as f32;
        let (i, f) = (u as usize, u.fract());
        let (a, b) = (stops[i], stops[(i + 1).min(stops.len() - 1)]);
        a.lerp_to_gamma(b, f)
    };
    let mut mesh = egui::Mesh::default();
    let n = 32;
    for i in 0..=n {
        let f = i as f32 / n as f32;
        let x = r.left() + r.width() * f;
        let c = colour(f + shift);
        mesh.colored_vertex(egui::pos2(x, top), c);
        mesh.colored_vertex(egui::pos2(x, bottom), c);
        if i > 0 {
            let b = (i as u32) * 2;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b, b + 1);
        }
    }
    // Background layer: drawn after the panel's fill, but under windows.
    ctx.layer_painter(egui::LayerId::background()).add(mesh);
}

/// Extra glow under the selected tab's underline, Miami only.
pub fn glow(painter: &egui::Painter, x: std::ops::RangeInclusive<f32>, y: f32) {
    if current() == Theme::Miami {
        painter.hline(x, y, Stroke::new(6.0, PINK.gamma_multiply(0.25)));
    }
}

/// Everything remembered between launches.
#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub theme: Theme,
    /// Miami backdrop and sunset rule move; ignored by the other themes.
    pub animate: bool,
    /// A soft glow on the bright parts of the Miami backdrop.
    pub glow: bool,
    /// Window size (points) at the last exit.
    pub size: Option<(u32, u32)>,
    pub backdrop: Source,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            animate: true,
            glow: true,
            size: None,
            backdrop: Source::Scene,
        }
    }
}

impl Settings {
    fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match k.trim() {
                "theme" => s.theme = Theme::from_key(v).unwrap_or_default(),
                "animate" => s.animate = v.trim() != "0",
                "glow" => s.glow = v.trim() != "0",
                "size" => {
                    s.size = v
                        .trim()
                        .split_once('x')
                        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                }
                "backdrop" => {
                    s.backdrop = match v.trim() {
                        "none" => Source::None,
                        v => v
                            .strip_prefix("image:")
                            .map_or(Source::Scene, |p| Source::Image(p.into())),
                    }
                }
                _ => {}
            }
        }
        s
    }

    fn text(&self) -> String {
        let backdrop = match &self.backdrop {
            Source::Scene => "scene".to_string(),
            Source::None => "none".to_string(),
            Source::Image(p) => format!("image:{}", p.display()),
        };
        let mut s = format!(
            "theme={}\nanimate={}\nglow={}\nbackdrop={backdrop}\n",
            self.theme.key(),
            self.animate as u8,
            self.glow as u8
        );
        if let Some((w, h)) = self.size {
            s += &format!("size={w}x{h}\n");
        }
        s
    }
}

/// What the theme menu asks the app to do.
pub enum MenuAction {
    /// A setting changed: apply and save.
    Changed,
    /// Ask for an image file for the Miami backdrop.
    PickImage,
}

/// Footer "Theme" menu: the themes (the default marked), and Miami's
/// backdrop and animation.
/// `renderer` says what draws the backdrop (GPU, or CPU and why).
pub fn menu(ui: &mut egui::Ui, s: &mut Settings, renderer: &str, gpu: bool) -> Option<MenuAction> {
    let mut act = None;
    ui.menu_button(format!("Theme: {} ⏷", s.theme.name()), |ui| {
        ui.label(egui::RichText::new("Theme").strong());
        for t in Theme::ALL {
            let (label, tip) = match t {
                Theme::Dark => ("Dark (default)", "egui's standard dark look"),
                Theme::Light => ("Light", "For bright rooms and screenshots in tickets"),
                Theme::Miami => ("Miami", "Midnight purple, hot pink and teal, with a 16-bit sunset"),
            };
            if ui.radio(s.theme == t, label).on_hover_text(tip).clicked() && s.theme != t {
                s.theme = t;
                act = Some(MenuAction::Changed);
            }
        }
        ui.separator();
        ui.add_enabled_ui(s.theme == Theme::Miami, |ui| {
            ui.label(egui::RichText::new("Miami backdrop").strong());
            if ui
                .radio(s.backdrop == Source::Scene, "16-bit sunset scene")
                .clicked()
                && s.backdrop != Source::Scene
            {
                s.backdrop = Source::Scene;
                act = Some(MenuAction::Changed);
            }
            let mine = match &s.backdrop {
                Source::Image(p) => format!(
                    "Your image: {}",
                    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
                ),
                _ => "Your image…".to_string(),
            };
            if ui
                .radio(matches!(s.backdrop, Source::Image(_)), mine)
                .on_hover_text("Pick a picture from this computer. It stays here; only its path is remembered.")
                .clicked()
            {
                act = Some(MenuAction::PickImage);
                ui.close();
            }
            if ui.radio(s.backdrop == Source::None, "None").clicked() && s.backdrop != Source::None {
                s.backdrop = Source::None;
                act = Some(MenuAction::Changed);
            }
            let tip = if gpu {
                "At your display's refresh rate on the GPU, paused while the window isn't focused"
            } else {
                "10 frames a second (no usable GPU), paused while the window isn't focused"
            };
            if ui.checkbox(&mut s.animate, "Animate").on_hover_text(tip).changed() {
                act = Some(MenuAction::Changed);
            }
            let tip = if gpu {
                "A soft glow and haze on the bright parts: the sun, neon and windows, or your picture's highlights"
            } else {
                "A soft glow on your picture's highlights (the 16-bit scene glows only on the GPU)"
            };
            if ui.checkbox(&mut s.glow, "Glow").on_hover_text(tip).changed() {
                act = Some(MenuAction::Changed);
            }
            ui.label(egui::RichText::new(renderer).small().weak())
                .on_hover_text("What draws the backdrop. Set IS_ALIVE_GPU=off to force the CPU.");
        });
    });
    act
}

/// This app's folder in the user's profile (settings, autosaved notes).
pub fn data_dir() -> Option<std::path::PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(std::path::PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| std::path::PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
            })
    };
    Some(base?.join("is-alive"))
}

fn settings_path() -> Option<std::path::PathBuf> {
    Some(data_dir()?.join("settings"))
}

/// Saved settings, or the defaults. A missing or unreadable file is not an error.
pub fn load() -> Settings {
    settings_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| Settings::parse(&t))
        .unwrap_or_default()
}

/// Best effort: a read-only profile just means the choice isn't remembered.
pub fn save(s: &Settings) {
    if let Some(p) = settings_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, s.text());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        for t in Theme::ALL {
            assert_eq!(Theme::from_key(t.key()), Some(t));
        }
        assert_eq!(Theme::from_key("nope"), None);
    }

    #[test]
    fn settings_round_trip() {
        let s = Settings {
            theme: Theme::Miami,
            animate: false,
            glow: false,
            size: Some((1280, 800)),
            backdrop: Source::Image("/home/me/Pictures/delorean.png".into()),
        };
        assert_eq!(Settings::parse(&s.text()), s);
        assert_eq!(Settings::parse(""), Settings::default());
        // A file from 0.10.0 had only the theme line.
        let old = Settings::parse("theme=miami\n");
        assert_eq!(
            (old.theme, old.animate, old.backdrop),
            (Theme::Miami, true, Source::Scene)
        );
    }

    #[test]
    fn status_colours_differ_per_theme() {
        let ctx = egui::Context::default();
        let mut seen = Vec::new();
        for t in Theme::ALL {
            apply(&ctx, t);
            assert_eq!(current(), t);
            seen.push(status());
        }
        assert!(seen[0] != seen[1] && seen[1] != seen[2]);
        apply(&ctx, Theme::Dark);
    }
}
