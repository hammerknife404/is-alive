use crate::job::{Job, pump};
use crate::net::{self, dnsq};
use crate::widgets::{self, Action};
use eframe::egui::{self, RichText, Ui};
use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::time::Instant;

/// Where a lookup is sent.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    /// The OS resolver (hosts file, DNS suffixes) for addresses; the
    /// configured DNS server for every other record type.
    System,
    Public(&'static str, &'static str),
    /// The zone's own nameserver: what the domain publishes, before caching.
    Authoritative,
    /// Every source side by side, to spot split DNS or stale caches.
    Compare,
}

const SOURCES: &[Source] = &[
    Source::System,
    Source::Public("Cloudflare", "1.1.1.1"),
    Source::Public("Google", "8.8.8.8"),
    Source::Public("Quad9", "9.9.9.9"),
    Source::Authoritative,
    Source::Compare,
];

impl Source {
    fn label(self) -> String {
        match self {
            Source::System => "System".into(),
            Source::Public(n, ip) => format!("{n} {ip}"),
            Source::Authoritative => "Authoritative".into(),
            Source::Compare => "Compare all".into(),
        }
    }
}

/// One source's answer: (type, value) rows, or why it failed.
type Rows = Result<Vec<(String, String)>, String>;

/// "A + AAAA" first (the default), then single record types.
const ADDRS: &str = "A + AAAA";

enum Ev {
    /// (source, type, value)
    Record(String, String, String),
    Note(String),
    Done(f64),
    Error(String),
}

pub struct DnsTool {
    query: String,
    rtype: &'static str,
    source: Source,
    job: Option<Job<Ev>>,
    run_started: Option<String>,
    /// Query, type and source of the last run, for the report header.
    ran: String,
    records: Vec<(String, String, String)>,
    notes: Vec<String>,
    took: Option<f64>,
    err: Option<String>,
}

impl Default for DnsTool {
    fn default() -> Self {
        Self {
            query: String::new(),
            rtype: ADDRS,
            source: Source::System,
            job: None,
            run_started: None,
            ran: String::new(),
            records: Vec::new(),
            notes: Vec::new(),
            took: None,
            err: None,
        }
    }
}

