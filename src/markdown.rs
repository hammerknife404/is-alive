//! Just enough Markdown for tech notes, drawn with egui: headings, lists,
//! task boxes, quotes, code blocks, rules, and **bold**, *italic*, `code`
//! and [links](…) inline. In Notes links are shown, not followed; the
//! manual follows them (`render_following`). Redaction markers are picked
//! out so it's plain what was removed.

use eframe::egui::{self, FontFamily, FontId, RichText, Stroke, TextFormat, Ui, text::LayoutJob};

/// (emoji, Markdown shortcode, what it's for).
pub type Shortcode = (&'static str, &'static str, &'static str);

/// The emoji picker's emoji by group, with their shortcodes (GitHub's
/// names, where GitHub has one). Preview draws `:fire:` as the emoji.
pub const SHORTCODES: &[(&str, &[Shortcode])] = &[
    (
        "Status",
        &[
            ("✅", "white_check_mark", "Done / OK"),
            ("☑", "ballot_box_with_check", "Ticked / approved"),
            ("❌", "x", "Failed / no"),
            ("⚠", "warning", "Warning"),
            ("❗", "exclamation", "Important"),
            ("❓", "question", "Question / unknown"),
            ("🚫", "no_entry_sign", "Blocked / not allowed"),
            ("⏳", "hourglass_flowing_sand", "Waiting / in progress"),
            ("🔄", "arrows_counterclockwise", "Restart / sync / retry"),
            ("🆗", "ok", "OK"),
            ("🚧", "construction", "Work in progress"),
            ("🐛", "bug", "Bug"),
            ("🔥", "fire", "Fire / urgent"),
        ],
    ),
    (
        "People",
        &[
            ("👍", "+1", "Thumbs up"),
            ("👎", "-1", "Thumbs down"),
            ("🙏", "pray", "Thanks / please"),
            ("🎉", "tada", "Celebrate / fixed"),
            ("☺", "relaxed", "Smile"),
            ("💬", "speech_balloon", "Said / comment"),
            ("❤", "heart", "Love"),
        ],
    ),
    (
        "Notes",
        &[
            ("⭐", "star", "Star"),
            ("✨", "sparkles", "New"),
            ("💡", "bulb", "Idea"),
            ("📌", "pushpin", "Pinned"),
            ("📝", "memo", "Note"),
            ("📋", "clipboard", "Checklist / clipboard"),
            ("📎", "paperclip", "Attachment"),
            ("🗑", "wastebasket", "Remove / delete"),
            ("📦", "package", "Package / install"),
            ("📅", "date", "Date"),
            ("⏰", "alarm_clock", "Time / reminder"),
            ("🔒", "lock", "Locked"),
            ("🔓", "unlock", "Unlocked"),
            ("🔑", "key", "Key / credential"),
            ("🔧", "wrench", "Fix / config"),
            ("🔍", "mag", "Look into / search"),
        ],
    ),
    (
        "Tech",
        &[
            ("🗄", "file_cabinet", "Server rack"),
            ("🛜", "wireless", "Wi-Fi"),
            ("🖧", "network", "Network / LAN"),
            ("🌐", "globe_with_meridians", "Internet / web"),
            ("☁", "cloud", "Cloud"),
            ("🛡", "shield", "Firewall / security"),
            ("📶", "signal_strength", "Signal strength"),
            ("📡", "satellite", "Antenna / wireless link"),
            ("🖥", "desktop_computer", "Desktop"),
            ("💻", "computer", "Laptop"),
            ("📱", "iphone", "Phone"),
            ("🖶", "printer", "Printer"),
            ("📠", "fax", "Fax / printer"),
            ("📞", "telephone_receiver", "Call"),
            ("📹", "video_camera", "Camera / CCTV"),
            ("⌨", "keyboard", "Keyboard"),
            ("🖱", "computer_mouse", "Mouse"),
            ("🔌", "electric_plug", "Power / cable"),
            ("🔋", "battery", "Battery / UPS"),
            ("💾", "floppy_disk", "Save / backup"),
            ("🖴", "hard_disk", "Disk / storage"),
        ],
    ),
];

/// Every picker emoji as (emoji, shortcode, what it's for).
pub fn shortcodes() -> impl Iterator<Item = &'static Shortcode> {
    SHORTCODES.iter().flat_map(|g| g.1)
}

