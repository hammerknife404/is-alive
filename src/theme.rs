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
    /// Gloss black, scanner red and dash amber; a red light sweeps the rule.
    Scanner,
    Paper,
}

impl Theme {
    /// Menu order: the dark ones, then the light ones.
    pub const ALL: [Theme; 10] = [
        Theme::Dark,
        Theme::Slate,
        Theme::Phosphor,
        Theme::Blueprint,
        Theme::Contrast,
        Theme::Rgb,
        Theme::Scanner,
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
            Theme::Scanner => "Scanner",
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
            Theme::Scanner => "scanner",
            Theme::Paper => "paper",
        }
    }

    pub fn is_light(self) -> bool {
        matches!(self, Theme::Light | Theme::Paper)
    }

    /// A rule with moving colours under the title bar.
    fn has_rule(self) -> bool {
        matches!(self, Theme::Miami | Theme::Rgb | Theme::Scanner)
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
        // A dashboard's lamps: green, amber, red.
        Theme::Scanner => (
            Color32::from_rgb(57, 224, 106),
            Color32::from_rgb(255, 176, 0),
            Color32::from_rgb(255, 59, 48),
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

    fn from_key(v: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|e| e.key() == v.trim())
            .unwrap_or_default()
    }

    /// The invisible mark after an emoji in the notes that keeps it in this
    /// style: variation selectors 1–3, which no emoji defines, so every
    /// other program ignores them. None for Match theme: an emoji with no
    /// mark follows the app's style.
    pub fn marker(self) -> Option<char> {
        match self {
            Self::Theme => None,
            Self::Mono => Some('\u{FE00}'),
            Self::Color => Some('\u{FE01}'),
            Self::Neon => Some('\u{FE02}'),
        }
    }

    fn from_marker(c: char) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.marker() == Some(c))
    }
}

static EMOJI: AtomicU8 = AtomicU8::new(0);

/// The emoji style every text drawn from now on uses.
pub fn set_emoji_style(s: EmojiStyle) {
    let i = EmojiStyle::ALL.iter().position(|x| *x == s).unwrap_or(0);
    EMOJI.store(i as u8, Ordering::Relaxed);
}

/// The app's emoji style, as Settings has it.
pub fn emoji_style() -> EmojiStyle {
    EmojiStyle::ALL[EMOJI.load(Ordering::Relaxed) as usize % EmojiStyle::ALL.len()]
}

/// An emoji's colour in the app's style.
pub fn emoji_colour(c: char, text: Color32) -> Color32 {
    emoji_colour_in(emoji_style(), c, text)
}

/// An emoji's colour in `style`: the text's own (monochrome), one of
/// Miami's neons (the same one every time), or its natural colour.
pub fn emoji_colour_in(style: EmojiStyle, c: char, text: Color32) -> Color32 {
    let style = match style {
        EmojiStyle::Theme if matches!(current(), Theme::Miami | Theme::Rgb) => EmojiStyle::Neon,
        EmojiStyle::Theme => EmojiStyle::Mono,
        s => s,
    };
    let colour = match style {
        EmojiStyle::Neon => {
            const NEON: [Color32; 4] = [PINK, TEAL, SUNSET, Color32::from_rgb(178, 120, 255)];
            NEON[c as usize % NEON.len()]
        }
        EmojiStyle::Color => match natural(c) {
            Some(n) => n,
            None => return text,
        },
        _ => return text,
    };
    // A shade darker on a light background, so yellow and teal still read:
    // emoji in the notes keep their style whatever the theme.
    if current().is_light() {
        colour.lerp_to_gamma(Color32::BLACK, 0.3)
    } else {
        colour
    }
}

