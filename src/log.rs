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
    /// What the run tested (host, range, domain), from its table's first
    /// column; None for runs without one (Host info, early failures).
    pub fn target(&self) -> Option<&str> {
        let t = self.table.as_ref()?;
        let first = *t.columns.first()?;
        ["target", "host", "range", "lookup", "domain"]
            .contains(&first)
            .then(|| t.rows.first()?.first().map(String::as_str))
            .flatten()
            .filter(|v| !v.is_empty())
    }

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
    /// Changes whenever the entries do (pushes, drops, clears).
    pub fn stamp(&self) -> (usize, usize, usize) {
        (self.entries.len(), self.dropped, self.bytes)
    }

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
                "One row per result with each tool's columns (port, state, RTT…); \
filter on the tool column when several tools are exported"
            }
            Format::Json => "Array of runs, for scripts and other tools",
            Format::Markdown => "Headings and code blocks; pastes into tickets, wikis and chat",
            Format::Html => "Self-contained page; opens in any browser or attaches to an email",
        }
    }

    pub fn export(self, entries: &[&Entry]) -> String {
        match self {
            Format::Csv => csv(entries),
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

/// One file for any mix of runs. Runs of a single tool get just that tool's
/// columns; a mix gets a tool column and the union of every tool's columns
/// (shared names like ip and rtt_ms line up), blank where a tool has none.
/// Runs with no table (failed early) put their text in an output column.
fn csv(entries: &[&Entry]) -> String {
    let mut columns: Vec<&str> = Vec::new();
    for c in entries
        .iter()
        .flat_map(|e| e.table.iter().flat_map(|t| t.columns))
    {
        if !columns.contains(c) {
            columns.push(c);
        }
    }
    let single = entries.windows(2).all(|w| w[0].tool == w[1].tool);
    let text_only = entries.iter().any(|e| e.table.is_none());
    let mut head: Vec<&str> = Vec::new();
    if !single || text_only {
        head.push("tool");
    }
    head.push("started");
    head.extend(&columns);
    if text_only {
        head.push("output");
    }

    let mut s = String::from(CSV_START);
    s += &csv_row(head.iter().copied());
    for e in entries {
        let mut prefix: Vec<&str> = Vec::new();
        if !single || text_only {
            prefix.push(e.tool);
        }
        prefix.push(&e.started);
        match &e.table {
            Some(t) => {
                for r in &t.rows {
                    let mut row = prefix.clone();
                    row.extend(columns.iter().map(|c| {
                        t.columns
                            .iter()
                            .position(|tc| tc == c)
                            .map_or("", |i| r[i].as_str())
                    }));
                    if text_only {
                        row.push("");
                    }
                    s += &csv_row(row);
                }
            }
            None => {
                let mut row = prefix.clone();
                row.extend(columns.iter().map(|_| ""));
                row.push(e.text.trim_end());
                s += &csv_row(row);
            }
        }
    }
    s
}

/// Quote when needed, and defuse cells Excel would run as a formula: names
/// and records come from the network (a PTR or TXT of "=HYPERLINK(…)").
fn csv_field(v: &str) -> String {
    // Full-width = + - @ count too: Excel reads them as the ASCII ones.
    let formula = v.starts_with([
        '=', '+', '-', '@', '|', '\t', '\r', '\u{FF1D}', '\u{FF0B}', '\u{FF0D}', '\u{FF20}',
    ]) && v.parse::<f64>().is_err();
    let v = &if formula {
        format!("'{v}")
    } else {
        v.to_string()
    };
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
        assert!(csv.ends_with("DNS Lookup,a,\"x, \"\"y\"\"\n<z> ```\"\r\n"));

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
    fn csv_defuses_formulas() {
        assert_eq!(csv_field("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(csv_field("@cmd"), "'@cmd");
        assert_eq!(csv_field("|calc"), "'|calc");
        assert_eq!(csv_field("\u{FF1D}1+1"), "'\u{FF1D}1+1");
        assert_eq!(csv_field("-1"), "-1");
        assert_eq!(csv_field("+5.5"), "+5.5");
        assert_eq!(csv_field("v=spf1 -all"), "v=spf1 -all");
    }

    #[test]
    fn target_from_first_column() {
        let mut e = run("Port Check", "t", &[&["10.0.0.1", "open"]]);
        e.table.as_mut().unwrap().columns = &["host", "state"];
        assert_eq!(e.target(), Some("10.0.0.1"));
        e.table.as_mut().unwrap().columns = &["field", "value"];
        assert_eq!(e.target(), None);
        e.table = None;
        assert_eq!(e.target(), None);
    }

    #[test]
    fn csv_uses_tool_columns() {
        let a = run("Port Check", "t1", &[&["22", "open"], &["80", "closed"]]);
        let b = run("Port Check", "t2", &[&["443", "open"]]);
        assert_eq!(
            Format::Csv.export(&[&a, &b]),
            "\u{feff}started,port,state\r\nt1,22,open\r\nt1,80,closed\r\nt2,443,open\r\n"
        );
    }

    #[test]
    fn csv_mixes_tools_in_one_file() {
        let a = run("Port Check", "t1", &[&["22", "open"]]);
        let mut b = run("Subnet Sweep", "t2", &[&["10.0.0.1", "1.5"]]);
        b.table.as_mut().unwrap().columns = &["ip", "rtt_ms"];
        let mut c = run("Ping", "t3", &[]);
        c.table = None;
        c.text = "error: no route".into();
        assert_eq!(
            Format::Csv.export(&[&a, &b, &c]),
            "\u{feff}tool,started,port,state,ip,rtt_ms,output\r\n\
Port Check,t1,22,open,,,\r\n\
Subnet Sweep,t2,,,10.0.0.1,1.5,\r\n\
Ping,t3,,,,,error: no route\r\n"
        );
    }
}
