//! Compare the live map with a saved one (⋯ › Compare in the toolbar):
//! cards new since then get a NEW mark, cards with something changed a
//! CHANGED mark, and the sidebar lists them with the devices gone. The
//! comparing is in `map::compare`; this is how it shows.
//!
//! The live map is compared as built with every client, whatever Clients
//! shows, so a client hidden now isn't taken for gone. Worked out again
//! when the map is rebuilt (a sweep filling in: once a second at most).

use super::*;
use crate::map::compare::{self, Diff};

/// The saved map the live map is compared with.
pub(super) struct Since {
    /// The saved map's name and when it was saved (RFC 3339).
    pub name: String,
    pub created: String,
    was: Map,
    /// The saved map showed every client.
    was_all: bool,
    pub diff: Diff,
    /// The rebuild it was worked out for (None: not yet), and when.
    at: Option<u64>,
    at_time: f64,
}

/// What the sidebar's section asked for.
pub(super) enum SinceEv {
    /// Show these cards.
    Focus(Vec<usize>),
    Stop,
}

/// Colours of the marks: new is good news to check, changed a caution.
pub(super) fn new_colour() -> Color32 {
    widgets::ok()
}
pub(super) fn changed_colour() -> Color32 {
    widgets::warn()
}

impl Since {
    pub(super) fn new(name: String, created: String, was: Map, was_all: bool) -> Self {
        Self {
            name,
            created,
            was,
            was_all,
            diff: Diff::default(),
            at: None,
            at_time: 0.0,
        }
    }

    /// The day it was saved, as the bar shows it.
    pub(super) fn day(&self) -> String {
        day(&self.created)
    }
}

/// "2026-10-01" from a saved map's date, in local time.
pub(super) fn day(created: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(created)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|_| created.chars().take(10).collect())
}

impl MapView {
    /// Compare the live map with a saved one from now on.
    pub(super) fn compare_with(&mut self, since: Since) {
        self.since = Some(since);
    }

    /// Work the comparison out again after a rebuild: during a sweep, once
    /// a second at most.
    pub(super) fn refresh_since(&mut self, src: &Sources, now: f64) {
        // An example or the blank map is no site's: nothing to compare.
        if matches!(
            self.note_scope,
            notes::Scope::Example(_) | notes::Scope::Blank
        ) {
            self.since = None;
        }
        let built = self.built;
        let Some(s) = &self.since else { return };
        if s.at == Some(built) {
            return;
        }
        if s.at.is_some() && src.sweep.is_running() && now - s.at_time < 1.0 {
            return;
        }
        let all = {
            let mut input = self.input(src);
            input.clients = Clients::All;
            input.reveal = None;
            if input.is_empty() {
                Map::default()
            } else {
                crate::map::build(&input)
            }
        };
        let Some(s) = &mut self.since else { return };
        s.diff = compare::diff(&s.was, &all, s.was_all);
        s.at = Some(built);
        s.at_time = now;
    }

    /// Card `i`'s mark: NEW or CHANGED, or none.
    pub(super) fn since_mark_of(&self, i: usize) -> Option<(&'static str, Color32)> {
        let s = self.since.as_ref()?;
        let id = &self.map.nodes.get(i)?.id;
        if s.diff.is_new(id) {
            Some(("NEW", new_colour()))
        } else if s.diff.changes(id).is_some() {
            Some(("CHANGED", changed_colour()))
        } else {
            None
        }
    }

    /// Card `i`'s section in the detail panel: new since the saved map, or
    /// what changed on it.
    pub(super) fn since_panel(&self, ui: &mut Ui, i: usize) {
        let Some(s) = &self.since else { return };
        let Some(n) = self.map.nodes.get(i) else {
            return;
        };
        let lines: Vec<(String, Color32)> = if s.diff.is_new(&n.id) {
            vec![(format!("New: not on \"{}\"", s.name), new_colour())]
        } else if let Some(cs) = s.diff.changes(&n.id) {
            cs.iter().map(|c| (c.text(), changed_colour())).collect()
        } else {
            return;
        };
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("SINCE {}", s.day()))
                .small()
                .strong()
                .color(ui.visuals().weak_text_color()),
        )
        .on_hover_text(format!("Compared with the saved map \"{}\"", s.name));
        for (t, c) in lines {
            ui.add(egui::Label::new(RichText::new(t).color(c)).wrap());
        }
    }
}

