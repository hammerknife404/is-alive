//! The site report: one web page that documents a visit, for the client,
//! the ticket or the next technician. It puts together what the tabs
//! already make: the map as a picture, its findings, what changed since
//! the last visit, every device, the notes and, if chosen, the test runs.
//!
//! Safe to open anywhere: every value from the network, a config or the
//! notes is escaped; the map is an SVG inside an `<img>`, where nothing in
//! it can run; and the page's own policy lets it load nothing and run
//! nothing (no script, font or request), so opening it tells no one.

use crate::map::Sev;
use crate::map::compare::{Change, Diff};
use crate::markdown_out::escape;
use crate::net::mac::{self, Mac};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::fmt::Write;
use std::net::Ipv4Addr;

/// What the map adds to the report.
pub struct MapPart {
    /// Where it came from: "Live map", or a saved map's name.
    pub source: String,
    pub title: String,
    /// A plan's, a training map's or an example's caption: not what was found.
    pub caption: Option<String>,
    /// The map as laid out on the Map tab, as SVG on white.
    pub svg: String,
    /// Worst first.
    pub findings: Vec<Finding>,
    /// Every device, every client included whatever the map shows.
    pub devices: Vec<Device>,
    /// The live map compared with a saved one.
    pub since: Option<Since>,
}

pub struct Finding {
    pub sev: Sev,
    pub text: String,
    /// The devices it names.
    pub devices: Vec<String>,
}

#[derive(Default)]
pub struct Device {
    pub name: String,
    pub role: String,
    /// The role's colour on white.
    pub rgb: [u8; 3],
    pub kind: String,
    pub ip: Option<Ipv4Addr>,
    pub mac: Option<Mac>,
    pub vendor: String,
    pub site: String,
    /// What it plugs into, with the port: "usw-office · Port 7".
    pub uplink: String,
    /// The link up isn't in the data: placed where it answered.
    pub assumed: bool,
    pub ports: Vec<u16>,
    pub flags: Vec<String>,
    /// The technician's note on its card, redacted.
    pub note: String,
    pub client: bool,
}

/// The changes since a saved map.
pub struct Since {
    pub name: String,
    /// The day it was saved.
    pub day: String,
    pub diff: Diff,
}

/// One run from Results.
pub struct Run {
    pub tool: String,
    pub target: Option<String>,
    pub started: String,
    pub ended: String,
    pub text: String,
}

/// The map's sections that go in.
#[derive(Clone, Copy)]
pub struct Parts {
    pub picture: bool,
    pub findings: bool,
    pub changes: bool,
    pub infra: bool,
    pub clients: bool,
}

