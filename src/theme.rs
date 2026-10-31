//! Look and feel: Dark (egui's stock look, the default), Light, Miami, and
//! themes after /mapnetwork's (Slate, Phosphor, Blueprint, High contrast,
//! RGB, Paper); status colours that stay readable on each, and the choice
//! remembered between launches.

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
    // After /mapnetwork's themes.
    Slate,
    Phosphor,
    Blueprint,
    Contrast,
    /// Neon on black, a rainbow rule.
    Rgb,
    Paper,
}

impl Theme {
    /// Menu order: the dark ones, then the light ones.
    pub const ALL: [Theme; 9] = [
        Theme::Dark,
        Theme::Slate,
        Theme::Phosphor,
        Theme::Blueprint,
        Theme::Contrast,
        Theme::Rgb,
        Theme::Miami,
        Theme::Light,
        Theme::Paper,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::Miami => "Miami",
            Theme::Slate => "Slate",
            Theme::Phosphor => "Phosphor",
            Theme::Blueprint => "Blueprint",
            Theme::Contrast => "High contrast",
            Theme::Rgb => "RGB",
            Theme::Paper => "Paper",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Miami => "miami",
            Theme::Slate => "slate",
            Theme::Phosphor => "phosphor",
            Theme::Blueprint => "blueprint",
            Theme::Contrast => "contrast",
            Theme::Rgb => "rgb",
            Theme::Paper => "paper",
        }
    }

    pub fn is_light(self) -> bool {
        matches!(self, Theme::Light | Theme::Paper)
    }

    /// A rule with moving colours along the tab bar.
    fn has_rule(self) -> bool {
        matches!(self, Theme::Miami | Theme::Rgb)
    }

    fn from_key(k: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.key() == k.trim())
    }
}

/// The theme in use, for code that picks colours without a `Ui` at hand.
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Theme {
    Theme::ALL[CURRENT.load(Ordering::Relaxed) as usize % Theme::ALL.len()]
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
        Theme::Paper => (
            Color32::from_rgb(43, 125, 75),
            Color32::from_rgb(160, 100, 10),
            Color32::from_rgb(178, 58, 34),
        ),
        Theme::Slate => (
            Color32::from_rgb(114, 212, 156),
            Color32::from_rgb(230, 195, 106),
            Color32::from_rgb(255, 154, 144),
        ),
        Theme::Phosphor => (
            Color32::from_rgb(92, 242, 138),
            Color32::from_rgb(255, 180, 84),
            Color32::from_rgb(255, 138, 122),
        ),
        Theme::Blueprint => (
            Color32::from_rgb(142, 230, 176),
            Color32::from_rgb(255, 224, 138),
            Color32::from_rgb(255, 176, 168),
        ),
        Theme::Contrast => (
            Color32::from_rgb(86, 240, 138),
            Color32::from_rgb(255, 224, 102),
            Color32::from_rgb(255, 107, 107),
        ),
        Theme::Rgb => (
            Color32::from_rgb(57, 255, 20),
            Color32::from_rgb(255, 242, 0),
            Color32::from_rgb(255, 45, 85),
        ),
        Theme::Miami => (
            Color32::from_rgb(64, 232, 200),
            Color32::from_rgb(255, 172, 70),
            Color32::from_rgb(255, 78, 120),
        ),
    }
}

/// Emoji and pictographs: drawn a size up and coloured by `emoji_colour`
/// so egui's monochrome glyphs read clearly. Arrows and plain symbols that
/// sit in running text aren't included.
pub fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x2300..=0x23FF   // ⏰ ⌛
        | 0x2600..=0x27BF // ☀ ⚠ ✅ ✔ ❌ ❗ ❓
        | 0x2B50..=0x2B55 // ⭐
        | 0x1F000..=0x1FAFF)
}

/// How much bigger an emoji is than the text around it.
pub const EMOJI_SCALE: f32 = 1.3;

/// How emoji are coloured. egui draws every glyph in one colour, so
/// "colour" means each emoji in its own natural colour, not full-colour art.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum EmojiStyle {
    /// Monochrome, or Miami's neons in Miami.
    #[default]
    Theme,
    Mono,
    Color,
    Neon,
}

