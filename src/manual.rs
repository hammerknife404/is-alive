//! The user manual, built in: docs/manual/MANUAL.md is compiled into the
//! app, so Help and F1 work with no network (the usual state of a site with
//! a problem) and always describe this version. A window with the chapters
//! down the left. It's text only: no screenshots to fall out of date.

use crate::{Tab, widgets};
use eframe::egui;
use std::sync::OnceLock;

const TEXT: &str = include_str!("../docs/manual/MANUAL.md");

pub struct Chapter {
    /// As the manual numbers it: "4. Ping".
    pub title: &'static str,
    body: &'static str,
}

impl Chapter {
    /// The title without its number: "Ping".
    fn name(&self) -> &'static str {
        self.title.split_once(". ").map_or(self.title, |(_, n)| n)
    }
}

/// The manual's chapters, each `## ` heading outside a code block. The
/// contents list is left out (the window's list is one) and so is the
/// chapter for maintainers.
pub fn chapters() -> &'static [Chapter] {
    static C: OnceLock<Vec<Chapter>> = OnceLock::new();
    C.get_or_init(|| split(TEXT))
}

fn split(text: &'static str) -> Vec<Chapter> {
    let mut out = Vec::new();
    let mut fence = false;
    let mut open: Option<(&'static str, usize)> = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_end();
        if t.trim_start().starts_with("```") {
            fence = !fence;
        }
        if !fence && let Some(title) = t.strip_prefix("## ") {
            if let Some((title, from)) = open {
                out.push(Chapter {
                    title,
                    body: &text[from..at],
                });
            }
            open = Some((title.trim(), at + line.len()));
        }
        at += line.len();
    }
    if let Some((title, from)) = open {
        out.push(Chapter {
            title,
            body: &text[from..],
        });
    }
    out.retain(|c| c.title != "Contents" && c.name() != "Updating this manual");
    out
}

/// The chapter about a tab.
pub fn chapter_of(tab: Tab) -> usize {
    chapter_named(match tab {
        Tab::Ping => "Ping",
        Tab::Ports => "Port Check",
        Tab::Trace => "Traceroute",
        Tab::Sweep => "Subnet Sweep",
        Tab::Dns => "DNS Lookup",
        Tab::Advanced => "Advanced",
        Tab::Export => "Config Export",
        Tab::Map => "Map",
        Tab::Notes => "Notes",
        Tab::Speed => "Speed Test",
        Tab::Calc => "Subnet Calc",
        Tab::Results => "Results",
    })
}

/// The chapter whose name starts with `name`; the first if none does.
pub fn chapter_named(name: &str) -> usize {
    chapters()
        .iter()
        .position(|c| c.name().starts_with(name))
        .unwrap_or(0)
}

#[derive(Default)]
pub struct Manual {
    pub open: bool,
    chapter: usize,
    /// Scroll the text back to the top, and the list to the chapter, on
    /// the next frame.
    top: bool,
    /// Find: only the chapters that mention it are listed.
    find: String,
}

impl Manual {
    /// The chapter on show.
    pub fn chapter(&self) -> usize {
        self.chapter
    }

    /// Open the window at chapter `i`.
    pub fn show(&mut self, i: usize) {
        self.open = true;
        self.chapter = i.min(chapters().len().saturating_sub(1));
        self.top = true;
    }

    pub fn window(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let screen = ctx.content_rect();
        widgets::compact_title(ctx, || {
            egui::Window::new("Manual")
                .open(&mut open)
                .collapsible(false)
                .default_rect(egui::Rect::from_center_size(
                    screen.center(),
                    egui::vec2(
                        (screen.width() - 80.0).clamp(560.0, 940.0),
                        (screen.height() - 120.0).clamp(300.0, 640.0),
                    ),
                ))
                .min_size([560.0, 300.0])
                .max_size(screen.size() - egui::vec2(24.0, 24.0))
                .show(ctx, |ui| self.body(ui));
        });
        self.open = open;
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        let list = chapters();
        let top = self.top;
        egui::Panel::left("manual_chapters")
            .resizable(false)
            .exact_size(210.0)
            .show(ui, |ui| {
                ui.weak(egui::RichText::new("Built in: works offline").small());
                ui.add_space(4.0);
                crate::widgets::line(
                    ui,
                    crate::input::Field::Text,
                    &mut self.find,
                    ui.available_width(),
                    "Find",
                );
                ui.add_space(4.0);
                let found = matching(&self.find);
                egui::ScrollArea::vertical()
                    .id_salt("manual_list")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if found.is_empty() {
                            ui.weak("No chapter mentions it");
                        }
                        for (i, hits) in found {
                            let c = &list[i];
                            let label = match hits {
                                0 => c.title.to_string(),
                                n => format!("{} ({n})", c.title),
                            };
                            let r = ui.selectable_label(self.chapter == i, label);
                            if r.clicked() {
                                self.chapter = i;
                                self.top = true;
                            }
                            if top && self.chapter == i {
                                r.scroll_to_me(None);
                            }
                        }
                    });
            });
        let Some(c) = list.get(self.chapter) else {
            return;
        };
        let top = std::mem::take(&mut self.top);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let mut area = egui::ScrollArea::vertical()
                .id_salt("manual_text")
                .auto_shrink([false, false]);
            if top {
                area = area.vertical_scroll_offset(0.0);
            }
            area.show(ui, |ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(c.title).heading().size(22.0).strong());
                ui.add_space(6.0);
                text(ui, c.body);
                ui.add_space(12.0);
            });
        });
    }
}

