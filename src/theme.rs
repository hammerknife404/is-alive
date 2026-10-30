//! Look and feel: three themes (Dark is the default), status colours that
//! stay readable on each, and the choice remembered between launches.

use eframe::egui::{self, Color32, CornerRadius, Shadow, Stroke, Style, Visuals};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    /// Miami Vice: midnight purple, hot pink and teal, a sunset rule.
    Miami,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Dark, Theme::Light, Theme::Miami];

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::Miami => "Miami Vice",
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
        Theme::Dark => (
            Color32::from_rgb(88, 204, 132),
            Color32::from_rgb(232, 184, 72),
            Color32::from_rgb(238, 98, 98),
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
        Theme::Dark => Color32::from_rgb(86, 160, 230),
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
        Theme::Dark => (egui::Theme::Dark, dark()),
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

/// The original look, tidied: cooler greys, softer corners, a calmer accent.
fn dark() -> Visuals {
    let mut v = Visuals::dark();
    v.panel_fill = Color32::from_rgb(25, 27, 31);
    v.window_fill = Color32::from_rgb(32, 35, 40);
    v.extreme_bg_color = Color32::from_rgb(14, 16, 19);
    v.faint_bg_color = Color32::from_rgb(33, 36, 42);
    v.code_bg_color = Color32::from_rgb(36, 39, 45);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(52, 56, 64));
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    v.selection.bg_fill = Color32::from_rgb(38, 96, 150);
    v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(170, 210, 245));
    v.hyperlink_color = accent_for(Theme::Dark);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(44, 48, 55);
    v.widgets.inactive.bg_fill = Color32::from_rgb(44, 48, 55);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(56, 61, 70);
    v.widgets.hovered.bg_fill = Color32::from_rgb(56, 61, 70);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(66, 72, 82);
    rounded(&mut v, 5);
    v
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
    v.panel_fill = Color32::from_rgb(22, 14, 44);
    v.window_fill = Color32::from_rgb(30, 19, 58);
    v.extreme_bg_color = Color32::from_rgb(13, 8, 30);
    v.faint_bg_color = Color32::from_rgb(32, 22, 62);
    v.code_bg_color = Color32::from_rgb(36, 24, 70);
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

/// Miami Vice's sunset rule along the bottom of the tab bar: pink to orange
/// to teal. Nothing in the other themes.
pub fn top_rule(ctx: &egui::Context, y: f32) {
    if current() != Theme::Miami {
        return;
    }
    let r = ctx.content_rect();
    let rect = egui::Rect::from_min_max(
        egui::pos2(r.left(), y - 1.0),
        egui::pos2(r.right(), y + 1.0),
    );
    let mut mesh = egui::Mesh::default();
    let stops = [PINK, SUNSET, TEAL];
    for (i, c) in stops.iter().enumerate() {
        let x = rect.left() + rect.width() * i as f32 / (stops.len() - 1) as f32;
        mesh.colored_vertex(egui::pos2(x, rect.top()), *c);
        mesh.colored_vertex(egui::pos2(x, rect.bottom()), *c);
        if i > 0 {
            let b = (i as u32) * 2;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b, b + 1);
        }
    }
    // Background layer: drawn after the panel's fill, but under windows.
    ctx.layer_painter(egui::LayerId::background()).add(mesh);
}

/// Extra glow under the selected tab's underline, Miami Vice only.
pub fn glow(painter: &egui::Painter, x: std::ops::RangeInclusive<f32>, y: f32) {
    if current() == Theme::Miami {
        painter.hline(x, y, Stroke::new(6.0, PINK.gamma_multiply(0.25)));
    }
}

/// Footer picker; returns the new theme when it changes.
pub fn picker(ui: &mut egui::Ui, theme: Theme) -> Option<Theme> {
    let mut picked = None;
    for t in Theme::ALL.iter().rev() {
        let tip = match t {
            Theme::Dark => "Dark (default)",
            Theme::Light => "Light, for bright rooms and screenshots in tickets",
            Theme::Miami => "Midnight purple, hot pink and teal",
        };
        if ui
            .selectable_label(theme == *t, egui::RichText::new(t.name()).small())
            .on_hover_text(tip)
            .clicked()
            && theme != *t
        {
            picked = Some(*t);
        }
    }
    picked
}

fn settings_path() -> Option<std::path::PathBuf> {
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
    Some(base?.join("is-alive").join("settings"))
}

/// The saved theme, or Dark. A missing or unreadable file is not an error.
pub fn load() -> Theme {
    settings_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("theme=").and_then(Theme::from_key))
        })
        .unwrap_or_default()
}

/// Best effort: a read-only profile just means the choice isn't remembered.
pub fn save(theme: Theme) {
    if let Some(p) = settings_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, format!("theme={}\n", theme.key()));
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
