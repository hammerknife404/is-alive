//! The user manual, built in: docs/manual/MANUAL.md is compiled into the
//! app, so Help and F1 work with no network (the usual state of a site with
//! a problem) and always describe this version. A window with the chapters
//! down the left, over the app or (Pop out) a window of its own to keep
//! beside it or on another screen. Its links go to the chapter or heading
//! they name, or open in the browser. It's text only: no screenshots to
//! fall out of date.

use crate::{Tab, widgets};
use eframe::egui;
use std::sync::{Arc, Mutex, OnceLock};

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

/// The manual's own window, when it's popped out.
const OWN: &str = "manual_window";
/// How long after it opens the manual's window gives the keyboard back.
const GIVE_BACK_SECS: f64 = 1.5;

/// The manual. Its state is shared with its own window, which draws on
/// its own (an egui deferred viewport): the app and the manual each
/// repaint and take input independently, and neither waits for the other.
#[derive(Default)]
pub struct Manual {
    s: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    open: bool,
    /// In a window of its own (Pop out) rather than over the app; kept in
    /// the settings.
    own_window: bool,
    /// Pop out or Back in is-alive was clicked: for the settings.
    moved: bool,
    /// F1 or Help while its own window is open: flash it in the taskbar.
    /// It asks to be looked at; it never takes the keyboard from the app.
    attention: bool,
    /// Its own window is up (since it last opened or popped out).
    shown_own: bool,
    /// Until when (egui's clock) a just-opened window hands the keyboard
    /// back to the app once the system gives it focus. Asked to open
    /// inactive, Windows does; X11 and Wayland window managers focus a new
    /// window anyway. A click on it later is left alone.
    give_back_until: f64,
    /// The heading a link named, to bring into view: its anchor.
    heading: Option<String>,
    chapter: usize,
    /// Scroll the text back to the top, and the list to the chapter, on
    /// the next frame.
    top: bool,
    /// Find: only the chapters that mention it are listed.
    find: String,
}

impl Manual {
    /// Closed, over the app or (`own_window`) popped out, as last left.
    pub fn new(own_window: bool) -> Self {
        Self {
            s: Arc::new(Mutex::new(State {
                own_window,
                ..Default::default()
            })),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The chapter on show.
    pub fn chapter(&self) -> usize {
        self.state().chapter
    }

    /// Open the manual at chapter `i`.
    pub fn show(&mut self, i: usize) {
        let mut s = self.state();
        s.attention = s.open && s.own_window;
        s.open = true;
        s.go(i);
    }

    /// Draw the manual if it's open, over the app or in its own window.
    /// Some(popped out) when Pop out or Back in is-alive was clicked (to
    /// save it).
    pub fn window(&mut self, ctx: &egui::Context) -> Option<bool> {
        let now = ctx.input(|i| i.time);
        // Not held while drawing: the own window's callback takes it.
        let (open, own, moved) = {
            let mut s = self.state();
            if !(s.open && s.own_window) {
                s.shown_own = false;
            } else if !s.shown_own {
                s.shown_own = true;
                s.give_back_until = now + GIVE_BACK_SECS;
            }
            (s.open, s.own_window, std::mem::take(&mut s.moved))
        };
        if open && own {
            self.own(ctx);
        } else if open {
            self.over_app(ctx);
        }
        moved.then_some(own)
    }

    /// A window of its own, moved, sized and closed like any other. It
    /// opens without taking the keyboard, and draws on its own, so the app
    /// and the manual can be used side by side.
    fn own(&self, ctx: &egui::Context) {
        static ICON: OnceLock<Arc<egui::IconData>> = OnceLock::new();
        let icon = ICON.get_or_init(|| Arc::new(crate::icon())).clone();
        let builder = egui::ViewportBuilder::default()
            .with_title(format!("is-alive {} manual", crate::VERSION))
            .with_app_id("is-alive")
            .with_icon(icon)
            .with_active(false)
            .with_inner_size([940.0, 680.0])
            .with_min_inner_size([560.0, 300.0]);
        let shared = Arc::clone(&self.s);
        ctx.show_viewport_deferred(
            egui::ViewportId::from_hash_of(OWN),
            builder,
            move |ui, _| {
                let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                let ctx = ui.ctx().clone();
                if ui.input(|i| i.viewport().close_requested()) {
                    s.open = false;
                    ctx.request_repaint_of(egui::ViewportId::ROOT);
                }
                let (now, focused) = ui.input(|i| (i.time, i.viewport().focused));
                if now < s.give_back_until {
                    if focused == Some(true) {
                        s.give_back_until = 0.0;
                        ctx.send_viewport_cmd_to(
                            egui::ViewportId::ROOT,
                            egui::ViewportCommand::Focus,
                        );
                    } else {
                        ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    }
                }
                if std::mem::take(&mut s.attention) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                        egui::UserAttentionType::Informational,
                    ));
                }
                // Opaque: Miami's panels are see-through over a backdrop this
                // window doesn't have.
                let bg = ui.visuals().window_fill.to_opaque();
                ui.painter().rect_filled(ui.max_rect(), 0.0, bg);
                egui::Frame::new().inner_margin(8).show(ui, |ui| s.body(ui));
                if s.moved {
                    // Back in is-alive: the app draws it from now on.
                    ctx.request_repaint_of(egui::ViewportId::ROOT);
                }
                drop(s);
                // This window's links and copies, as the app's own go.
                widgets::open_requested_links(&ctx);
                widgets::route_copies(&ctx);
            },
        );
    }

    /// Over the app, inside its window.
    fn over_app(&self, ctx: &egui::Context) {
        let mut s = self.state();
        s.attention = false;
        let mut open = true;
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
                .show(ctx, |ui| s.body(ui));
        });
        if !open {
            s.open = false;
        }
    }
}