impl EmojiStyle {
    pub const ALL: [EmojiStyle; 4] = [Self::Theme, Self::Mono, Self::Color, Self::Neon];

    pub fn name(self) -> &'static str {
        match self {
            Self::Theme => "Match theme",
            Self::Mono => "Monochrome",
            Self::Color => "Colour",
            Self::Neon => "Miami neon",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Mono => "mono",
            Self::Color => "color",
            Self::Neon => "neon",
        }
    }
}

static EMOJI: AtomicU8 = AtomicU8::new(0);

/// The emoji style every text drawn from now on uses.
pub fn set_emoji_style(s: EmojiStyle) {
    let i = EmojiStyle::ALL.iter().position(|x| *x == s).unwrap_or(0);
    EMOJI.store(i as u8, Ordering::Relaxed);
}

fn emoji_style() -> EmojiStyle {
    EmojiStyle::ALL[EMOJI.load(Ordering::Relaxed) as usize % EmojiStyle::ALL.len()]
}

/// An emoji's colour in the current style: the text's own (monochrome),
/// one of Miami's neons (the same one every time), or its natural colour.
pub fn emoji_colour(c: char, text: Color32) -> Color32 {
    let style = match emoji_style() {
        EmojiStyle::Theme if matches!(current(), Theme::Miami | Theme::Rgb) => EmojiStyle::Neon,
        EmojiStyle::Theme => EmojiStyle::Mono,
        s => s,
    };
    match style {
        EmojiStyle::Neon => {
            const NEON: [Color32; 4] = [PINK, TEAL, SUNSET, Color32::from_rgb(178, 120, 255)];
            NEON[c as usize % NEON.len()]
        }
        EmojiStyle::Color => match natural(c) {
            // A shade darker on a light background, so yellow still reads.
            Some(n) if current().is_light() => n.lerp_to_gamma(Color32::BLACK, 0.3),
            Some(n) => n,
            None => text,
        },
        _ => text,
    }
}

/// What colour the emoji is, drawn in colour: faces and hands yellow, stop
/// signs red, a tick green, and so on. None for those with no one colour.
fn natural(c: char) -> Option<Color32> {
    let rgb = |r, g, b| Some(Color32::from_rgb(r, g, b));
    const YELLOW: (u8, u8, u8) = (250, 200, 50);
    match c {
        '✅' | '✔' | '🔋' | '🐛' | '➕' | '♻' => rgb(70, 190, 90),
        '❌' | '✖' | '❗' | '⛔' | '🚫' | '🔴' | '📌' | '📍' | '❤' | '💔' | '📅' | '📆' | '⏰'
        | '🚨' | '💯' | '🍎' => rgb(230, 70, 70),
        '❓' | '❔' | '❕' | '➖' => rgb(230, 90, 90),
        '⚠' | '⭐' | '🌟' | '✨' | '💡' | '⚡' | '🔑' | '🔒' | '🔓' | '🏆' | '🔔' | '☀' | '🌞' => {
            rgb(YELLOW.0, YELLOW.1, YELLOW.2)
        }
        '🔥' | '🚧' | '🎃' => rgb(250, 130, 40),
        '🔵' | '🌐' | '💧' | '🌊' | '💙' | '❄' | '📘' | 'ℹ' => rgb(70, 150, 240),
        '💜' | '🔮' => rgb(170, 110, 240),
        '📦' | '🗄' => rgb(190, 130, 70),
        '📝' | '📄' | '📃' | '📋' | '☁' | '⚪' => rgb(225, 225, 225),
        '🔧' | '🔩' | '🛠' | '⚙' | '🔨' | '🔌' | '📎' | '⛓' => rgb(150, 165, 185),
        '🖥' | '💻' | '📱' | '📠' | '📞' | '☎' | '⌨' | '🖱' | '📡' | '📶' => {
            rgb(120, 175, 230)
        }
        '⏳' | '⌛' => rgb(210, 170, 100),
        _ => match c as u32 {
            // Faces and hands.
            0x1F600..=0x1F64F
            | 0x1F910..=0x1F92F
            | 0x1F970..=0x1F97F
            | 0x1F44A..=0x1F450
            | 0x1F590..=0x1F596
            | 0x270A..=0x270D
            | 0x261D
            | 0x263A
            | 0x263B => rgb(YELLOW.0, YELLOW.1, YELLOW.2),
            // Green plants.
            0x1F331..=0x1F343 => rgb(80, 180, 90),
            _ => None,
        },
    }
}