pub struct Report {
    pub title: String,
    /// When it was made, local time.
    pub made: String,
    /// Where is-alive ran from: (LAN, "192.168.1.50/24"), Gateway, WAN.
    pub from: Vec<(&'static str, String)>,
    pub map: Option<MapPart>,
    pub parts: Parts,
    /// The notes, redacted (Markdown).
    pub notes: Option<String>,
    pub runs: Vec<Run>,
}

/// Nothing loads and nothing runs: pictures only from the page itself.
const POLICY: &str = "default-src 'none'; img-src data:; style-src 'unsafe-inline'";

const STYLE: &str = "\
:root{--ink:#1d2128;--weak:#5f6773;--line:#d7dbe1;--soft:#f4f6f8;--accent:#1f5fae}\
*{box-sizing:border-box}\
body{font:14px/1.5 system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;color:var(--ink);\
background:#fff;max-width:1100px;margin:28px auto;padding:0 20px}\
header{border-bottom:3px solid var(--accent);padding-bottom:10px;margin-bottom:18px}\
h1{font-size:26px;margin:0}h2{font-size:18px;margin:28px 0 8px;padding-bottom:4px;\
border-bottom:1px solid var(--line)}h3{font-size:15px;margin:18px 0 6px}\
.meta,.sub,.weak{color:var(--weak)}.meta{margin:2px 0 0}.sub{margin:0 0 8px}\
.caption{background:#fff3cd;color:#6b4a00;border-radius:6px;padding:6px 10px;margin:10px 0 0;\
font-weight:600}\
nav{font-size:13px;color:var(--weak);margin:0 0 14px}nav a{color:var(--accent);margin-right:12px}\
.glance{display:flex;flex-wrap:wrap;gap:10px;margin:0 0 6px}\
.tile{flex:1 1 200px;border:1px solid var(--line);border-radius:8px;padding:8px 12px;\
background:var(--soft)}.tile b{display:block;font-size:12px;color:var(--weak);font-weight:600;\
text-transform:uppercase;letter-spacing:.04em}\
.map{border:1px solid var(--line);border-radius:8px;padding:6px;text-align:center}\
.map img{max-width:100%;height:auto}\
table{border-collapse:collapse;width:100%;margin:6px 0;font-size:13px}\
th,td{border-bottom:1px solid var(--line);padding:4px 8px;text-align:left;vertical-align:top;\
overflow-wrap:anywhere}th{background:var(--soft);font-weight:600;white-space:nowrap}\
td.mono,.mono{font-family:ui-monospace,Consolas,'DejaVu Sans Mono',monospace;font-size:12px;\
white-space:nowrap}\
.sev{display:inline-block;min-width:44px;text-align:center;border-radius:4px;padding:0 6px;\
font-size:11px;font-weight:700;text-transform:uppercase}\
.sev.high{background:#fde2e1;color:#a4161a}.sev.med{background:#fff0c2;color:#7a5200}\
.sev.low{background:#eceff3;color:#4b5563}\
.dot{display:inline-block;width:9px;height:9px;border-radius:50%;margin-right:6px}\
.chip{display:inline-block;font-size:11px;border:1px solid var(--line);border-radius:4px;\
padding:0 4px;margin:0 4px 0 0;color:var(--weak)}\
.new{color:#16794a;font-weight:600}.changed{color:#8a5a00;font-weight:600}\
.gone{color:#a4161a;font-weight:600}\
.notes{border-left:3px solid var(--line);padding-left:14px}\
.notes h1{font-size:18px}.notes h2{font-size:16px;border:0}.notes h3{font-size:14px}\
.notes img{max-width:100%;border-radius:6px}\
code,pre{font-family:ui-monospace,Consolas,'DejaVu Sans Mono',monospace;background:var(--soft);\
border-radius:4px}code{padding:.1em .3em}pre{padding:10px;overflow:auto;font-size:12px;\
white-space:pre-wrap}pre code{padding:0}\
blockquote{margin:0;padding:0 1em;color:var(--weak);border-left:3px solid var(--line)}\
li.task{list-style:none;margin-left:-1.2em}\
.redacted{background:#fff3cd;color:#7a5600;border-radius:3px;padding:0 .2em}\
footer{margin-top:32px;padding-top:8px;border-top:1px solid var(--line);font-size:12px;\
color:var(--weak)}\
@media print{body{margin:0;max-width:none;font-size:12px}nav{display:none}\
h2{break-after:avoid}tr,.tile,.map{break-inside:avoid}a{color:inherit;text-decoration:none}}";

/// The report as one self-contained web page.
pub fn html(r: &Report) -> String {
    let mut body = String::new();
    let title = match r.title.trim() {
        "" => "Site report",
        t => t,
    };
    let _ = write!(
        body,
        "<header>\n<h1>{}</h1>\n<p class=\"meta\">Site report · {} · is-alive {}</p>\n",
        escape(title),
        escape(&r.made),
        crate::VERSION
    );
    if let Some(c) = r.map.as_ref().and_then(|m| m.caption.as_deref()) {
        let _ = writeln!(body, "<p class=\"caption\">{}</p>", escape(c));
    }
    body += "</header>\n";

    let sections = contents(r);
    if sections.len() > 1 {
        body += "<nav>";
        for (id, name) in &sections {
            let _ = write!(body, "<a href=\"#{id}\">{name}</a>");
        }
        body += "</nav>\n";
    }
    glance(&mut body, r);

    if let Some(m) = &r.map {
        let p = r.parts;
        if p.picture {
            picture(&mut body, m);
        }
        if p.findings {
            findings(&mut body, &m.findings);
        }
        if p.changes
            && let Some(s) = &m.since
        {
            since(&mut body, s);
        }
        // One site is in the title already: a column only for two or more.
        let first = m.devices.first().map(|d| d.site.as_str());
        let show_site = m.devices.iter().any(|d| Some(d.site.as_str()) != first);
        // A table with nothing in it stays out.
        let (clients, infra): (Vec<&Device>, Vec<&Device>) =
            m.devices.iter().partition(|d| d.client);
        if p.infra && !infra.is_empty() {
            devices(
                &mut body,
                "infrastructure",
                "Infrastructure",
                &infra,
                show_site,
            );
        }
        if p.clients && !clients.is_empty() {
            devices(&mut body, "clients", "Clients", &clients, show_site);
        }
    }
    if let Some(n) = r.notes.as_deref().filter(|n| !n.trim().is_empty()) {
        let _ = write!(
            body,
            "<h2 id=\"notes\">Technician notes</h2>\n<div class=\"notes\">\n{}</div>\n",
            crate::markdown_out::body_html(n)
        );
    }
    if !r.runs.is_empty() {
        runs(&mut body, &r.runs);
    }
    let _ = writeln!(
        body,
        "<footer>Made with is-alive {} on the technician's computer. Configs were sanitized \
         and notes redacted there before anything was written. Devices and links come from \
         what was observed and the config files opened: a link marked <i>assumed</i> \
         wasn't in any of them.</footer>",
        crate::VERSION
    );
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{POLICY}\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        escape(title)
    )
}

