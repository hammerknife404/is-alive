//! Domain and mail DNS health checks. Each check validates, not just detects:
//! most real problems are records that exist but are broken.

use super::dnsq::{self, Client, Data, Msg};
use std::collections::{HashMap, HashSet};
use std::io;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Info,
    Pass,
    Warn,
    Fail,
}

impl Status {
    pub fn tag(self) -> &'static str {
        match self {
            Status::Info => "INFO",
            Status::Pass => "PASS",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Check {
    pub findings: Vec<(Status, String)>,
    /// Raw records the findings are based on.
    pub records: Vec<String>,
}

impl Check {
    pub fn status(&self) -> Status {
        self.findings
            .iter()
            .map(|f| f.0)
            .max()
            .unwrap_or(Status::Info)
    }

    fn add(&mut self, s: Status, msg: impl Into<String>) {
        self.findings.push((s, msg.into()));
    }
    fn pass(&mut self, m: impl Into<String>) {
        self.add(Status::Pass, m)
    }
    fn info(&mut self, m: impl Into<String>) {
        self.add(Status::Info, m)
    }
    fn warn(&mut self, m: impl Into<String>) {
        self.add(Status::Warn, m)
    }
    fn fail(&mut self, m: impl Into<String>) {
        self.add(Status::Fail, m)
    }
    fn rec(&mut self, r: impl Into<String>) {
        self.records.push(r.into());
    }

    /// Wrap a check body: a DNS error ends the check as a failure.
    fn run(f: impl FnOnce(&mut Check) -> io::Result<()>) -> Check {
        let mut c = Check::default();
        if let Err(e) = f(&mut c) {
            c.fail(format!("lookup failed: {e}"));
        }
        c
    }
}

/// Check names in display order; the first `CORE` are the core email checks.
pub const NAMES: [&str; 13] = [
    "MX",
    "SPF",
    "DMARC",
    "DKIM",
    "Nameservers",
    "MTA-STS",
    "TLS-RPT",
    "BIMI",
    "DNSSEC",
    "CAA",
    "MX reverse DNS",
    "Autodiscover",
    "Domain expiry",
];
pub const CORE: usize = 5;

/// Common DKIM selectors; DKIM keys can't be listed, only probed.
pub const SELECTORS: &[&str] = &[
    "selector1", // Microsoft 365
    "selector2",
    "google", // Google Workspace
    "k1",     // Mailchimp / Mandrill
    "k2",
    "k3",
    "s1", // SendGrid
    "s2",
    "default",
    "dkim",
    "mail",
    "smtp",
    "mxvault",
];

/// Turn "user@Example.com", "https://example.com/x" etc. into "example.com".
pub fn normalize(input: &str) -> String {
    let s = input.trim();
    let s = s.rsplit('@').next().unwrap_or(s);
    let s = s.split("://").last().unwrap_or(s);
    let s = s.split(['/', '?', '#']).next().unwrap_or(s);
    s.trim_end_matches('.').to_ascii_lowercase()
}

/// Run every check, reporting each as it finishes via `on(index, check)`.
pub fn run(
    c: &Client,
    domain: &str,
    extra_selectors: &[String],
    cancel: &AtomicBool,
    on: impl Fn(usize, Check),
) {
    // MX first: several checks need its hosts and addresses.
    let (check, mx) = mx(c, domain);
    on(0, check);
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    type CheckFn = fn(&Client, &str, &[String], &MxInfo) -> Check;
    let jobs: [CheckFn; 12] = [
        |c, d, _, _| spf(c, d),
        |c, d, _, _| dmarc(c, d),
        |c, d, sels, _| dkim(c, d, sels),
        |c, d, _, _| nameservers(c, d),
        |c, d, _, mx| mta_sts(c, d, mx),
        |c, d, _, _| tls_rpt(c, d),
        |c, d, _, _| bimi(c, d),
        |c, d, _, _| dnssec(c, d),
        |c, d, _, _| caa(c, d),
        |c, _, _, mx| mx_ptr(c, mx),
        |c, d, _, mx| autodiscover(c, d, mx),
        |_, d, _, _| expiry(d),
    ];
    // Detached threads, so Stop returns at once instead of waiting out
    // timeouts; checks still in flight finish into a dropped channel.
    let mx = Arc::new(mx);
    let (tx, rx) = mpsc::channel();
    for (i, job) in jobs.into_iter().enumerate() {
        let (tx, c, d, sels, mx) = (
            tx.clone(),
            c.clone(),
            domain.to_string(),
            extra_selectors.to_vec(),
            mx.clone(),
        );
        std::thread::spawn(move || {
            let _ = tx.send((i + 1, job(&c, &d, &sels, &mx)));
        });
    }
    drop(tx);
    while !cancel.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok((i, check)) => on(i, check),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

// ---------- shared lookups ----------

#[derive(Default)]
pub struct MxInfo {
    pub hosts: Vec<String>,
    pub ips: Vec<IpAddr>,
}

impl MxInfo {
    fn m365(&self) -> bool {
        self.hosts
            .iter()
            .any(|h| h.ends_with(".mail.protection.outlook.com"))
    }
    /// Every MX is a hosted mail/filtering service, whose inbound relays
    /// don't send the domain's mail, so their reverse DNS doesn't matter here.
    fn hosted(&self) -> bool {
        const HOSTED: &[&str] = &[
            ".mail.protection.outlook.com", // Microsoft 365
            ".google.com",                  // Google Workspace / Gmail
            ".googlemail.com",
            ".cf-emailsecurity.net", // Cloudflare Email Security
            ".mimecast.com",
            ".pphosted.com", // Proofpoint
            ".ppe-hosted.com",
            ".barracudanetworks.com",
        ];
        !self.hosts.is_empty()
            && self
                .hosts
                .iter()
                .all(|h| HOSTED.iter().any(|s| h.ends_with(s)))
    }
    fn google(&self) -> bool {
        self.hosts
            .iter()
            .any(|h| h.ends_with(".google.com") || h.ends_with(".googlemail.com"))
    }
}

/// A and AAAA addresses of `host`, plus whether the name is a CNAME.
fn addrs(c: &Client, host: &str) -> io::Result<(Vec<IpAddr>, bool)> {
    let a = c.ask(host, dnsq::A)?;
    let cname = !a.names(dnsq::CNAME).is_empty();
    let mut ips = a.ips();
    ips.extend(c.ask(host, dnsq::AAAA)?.ips());
    Ok((ips, cname))
}

/// TXT records at `name` that start with `prefix` (case-insensitive).
fn txt_with(c: &Client, name: &str, prefix: &str) -> io::Result<Vec<String>> {
    Ok(c.ask(name, dnsq::TXT)?
        .txt()
        .into_iter()
        .filter(|t| {
            t.trim_start()
                .to_ascii_lowercase()
                .starts_with(&prefix.to_ascii_lowercase())
        })
        .collect())
}

/// `tag=value; ...` pairs (DMARC, DKIM, MTA-STS, BIMI, TLS-RPT), tags lowercased.
fn tags(record: &str) -> Vec<(String, String)> {
    record
        .split(';')
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect()
}

fn tag<'a>(t: &'a [(String, String)], key: &str) -> Option<&'a str> {
    t.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// The zone apex holding `name` (the owner of the SOA that answers for it).
fn zone_of(c: &Client, name: &str) -> io::Result<String> {
    let m = c.ask(name, dnsq::SOA)?;
    Ok(m.answers
        .iter()
        .chain(&m.authority)
        .find(|r| r.rtype == dnsq::SOA)
        .map_or_else(|| name.to_string(), |r| r.name.clone()))
}

fn parent(name: &str) -> Option<&str> {
    name.split_once('.')
        .map(|(_, p)| p)
        .filter(|p| !p.is_empty())
}

// ---------- core ----------

fn mx(c: &Client, d: &str) -> (Check, MxInfo) {
    let mut info = MxInfo::default();
    let check = Check::run(|k| {
        let m = c.ask(d, dnsq::MX)?;
        if m.rcode == dnsq::NXDOMAIN {
            k.fail(format!("{d} does not exist (NXDOMAIN)"));
            return Ok(());
        }
        let mut mxs: Vec<(u16, String)> = m
            .of(dnsq::MX)
            .filter_map(|d| match d {
                Data::Mx(p, h) => Some((*p, h.clone())),
                _ => None,
            })
            .collect();
        mxs.sort();
        if mxs.is_empty() {
            k.fail(
                "no MX records: senders fall back to the A record, which is rarely a mail server",
            );
            return Ok(());
        }
        if mxs.len() == 1 && mxs[0].1.is_empty() {
            k.rec("0 .");
            k.info("null MX (RFC 7505): this domain explicitly accepts no mail");
            return Ok(());
        }
        let mut bad = 0;
        for (pref, host) in &mxs {
            if host.parse::<IpAddr>().is_ok() {
                k.rec(format!("{pref:>3} {host}"));
                k.fail(format!("{host} is an IP address; MX must be a host name"));
                bad += 1;
                continue;
            }
            let (ips, cname) = addrs(c, host)?;
            let list: Vec<String> = ips.iter().map(|i| i.to_string()).collect();
            k.rec(format!("{pref:>3} {host}  ➡ {}", list.join(", ")));
            if cname {
                k.warn(format!(
                    "{host} is a CNAME; MX targets must be A/AAAA names (RFC 2181), some senders refuse it"
                ));
            }
            if ips.is_empty() {
                k.fail(format!("{host} does not resolve"));
                bad += 1;
            }
            info.hosts.push(host.clone());
            info.ips.extend(ips);
        }
        if bad == 0 {
            k.pass(format!("{} MX host(s), all resolve", mxs.len()));
        } else if bad == mxs.len() {
            k.fail("no working MX host: inbound mail will bounce");
        }
        if info.m365() {
            k.info("mail goes to Microsoft 365 (Exchange Online Protection)");
        } else if info.google() {
            k.info("mail goes to Google (Gmail / Google Workspace)");
        }
        Ok(())
    });
    (check, info)
}

/// SPF evaluation cost (RFC 7208 §4.6.4): terms that query DNS count on
/// every evaluation (an include reached twice costs twice), and queries
/// that return nothing are void lookups (limit 2).
struct SpfWalk<'a> {
    c: &'a Client,
    /// SPF records per name, so repeated includes are counted but not re-queried.
    cache: HashMap<String, Vec<String>>,
    /// Names on the current include chain, for loop detection.
    path: Vec<String>,
    lookups: usize,
    voids: usize,
}

impl<'a> SpfWalk<'a> {
    fn new(c: &'a Client) -> Self {
        Self {
            c,
            cache: HashMap::new(),
            path: Vec::new(),
            lookups: 0,
            voids: 0,
        }
    }