/// The chapters to list for Find, each with how often it mentions the
/// words (0 when nothing is typed: every chapter).
fn matching(find: &str) -> Vec<(usize, usize)> {
    // Lowercased once: Find runs every frame the window is drawn.
    static LOWER: OnceLock<Vec<String>> = OnceLock::new();
    let lower = LOWER.get_or_init(|| {
        chapters()
            .iter()
            .map(|c| format!("{}\n{}", c.title, c.body).to_lowercase())
            .collect()
    });
    let find = find.trim().to_lowercase();
    lower
        .iter()
        .enumerate()
        .filter_map(|(i, text)| {
            if find.is_empty() {
                return Some((i, 0));
            }
            let n = text.matches(&find).count();
            (n > 0).then_some((i, n))
        })
        .collect()
}

/// A chapter's Markdown: tables here, the rest by the Notes renderer.
fn text(ui: &mut egui::Ui, body: &str) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut plain = String::new();
    let flush = |ui: &mut egui::Ui, plain: &mut String| {
        if !plain.trim().is_empty() {
            crate::markdown::render(ui, plain, &font);
        }
        plain.clear();
    };
    let mut fence = false;
    let mut lines = body.lines().peekable();
    while let Some(line) = lines.next() {
        let t = line.trim();
        if t.starts_with("```") {
            fence = !fence;
        }
        if !fence {
            if t.starts_with('|') {
                flush(ui, &mut plain);
                let mut rows = vec![t];
                while let Some(next) = lines.peek().map(|l| l.trim())
                    && next.starts_with('|')
                {
                    rows.push(next);
                    lines.next();
                }
                table(ui, &rows);
                continue;
            }
            if t == "---" {
                continue;
            }
        }
        plain.push_str(line);
        plain.push('\n');
    }
    flush(ui, &mut plain);
}

/// The cells of a table row: `| a | b |` is `["a", "b"]`.
fn cells(row: &str) -> Vec<&str> {
    let row = row.trim().trim_start_matches('|').trim_end_matches('|');
    row.split('|').map(str::trim).collect()
}

fn table(ui: &mut egui::Ui, rows: &[&str]) {
    let Some((head, rest)) = rows.split_first() else {
        return;
    };
    let head = cells(head);
    let body: Vec<Vec<&str>> = rest
        .iter()
        .map(|r| cells(r))
        .filter(|c| {
            !c.iter()
                .all(|c| c.chars().all(|ch| matches!(ch, '-' | ':')))
        })
        .collect();
    let n = head.len().max(1);
    let gap = 12.0;
    // Kept clear of the edge: a table as wide as the window would widen it,
    // and the next frame's table with it.
    let w = ui.available_width() - gap * (n as f32 - 1.0) - 24.0;
    // Two columns: a name and what it means, the second gets the room.
    let widths: Vec<f32> = if n == 2 {
        vec![w * 0.3, w * 0.7]
    } else {
        vec![w / n as f32; n]
    };
    let family = egui::FontFamily::Proportional;
    let size = egui::TextStyle::Body.resolve(ui.style()).size;
    ui.add_space(2.0);
    egui::Grid::new(ui.next_auto_id())
        .num_columns(n)
        .striped(true)
        .spacing([gap, 5.0])
        .show(ui, |ui| {
            for (row, strong) in
                std::iter::once((&head, true)).chain(body.iter().map(|r| (r, false)))
            {
                for (i, w) in widths.iter().enumerate() {
                    let cell = row.get(i).copied().unwrap_or("");
                    ui.scope(|ui| {
                        ui.set_max_width(*w);
                        // A grid's cells sit in a row, where text runs on
                        // unless told to wrap.
                        ui.add(
                            egui::Label::new(crate::markdown::inline(
                                ui, cell, size, strong, &family,
                            ))
                            .wrap(),
                        );
                    });
                }
                ui.end_row();
            }
        });
    ui.add_space(6.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TABS;

    #[test]
    fn chapters_are_the_manuals_numbered_ones() {
        let c = chapters();
        assert!(c.len() >= 20, "{}", c.len());
        for ch in c {
            assert!(
                ch.title.starts_with(|c: char| c.is_ascii_digit()),
                "{}",
                ch.title
            );
        }
        // A heading inside a code block (Notes' example) isn't a chapter.
        assert!(c.iter().all(|ch| !ch.title.starts_with("Site visit")));
    }

    #[test]
    fn every_tab_has_a_chapter() {
        let mut seen = std::collections::HashSet::new();
        for (tab, name, _) in TABS {
            let i = chapter_of(*tab);
            assert!(seen.insert(i), "{name} shares chapter {i}");
        }
        for name in ["Troubleshooting", "Keyboard and mouse"] {
            assert_eq!(chapters()[chapter_named(name)].name(), name);
        }
    }

    /// Text only: no screenshots to fall out of date (CLAUDE.md).
    #[test]
    fn the_manual_has_no_pictures() {
        assert!(!TEXT.contains("](img/"));
        assert!(TEXT.lines().all(|l| !l.trim().starts_with("![")));
    }

    #[test]
    fn find_lists_the_chapters_that_mention_it() {
        assert_eq!(matching("").len(), chapters().len());
        let found = matching("BUFFERBLOAT");
        assert!(
            found
                .iter()
                .any(|&(i, _)| chapters()[i].name() == "Speed Test")
        );
        assert!(found.iter().all(|&(_, n)| n > 0));
        assert!(matching("no chapter says xyzzy").is_empty());
    }

    #[test]
    fn table_cells() {
        assert_eq!(cells("| **Host** | A name |"), ["**Host**", "A name"]);
        assert_eq!(cells("|---|---|"), ["---", "---"]);
    }
}