/// The sections the page has, for its contents line: (anchor, heading).
fn contents(r: &Report) -> Vec<(&'static str, &'static str)> {
    let mut v = Vec::new();
    if let Some(m) = &r.map {
        let p = r.parts;
        if p.picture {
            v.push(("map", "Map"));
        }
        if p.findings {
            v.push(("findings", "Findings"));
        }
        if p.changes && m.since.is_some() {
            v.push(("changes", "Changes"));
        }
        let clients = m.devices.iter().filter(|d| d.client).count();
        if p.infra && clients < m.devices.len() {
            v.push(("infrastructure", "Infrastructure"));
        }
        if p.clients && clients > 0 {
            v.push(("clients", "Clients"));
        }
    }
    if r.notes.as_deref().is_some_and(|n| !n.trim().is_empty()) {
        v.push(("notes", "Notes"));
    }
    if !r.runs.is_empty() {
        v.push(("results", "Test results"));
    }
    v
}

/// The tiles at the top: devices, findings, changes, and where it ran from.
fn glance(body: &mut String, r: &Report) {
    let mut tiles: Vec<(String, String)> = Vec::new();
    if let Some(m) = &r.map {
        let clients = m.devices.iter().filter(|d| d.client).count();
        tiles.push((
            "Devices".into(),
            format!(
                "{} infrastructure · {clients} client{}",
                m.devices.len() - clients,
                if clients == 1 { "" } else { "s" }
            ),
        ));
        tiles.push(("Findings".into(), finding_counts(&m.findings)));
        if let Some(s) = &m.since {
            tiles.push((format!("Since {}", s.day), s.diff.counts()));
        }
    }
    if !r.from.is_empty() {
        let v: Vec<String> = r.from.iter().map(|(k, v)| format!("{k} {v}")).collect();
        tiles.push(("Scanned from".into(), v.join(" · ")));
    }
    if tiles.is_empty() {
        return;
    }
    body.push_str("<div class=\"glance\">\n");
    for (k, v) in tiles {
        let _ = writeln!(
            body,
            "<div class=\"tile\"><b>{}</b>{}</div>",
            escape(&k),
            escape(&v)
        );
    }
    body.push_str("</div>\n");
}

/// "1 high · 2 med", or "None".
fn finding_counts(f: &[Finding]) -> String {
    let v: Vec<String> = [Sev::High, Sev::Med, Sev::Low]
        .into_iter()
        .filter_map(|s| {
            let n = f.iter().filter(|x| x.sev == s).count();
            (n > 0).then(|| format!("{n} {}", s.label()))
        })
        .collect();
    if v.is_empty() {
        "None".into()
    } else {
        v.join(" · ")
    }
}

fn picture(body: &mut String, m: &MapPart) {
    // The picture carries the map's own title and what it was built from.
    let _ = writeln!(
        body,
        "<h2 id=\"map\">Network map</h2>\n<p class=\"sub\">{}, as laid out on the Map tab</p>",
        escape(&m.source)
    );
    let alt = match m.title.trim() {
        "" => "Network map",
        t => t,
    };
    let _ = writeln!(
        body,
        "<div class=\"map\"><img alt=\"{}\" src=\"data:image/svg+xml;base64,{}\"></div>",
        escape(alt),
        STANDARD.encode(&m.svg)
    );
}