    fn records(&mut self, name: &str) -> io::Result<Vec<String>> {
        if let Some(r) = self.cache.get(name) {
            return Ok(r.clone());
        }
        let r = txt_with(self.c, name, "v=spf1")?;
        self.cache.insert(name.to_string(), r.clone());
        Ok(r)
    }

    /// Whether an `a`/`mx`/`exists` target has anything to match.
    fn has_answer(&self, mech: &str, target: &str) -> io::Result<bool> {
        if mech == "mx" {
            return Ok(self.c.ask(target, dnsq::MX)?.of(dnsq::MX).next().is_some());
        }
        Ok(!self.c.ask(target, dnsq::A)?.ips().is_empty()
            || (mech == "a" && !self.c.ask(target, dnsq::AAAA)?.ips().is_empty()))
    }

    fn walk(&mut self, name: &str, k: &mut Check) -> io::Result<()> {
        let recs = self.records(name)?;
        let top = self.path.is_empty();
        let Some(rec) = recs.first() else {
            // The top-level record's absence is reported by spf().
            if !top {
                k.fail(format!(
                    "{name} (included) has no SPF record: permerror for the whole SPF check"
                ));
            }
            return Ok(());
        };
        if !top && recs.len() > 1 {
            k.fail(format!("{name} (included) has {} SPF records", recs.len()));
        }
        self.path.push(name.to_string());
        for term in rec.split_whitespace().skip(1) {
            // Past the limit SPF has already failed; stop querying.
            if self.lookups > 10 {
                break;
            }
            let t = term.trim_start_matches(['+', '-', '~', '?']);
            let mech: String = t
                .chars()
                .take_while(|c| !matches!(c, ':' | '=' | '/'))
                .collect::<String>()
                .to_ascii_lowercase();
            let arg = t
                .split_once([':', '='])
                .map(|(_, a)| a.to_ascii_lowercase());
            match mech.as_str() {
                "ptr" => self.lookups += 1,
                "a" | "mx" | "exists" => {
                    self.lookups += 1;
                    // a:host/24, mx/24 (current domain), exists:host
                    let target = arg
                        .as_deref()
                        .map(|a| a.split('/').next().unwrap_or(a).to_string())
                        .unwrap_or_else(|| name.to_string());
                    if target.contains('%') || (mech == "exists" && arg.is_none()) {
                        continue;
                    }
                    if !self.has_answer(&mech, &target)? {
                        self.voids += 1;
                        k.warn(format!(
                            "{term} in {name} matches nothing ({target} has no {} record): void lookup",
                            if mech == "mx" { "MX" } else { "A" }
                        ));
                    }
                }
                "include" | "redirect" => {
                    self.lookups += 1;
                    let Some(target) = arg else { continue };
                    if target.contains('%') {
                        k.info(format!("{term}: macro, not expanded"));
                    } else if self.path.contains(&target) {
                        k.fail(format!(
                            "{term} in {name} loops back to {target}: permerror"
                        ));
                    } else {
                        self.walk(&target, k)?;
                    }
                }
                _ => {}
            }
        }
        self.path.pop();
        Ok(())
    }
}

fn spf(c: &Client, d: &str) -> Check {
    Check::run(|k| {
        let recs = txt_with(c, d, "v=spf1")?;
        for r in &recs {
            k.rec(r);
        }
        let Some(rec) = recs.first() else {
            k.fail("no SPF record: anyone can send as this domain undetected");
            return Ok(());
        };
        if recs.len() > 1 {
            k.fail(format!(
                "{} SPF records: receivers treat this as permerror (SPF fails). Merge them into one",
                recs.len()
            ));
        }
        let terms: Vec<String> = rec
            .split_whitespace()
            .map(|t| t.to_ascii_lowercase())
            .collect();
        // The effective 'all' may live behind redirect= (e.g. gmail.com).
        let (mut cur, mut via, mut hops) = (terms.clone(), String::new(), 0);
        let all = loop {
            let all = cur
                .iter()
                .find(|t| t.trim_start_matches(['+', '-', '~', '?']) == "all");
            if let Some(a) = all {
                break Some(a.clone());
            }
            let Some(target) = cur.iter().find_map(|t| t.strip_prefix("redirect=")) else {
                break None;
            };
            // A missing redirect target is reported by the lookup walk below.
            if target.contains('%') || hops >= 5 {
                break Some(String::new());
            }
            let Some(next) = txt_with(c, target, "v=spf1")?.into_iter().next() else {
                break Some(String::new());
            };
            via += &format!(" ➡ redirect={target}");
            hops += 1;
            cur = next
                .split_whitespace()
                .map(|t| t.to_ascii_lowercase())
                .collect();
        };
        match all.as_deref() {
            Some("") => {}
            Some("-all") => k.pass(format!("ends in -all (hard fail){via}")),
            Some("~all") => k.pass(format!(
                "ends in ~all (soft fail); fine with DMARC enforcing{via}"
            )),
            Some("?all") => k.warn(format!(
                "?all (neutral): SPF never fails, it gives no protection{via}"
            )),
            Some(_) => k.fail(format!(
                "+all: authorizes every server on the internet to send as you{via}"
            )),
            None => k.warn(format!(
                "no 'all' mechanism: unmatched senders get neutral, no protection{via}"
            )),
        }
        if terms.iter().any(|t| {
            t.trim_start_matches(['+', '-', '~', '?'])
                .starts_with("ptr")
        }) {
            k.warn("uses ptr: deprecated (RFC 7208), slow, and ignored by some receivers");
        }
        let mut w = SpfWalk::new(c);
        w.walk(d, k)?;
        match w.lookups {
            n @ 0..=8 => k.pass(format!("{n}/10 DNS lookups")),
            n @ 9..=10 => k.warn(format!(
                "{n}/10 DNS lookups: at the limit, one more include breaks SPF"
            )),
            n => k.fail(format!(
                "over 10 DNS lookups (stopped counting at {n}): receivers return permerror (SPF fails)"
            )),
        }
        if w.voids > 2 {
            k.fail(format!(
                "{} void lookups (limit 2): receivers return permerror (SPF fails)",
                w.voids
            ));
        }
        Ok(())
    })
}

/// DMARC record for `d`, falling back to parent domains (org-domain policy).
/// Returns the record and the domain it was found on.
fn dmarc_record(c: &Client, d: &str) -> io::Result<Option<(String, String)>> {
    let mut name = d;
    loop {
        let recs = txt_with(c, &format!("_dmarc.{name}"), "v=DMARC1")?;
        if let Some(r) = recs.into_iter().next() {
            return Ok(Some((r, name.to_string())));
        }
        match parent(name) {
            // Stop before the TLD.
            Some(p) if p.contains('.') => name = p,
            _ => return Ok(None),
        }
    }
}

fn same_org(a: &str, b: &str) -> bool {
    a == b || a.ends_with(&format!(".{b}")) || b.ends_with(&format!(".{a}"))
}

fn dmarc(c: &Client, d: &str) -> Check {
    Check::run(|k| {
        let all = txt_with(c, &format!("_dmarc.{d}"), "v=DMARC1")?;
        if all.len() > 1 {
            k.fail(format!(
                "{} DMARC records: receivers ignore DMARC entirely. Keep one",
                all.len()
            ));
        }
        let Some((rec, at)) = dmarc_record(c, d)? else {
            k.fail("no DMARC record: spoofed mail from this domain is delivered");
            return Ok(());
        };
        k.rec(format!("_dmarc.{at}  {rec}"));
        if at != d {
            k.info(format!("inherited from the organizational domain {at}"));
        }
        let t = tags(&rec);
        let policy = if at != d {
            tag(&t, "sp").or(tag(&t, "p"))
        } else {
            tag(&t, "p")
        };
        match policy.map(str::to_ascii_lowercase).as_deref() {
            Some("reject") => k.pass("policy reject: spoofed mail is refused"),
            Some("quarantine") => k.pass("policy quarantine: spoofed mail goes to junk"),
            Some("none") => k.warn("policy none: monitoring only, spoofed mail is still delivered"),
            Some(p) => k.fail(format!("invalid policy p={p}")),
            None => k.fail("no p= tag: record is invalid"),
        }
        if at == d
            && let (Some(sp), Some(p)) = (tag(&t, "sp"), tag(&t, "p"))
            && sp.eq_ignore_ascii_case("none")
            && !p.eq_ignore_ascii_case("none")
        {
            k.warn("sp=none: subdomains can be spoofed even though the domain is protected");
        }
        match tag(&t, "pct").map(|p| p.parse::<u32>()) {
            Some(Ok(p)) if p < 100 => k.warn(format!(
                "pct={p}: policy applies to only {p}% of failing mail"
            )),
            Some(Err(_)) => k.warn("pct is not a number"),
            _ => {}
        }
        let rua = tag(&t, "rua").unwrap_or("");
        if rua.is_empty() {
            k.warn(
                "no rua= address: you get no aggregate reports, so you can't see who sends as you",
            );
        }
        // Reports to another domain need that domain's consent (RFC 7489 §7.1).
        for uri in rua
            .split(',')
            .chain(tag(&t, "ruf").unwrap_or("").split(','))
        {
            let Some(addr) = uri.trim().strip_prefix("mailto:") else {
                continue;
            };
            let host = addr.split('!').next().unwrap_or(addr);
            let Some((_, rd)) = host.rsplit_once('@') else {
                continue;
            };
            let rd = rd.to_ascii_lowercase();
            if same_org(&rd, &at) {
                continue;
            }
            let auth = format!("{at}._report._dmarc.{rd}");
            if txt_with(c, &auth, "v=DMARC1")?.is_empty() {
                k.warn(format!(
                    "reports to {rd} aren't authorized ({auth} missing): receivers won't send them"
                ));
            } else {
                k.pass(format!("external report address {rd} is authorized"));
            }
        }
        Ok(())
    })
}

/// RSA modulus size in bits from a DKIM p= key (SubjectPublicKeyInfo or bare RSAPublicKey).
fn rsa_bits(p: &str) -> Option<usize> {
    let der = base64(p)?;
    // Returns (tag, contents, rest).
    fn tlv(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
        let tag = *b.first()?;
        let l = *b.get(1)? as usize;
        let (len, h) = if l < 0x80 {
            (l, 2)
        } else {
            let n = l & 0x7f;
            if n == 0 || n > 3 {
                return None;
            }
            let len = b
                .get(2..2 + n)?
                .iter()
                .fold(0usize, |a, &x| a << 8 | x as usize);
            (len, 2 + n)
        };
        Some((tag, b.get(h..h + len)?, &b[h + len..]))
    }
    let (t, outer, _) = tlv(&der)?;
    if t != 0x30 {
        return None;
    }
    let (t, first, rest) = tlv(outer)?;
    let modulus = match t {
        0x02 => first, // bare RSAPublicKey
        0x30 => {
            let (t, bits, _) = tlv(rest)?;
            if t != 0x03 {
                return None;
            }
            let (_, rsa, _) = tlv(bits.get(1..)?)?;
            tlv(rsa)?.1
        }
        _ => return None,
    };
    let m: Vec<u8> = modulus.iter().copied().skip_while(|&b| b == 0).collect();
    Some(m.len() * 8 - m.first()?.leading_zeros() as usize)
}

fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut n) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((acc >> n) as u8);
        }
    }
    Some(out)
}

