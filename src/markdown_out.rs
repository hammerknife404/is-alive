//! Notes out of the app in other formats: a web page (HTML) for a client or
//! to print as PDF, and plain text for a ticket. Both follow the rules the
//! Preview draws by (markdown.rs): headings, lists, task boxes, quotes,
//! code blocks, rules, tables, photos, and **bold**, *italic*, ~~struck~~,
//! `code` and [links](…) inline. Callers pass the redacted text.
//!
//! The HTML is built here, every character of the notes escaped: nothing
//! in them becomes markup or script. Links are kept only for http, https
//! and mailto; photos only as the PNG data the notes already hold.

use crate::markdown::shortcode;

/// The notes as a web page of their own, styled to read and print well.
pub fn to_html(text: &str, title: &str) -> String {
    let mut body = String::new();
    let mut list: Option<&'static str> = None;
    let close_list = |body: &mut String, list: &mut Option<&'static str>| {
        if let Some(tag) = list.take() {
            body.push_str(&format!("</{tag}>\n"));
        }
    };
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let t = line.trim_start();
        if t.starts_with("```") {
            close_list(&mut body, &mut list);
            let mut code = Vec::new();
            for l in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
                code.push(escape(l));
            }
            body.push_str(&format!("<pre><code>{}</code></pre>\n", code.join("\n")));
            continue;
        }
        if let Some(src) = photo(t) {
            close_list(&mut body, &mut list);
            body.push_str(&format!("<p><img alt=\"Photo\" src=\"{src}\"></p>\n"));
            continue;
        }
        if t.starts_with('|') {
            close_list(&mut body, &mut list);
            let mut rows = vec![t];
            while let Some(next) = lines.peek().map(|l| l.trim_start())
                && next.starts_with('|')
            {
                rows.push(next);
                lines.next();
            }
            body.push_str(&table_html(&rows));
            continue;
        }
        if t.is_empty() {
            close_list(&mut body, &mut list);
            continue;
        }
        if is_rule(t) {
            close_list(&mut body, &mut list);
            body.push_str("<hr>\n");
            continue;
        }
        if let Some((level, h)) = heading(t) {
            close_list(&mut body, &mut list);
            body.push_str(&format!("<h{level}>{}</h{level}>\n", inline_html(h)));
            continue;
        }
        if let Some(q) = t.strip_prefix('>') {
            close_list(&mut body, &mut list);
            body.push_str(&format!(
                "<blockquote>{}</blockquote>\n",
                inline_html(q.trim())
            ));
            continue;
        }
        if let Some(item) = list_item(t) {
            let tag = if matches!(item, Item::Number(..)) {
                "ol"
            } else {
                "ul"
            };
            if list != Some(tag) {
                close_list(&mut body, &mut list);
                body.push_str(&format!("<{tag}>\n"));
                list = Some(tag);
            }
            body.push_str(&match item {
                Item::Bullet(r) | Item::Number(_, r) => format!("<li>{}</li>\n", inline_html(r)),
                Item::Task(done, r) => format!(
                    "<li class=\"task\"><input type=\"checkbox\" disabled{}> {}</li>\n",
                    if done { " checked" } else { "" },
                    inline_html(r)
                ),
            });
            continue;
        }
        close_list(&mut body, &mut list);
        body.push_str(&format!("<p>{}</p>\n", inline_html(line.trim())));
    }
    close_list(&mut body, &mut list);
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        escape(title)
    )
}

const STYLE: &str = "\
body{font:15px/1.55 system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;color:#1f2328;\
max-width:820px;margin:32px auto;padding:0 20px}\
h1,h2{border-bottom:1px solid #d0d7de;padding-bottom:.25em}\
code,pre{font-family:ui-monospace,Consolas,'DejaVu Sans Mono',monospace;background:#f3f4f6;\
border-radius:4px}code{padding:.1em .3em}pre{padding:10px;overflow:auto}pre code{padding:0}\
blockquote{margin:0;padding:0 1em;color:#59636e;border-left:3px solid #d0d7de}\
table{border-collapse:collapse;margin:.6em 0}th,td{border:1px solid #d0d7de;padding:4px 10px;\
text-align:left;vertical-align:top}th{background:#f6f8fa}\
li.task{list-style:none;margin-left:-1.2em}img{max-width:100%;border-radius:6px}\
.redacted{background:#fff3cd;color:#7a5600;border-radius:3px;padding:0 .2em}\
@media print{body{margin:0;max-width:none}}";