fn findings(body: &mut String, f: &[Finding]) {
    body.push_str("<h2 id=\"findings\">Findings</h2>\n");
    if f.is_empty() {
        body.push_str(
            "<p class=\"weak\">None found in what was observed and the config files opened.</p>\n",
        );
        return;
    }
    body.push_str("<table>\n<tr><th>Severity</th><th>Finding</th><th>Devices</th></tr>\n");
    for x in f {
        let _ = writeln!(
            body,
            "<tr><td><span class=\"sev {0}\">{0}</span></td><td>{1}</td><td>{2}</td></tr>",
            x.sev.label(),
            escape(&x.text),
            escape(&x.devices.join(", "))
        );
    }
    body.push_str("</table>\n");
}

fn since(body: &mut String, s: &Since) {
    let d = &s.diff;
    let _ = writeln!(
        body,
        "<h2 id=\"changes\">Changes since “{}”</h2>\n<p class=\"sub\">Saved {} · {}</p>",
        escape(&s.name),
        escape(&s.day),
        escape(&d.counts())
    );
    if d.is_empty() {
        let _ = writeln!(
            body,
            "<p class=\"weak\">No changes: {} devices as they were.</p>",
            d.same
        );
    } else {
        body.push_str(
            "<table>\n<tr><th></th><th>Device</th><th>Kind</th><th>IP</th><th>MAC</th>\
             <th>What</th></tr>\n",
        );
        let row = |body: &mut String,
                   (class, what): (&str, &str),
                   (label, kind, ip, m): (&str, &str, Option<Ipv4Addr>, Option<Mac>),
                   detail: String| {
            let _ = writeln!(
                body,
                "<tr><td class=\"{class}\">{what}</td><td>{}</td><td>{}</td><td class=\"mono\">{}\
                 </td><td class=\"mono\">{}</td><td>{}</td></tr>",
                escape(label),
                escape(kind),
                ip.map(|i| i.to_string()).unwrap_or_default(),
                m.map(|m| mac::fmt(&m)).unwrap_or_default(),
                escape(&detail)
            );
        };
        for h in &d.new {
            row(
                body,
                ("new", "New"),
                (&h.label, &h.kind, h.ip, h.mac),
                String::new(),
            );
        }
        for (h, cs) in &d.changed {
            let what: Vec<String> = cs.iter().map(Change::text).collect();
            row(
                body,
                ("changed", "Changed"),
                (&h.label, &h.kind, h.ip, h.mac),
                what.join("; "),
            );
        }
        for g in &d.gone {
            row(
                body,
                ("gone", "Gone"),
                (&g.label, &g.kind, g.ip, g.mac),
                String::new(),
            );
        }
        body.push_str("</table>\n");
    }
    let mut notes = Vec::new();
    if !d.ports {
        notes.push("Open ports weren't compared: they weren't checked on both maps.");
    }
    if !d.clients {
        notes.push("New clients weren't looked for: the saved map didn't hold every client.");
    }
    for n in notes {
        let _ = writeln!(body, "<p class=\"weak\">{n}</p>");
    }
}

/// A device table's column: its heading, whether it's monospace, and the
/// cell (HTML, escaped) for a device.
type Column<'a> = (&'a str, bool, Box<dyn Fn(&Device) -> String>);

