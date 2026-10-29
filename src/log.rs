//! Session log: every finished run, timestamped, kept in memory with a cap.

use std::collections::VecDeque;

/// Oldest entries are dropped past either limit.
const MAX_ENTRIES: usize = 500;
const MAX_BYTES: usize = 8 * 1024 * 1024;

pub fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Text logged for a finished run: its output, else its error, else a placeholder.
pub fn body(output: Option<String>, err: &Option<String>) -> String {
    output
        .or_else(|| err.as_ref().map(|e| format!("error: {e}")))
        .unwrap_or_else(|| "(no results)".into())
}

pub struct Entry {
    pub tool: &'static str,
    pub started: String,
    pub ended: String,
    pub text: String,
    /// The run's data as rows, for CSV; None for a run that only has text
    /// (one that failed before producing anything).
    pub table: Option<Table>,
}

/// Structured results of one run. Columns are fixed per tool, so runs of the
/// same tool stack into one CSV.
pub struct Table {
    pub columns: &'static [&'static str],
    pub rows: Vec<Vec<String>>,
}

/// Cell for an optional value: empty when absent.
pub fn cell<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}

/// Milliseconds with fixed precision, no unit, for spreadsheet columns.
pub fn ms(v: f64) -> String {
    format!("{v:.2}")
}

impl Entry {
    pub fn header(&self) -> String {
        format!(
            "{}  started {}, ended {}",
            self.tool, self.started, self.ended
        )
    }

    pub fn render(&self) -> String {
        format!("=== {} ===\n{}\n", self.header(), self.text.trim_end())
    }
}

#[derive(Default)]
pub struct Log {
    pub entries: VecDeque<Entry>,
    bytes: usize,
    pub dropped: usize,
}

impl Log {
    pub fn push(&mut self, e: Entry) {
        self.bytes += e.text.len();
        self.entries.push_back(e);
        while self.entries.len() > MAX_ENTRIES || (self.bytes > MAX_BYTES && self.entries.len() > 1)
        {
            if let Some(old) = self.entries.pop_front() {
                self.bytes -= old.text.len();
                self.dropped += 1;
            }
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn render(&self) -> String {
        let mut s = String::new();
        if self.dropped > 0 {
            s += &format!("({} older entries dropped)\n\n", self.dropped);
        }
        for e in &self.entries {
            s += &e.render();
            s += "\n";
        }
        s
    }
}

/// File formats for exporting log entries.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Format {
    Csv,
    Json,
    Markdown,
    Html,
}

impl Format {
    pub const ALL: [Format; 4] = [Format::Csv, Format::Json, Format::Markdown, Format::Html];

    pub fn name(self) -> &'static str {
        match self {
            Format::Csv => "CSV",
            Format::Json => "JSON",
            Format::Markdown => "Markdown",
            Format::Html => "HTML",
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Markdown => "md",
            Format::Html => "html",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Format::Csv => {
                "Each tool's own columns (port, state, RTT…), one row per result; \
runs of several tools save one file per tool into a folder"
            }
            Format::Json => "Array of runs, for scripts and other tools",
            Format::Markdown => "Headings and code blocks; pastes into tickets, wikis and chat",
            Format::Html => "Self-contained page; opens in any browser or attaches to an email",
        }
    }

    /// Files to write as (file name, contents). One file, except CSV across
    /// several tools: their columns differ, so each tool gets its own file.
    pub fn files(self, entries: &[&Entry], stem: &str) -> Vec<(String, String)> {
        let name = |s: &str| format!("is-alive-{s}.{}", self.ext());
        if self != Format::Csv {
            return vec![(name(stem), self.export(entries))];
        }
        // Group by tool in first-seen order; runs without a table share one file.
        let mut groups: Vec<(Option<&'static str>, Vec<&Entry>)> = Vec::new();
        for e in entries {
            let key = e.table.as_ref().map(|_| e.tool);
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => g.1.push(e),
                None => groups.push((key, vec![e])),
            }
        }
        let single = groups.len() == 1;
        groups
            .into_iter()
            .map(|(tool, es)| {
                let file = match tool {
                    _ if single => name(stem),
                    Some(t) => name(&format!("{stem}-{}", slug(t))),
                    None => name(&format!("{stem}-runs")),
                };
                let body = match tool {
                    Some(_) => csv_table(&es),
                    None => csv_runs(&es),
                };
                (file, body)
            })
            .collect()
    }

    pub fn export(self, entries: &[&Entry]) -> String {
        match self {
            Format::Csv => csv_runs(entries),
            Format::Json => {
                let runs: Vec<_> = entries
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "tool": e.tool,
                            "started": e.started,
                            "ended": e.ended,
                            "output": e.text.trim_end(),
                        })
                    })
                    .collect();
                serde_json::to_string_pretty(&runs).unwrap_or_default() + "\n"
            }
            Format::Markdown => {
                let mut s = String::new();
                for e in entries {
                    // A fence longer than any backtick run in the output can't be closed early.
                    let fence = "`".repeat(longest_run(&e.text, '`').max(2) + 1);
                    s += &format!(
                        "## {}\n\nStarted {}, ended {}\n\n{fence}\n{}\n{fence}\n\n",
                        e.tool,
                        e.started,
                        e.ended,
                        e.text.trim_end()
                    );
                }
                s
            }
            Format::Html => {
                let mut s = String::from(
                    "<!doctype html>\n<html><head><meta charset=\"utf-8\">\
<title>is-alive results</title>\n<style>\
body{font-family:system-ui,sans-serif;margin:2em;max-width:70em}\
h2{margin:1.5em 0 .2em;font-size:1.1em}.t{color:#666;font-size:.9em}\
pre{background:#f4f4f4;padding:.8em;overflow-x:auto}\
</style></head><body>\n",
                );
                for e in entries {
                    s += &format!(
                        "<h2>{}</h2>\n<div class=\"t\">Started {}, ended {}</div>\n<pre>{}</pre>\n",
                        html_escape(e.tool),
                        html_escape(&e.started),
                        html_escape(&e.ended),
                        html_escape(e.text.trim_end())
                    );
                }
                s + "</body></html>\n"
            }
        }
    }
}