impl DnsTool {
    /// Drain worker messages. Called every frame so results stay current
    /// while another tab is shown.
    pub fn poll(&mut self) -> bool {
        pump(&mut self.job, |ev| match ev {
            Ev::Record(s, k, v) => self.records.push((s, k, v)),
            Ev::Note(n) => self.notes.push(n),
            Ev::Done(ms) => self.took = Some(ms),
            Ev::Error(e) => self.err = Some(e),
        })
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let running = self.poll();

        let mut enter = false;
        ui.horizontal(|ui| {
            enter |= widgets::field(
                ui,
                "Name or IP",
                &mut self.query,
                220.0,
                "example.com or 8.8.8.8",
            );
            ui.label("Type");
            egui::ComboBox::from_id_salt("dns-type")
                .selected_text(self.rtype)
                .width(80.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.rtype, ADDRS, ADDRS);
                    for (name, _) in dnsq::TYPES {
                        ui.selectable_value(&mut self.rtype, *name, *name);
                    }
                });
            ui.label("Resolver");
            egui::ComboBox::from_id_salt("dns-source")
                .selected_text(self.source.label())
                .show_ui(ui, |ui| {
                    for s in SOURCES {
                        ui.selectable_value(&mut self.source, *s, s.label());
                    }
                });
            ui.separator();
            match widgets::run_button(ui, running, enter) {
                Action::Start => self.start(ui.ctx()),
                Action::Stop => self.job.as_ref().map_or((), Job::stop),
                Action::None => {}
            }
        });
        ui.weak(match self.source {
            Source::System => {
                "System: addresses as apps see them (hosts file, DNS suffixes); other types from this machine's DNS server."
            }
            Source::Authoritative => {
                "Authoritative: asks the zone's own nameserver, so you see what is published before any caching."
            }
            Source::Compare => {
                "Compare: asks every resolver and the authoritative server, and flags answers that differ (split DNS, stale cache)."
            }
            Source::Public(..) => "Asks this public resolver directly: what the internet sees.",
        });
        widgets::error(ui, &self.err);
        for n in &self.notes {
            ui.label(RichText::new(format!("⚠ {n}")).color(widgets::WARN));
        }
        ui.separator();

        ui.horizontal(|ui| {
            if let Some(ms) = self.took {
                ui.label(format!(
                    "{} record(s) in {}",
                    self.records.len(),
                    widgets::fmt_ms(ms)
                ));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!self.records.is_empty(), |ui| {
                    widgets::save_button(ui, "dns", || self.report());
                    widgets::copy_button(ui, || self.report());
                });
            });
        });
        if self.records.is_empty() && !running {
            widgets::empty_hint(ui, "Enter a name or IP and press Enter.");
            return;
        }
        let show_source = self.records.iter().any(|r| r.0 != self.records[0].0);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Grid::new("dns")
                    .striped(true)
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        for (s, k, v) in &self.records {
                            if show_source {
                                ui.weak(s);
                            }
                            ui.label(RichText::new(k).strong());
                            ui.add(egui::Label::new(RichText::new(v).monospace()).selectable(true));
                            ui.end_row();
                        }
                    });
            });
    }

    /// Start time and logged text of a run that has just ended, once.
    pub fn take_finished(&mut self) -> Option<(String, String)> {
        if self.job.is_some() {
            return None;
        }
        let started = self.run_started.take()?;
        Some((started, crate::log::body(self.output(), &self.err)))
    }

    /// Start time of the run in progress.
    pub fn running_since(&self) -> Option<&str> {
        self.job.as_ref().and(self.run_started.as_deref())
    }

    /// Report text for the Results tab; None until something has run.
    pub fn output(&self) -> Option<String> {
        (!self.records.is_empty()).then(|| self.report())
    }

    fn report(&self) -> String {
        let mut s = format!("DNS lookup of {}\n", self.ran);
        for n in &self.notes {
            s += &format!("! {n}\n");
        }
        let w = self.records.iter().map(|r| r.0.len()).max().unwrap_or(0);
        for (src, k, v) in &self.records {
            s += &format!("{src:<w$}  {k:<6} {v}\n");
        }
        s
    }

    /// Start a run with the given target (used by the full scan).
    pub fn run(&mut self, ctx: &egui::Context, query: &str) {
        self.query = query.into();
        self.rtype = ADDRS;
        self.source = Source::System;
        self.start(ctx);
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.err = None;
        self.took = None;
        self.records.clear();
        self.notes.clear();
        let q = self.query.trim().trim_end_matches('.').to_ascii_lowercase();
        if q.is_empty() {
            self.err = Some("enter a name or IP".into());
            return;
        }
        // An IP is always a reverse (PTR) lookup.
        let types: Vec<u16> = if q.parse::<IpAddr>().is_ok() {
            vec![dnsq::PTR]
        } else if self.rtype == ADDRS {
            vec![dnsq::A, dnsq::AAAA]
        } else {
            dnsq::TYPES
                .iter()
                .filter(|t| t.0 == self.rtype)
                .map(|t| t.1)
                .collect()
        };
        let source = self.source;
        let shown = if types == [dnsq::PTR] {
            "PTR"
        } else {
            self.rtype
        };
        self.ran = format!("{q} ({shown}) via {}", source.label());
        self.run_started = Some(crate::log::now());
        self.job = Some(Job::spawn(ctx, move |tx| {
            let t0 = Instant::now();
            let sources: Vec<Source> = if source == Source::Compare {
                SOURCES[..SOURCES.len() - 1].to_vec()
            } else {
                vec![source]
            };
            let results: Vec<(String, Rows)> = std::thread::scope(|s| {
                let hs: Vec<_> = sources
                    .iter()
                    .map(|&src| {
                        let (q, types) = (&q, &types);
                        s.spawn(move || lookup(src, q, types))
                    })
                    .collect();
                hs.into_iter().filter_map(|h| h.join().ok()).collect()
            });
            let compare = sources.len() > 1;
            let mut sets = Vec::new();
            for (label, r) in &results {
                match r {
                    // One error slot: in Compare each empty source gets a note.
                    Ok(rows) if rows.is_empty() && compare => {
                        tx.send(Ev::Note(format!("{label}: no records")))
                    }
                    Ok(rows) if rows.is_empty() => {
                        tx.send(Ev::Error("no records of that type".into()))
                    }
                    Ok(rows) => {
                        for (k, v) in rows {
                            tx.send(Ev::Record(label.clone(), k.clone(), v.clone()));
                        }
                    }
                    Err(e) if compare => {
                        tx.send(Ev::Record(label.clone(), "error".into(), e.clone()))
                    }
                    Err(e) => tx.send(Ev::Error(e.clone())),
                }
                if let Ok(rows) = r {
                    let set = answer_set(rows, &types);
                    // An authoritative server answers an alias with just the
                    // CNAME when the target is in another zone: not a difference.
                    let alias_only = set.is_empty() && rows.iter().any(|(k, _)| k == "CNAME");
                    if !alias_only {
                        sets.push((label, set));
                    }
                }
            }
            if compare {
                for n in differences(&sets) {
                    tx.send(Ev::Note(n));
                }
            }
            tx.send(Ev::Done(t0.elapsed().as_secs_f64() * 1000.0));
        }));
    }
}

/// The records of the requested types (not CNAMEs met on the way), so
/// sources that do and don't chase an alias compare equal.
fn answer_set(rows: &[(String, String)], types: &[u16]) -> BTreeSet<String> {
    let names: Vec<String> = types.iter().map(|&t| dnsq::type_name(t)).collect();
    rows.iter()
        .filter(|(k, _)| names.contains(k))
        .map(|(k, v)| format!("{k} {v}"))
        .collect()
}

/// One note per source whose answer differs from the first source's.
fn differences(sets: &[(&String, BTreeSet<String>)]) -> Vec<String> {
    let Some((base_label, base)) = sets.first() else {
        return Vec::new();
    };
    sets[1..]
        .iter()
        .filter(|(_, s)| s != base)
        .map(|(label, _)| {
            format!(
                "{label} answers differently from {base_label} (normal for CDN / geo-DNS names; for your own records it means split DNS or a stale cache)"
            )
        })
        .collect()
}

