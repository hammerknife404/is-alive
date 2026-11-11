//! The nav down the window's left side: every tool, then Settings and Help
//! at its foot. Open (icons and names) by default, on every tab; an icon
//! rail only on a window too narrow for it. The fold button pins it open
//! or folded. Each tool's icon has its own colour from the theme.

use crate::chrome::{Call, map_glow, map_ring};
use crate::icons::{self, Icon};
use crate::{App, TABS, Tab, tab_key, theme, widgets};
use eframe::egui::{self, Align2, FontId, Rect, Sense, Vec2, vec2};

/// The nav's width open, and as a rail.
pub const OPEN: f32 = 216.0;
pub const RAIL: f32 = 56.0;
/// Below this window width the nav folds to the rail on its own: the
/// window where panels float over the content (CLAUDE.md), 620 at the least.
pub const OPEN_FROM: f32 = 1100.0;

/// Whether the nav is open: what the user pinned, or else open on any
/// window at least `OPEN_FROM` wide (the app opens at 1120), the Map too.
pub fn open(width: f32, pinned: Option<bool>) -> bool {
    pinned.unwrap_or(width >= OPEN_FROM)
}

/// What the fold button pins: the other state, or nothing (back to
/// following the window) when that's what the window would pick anyway.
pub fn toggled(width: f32, pinned: Option<bool>) -> Option<bool> {
    let want = !open(width, pinned);
    (want != open(width, None)).then_some(want)
}

/// A tool's icon colour: one of the theme's Map role colours, so every
/// theme keeps its own palette, and no two tools side by side share one.
pub fn tint(tab: Tab, theme: theme::Theme) -> egui::Color32 {
    use crate::map::Zone;
    let zone = match tab {
        Tab::Ping => Zone::Wireless,
        Tab::Ports => Zone::Server,
        Tab::Trace => Zone::Gear,
        Tab::Sweep => Zone::Device,
        Tab::Dns => Zone::Gateway,
        Tab::Advanced => Zone::Switch,
        Tab::Export => Zone::Device,
        Tab::Map => Zone::Gear,
        Tab::Notes => Zone::Server,
        Tab::Speed => Zone::Wireless,
        Tab::Calc => Zone::Gateway,
        Tab::Results => Zone::Switch,
    };
    zone.colour_in(theme)
}

pub fn icon(tab: Tab) -> Icon {
    match tab {
        Tab::Ping => Icon::Ping,
        Tab::Ports => Icon::Ports,
        Tab::Trace => Icon::Trace,
        Tab::Sweep => Icon::Sweep,
        Tab::Dns => Icon::Lookup,
        Tab::Advanced => Icon::Domains,
        Tab::Export => Icon::Configs,
        Tab::Map => Icon::Map,
        Tab::Notes => Icon::Notes,
        Tab::Speed => Icon::Speed,
        Tab::Calc => Icon::Calc,
        Tab::Results => Icon::Results,
    }
}

/// A line above the tabs that start a group (as the tab bar's separators were).
fn starts_group(tab: Tab) -> bool {
    matches!(tab, Tab::Dns | Tab::Export | Tab::Notes | Tab::Results)
}

/// Height of one row: taller as a rail, where it's a square button.
fn row_height(open: bool) -> f32 {
    if open { 32.0 } else { 36.0 }
}

/// The icon's side: a touch over the text's height, to find a tool at a glance.
const ICON: f32 = 20.0;

/// What a nav row looks like this frame.
struct Row<'a> {
    icon: Icon,
    label: &'a str,
    /// The tool's own colour for its icon; None (Settings, Help, Fold)
    /// draws it as the text.
    tint: Option<egui::Color32>,
    on: bool,
    /// The row's job is running: a pulsing dot.
    running: bool,
    /// The Map calling after a full scan: 0–1 glow, and the ring.
    glow: f32,
    ring: bool,
}

