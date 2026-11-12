//! The inspector: one panel on the right for whatever is picked (a host in
//! Sweep, a card on the Map; Results' runs use its pieces), so a device
//! reads the same wherever it's picked. Docked beside the content on a wide
//! window, floating over it on a small one, where Esc closes it (CLAUDE.md,
//! "UI: built for a field laptop first").

use crate::{theme, widgets};
use eframe::egui::{self, Color32, Rect, RichText, Ui, Vec2, vec2};

/// Where the inspector goes for a window `width` points wide.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dock {
    /// Beside the content, this wide: the content gets narrower.
    Docked(f32),
    /// Over the content's right edge, this wide: the content keeps its width.
    Over(f32),
}

/// 360 docked from 1440 points wide, 320 docked from 1100, and over the
/// content below that (a small laptop, half a screen).
pub fn dock(width: f32) -> Dock {
    if width >= 1440.0 {
        Dock::Docked(360.0)
    } else if width >= 1100.0 {
        Dock::Docked(320.0)
    } else {
        Dock::Over(320.0_f32.min(width - 140.0).max(240.0))
    }
}

/// Where the Map's inspector goes: docked at 360 from 1440 points wide (the
/// map keeps its zoom), over the canvas below that, so the map keeps its
/// width too.
pub fn map_dock(width: f32) -> Dock {
    match dock(width) {
        Dock::Docked(_) if width < 1440.0 => Dock::Over(320.0),
        d => d,
    }
}

/// A frameless button in the inspector's header (×, pin, ⋯): 28 points
/// square, so it's easy to hit on a trackpad.
pub const ICON: Vec2 = vec2(28.0, 28.0);

/// What a button in the inspector asks the app to do with a device.
#[derive(Clone, PartialEq, Debug)]
pub enum Ask {
    Ping(String),
    Ports(String),
    Trace(String),
    /// Select it on the Map.
    OnMap(std::net::Ipv4Addr),
}

/// The inspector beside (or over) the rest of `ui`, while `open`. Call it
/// before the content when docked so the content takes what's left; when
/// floating it covers the right of `ui`'s rect, so give it a `ui` that
/// starts below anything that must stay reachable (a run bar). Shut by its
/// ×, or by Esc when floating.
pub fn show(ui: &mut Ui, id: &str, open: &mut bool, add: impl FnOnce(&mut Ui, &mut bool)) {
    let width = ui.ctx().content_rect().width();
    show_as(ui, id, dock(width), true, open, add);
}

/// `show`, placed as `at` says rather than by the window's width (the Map
/// floats it on a window where a tool page docks it). `esc`: Esc closes it
/// when floating (the Map handles Esc itself).
pub fn show_as(
    ui: &mut Ui,
    id: &str,
    at: Dock,
    esc: bool,
    open: &mut bool,
    add: impl FnOnce(&mut Ui, &mut bool),
) {
    // A floating panel comes in from its edge as it opens, fading in.
    let shown = crate::theme::appear(ui.ctx(), egui::Id::new((id, "appear")), *open);
    if !*open {
        return;
    }
    match at {
        Dock::Docked(w) => {
            egui::Panel::right(egui::Id::new(id))
                .exact_size(w)
                .resizable(false)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt((id, "scroll"))
                        .auto_shrink(false)
                        .show(ui, |ui| add(ui, open));
                });
        }
        Dock::Over(w) => {
            let room = ui.max_rect();
            let at = Rect::from_min_max(
                egui::pos2(room.max.x - w, room.min.y),
                egui::pos2(room.max.x, room.max.y),
            );
            // Esc that closes a menu or leaves a text box is theirs.
            let ctx = ui.ctx().clone();
            if esc
                && ui.input(|i| i.key_pressed(egui::Key::Escape))
                && !ctx.text_edit_focused()
                && !egui::Popup::is_any_open(&ctx)
            {
                *open = false;
                return;
            }
            let v = ui.visuals().clone();
            egui::Area::new(egui::Id::new(id))
                .order(egui::Order::Middle)
                .fixed_pos(at.min + vec2((1.0 - shown) * SLIDE, 0.0))
                .show(ui.ctx(), |ui| {
                    // Never from nothing: readable from its first frame.
                    ui.multiply_opacity(
                        crate::inspector::FAINT + (1.0 - crate::inspector::FAINT) * shown,
                    );
                    egui::Frame::new()
                        .fill(crate::theme::solid(v.panel_fill))
                        .stroke(v.widgets.noninteractive.bg_stroke)
                        .inner_margin(egui::Margin::same(10))
                        .shadow(egui::Shadow {
                            offset: [-8, 0],
                            blur: 24,
                            spread: 0,
                            color: Color32::from_black_alpha(90),
                        })
                        .show(ui, |ui| {
                            ui.set_width(w - 20.0);
                            ui.set_height(at.height() - 20.0);
                            egui::ScrollArea::vertical()
                                .id_salt((id, "scroll"))
                                .auto_shrink(false)
                                .show(ui, |ui| add(ui, open));
                        });
                });
        }
    }
}