impl State {
    /// Chapter `i`, from its top.
    fn go(&mut self, i: usize) {
        self.chapter = i.min(chapters().len().saturating_sub(1));
        self.top = true;
        self.heading = None;
    }

    /// Follow a link: another chapter or one of its headings
    /// (`#13-speed-test`, `#bufferbloat-grades`), a web page, or a file in
    /// the repository (`../../FOLLOW-ALONG.md`), opened on GitHub.
    fn follow(&mut self, to: &str) {
        match place(to) {
            Some(Place::Chapter(i)) => self.go(i),
            Some(Place::Heading(i, anchor)) => {
                self.go(i);
                self.heading = Some(anchor);
            }
            Some(Place::Web(url)) => widgets::open_url(url),
            None => widgets::toast(format!("The manual can't open {to}"), false),
        }
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
        let mut follow = crate::markdown::Follow {
            show: self.heading.clone(),
            clicked: None,
        };
        egui::CentralPanel::no_frame().show(ui, |ui| {
            // The chapter's title stays put while its text scrolls, with
            // Pop out (or Back in is-alive) beside it.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(c.title).heading().size(22.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (label, tip) = if self.own_window {
                        ("Back in is-alive", "Put the manual back over the app")
                    } else {
                        (
                            "Pop out",
                            "Open the manual in a window of its own, to keep beside is-alive \
                             or on another screen",
                        )
                    };
                    if ui
                        .add(egui::Button::new(label).min_size(egui::vec2(0.0, widgets::TARGET_H)))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        self.own_window = !self.own_window;
                        self.moved = true;
                    }
                });
            });
            ui.add_space(4.0);
            let mut area = egui::ScrollArea::vertical()
                .id_salt("manual_text")
                .auto_shrink([false, false]);
            if top {
                area = area.vertical_scroll_offset(0.0);
            }
            area.show(ui, |ui| {
                ui.add_space(2.0);
                text(ui, c.body, &mut follow);
                ui.add_space(12.0);
            });
        });
        // Brought into view: done with it. Not found (yet): kept a frame.
        if follow.show.is_none() {
            self.heading = None;
        }
        if let Some(to) = follow.clicked {
            self.follow(&to);
        }
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

/// Where a link in the manual goes.
#[derive(Debug, PartialEq)]
enum Place {
    Chapter(usize),
    /// A heading inside a chapter, by its anchor.
    Heading(usize, String),
    Web(String),
}

/// What a link's target names: a chapter or heading (`#…`), a web page,
/// or a file in the repository (`../../FOLLOW-ALONG.md`, this version's
/// copy on GitHub). None for anything else.
fn place(to: &str) -> Option<Place> {
    if let Some(anchor) = to.strip_prefix('#') {
        let list = chapters();
        if let Some(i) = list
            .iter()
            .position(|c| crate::markdown::anchor(c.title) == anchor)
        {
            return Some(Place::Chapter(i));
        }
        let mut fence = false;
        return list.iter().enumerate().find_map(|(i, c)| {
            c.body.lines().find_map(|l| {
                let t = l.trim();
                if t.starts_with("```") {
                    fence = !fence;
                }
                let title = t.strip_prefix('#')?.trim_start_matches('#');
                (!fence && title.starts_with(' ') && crate::markdown::anchor(title) == anchor)
                    .then(|| Place::Heading(i, anchor.to_string()))
            })
        });
    }
    let lower = to.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return Some(Place::Web(to.to_string()));
    }
    // The manual is docs/manual/MANUAL.md: ../../ is the repository.
    let file = to.strip_prefix("../../")?;
    (!file.is_empty() && !file.contains("..")).then(|| {
        Place::Web(format!(
            "{}/blob/v{}/{file}",
            crate::help::REPO,
            crate::VERSION
        ))
    })
}

