//! Site Report (Report… in the title bar): one web page that documents the
//! visit, put together from what the other tabs hold. The page itself is
//! written by `crate::report`; this is the window that picks what goes in.

use crate::log::Log;
use crate::map::view::MapTabs;
use crate::net::hostinfo::HostInfo;
use crate::report::{self, MapPart, Parts, Report, Run};
use crate::tools::bundle::{log_targets, part_row};
use crate::tools::notes::NotesTool;
use crate::widgets;
use eframe::egui::{self, RichText, Ui};

/// What the report can draw on, borrowed from the other tabs each frame.
pub struct Sources<'a> {
    pub map: &'a MapTabs,
    /// What the live map is built from.
    pub map_src: crate::map::view::Sources<'a>,
    pub log: &'a Log,
    pub info: &'a HostInfo,
    pub notes: &'a NotesTool,
}

pub struct ReportTool {
    pub open: bool,
    title: String,
    use_picture: bool,
    use_findings: bool,
    use_changes: bool,
    use_infra: bool,
    use_clients: bool,
    use_notes: bool,
    /// Off by default: Results can hold other clients' hosts and domains.
    use_results: bool,
    /// What the map gives, without its picture, for the rows: worked out
    /// again when the map changes (once a second at most during a sweep).
    preview: Option<(u64, Option<MapPart>)>,
    previewed_at: f64,
}

impl Default for ReportTool {
    fn default() -> Self {
        Self {
            open: false,
            title: String::new(),
            use_picture: true,
            use_findings: true,
            use_changes: true,
            use_infra: true,
            use_clients: true,
            use_notes: true,
            use_results: false,
            preview: None,
            previewed_at: f64::NEG_INFINITY,
        }
    }
}

impl ReportTool {
    /// Another site opened: its name as the title.
    pub fn for_site(&mut self, name: Option<&str>) {
        self.title = name
            .map(|n| crate::input::line(n, crate::input::Field::Text.max()))
            .unwrap_or_default();
        self.preview = None;
    }

    /// The Site Report window; call every frame.
    pub fn window(&mut self, ctx: &egui::Context, src: Sources) {
        if !self.open {
            return;
        }
        let key = src.map.report_key();
        let now = ctx.input(|i| i.time);
        let stale = self.preview.as_ref().is_none_or(|(k, _)| *k != key);
        if stale && (self.preview.is_none() || now - self.previewed_at >= 1.0) {
            self.preview = Some((key, src.map.report(&src.map_src, false)));
            self.previewed_at = now;
        } else if stale {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        if self.title.trim().is_empty()
            && let Some(t) = self
                .preview
                .as_ref()
                .and_then(|(_, m)| m.as_ref())
                .map(|m| m.title.trim())
                .filter(|t| !t.is_empty())
        {
            self.title = crate::input::line(t, crate::input::Field::Text.max());
        }
        let mut open = self.open;
        widgets::compact_title(ctx, || {
            egui::Window::new("Site Report")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_width(560.0)
                .show(ctx, |ui| self.body(ui, &src));
        });
        self.open = open;
    }

    fn body(&mut self, ui: &mut Ui, src: &Sources) {
        ui.weak(
            "One web page that documents the visit: for the client, the ticket or the next \
             technician.",
        );
        ui.add_space(4.0);
        egui::Grid::new("report_form")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Title");
                widgets::line(
                    ui,
                    crate::input::Field::Text,
                    &mut self.title,
                    380.0,
                    "Client or site name",
                );
                ui.end_row();
            });
        ui.separator();