// BOM so Excel reads UTF-8 (…, ✓) correctly; CRLF per RFC 4180.
const CSV_START: &str = "\u{feff}";

fn csv_row<'a>(cells: impl IntoIterator<Item = &'a str>) -> String {
    let cells: Vec<String> = cells.into_iter().map(csv_field).collect();
    cells.join(",") + "\r\n"
}

/// Runs of one tool: the tool's own columns, each row tagged with its run's start.
fn csv_table(entries: &[&Entry]) -> String {
    let Some(columns) = entries
        .iter()
        .find_map(|e| e.table.as_ref().map(|t| t.columns))
    else {
        return csv_runs(entries);
    };
    let mut s = String::from(CSV_START);
    s += &csv_row(["started"].into_iter().chain(columns.iter().copied()));
    for e in entries {
        for row in e.table.iter().flat_map(|t| &t.rows) {
            s += &csv_row(
                [e.started.as_str()]
                    .into_iter()
                    .chain(row.iter().map(String::as_str)),
            );
        }
    }
    s
}

/// One row per run with its text output, for runs with no table.
fn csv_runs(entries: &[&Entry]) -> String {
    let mut s = String::from(CSV_START);
    s += &csv_row(["tool", "started", "ended", "output"]);
    for e in entries {
        s += &csv_row([e.tool, &e.started, &e.ended, e.text.trim_end()]);
    }
    s
}

fn slug(tool: &str) -> String {
    tool.to_lowercase().replace(' ', "-")
}

fn csv_field(v: &str) -> String {
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

fn html_escape(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn longest_run(s: &str, c: char) -> usize {
    let (mut best, mut cur) = (0, 0);
    for ch in s.chars() {
        cur = if ch == c { cur + 1 } else { 0 };
        best = best.max(cur);
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: usize) -> Entry {
        Entry {
            tool: "Ping",
            started: "a".into(),
            ended: "b".into(),
            text: "x".repeat(n),
            table: None,
        }
    }

    #[test]
    fn caps_entries_and_bytes() {
        let mut log = Log::default();
        for _ in 0..MAX_ENTRIES + 5 {
            log.push(entry(1));
        }
        assert_eq!(log.entries.len(), MAX_ENTRIES);
        assert_eq!(log.dropped, 5);

        let mut log = Log::default();
        log.push(entry(MAX_BYTES / 2));
        log.push(entry(MAX_BYTES / 2));
        log.push(entry(10));
        assert_eq!(log.entries.len(), 2);
        // A single oversized entry is still kept.
        log.push(entry(MAX_BYTES * 2));
        assert_eq!(log.entries.len(), 1);
    }

    #[test]
    fn exports_escape_output() {
        let e = Entry {
            tool: "DNS Lookup",
            started: "a".into(),
            ended: "b".into(),
            text: "x, \"y\"\n<z> ```\n".into(),
            table: None,
        };
        let csv = Format::Csv.export(&[&e]);
        assert!(csv.ends_with("DNS Lookup,a,b,\"x, \"\"y\"\"\n<z> ```\"\r\n"));

        let json: serde_json::Value = serde_json::from_str(&Format::Json.export(&[&e])).unwrap();
        assert_eq!(json[0]["output"], "x, \"y\"\n<z> ```");

        assert!(Format::Markdown.export(&[&e]).contains("````\nx, "));
        assert!(Format::Html.export(&[&e]).contains("&lt;z&gt;"));
    }

    fn run(tool: &'static str, started: &str, rows: &[&[&str]]) -> Entry {
        Entry {
            tool,
            started: started.into(),
            ended: "e".into(),
            text: "text".into(),
            table: Some(Table {
                columns: &["port", "state"],
                rows: rows
                    .iter()
                    .map(|r| r.iter().map(|c| c.to_string()).collect())
                    .collect(),
            }),
        }
    }

    #[test]
    fn csv_uses_tool_columns() {
        let a = run("Port Check", "t1", &[&["22", "open"], &["80", "closed"]]);
        let b = run("Port Check", "t2", &[&["443", "open"]]);
        let files = Format::Csv.files(&[&a, &b], "log");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "is-alive-log.csv");
        assert_eq!(
            files[0].1,
            "\u{feff}started,port,state\r\nt1,22,open\r\nt1,80,closed\r\nt2,443,open\r\n"
        );
    }

    #[test]
    fn csv_splits_tools_into_files() {
        let a = run("Port Check", "t1", &[&["22", "open"]]);
        let b = run("Subnet Sweep", "t2", &[]);
        let mut c = run("Ping", "t3", &[]);
        c.table = None;
        let names: Vec<String> = Format::Csv
            .files(&[&a, &b, &c], "log")
            .into_iter()
            .map(|f| f.0)
            .collect();
        assert_eq!(
            names,
            [
                "is-alive-log-port-check.csv",
                "is-alive-log-subnet-sweep.csv",
                "is-alive-log-runs.csv"
            ]
        );
        assert_eq!(Format::Json.files(&[&a, &b], "log").len(), 1);
    }
}