/// Append `text` to `job` in `format`, with every emoji a size up and
/// coloured to match the theme.
pub fn append_with_emoji(job: &mut egui::text::LayoutJob, text: &str, format: &egui::TextFormat) {
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if !is_emoji(c) {
            continue;
        }
        if start < i {
            job.append(&text[start..i], 0.0, format.clone());
        }
        let end = i + c.len_utf8();
        // A variation selector belongs to the emoji before it.
        let end = text[end..]
            .strip_prefix('\u{FE0F}')
            .map_or(end, |_| end + '\u{FE0F}'.len_utf8());
        let mut f = format.clone();
        f.font_id.size *= EMOJI_SCALE;
        f.color = emoji_colour(c, format.color);
        f.valign = egui::Align::Center;
        job.append(&text[i..end], 0.0, f);
        start = end;
    }
    if start < text.len() {
        job.append(&text[start..], 0.0, format.clone());
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
        t => tokens(t).map_or(PINK, |k| k.accent),
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
            let mut style = Style::default();
            text_sizes(&mut style);
            ctx.set_style_of(egui::Theme::Dark, style);
            return;
        }
        Theme::Light => (egui::Theme::Light, light()),
        Theme::Miami => (egui::Theme::Dark, miami()),
        t => {
            let k = tokens(t).expect("every other theme has tokens");
            let slot = if k.light {
                egui::Theme::Light
            } else {
                egui::Theme::Dark
            };
            (slot, from_tokens(&k))
        }
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
    text_sizes(&mut style);
    ctx.set_theme(slot);
    ctx.set_style_of(slot, style);
}

/// A size up from egui's defaults (12.5 body) for every theme: easier to
/// read at arm's length on a laptop in a server closet.
fn text_sizes(style: &mut Style) {
    use egui::{FontFamily::*, FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Small, FontId::new(11.0, Proportional)),
        (TextStyle::Body, FontId::new(14.0, Proportional)),
        (TextStyle::Button, FontId::new(14.0, Proportional)),
        (TextStyle::Monospace, FontId::new(13.5, Monospace)),
        (TextStyle::Heading, FontId::new(20.0, Proportional)),
    ]
    .into();
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

/// A theme's colours, after /mapnetwork's theme tokens.
struct Tokens {
    light: bool,
    /// Panels and the map canvas.
    panel: Color32,
    /// Windows, menus and the detail panel's cards.
    window: Color32,
    /// Text fields, plots.
    input: Color32,
    /// Table stripes.
    faint: Color32,
    /// Buttons and code.
    raised: Color32,
    border: Color32,
    text: Color32,
    strong: Color32,
    muted: Color32,
    accent: Color32,
    link: Color32,
}

