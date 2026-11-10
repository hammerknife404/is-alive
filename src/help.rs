//! Help: the nav's Help menu. The manual built into the app comes
//! first (F1 opens it at the tab on screen); then the wiki, the follow-along
//! guide, release notes and where to report a problem, online.

use crate::{Tab, VERSION};
use eframe::egui;

const REPO: &str = "https://github.com/hammerknife404/is-alive";
/// The wiki's front page.
pub const WIKI: &str = "https://github.com/hammerknife404/is-alive/wiki";

/// A new GitHub issue with this version and system filled in. Opening it
/// sends nothing: the browser shows the form, and only Submit does.
pub fn new_issue() -> String {
    let os = match std::env::consts::OS {
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    let body = format!(
        "**Version:** is-alive v{VERSION} on {os}\n\n\
         **What happened:**\n\n\
         **What you expected:**\n\n\
         **Steps to see it again:**\n"
    );
    format!("{REPO}/issues/new?body={}", encode(&body))
}

/// Percent-encodes everything but RFC 3986's unreserved characters.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Where an entry goes: a chapter of the built-in manual, or a page online.
#[derive(Clone, Debug, PartialEq)]
pub enum Go {
    Chapter(usize),
    Web(String),
}

/// One entry: its name, a line on what's there, where it goes.
struct Link {
    title: String,
    about: &'static str,
    go: Go,
    key: Option<&'static str>,
}

/// The menu's entries, in groups under a heading: the manual built into
/// the app first (it works with no network), then pages online.
fn links(tab: Tab, tab_name: &str) -> [(&'static str, Vec<Link>); 2] {
    use crate::manual::{chapter_named, chapter_of};
    let link = |title: &str, about, go| Link {
        title: title.to_string(),
        about,
        go,
        key: None,
    };
    let web = |title: &str, about, url: String| link(title, about, Go::Web(url));
    [
        (
            "In the app · works offline",
            vec![
                Link {
                    key: Some("F1"),
                    ..link(
                        &format!("This tab: {tab_name}"),
                        "Its chapter in the manual: controls, results, examples",
                        Go::Chapter(chapter_of(tab)),
                    )
                },
                link(
                    "Manual",
                    "Every tab, the Site Bundle, privacy and a glossary",
                    Go::Chapter(0),
                ),
                link(
                    "Troubleshooting",
                    "When something doesn't look right",
                    Go::Chapter(chapter_named("Troubleshooting")),
                ),
                link(
                    "Keyboard and mouse",
                    "Every shortcut, and what dragging and dropping do",
                    Go::Chapter(chapter_named("Keyboard and mouse")),
                ),
            ],
        ),
        (
            "Online · opens in the browser",
            vec![
                web(
                    "Wiki",
                    "A shorter page per tab, and the guides",
                    WIKI.to_string(),
                ),
                web(
                    "Follow-along guide",
                    "A 20-minute walk-through for people new to networking",
                    format!("{REPO}/blob/v{VERSION}/FOLLOW-ALONG.md"),
                ),
                web(
                    "Release notes",
                    "What changed in each version, and its downloads",
                    crate::update::RELEASES.to_string(),
                ),
                web(
                    "Report a problem",
                    "A new GitHub issue with this version filled in",
                    new_issue(),
                ),
                web(
                    "Source on GitHub",
                    "The code, the licence and the README",
                    REPO.to_string(),
                ),
            ],
        ),
    ]
}

/// The Help menu, opened from the nav. Opens a page online itself;
/// returns the manual's chapter to show.
pub fn menu(ui: &mut egui::Ui, tab: Tab, tab_name: &str) -> Option<usize> {
    ui.set_min_width(380.0);
    let mut chapter = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Help").strong());
        ui.weak(format!("v{VERSION}"));
    });
    for (heading, group) in links(tab, tab_name) {
        ui.separator();
        ui.weak(egui::RichText::new(heading).small());
        for l in group {
            let mut job = egui::text::LayoutJob::default();
            let style = ui.style().clone();
            egui::RichText::new(&l.title)
                .color(ui.visuals().strong_text_color())
                .append_to(
                    &mut job,
                    &style,
                    egui::FontSelection::Default,
                    egui::Align::Center,
                );
            egui::RichText::new(format!("\n{}", l.about))
                .small()
                .color(ui.visuals().weak_text_color())
                .append_to(
                    &mut job,
                    &style,
                    egui::FontSelection::Default,
                    egui::Align::Center,
                );
            let mut b = egui::Button::new(job);
            if let Some(k) = l.key {
                b = b.shortcut_text(egui::RichText::new(k).weak());
            }
            let hover = match &l.go {
                Go::Chapter(i) => format!(
                    "Opens the manual at {}",
                    crate::manual::chapters()
                        .get(*i)
                        .map_or("its start", |c| c.title)
                ),
                Go::Web(u) if u.contains("/issues/new") => {
                    "Opens the form on GitHub. Nothing is sent until you press Submit there."
                        .to_string()
                }
                Go::Web(u) => u.clone(),
            };
            if ui.add(b).on_hover_text(hover).clicked() {
                match l.go {
                    Go::Chapter(i) => chapter = Some(i),
                    Go::Web(u) => ui.ctx().open_url(egui::OpenUrl::new_tab(u)),
                }
                ui.close();
            }
        }
    }
    chapter
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_link_stays_on_the_project() {
        for group in links(Tab::Map, "Map") {
            for l in group.1 {
                if let Go::Web(u) = l.go {
                    assert!(u.starts_with(REPO), "{u}");
                }
            }
        }
    }

    #[test]
    fn a_new_issue_carries_the_version_and_nothing_else_of_this_machine() {
        let url = new_issue();
        let (base, query) = url.split_once('?').unwrap();
        assert_eq!(base, format!("{REPO}/issues/new"));
        let body = query.strip_prefix("body=").unwrap();
        // Only unreserved characters and escapes: no raw space, & or #.
        assert!(
            body.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.~%".contains(&b))
        );
        assert!(body.contains(&encode(&format!("v{VERSION}"))));
    }

    #[test]
    fn encodes_utf8_and_reserved_characters() {
        assert_eq!(encode("a b&c#d"), "a%20b%26c%23d");
        assert_eq!(encode("…"), "%E2%80%A6");
        assert_eq!(encode("v0.26.0-x_y~"), "v0.26.0-x_y~");
    }
}