/// The notes as plain text: the Markdown marks taken out, headings
/// underlined, tables lined up. For tickets and email.
pub fn to_text(text: &str) -> String {
    let mut out = String::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let t = line.trim_start();
        if t.starts_with("```") {
            for l in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
                out.push_str("    ");
                out.push_str(l);
                out.push('\n');
            }
            continue;
        }
        if photo(t).is_some() || t.starts_with("![Photo") {
            out.push_str("[Photo]\n");
            continue;
        }
        if t.starts_with('|') {
            let mut rows = vec![t];
            while let Some(next) = lines.peek().map(|l| l.trim_start())
                && next.starts_with('|')
            {
                rows.push(next);
                lines.next();
            }
            out.push_str(&table_text(&rows));
            continue;
        }
        if is_rule(t) {
            out.push_str(&"-".repeat(40));
            out.push('\n');
            continue;
        }
        if let Some((level, h)) = heading(t) {
            let h = inline_text(h);
            out.push_str(&h);
            out.push('\n');
            if level <= 2 {
                let mark = if level == 1 { "=" } else { "-" };
                out.push_str(&mark.repeat(h.chars().count().max(3)));
                out.push('\n');
            }
            continue;
        }
        if let Some(q) = t.strip_prefix('>') {
            out.push_str("> ");
            out.push_str(&inline_text(q.trim()));
            out.push('\n');
            continue;
        }
        let indent = &line[..line.len() - t.len()];
        if let Some(item) = list_item(t) {
            out.push_str(indent);
            out.push_str(&match item {
                Item::Bullet(r) => format!("- {}", inline_text(r)),
                Item::Number(n, r) => format!("{n} {}", inline_text(r)),
                Item::Task(done, r) => {
                    format!("[{}] {}", if done { "x" } else { " " }, inline_text(r))
                }
            });
            out.push('\n');
            continue;
        }
        out.push_str(&inline_text(line));
        out.push('\n');
    }
    out
}

fn is_rule(t: &str) -> bool {
    t.len() >= 3 && t.chars().all(|c| c == '-' || c == '*' || c == '_')
}

/// `## Title` is (2, "Title").
fn heading(t: &str) -> Option<(usize, &str)> {
    let hashes = t.chars().take_while(|&c| c == '#').count();
    ((1..=6).contains(&hashes) && t[hashes..].starts_with(' '))
        .then(|| (hashes, t[hashes..].trim()))
}

enum Item<'a> {
    Bullet(&'a str),
    Number(&'a str, &'a str),
    Task(bool, &'a str),
}

fn list_item(t: &str) -> Option<Item<'_>> {
    for m in ["- ", "* ", "+ "] {
        if let Some(r) = t.strip_prefix(m) {
            for (b, done) in [("[ ] ", false), ("[x] ", true), ("[X] ", true)] {
                if let Some(r) = r.strip_prefix(b) {
                    return Some(Item::Task(done, r));
                }
            }
            return Some(Item::Bullet(r));
        }
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    (digits > 0 && t[digits..].starts_with(". "))
        .then(|| Item::Number(&t[..=digits], &t[digits + 2..]))
}

/// A photo line's PNG data URI, as Notes keeps photos; nothing else.
fn photo(t: &str) -> Option<&str> {
    let src = t
        .strip_prefix("![")?
        .split_once("](")?
        .1
        .strip_suffix(')')?;
    let data = src.strip_prefix("data:image/png;base64,")?;
    data.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
        .then_some(src)
}

fn cells(row: &str) -> Vec<&str> {
    let row = row.trim().trim_start_matches('|').trim_end_matches('|');
    row.split('|').map(str::trim).collect()
}

fn is_divider(row: &[&str]) -> bool {
    row.iter()
        .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':')))
}

