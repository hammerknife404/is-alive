//! The Export menu, on the toolbar and the canvas's right-click menu: the
//! map as it's laid out now, in each format `map::export` writes.

use super::super::export::{self, Drawn, Format, Look, Picture};
use super::{Clients, Link, Map, MapView, NOTHING_YET, Sources, Zone, notes};
use crate::report;
use crate::widgets;
use eframe::egui::{self, Pos2, Ui};
use std::collections::HashMap;

impl MapView {
    /// "Export ▾" on the toolbar.
    pub(super) fn export_button(&self, ui: &mut Ui, empty: bool) {
        ui.add_enabled_ui(!empty, |ui| {
            ui.menu_button("Export ▾", |ui| self.export_items(ui))
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
        let white = matches!(f, Format::Png { white: true, .. });
        let look = Look::new(vis, white);
        let name = export::file_name(&self.map.title, f.ext());
        match f {
            Format::Png { scale, .. } => {
                let pic = self.drawn(|d| Picture::new(d, look));
                widgets::save_made(name, f.name(), vec![f.ext()], move || {
                    pic.png(f32::from(scale))
                });
            }
            _ => {
                let text = self.drawn(|d| f.text(d, look)).unwrap_or_default();
                if let Some(w) = f.warning(&text) {
                    widgets::toast(w, false);
                }
                widgets::save_file(name, f.name(), vec![f.ext()], text);
            }
        }
    }

    /// The map as laid out now, for any export: its pictures as the canvas
    /// shows it, opened up when it is.
    pub(super) fn drawn<R>(&self, f: impl FnOnce(&Drawn) -> R) -> R {
        self.drawn_as(true, f)
    }

    /// As `drawn`; without `as_shown`, flat whatever the canvas shows.
    fn drawn_as<R>(&self, as_shown: bool, f: impl FnOnce(&Drawn) -> R) -> R {
        let pos: Vec<Option<Pos2>> = (0..self.map.nodes.len())
            .map(|i| self.flat_target(i))
            .collect();
        let moved: Vec<bool> = (0..self.map.nodes.len()).map(|i| self.freed(i)).collect();
        f(&Drawn {
            map: &self.map,
            lay: &self.lay,
            pos: &pos,
            moved: &moved,
            legend: self.legend,
            caption: self.purpose.caption(),
            exploded: self.scene().filter(|_| as_shown),
        })
    }

    /// Changes when what the report would take from this map does.
    pub(super) fn report_key(&self, h: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        self.built.hash(h);
        self.map.nodes.len().hash(h);
        self.frozen.is_some().hash(h);
        self.note_scope.hash(h);
        if let Some(s) = &self.since {
            (&s.name, s.diff.counts()).hash(h);
        }
    }

    /// What the site report shows of this map: its picture as laid out now
    /// (on white, without the findings under it: the report lists them),
    /// and its findings and devices with every client and every site open,
    /// whatever the map shows. The live map adds what changed since the
    /// saved map it's compared with. None while the map is empty.
    /// `picture` draws the map too (the slow part; a preview goes without).
    pub(super) fn report(
        &self,
        src: &Sources,
        source: String,
        picture: bool,
    ) -> Option<report::MapPart> {
        if self.map.nodes.is_empty() {
            return None;
        }
        let svg = if picture {
            let look = Look::new(&egui::Visuals::light(), true).without_findings();
            self.drawn_as(false, |d| Format::Svg.text(d, look))?
        } else {
            String::new()
        };
        // A saved map is drawn from its file, every site open; the live one
        // is built again with every client.
        let built;
        let full: &Map = match &self.frozen {
            Some(m) => m,
            None => {
                let mut input = self.input(src);
                input.clients = Clients::All;
                input.reveal = None;
                built = crate::map::build(&input);
                &built
            }
        };
        let full = if full.nodes.is_empty() {
            &self.map
        } else {
            full
        };
        // Card notes live on the cards shown; a client folded away has none.
        let notes_by_id: HashMap<&str, &str> = self
            .map
            .nodes
            .iter()
            .filter(|n| !n.note.is_empty())
            .map(|n| (n.id.as_str(), n.note.as_str()))
            .collect();
        // Infrastructure by role, then clients; by address within each.
        let mut nodes: Vec<&crate::map::Node> = full
            .nodes
            .iter()
            .filter(|n| n.zone != Zone::Internet && !n.id.starts_with("more:"))
            .collect();
        nodes.sort_by(|a, b| {
            (a.client, a.zone, a.ip, &a.label).cmp(&(b.client, b.zone, b.ip, &b.label))
        });
        let devices: Vec<report::Device> = nodes
            .into_iter()
            .map(|n| {
                let uplink = n.up.as_ref().map_or(String::new(), |(p, _, port)| {
                    let up = full.nodes.get(*p).map_or("", |u| u.label.as_str());
                    match port.as_deref().filter(|p| !p.is_empty()) {
                        Some(port) => format!("{up} · {port}"),
                        None => up.to_string(),
                    }
                });
                let note = match n.note.as_str() {
                    "" => notes_by_id.get(n.id.as_str()).copied().unwrap_or(""),
                    s => s,
                };
                report::Device {
                    name: n.label.clone(),
                    role: n.zone.name().to_string(),
                    rgb: n.zone.rgb(false),
                    kind: n.kind.clone(),
                    ip: n.ip,
                    mac: n.mac,
                    vendor: export::vendor(n).unwrap_or_default(),
                    site: n.site.clone().unwrap_or_default(),
                    uplink,
                    assumed: n.link() == Some(Link::Assumed),
                    role_inferred: n.role_inferred(),
                    ports: n.open.clone(),
                    flags: n.flags.clone(),
                    note: note.to_string(),
                    client: n.client,
                }
            })
            .collect();
        let findings = full
            .findings
            .iter()
            .map(|f| report::Finding {
                sev: f.sev,
                text: f.text.clone(),
                devices: f
                    .nodes
                    .iter()
                    .filter_map(|&i| full.nodes.get(i).map(|n| n.label.clone()))
                    .collect(),
            })
            .collect();
        let since = self
            .since
            .as_ref()
            .filter(|_| self.frozen.is_none())
            .map(|s| report::Since {
                name: s.name.clone(),
                day: s.day(),
                diff: s.diff.clone(),
            });
        let caption = match &self.note_scope {
            notes::Scope::Example(_) => {
                Some("Example — a fictional site, not this network".to_string())
            }
            _ => self.purpose.caption().map(str::to_string),
        };
        let title = match self.map.title.trim() {
            "" => full.title.clone(),
            t => t.to_string(),
        };
        Some(report::MapPart {
            source,
            title,
            caption,
            svg,
            findings,
            devices,
            since,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Parts, Report};

    /// A report of example `i` as the Map tab first shows it. With
    /// IS_ALIVE_REPORT_OUT set to a folder, the page is written there to
    /// look at.
    fn example_report(i: usize, file: &str) -> report::MapPart {
        let ctx = egui::Context::default();
        let sweep = crate::tools::sweep::SweepTool::default();
        let export = crate::tools::export::ExportTool::with_example(i);
        let info = crate::net::hostinfo::HostInfo::default();
        let ov = crate::redact::Overrides::default();
        let src = Sources {
            sweep: &sweep,
            export: &export,
            info: &info,
            busy: false,
            note: false,
            legend: true,
            redact: &ov,
            animate: false,
        };
        let mut v = MapView::new(None);
        v.refresh(&ctx, &src);
        let m = v.report(&src, "Live map".into(), true).expect("a map");
        if let Some(dir) = std::env::var_os("IS_ALIVE_REPORT_OUT") {
            let r = Report {
                title: m.title.clone(),
                made: "2026-11-09 20:41".into(),
                from: vec![("LAN", "10.0.0.50/24".into())],
                map: Some(v.report(&src, "Live map".into(), true).expect("a map")),
                parts: Parts {
                    picture: true,
                    findings: true,
                    changes: true,
                    infra: true,
                    clients: true,
                },
                notes: Some("## Visit\n- [x] Checked the rack\n".into()),
                runs: Vec::new(),
            };
            std::fs::write(std::path::Path::new(&dir).join(file), report::html(&r)).unwrap();
        }
        m
    }

    #[test]
    fn a_report_takes_every_device_and_the_picture_without_findings() {
        let m = example_report(1, "contoso.html");
        // An example says it isn't this network.
        assert!(
            m.caption
                .as_deref()
                .is_some_and(|c| c.contains("fictional"))
        );
        assert!(m.svg.starts_with("<svg") || m.svg.contains("<svg"));
        // The findings are listed in the report, not drawn under the map.
        assert!(!m.findings.is_empty());
        assert!(!m.svg.contains(">Findings<"));
        // Every device, every site open: Contoso's 81, less the internet.
        assert!(m.devices.len() >= 80, "{}", m.devices.len());
        assert!(m.devices.iter().all(|d| !d.name.starts_with('+')));
        assert!(m.devices.iter().any(|d| !d.site.is_empty()));
        // Infrastructure first, then clients.
        let first_client = m.devices.iter().position(|d| d.client);
        if let Some(k) = first_client {
            assert!(m.devices[k..].iter().all(|d| d.client));
        }
        // Each finding names its devices.
        assert!(m.findings.iter().any(|f| !f.devices.is_empty()));
    }

    #[test]
    fn a_report_of_one_campus() {
        let m = example_report(0, "acme.html");
        assert!(!m.devices.is_empty());
        assert!(m.since.is_none());
    }
}