/// The emoji for the shortcode `rest` starts with, and the code's length.
pub(crate) fn shortcode(rest: &str) -> Option<(&'static str, usize)> {
    let end = rest[1..].find(':')? + 2;
    let code = &rest[1..end - 1];
    shortcodes()
        .find(|(_, c, _)| *c == code)
        .map(|(e, ..)| (*e, end))
}

/// A link in a laid-out line: the characters it covers (not bytes, as a
/// galley's cursors count) and where it goes, as written.
pub type Links = Vec<(std::ops::Range<usize>, String)>;

/// Inline spans of one line, styled.
pub(crate) fn inline(
    ui: &Ui,
    text: &str,
    size: f32,
    strong: bool,
    family: &FontFamily,
) -> LayoutJob {
    inline_links(ui, text, size, strong, family).0
}

/// `inline`, and where each link in it is and goes.
pub(crate) fn inline_links(
    ui: &Ui,
    text: &str,
    size: f32,
    strong: bool,
    family: &FontFamily,
) -> (LayoutJob, Links) {
    let mut links = Links::new();
    let vis = ui.visuals();
    let body = if strong {
        vis.strong_text_color()
    } else {
        vis.text_color()
    };
    let base = TextFormat::simple(FontId::new(size, family.clone()), body);
    let mut job = LayoutJob::default();
    let mut rest = text;
    let mut bold = false;
    let mut italic = false;
    let mut strike = false;
    let push = |job: &mut LayoutJob, s: &str, bold: bool, italic: bool, strike: bool| {
        if s.is_empty() {
            return;
        }
        let mut f = base.clone();
        if bold {
            f.color = vis.strong_text_color();
        }
        f.italics = italic;
        if strike {
            f.strikethrough = Stroke::new(1.0, f.color);
        }
        crate::theme::append_with_emoji(job, s, &f);
    };
    while !rest.is_empty() {
        let next = rest
            .char_indices()
            .find(|(_, c)| matches!(c, '*' | '_' | '`' | '[' | ':' | '~'))
            .map(|(i, _)| i);
        let Some(i) = next else {
            push(&mut job, rest, bold, italic, strike);
            break;
        };
        push(&mut job, &rest[..i], bold, italic, strike);
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix("**") {
            bold = !bold;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("~~") {
            strike = !strike;
            rest = r;
        } else if rest.starts_with(':')
            && let Some((emoji, len)) = shortcode(rest)
        {
            push(&mut job, emoji, bold, italic, strike);
            rest = &rest[len..];
        } else if let Some(r) = rest.strip_prefix('`')
            && let Some(end) = r.find('`')
        {
            let mut f = TextFormat::simple(FontId::monospace(size * 0.92), body);
            f.background = vis.code_bg_color;
            job.append(&r[..end], 0.0, f);
            rest = &r[end + 1..];
        } else if (rest.starts_with("[🔑 REDACTED: ") || rest.starts_with("[REDACTED: "))
            && let Some(end) = rest.find(']')
        {
            let warn = crate::widgets::warn();
            let mut f = TextFormat::simple(FontId::new(size * 0.9, family.clone()), warn);
            f.background = warn.gamma_multiply(0.15);
            job.append(&rest[..=end], 0.0, f);
            rest = &rest[end + 1..];
        } else if rest.starts_with('[')
            && let Some(mid) = rest.find("](")
            && let Some(end) = rest[mid..].find(')')
        {
            let mut f = base.clone();
            f.color = vis.hyperlink_color;
            f.underline = Stroke::new(1.0, vis.hyperlink_color);
            let from = job.text.chars().count();
            job.append(&rest[1..mid], 0.0, f);
            let to = rest[mid + 2..mid + end].to_string();
            links.push((from..job.text.chars().count(), to));
            rest = &rest[mid + end + 1..];
        } else if (rest.starts_with('*') || rest.starts_with('_'))
            // A lone * or _ before a space isn't emphasis.
            && (italic || rest[1..].starts_with(|c: char| !c.is_whitespace()))
            // An _ inside a word (snake_case, IS_ALIVE_GPU) isn't either.
            && (rest.starts_with('*') || {
                let prev = text[..text.len() - rest.len()].chars().next_back();
                let next = rest[1..].chars().next();
                let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
                if italic { !word(next) } else { !word(prev) }
            })
        {
            italic = !italic;
            rest = &rest[1..];
        } else {
            push(&mut job, &rest[..1], bold, italic, strike);
            rest = &rest[1..];
        }
    }
    (job, links)
}

/// A label whose links can be clicked: the pointer turns to a hand over
/// one, a web link's address shows on hover, and the clicked link's
/// target comes back with the label's response. Text stays selectable,
/// as other labels are.
pub fn linked_label(
    ui: &mut Ui,
    job: LayoutJob,
    links: &[(std::ops::Range<usize>, String)],
    wrap: bool,
) -> (egui::Response, Option<String>) {
    let label = egui::Label::new(job);
    let label = if wrap { label.wrap() } else { label };
    if links.is_empty() {
        return (ui.add(label), None);
    }
    let (pos, galley, response) = label.sense(egui::Sense::click()).layout_in_ui(ui);
    let hit = response.hover_pos().and_then(|p| {
        let rel = p - pos;
        let c = galley.cursor_from_pos(rel);
        // The cursor sits between characters: the one under the pointer
        // is before it when the pointer is left of it.
        let k = if galley.pos_from_cursor(c).min.x > rel.x {
            c.index.0.saturating_sub(1)
        } else {
            c.index.0
        };
        links
            .iter()
            .find(|(r, _)| r.contains(&k))
            .map(|(_, to)| to.clone())
    });
    let colour = ui.visuals().text_color();
    egui::text_selection::LabelSelectionState::label_text_selection(
        ui,
        &response,
        pos,
        galley,
        colour,
        Stroke::NONE,
    );
    let Some(to) = hit else {
        return (response, None);
    };
    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    let clicked = response.clicked();
    let response = if to.starts_with("http") {
        response.on_hover_text(to.as_str())
    } else {
        response
    };
    (response, clicked.then_some(to))
}

/// A heading's anchor as GitHub makes it: "13. Speed Test" is
/// "13-speed-test", "The inspector (right)" is "the-inspector-right".
pub fn anchor(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// For a renderer that follows links (the manual): the heading to bring
/// into view, by its anchor, and the link clicked this frame.
#[derive(Default)]
pub struct Follow {
    pub show: Option<String>,
    pub clicked: Option<String>,
}

/// List marker and the rest: ("•", "text"), ("3.", "text") or a task box.
enum Item<'a> {
    Bullet(&'a str),
    Number(&'a str, &'a str),
    Task(bool, &'a str),
}

fn item(line: &str) -> Option<(usize, Item<'_>)> {
    let indent = line.len() - line.trim_start().len();
    let t = line.trim_start();
    for m in ["- ", "* ", "+ "] {
        if let Some(r) = t.strip_prefix(m) {
            for (b, done) in [("[ ] ", false), ("[x] ", true), ("[X] ", true)] {
                if let Some(r) = r.strip_prefix(b) {
                    return Some((indent, Item::Task(done, r)));
                }
            }
            return Some((indent, Item::Bullet(r)));
        }
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && t[digits..].starts_with(". ") {
        return Some((indent, Item::Number(&t[..=digits], &t[digits + 2..])));
    }
    None
}

/// Draw `text` in `font` (headings scale from its size). Returns the line
/// of a task box that was clicked. Links are shown, not followed.
pub fn render(ui: &mut Ui, text: &str, font: &FontId) -> Option<usize> {
    render_in(ui, text, font, None)
}

/// `render` with links that can be clicked (`follow.clicked`), bringing
/// the heading `follow.show` names into view.
pub fn render_following(ui: &mut Ui, text: &str, font: &FontId, follow: &mut Follow) {
    render_in(ui, text, font, Some(follow));
}

/// One line of inline text: a plain label, or one whose links can be
/// clicked when they're followed.
fn say(
    ui: &mut Ui,
    follow: &mut Option<&mut Follow>,
    text: &str,
    size: f32,
    strong: bool,
    family: &FontFamily,
    wrap: bool,
) -> egui::Response {
    match follow {
        Some(f) => {
            let (job, links) = inline_links(ui, text, size, strong, family);
            let (r, to) = linked_label(ui, job, &links, wrap);
            if to.is_some() {
                f.clicked = to;
            }
            r
        }
        None => {
            let job = inline(ui, text, size, strong, family);
            if wrap {
                ui.add(egui::Label::new(job).wrap())
            } else {
                ui.label(job)
            }
        }
    }
}

fn render_in(
    ui: &mut Ui,
    text: &str,
    font: &FontId,
    mut follow: Option<&mut Follow>,
) -> Option<usize> {
    let mut toggled = None;
    let body = font.size;
    let mut lines = text.lines().enumerate().peekable();
    while let Some((n, line)) = lines.next() {
        let t = line.trim_start();
        if crate::photos::render(ui, t) {
            continue;
        }
        if t.starts_with("```") {
            let mut code = Vec::new();
            for (_, l) in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
                code.push(l);
            }
            egui::Frame::new()
                .fill(ui.visuals().code_bg_color)
                .corner_radius(4)
                .inner_margin(6)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(code.join("\n")).monospace().size(body * 0.95));
                });
            continue;
        }
        if t.starts_with('|') {
            let mut rows = vec![t];
            while let Some((_, next)) = lines.peek()
                && next.trim_start().starts_with('|')
            {
                rows.push(next.trim_start());
                lines.next();
            }
            table(ui, &rows, font, &mut follow);
            continue;
        }
        if t.is_empty() {
            ui.add_space(body * 0.5);
            continue;
        }
        if t.chars().all(|c| c == '-' || c == '*' || c == '_') && t.len() >= 3 {
            ui.separator();
            continue;
        }
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            let size = body * [1.6, 1.35, 1.15, 1.05, 1.0, 1.0][hashes - 1];
            ui.add_space(body * 0.3);
            let title = t[hashes..].trim();
            let r = say(ui, &mut follow, title, size, true, &font.family, false);
            if let Some(f) = follow.as_deref_mut()
                && f.show.as_deref() == Some(anchor(title).as_str())
            {
                r.scroll_to_me(Some(egui::Align::TOP));
                f.show = None;
            }
            if hashes <= 2 {
                let r = ui.min_rect();
                let y = ui.cursor().top();
                ui.painter().hline(
                    r.left()..=r.right(),
                    y,
                    Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
                );
                ui.add_space(3.0);
            }
            continue;
        }
        if let Some(q) = t.strip_prefix('>') {
            ui.horizontal(|ui| {
                let (r, _) =
                    ui.allocate_exact_size(egui::vec2(3.0, body * 1.3), egui::Sense::hover());
                ui.painter()
                    .rect_filled(r, 1, crate::theme::accent().gamma_multiply(0.6));
                say(ui, &mut follow, q.trim(), body, false, &font.family, true);
            });
            continue;
        }
        if let Some((indent, it)) = item(line) {
            ui.horizontal_top(|ui| {
                ui.add_space(8.0 + indent as f32 * 6.0);
                match it {
                    Item::Bullet(r) => {
                        ui.label(RichText::new("•").size(body).color(crate::theme::accent()));
                        say(ui, &mut follow, r, body, false, &font.family, true);
                    }
                    Item::Number(num, r) => {
                        ui.label(RichText::new(num).size(body).color(crate::theme::accent()));
                        say(ui, &mut follow, r, body, false, &font.family, true);
                    }
                    Item::Task(done, r) => {
                        let mut d = done;
                        if ui.checkbox(&mut d, "").clicked() {
                            toggled = Some(n);
                        }
                        let mut job = inline(ui, r, body, false, &font.family);
                        if done {
                            for s in &mut job.sections {
                                s.format.strikethrough =
                                    Stroke::new(1.0, ui.visuals().weak_text_color());
                                s.format.color = ui.visuals().weak_text_color();
                            }
                        }
                        // In a row, a label runs on unless told to wrap.
                        ui.add(egui::Label::new(job).wrap());
                    }
                }
            });
            continue;
        }
        say(ui, &mut follow, line, body, false, &font.family, false);
    }
    toggled
}