impl App {
    /// The nav panel, left of everything but the title and status bars.
    pub(crate) fn nav(&mut self, ui: &mut egui::Ui) {
        let width = ui.ctx().content_rect().width();
        let is_open = open(width, self.look.nav);
        let w = ui.ctx().animate_value_with_time(
            egui::Id::new("nav_width"),
            if is_open { OPEN } else { RAIL },
            0.14,
        );
        let labels = w > (OPEN + RAIL) / 2.0;
        let visuals = ui.visuals().clone();
        let frame = egui::Frame::side_top_panel(ui.style())
            .fill(visuals.panel_fill)
            .inner_margin(egui::Margin::symmetric(8, 8));
        egui::Panel::left("nav")
            .exact_size(w)
            .resizable(false)
            .frame(frame)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let call = self.map_call(ui);
                let mut dots = Vec::new();
                // The tools scroll, with no bar, when the window is too short
                // for them all (620 × 400 at the least): Settings, Help and
                // the fold button below stay put.
                let row = row_height(labels) + 2.0;
                let foot = 3.0 * row + 11.0;
                let room = (ui.available_height() - foot).max(row);
                egui::ScrollArea::vertical()
                    .max_height(room)
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (n, (tab, name, tip)) in TABS.iter().enumerate() {
                    if starts_group(*tab) {
                        separator(ui);
                    }
                    let (glow, ring) = match call {
                        Call::Glow { glow, .. } if *tab == Tab::Map => (glow, true),
                        Call::Ring if *tab == Tab::Map => (0.0, true),
                        _ => (0.0, false),
                    };
                    let row = Row {
                        icon: icon(*tab),
                        label: name,
                        tint: Some(tint(*tab, theme::current())),
                        on: self.tab == *tab,
                        running: self.tab_running(*tab),
                        glow,
                        ring,
                    };
                    let r = nav_row(ui, &row, labels).on_hover_text(match tab_key(n) {
                        Some(d) => format!("{name}: {tip}\n\nCtrl+{d}"),
                        None => format!("{name}: {tip}"),
                    });
                    if r.clicked() {
                        self.tab = *tab;
                    }
                    if row.running {
                        dots.push((r.rect, labels));
                    }
                }
                running_dots(ui, &dots);
                    });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let fold = Row {
                        icon: if is_open { Icon::Fold } else { Icon::Unfold },
                        label: if is_open { "Fold" } else { "Open" },
                        tint: None,
                        on: false,
                        running: false,
                        glow: 0.0,
                        ring: false,
                    };
                    let fold_tip = if self.look.nav.is_some() {
                        "Fold or open the nav. It stays as you leave it; open or fold it back the way the window \
                         would have it to let it follow the window's width again."
                    } else {
                        "Fold or open the nav. Now it follows the window: open from 1100 points wide, \
                         folded below that."
                    };
                    if nav_row(ui, &fold, labels).on_hover_text(fold_tip).clicked() {
                        self.look.nav = toggled(width, self.look.nav);
                        theme::save(&self.look);
                    }
                    let help = Row {
                        icon: Icon::Help,
                        label: "Help",
                        tint: None,
                        on: false,
                        running: false,
                        glow: 0.0,
                        ring: false,
                    };
                    let help = nav_row(ui, &help, labels).on_hover_text(
                        "The manual (built in, works offline), the wiki, release notes and where \
                         to report a problem. F1 opens this tab's chapter.",
                    );
                    self.help_menu(ui, &help);
                    let settings = Row {
                        icon: Icon::Settings,
                        label: "Settings",
                        tint: None,
                        on: false,
                        running: false,
                        glow: 0.0,
                        ring: false,
                    };
                    let settings = nav_row(ui, &settings, labels)
                        .on_hover_text(format!("Settings · {} theme", self.look.theme.name()));
                    self.settings_menu(ui, &settings);
                    separator(ui);
                });
            });
    }

    /// Help's menu, opening to the right of its nav row.
    fn help_menu(&mut self, ui: &egui::Ui, help: &egui::Response) {
        if self.shots_help {
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(help));
        }
        let name = TABS.iter().find(|t| t.0 == self.tab).map_or("", |t| t.1);
        let tab = self.tab;
        let picked = egui::Popup::menu(help)
            .align(egui::RectAlign::RIGHT_END)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| crate::help::menu(ui, tab, name))
            .and_then(|r| r.inner);
        if let Some(i) = picked {
            self.manual.show(i);
        }
    }

    /// Settings, opening to the right of its nav row.
    fn settings_menu(&mut self, ui: &egui::Ui, settings: &egui::Response) {
        if self.shots_settings {
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(settings));
        }
        egui::Popup::menu(settings)
            .align(egui::RectAlign::RIGHT_END)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                // A menu stops at the window's edge: scroll rather than cut
                // off the end at a small window. The popup starts at egui's
                // default area size (400 high), so the height is asked for.
                let room = (ui.ctx().content_rect().height() - 80.0).max(200.0);
                egui::ScrollArea::vertical()
                    .max_height(room)
                    .min_scrolled_height(room)
                    .show(ui, |ui| self.settings_ui(ui));
            });
    }
}

