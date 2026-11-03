//! The Export menu, on the toolbar and the canvas's right-click menu: the
//! map as it's laid out now, in each format `map::export` writes.

use super::super::export::{self, Drawn, Format, Look, Picture};
use super::{MapView, NOTHING_YET};
use crate::widgets;
use eframe::egui::{self, Pos2, Ui};

impl MapView {
    /// "Export ⏷" on the toolbar.
    pub(super) fn export_button(&self, ui: &mut Ui, empty: bool) {
        ui.add_enabled_ui(!empty, |ui| {
            ui.menu_button("Export ⏷", |ui| self.export_items(ui))
                .response
                .on_hover_text(
                    "Save the map as a picture (SVG, PNG), a diagram to edit (draw.io, which \
                     also saves to Visio), text for docs (Mermaid, Graphviz) or a spreadsheet \
                     (CSV)",
                )
                .on_disabled_hover_text(NOTHING_YET);
        });
    }

    /// One entry per format; PNG opens its sizes and backgrounds.
    pub(super) fn export_items(&self, ui: &mut Ui) {
        let vis = ui.visuals().clone();
        for f in Format::MENU {
            if let Format::Png { .. } = f {
                ui.menu_button(f.name(), |ui| self.png_items(ui, &vis))
                    .response
                    .on_hover_text(f.hint());
                continue;
            }
            if ui
                .button(format!("{}…", f.name()))
                .on_hover_text(f.hint())
                .clicked()
            {
                self.export(f, &vis);
                ui.close();
            }
        }
    }

    /// 1× or 2×, on the theme's background or on white.
    fn png_items(&self, ui: &mut Ui, vis: &egui::Visuals) {
        egui::Grid::new("map_png").num_columns(3).show(ui, |ui| {
            for (row, white) in [("Theme background", false), ("White background", true)] {
                ui.label(row);
                for scale in [1u8, 2] {
                    let tip = match scale {
                        1 => "The map's own size",
                        _ => "Twice the size: sharper on big screens and in print",
                    };
                    if ui.button(format!("{scale}×…")).on_hover_text(tip).clicked() {
                        self.export(Format::Png { scale, white }, vis);
                        ui.close();
                    }
                }
                ui.end_row();
            }
        });
    }

    /// Ask where to save the map in `f`, as it's laid out now.
    fn export(&self, f: Format, vis: &egui::Visuals) {
        let pos: Vec<Option<Pos2>> = (0..self.map.nodes.len()).map(|i| self.target(i)).collect();
        let moved: Vec<bool> = (0..self.map.nodes.len()).map(|i| self.freed(i)).collect();
        let d = Drawn {
            map: &self.map,
            lay: &self.lay,
            pos: &pos,
            moved: &moved,
            legend: self.legend,
            caption: self.purpose.caption(),
        };
        let white = matches!(f, Format::Png { white: true, .. });
        let look = Look::new(vis, white);
        let name = export::file_name(&self.map.title, f.ext());
        match f {
            Format::Png { scale, .. } => {
                let pic = Picture::new(&d, look);
                widgets::save_made(name, f.name(), vec![f.ext()], move || {
                    pic.png(f32::from(scale))
                });
            }
            _ => {
                let text = f.text(&d, look).unwrap_or_default();
                if let Some(w) = f.warning(&text) {
                    widgets::toast(w, false);
                }
                widgets::save_file(name, f.name(), vec![f.ext()], text);
            }
        }
    }
}