/// How far a floating panel travels as it opens, from its edge.
pub const SLIDE: f32 = 24.0;
/// How solid it is on the frame it opens.
pub const FAINT: f32 = 0.3;

/// The top of the inspector: a small capitalised kind, × (when `close`
/// is given), the name, and its address in the accent colour.
pub fn header(
    ui: &mut Ui,
    kind: &str,
    title: &str,
    address: Option<&str>,
    close: Option<&mut bool>,
) {
    header_with(
        ui,
        kind,
        None,
        title,
        |_| {},
        |ui| {
            if let Some(open) = close
                && icon_button(ui, "×", "Close (Esc on a small window)").clicked()
            {
                *open = false;
            }
        },
    );
    if let Some(a) = address {
        self::address(ui, a);
    }
    ui.add_space(4.0);
}

/// The header's first two lines: `lead` (a grip), a `colour` swatch (a
/// card's role), the kind, `buttons` drawn right to left (× first), then
/// the name.
pub fn header_with(
    ui: &mut Ui,
    kind: &str,
    colour: Option<Color32>,
    title: &str,
    lead: impl FnOnce(&mut Ui),
    buttons: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.set_min_height(ICON.y);
        lead(ui);
        if let Some(c) = colour {
            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 2.0, c);
        }
        ui.label(
            RichText::new(kind.to_uppercase())
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            buttons(ui);
        });
    });
    ui.add(egui::Label::new(RichText::new(title).heading().strong()).wrap());
}

/// An address under the name, in the accent colour, selectable to copy.
pub fn address(ui: &mut Ui, a: &str) {
    ui.add(egui::Label::new(RichText::new(a).monospace().color(theme::accent())).selectable(true));
}

/// A header button: frameless, `ICON` square.
pub fn icon_button(ui: &mut Ui, text: &str, tip: &str) -> egui::Response {
    ui.add(egui::Button::new(text).frame(false).min_size(ICON))
        .on_hover_text(tip)
}

/// One button in the inspector's action row.
pub struct Act<'a> {
    pub label: &'a str,
    pub tip: &'a str,
    pub enabled: bool,
}

/// A row of equal buttons, wrapping on a narrow inspector; which was clicked.
pub fn actions(ui: &mut Ui, acts: &[Act]) -> Option<usize> {
    let mut hit = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (n, a) in acts.iter().enumerate() {
            if ui
                .add_enabled(
                    a.enabled,
                    egui::Button::new(a.label).min_size(vec2(56.0, 28.0)),
                )
                .on_hover_text(a.tip)
                .on_disabled_hover_text(a.tip)
                .clicked()
            {
                hit = Some(n);
            }
        }
    });
    ui.add_space(6.0);
    hit
}

/// A section's small capitalised heading.
pub fn section(ui: &mut Ui, label: &str) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(label.to_uppercase())
            .small()
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(2.0);
}