/// A chapter's Markdown: tables here, the rest by the Notes renderer, with
/// links to follow.
fn text(ui: &mut egui::Ui, body: &str, follow: &mut crate::markdown::Follow) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut plain = String::new();
    let flush = |ui: &mut egui::Ui, plain: &mut String, follow: &mut crate::markdown::Follow| {
        if !plain.trim().is_empty() {
            crate::markdown::render_following(ui, plain, &font, follow);
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
                flush(ui, &mut plain, follow);
                let mut rows = vec![t];
                while let Some(next) = lines.peek().map(|l| l.trim())
                    && next.starts_with('|')
                {
                    rows.push(next);
                    lines.next();
                }
                if let Some(to) = table(ui, &rows) {
                    follow.clicked = Some(to);
                }
                continue;
            }
            if t == "---" {
                continue;
            }
        }
        plain.push_str(line);
        plain.push('\n');
    }
    flush(ui, &mut plain, follow);
}

/// The cells of a table row: `| a | b |` is `["a", "b"]`.
fn cells(row: &str) -> Vec<&str> {
    let row = row.trim().trim_start_matches('|').trim_end_matches('|');
    row.split('|').map(str::trim).collect()
}

/// A table, as a striped grid; the link clicked in it, if one was.
fn table(ui: &mut egui::Ui, rows: &[&str]) -> Option<String> {
    let (head, rest) = rows.split_first()?;
    let mut clicked = None;
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
                        let (job, links) =
                            crate::markdown::inline_links(ui, cell, size, strong, &family);
                        if let (_, Some(to)) = crate::markdown::linked_label(ui, job, &links, true)
                        {
                            clicked = Some(to);
                        }
                    });
                }
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    clicked
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

    /// Every link in the manual's chapters goes somewhere: a chapter, a
    /// heading in one, a web page or a file in the repository. (Links in
    /// code, shown as examples, aren't links.)
    #[test]
    fn every_link_in_the_manual_goes_somewhere() {
        let mut checked = 0;
        for c in chapters() {
            let mut fence = false;
            for line in c.body.lines() {
                if line.trim_start().starts_with("```") {
                    fence = !fence;
                }
                if fence {
                    continue;
                }
                // Code spans out: `[text](https://)` is an example.
                let prose: String = line
                    .split('`')
                    .enumerate()
                    .filter(|(i, _)| i % 2 == 0)
                    .map(|(_, s)| s)
                    .collect();
                let mut rest = prose.as_str();
                while let Some(at) = rest.find("](") {
                    let after = &rest[at + 2..];
                    let Some(end) = after.find(')') else { break };
                    let to = &after[..end];
                    assert!(place(to).is_some(), "{}: [..]({to}) goes nowhere", c.title);
                    checked += 1;
                    rest = &after[end..];
                }
            }
        }
        assert!(checked >= 40, "only {checked} links found");
    }

    #[test]
    fn links_go_to_chapters_headings_and_the_web() {
        let speed = chapter_named("Speed Test");
        assert_eq!(place("#13-speed-test"), Some(Place::Chapter(speed)));
        assert_eq!(
            place("#bufferbloat-grades"),
            Some(Place::Heading(speed, "bufferbloat-grades".into()))
        );
        assert_eq!(
            place("https://github.com/hammerknife404/is-alive/wiki"),
            Some(Place::Web(
                "https://github.com/hammerknife404/is-alive/wiki".into()
            ))
        );
        assert_eq!(
            place("../../FOLLOW-ALONG.md"),
            Some(Place::Web(format!(
                "{}/blob/v{}/FOLLOW-ALONG.md",
                crate::help::REPO,
                crate::VERSION
            )))
        );
        assert_eq!(place("#no-such-heading"), None);
        assert_eq!(place("file:///etc/passwd"), None);
        assert_eq!(place("../../../../etc/passwd"), None);
    }

    #[test]
    fn table_cells() {
        assert_eq!(cells("| **Host** | A name |"), ["**Host**", "A name"]);
        assert_eq!(cells("|---|---|"), ["---", "---"]);
    }
}