        let map = self.preview.as_ref().and_then(|(_, m)| m.as_ref());
        const NO_MAP: &str = "Nothing on the map yet: run a Full scan or a Subnet Sweep";
        let runs = src.log.entries.len();
        let targets = log_targets(src.log);
        let (_, hits) = src.notes.sanitized();
        ui.label(RichText::new("Includes").strong());
        egui::Grid::new("report_parts")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                part_row(
                    ui,
                    &mut self.use_picture,
                    "Map",
                    map.map(|m| format!("{} as laid out on the Map tab, on white", m.source)),
                    NO_MAP,
                );
                part_row(
                    ui,
                    &mut self.use_findings,
                    "Findings",
                    map.map(|m| match m.findings.len() {
                        0 => "None found".to_string(),
                        _ => counts(&m.findings),
                    }),
                    NO_MAP,
                );
                part_row(
                    ui,
                    &mut self.use_changes,
                    "Changes since",
                    map.and_then(|m| m.since.as_ref())
                        .map(|s| format!("“{}” ({}) · {}", s.name, s.day, s.diff.counts())),
                    "Compare ⏷ on the Map tab picks the last visit's saved map",
                );
                let clients = map.map_or(0, |m| m.devices.iter().filter(|d| d.client).count());
                part_row(
                    ui,
                    &mut self.use_infra,
                    "Infrastructure",
                    map.map(|m| {
                        format!(
                            "{} device(s): role, IP, MAC, vendor, uplink, open ports, card notes",
                            m.devices.len() - clients
                        )
                    }),
                    NO_MAP,
                );
                part_row(
                    ui,
                    &mut self.use_clients,
                    "Clients",
                    (clients > 0).then(|| {
                        format!("{clients} client(s): every one known, whatever the map shows")
                    }),
                    "No clients on the map",
                );
                part_row(
                    ui,
                    &mut self.use_notes,
                    "Tech notes",
                    (!src.notes.is_empty()).then(|| match hits.len() {
                        0 => "Redacted as in Notes · nothing redacted".to_string(),
                        _ => format!("Redacted as in Notes · {}", crate::redact::summary(&hits)),
                    }),
                    "Nothing in Notes yet",
                );
                part_row(
                    ui,
                    &mut self.use_results,
                    "Test results",
                    (runs > 0).then(|| match targets.is_empty() {
                        true => format!("{runs} run(s)"),
                        false => format!("{runs} run(s): {}", targets.join(", ")),
                    }),
                    "Nothing in Results yet",
                );
            });
        let parts = self.parts();
        let chosen = (map.is_some()
            && (parts.picture
                || parts.findings
                || parts.infra
                || (parts.clients && clients_in(map) > 0)
                || (parts.changes && map.is_some_and(|m| m.since.is_some()))))
            || (self.use_notes && !src.notes.is_empty())
            || (self.use_results && runs > 0);
        ui.separator();
        ui.horizontal(|ui| {
            let save = ui
                .add_enabled(chosen, egui::Button::new("Save report…"))
                .on_hover_text("Save the page; open it in a browser to read, send or print")
                .on_disabled_hover_text("Nothing to report yet");
            if save.clicked() {
                self.save(src);
            }
        });
        ui.weak("A web page: opens in any browser, and prints to PDF from there.");
    }

    fn parts(&self) -> Parts {
        Parts {
            picture: self.use_picture,
            findings: self.use_findings,
            changes: self.use_changes,
            infra: self.use_infra,
            clients: self.use_clients,
        }
    }

    /// Put the page together and ask where to save it. The map is drawn
    /// now, as it's laid out; the page is written once a file is picked.
    fn save(&mut self, src: &Sources) {
        let parts = self.parts();
        let any_map =
            parts.picture || parts.findings || parts.changes || parts.infra || parts.clients;
        let map = any_map
            .then(|| src.map.report(&src.map_src, parts.picture))
            .flatten();
        let na = |v: Option<String>| v.filter(|s| !s.is_empty());
        let from: Vec<(&'static str, String)> = [
            ("LAN", na(src.info.lan.map(|(ip, b)| format!("{ip}/{b}")))),
            ("Gateway", na(src.info.gateway.map(|g| g.to_string()))),
            ("WAN", na(src.info.wan.map(|w| w.to_string()))),
        ]
        .into_iter()
        .filter_map(|(k, v)| Some((k, v?)))
        .collect();
        let runs = if self.use_results {
            src.log
                .entries
                .iter()
                .map(|e| Run {
                    tool: e.tool.to_string(),
                    target: e.target().map(str::to_string),
                    started: e.started.clone(),
                    ended: e.ended.clone(),
                    text: e.text.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let r = Report {
            title: self.title.trim().to_string(),
            made: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            from,
            map,
            parts,
            notes: (self.use_notes && !src.notes.is_empty()).then(|| src.notes.sanitized_plain()),
            runs,
        };
        let name = file_name(
            &r.title,
            &chrono::Local::now().format("%Y-%m-%d").to_string(),
        );
        widgets::save_made(name, "Web page", vec!["html", "htm"], move || {
            Ok(report::html(&r).into_bytes())
        });
    }
}

fn clients_in(map: Option<&MapPart>) -> usize {
    map.map_or(0, |m| m.devices.iter().filter(|d| d.client).count())
}

/// "1 high · 2 med · 3 low".
fn counts(f: &[report::Finding]) -> String {
    use crate::map::Sev;
    [Sev::High, Sev::Med, Sev::Low]
        .into_iter()
        .filter_map(|s| {
            let n = f.iter().filter(|x| x.sev == s).count();
            (n > 0).then(|| format!("{n} {}", s.label()))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// "Harbor Dental — Front office" on 2026-11-09 →
/// "harbor-dental-front-office-report-2026-11-09.html"; no title,
/// "site-report-2026-11-09.html".
fn file_name(title: &str, day: &str) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(60).collect();
    match slug.trim_end_matches('-') {
        "" => format!("site-report-{day}.html"),
        s => format!("{s}-report-{day}.html"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_from_the_title() {
        assert_eq!(
            file_name("Harbor Dental — Front office", "2026-11-09"),
            "harbor-dental-front-office-report-2026-11-09.html"
        );
        assert_eq!(
            file_name("  ../..  ", "2026-11-09"),
            "site-report-2026-11-09.html"
        );
        assert_eq!(file_name("", "2026-11-09"), "site-report-2026-11-09.html");
    }
}