/// One detail: its name on the left, the value beside it (monospace for
/// an address, a MAC or a port), selectable to copy.
pub fn row(ui: &mut Ui, key: &str, value: &str, mono: bool) {
    ui.horizontal_top(|ui| {
        let weak = ui.visuals().weak_text_color();
        ui.allocate_ui_with_layout(
            vec2(88.0, 18.0),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                ui.set_min_width(88.0);
                ui.label(RichText::new(key).color(weak));
            },
        );
        let text = if mono {
            RichText::new(value).monospace()
        } else {
            RichText::new(value)
        };
        ui.add(egui::Label::new(text).wrap().selectable(true));
    });
}

/// Small rounded tags in a wrapping row (open ports, flags).
pub fn chips(ui: &mut Ui, items: &[String]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        let v = ui.visuals();
        let (fill, stroke) = (
            v.widgets.inactive.weak_bg_fill,
            v.widgets.inactive.bg_stroke,
        );
        for c in items {
            egui::Frame::new()
                .fill(fill)
                .stroke(stroke)
                .corner_radius(crate::theme::R_PILL)
                .inner_margin(egui::Margin::symmetric(6, 1))
                .show(ui, |ui| {
                    ui.label(RichText::new(c).monospace().small());
                });
        }
    });
}

/// A plain line under the details: what the panel can't say, or a hint.
pub fn note(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .small()
                .color(ui.visuals().weak_text_color()),
        )
        .wrap(),
    );
}

/// A host's web page from its open ports (https by default).
pub fn web_url(ip: std::net::Ipv4Addr, open: &[u16]) -> String {
    let (scheme, port) = [
        (443, "https", ""),
        (8443, "https", ":8443"),
        (80, "http", ""),
    ]
    .into_iter()
    .find(|(p, ..)| open.contains(p))
    .map_or(("https", ""), |(_, s, p)| (s, p));
    format!("{scheme}://{ip}{port}")
}

/// Open `url` in the browser, through the app's own launcher.
pub fn open(url: String) {
    widgets::open_url(url);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inspector_docks_on_wide_windows_and_floats_on_small_ones() {
        assert_eq!(dock(1920.0), Dock::Docked(360.0));
        assert_eq!(dock(1440.0), Dock::Docked(360.0));
        // A field laptop (1080p at 150%).
        assert_eq!(dock(1280.0), Dock::Docked(320.0));
        assert_eq!(dock(1100.0), Dock::Docked(320.0));
        // Below that it covers the content's right edge rather than squeeze it.
        assert_eq!(dock(1099.0), Dock::Over(320.0));
        // The smallest window (620) still leaves the content a strip to click.
        let Dock::Over(w) = dock(620.0) else {
            panic!("floats")
        };
        assert!((240.0..=480.0).contains(&w));
    }

    #[test]
    fn the_maps_inspector_floats_over_the_canvas_below_1440() {
        assert_eq!(map_dock(1920.0), Dock::Docked(360.0));
        assert_eq!(map_dock(1440.0), Dock::Docked(360.0));
        // Where a tool page docks it, the Map floats it: the map keeps its
        // width as well as its zoom.
        assert_eq!(map_dock(1439.0), Dock::Over(320.0));
        assert_eq!(map_dock(1280.0), Dock::Over(320.0));
        assert_eq!(map_dock(1120.0), Dock::Over(320.0));
        assert_eq!(map_dock(1000.0), dock(1000.0));
        assert_eq!(map_dock(620.0), dock(620.0));
    }

    #[test]
    fn web_pages_follow_the_open_ports() {
        let ip = std::net::Ipv4Addr::new(192, 0, 2, 1);
        assert_eq!(web_url(ip, &[]), "https://192.0.2.1");
        assert_eq!(web_url(ip, &[22, 80]), "http://192.0.2.1");
        assert_eq!(web_url(ip, &[80, 443]), "https://192.0.2.1");
        assert_eq!(web_url(ip, &[8443]), "https://192.0.2.1:8443");
    }
}