const fn hex(h: u32) -> Color32 {
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

fn tokens(t: Theme) -> Option<Tokens> {
    let k = |light,
             [
        panel,
        window,
        input,
        faint,
        raised,
        border,
        text,
        strong,
        muted,
        accent,
        link,
    ]: [u32; 11]| Tokens {
        light,
        panel: hex(panel),
        window: hex(window),
        input: hex(input),
        faint: hex(faint),
        raised: hex(raised),
        border: hex(border),
        text: hex(text),
        strong: hex(strong),
        muted: hex(muted),
        accent: hex(accent),
        link: hex(link),
    };
    Some(match t {
        Theme::Slate => k(
            false,
            [
                0x1f2533, 0x212838, 0x1c2230, 0x262d3d, 0x2c3446, 0x374055, 0xdde3ee, 0xeef2f8,
                0x8a94a8, 0x7aa7ff, 0xa9c2ec,
            ],
        ),
        Theme::Phosphor => k(
            false,
            [
                0x09100b, 0x0a120d, 0x08120b, 0x0d1610, 0x102016, 0x1f3325, 0xbff0cc, 0xd4ffe0,
                0x5f8c6c, 0x5cf28a, 0x4fd1ff,
            ],
        ),
        Theme::Blueprint => k(
            false,
            [
                0x0c2443, 0x0f2b50, 0x0b2342, 0x13355e, 0x173d6a, 0x2d5a8e, 0xdfeafa, 0xf2f8ff,
                0x8fb0d6, 0x8cc4ff, 0xbfe0ff,
            ],
        ),
        Theme::Contrast => k(
            false,
            [
                0x000000, 0x000000, 0x000000, 0x141414, 0x1a1a1a, 0xe6e6e6, 0xffffff, 0xffffff,
                0xd0d0d0, 0xffe066, 0x6cb6ff,
            ],
        ),
        Theme::Rgb => k(
            false,
            [
                0x0b0514, 0x0d0618, 0x07030d, 0x110822, 0x1a0b33, 0x3a1466, 0xefe6ff, 0xffffff,
                0xa58cd6, 0xff00c8, 0x00e5ff,
            ],
        ),
        Theme::Paper => k(
            true,
            [
                0xf5f0e6, 0xfbf8f2, 0xfffdf8, 0xf1ebdf, 0xece4d4, 0xddd2bd, 0x3e362a, 0x2a241b,
                0x8a7f6c, 0x2f5fb3, 0x2f5fb3,
            ],
        ),
        Theme::Dark | Theme::Light | Theme::Miami => return None,
    })
}

fn from_tokens(k: &Tokens) -> Visuals {
    let mut v = if k.light {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    v.override_text_color = Some(k.text);
    v.weak_text_color = Some(k.muted);
    v.panel_fill = k.panel;
    v.window_fill = k.window;
    v.extreme_bg_color = k.input;
    v.faint_bg_color = k.faint;
    v.code_bg_color = k.raised;
    v.window_stroke = Stroke::new(1.0, k.border);
    if !k.light {
        v.window_shadow.color = Color32::from_black_alpha(110);
    }
    v.selection.bg_fill = k
        .accent
        .lerp_to_gamma(k.panel, if k.light { 0.75 } else { 0.6 });
    v.selection.stroke = Stroke::new(1.0, k.strong);
    v.hyperlink_color = k.link;
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, k.border.lerp_to_gamma(k.panel, 0.3));
    w.noninteractive.fg_stroke = Stroke::new(1.0, k.text);
    for (s, mix, stroke) in [
        (&mut w.inactive, 0.0, k.border),
        (&mut w.hovered, 0.18, k.accent),
        (&mut w.active, 0.32, k.accent),
        (&mut w.open, 0.18, k.accent),
    ] {
        s.bg_fill = k.raised.lerp_to_gamma(k.accent, mix);
        s.weak_bg_fill = s.bg_fill;
        s.bg_stroke = Stroke::new(1.0, stroke);
        s.fg_stroke = Stroke::new(1.0, if mix > 0.0 { k.strong } else { k.text });
    }
    rounded(&mut v, 6);
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
    let theme = current();
    if !theme.has_rule() {
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
    // Pink → orange → teal → pink (RGB: the rainbow), so the cycle has no seam.
    const RAINBOW: [Color32; 8] = [
        hex(0xff0040),
        hex(0xff8a00),
        hex(0xfff200),
        hex(0x39ff14),
        hex(0x00e5ff),
        hex(0xb14dff),
        hex(0xff00c8),
        hex(0xff0040),
    ];
    let stops: &[Color32] = if theme == Theme::Rgb {
        &RAINBOW
    } else {
        &[PINK, SUNSET, TEAL, PINK]
    };
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

/// Extra glow under the selected tab's underline, Miami and RGB only.
pub fn glow(painter: &egui::Painter, x: std::ops::RangeInclusive<f32>, y: f32) {
    if current().has_rule() {
        painter.hline(x, y, Stroke::new(6.0, accent().gamma_multiply(0.25)));
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
    /// Show the Map tab's accuracy note (until it's closed).
    pub map_note: bool,
    pub backdrop: Source,
    /// Notes and text-viewer font (see `fonts`) and size in points.
    pub notes_font: String,
    pub notes_size: u8,
    /// Share of the width the Notes editor takes in Split view.
    pub notes_split: f32,
    pub emoji: EmojiStyle,
    /// Map layout picked (Cluster, Tree, Zones, Sites); None until one is.
    pub map_layout: Option<crate::map::layout::Mode>,
}

/// Font sizes the Notes tab offers.
pub const NOTES_SIZES: &[u8] = &[8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32];

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            animate: true,
            glow: true,
            size: None,
            map_note: true,
            notes_font: crate::fonts::MONO.into(),
            notes_size: 13,
            notes_split: 0.5,
            emoji: EmojiStyle::Theme,
            backdrop: Source::Scene,
            map_layout: None,
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
                "map_note" => s.map_note = v.trim() != "0",
                "map_layout" => s.map_layout = crate::map::layout::Mode::from_key(v),
                // 0.18 builds wrote sans / mono.
                "notes_font" => {
                    s.notes_font = match v.trim() {
                        "sans" => crate::fonts::SANS.into(),
                        "mono" => crate::fonts::MONO.into(),
                        name => name.into(),
                    }
                }
                "notes_split" => {
                    if let Ok(f) = v.trim().parse::<f32>() {
                        s.notes_split = f.clamp(0.2, 0.8);
                    }
                }
                "emoji" => {
                    s.emoji = EmojiStyle::ALL
                        .into_iter()
                        .find(|e| e.key() == v.trim())
                        .unwrap_or_default()
                }
                "notes_size" => {
                    if let Ok(n) = v.trim().parse::<u8>() {
                        s.notes_size = n.clamp(8, 32);
                    }
                }
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
        if !self.map_note {
            s += "map_note=0\n";
        }
        if self.notes_font != crate::fonts::MONO {
            s += &format!("notes_font={}\n", self.notes_font);
        }
        if self.notes_split != 0.5 {
            s += &format!("notes_split={:.3}\n", self.notes_split);
        }
        if self.notes_size != 13 {
            s += &format!("notes_size={}\n", self.notes_size);
        }
        if self.emoji != EmojiStyle::Theme {
            s += &format!("emoji={}\n", self.emoji.key());
        }
        if let Some(m) = self.map_layout {
            s += &format!("map_layout={}\n", m.key());
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
        ui.set_min_width(240.0);
        ui.label(egui::RichText::new("Theme").strong());
        for t in Theme::ALL {
            let (label, tip) = match t {
                Theme::Dark => ("Dark (default)", "egui's standard dark look"),
                Theme::Light => ("Light", "For bright rooms and screenshots in tickets"),
                Theme::Miami => (
                    "Miami",
                    "Midnight purple, hot pink and teal, with a 16-bit sunset",
                ),
                Theme::Slate => ("Slate", "Soft blue-grey, easy on the eyes"),
                Theme::Phosphor => ("Phosphor", "Green-screen terminal"),
                Theme::Blueprint => ("Blueprint", "White lines on drafting blue"),
                Theme::Contrast => ("High contrast", "Black and white, bright colours"),
                Theme::Rgb => ("RGB", "Neon on black, with a rainbow rule"),
                Theme::Paper => ("Paper", "Warm light, like a printed site map"),
            };
            if t == Theme::Light {
                ui.separator();
            }
            // Its map colours, as /mapnetwork's Themes menu shows them.
            let r = ui
                .horizontal(|ui| {
                    let r = ui.radio(s.theme == t, label).on_hover_text(tip);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for z in crate::map::Zone::SWATCH.iter().rev() {
                            let (dot, _) =
                                ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
                            ui.painter()
                                .circle_filled(dot.center(), 4.0, z.colour_in(t));
                        }
                    });
                    r
                })
                .inner;
            if r.clicked() && s.theme != t {
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
                    p.file_name().map_or_else(
                        || p.display().to_string(),
                        |n| n.to_string_lossy().into_owned()
                    )
                ),
                _ => "Your image…".to_string(),
            };
            if ui
                .radio(matches!(s.backdrop, Source::Image(_)), mine)
                .on_hover_text("Pick a picture (only its path is saved)")
                .clicked()
            {
                act = Some(MenuAction::PickImage);
                ui.close();
            }
            if ui.radio(s.backdrop == Source::None, "None").clicked() && s.backdrop != Source::None
            {
                s.backdrop = Source::None;
                act = Some(MenuAction::Changed);
            }
            let tip = if gpu {
                "At your display's refresh rate on the GPU, paused while the window isn't focused"
            } else {
                "10 frames a second (no usable GPU), paused while the window isn't focused"
            };
            if ui
                .checkbox(&mut s.animate, "Animate")
                .on_hover_text(tip)
                .changed()
            {
                act = Some(MenuAction::Changed);
            }
            let tip = if gpu {
                "Soft glow on bright parts"
            } else {
                "A soft glow on your picture's highlights (the 16-bit scene glows only on the GPU)"
            };
            if ui
                .checkbox(&mut s.glow, "Glow")
                .on_hover_text(tip)
                .changed()
            {
                act = Some(MenuAction::Changed);
            }
            ui.label(egui::RichText::new(renderer).small().weak())
                .on_hover_text("What draws the backdrop. Set IS_ALIVE_GPU=off to force the CPU.");
        });
    });
    act
}

