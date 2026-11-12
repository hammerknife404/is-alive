//! Compare two runs of the same tool in Results: what's new, gone and
//! changed between them, the earlier run first. Rows are matched by what
//! they're about (a sweep's host by IP, a port by IP and port, a hop by
//! its number, a DNS answer by itself); measurements (Ping, Speed Test)
//! are set side by side with the difference. Noise (a reply time in a
//! sweep) isn't counted as a change.

use crate::log::{self, Entry, Table};
use std::collections::BTreeMap;

/// One difference between the runs.
#[derive(Debug, PartialEq)]
pub enum Change {
    /// In the later run only.
    New(String),
    /// In the earlier run only.
    Gone(String),
    /// In both, not the same: what it is, and how it changed.
    Changed(String, String),
}

/// A figure from both runs: its name, before, after, and the difference.
#[derive(Debug, PartialEq)]
pub struct Figure {
    pub name: &'static str,
    pub before: String,
    pub after: String,
    pub delta: String,
}

/// What changed from `before` to `after`.
#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    pub changes: Vec<Change>,
    pub figures: Vec<Figure>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.figures.is_empty()
    }

    pub fn count(&self, f: fn(&Change) -> bool) -> usize {
        self.changes.iter().filter(|c| f(c)).count()
    }

    /// As text, for Copy: a heading, then each difference on a line.
    pub fn render(&self, title: &str, before: &str, after: &str) -> String {
        let mut s = format!("=== Compare: {title} ===\nBefore: {before}\nAfter:  {after}\n");
        if self.is_empty() {
            s += "No differences.\n";
        }
        for f in &self.figures {
            s += &format!("{}: {} → {} ({})\n", f.name, f.before, f.after, f.delta);
        }
        for c in &self.changes {
            s += &match c {
                Change::New(w) => format!("+ New: {w}\n"),
                Change::Gone(w) => format!("- Gone: {w}\n"),
                Change::Changed(w, how) => format!("~ Changed: {w}: {how}\n"),
            };
        }
        s
    }
}

/// How a tool's rows are matched (`key`) and which of their cells count
/// as a change (`fields`); other cells (reply times) are noise.
struct Spec {
    key: &'static [&'static str],
    fields: &'static [&'static str],
}

fn spec(tool: &str) -> Option<Spec> {
    let s = |key, fields| Some(Spec { key, fields });
    match tool {
        "Subnet Sweep" => s(&["ip"], &["mac", "vendor", "name", "open_ports"]),
        "Port Check" => s(
            &["ip", "port"],
            &[
                "state",
                "service",
                "tls_status",
                "tls_issued_to",
                "tls_expires",
            ],
        ),
        "Traceroute" => s(&["hop"], &["address", "name"]),
        "DNS Lookup" => s(&["lookup", "source", "type", "value"], &[]),
        "Domain Health" => s(&["section", "check", "kind"], &["status", "text"]),
        "Network checks" => s(&["check"], &["status"]),
        "Domain join" => s(&["section", "field"], &["value"]),
        "Domain join (others)" => s(&["host"], &["verdict", "computer", "dns_domain"]),
        "Speed Test" => s(
            &["phase"],
            &["mbps", "latency_avg_ms", "jitter_ms", "loss_pct", "mos"],
        ),
        _ => None,
    }
}

/// Compare two runs of one tool; `before` is the earlier.
pub fn compare(before: &Entry, after: &Entry) -> Diff {
    let mut d = Diff::default();
    if before.tool == "Ping" {
        d.figures = ping_figures(before, after);
        return d;
    }
    match (spec(before.tool), &before.table, &after.table) {
        (Some(spec), Some(a), Some(b)) => d.changes = rows(&spec, a, b),
        _ => d.changes = lines(&before.text, &after.text),
    }
    d
}