/// The cells of a table row: `| a | b |` is `["a", "b"]`.
fn cells(row: &str) -> Vec<&str> {
    let row = row.trim().trim_start_matches('|').trim_end_matches('|');
    row.split('|').map(str::trim).collect()
}

/// A Markdown table as a striped grid: the first row as its header, the
/// `|---|` row left out, text wrapped to share the width.
fn table(ui: &mut Ui, rows: &[&str], font: &FontId, follow: &mut Option<&mut Follow>) {
    let Some((head, rest)) = rows.split_first() else {
        return;
    };
    let head = cells(head);
    let body: Vec<Vec<&str>> = rest
        .iter()
        .map(|r| cells(r))
        .filter(|c| {
            !c.iter()
                .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':')))
        })
        .collect();
    let n = head.len().max(1);
    let gap = 12.0;
    // Kept clear of the edge: a table as wide as its panel would widen it.
    let w = (ui.available_width() - gap * (n as f32 - 1.0) - 24.0).max(40.0 * n as f32);
    let widths: Vec<f32> = if n == 2 {
        vec![w * 0.3, w * 0.7]
    } else {
        vec![w / n as f32; n]
    };
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
                        say(ui, follow, cell, font.size, strong, &font.family, true);
                    });
                }
                ui.end_row();
            }
        });
    ui.add_space(6.0);
}

