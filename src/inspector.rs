//! The inspector: one panel on the right for whatever is picked (a host in
//! Sweep now; a run in Results and a card on the Map to follow), so a device
//! reads the same wherever it's picked. Docked beside the content on a wide
//! window, floating over it on a small one, where Esc closes it (CLAUDE.md,
//! "UI: built for a field laptop first").

use crate::{theme, widgets};
use eframe::egui::{self, Color32, Rect, RichText, Ui, vec2};

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
    if !*open {
        return;
    }
    let width = ui.ctx().content_rect().width();
    match dock(width) {
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
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().text_edit_focused() {
                *open = false;
                return;
            }
            let v = ui.visuals().clone();
            egui::Area::new(egui::Id::new(id))
                .order(egui::Order::Middle)
                .fixed_pos(at.min)
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(v.panel_fill)
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

/// The top of the inspector: a small capitalised kind, ×, the name, and
/// its address in the accent colour.
pub fn header(ui: &mut Ui, kind: &str, title: &str, address: Option<&str>, open: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(kind.to_uppercase())
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new("×").frame(false))
                .on_hover_text("Close (Esc on a small window)")
                .clicked()
            {
                *open = false;
            }
        });
    });
    ui.add(egui::Label::new(RichText::new(title).heading().strong()).wrap());
    if let Some(a) = address {
        ui.add(
            egui::Label::new(RichText::new(a).monospace().color(theme::accent())).selectable(true),
        );
    }
    ui.add_space(4.0);
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
                .corner_radius(4)
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
    fn web_pages_follow_the_open_ports() {
        let ip = std::net::Ipv4Addr::new(192, 0, 2, 1);
        assert_eq!(web_url(ip, &[]), "https://192.0.2.1");
        assert_eq!(web_url(ip, &[22, 80]), "http://192.0.2.1");
        assert_eq!(web_url(ip, &[80, 443]), "https://192.0.2.1");
        assert_eq!(web_url(ip, &[8443]), "https://192.0.2.1:8443");
    }
}