fn dkim(c: &Client, d: &str, extra: &[String]) -> Check {
    Check::run(|k| {
        let mut sels: Vec<String> = extra.iter().map(|s| s.to_ascii_lowercase()).collect();
        sels.extend(SELECTORS.iter().map(|s| s.to_string()));
        let mut seen = HashSet::new();
        sels.retain(|s| !s.is_empty() && seen.insert(s.clone()));
        let results: Vec<(String, io::Result<Msg>)> = std::thread::scope(|s| {
            let hs: Vec<_> = sels
                .iter()
                .map(|sel| {
                    let name = format!("{sel}._domainkey.{d}");
                    s.spawn(move || (sel.clone(), c.ask(&name, dnsq::TXT)))
                })
                .collect();
            hs.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let (mut found, mut active, mut revoked) = (0, 0, 0);
        // One lost packet shouldn't discard the selectors that did answer.
        let mut failed = Vec::new();
        for (sel, r) in results {
            let m = match r {
                Ok(m) => m,
                Err(e) => {
                    failed.push(format!("{sel} ({e})"));
                    continue;
                }
            };
            let cname = m.names(dnsq::CNAME);
            let keys: Vec<String> = m.txt().into_iter().filter(|t| t.contains("p=")).collect();
            if keys.is_empty() {
                if let Some(target) = cname.last() {
                    found += 1;
                    k.rec(format!("{sel}  CNAME ➡ {target}  (no key)"));
                    k.warn(format!(
                        "{sel}: CNAME to {target} but no key there; DKIM not enabled at the provider yet?"
                    ));
                }
                continue;
            }
            found += 1;
            let key = &keys[0];
            let target = cname.last();
            let via = target.map(|t| format!(" (via {t})")).unwrap_or_default();
            // A CNAME out of the domain means a provider publishes (and rotates) the key.
            let fix = match target.filter(|t| !t.ends_with(&format!(".{d}"))) {
                Some(t) => format!("published by {t}, only that provider can rotate it"),
                None => "rotate to a 2048-bit key".to_string(),
            };
            let t = tags(key);
            let kt = tag(&t, "k").unwrap_or("rsa").to_ascii_lowercase();
            let p = tag(&t, "p").unwrap_or("");
            if p.is_empty() {
                k.rec(format!("{sel}{via}  revoked (empty p=)"));
                k.info(format!("{sel}: key revoked (empty p=)"));
                revoked += 1;
                continue;
            }
            active += 1;
            if kt == "ed25519" {
                k.rec(format!("{sel}{via}  ed25519"));
                k.pass(format!("{sel}: ed25519 key"));
                continue;
            }
            match rsa_bits(p) {
                Some(b) => {
                    k.rec(format!("{sel}{via}  RSA {b}-bit"));
                    if b < 1024 {
                        k.fail(format!(
                            "{sel}: {b}-bit key, Gmail and others ignore keys under 1024; {fix}"
                        ));
                    } else if b < 2048 {
                        k.warn(format!("{sel}: {b}-bit key; {fix}"));
                    } else {
                        k.pass(format!("{sel}: {b}-bit key"));
                    }
                }
                None => {
                    k.rec(format!("{sel}{via}  unparseable key"));
                    k.fail(format!("{sel}: p= is not a valid RSA public key; {fix}"));
                }
            }
        }
        if revoked > 0 && active == 0 {
            k.warn("no active DKIM key found, only revoked ones; the current selector isn't in the probe list (check a message's s= tag)");
        }
        if failed.len() == sels.len() {
            return Err(io::Error::other(failed.join(", ")));
        }
        if !failed.is_empty() {
            k.warn(format!(
                "{} selector lookup(s) failed, results may be incomplete: {}",
                failed.len(),
                failed.join(", ")
            ));
        }
        if found == 0 {
            k.warn(format!(
                "no DKIM key on {} common selectors; check a received message's DKIM-Signature s= tag and add it above",
                sels.len() - failed.len()
            ));
        }
        Ok(())
    })
}

fn norm_set(v: &[String]) -> Vec<String> {
    let mut v: Vec<String> = v
        .iter()
        .map(|s| s.trim_end_matches('.').to_ascii_lowercase())
        .collect();
    v.sort();
    v.dedup();
    v
}

fn nameservers(c: &Client, d: &str) -> Check {
    Check::run(|k| {
        let zone = zone_of(c, d)?;
        if zone != d {
            k.info(format!("{d} lives in zone {zone}; checking that zone"));
        }
        let ns = norm_set(&c.ask(&zone, dnsq::NS)?.names(dnsq::NS));
        match ns.len() {
            0 => {
                k.fail("no NS records");
                return Ok(());
            }
            1 => k.fail("only 1 nameserver: no redundancy (at least 2 required)"),
            n => k.pass(format!("{n} nameservers")),
        }

        // Ask each nameserver directly for the SOA: it must answer, authoritatively.
        let probes: Vec<(String, Vec<IpAddr>, Result<u32, String>)> = std::thread::scope(|s| {
            let hs: Vec<_> = ns
                .iter()
                .map(|n| {
                    let zone = &zone;
                    s.spawn(move || {
                        let ips = addrs(c, n).map(|r| r.0).unwrap_or_default();
                        let Some(ip) = ips.iter().find(|i| i.is_ipv4()).or(ips.first()) else {
                            return (n.clone(), ips, Err("does not resolve".into()));
                        };
                        let r = match Client::direct(*ip).query(zone, dnsq::SOA, false) {
                            Err(e) => Err(format!("{ip}: {e}")),
                            Ok(m) if m.rcode != dnsq::NOERROR => {
                                Err(format!("{ip}: answered rcode {}", m.rcode))
                            }
                            Ok(m) if !m.aa => {
                                Err(format!("{ip}: not authoritative (lame delegation)"))
                            }
                            Ok(m) => m
                                .of(dnsq::SOA)
                                .find_map(|d| match d {
                                    Data::Soa { serial, .. } => Some(*serial),
                                    _ => None,
                                })
                                .ok_or_else(|| format!("{ip}: no SOA in answer")),
                        };
                        (n.clone(), ips, r)
                    })
                })
                .collect();
            hs.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let mut serials = Vec::new();
        for (n, ips, r) in &probes {
            let list: Vec<String> = ips.iter().map(|i| i.to_string()).collect();
            match r {
                Ok(s) => {
                    k.rec(format!("{n}  ({})  serial {s}", list.join(", ")));
                    serials.push(*s);
                }
                Err(e) => {
                    k.rec(format!("{n}  ({})  ✖", list.join(", ")));
                    k.fail(format!("{n} {e}"));
                }
            }
        }
        serials.sort();
        serials.dedup();
        if serials.len() > 1 {
            k.warn("SOA serials differ between nameservers: zone changes haven't reached them all");
        } else if serials.len() == 1 && probes.iter().all(|p| p.2.is_ok()) {
            k.pass("all nameservers answer authoritatively with the same serial");
        }

        // Delegation at the parent (what the registrar published) vs the zone's own NS set.
        let Some(p) = parent(&zone) else {
            return Ok(());
        };
        let pzone = zone_of(c, p)?;
        let pns = c.ask(&pzone, dnsq::NS)?.names(dnsq::NS);
        let mut delegated = None;
        for host in pns.iter().take(3) {
            let Some(ip) = addrs(c, host)?.0.into_iter().find(IpAddr::is_ipv4) else {
                continue;
            };
            if let Ok(m) = Client::direct(ip).query(&zone, dnsq::NS, false) {
                let mut v: Vec<String> = m
                    .authority
                    .iter()
                    .chain(&m.answers)
                    .filter(|r| r.rtype == dnsq::NS && r.name == zone)
                    .filter_map(|r| match &r.data {
                        Data::Name(n) => Some(n.clone()),
                        _ => None,
                    })
                    .collect();
                v = norm_set(&v);
                if !v.is_empty() {
                    delegated = Some(v);
                    break;
                }
            }
        }
        match delegated {
            None => k.info(format!(
                "couldn't read the delegation from the {pzone} servers"
            )),
            Some(v) if v == ns => {
                k.pass(format!("delegation at {pzone} matches the zone's NS set"))
            }
            Some(v) => {
                k.rec(format!("parent ({pzone}) delegates to: {}", v.join(", ")));
                k.warn(format!(
                    "registrar/parent NS ({}) differ from the zone's NS ({}): update one to match",
                    v.join(", "),
                    ns.join(", ")
                ));
            }
        }
        Ok(())
    })
}

// ---------- nice-to-haves ----------

fn http_get(url: &str, follow: bool) -> Result<String, String> {
    // OS trust store, so TLS-inspecting firewalls and proxies work.
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(10)))
        .max_redirects(if follow { 5 } else { 0 })
        .user_agent(concat!("is-alive/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut r = agent.get(url).call().map_err(|e| e.to_string())?;
    r.body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| e.to_string())
}

/// MX host `h` matches an MTA-STS `mx:` pattern (exact, or `*.` one label).
fn sts_match(pattern: &str, h: &str) -> bool {
    let p = pattern.trim_end_matches('.').to_ascii_lowercase();
    match p.strip_prefix("*.") {
        Some(rest) => h.split_once('.').is_some_and(|(_, tail)| tail == rest),
        None => p == h,
    }
}

fn mta_sts(c: &Client, d: &str, mx: &MxInfo) -> Check {
    Check::run(|k| {
        let recs = txt_with(c, &format!("_mta-sts.{d}"), "v=STSv1")?;
        let Some(rec) = recs.first() else {
            k.info("not configured: inbound mail TLS isn't enforced (senders fall back to plaintext if TLS fails)");
            return Ok(());
        };
        k.rec(format!("_mta-sts.{d}  {rec}"));
        if recs.len() > 1 {
            k.fail("multiple MTA-STS TXT records: senders ignore the policy");
        }
        if tag(&tags(rec), "id").is_none_or(str::is_empty) {
            k.fail("TXT record has no id=");
        }
        let host = format!("mta-sts.{d}");
        let url = format!("https://{host}/.well-known/mta-sts.txt");
        let policy = match http_get(&url, false) {
            Ok(p) => p,
            Err(e) => {
                // Only on failure: say plainly when the host simply doesn't exist.
                let missing = c.ask(&host, dnsq::A).is_ok_and(|m| m.ips().is_empty())
                    && c.ask(&host, dnsq::AAAA).is_ok_and(|m| m.ips().is_empty());
                if missing {
                    k.fail(format!(
                        "{host} doesn't resolve: the policy can't be fetched, so senders ignore MTA-STS. Publish the host or remove the TXT record"
                    ));
                } else {
                    k.fail(format!("policy not reachable at {url}: {e}"));
                }
                return Ok(());
            }
        };
        let mut mode = None;
        let mut max_age = None;
        let mut patterns = Vec::new();
        for line in policy.lines() {
            k.rec(format!("  {}", line.trim()));
            let Some((key, v)) = line.split_once(':') else {
                continue;
            };
            let v = v.trim().to_string();
            match key.trim() {
                "mode" => mode = Some(v),
                "max_age" => max_age = v.parse::<u64>().ok(),
                "mx" => patterns.push(v),
                _ => {}
            }
        }
        match mode.as_deref() {
            Some("enforce") => k.pass("mode enforce: senders refuse to deliver without valid TLS"),
            Some("testing") => k.warn("mode testing: failures are only reported, not enforced"),
            Some("none") => k.warn("mode none: policy disabled"),
            _ => k.fail("policy has no valid mode"),
        }
        match max_age {
            Some(a) if a < 86_400 => k.warn(format!(
                "max_age {a}s is short; 1-2 weeks (604800+) is typical"
            )),
            None => k.fail("policy has no max_age"),
            _ => {}
        }
        for h in &mx.hosts {
            if !patterns.iter().any(|p| sts_match(p, h)) {
                k.fail(format!(
                    "MX {h} isn't listed in the policy: senders in enforce mode won't deliver to it"
                ));
            }
        }
        Ok(())
    })
}

fn tls_rpt(c: &Client, d: &str) -> Check {
    Check::run(|k| {
        let recs = txt_with(c, &format!("_smtp._tls.{d}"), "v=TLSRPTv1")?;
        let Some(rec) = recs.first() else {
            k.info("not configured: you won't hear about TLS failures on inbound mail");
            return Ok(());
        };
        k.rec(format!("_smtp._tls.{d}  {rec}"));
        let t = tags(rec);
        let rua = tag(&t, "rua").unwrap_or("");
        if rua.is_empty() {
            k.fail("no rua= destination: reports have nowhere to go");
        } else if rua
            .split(',')
            .all(|u| u.trim().starts_with("mailto:") || u.trim().starts_with("https:"))
        {
            k.pass(format!("reports go to {rua}"));
        } else {
            k.warn("rua= must be mailto: or https: URIs");
        }
        Ok(())
    })
}

fn bimi(c: &Client, d: &str) -> Check {
    Check::run(|k| {
        let recs = txt_with(c, &format!("default._bimi.{d}"), "v=BIMI1")?;
        let Some(rec) = recs.first() else {
            k.info("not configured (no brand logo in supporting inboxes)");
            return Ok(());
        };
        k.rec(format!("default._bimi.{d}  {rec}"));
        let t = tags(rec);
        match tag(&t, "l") {
            // A query string or fragment (cache busting) is fine after .svg.
            Some(l)
                if l.starts_with("https://")
                    && l.split(['?', '#'])
                        .next()
                        .is_some_and(|p| p.to_ascii_lowercase().ends_with(".svg")) =>
            {
                k.pass("logo is an HTTPS SVG")
            }
            Some("") | None => k.info("no logo (l= empty): declines BIMI"),
            Some(l) => k.fail(format!("logo {l} must be an https:// SVG (Tiny PS)")),
        }
        match tag(&t, "a") {
            Some(a) if !a.is_empty() => {
                k.pass("has a certificate (VMC/CMC): needed for Gmail and Apple")
            }
            _ => k.warn("no a= certificate (VMC/CMC): Gmail and Apple Mail won't show the logo"),
        }
        // BIMI only works with an enforcing DMARC policy at 100%.
        let dm = dmarc_record(c, d)?
            .map(|(r, _)| tags(&r))
            .unwrap_or_default();
        let p = tag(&dm, "p").unwrap_or("").to_ascii_lowercase();
        let pct = tag(&dm, "pct")
            .and_then(|p| p.parse::<u32>().ok())
            .unwrap_or(100);
        if !(p == "quarantine" || p == "reject") || pct < 100 {
            k.fail("DMARC must be p=quarantine or reject at pct=100, otherwise BIMI is ignored");
        } else {
            k.pass(format!("DMARC is p={p}"));
        }
        Ok(())
    })
}

fn dnssec(c: &Client, d: &str) -> Check {
    Check::run(|k| {
        let zone = zone_of(c, d)?;
        let ds: Vec<Data> = c.ask(&zone, dnsq::DS)?.of(dnsq::DS).cloned().collect();
        let keys: Vec<Data> = c
            .query(&zone, dnsq::DNSKEY, true)?
            .of(dnsq::DNSKEY)
            .cloned()
            .collect();
        for r in &ds {
            if let Data::Ds {
                key_tag,
                alg,
                digest_type,
            } = r
            {
                k.rec(format!(
                    "DS  tag {key_tag}  alg {alg}  digest {digest_type}"
                ));
                if *digest_type == 1 {
                    k.warn(format!(
                        "DS {key_tag} uses SHA-1 digest; publish SHA-256 (type 2)"
                    ));
                }
            }
        }
        for r in &keys {
            if let Data::Dnskey { flags, alg } = r {
                let role = if flags & 1 == 1 { "KSK" } else { "ZSK" };
                k.rec(format!("DNSKEY  {role}  alg {alg}"));
            }
        }
        let algs: HashSet<u8> = keys
            .iter()
            .filter_map(|r| match r {
                Data::Dnskey { alg, .. } => Some(*alg),
                _ => None,
            })
            .collect();
        if algs.iter().any(|a| matches!(a, 1 | 3 | 5 | 6 | 7 | 12)) {
            k.warn("uses a deprecated algorithm (RSA/SHA-1, DSA or GOST); move to 8, 13 or 15");
        }
        if ds.is_empty() {
            if keys.is_empty() {
                k.info(format!("{zone} is not signed"));
            } else {
                k.warn("zone is signed but no DS at the registrar: nothing validates it. Publish the DS record");
            }
            return Ok(());
        }
        let probe = c.query(&zone, dnsq::SOA, false)?;
        if probe.rcode == dnsq::SERVFAIL {
            let cd = c.query(&zone, dnsq::SOA, true)?;
            if cd.rcode == dnsq::NOERROR {
                k.fail("DNSSEC is broken (bogus): validating resolvers return SERVFAIL, so the domain is down for them");
            } else {
                k.fail("resolver returns SERVFAIL for the zone");
            }
        } else if keys.is_empty() {
            k.fail("DS at the registrar but the zone has no DNSKEY: validation fails");
        } else if probe.ad {
            k.pass("signed and validates (resolver set AD)");
        } else {
            k.warn("DS present but the resolver didn't mark answers validated; is it a validating resolver?");
        }
        Ok(())
    })
}

fn caa(c: &Client, d: &str) -> Check {
    Check::run(|k| {
        // CAA is inherited from the closest ancestor that has any (RFC 8659).
        let mut name = d;
        let (at, recs) = loop {
            let r: Vec<Data> = c.ask(name, dnsq::CAA)?.of(dnsq::CAA).cloned().collect();
            if !r.is_empty() {
                break (name, r);
            }
            match parent(name) {
                Some(p) if p.contains('.') => name = p,
                _ => {
                    k.info("no CAA: any public CA may issue certificates for this domain");
                    return Ok(());
                }
            }
        };
        if at != d {
            k.info(format!("inherited from {at}"));
        }
        let mut issuers = Vec::new();
        for r in &recs {
            let Data::Caa { flags, tag, value } = r else {
                continue;
            };
            k.rec(format!("{flags} {tag} \"{value}\""));
            match tag.as_str() {
                "issue" | "issuewild" => {
                    let ca = value.split(';').next().unwrap_or("").trim();
                    issuers.push(if ca.is_empty() { format!("{tag}: none") } else { format!("{tag}: {ca}") });
                }
                "iodef" => {}
                _ if flags & 0x80 != 0 => k.warn(format!(
                    "critical unknown tag '{tag}': CAs that don't understand it must refuse to issue"
                )),
                _ => {}
            }
        }
        if issuers.is_empty() {
            k.warn("CAA records but no issue/issuewild tag: doesn't restrict issuance");
        } else {
            k.pass(format!("restricted to {}", issuers.join(", ")));
        }
        Ok(())
    })
}

/// PTR names of an address and whether one resolves back to it.
type PtrResult = (Vec<String>, bool);

fn mx_ptr(c: &Client, mx: &MxInfo) -> Check {
    Check::run(|k| {
        if mx.ips.is_empty() {
            k.info("no MX addresses to check");
            return Ok(());
        }
        let mut ips = mx.ips.clone();
        ips.sort();
        ips.dedup();
        ips.truncate(12);
        let results: Vec<(IpAddr, io::Result<PtrResult>)> = std::thread::scope(|s| {
            let hs: Vec<_> = ips
                .iter()
                .map(|&ip| {
                    s.spawn(move || {
                        let r = (|| {
                            let names = c.ask(&dnsq::reverse_name(ip), dnsq::PTR)?.names(dnsq::PTR);
                            // Forward-confirmed: some PTR name resolves back to the IP.
                            let mut fc = false;
                            for n in &names {
                                if addrs(c, n)?.0.contains(&ip) {
                                    fc = true;
                                    break;
                                }
                            }
                            Ok((names, fc))
                        })();
                        (ip, r)
                    })
                })
                .collect();
            hs.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        // A hosted filter's inbound relays don't send mail, so a missing PTR is
        // informational there; a self-hosted MX usually sends too.
        let hosted = mx.hosted();
        if hosted {
            k.info("MX is a hosted mail service: its inbound relays don't send your mail");
        }
        let mut ok = 0;
        for (ip, r) in results {
            match r {
                Ok((names, _)) if names.is_empty() => {
                    k.rec(format!("{ip}  ➡ (none)"));
                    if hosted {
                        k.info(format!(
                            "{ip} has no PTR record (doesn't matter for a receive-only relay)"
                        ));
                    } else {
                        k.warn(format!(
                            "{ip} has no PTR record: if this server also sends mail, receivers will junk or reject it"
                        ));
                    }
                }
                Ok((names, fc)) => {
                    k.rec(format!("{ip}  ➡ {}", names.join(", ")));
                    if fc {
                        ok += 1;
                    } else if hosted {
                        k.info(format!(
                            "{ip}: PTR {} doesn't resolve back to {ip}",
                            names[0]
                        ));
                    } else {
                        k.warn(format!(
                            "{ip}: PTR {} doesn't resolve back to {ip}; receivers check this for sending servers",
                            names[0]
                        ));
                    }
                }
                Err(e) => k.warn(format!("{ip}: {e}")),
            }
        }
        if ok > 0 {
            k.pass(format!(
                "{ok}/{} MX address(es) have matching forward and reverse DNS",
                ips.len()
            ));
        }
        Ok(())
    })
}

fn autodiscover(c: &Client, d: &str, mx: &MxInfo) -> Check {
    Check::run(|k| {
        let name = format!("autodiscover.{d}");
        let m = c.ask(&name, dnsq::A)?;
        let cname = m.names(dnsq::CNAME);
        let ips = m.ips();
        if let Some(t) = cname.first() {
            k.rec(format!("{name}  CNAME ➡ {t}"));
        } else if !ips.is_empty() {
            let l: Vec<String> = ips.iter().map(|i| i.to_string()).collect();
            k.rec(format!("{name}  A ➡ {}", l.join(", ")));
        }
        let to_m365 = cname
            .first()
            .is_some_and(|t| t == "autodiscover.outlook.com");
        match (to_m365, mx.m365(), cname.is_empty() && ips.is_empty()) {
            (true, _, _) => k.pass("CNAME to autodiscover.outlook.com (Microsoft 365)"),
            (false, true, true) => k.warn(
                "missing: mail is on Microsoft 365, add CNAME autodiscover ➡ autodiscover.outlook.com",
            ),
            (false, true, false) => k.warn(
                "mail is on Microsoft 365 but autodiscover points elsewhere: Outlook may find an old server or prompt for credentials",
            ),
            (false, false, true) => k.info("no autodiscover record (only matters for Exchange / Microsoft 365)"),
            (false, false, false) => k.info("points to a non-Microsoft 365 host (on-prem Exchange or a web host catch-all?)"),
        }
        Ok(())
    })
}

/// RDAP base URL for a TLD from the IANA bootstrap file.
fn rdap_base(bootstrap: &str, tld: &str) -> Option<String> {
    let j: serde_json::Value = serde_json::from_str(bootstrap).ok()?;
    j["services"].as_array()?.iter().find_map(|s| {
        let tlds = s.get(0)?.as_array()?;
        tlds.iter()
            .any(|t| t.as_str().is_some_and(|t| t.eq_ignore_ascii_case(tld)))
            .then(|| s.get(1)?.get(0)?.as_str().map(String::from))?
    })
}

struct Rdap {
    expires: Option<String>,
    registrar: Option<String>,
    status: Vec<String>,
}

fn parse_rdap(body: &str) -> Option<Rdap> {
    let j: serde_json::Value = serde_json::from_str(body).ok()?;
    let expires = j["events"].as_array().and_then(|ev| {
        ev.iter().find_map(|e| {
            (e["eventAction"] == "expiration").then(|| e["eventDate"].as_str().map(String::from))?
        })
    });
    let registrar = j["entities"].as_array().and_then(|es| {
        es.iter().find_map(|e| {
            e["roles"]
                .as_array()?
                .iter()
                .any(|r| r == "registrar")
                .then_some(())?;
            e["vcardArray"][1]
                .as_array()?
                .iter()
                .find_map(|f| (f[0] == "fn").then(|| f[3].as_str().map(String::from))?)
        })
    });
    let status = j["status"]
        .as_array()
        .map(|s| {
            s.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Some(Rdap {
        expires,
        registrar,
        status,
    })
}

fn expiry(d: &str) -> Check {
    let mut k = Check::default();
    let tld = d.rsplit('.').next().unwrap_or(d);
    // The IANA bootstrap file changes rarely; fetch it once per session.
    static BOOTSTRAP: OnceLock<String> = OnceLock::new();
    let boot = match BOOTSTRAP.get() {
        Some(b) => Ok(b.clone()),
        None => http_get("https://data.iana.org/rdap/dns.json", true)
            .inspect(|b| _ = BOOTSTRAP.set(b.clone())),
    };
    let base = match boot {
        Ok(b) => rdap_base(&b, tld),
        Err(e) => {
            k.warn(format!("couldn't reach the IANA RDAP registry: {e}"));
            return k;
        }
    };
    let Some(base) = base else {
        k.info(format!(".{tld} has no RDAP service; check WHOIS manually"));
        return k;
    };
    // Walk up from the name to the registered domain.
    let mut name = d;
    let body = loop {
        let url = format!("{}/domain/{name}", base.trim_end_matches('/'));
        match http_get(&url, true) {
            Ok(b) => break b,
            Err(e) => match parent(name) {
                Some(p) if p.contains('.') => name = p,
                _ => {
                    k.warn(format!("RDAP lookup failed: {e}"));
                    return k;
                }
            },
        }
    };
    let Some(r) = parse_rdap(&body) else {
        k.warn("RDAP answer couldn't be parsed");
        return k;
    };
    k.rec(format!("registered domain  {name}"));
    if let Some(reg) = &r.registrar {
        k.rec(format!("registrar  {reg}"));
    }
    if !r.status.is_empty() {
        k.rec(format!("status  {}", r.status.join(", ")));
    }
    if r.status.iter().any(|s| s.contains("hold")) {
        k.fail("domain is on hold: it doesn't resolve");
    }
    if r.status
        .iter()
        .any(|s| s.contains("redemption") || s.contains("pending delete"))
    {
        k.fail("domain is expired and pending deletion");
    }
    match r.expires {
        None => k.info("registry doesn't publish an expiry date"),
        Some(e) => match chrono::DateTime::parse_from_rfc3339(&e) {
            Err(_) => k.info(format!("expires {e}")),
            Ok(t) => {
                k.rec(format!("expires  {}", t.format("%Y-%m-%d")));
                let days = (t.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_days();
                match days {
                    ..0 => k.fail(format!("expired {} day(s) ago", -days)),
                    0..14 => k.fail(format!("expires in {days} day(s)")),
                    14..45 => k.warn(format!("expires in {days} days; check auto-renew")),
                    _ => k.pass(format!("expires in {days} days")),
                }
            }
        },
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_input() {
        assert_eq!(normalize(" User@Example.COM "), "example.com");
        assert_eq!(
            normalize("https://www.example.com/path?q"),
            "www.example.com"
        );
        assert_eq!(normalize("example.com."), "example.com");
    }

    #[test]
    fn parses_tags() {
        let t = tags("v=DMARC1; p=reject; rua=mailto:a@b.com;pct=50");
        assert_eq!(tag(&t, "p"), Some("reject"));
        assert_eq!(tag(&t, "pct"), Some("50"));
        assert_eq!(tag(&t, "sp"), None);
    }

    #[test]
    fn rsa_key_sizes() {
        let k1024 = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDGpt/taRejdyKY59ZXnJz/a8gbnjW4kSwMiMc9x2J+kFNyT88QG5IbpHEx6uMrRvS6y5AOq/QUDBFNnccruUJSwmddtFUKlD7SpfXiPJeYvs77/ey81FUbbHHSL3QJsKz8ZKJOi0CX+iy6IO/VzHjJMUWmqXr677n1RwXhGR7qPQIDAQAB";
        let k2048 = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsyI2421XCbXzxvX4+DhVzaIE0lEdP3TqPuvPfybCvFAXUJJk+ubpjTF+IVm7sgK/IPWjxViNQjso/mamLKHFIoQQR204HvGO1fKhzbIoWaNmYZNomx6sqrXLdHYGonb/Oig0iBmQUm5oroBdqyjrfNpc1GblNDyokWLNzBruZFRoK1KRksYcYrJR1wZ3ZX2rmtfVVNzaFrQKxNI7xsxZXwjuYOde8Igv+Scy8v4ysClsBibksTmQoFjS479xJtY0F/+M+cE9Ymi3k/qLKf2+1JMXQKLaIhJgurtgltnpmqVlr6/5c1+HjAYSdIrdL7QKg8nbCNUsV4tTnIRiSZyTeQIDAQAB";
        assert_eq!(rsa_bits(k1024), Some(1024));
        assert_eq!(rsa_bits(k2048), Some(2048));
        assert_eq!(rsa_bits("not base64!"), None);
        assert_eq!(rsa_bits("AAAA"), None);
    }

    #[test]
    fn decodes_base64() {
        assert_eq!(base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64("aGVs bG8").unwrap(), b"hello");
    }

    #[test]
    fn sts_patterns() {
        assert!(sts_match("mail.example.com", "mail.example.com"));
        assert!(sts_match(
            "*.mail.protection.outlook.com",
            "example-com.mail.protection.outlook.com"
        ));
        assert!(!sts_match("*.example.com", "a.b.example.com"));
        assert!(!sts_match("mail.example.com", "mx.example.com"));
    }

    #[test]
    fn org_matching() {
        assert!(same_org("example.com", "example.com"));
        assert!(same_org("reports.example.com", "example.com"));
        assert!(!same_org("dmarc.vendor.net", "example.com"));
    }

    #[test]
    fn rdap_bootstrap_and_record() {
        let boot = r#"{"services":[[["net","com"],["https://rdap.verisign.com/com/v1/"]]]}"#;
        assert_eq!(
            rdap_base(boot, "COM").as_deref(),
            Some("https://rdap.verisign.com/com/v1/")
        );
        assert_eq!(rdap_base(boot, "org"), None);
        let body = r#"{"status":["client transfer prohibited"],
            "events":[{"eventAction":"registration","eventDate":"1997-09-15T04:00:00Z"},
                      {"eventAction":"expiration","eventDate":"2028-09-14T04:00:00Z"}],
            "entities":[{"roles":["registrar"],"vcardArray":["vcard",[["version",{},"text","4.0"],["fn",{},"text","MarkMonitor Inc."]]]}]}"#;
        let r = parse_rdap(body).unwrap();
        assert_eq!(r.expires.as_deref(), Some("2028-09-14T04:00:00Z"));
        assert_eq!(r.registrar.as_deref(), Some("MarkMonitor Inc."));
        assert_eq!(r.status, vec!["client transfer prohibited"]);
    }

    /// Tiny UDP DNS server answering from a fixed zone, following one CNAME.
    fn mock(zone: Vec<(&'static str, u16, Vec<u8>)>) -> Client {
        let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            while let Ok((n, from)) = sock.recv_from(&mut buf) {
                let q = &buf[..n];
                let (mut labels, mut i) = (Vec::new(), 12);
                while q[i] != 0 {
                    let l = q[i] as usize;
                    labels.push(String::from_utf8_lossy(&q[i + 1..i + 1 + l]).to_string());
                    i += 1 + l;
                }
                let qtype = u16::from_be_bytes([q[i + 1], q[i + 2]]);
                let mut name = labels.join(".");
                let mut ans = Vec::new();
                for _ in 0..2 {
                    let hit: Vec<_> = zone
                        .iter()
                        .filter(|r| r.0 == name && r.1 == qtype)
                        .collect();
                    if !hit.is_empty() {
                        ans.extend(hit);
                        break;
                    }
                    let Some(cn) = zone.iter().find(|r| r.0 == name && r.1 == dnsq::CNAME) else {
                        break;
                    };
                    ans.push(cn);
                    name = String::from_utf8_lossy(&cn.2[1..cn.2.len() - 1])
                        .chars()
                        .map(|c| if c.is_ascii_graphic() { c } else { '.' })
                        .collect();
                }
                let mut r = q[..i + 5].to_vec();
                // Names under "fail." answer SERVFAIL.
                let rcode = if labels[0] == "fail" { 2 } else { 0 };
                r[2..4].copy_from_slice(&[0x81, 0x80 | rcode]);
                r[6..8].copy_from_slice(&(ans.len() as u16).to_be_bytes());
                r[10..12].copy_from_slice(&[0, 0]);
                for (owner, t, data) in ans {
                    r.extend(wire(owner));
                    r.extend(t.to_be_bytes());
                    r.extend([0, 1, 0, 0, 0, 60]);
                    r.extend((data.len() as u16).to_be_bytes());
                    r.extend(data);
                }
                let _ = sock.send_to(&r, from);
            }
        });
        Client::new(addr)
    }

    fn wire(name: &str) -> Vec<u8> {
        let mut v = Vec::new();
        for l in name.split('.') {
            v.push(l.len() as u8);
            v.extend(l.as_bytes());
        }
        v.push(0);
        v
    }

    fn txt(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks(255)
            .flat_map(|c| std::iter::once(c.len() as u8).chain(c.iter().copied()))
            .collect()
    }

    fn has(k: &Check, s: Status, needle: &str) -> bool {
        k.findings.iter().any(|f| f.0 == s && f.1.contains(needle))
    }

    #[test]
    fn stop_skips_remaining_checks() {
        let c = mock(vec![]);
        let cancel = AtomicBool::new(true);
        let got = std::sync::Mutex::new(Vec::new());
        run(&c, "example.test", &[], &cancel, |i, _| {
            got.lock().unwrap().push(i)
        });
        // Only the MX check (which the others depend on) ran.
        assert_eq!(*got.lock().unwrap(), vec![0]);
    }

    #[test]
    fn mail_checks_against_mock_zone() {
        let key = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDGpt/taRejdyKY59ZXnJz/a8gbnjW4kSwMiMc9x2J+kFNyT88QG5IbpHEx6uMrRvS6y5AOq/QUDBFNnccruUJSwmddtFUKlD7SpfXiPJeYvs77/ey81FUbbHHSL3QJsKz8ZKJOi0CX+iy6IO/VzHjJMUWmqXr677n1RwXhGR7qPQIDAQAB";
        let mut mx_rd = vec![0, 10];
        mx_rd.extend(wire("mx.example.test"));
        let c = mock(vec![
            ("example.test", dnsq::MX, mx_rd.clone()),
            ("mx.example.test", dnsq::A, vec![127, 0, 0, 9]),
            (
                "example.test",
                dnsq::TXT,
                txt("v=spf1 include:a.test include:missing.test ~all"),
            ),
            ("example.test", dnsq::TXT, txt("google-site-verification=x")),
            ("a.test", dnsq::TXT, txt("v=spf1 a mx include:b.test -all")),
            ("b.test", dnsq::TXT, txt("v=spf1 ip4:192.0.2.1 -all")),
            // b.test reached twice: each evaluation counts.
            (
                "dup.test",
                dnsq::TXT,
                txt("v=spf1 include:b.test include:c2.test -all"),
            ),
            ("c2.test", dnsq::TXT, txt("v=spf1 include:b.test -all")),
            (
                "loop1.test",
                dnsq::TXT,
                txt("v=spf1 include:loop2.test -all"),
            ),
            (
                "loop2.test",
                dnsq::TXT,
                txt("v=spf1 include:loop1.test -all"),
            ),
            (
                "void.test",
                dnsq::TXT,
                txt("v=spf1 a:gone1.test a:gone2.test mx:gone3.test mx -all"),
            ),
            ("void.test", dnsq::MX, mx_rd.clone()),
            ("redir.test", dnsq::TXT, txt("v=spf1 redirect=b.test")),
            (
                "_dmarc.example.test",
                dnsq::TXT,
                txt("v=DMARC1; p=none; pct=50; rua=mailto:r@vendor.test"),
            ),
            (
                "selector1._domainkey.example.test",
                dnsq::CNAME,
                wire("selector1-example.onmicrosoft.test"),
            ),
            (
                "old._domainkey.revoked.test",
                dnsq::TXT,
                txt("v=DKIM1; k=rsa; p="),
            ),
            ("_mta-sts.example.test", dnsq::TXT, txt("v=STSv1; id=1")),
            ("_mta-sts.sts.test", dnsq::TXT, txt("v=STSv1; id=1")),
            (
                "default._bimi.bimi.test",
                dnsq::TXT,
                txt("v=BIMI1; l=https://bimi.test/logo.svg?v=2; a=https://bimi.test/vmc.pem"),
            ),
            ("_dmarc.bimi.test", dnsq::TXT, txt("v=DMARC1; p=reject")),
            (
                "selector1._domainkey.cn.test",
                dnsq::CNAME,
                wire("selector1-cn.onmicrosoft.test"),
            ),
            ("mta-sts.sts.test", dnsq::A, vec![127, 0, 0, 1]),
            (
                "default._domainkey.own.test",
                dnsq::CNAME,
                wire("k2024._domainkey.own.test"),
            ),
            (
                "k2024._domainkey.own.test",
                dnsq::TXT,
                txt(&format!("v=DKIM1; p={key}")),
            ),
            (
                "k1._domainkey.example.test",
                dnsq::CNAME,
                wire("dkim.vendor.test"),
            ),
            (
                "dkim.vendor.test",
                dnsq::TXT,
                txt(&format!("k=rsa; p={key}")),
            ),
            (
                "google._domainkey.example.test",
                dnsq::TXT,
                txt(&format!("v=DKIM1; k=rsa; p={key}")),
            ),
        ]);
        let d = "example.test";
        let (m, info) = mx(&c, d);
        assert!(has(&m, Status::Pass, "1 MX host"), "{m:?}");
        assert_eq!(info.ips, vec![IpAddr::from([127, 0, 0, 9])]);

        let r = spf(&c, "redir.test");
        assert!(
            has(&r, Status::Pass, "-all (hard fail) ➡ redirect=b.test"),
            "{r:?}"
        );
        assert!(has(&r, Status::Pass, "1/10 DNS lookups"), "{r:?}");

        let s = spf(&c, "dup.test");
        assert!(has(&s, Status::Pass, "3/10 DNS lookups"), "{s:?}");
        let s = spf(&c, "loop1.test");
        assert!(has(&s, Status::Fail, "loops back to loop1.test"), "{s:?}");
        let s = spf(&c, "void.test");
        assert!(
            has(
                &s,
                Status::Warn,
                "a:gone1.test in void.test matches nothing"
            ),
            "{s:?}"
        );
        assert!(has(&s, Status::Fail, "3 void lookups"), "{s:?}");
        // The bare "mx" uses void.test's own MX, which exists.
        assert!(!has(&s, Status::Warn, "mx in void.test"), "{s:?}");

        let s = spf(&c, d);
        assert!(has(&s, Status::Fail, "missing.test"), "{s:?}");
        assert!(has(&s, Status::Pass, "5/10 DNS lookups"), "{s:?}");
        assert!(has(&s, Status::Pass, "~all"), "{s:?}");

        let dm = dmarc(&c, d);
        assert!(has(&dm, Status::Warn, "policy none"), "{dm:?}");
        assert!(has(&dm, Status::Warn, "pct=50"), "{dm:?}");
        assert!(
            has(&dm, Status::Warn, "vendor.test aren't authorized"),
            "{dm:?}"
        );

        let rv = dkim(&c, "revoked.test", &["old".into()]);
        assert!(has(&rv, Status::Warn, "no active DKIM key"), "{rv:?}");

        // A failing selector is reported; the ones that answered still count.
        let dk = dkim(&c, d, &["fail".into()]);
        assert!(
            has(&dk, Status::Warn, "1 selector lookup(s) failed"),
            "{dk:?}"
        );
        assert!(has(&dk, Status::Warn, "google: 1024-bit"), "{dk:?}");

        let dk = dkim(&c, d, &[]);
        assert!(has(&dk, Status::Warn, "selector1: CNAME"), "{dk:?}");
        // A keyless CNAME (M365 before DKIM is enabled) is not a revoked key.
        let cn = dkim(&c, "cn.test", &[]);
        assert!(has(&cn, Status::Warn, "selector1: CNAME"), "{cn:?}");
        assert!(!has(&cn, Status::Warn, "only revoked"), "{cn:?}");
        assert!(has(&dk, Status::Warn, "google: 1024-bit"), "{dk:?}");
        assert!(
            has(
                &dk,
                Status::Warn,
                "k1: 1024-bit key; published by dkim.vendor.test"
            ),
            "{dk:?}"
        );

        let b = bimi(&c, "bimi.test");
        assert!(has(&b, Status::Pass, "logo is an HTTPS SVG"), "{b:?}");
        assert_eq!(b.status(), Status::Pass, "{b:?}");

        // No PTR in the zone: a self-hosted MX warns, a hosted filter doesn't.
        let own = mx_ptr(&c, &info);
        assert!(has(&own, Status::Warn, "has no PTR record"), "{own:?}");
        let hosted = MxInfo {
            hosts: vec!["example-test.mail.protection.outlook.com".into()],
            ips: info.ips.clone(),
        };
        assert_eq!(mx_ptr(&c, &hosted).status(), Status::Info);

        let ad = autodiscover(&c, d, &info);
        assert_eq!(ad.status(), Status::Info, "{ad:?}");
        assert_eq!(caa(&c, d).status(), Status::Info);
        let sts = mta_sts(&c, d, &info);
        assert!(
            has(&sts, Status::Fail, "mta-sts.example.test doesn't resolve"),
            "{sts:?}"
        );
        // Host resolves (via the chosen resolver) but the fetch fails.
        let sts = mta_sts(&c, "sts.test", &info);
        assert!(has(&sts, Status::Fail, "policy not reachable"), "{sts:?}");

        let own = dkim(&c, "own.test", &[]);
        assert!(
            has(
                &own,
                Status::Warn,
                "default: 1024-bit key; rotate to a 2048-bit key"
            ),
            "{own:?}"
        );
    }
}