/// Flip the task box on `line` of `text`.
pub fn toggle_task(text: &mut String, line: usize) {
    let mut out = String::with_capacity(text.len());
    for (i, l) in text.split_inclusive('\n').enumerate() {
        if i == line {
            if let Some(p) = l.find("[ ]") {
                out += &format!("{}[x]{}", &l[..p], &l[p + 3..]);
                continue;
            }
            if let Some(p) = l.find("[x]").or_else(|| l.find("[X]")) {
                out += &format!("{}[ ]{}", &l[..p], &l[p + 3..]);
                continue;
            }
        }
        out += l;
    }
    *text = out;
}

/// What a new line after `line` starts with: the list continues, or None.
/// An empty item ends the list (Some("") clears the marker).
pub fn continue_list(line: &str) -> Option<String> {
    let (indent, it) = item(line)?;
    let pad = " ".repeat(indent);
    let (marker, rest) = match it {
        Item::Bullet(r) => (line.trim_start()[..2].to_string(), r),
        Item::Task(_, r) => (format!("{}[ ] ", &line.trim_start()[..2]), r),
        Item::Number(n, r) => {
            let next = n.trim_end_matches('.').parse::<u64>().map_or(1, |v| v + 1);
            (format!("{next}. "), r)
        }
    };
    if rest.trim().is_empty() {
        Some(String::new())
    } else {
        Some(format!("{pad}{marker}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A link in a followed line can be clicked, and goes where it says;
    /// the plain text beside it doesn't.
    #[test]
    fn a_followed_link_is_clicked_by_its_text() {
        let ctx = egui::Context::default();
        let text = "Plain words first, then [the Speed Test](#13-speed-test) after.";
        let frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput {
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 200.0),
                )),
                time: Some(ctx.input(|i| i.time) + 0.5),
                ..Default::default()
            };
            let mut follow = Follow::default();
            let mut out = ctx.run_ui(input, |ui| {
                render_following(ui, text, &FontId::proportional(14.0), &mut follow);
            });
            out.textures_delta.clear();
            let mut row = egui::Rect::NOTHING;
            for c in &out.shapes {
                if let egui::Shape::Text(t) = &c.shape {
                    row = t.visual_bounding_rect();
                }
            }
            (row, follow.clicked)
        };
        let click = |at: egui::Pos2| {
            let press = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(vec![egui::Event::PointerMoved(at)]);
            frame(vec![press(true)]);
            frame(vec![press(false)]).1
        };
        let (row, _) = frame(vec![]);
        assert!(row.is_positive());
        // "the Speed Test" sits from about half to four fifths along;
        // "Plain" at the start.
        let at = |share: f32| egui::pos2(row.left() + row.width() * share, row.center().y);
        assert_eq!(click(at(0.65)), Some("#13-speed-test".to_string()));
        assert_eq!(click(at(0.05)), None);
    }

    #[test]
    fn anchors_are_githubs() {
        assert_eq!(anchor("13. Speed Test"), "13-speed-test");
        assert_eq!(anchor("The inspector (right)"), "the-inspector-right");
        assert_eq!(anchor("The .exe doesn't open"), "the-exe-doesnt-open");
        assert_eq!(
            anchor("9. Advanced (mail and domain health)"),
            "9-advanced-mail-and-domain-health"
        );
    }

    #[test]
    fn draws_the_format_test_file() {
        // assets/notes-format-test.md: every element, drawn without a panic.
        let text = include_str!("../assets/notes-format-test.md");
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(render(ui, text, &FontId::proportional(13.0)), None);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn shortcodes_become_emoji() {
        assert_eq!(shortcode(":fire: hot"), Some(("🔥", 6)));
        assert_eq!(shortcode(":+1:"), Some(("👍", 4)));
        assert_eq!(shortcode(":nope:"), None);
        assert_eq!(shortcode("10:30"), None);
        assert_eq!(shortcode(":"), None);
        // Every picker emoji has one code, and no code is used twice.
        let mut codes: Vec<_> = shortcodes().map(|s| s.1).collect();
        let n = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), n);
        let mut emoji: Vec<_> = shortcodes().map(|s| s.0).collect();
        emoji.sort_unstable();
        emoji.dedup();
        assert_eq!(emoji.len(), n, "an emoji is in the picker twice");
        assert_eq!(shortcode(":wireless:"), Some(("🛜", 10)));
    }

    #[test]
    fn underscores_inside_words_are_literal() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let text = |t: &str| inline(ui, t, 14.0, false, &FontFamily::Proportional).text;
            assert_eq!(text("IS_ALIVE_GPU=off"), "IS_ALIVE_GPU=off");
            assert_eq!(text("snake_case_name"), "snake_case_name");
            // Real emphasis still works, and drops its markers.
            assert_eq!(text("an _italic_ word"), "an italic word");
            assert_eq!(text("**bold** and *it*"), "bold and it");
        });
        out.textures_delta.clear();
    }

    #[test]
    fn task_toggles_only_that_line() {
        let mut t = "- [ ] a\n- [x] b\n- [ ] c".to_string();
        toggle_task(&mut t, 1);
        assert_eq!(t, "- [ ] a\n- [ ] b\n- [ ] c");
        toggle_task(&mut t, 2);
        assert_eq!(t, "- [ ] a\n- [ ] b\n- [x] c");
    }

    #[test]
    fn lists_continue_and_end() {
        assert_eq!(continue_list("- swap PSU").as_deref(), Some("- "));
        assert_eq!(continue_list("  * nested").as_deref(), Some("  * "));
        assert_eq!(continue_list("- [x] done").as_deref(), Some("- [ ] "));
        assert_eq!(continue_list("9. nine").as_deref(), Some("10. "));
        assert_eq!(continue_list("- ").as_deref(), Some(""));
        assert_eq!(continue_list("plain text"), None);
    }
}