/// Query one source. Returns its display label and (type, value) rows.
fn lookup(src: Source, q: &str, types: &[u16]) -> (String, Rows) {
    match src {
        // The OS resolver answers addresses and reverse lookups of an IP; a
        // PTR query for a name (x.in-addr.arpa) goes to the DNS server.
        Source::System
            if types == [dnsq::A, dnsq::AAAA]
                || (types == [dnsq::PTR] && q.parse::<IpAddr>().is_ok()) =>
        {
            ("System".into(), system_lookup(q))
        }
        Source::System => match dnsq::system_servers().first() {
            Some(&ip) => {
                let label = format!("System {ip}");
                (
                    label,
                    direct(dnsq::Client::new(SocketAddr::new(ip, 53)), q, types),
                )
            }
            None => ("System".into(), Err("no system DNS server found".into())),
        },
        Source::Public(_, ip) => {
            let addr = SocketAddr::new(ip.parse().expect("valid resolver IP"), 53);
            (src.label(), direct(dnsq::Client::new(addr), q, types))
        }
        Source::Authoritative => match authoritative(q) {
            Ok((ns, c)) => (format!("Auth {ns}"), direct(c, q, types)),
            Err(e) => ("Authoritative".into(), Err(e)),
        },
        Source::Compare => unreachable!("Compare is expanded into its sources"),
    }
}

/// Addresses through the OS resolver, plus where each one points back to.
fn system_lookup(q: &str) -> Rows {
    if let Ok(ip) = q.parse::<IpAddr>() {
        return Ok(net::reverse(ip)
            .map(|n| vec![("PTR".into(), n)])
            .unwrap_or_default());
    }
    let ips = net::lookup_all(q).map_err(|e| format!("lookup failed: {e}"))?;
    let mut rows: Vec<(String, String)> = ips
        .iter()
        .map(|ip| {
            let kind = if ip.is_ipv4() { "A" } else { "AAAA" };
            (kind.to_string(), ip.to_string())
        })
        .collect();
    // Reverse each address: shows where a name really lands.
    for ip in ips {
        if let Some(n) = net::reverse(ip) {
            rows.push(("PTR".into(), format!("{ip} ➡ {n}")));
        }
    }
    Ok(rows)
}

/// Raw query of each type; answers include any CNAME chain on the way.
fn direct(c: dnsq::Client, q: &str, types: &[u16]) -> Rows {
    let name = match q.parse::<IpAddr>() {
        Ok(ip) => dnsq::reverse_name(ip),
        Err(_) => q.to_string(),
    };
    let mut rows: Vec<(String, String)> = Vec::new();
    for &t in types {
        let m = c.ask(&name, t).map_err(|e| e.to_string())?;
        if m.rcode == dnsq::NXDOMAIN {
            return Err(format!("{name} does not exist (NXDOMAIN)"));
        }
        for rr in m.answers {
            let row = (dnsq::type_name(rr.rtype), rr.data.to_string());
            if !rows.contains(&row) {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

/// A client for the first reachable nameserver of the zone holding `q`.
fn authoritative(q: &str) -> Result<(String, dnsq::Client), String> {
    let c = dnsq::Client::new(SocketAddr::from(([1, 1, 1, 1], 53)));
    let name = match q.parse::<IpAddr>() {
        Ok(ip) => dnsq::reverse_name(ip),
        Err(_) => q.to_string(),
    };
    let zone = net::domain::zone_of(&c, &name).map_err(|e| e.to_string())?;
    let ns = c
        .ask(&zone, dnsq::NS)
        .map_err(|e| e.to_string())?
        .names(dnsq::NS);
    for host in &ns {
        let ips = c.ask(host, dnsq::A).map(|m| m.ips()).unwrap_or_default();
        if let Some(&ip) = ips.first() {
            let direct = dnsq::Client::direct(ip);
            // Skip a nameserver that is down rather than failing the lookup.
            if direct.ask(&zone, dnsq::SOA).is_ok() {
                return Ok((host.clone(), direct));
            }
        }
    }
    Err(format!("no reachable nameserver for {zone}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_answer_sets() {
        let types = [dnsq::A, dnsq::AAAA];
        let row = |k: &str, v: &str| (k.to_string(), v.to_string());
        let a = answer_set(&[row("CNAME", "x.cdn"), row("A", "192.0.2.1")], &types);
        let b = answer_set(&[row("A", "192.0.2.1")], &types);
        let c = answer_set(&[row("A", "10.0.0.5")], &types);
        assert_eq!(a, b, "a CNAME on the way doesn't count as a difference");
        let (la, lb, lc) = (
            "System".to_string(),
            "Cloudflare".to_string(),
            "Auth".to_string(),
        );
        assert!(differences(&[(&la, a.clone()), (&lb, b)]).is_empty());
        let d = differences(&[(&la, a), (&lc, c)]);
        assert_eq!(d.len(), 1);
        assert!(d[0].starts_with("Auth answers differently from System"));
    }
}