/// One row: the icon, and its name when the nav is open.
fn nav_row(ui: &mut egui::Ui, row: &Row, labels: bool) -> egui::Response {
    let size = vec2(ui.available_width(), row_height(labels));
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, row.on, row.label)
    });
    if !ui.is_rect_visible(rect) {
        return r;
    }
    let v = ui.visuals();
    let accent = theme::accent();
    let fill = if row.on {
        accent.gamma_multiply(0.16)
    } else if r.hovered() {
        v.widgets.hovered.weak_bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    let p = ui.painter();
    p.rect_filled(rect, 6.0, fill);
    if row.glow > 0.0 {
        map_glow(p, rect, row.glow);
    }
    let colour = if row.on {
        accent
    } else if r.hovered() {
        v.strong_text_color()
    } else {
        v.text_color()
    };
    let colour = colour.lerp_to_gamma(accent, row.glow);
    // A tool's icon keeps its colour whether picked, hovered or not; the
    // name and the row's fill say which is picked.
    let ink = row
        .tint
        .map_or(colour, |t| t.lerp_to_gamma(accent, row.glow));
    let side = ICON;
    let icon_at = if labels {
        Rect::from_center_size(
            egui::pos2(rect.left() + 10.0 + side / 2.0, rect.center().y),
            Vec2::splat(side),
        )
    } else {
        Rect::from_center_size(rect.center(), Vec2::splat(side))
    };
    icons::paint(p, icon_at, row.icon, ink);
    if labels {
        p.text(
            egui::pos2(icon_at.right() + 10.0, rect.center().y),
            Align2::LEFT_CENTER,
            row.label,
            FontId::proportional(14.0),
            colour,
        );
    }
    if row.ring {
        map_ring(p, icon_at.expand(3.0));
    }
    r
}

/// A hairline between groups.
fn separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    let colour = ui.visuals().widgets.noninteractive.bg_stroke.color;
    ui.painter().hline(
        rect.x_range().shrink(6.0),
        rect.center().y,
        egui::Stroke::new(1.0, colour),
    );
}

/// A pulsing dot on each row whose job is running: at the row's right end
/// when the nav is open, in the icon's corner as a rail.
fn running_dots(ui: &egui::Ui, rows: &[(Rect, bool)]) {
    if rows.is_empty() {
        return;
    }
    // Slow pulse on the spinner's 10 fps tick, held still while the window
    // isn't focused: a continuous ping can run for hours, and each frame
    // redraws the window.
    let focused = ui.input(|i| i.viewport().focused != Some(false));
    let t = crate::backdrop::busy_time(ui.ctx());
    let a = if focused {
        0.35 + 0.65 * (0.5 + 0.5 * (t * 3.0).sin())
    } else {
        1.0
    };
    for (r, open) in rows {
        let at = if *open {
            egui::pos2(r.right() - 12.0, r.center().y)
        } else {
            egui::pos2(r.center().x + 11.0, r.center().y - 10.0)
        };
        ui.painter()
            .circle_filled(at, 3.0, widgets::ok().gamma_multiply(a));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_is_open_by_default_unless_pinned() {
        // Open on a field laptop (1280), the window the app opens at
        // (1120) and anything bigger; the rail only on a narrow window.
        assert!(open(1280.0, None));
        assert!(open(1120.0, None));
        assert!(open(1920.0, None));
        assert!(!open(1000.0, None));
        assert!(!open(620.0, None));
        // Pinned, it stays as the user left it.
        assert!(open(620.0, Some(true)));
        assert!(!open(1920.0, Some(false)));
    }

    #[test]
    fn fold_button_pins_only_what_differs_from_the_window() {
        // Folding on a laptop pins it folded.
        assert_eq!(toggled(1280.0, None), Some(false));
        // Opening it again there is what the window would pick: unpinned.
        assert_eq!(toggled(1280.0, Some(false)), None);
        // Opening on a narrow window pins it open.
        assert_eq!(toggled(1000.0, None), Some(true));
        assert_eq!(toggled(1000.0, Some(true)), None);
    }

    #[test]
    fn tools_side_by_side_have_different_colours() {
        // In every theme, so each row reads apart from its neighbours.
        for t in theme::Theme::ALL {
            for pair in TABS.windows(2) {
                let (a, b) = (&pair[0], &pair[1]);
                assert_ne!(tint(a.0, t), tint(b.0, t), "{t:?}: {} and {}", a.1, b.1);
            }
        }
    }

    #[test]
    fn every_tab_has_its_own_icon() {
        let mut seen = std::collections::HashSet::new();
        for (tab, name, _) in TABS {
            assert!(seen.insert(format!("{:?}", icon(*tab))), "{name}");
        }
    }

    #[test]
    fn a_row_is_a_comfortable_click_target() {
        // CLAUDE.md: at least 28 points tall, for a trackpad.
        assert!(row_height(true) >= 28.0);
        assert!(row_height(false) >= 28.0);
        const { assert!(RAIL >= 44.0) };
    }
}