/// A column's cell in `row`, or "".
fn cell<'a>(t: &Table, row: &'a [String], col: &str) -> &'a str {
    t.columns
        .iter()
        .position(|c| *c == col)
        .and_then(|i| row.get(i))
        .map_or("", String::as_str)
}

/// The rows of each table by key, their fields as one tuple each.
fn by_key(spec: &Spec, t: &Table) -> BTreeMap<String, Vec<Vec<String>>> {
    let mut m: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    for row in &t.rows {
        let key: Vec<&str> = spec.key.iter().map(|k| cell(t, row, k)).collect();
        if key.iter().all(|k| k.is_empty()) {
            continue;
        }
        let fields = spec
            .fields
            .iter()
            .map(|f| cell(t, row, f).to_string())
            .collect();
        m.entry(key.join(" · ")).or_default().push(fields);
    }
    for v in m.values_mut() {
        v.sort();
        v.dedup();
    }
    m
}

/// What a row's fields say, for New and Gone: "MAC …, name …".
fn says(spec: &Spec, fields: &[Vec<String>]) -> String {
    let one = |f: &Vec<String>| {
        spec.fields
            .iter()
            .zip(f)
            .filter(|(_, v)| !v.is_empty())
            .map(|(n, v)| format!("{} {v}", label(n)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    fields
        .iter()
        .map(one)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

/// A column's name as words: "open_ports" is "open ports".
fn label(col: &str) -> String {
    col.trim_end_matches("_ms").replace('_', " ")
}

fn rows(spec: &Spec, a: &Table, b: &Table) -> Vec<Change> {
    let (was, now) = (by_key(spec, a), by_key(spec, b));
    let with = |key: &str, what: String| {
        if what.is_empty() {
            key.to_string()
        } else {
            format!("{key} ({what})")
        }
    };
    let mut out = Vec::new();
    for (k, f) in &now {
        if !was.contains_key(k) {
            out.push(Change::New(with(k, says(spec, f))));
        }
    }
    for (k, f) in &was {
        if !now.contains_key(k) {
            out.push(Change::Gone(with(k, says(spec, f))));
        }
    }
    // A cell one run didn't look at (a host a Smart sweep didn't
    // port-check) is no change.
    let same = |o: &String, n: &String| o == n || o == log::NOT_CHECKED || n == log::NOT_CHECKED;
    let alike = |a: &[Vec<String>], b: &[Vec<String>]| {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(x, y)| x.len() == y.len() && x.iter().zip(y).all(|(o, n)| same(o, n)))
    };
    for (k, old) in &was {
        let Some(new) = now.get(k) else { continue };
        if alike(old, new) {
            continue;
        }
        let how = if let ([o], [n]) = (old.as_slice(), new.as_slice()) {
            spec.fields
                .iter()
                .zip(o.iter().zip(n))
                .filter(|(_, (o, n))| !same(o, n))
                .map(|(f, (o, n))| format!("{} {}", label(f), moved(o, n)))
                .collect::<Vec<_>>()
                .join("; ")
        } else {
            format!("was {}; now {}", says(spec, old), says(spec, new))
        };
        out.push(Change::Changed(k.clone(), how));
    }
    out
}

/// "a → b", with the difference when both are numbers: "412 → 380 (−32)".
fn moved(o: &str, n: &str) -> String {
    let show = |s: &str| {
        if s.is_empty() {
            "—".to_string()
        } else {
            s.to_string()
        }
    };
    match (o.parse::<f64>(), n.parse::<f64>()) {
        (Ok(a), Ok(b)) => format!("{} → {} ({})", show(o), show(n), delta(b - a)),
        _ => format!("{} → {}", show(o), show(n)),
    }
}

/// A difference with its sign: "+3.2", "−0.5", "±0".
fn delta(d: f64) -> String {
    if d.abs() < 0.005 {
        "±0".into()
    } else if d > 0.0 {
        format!("+{}", trim(d))
    } else {
        format!("−{}", trim(-d))
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Ping's figures: replies, loss and reply times, side by side.
fn ping_figures(a: &Entry, b: &Entry) -> Vec<Figure> {
    struct P {
        sent: usize,
        got: Vec<f64>,
    }
    let read = |e: &Entry| {
        let t = e.table.as_ref()?;
        let got = t
            .rows
            .iter()
            .filter_map(|r| cell(t, r, "rtt_ms").parse::<f64>().ok())
            .collect::<Vec<_>>();
        Some(P {
            sent: t.rows.len(),
            got,
        })
    };
    let (Some(x), Some(y)) = (read(a), read(b)) else {
        return Vec::new();
    };
    let loss = |p: &P| 100.0 * (p.sent - p.got.len()) as f64 / p.sent.max(1) as f64;
    let avg = |p: &P| (!p.got.is_empty()).then(|| p.got.iter().sum::<f64>() / p.got.len() as f64);
    let min = |p: &P| p.got.iter().copied().reduce(f64::min);
    let max = |p: &P| p.got.iter().copied().reduce(f64::max);
    let fig = |name, o: Option<f64>, n: Option<f64>, unit: &str| {
        let show = |v: Option<f64>| v.map_or("—".to_string(), |v| format!("{}{unit}", trim(v)));
        Figure {
            name,
            before: show(o),
            after: show(n),
            delta: match (o, n) {
                (Some(o), Some(n)) => format!("{}{unit}", delta(n - o)),
                _ => "—".into(),
            },
        }
    };
    vec![
        fig(
            "Replies",
            Some(x.got.len() as f64),
            Some(y.got.len() as f64),
            "",
        ),
        fig("Loss", Some(loss(&x)), Some(loss(&y)), "%"),
        fig("Avg", avg(&x), avg(&y), " ms"),
        fig("Min", min(&x), min(&y), " ms"),
        fig("Max", max(&x), max(&y), " ms"),
    ]
}

/// Runs with no table (or a tool without a spec): the lines of output in
/// one run and not the other.
fn lines(a: &str, b: &str) -> Vec<Change> {
    let set = |s: &str| {
        s.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect::<std::collections::BTreeSet<_>>()
    };
    let (x, y) = (set(a), set(b));
    y.difference(&x)
        .map(|l| Change::New(l.clone()))
        .chain(x.difference(&y).map(|l| Change::Gone(l.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tool: &'static str, columns: &'static [&'static str], rows: &[&[&str]]) -> Entry {
        Entry {
            tool,
            started: "2026-11-10 20:02:09".into(),
            ended: "2026-11-10 20:02:24".into(),
            text: String::new(),
            table: Some(Table {
                columns,
                rows: rows
                    .iter()
                    .map(|r| r.iter().map(|c| c.to_string()).collect())
                    .collect(),
            }),
        }
    }

    const SWEEP: &[&str] = crate::tools::sweep::COLUMNS;

    /// A host new, a host gone, a host whose name and open ports changed;
    /// a reply time that changed is noise.
    #[test]
    fn sweeps_compare_by_host() {
        let row = |ip, rtt, name, ports| {
            let mut r = vec![""; SWEEP.len()];
            for (c, v) in [
                ("ip", ip),
                ("rtt_ms", rtt),
                ("name", name),
                ("open_ports", ports),
            ] {
                r[SWEEP.iter().position(|x| *x == c).unwrap()] = v;
            }
            r
        };
        let a = entry(
            "Subnet Sweep",
            SWEEP,
            &[
                &row("192.0.2.1", "0.4", "gw", "443"),
                &row("192.0.2.30", "1.1", "printer", ""),
                &row("192.0.2.51", "0.3", "NAS", "22"),
            ],
        );
        let b = entry(
            "Subnet Sweep",
            SWEEP,
            &[
                &row("192.0.2.1", "0.9", "gw", "443"),
                &row("192.0.2.51", "0.3", "TRUENAS", "22 443"),
                &row("192.0.2.77", "2.0", "DESKTOP-7Q2", ""),
            ],
        );
        let d = compare(&a, &b);
        assert_eq!(
            d.changes,
            vec![
                Change::New("192.0.2.77 (name DESKTOP-7Q2)".into()),
                Change::Gone("192.0.2.30 (name printer)".into()),
                Change::Changed(
                    "192.0.2.51".into(),
                    "name NAS → TRUENAS; open ports 22 → 22 443".into()
                ),
            ]
        );
        assert_eq!(compare(&a, &a), Diff::default(), "a run against itself");
        // A Smart sweep that didn't port-check the NAS this time: its ports
        // weren't looked at, so they didn't change; its name still did.
        let c = entry(
            "Subnet Sweep",
            SWEEP,
            &[
                &row("192.0.2.1", "0.4", "gw", "443"),
                &row("192.0.2.30", "1.1", "printer", ""),
                &row("192.0.2.51", "0.3", "TRUENAS", log::NOT_CHECKED),
            ],
        );
        assert_eq!(
            compare(&a, &c).changes,
            vec![Change::Changed(
                "192.0.2.51".into(),
                "name NAS → TRUENAS".into()
            )]
        );
    }

    /// Numbers that changed say by how much.
    #[test]
    fn speed_tests_show_how_much_each_figure_moved() {
        const COLS: &[&str] = &["phase", "mbps", "latency_avg_ms"];
        let a = entry("Speed Test", COLS, &[&["Download", "412", "12.5"]]);
        let b = entry("Speed Test", COLS, &[&["Download", "380", "60.3"]]);
        assert_eq!(
            compare(&a, &b).changes,
            vec![Change::Changed(
                "Download".into(),
                "mbps 412 → 380 (−32); latency avg 12.5 → 60.3 (+47.8)".into()
            )]
        );
    }

    #[test]
    fn pings_compare_their_figures() {
        const COLS: &[&str] = crate::tools::ping::COLUMNS;
        let row = |rtt: &'static str| {
            let mut r = vec![""; COLS.len()];
            r[COLS.iter().position(|c| *c == "rtt_ms").unwrap()] = rtt;
            r
        };
        let a = entry(
            "Ping",
            COLS,
            &[&row("10"), &row("12"), &row("11"), &row("")],
        );
        let b = entry(
            "Ping",
            COLS,
            &[&row("20"), &row("22"), &row("21"), &row("21")],
        );
        let d = compare(&a, &b);
        let f = |name| d.figures.iter().find(|f| f.name == name).unwrap();
        assert_eq!(
            (f("Loss").before.as_str(), f("Loss").after.as_str()),
            ("25%", "0%")
        );
        assert_eq!(f("Loss").delta, "−25%");
        assert_eq!(f("Avg").delta, "+10 ms");
    }

    /// A run with no table compares its lines.
    #[test]
    fn runs_without_a_table_compare_their_lines() {
        let mut a = entry("Host info", &[], &[]);
        a.table = None;
        a.text = "LAN 10.0.0.91/24\nGateway 10.0.0.1".into();
        let mut b = entry("Host info", &[], &[]);
        b.table = None;
        b.text = "LAN 10.0.0.92/24\nGateway 10.0.0.1".into();
        assert_eq!(
            compare(&a, &b).changes,
            vec![
                Change::New("LAN 10.0.0.92/24".into()),
                Change::Gone("LAN 10.0.0.91/24".into())
            ]
        );
    }

    #[test]
    fn copy_says_what_changed() {
        let d = Diff {
            changes: vec![Change::New("192.0.2.77".into())],
            figures: vec![],
        };
        let t = d.render("Subnet Sweep 192.0.2.0/24", "11-10 20:02", "11-11 09:15");
        assert!(t.contains("+ New: 192.0.2.77"), "{t}");
        assert!(
            Diff::default()
                .render("x", "a", "b")
                .contains("No differences")
        );
    }
}