/// The sidebar's section: the counts, then the new, changed and gone
/// devices. New and changed ones show their card on a click.
pub(super) fn since_side(ui: &mut Ui, s: &Since, map: &Map) -> Option<SinceEv> {
    let mut ev = None;
    let index: HashMap<&str, usize> = map
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let d = &s.diff;
    egui::CollapsingHeader::new(RichText::new(format!("SINCE {}", s.day())).small().strong())
        .id_salt("map_side_since")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(&s.name).strong()).truncate())
                    .on_hover_text(format!(
                        "Compared with the saved map \"{}\", saved {}",
                        s.name,
                        s.day()
                    ));
            });
            ui.horizontal(|ui| {
                if ui
                    .small_button("Copy")
                    .on_hover_text("Copy the changes as text, for a report")
                    .clicked()
                {
                    widgets::copy(ui.ctx(), d.report(&s.name, &s.day()));
                    widgets::toast("Changes copied", true);
                }
                if ui
                    .small_button("Stop")
                    .on_hover_text("Stop comparing: the marks go")
                    .clicked()
                {
                    ev = Some(SinceEv::Stop);
                }
            });
            if d.is_empty() {
                ui.weak(format!("No changes: {} devices as they were.", d.same));
            }
            let row = |ui: &mut Ui, text: String, c: Color32, at: Option<usize>, tip: String| {
                let r = ui
                    .horizontal_top(|ui| {
                        let (dot, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
                        ui.painter()
                            .circle_filled(dot.center() - vec2(0.0, 2.0), 4.0, c);
                        ui.add(egui::Label::new(text).wrap().selectable(false));
                    })
                    .response;
                match at {
                    Some(i) => {
                        let r = r.interact(Sense::click()).on_hover_text(tip);
                        if r.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        r.clicked().then_some(i)
                    }
                    None => {
                        r.on_hover_text(tip);
                        None
                    }
                }
            };
            let addr = |ip: Option<std::net::Ipv4Addr>, label: &str| {
                ip.map(|ip| ip.to_string())
                    .filter(|ip| ip != label)
                    .map(|ip| format!(" · {ip}"))
                    .unwrap_or_default()
            };
            let heading = |ui: &mut Ui, t: String, all: Vec<usize>| -> bool {
                ui.add_space(4.0);
                let r = ui.add(egui::Label::new(RichText::new(t).small().strong()).sense(
                    if all.is_empty() {
                        Sense::hover()
                    } else {
                        Sense::click()
                    },
                ));
                if all.is_empty() {
                    return false;
                }
                let r = r.on_hover_text("Show them all on the map");
                if r.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                r.clicked()
            };
            if !d.new.is_empty() {
                let all: Vec<usize> = d
                    .new
                    .iter()
                    .filter_map(|h| index.get(h.id.as_str()).copied())
                    .collect();
                if heading(ui, format!("NEW ({})", d.new.len()), all.clone()) {
                    ev = Some(SinceEv::Focus(all));
                }
                for h in &d.new {
                    let at = index.get(h.id.as_str()).copied();
                    let tip = match at {
                        Some(_) => "Show it on the map".to_string(),
                        None => "A client not drawn now: Clients ⏷ shows more".to_string(),
                    };
                    if let Some(i) = row(
                        ui,
                        format!("{}{}", h.label, addr(h.ip, &h.label)),
                        new_colour(),
                        at,
                        tip,
                    ) {
                        ev = Some(SinceEv::Focus(vec![i]));
                    }
                }
            }
            if !d.changed.is_empty() {
                let all: Vec<usize> = d
                    .changed
                    .iter()
                    .filter_map(|(h, _)| index.get(h.id.as_str()).copied())
                    .collect();
                if heading(ui, format!("CHANGED ({})", d.changed.len()), all.clone()) {
                    ev = Some(SinceEv::Focus(all));
                }
                for (h, cs) in &d.changed {
                    let at = index.get(h.id.as_str()).copied();
                    let what: Vec<String> = cs.iter().map(|c| c.text()).collect();
                    if let Some(i) = row(
                        ui,
                        format!("{}: {}", h.label, what.join("; ")),
                        changed_colour(),
                        at,
                        "Show it on the map".to_string(),
                    ) {
                        ev = Some(SinceEv::Focus(vec![i]));
                    }
                }
            }
            if !d.gone.is_empty() {
                heading(ui, format!("GONE ({})", d.gone.len()), Vec::new());
                for g in &d.gone {
                    let mut tip = format!("On \"{}\", not on the map now", s.name);
                    if let Some(m) = g.mac {
                        tip.push_str(&format!(". MAC {}", crate::net::mac::fmt(&m)));
                    }
                    row(
                        ui,
                        format!("{}{}", g.label, addr(g.ip, &g.label)),
                        ui.visuals().weak_text_color(),
                        None,
                        tip,
                    );
                }
            }
            let mut notes = Vec::new();
            if !d.ports {
                notes.push("Open ports aren't compared: they weren't checked on both maps.");
            }
            if !d.clients {
                notes.push(
                    "New clients aren't flagged: the saved map didn't show every client \
                 (save one with Clients: All to compare them next time).",
                );
            }
            for n in notes {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(n).small().weak()).wrap());
            }
        });
    ev
}

/// A card's NEW or CHANGED mark: a small tag over its top edge, on the
/// left. Zoomed far out, a dot.
pub(super) fn mark(p: &Painter, r: Rect, z: f32, a: f32, (text, c): (&str, Color32)) {
    let c = c.gamma_multiply(a);
    if z < 0.45 {
        // A dot on the corner, ringed so it reads on any card colour.
        let at = r.left_top() + vec2(3.0, 3.0);
        p.circle_filled(at, 5.0, Color32::BLACK.gamma_multiply(0.6 * a));
        p.circle_filled(at, 4.0, c);
        return;
    }
    let font = FontId::proportional((8.5 * z).clamp(8.0, 10.0));
    let g = p.layout_no_wrap(text.to_string(), font, Color32::BLACK.gamma_multiply(a));
    let pad = vec2(4.0, 1.0);
    let size = g.size() + pad * 2.0;
    let at = pos2(r.min.x + 10.0 * z, r.min.y - size.y / 2.0);
    let tag = Rect::from_min_size(at, size);
    p.rect_filled(tag, CornerRadius::same((size.y / 2.0) as u8), c);
    p.galley(at + pad, g, Color32::BLACK);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_date_reads_as_a_day() {
        let d = day("2026-10-01T12:00:00Z");
        // Local time: the 1st, or a day either side far from UTC.
        assert!(
            ["2026-09-30", "2026-10-01", "2026-10-02"].contains(&d.as_str()),
            "{d}"
        );
        assert_eq!(day("garbled date"), "garbled da");
    }
}