/// Write a file in the profile only this user can read (notes can hold
/// site details); on Windows the profile folder already is.
pub fn write_private(path: &std::path::Path, data: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        o.mode(0o600);
        // An older file keeps its mode on open: narrow it too.
        if path.exists() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    o.open(path)?.write_all(data.as_ref())
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
    fn emoji_are_a_size_up_and_in_their_own_sections() {
        let mut job = egui::text::LayoutJob::default();
        let f = egui::TextFormat::simple(egui::FontId::proportional(10.0), Color32::WHITE);
        append_with_emoji(&mut job, "ok ✅ done ⚠\u{FE0F}!", &f);
        assert_eq!(job.text, "ok ✅ done ⚠\u{FE0F}!");
        let sizes: Vec<f32> = job.sections.iter().map(|s| s.format.font_id.size).collect();
        assert_eq!(
            sizes,
            [10.0, 10.0 * EMOJI_SCALE, 10.0, 10.0 * EMOJI_SCALE, 10.0]
        );
        assert!(is_emoji('🔑') && is_emoji('⭐') && !is_emoji('\u{2192}') && !is_emoji('a'));
    }

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
            map_note: false,
            backdrop: Source::Image("/home/me/Pictures/delorean.png".into()),
            notes_font: "DejaVu Sans Mono".into(),
            notes_size: 16,
            notes_split: 0.625,
            emoji: EmojiStyle::Color,
            map_layout: Some(crate::map::layout::Mode::Zones),
        };
        assert_eq!(Settings::parse(&s.text()), s);
        assert_eq!(
            Settings::parse("notes_font=sans\n").notes_font,
            crate::fonts::SANS
        );
        assert_eq!(Settings::parse(""), Settings::default());
        // A file from 0.10.0 had only the theme line.
        let old = Settings::parse("theme=miami\n");
        assert_eq!(
            (old.theme, old.animate, old.backdrop),
            (Theme::Miami, true, Source::Scene)
        );
    }

    #[test]
    fn every_theme_tells_the_map_roles_apart() {
        use crate::map::Zone;
        for t in Theme::ALL {
            let c: Vec<Color32> = Zone::ALL.iter().map(|z| z.colour_in(t)).collect();
            for (i, a) in c.iter().enumerate() {
                for b in &c[i + 1..] {
                    assert!(a != b, "{t:?} repeats a role colour");
                }
            }
        }
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
        for (i, a) in seen.iter().enumerate() {
            for b in &seen[i + 1..] {
                assert!(a != b);
            }
        }
        apply(&ctx, Theme::Dark);
    }
}