fn table_html(rows: &[&str]) -> String {
    let mut out = String::from("<table>\n");
    for (i, row) in rows.iter().enumerate() {
        let c = cells(row);
        if is_divider(&c) {
            continue;
        }
        let tag = if i == 0 { "th" } else { "td" };
        out.push_str("<tr>");
        for cell in c {
            out.push_str(&format!("<{tag}>{}</{tag}>", inline_html(cell)));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
    out
}

/// A table with its columns lined up, the header underlined.
fn table_text(rows: &[&str]) -> String {
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| cells(r))
        .filter(|c| !is_divider(c))
        .map(|c| c.into_iter().map(inline_text).collect())
        .collect();
    let n = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..n)
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .map(|c| c.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |r: &Vec<String>| {
        let cols: Vec<String> = (0..n)
            .map(|i| {
                let c = r.get(i).map_or("", String::as_str);
                format!("{c:<w$}", w = widths[i])
            })
            .collect();
        format!("{}\n", cols.join("  ").trim_end())
    };
    let mut out = String::new();
    for (i, r) in rows.iter().enumerate() {
        out.push_str(&line(r));
        if i == 0 {
            let rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
            out.push_str(&format!("{}\n", rule.join("  ")));
        }
    }
    out
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Only these become links; anything else stays text.
fn safe_href(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:"))
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// Inline Markdown to HTML or plain text, by the Preview's rules.
fn inline(text: &str, html: bool) -> String {
    let mut out = String::new();
    let push = |out: &mut String, s: &str| {
        if html {
            out.push_str(&escape(s));
        } else {
            out.push_str(s);
        }
    };
    let (mut bold, mut italic, mut strike) = (false, false, false);
    let mut rest = text;
    while !rest.is_empty() {
        let next = rest
            .char_indices()
            .find(|(_, c)| matches!(c, '*' | '_' | '`' | '[' | ':' | '~'))
            .map(|(i, _)| i);
        let Some(i) = next else {
            push(&mut out, rest);
            break;
        };
        push(&mut out, &rest[..i]);
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix("**") {
            bold = !bold;
            if html {
                out.push_str(if bold { "<strong>" } else { "</strong>" });
            }
            rest = r;
        } else if let Some(r) = rest.strip_prefix("~~") {
            strike = !strike;
            if html {
                out.push_str(if strike { "<del>" } else { "</del>" });
            }
            rest = r;
        } else if rest.starts_with(':')
            && let Some((emoji, len)) = shortcode(rest)
        {
            out.push_str(emoji);
            rest = &rest[len..];
        } else if let Some(r) = rest.strip_prefix('`')
            && let Some(end) = r.find('`')
        {
            if html {
                out.push_str(&format!("<code>{}</code>", escape(&r[..end])));
            } else {
                out.push_str(&r[..end]);
            }
            rest = &r[end + 1..];
        } else if (rest.starts_with("[🔑 REDACTED: ") || rest.starts_with("[REDACTED: "))
            && let Some(end) = rest.find(']')
        {
            if html {
                out.push_str(&format!(
                    "<span class=\"redacted\">{}</span>",
                    escape(&rest[..=end])
                ));
            } else {
                out.push_str(&rest[..=end]);
            }
            rest = &rest[end + 1..];
        } else if rest.starts_with('[')
            && let Some(mid) = rest.find("](")
            && let Some(end) = rest[mid..].find(')')
        {
            let label = &rest[1..mid];
            let url = &rest[mid + 2..mid + end];
            match (html, safe_href(url)) {
                (true, true) => out.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    escape(url.trim()),
                    escape(label)
                )),
                (true, false) => out.push_str(&escape(label)),
                (false, _) if label.is_empty() || label == url => out.push_str(url),
                (false, _) => out.push_str(&format!("{label} ({url})")),
            }
            rest = &rest[mid + end + 1..];
        } else if (rest.starts_with('*') || rest.starts_with('_'))
            && (italic || rest[1..].starts_with(|c: char| !c.is_whitespace()))
            && (rest.starts_with('*') || {
                let prev = text[..text.len() - rest.len()].chars().next_back();
                let next = rest[1..].chars().next();
                let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
                if italic { !word(next) } else { !word(prev) }
            })
        {
            italic = !italic;
            if html {
                out.push_str(if italic { "<em>" } else { "</em>" });
            }
            rest = &rest[1..];
        } else {
            let c = rest.chars().next().map_or(1, char::len_utf8);
            push(&mut out, &rest[..c]);
            rest = &rest[c..];
        }
    }
    // A mark left open at the end of the line is closed there.
    if html {
        for (open, tag) in [(italic, "</em>"), (strike, "</del>"), (bold, "</strong>")] {
            if open {
                out.push_str(tag);
            }
        }
    }
    out
}

fn inline_html(text: &str) -> String {
    inline(text, true)
}

fn inline_text(text: &str) -> String {
    inline(text, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES: &str = "\
# Harbor Dental
## Front office
**Reported:** phones drop *in the afternoon* :fire:
- [x] Full scan
- [ ] Re-test ~~at 2~~ after 3 pm
1. First
2. Second
> Printer: port 9100 closed.
Wi-Fi: [🔑 REDACTED: password]
See [the wiki](https://example.com/wiki) or [this](javascript:alert(1)).

| Device | IP |
|---|---|
| `SW-FRONT-01` | 10.0.0.2 |
---
```
show run <x>
```";

    #[test]
    fn html_has_the_structure_and_escapes_everything() {
        let h = to_html(NOTES, "Notes <for> \"you\"");
        assert!(h.starts_with("<!doctype html>"));
        assert!(h.contains("<title>Notes &lt;for&gt; &quot;you&quot;</title>"));
        assert!(h.contains("<h1>Harbor Dental</h1>"));
        assert!(h.contains("<h2>Front office</h2>"));
        assert!(h.contains("<strong>Reported:</strong>"));
        assert!(h.contains("<em>in the afternoon</em>"));
        assert!(h.contains("🔥"));
        assert!(h.contains("<input type=\"checkbox\" disabled checked> Full scan"));
        assert!(h.contains("<del>at 2</del>"));
        assert!(h.contains("<ol>\n<li>First</li>\n<li>Second</li>\n</ol>"));
        assert!(h.contains("<blockquote>Printer: port 9100 closed.</blockquote>"));
        assert!(h.contains("<span class=\"redacted\">[🔑 REDACTED: password]</span>"));
        assert!(h.contains("<a href=\"https://example.com/wiki\">the wiki</a>"));
        // Not a link: no javascript: anywhere in an href.
        assert!(!h.contains("href=\"javascript"));
        assert!(h.contains("<th>Device</th><th>IP</th>"));
        assert!(h.contains("<td><code>SW-FRONT-01</code></td>"));
        assert!(h.contains("<hr>"));
        assert!(h.contains("<pre><code>show run &lt;x&gt;</code></pre>"));
    }

    #[test]
    fn html_never_carries_markup_from_the_notes() {
        let h = to_html(
            "<script>alert(1)</script>\n- <img src=x onerror=y>\n**<b>**",
            "t",
        );
        assert!(!h.contains("<script>alert"));
        assert!(!h.contains("<img src=x"));
        assert!(!h.contains("<b>"));
        assert!(h.contains("&lt;script&gt;"));
    }

    #[test]
    fn photos_only_as_png_data() {
        let h = to_html("![Photo](data:image/png;base64,iVBORw0KGgo=)", "t");
        assert!(h.contains("<img alt=\"Photo\" src=\"data:image/png;base64,iVBORw0KGgo=\">"));
        let h = to_html("![Photo](data:image/svg+xml;base64,PHN2Zz4=)", "t");
        assert!(!h.contains("<img"));
        let h = to_html("![Photo](data:image/png;base64,AA\"onerror=x)", "t");
        assert!(!h.contains("<img"));
    }

    #[test]
    fn text_takes_the_marks_out() {
        let t = to_text(NOTES);
        assert!(t.contains("Harbor Dental\n============="));
        assert!(t.contains("Front office\n------------"));
        assert!(t.contains("Reported: phones drop in the afternoon 🔥"));
        assert!(t.contains("[x] Full scan"));
        assert!(t.contains("[ ] Re-test at 2 after 3 pm"));
        assert!(t.contains("1. First"));
        assert!(t.contains("> Printer: port 9100 closed."));
        assert!(t.contains("[🔑 REDACTED: password]"));
        assert!(t.contains("See the wiki (https://example.com/wiki)"));
        assert!(t.contains("Device       IP\n-----------  --------\nSW-FRONT-01  10.0.0.2\n"));
        assert!(t.contains("    show run <x>"));
        assert!(!t.contains("**"));
        assert!(!t.contains('`'));
    }

    #[test]
    fn text_photo_is_a_placeholder() {
        assert_eq!(to_text("![Photo](data:image/png;base64,AAAA)"), "[Photo]\n");
    }
}