fn devices(body: &mut String, id: &str, heading: &str, list: &[&Device], site: bool) {
    let _ = writeln!(body, "<h2 id=\"{id}\">{heading} ({})</h2>", list.len());
    let clients = id == "clients";
    let mut cols: Vec<Column> = Vec::new();
    if !clients {
        cols.push((
            "Role",
            false,
            Box::new(|d| match d.kind.trim() {
                "" => escape(&d.role),
                k => escape(&format!("{} · {}", d.role, k.to_lowercase())),
            }),
        ));
    }
    if site {
        cols.push(("Site", false, Box::new(|d| escape(&d.site))));
    }
    cols.push((
        "IP",
        true,
        Box::new(|d| d.ip.map(|i| i.to_string()).unwrap_or_default()),
    ));
    cols.push((
        "MAC",
        true,
        Box::new(|d| d.mac.map(|m| mac::fmt(&m)).unwrap_or_default()),
    ));
    cols.push(("Vendor", false, Box::new(|d| escape(&d.vendor))));
    cols.push((
        if clients { "Connected to" } else { "Uplink" },
        false,
        Box::new(|d| match (d.uplink.is_empty(), d.assumed) {
            (false, true) => format!("{} <i>(assumed)</i>", escape(&d.uplink)),
            _ => escape(&d.uplink),
        }),
    ));
    cols.push((
        "Open ports",
        true,
        Box::new(|d| {
            d.ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        }),
    ));
    cols.push(("Note", false, Box::new(|d| escape(&d.note))));
    // A column with nothing in it for any of these devices stays out.
    cols.retain(|(_, _, cell)| list.iter().any(|d| !cell(d).is_empty()));

    body.push_str("<table>\n<tr><th>Name</th>");
    for (head, ..) in &cols {
        let _ = write!(body, "<th>{head}</th>");
    }
    body.push_str("</tr>\n");
    for d in list {
        let [r, g, b] = d.rgb;
        let _ = write!(
            body,
            "<tr><td><span class=\"dot\" style=\"background:#{r:02x}{g:02x}{b:02x}\"></span>{}",
            escape(&d.name)
        );
        for f in &d.flags {
            let _ = write!(body, " <span class=\"chip\">{}</span>", escape(f));
        }
        body.push_str("</td>");
        for (_, mono, cell) in &cols {
            let class = if *mono { " class=\"mono\"" } else { "" };
            let _ = write!(body, "<td{class}>{}</td>", cell(d));
        }
        body.push_str("</tr>\n");
    }
    body.push_str("</table>\n");
}