/// What colour the emoji is, drawn in colour: faces and hands yellow, stop
/// signs red, a tick green, and so on. None for those with no one colour.
fn natural(c: char) -> Option<Color32> {
    let rgb = |r, g, b| Some(Color32::from_rgb(r, g, b));
    const YELLOW: (u8, u8, u8) = (250, 200, 50);
    match c {
        '✅' | '✔' | '☑' | '🔋' | '🐛' | '➕' | '♻' => rgb(70, 190, 90),
        '❌' | '✖' | '❗' | '⛔' | '🚫' | '🔴' | '📌' | '📍' | '❤' | '💔' | '📅' | '📆' | '⏰'
        | '🚨' | '💯' | '🍎' => rgb(230, 70, 70),
        '❓' | '❔' | '❕' | '➖' => rgb(230, 90, 90),
        '⚠' | '⭐' | '🌟' | '✨' | '💡' | '⚡' | '🔑' | '🔒' | '🔓' | '🏆' | '🔔' | '☀' | '🌞' => {
            rgb(YELLOW.0, YELLOW.1, YELLOW.2)
        }
        '🔥' | '🚧' | '🎃' => rgb(250, 130, 40),
        '🔵' | '🌐' | '💧' | '🌊' | '💙' | '❄' | '📘' | 'ℹ' | '🔄' | '🆗' => {
            rgb(70, 150, 240)
        }
        '☁' => rgb(150, 190, 230),
        '💜' | '🔮' => rgb(170, 110, 240),
        '🎉' => rgb(240, 110, 180),
        '📦' => rgb(190, 130, 70),
        '📝' | '📄' | '📃' | '⚪' => rgb(225, 225, 225),
        '🔧' | '🔩' | '🛠' | '⚙' | '🔨' | '🔌' | '📎' | '⛓' | '🗑' | '🔍' | '🛡' | '💬' | '📋' => {
            rgb(150, 165, 185)
        }
        '🖥' | '💻' | '📱' | '📠' | '📞' | '☎' | '⌨' | '🖱' | '📡' | '📶' | '🗄' | '🛜' | '🖧'
        | '🖶' | '📹' | '💾' | '🖴' => rgb(120, 175, 230),
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

/// Where the emoji at byte `i` ends, past the U+FE0F and style mark that
/// belong to it (in either order), and the style the mark asks for.
pub fn emoji_end(text: &str, i: usize) -> (usize, Option<EmojiStyle>) {
    let mut end = i + text[i..].chars().next().map_or(0, char::len_utf8);
    let (mut vs16, mut style) = (false, None);
    for c in text[end..].chars() {
        let mark = EmojiStyle::from_marker(c);
        if c == '\u{FE0F}' && !vs16 {
            vs16 = true;
        } else if mark.is_some() && style.is_none() {
            style = mark;
        } else {
            break;
        }
        end += c.len_utf8();
    }
    (end, style)
}

/// The emoji at or before byte `at`, as (start, end) in bytes, when `at`
/// falls between it and its U+FE0F or style mark: a cursor there sits
/// after the emoji on screen but before marks it can't see.
pub fn inside_emoji(text: &str, at: usize) -> Option<(usize, usize)> {
    let mut i = at;
    for c in text[..at].chars().rev().take(3) {
        i -= c.len_utf8();
        if is_emoji(c) {
            let (end, _) = emoji_end(text, i);
            return (at < end).then_some((i, end));
        }
        if c != '\u{FE0F}' && EmojiStyle::from_marker(c).is_none() {
            return None;
        }
    }
    None
}

/// `emoji` with the mark that keeps it in `style` (see `EmojiStyle::marker`).
pub fn styled_emoji(emoji: &str, style: EmojiStyle) -> String {
    restyle_emoji(emoji, 0..emoji.len(), style, &[]).0
}

/// Every emoji that starts in `range` (bytes) set to `style`, except those
/// inside a `skip` range; returns the new text and how many changed style.
pub fn restyle_emoji(
    text: &str,
    range: std::ops::Range<usize>,
    style: EmojiStyle,
    skip: &[std::ops::Range<usize>],
) -> (String, usize) {
    let mut out = String::with_capacity(text.len() + 8);
    let (mut i, mut n) = (0, 0);
    while let Some(c) = text[i..].chars().next() {
        let skipped = skip.iter().any(|r| r.contains(&i));
        if !is_emoji(c) || !range.contains(&i) || skipped {
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        let (end, was) = emoji_end(text, i);
        out.extend(
            text[i..end]
                .chars()
                .filter(|c| EmojiStyle::from_marker(*c).is_none()),
        );
        out.extend(style.marker());
        if was.unwrap_or_default() != style {
            n += 1;
        }
        i = end;
    }
    (out, n)
}

/// `text` without the emoji style marks, for anything that leaves the app
/// (Copy, Save as, the Site Bundle): they're invisible, but other programs
/// get the plain emoji. The autosaved notes keep them.
pub fn strip_emoji_styles(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        if is_emoji(c) {
            let (end, _) = emoji_end(text, i);
            out.extend(
                text[i..end]
                    .chars()
                    .filter(|c| EmojiStyle::from_marker(*c).is_none()),
            );
            i = end;
        } else {
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Append `text` to `job` in `format`, with every emoji a size up and
/// coloured in its own style (a mark after it, in the notes) or the app's.
pub fn append_with_emoji(job: &mut egui::text::LayoutJob, text: &str, format: &egui::TextFormat) {
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if !is_emoji(c) {
            continue;
        }
        if start < i {
            job.append(&text[start..i], 0.0, format.clone());
        }
        // Its variation selector and style mark go with it (both draw as nothing).
        let (end, style) = emoji_end(text, i);
        let mut f = format.clone();
        f.font_id.size *= EMOJI_SCALE;
        f.color = emoji_colour_in(style.unwrap_or_else(emoji_style), c, format.color);
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

pub(crate) fn accent_for(t: Theme) -> Color32 {
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
        // egui's own dark colours, as the app always looked.
        Theme::Dark => (egui::Theme::Dark, Visuals::dark()),
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
    spacing(&mut style.spacing);
    text_sizes(&mut style);
    ctx.set_theme(slot);
    ctx.set_style_of(slot, style);
}

/// One set of spacing for every theme, so a tab is laid out the same in
/// each: rows and buttons don't change size with the colours.
fn spacing(s: &mut egui::style::Spacing) {
    s.item_spacing = egui::vec2(8.0, 6.0);
    s.button_padding = egui::vec2(5.0, 4.0);
    s.window_margin = egui::Margin::same(12);
    s.menu_margin = egui::Margin::same(6);
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
        Theme::Scanner => k(
            false,
            [
                0x0b0b0c, 0x111113, 0x050506, 0x161618, 0x1d1d20, 0x3d1512, 0xe4e4e6, 0xffffff,
                0x8c8c92, 0xff2a1a, 0xffb000,
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

/// Miami's sunset rule along the bottom of the title bar: pink, orange
/// and teal, drifting slowly from side to side and back when animating.
pub fn top_rule(ctx: &egui::Context, y: f32, animate: bool) {
    let theme = current();
    if !theme.has_rule() {
        return;
    }
    if theme == Theme::Scanner {
        return scanner_rule(ctx, y, animate);
    }
    let shift = if animate {
        // In step with the backdrop's 10 frames a second.
        let (t, wait) = crate::backdrop::tick(ctx.input(|i| i.time), crate::backdrop::FPS);
        let t = t as f32;
        if ctx.input(|i| i.viewport().focused != Some(false)) {
            crate::backdrop::repaint_in(ctx, wait);
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

/// Lamps across the rule under the tabs.
const LAMPS: usize = 24;
/// Seconds for the light to cross and come back.
const SWEEP_SECS: f64 = 2.0;
/// Lamps lit behind the light as it moves, fading.
const TRAIL: f32 = 4.0;
/// Frames a second for the sweep: lamps step, as a scanner's do.
const SCANNER_FPS: f64 = 15.0;

/// Where the light is (0 to LAMPS - 1) and which way it's going, `t`
/// seconds in: across and back, slowing a little at each end.
fn scanner_at(t: f64) -> (f32, f32) {
    let phase = (t / SWEEP_SECS).rem_euclid(1.0);
    let (u, dir) = if phase < 0.5 {
        (phase * 2.0, 1.0)
    } else {
        (2.0 - phase * 2.0, -1.0)
    };
    // Eased, so it turns at the ends rather than bouncing off them.
    let eased = 0.5 - 0.5 * (u * std::f64::consts::PI).cos();
    ((eased * (LAMPS - 1) as f64) as f32, dir)
}

/// How bright lamp `i` is (0 to 1) with the light at `x` going `dir`: full
/// under it, fading over TRAIL lamps behind, dark ahead.
fn lamp(i: usize, x: f32, dir: f32) -> f32 {
    let behind = (x - i as f32) * dir;
    if behind < -0.5 {
        0.0
    } else if behind <= 0.5 {
        1.0
    } else {
        (1.0 - (behind - 0.5) / TRAIL).max(0.0)
    }
}

/// Scanner's rule: a row of red lamps under the tabs with a light sweeping
/// across and back. Still, it rests in the middle.
fn scanner_rule(ctx: &egui::Context, y: f32, animate: bool) {
    let (x, dir) = if animate {
        let (t, wait) = crate::backdrop::tick(ctx.input(|i| i.time), SCANNER_FPS);
        if ctx.input(|i| i.viewport().focused != Some(false)) {
            crate::backdrop::repaint_in(ctx, wait);
        }
        scanner_at(t)
    } else {
        ((LAMPS - 1) as f32 / 2.0, 0.0)
    };
    let r = ctx.content_rect();
    let w = r.width() / LAMPS as f32;
    let red = accent_for(Theme::Scanner);
    let off = Color32::from_rgb(46, 10, 8);
    let painter = ctx.layer_painter(egui::LayerId::background());
    for i in 0..LAMPS {
        let b = if animate {
            lamp(i, x, dir)
        } else {
            (1.0 - (x - i as f32).abs() / 2.0).max(0.0)
        };
        let x0 = r.left() + w * i as f32;
        let cell = egui::Rect::from_min_max(
            egui::pos2(x0 + 1.0, y - 1.5),
            egui::pos2(x0 + w - 1.0, y + 1.5),
        );
        painter.rect_filled(cell, 1.0, off.lerp_to_gamma(red, b));
        if b > 0.0 {
            painter.rect_filled(
                cell.expand2(egui::vec2(1.0, 3.0)),
                3.0,
                red.gamma_multiply(0.22 * b),
            );
        }
    }
}

/// Settings › Animate: when moving effects play.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Animate {
    On,
    /// Only while the machine isn't running on its battery: on site, a
    /// laptop on battery holds still and lasts longer.
    #[default]
    Plugged,
    Off,
}

impl Animate {
    pub const ALL: [Animate; 3] = [Animate::On, Animate::Plugged, Animate::Off];

    fn key(self) -> &'static str {
        match self {
            Animate::On => "on",
            Animate::Plugged => "plugged",
            Animate::Off => "0",
        }
    }

    /// What the settings file says. Before 0.28.18 Animate was on or off and
    /// every file said which (animate=1 whether chosen or not): on reads as
    /// the new default.
    fn from_key(v: &str) -> Self {
        match v.trim() {
            "0" => Animate::Off,
            "on" => Animate::On,
            _ => Animate::Plugged,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Animate::On => "On",
            Animate::Plugged => "When plugged in",
            Animate::Off => "Off",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Animate::On => "Always, on battery too.",
            Animate::Plugged => {
                "While the laptop is plugged in. On battery everything holds still \
                 (checked every half minute); a desktop always counts as plugged in."
            }
            Animate::Off => "Everything holds still.",
        }
    }
}

/// Everything remembered between launches.
#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub theme: Theme,
    /// When moving effects play: the Miami backdrop and sunset rule, the
    /// Map tab's pulse after a full scan, the map's packets and warnings.
    /// See `moves` for this moment.
    pub animate: Animate,
    /// A soft glow on the bright parts of the Miami backdrop.
    pub glow: bool,
    /// The Miami scene as if off a worn VHS tape (on the GPU).
    pub vhs: bool,
    /// Window size (points) at the last exit.
    pub size: Option<(u32, u32)>,
    /// Show the Map tab's accuracy note (until it's closed).
    pub map_note: bool,
    /// Look up the public IP when the app starts (one DNS query to
    /// OpenDNS). Off: nothing leaves the machine until the user runs
    /// something.
    pub wan_on_start: bool,
    /// Ask GitHub for a newer release when the app starts.
    pub updates_on_start: bool,
    /// Notes save themselves a moment after each change. Off: only Save
    /// (Ctrl+S) writes them.
    pub notes_autosave: bool,
    /// Each site keeps its Results (in its folder) from one visit to the
    /// next, and Scratch for a day. Off: Results live in memory only.
    pub results_keep: bool,
    /// The site open when the app last closed (its folder name); None is
    /// Scratch.
    pub site: Option<String>,
    pub backdrop: Source,
    /// Notes and text-viewer font (see `fonts`) and size in points.
    pub notes_font: String,
    pub notes_size: u8,
    /// Share of the width the Notes editor takes in Split view.
    pub notes_split: f32,
    /// How emoji are drawn across the app.
    pub emoji: EmojiStyle,
    /// The style emoji picked in the Notes tab are inserted in.
    pub notes_emoji: EmojiStyle,
    /// Map layout picked (Cluster, Tree, Zones, Sites); None until one is.
    pub map_layout: Option<crate::map::layout::Mode>,
    /// Show the legend on the map.
    pub map_legend: bool,
    /// The manual in a window of its own (the default), not over the app.
    pub manual_window: bool,
    /// The nav pinned open (true) or folded (false); None follows the
    /// window's width (see `nav::open`).
    pub nav: Option<bool>,
    /// Where Save… dialogs start (Settings › Files); None leaves it to the
    /// system's dialog.
    pub save_dir: Option<std::path::PathBuf>,
    /// Inside `save_dir`, a folder named after the open site.
    pub save_by_site: bool,
    /// How a UniFi backup's client table is read (Config Export's "UniFi
    /// clients"): kept, so a site's backup reopens with the clients the Map
    /// shows. None (the default) leaves them out: they're often people's names.
    pub unifi_clients: crate::unifi::Clients,
    /// What draws the window: wgpu once it has opened it here (after
    /// OpenGL failed, or when asked for); None means OpenGL. See `startup`.
    pub renderer: Option<eframe::Renderer>,
}

/// Whether the public IP is looked up at start unless the user says.
pub const WAN_ON_START: bool = true;

/// Whether a newer release is looked for at start unless the user says.
pub const UPDATES_ON_START: bool = true;

/// Font sizes the Notes tab offers.
pub const NOTES_SIZES: &[u8] = &[8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32];

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            animate: Animate::default(),
            glow: true,
            vhs: true,
            size: None,
            map_note: true,
            wan_on_start: WAN_ON_START,
            updates_on_start: UPDATES_ON_START,
            notes_autosave: true,
            results_keep: true,
            site: None,
            notes_font: crate::fonts::MONO.into(),
            notes_size: 13,
            notes_split: 0.5,
            emoji: EmojiStyle::Theme,
            notes_emoji: EmojiStyle::Theme,
            backdrop: Source::Scene,
            map_layout: None,
            map_legend: false,
            manual_window: true,
            nav: None,
            unifi_clients: crate::unifi::Clients::None,
            save_dir: None,
            save_by_site: false,
            renderer: None,
        }
    }
}

impl Settings {
    /// Moving effects play now: Animate is on, or plugged in and the
    /// machine isn't on its battery.
    pub fn moves(&self) -> bool {
        match self.animate {
            Animate::On => true,
            Animate::Plugged => !crate::power::on_battery(),
            Animate::Off => false,
        }
    }

    fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match k.trim() {
                "theme" => s.theme = Theme::from_key(v).unwrap_or_default(),
                "animate" => s.animate = Animate::from_key(v),
                "glow" => s.glow = v.trim() != "0",
                "vhs" => s.vhs = v.trim() != "0",
                "map_note" => s.map_note = v.trim() != "0",
                "wan_on_start" => s.wan_on_start = v.trim() != "0",
                "updates_on_start" => s.updates_on_start = v.trim() != "0",
                "notes_autosave" => s.notes_autosave = v.trim() != "0",
                "results_keep" => s.results_keep = v.trim() != "0",
                "site" => {
                    let f = v.trim();
                    if !f.is_empty()
                        && f.len() <= 64
                        && f.bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                    {
                        s.site = Some(f.to_string());
                    }
                }
                "map_layout" => s.map_layout = crate::map::layout::Mode::from_key(v),
                "map_legend" => s.map_legend = v.trim() == "1",
                "manual_window" => s.manual_window = v.trim() != "0",
                "save_dir" => {
                    let v = v.trim();
                    // A path, as the backdrop's: no control characters.
                    if !v.is_empty() && v.len() <= 4096 && !v.chars().any(char::is_control) {
                        s.save_dir = Some(v.into());
                    }
                }
                "save_by_site" => s.save_by_site = v.trim() == "1",
                "unifi_clients" => {
                    s.unifi_clients =
                        crate::unifi::Clients::from_label(v.trim()).unwrap_or_default()
                }
                "nav" => {
                    s.nav = match v.trim() {
                        "open" => Some(true),
                        "rail" => Some(false),
                        _ => None,
                    }
                }
                "renderer" => s.renderer = v.trim().parse().ok(),
                // 0.18 builds wrote sans / mono.
                "notes_font" => {
                    s.notes_font = match v.trim() {
                        "sans" => crate::fonts::SANS.into(),
                        "mono" => crate::fonts::MONO.into(),
                        name => crate::input::line(name, 128),
                    }
                }
                "notes_split" => {
                    if let Ok(f) = v.trim().parse::<f32>()
                        && f.is_finite()
                    {
                        s.notes_split = f.clamp(0.2, 0.8);
                    }
                }
                "emoji" => s.emoji = EmojiStyle::from_key(v),
                "notes_emoji" => s.notes_emoji = EmojiStyle::from_key(v),
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
                        // A size no screen has (or a typo) opens at the default.
                        .filter(|&(w, h)| {
                            (320..=16_384).contains(&w) && (240..=16_384).contains(&h)
                        })
                }
                "backdrop" => {
                    s.backdrop = match v.trim() {
                        "none" => Source::None,
                        v => v
                            .strip_prefix("image:")
                            .filter(|p| p.len() <= 4096 && !p.chars().any(char::is_control))
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
            "theme={}\nglow={}\nbackdrop={backdrop}\n",
            self.theme.key(),
            self.glow as u8
        );
        if self.animate != Animate::default() {
            s += &format!("animate={}\n", self.animate.key());
        }
        if let Some((w, h)) = self.size {
            s += &format!("size={w}x{h}\n");
        }
        if !self.map_note {
            s += "map_note=0\n";
        }
        if self.wan_on_start != WAN_ON_START {
            s += &format!("wan_on_start={}\n", self.wan_on_start as u8);
        }
        if self.updates_on_start != UPDATES_ON_START {
            s += &format!("updates_on_start={}\n", self.updates_on_start as u8);
        }
        if !self.notes_autosave {
            s += "notes_autosave=0\n";
        }
        if !self.results_keep {
            s += "results_keep=0\n";
        }
        if let Some(site) = &self.site {
            s += &format!("site={site}\n");
        }
        if !self.vhs {
            s += "vhs=0\n";
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
        if self.notes_emoji != EmojiStyle::Theme {
            s += &format!("notes_emoji={}\n", self.notes_emoji.key());
        }
        if let Some(m) = self.map_layout {
            s += &format!("map_layout={}\n", m.key());
        }
        if self.map_legend {
            s += "map_legend=1\n";
        }
        if !self.manual_window {
            s += "manual_window=0\n";
        }
        if let Some(open) = self.nav {
            s += if open { "nav=open\n" } else { "nav=rail\n" };
        }
        if let Some(d) = &self.save_dir {
            s += &format!("save_dir={}\n", d.display());
        }
        if self.save_by_site {
            s += "save_by_site=1\n";
        }
        if self.unifi_clients != crate::unifi::Clients::None {
            s += &format!("unifi_clients={}\n", self.unifi_clients.label());
        }
        if let Some(r) = self.renderer {
            s += &format!("renderer={r}\n");
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
/// The appearance section of Settings: theme, emoji style, and the Miami backdrop.
pub fn appearance(ui: &mut egui::Ui, s: &mut Settings, gpu: bool) -> Option<MenuAction> {
    let mut act = None;
    {
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
                Theme::Scanner => (
                    "Scanner",
                    "Gloss black, red and amber, with a red light sweeping the rule",
                ),
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
        ui.label(egui::RichText::new("Emoji").strong());
        ui.horizontal_wrapped(|ui| {
            for e in EmojiStyle::ALL {
                let tip = match e {
                    EmojiStyle::Theme => "Monochrome, or neon in Miami and RGB",
                    EmojiStyle::Mono => "In the text's colour",
                    EmojiStyle::Color => "Each in its own colour: faces yellow, ticks green…",
                    EmojiStyle::Neon => "Miami's pink, teal, sunset and violet",
                };
                if ui
                    .radio(s.emoji == e, e.name())
                    .on_hover_text(tip)
                    .clicked()
                    && s.emoji != e
                {
                    s.emoji = e;
                    set_emoji_style(e);
                    act = Some(MenuAction::Changed);
                }
            }
        });
        ui.weak("Emoji in the notes can keep a style of their own (Notes emoji menu).");
        ui.separator();
        // In every theme: the Map tab's glow and the map's packets too.
        let tip = if gpu {
            "Moving effects: the Map tab pulses after a Full scan, dots flow along the map's \
             lit links until it's left alone for half a minute, a finding's warning pulses \
             three times, Miami's backdrop and the Miami and RGB line under the tabs drift, and \
             Scanner's light sweeps (10 to 30 frames a second on the GPU, paused while the \
             window isn't focused)."
        } else {
            "Moving effects: the Map tab pulses after a Full scan, dots flow along the map's \
             lit links until it's left alone for half a minute, a finding's warning pulses \
             three times, Miami's backdrop and the Miami and RGB line under the tabs drift, and \
             Scanner's light sweeps (10 to 30 frames a second, no usable GPU, paused while the \
             window isn't focused)."
        };
        ui.label(egui::RichText::new("Animate").strong())
            .on_hover_text(tip);
        for a in Animate::ALL {
            if ui
                .radio(s.animate == a, a.name())
                .on_hover_text(a.tip())
                .clicked()
                && s.animate != a
            {
                s.animate = a;
                act = Some(MenuAction::Changed);
            }
        }
        if s.animate == Animate::Plugged && crate::power::on_battery() {
            ui.weak("On battery now: everything holds still.");
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
            let tip = "Scanlines, a slight colour bleed, grain and a tracking band now and then, \
                as if off a worn tape (the 16-bit scene, on the GPU)";
            if ui
                .add_enabled(gpu, egui::Checkbox::new(&mut s.vhs, "VHS"))
                .on_hover_text(tip)
                .on_disabled_hover_text("Needs the GPU")
                .changed()
            {
                act = Some(MenuAction::Changed);
            }
        });
    }
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
        .and_then(|p| crate::input::read_capped(&p, 64 * 1024))
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

    /// Colour style gives every picker emoji a colour of its own, and the
    /// app sizes each one as an emoji.
    #[test]
    fn picker_emoji_are_emoji_with_a_natural_colour() {
        for (e, code, _) in crate::markdown::shortcodes() {
            let c = e.chars().next().unwrap();
            assert!(is_emoji(c), ":{code}: isn't sized as an emoji");
            assert!(natural(c).is_some(), ":{code}: has no natural colour");
        }
    }

    #[test]
    fn each_emoji_keeps_its_own_style_mark() {
        let fire = |s| styled_emoji("🔥", s);
        assert_eq!(fire(EmojiStyle::Theme), "🔥");
        assert_eq!(fire(EmojiStyle::Mono), "🔥\u{FE00}");
        assert_eq!(fire(EmojiStyle::Color), "🔥\u{FE01}");
        assert_eq!(fire(EmojiStyle::Neon), "🔥\u{FE02}");
        // The mark and U+FE0F belong to the emoji, in either order.
        let t = "⚠\u{FE0F}\u{FE02}x⚠\u{FE01}\u{FE0F}y⚠z";
        let warn = '⚠'.len_utf8();
        assert_eq!(emoji_end(t, 0), (warn + 6, Some(EmojiStyle::Neon)));
        let second = t.find('x').unwrap() + 1;
        assert_eq!(emoji_end(t, second).1, Some(EmojiStyle::Color));
        assert_eq!(
            emoji_end(t, t.find('z').unwrap() - warn),
            (t.len() - 1, None)
        );
        // Plain text for other programs; a VS1 after ordinary text stays.
        assert_eq!(strip_emoji_styles(t), "⚠\u{FE0F}x⚠\u{FE0F}y⚠z");
        assert_eq!(
            strip_emoji_styles("\u{2269}\u{FE00} ok"),
            "\u{2269}\u{FE00} ok"
        );
        // A cursor between an emoji and its marks is inside it.
        assert_eq!(inside_emoji(t, warn), Some((0, warn + 6)));
        assert_eq!(inside_emoji(t, warn + 3), Some((0, warn + 6)));
        assert_eq!(inside_emoji(t, warn + 6), None);
        assert_eq!(inside_emoji(t, 0), None);
        assert_eq!(inside_emoji("a⚠", "a⚠".len()), None);
    }

    #[test]
    fn restyling_sets_only_the_emoji_asked_for() {
        let t = "🔥 a ✅\u{FE00} b [🔑 x] ⭐";
        let marker = t.find('[').unwrap()..t.find(']').unwrap() + 1;
        let skip = std::slice::from_ref(&marker);
        let (neon, n) = restyle_emoji(t, 0..t.len(), EmojiStyle::Neon, skip);
        assert_eq!(n, 3);
        assert_eq!(neon, "🔥\u{FE02} a ✅\u{FE02} b [🔑 x] ⭐\u{FE02}");
        // Same as app: the marks go.
        let (plain, _) = restyle_emoji(&neon, 0..neon.len(), EmojiStyle::Theme, &[]);
        assert_eq!(plain, "🔥 a ✅ b [🔑 x] ⭐");
        // Only those that start in the range.
        let (one, n) = restyle_emoji(t, 0..1, EmojiStyle::Color, &[]);
        assert_eq!(
            (one.as_str(), n),
            ("🔥\u{FE01} a ✅\u{FE00} b [🔑 x] ⭐", 1)
        );
    }

    #[test]
    fn emoji_are_drawn_in_their_own_style() {
        let mut job = egui::text::LayoutJob::default();
        let text = Color32::from_rgb(1, 2, 3);
        let f = egui::TextFormat::simple(egui::FontId::proportional(10.0), text);
        let t = "🔥\u{FE00} 🔥\u{FE01} 🔥\u{FE02}\u{FE0F}";
        append_with_emoji(&mut job, t, &f);
        // The marks stay in the text (so the editor's cursor counts them)
        // and in their emoji's section.
        assert_eq!(job.text, t);
        let emoji: Vec<_> = job
            .sections
            .iter()
            .filter(|s| s.format.font_id.size > 10.0)
            .collect();
        assert_eq!(emoji.len(), 3);
        let span = |i: usize| emoji[i].byte_range.start.0..emoji[i].byte_range.end.0;
        assert_eq!(
            [0, 1, 2].map(|i| &job.text[span(i)]),
            ["🔥\u{FE00}", "🔥\u{FE01}", "🔥\u{FE02}\u{FE0F}"]
        );
        let [mono, colour, neon] = [0, 1, 2].map(|i| emoji[i].format.color);
        assert_eq!(mono, text);
        assert_eq!(neon, emoji_colour_in(EmojiStyle::Neon, '🔥', text));
        assert!(colour != text && colour != neon, "{colour:?}");
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
            animate: Animate::Off,
            glow: false,
            vhs: false,
            size: Some((1280, 800)),
            map_note: false,
            wan_on_start: !WAN_ON_START,
            updates_on_start: !UPDATES_ON_START,
            notes_autosave: false,
            results_keep: false,
            site: Some("harbor-dental".into()),
            backdrop: Source::Image("/home/me/Pictures/delorean.png".into()),
            notes_font: "DejaVu Sans Mono".into(),
            notes_size: 16,
            notes_split: 0.625,
            emoji: EmojiStyle::Color,
            notes_emoji: EmojiStyle::Neon,
            map_layout: Some(crate::map::layout::Mode::Zones),
            map_legend: false,
            manual_window: false,
            nav: Some(false),
            unifi_clients: crate::unifi::Clients::Anonymized,
            save_dir: Some("/home/me/Clients".into()),
            save_by_site: true,
            renderer: Some(eframe::Renderer::Wgpu),
        };
        assert_eq!(Settings::parse(&s.text()), s);
        // The map's legend: off unless turned on (it covers part of the
        // map), and only the on is written.
        assert!(!Settings::default().map_legend);
        assert!(!Settings::default().text().contains("map_legend"));
        assert!(!s.text().contains("map_legend"));
        assert!(Settings::parse("map_legend=1\n").map_legend);
        assert!(!Settings::parse("map_legend=0\n").map_legend);
        // The manual: in its own window unless put back over the app, and
        // only that is written.
        assert!(Settings::default().manual_window);
        assert!(!Settings::default().text().contains("manual_window"));
        assert!(!Settings::parse("manual_window=0\n").manual_window);
        // The nav: follows the window unless pinned, and only a pin is written.
        assert_eq!(Settings::default().nav, None);
        assert!(!Settings::default().text().contains("nav="));
        assert_eq!(Settings::parse("nav=open\n").nav, Some(true));
        assert_eq!(Settings::parse("nav=rail\n").nav, Some(false));
        assert_eq!(Settings::parse("nav=sideways\n").nav, None);
        assert_eq!(Settings::parse("renderer=vulkan\n").renderer, None);
        assert_eq!(
            Settings::parse("notes_font=sans\n").notes_font,
            crate::fonts::SANS
        );
        assert_eq!(Settings::parse(""), Settings::default());
        // Values edited by hand into nonsense fall back to the defaults.
        let odd = Settings::parse("notes_split=NaN\nsize=0x99999\nnotes_font=a\u{202E}b\n");
        assert_eq!(odd.notes_split, Settings::default().notes_split);
        assert_eq!(odd.size, None);
        assert_eq!(odd.notes_font, "ab");
        // The notes' emoji style is its own setting.
        let n = Settings::parse("emoji=mono\nnotes_emoji=color\n");
        assert_eq!(
            (n.emoji, n.notes_emoji),
            (EmojiStyle::Mono, EmojiStyle::Color)
        );
        assert!(!Settings::default().text().contains("emoji"));
        // A file from 0.10.0 had only the theme line.
        let old = Settings::parse("theme=miami\n");
        assert_eq!(
            (old.theme, old.animate, old.backdrop),
            (Theme::Miami, Animate::Plugged, Source::Scene)
        );
        // Animate: only when plugged in, unless set. Before 0.28.18 every file
        // said animate=1, chosen or not: that's the new default too.
        assert_eq!(Settings::default().animate, Animate::Plugged);
        assert!(!Settings::default().text().contains("animate"));
        assert_eq!(Settings::parse("animate=1\n").animate, Animate::Plugged);
        assert_eq!(Settings::parse("animate=0\n").animate, Animate::Off);
        for a in Animate::ALL {
            let s = Settings {
                animate: a,
                ..Settings::default()
            };
            assert_eq!(Settings::parse(&s.text()).animate, a);
        }
        // Always on, or off, moves (or not) whatever the power.
        let on = Settings {
            animate: Animate::On,
            ..Settings::default()
        };
        assert!(on.moves());
        assert!(!s.moves());
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

    #[test]
    fn scanner_sweeps_across_and_back() {
        let (x, dir) = scanner_at(0.0);
        assert_eq!((x, dir), (0.0, 1.0));
        let (x, dir) = scanner_at(SWEEP_SECS / 2.0 - 0.001);
        assert!(x > (LAMPS - 2) as f32 && dir == 1.0, "{x}");
        let (x, dir) = scanner_at(SWEEP_SECS * 0.75);
        assert!(
            (x - (LAMPS - 1) as f32 / 2.0).abs() < 0.5 && dir == -1.0,
            "{x}"
        );
        // The lamp under the light is full, the trail fades, ahead is dark.
        assert_eq!(lamp(10, 10.0, 1.0), 1.0);
        assert!(lamp(8, 10.0, 1.0) > 0.0 && lamp(8, 10.0, 1.0) < 1.0);
        assert_eq!(lamp(12, 10.0, 1.0), 0.0);
        assert_eq!(lamp(16, 10.0, 1.0), 0.0);
        assert_eq!(lamp(4, 10.0, 1.0), 0.0);
    }
}
