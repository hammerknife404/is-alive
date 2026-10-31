//! Just enough Markdown for tech notes, drawn with egui: headings, lists,
//! task boxes, quotes, code blocks, rules, and **bold**, *italic*, `code`
//! and [links](…) inline. Links are shown, not followed. Redaction markers
//! are picked out so it's plain what was removed.

use eframe::egui::{self, FontFamily, FontId, RichText, Stroke, TextFormat, Ui, text::LayoutJob};

/// Inline spans of one line, styled.
fn inline(ui: &Ui, text: &str, size: f32, strong: bool, family: &FontFamily) -> LayoutJob {
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
    let push = |job: &mut LayoutJob, s: &str, bold: bool, italic: bool| {
        if s.is_empty() {
            return;
        }
        let mut f = base.clone();
        if bold {
            f.color = vis.strong_text_color();
        }
        f.italics = italic;
        crate::theme::append_with_emoji(job, s, &f);
    };
    while !rest.is_empty() {
        let next = rest
            .char_indices()
            .find(|(_, c)| matches!(c, '*' | '_' | '`' | '['))
            .map(|(i, _)| i);
        let Some(i) = next else {
            push(&mut job, rest, bold, italic);
            break;
        };
        push(&mut job, &rest[..i], bold, italic);
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix("**") {
            bold = !bold;
            rest = r;
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
            job.append(&rest[1..mid], 0.0, f);
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
            push(&mut job, &rest[..1], bold, italic);
            rest = &rest[1..];
        }
    }
    job
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
/// of a task box that was clicked.
pub fn render(ui: &mut Ui, text: &str, font: &FontId) -> Option<usize> {
    let mut toggled = None;
    let body = font.size;
    let mut lines = text.lines().enumerate().peekable();
    while let Some((n, line)) = lines.next() {
        let t = line.trim_start();
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
            ui.label(inline(ui, t[hashes..].trim(), size, true, &font.family));
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
                ui.label(inline(ui, q.trim(), body, false, &font.family));
            });
            continue;
        }
        if let Some((indent, it)) = item(line) {
            ui.horizontal_top(|ui| {
                ui.add_space(8.0 + indent as f32 * 6.0);
                match it {
                    Item::Bullet(r) => {
                        ui.label(RichText::new("•").size(body).color(crate::theme::accent()));
                        ui.label(inline(ui, r, body, false, &font.family));
                    }
                    Item::Number(num, r) => {
                        ui.label(RichText::new(num).size(body).color(crate::theme::accent()));
                        ui.label(inline(ui, r, body, false, &font.family));
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
                        ui.label(job);
                    }
                }
            });
            continue;
        }
        ui.label(inline(ui, line, body, false, &font.family));
    }
    toggled
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
