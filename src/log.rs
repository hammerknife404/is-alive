//! Results log: every finished run, timestamped, with a cap. A site keeps
//! its own in `results.json` in its folder, so a visit's runs are there when
//! it opens again. Scratch (no site) keeps its own in the profile folder for
//! a day (`SCRATCH_HOURS`): through a crash, a flat battery or a close by
//! mistake, but not into the next client's visit, since its log can mix any
//! client's hosts.

use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

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
    /// Changed by every edit, from one counter for every log: a stamp
    /// taken before an edit (or of another site's log) never matches one
    /// taken after, so indices cached against it are never read stale.
    generation: u64,
}

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Log {
    /// Drop the runs that started more than `hours` before `now` (and any
    /// whose start can't be read).
    pub fn keep_since(&mut self, now: chrono::NaiveDateTime, hours: i64) {
        let from = now - chrono::Duration::hours(hours);
        let before = self.entries.len();
        self.entries.retain(|e| {
            chrono::NaiveDateTime::parse_from_str(&e.started, "%Y-%m-%d %H:%M:%S")
                .is_ok_and(|t| t >= from)
        });
        if self.entries.len() != before {
            self.edited();
        }
    }

    /// Changes whenever the entries do (pushes, drops, removals, clears),
    /// and never comes back to an earlier value.
    pub fn stamp(&self) -> u64 {
        self.generation
    }

    fn edited(&mut self) {
        self.bytes = self.entries.iter().map(|e| e.text.len()).sum();
        self.generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Take out the run of `tool` that started at `started`. True if it was there.
    pub fn remove(&mut self, tool: &str, started: &str) -> bool {
        let Some(i) = self
            .entries
            .iter()
            .position(|e| e.tool == tool && e.started == started)
        else {
            return false;
        };
        self.entries.remove(i);
        self.edited();
        true
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
        self.generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
        self.edited();
    }

    /// Count `n` more runs as dropped (from a log merged into this one).
    pub fn add_dropped(&mut self, n: usize) {
        if n > 0 {
            self.dropped += n;
            self.edited();
        }
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

// ---------------------------------------------------------------- saved

const FILE: &str = "results.json";
const FORMAT: &str = "is-alive-results-1";
/// The log's own cap with room for JSON's quoting and the tables.
const FILE_MAX: u64 = 3 * MAX_BYTES as u64;

/// Every tool that logs runs, by the name it logs under, with its table's
/// columns: a saved run is read back only as one of these (names and
/// columns are the app's own, never the file's).
const TOOLS: &[(&str, &[&str])] = &[
    ("Host info", crate::net::hostinfo::COLUMNS),
    ("Ping", crate::tools::ping::COLUMNS),
    ("Port Check", crate::tools::ports::COLUMNS),
    ("Traceroute", crate::tools::trace::COLUMNS),
    ("Speed Test", crate::tools::speed::COLUMNS),
    ("Subnet Sweep", crate::tools::sweep::COLUMNS),
    ("Lookup", crate::tools::dns::COLUMNS),
    ("Domain Health", crate::tools::advanced::COLUMNS),
    ("Network checks", crate::tools::netcheck::COLUMNS),
    ("Domain join", crate::net::join::COLUMNS),
];

/// Where the open site keeps its runs: its folder, or the profile folder
/// for Scratch.
pub fn site_file() -> Option<PathBuf> {
    Some(crate::site::dir()?.join(FILE))
}

/// A table cell for a value that wasn't looked at (a host a Smart sweep
/// didn't port-check): Compare doesn't count it as a change.
pub const NOT_CHECKED: &str = "not checked";

/// Hours Scratch's runs are kept for, from when each started.
pub const SCRATCH_HOURS: i64 = 24;

impl Log {
    fn to_json(&self) -> String {
        let runs: Vec<Value> = self
            .entries
            .iter()
            .map(|e| {
                let mut run = json!({
                    "tool": e.tool,
                    "started": e.started,
                    "ended": e.ended,
                    "text": e.text,
                });
                if let Some(t) = &e.table {
                    run["columns"] = json!(t.columns);
                    run["rows"] = json!(t.rows);
                }
                run
            })
            .collect();
        json!({ "format": FORMAT, "dropped": self.dropped, "runs": runs }).to_string()
    }

    /// Read back defensively: runs of tools this version doesn't log are
    /// left out, a table that isn't its tool's is kept as text only, and
    /// the caps apply as to any run.
    fn from_json(text: &str) -> Option<Log> {
        let v: Value = serde_json::from_str(text).ok()?;
        if v.get("format")?.as_str()? != FORMAT {
            return None;
        }
        let mut log = Log::default();
        let runs = v.get("runs")?.as_array()?;
        for r in runs.iter().skip(runs.len().saturating_sub(MAX_ENTRIES)) {
            let Some((tool, columns)) = r
                .get("tool")
                .and_then(Value::as_str)
                .and_then(|t| TOOLS.iter().find(|(name, _)| *name == t))
                .copied()
            else {
                continue;
            };
            let Some(text) = r.get("text").and_then(Value::as_str) else {
                continue;
            };
            let line =
                |k: &str| crate::input::line(r.get(k).and_then(Value::as_str).unwrap_or(""), 40);
            log.push(Entry {
                tool,
                started: line("started"),
                ended: line("ended"),
                text: text.to_string(),
                table: table(r, columns),
            });
        }
        let dropped = v.get("dropped").and_then(Value::as_u64).unwrap_or(0);
        log.dropped += usize::try_from(dropped).unwrap_or(0).min(1_000_000);
        Some(log)
    }

    /// The runs saved at `path`; an empty log when there are none or the
    /// file isn't a results file.
    pub fn load(path: &Path) -> Log {
        crate::input::read_capped(path, FILE_MAX)
            .and_then(|t| Log::from_json(&t))
            .unwrap_or_default()
    }

    /// Write the log to `path` in one step, readable by this user only; an
    /// empty log removes the file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if self.entries.is_empty() && self.dropped == 0 {
            return match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        crate::theme::write_private(&tmp, self.to_json())?;
        std::fs::rename(tmp, path)
    }
}

/// A saved run's table, if it has its tool's columns and every row fits them.
fn table(r: &Value, columns: &'static [&'static str]) -> Option<Table> {
    let cols = r.get("columns")?.as_array()?;
    if cols.len() != columns.len()
        || cols
            .iter()
            .zip(columns)
            .any(|(c, want)| c.as_str() != Some(want))
    {
        return None;
    }
    let rows = r
        .get("rows")?
        .as_array()?
        .iter()
        .map(|row| {
            let cells = row.as_array()?;
            (cells.len() == columns.len())
                .then(|| {
                    cells
                        .iter()
                        .map(|c| c.as_str().map(str::to_string))
                        .collect()
                })
                .flatten()
        })
        .collect::<Option<Vec<Vec<String>>>>()?;
    Some(Table { columns, rows })
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
pub(crate) const CSV_START: &str = "\u{feff}";

pub(crate) fn csv_row<'a>(cells: impl IntoIterator<Item = &'a str>) -> String {
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

    /// Results caches indices into the log against its stamp. Clearing it,
    /// taking a run out, or loading another site's log of the same size
    /// must each give a stamp it hasn't seen, or a cached index points past
    /// the end (the "out of bounds access" crash on Clear all).
    #[test]
    fn every_edit_gives_a_stamp_never_seen_before() {
        let mut seen = std::collections::HashSet::new();
        let mut log = Log::default();
        for n in 0..3 {
            log.push(Entry {
                started: format!("2026-11-11 10:00:0{n}"),
                ..entry(n)
            });
            assert!(seen.insert(log.stamp()));
        }
        let (tool, started) = (log.entries[1].tool, log.entries[1].started.clone());
        assert!(log.remove(tool, &started));
        assert_eq!(log.entries.len(), 2);
        assert!(seen.insert(log.stamp()));
        assert!(!log.remove(tool, &started), "already gone");
        log.clear();
        assert!(log.entries.is_empty());
        assert!(seen.insert(log.stamp()), "cleared: a new stamp");
        let mut other = Log::default();
        for n in 0..2 {
            other.push(entry(n));
        }
        assert!(seen.insert(other.stamp()), "another log of the same size");
        other.add_dropped(4);
        assert_eq!(other.dropped, 4);
        assert!(seen.insert(other.stamp()));
    }

    fn entry(n: usize) -> Entry {
        Entry {
            tool: "Ping",
            started: "a".into(),
            ended: "b".into(),
            text: "x".repeat(n),
            table: None,
        }
    }

    /// Scratch keeps a day of runs: older ones, and any whose start can't
    /// be read, go, and the size follows.
    #[test]
    fn scratch_keeps_a_day_of_runs() {
        let at = |started: &str, n: usize| Entry {
            started: started.into(),
            ..entry(n)
        };
        let mut log = Log::default();
        log.push(at("2026-11-09 08:00:00", 10)); // two days back
        log.push(at("2026-11-10 09:59:59", 20)); // a second too old
        log.push(at("2026-11-10 10:00:00", 30)); // just a day
        log.push(at("2026-11-11 09:30:00", 40));
        log.push(at("yesterday-ish", 50));
        let now = chrono::NaiveDateTime::parse_from_str("2026-11-11 10:00:00", "%Y-%m-%d %H:%M:%S")
            .unwrap();
        let before = log.stamp();
        log.keep_since(now, SCRATCH_HOURS);
        let kept: Vec<usize> = log.entries.iter().map(|e| e.text.len()).collect();
        assert_eq!(kept, [30, 40]);
        assert_eq!((log.entries.len(), log.dropped, log.bytes), (2, 0, 70));
        assert_ne!(log.stamp(), before, "an expiry has to be written back");
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "is-alive-log-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d.join(FILE)
    }

    #[test]
    fn every_tool_round_trips_with_its_table() {
        let mut log = Log::default();
        for (tool, columns) in TOOLS {
            log.push(Entry {
                tool,
                started: "2026-11-09 20:41:00".into(),
                ended: "2026-11-09 20:41:05".into(),
                text: format!("{tool} output\n<b>&\"quoted\""),
                table: Some(Table {
                    columns,
                    rows: vec![columns.iter().map(|c| format!("{c}-1")).collect()],
                }),
            });
        }
        log.push(entry(3));
        log.dropped = 2;
        let path = temp("round-trip");
        log.save(&path).unwrap();
        let back = Log::load(&path);
        assert_eq!(back.entries.len(), log.entries.len());
        assert_eq!(back.dropped, 2);
        for (a, b) in log.entries.iter().zip(&back.entries) {
            assert_eq!(
                (a.tool, &a.started, &a.ended, &a.text),
                (b.tool, &b.started, &b.ended, &b.text)
            );
            assert_eq!(
                a.table.as_ref().map(|t| (t.columns, &t.rows)),
                b.table.as_ref().map(|t| (t.columns, &t.rows))
            );
        }
        // The CSV is the same either way: the columns are the tool's own.
        let all = |l: &Log| Format::Csv.export(&l.entries.iter().collect::<Vec<_>>());
        assert_eq!(all(&log), all(&back));
        // Readable by this user only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // An empty log takes the file away.
        Log::default().save(&path).unwrap();
        assert!(!path.exists());
        Log::default().save(&path).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_saved_log_is_read_back_defensively() {
        // Not a results file: nothing.
        assert!(Log::from_json("{}").is_none());
        assert!(Log::from_json("not json").is_none());
        assert!(Log::from_json(r#"{"format":"other","runs":[]}"#).is_none());
        let text = json!({
            "format": FORMAT,
            "dropped": 1,
            "runs": [
                // A tool this version doesn't log: left out.
                {"tool": "Telnet", "started": "a", "ended": "b", "text": "x"},
                // No text: left out.
                {"tool": "Ping", "started": "a", "ended": "b"},
                // Columns that aren't Ping's: kept as text only.
                {"tool": "Ping", "started": "a", "ended": "b", "text": "kept",
                 "columns": ["evil"], "rows": [["=cmd"]]},
                // A short row: text only.
                {"tool": "Lookup", "started": "a", "ended": "b", "text": "dns",
                 "columns": crate::tools::dns::COLUMNS, "rows": [["one"]]},
                // A long start time is cut, control characters go.
                {"tool": "Ping", "started": "x\u{0007}".repeat(100), "ended": "b", "text": "t"},
            ],
        })
        .to_string();
        let log = Log::from_json(&text).unwrap();
        let got: Vec<(&str, &str, bool)> = log
            .entries
            .iter()
            .map(|e| (e.tool, e.text.as_str(), e.table.is_some()))
            .collect();
        assert_eq!(
            got,
            [
                ("Ping", "kept", false),
                ("Lookup", "dns", false),
                ("Ping", "t", false)
            ]
        );
        assert!(log.entries[2].started.chars().count() <= 40);
        assert!(!log.entries[2].started.contains('\u{0007}'));
        assert_eq!(log.dropped, 1);
        // The caps hold: only the newest MAX_ENTRIES runs are read.
        let many: Vec<Value> = (0..MAX_ENTRIES + 7)
            .map(|i| json!({"tool": "Ping", "started": "", "ended": "", "text": i.to_string()}))
            .collect();
        let log = Log::from_json(&json!({"format": FORMAT, "runs": many}).to_string()).unwrap();
        assert_eq!(log.entries.len(), MAX_ENTRIES);
        assert_eq!(log.entries[0].text, "7");
        // A file that isn't there is an empty log.
        assert!(
            Log::load(Path::new("/nonexistent/is-alive/results.json"))
                .entries
                .is_empty()
        );
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