fn runs(body: &mut String, runs: &[Run]) {
    let _ = writeln!(
        body,
        "<h2 id=\"results\">Test results ({})</h2>",
        runs.len()
    );
    for r in runs {
        let head = match &r.target {
            Some(t) => format!("{} · {t}", r.tool),
            None => r.tool.clone(),
        };
        let _ = writeln!(
            body,
            "<h3>{}</h3>\n<p class=\"sub\">Started {}, ended {}</p>\n<pre>{}</pre>",
            escape(&head),
            escape(&r.started),
            escape(&r.ended),
            escape(r.text.trim_end())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::compare::{Gone, Here};

    const EVIL: &str = "<script>alert(1)</script>\"'&";

    fn device(name: &str, client: bool) -> Device {
        Device {
            name: name.into(),
            role: if client { "Clients" } else { "Switches" }.into(),
            kind: "SWITCH".into(),
            ip: Some(Ipv4Addr::new(10, 0, 0, 2)),
            mac: Some([0xfc, 0xec, 0xda, 1, 2, 3]),
            vendor: "Ubiquiti".into(),
            uplink: "gateway · Port 1".into(),
            ports: vec![22, 443],
            client,
            ..Default::default()
        }
    }

    fn report(map: Option<MapPart>) -> Report {
        Report {
            title: "Harbor Dental".into(),
            made: "2026-11-09 20:41".into(),
            from: vec![
                ("LAN", "10.0.0.50/24".into()),
                ("Gateway", "10.0.0.1".into()),
            ],
            map,
            parts: Parts {
                picture: true,
                findings: true,
                changes: true,
                infra: true,
                clients: true,
            },
            notes: None,
            runs: Vec::new(),
        }
    }

    fn map() -> MapPart {
        MapPart {
            source: "Live map".into(),
            title: String::new(),
            caption: None,
            svg: "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>x()</script></svg>".into(),
            findings: vec![Finding {
                sev: Sev::High,
                text: "Telnet open".into(),
                devices: vec!["usw-office".into()],
            }],
            devices: vec![device("usw-office", false), device("frontdesk-pc", true)],
            since: None,
        }
    }

    #[test]
    fn a_page_of_its_own_that_loads_and_runs_nothing() {
        let h = html(&report(Some(map())));
        assert!(h.starts_with("<!doctype html>"));
        assert!(h.contains(POLICY));
        // The map is a picture: its markup never lands in the page.
        assert!(!h.contains("<svg"));
        assert!(!h.contains("<script"));
        assert!(h.contains("src=\"data:image/svg+xml;base64,"));
        assert!(h.contains("<h1>Harbor Dental</h1>"));
        assert!(h.contains("Infrastructure (1)"));
        assert!(h.contains("Clients (1)"));
        assert!(h.contains("1 infrastructure · 1 client"));
        assert!(h.contains("1 high"));
        assert!(h.contains("fc:ec:da:01:02:03"));
        assert!(h.contains("LAN 10.0.0.50/24 · Gateway 10.0.0.1"));
    }

    #[test]
    fn everything_from_the_network_is_escaped() {
        let mut m = map();
        m.title = EVIL.into();
        m.caption = Some(EVIL.into());
        m.findings[0].text = EVIL.into();
        m.findings[0].devices = vec![EVIL.into()];
        let mut d = device(EVIL, false);
        d.vendor = EVIL.into();
        d.note = EVIL.into();
        d.site = EVIL.into();
        d.uplink = EVIL.into();
        d.assumed = true;
        d.flags = vec![EVIL.into()];
        m.devices = vec![d];
        m.since = Some(Since {
            name: EVIL.into(),
            day: EVIL.into(),
            diff: Diff {
                new: vec![Here {
                    id: "x".into(),
                    label: EVIL.into(),
                    kind: EVIL.into(),
                    ip: None,
                    mac: None,
                    client: false,
                }],
                ..Default::default()
            },
        });
        let mut r = report(Some(m));
        r.title = EVIL.into();
        r.notes = Some(EVIL.into());
        r.runs = vec![Run {
            tool: EVIL.into(),
            target: Some(EVIL.into()),
            started: EVIL.into(),
            ended: EVIL.into(),
            text: EVIL.into(),
        }];
        let h = html(&r);
        assert!(!h.contains("<script"), "{h}");
        assert!(!h.contains("\"'&<"));
        assert!(h.contains("&lt;script&gt;"));
    }

    #[test]
    fn only_the_chosen_sections_go_in() {
        let mut r = report(Some(map()));
        r.parts = Parts {
            picture: false,
            findings: true,
            changes: true,
            infra: true,
            clients: false,
        };
        let h = html(&r);
        assert!(!h.contains("id=\"map\""));
        assert!(!h.contains("data:image/svg+xml"));
        assert!(!h.contains("frontdesk-pc"));
        assert!(h.contains("usw-office"));
        // The count stays: the page says clients were there, not who.
        assert!(h.contains("1 infrastructure · 1 client"));
        // No comparison: no Changes section, even when asked for.
        assert!(!h.contains("id=\"changes\""));
        // No clients: no Clients table or link to it, even when asked for.
        let mut m = map();
        m.devices.retain(|d| !d.client);
        let h = html(&report(Some(m)));
        assert!(!h.contains("id=\"clients\""));
        assert!(!h.contains("href=\"#clients\""));
        assert!(h.contains("0 clients"));
    }

    #[test]
    fn changes_since_the_last_visit() {
        let mut m = map();
        m.since = Some(Since {
            name: "HQ visit, October".into(),
            day: "2026-10-01".into(),
            diff: Diff {
                changed: vec![(
                    Here {
                        id: "fs".into(),
                        label: "File server".into(),
                        kind: "SERVER".into(),
                        ip: Some(Ipv4Addr::new(10, 0, 0, 5)),
                        mac: None,
                        client: false,
                    },
                    vec![Change::Opened(vec![3389])],
                )],
                gone: vec![Gone {
                    label: "Office printer".into(),
                    kind: "PRINTER".into(),
                    ip: Some(Ipv4Addr::new(10, 0, 0, 30)),
                    mac: None,
                    client: false,
                }],
                ports: true,
                ..Default::default()
            },
        });
        let h = html(&report(Some(m)));
        assert!(h.contains("Changes since “HQ visit, October”"));
        assert!(h.contains("Newly open: 3389"));
        assert!(h.contains("Office printer"));
        assert!(h.contains("1 changed · 1 gone"));
        assert!(!h.contains("Open ports weren't compared"));
        assert!(h.contains("New clients weren't looked for"));
    }

    #[test]
    fn no_map_still_makes_a_page() {
        let mut r = report(None);
        r.title = "  ".into();
        r.notes = Some("## Front office\n- [x] Rebooted the switch".into());
        let h = html(&r);
        assert!(h.contains("<h1>Site report</h1>"));
        assert!(h.contains("Technician notes"));
        assert!(h.contains("<h2>Front office</h2>"));
        assert!(!h.contains("<b>Devices</b>"));
        assert!(!h.contains("<nav>"), "one section needs no contents line");
    }
}
