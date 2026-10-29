//! Mail checks: MX, SPF, DMARC, DKIM, MTA-STS, TLS-RPT, BIMI, MX reverse
//! DNS and Autodiscover.

use super::dnsq::{self, Client, Data, Msg};
use super::{Check, MxInfo, SELECTORS, addrs, http_get, parent, tag, tags, txt_with};
use std::collections::{HashMap, HashSet};
use std::io;
use std::net::IpAddr;

pub(super) fn mx(c: &Client, d: &str) -> (Check, MxInfo) {
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
            if let Some(t) = cname {
                k.warn(format!(
                    "{host} is a CNAME (to {t}); MX targets must be A/AAAA names (RFC 2181), some senders refuse it"
                ));
                info.aliases.push((host.clone(), t));
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

pub(super) fn spf(c: &Client, d: &str) -> Check {
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

pub(super) fn dmarc(c: &Client, d: &str) -> Check {
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

pub(super) fn base64(s: &str) -> Option<Vec<u8>> {
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

/// Probed alongside the real selectors to detect a `*._domainkey` wildcard.
const WILDCARD_PROBE: &str = "is-alive-wildcard-probe";

pub(super) fn dkim(c: &Client, d: &str, extra: &[String]) -> Check {
    Check::run(|k| {
        let mut sels: Vec<String> = extra.iter().map(|s| s.to_ascii_lowercase()).collect();
        sels.extend(SELECTORS.iter().map(|s| s.to_string()));
        let mut seen = HashSet::new();
        sels.retain(|s| !s.is_empty() && seen.insert(s.clone()));
        // A selector nobody uses: if it answers, the zone has a *._domainkey
        // wildcard and every probe would just echo it.
        let probe = WILDCARD_PROBE.to_string();
        let results: Vec<(String, io::Result<Msg>)> = std::thread::scope(|s| {
            let hs: Vec<_> = std::iter::once(&probe)
                .chain(&sels)
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
        let mut wildcard: Option<String> = None;
        // Keyless CNAMEs, judged once we know whether another selector is live.
        let mut keyless = Vec::new();
        for (sel, r) in results {
            let is_probe = sel == WILDCARD_PROBE;
            let m = match r {
                Ok(m) => m,
                Err(_) if is_probe => continue,
                Err(e) => {
                    failed.push((sel, e.to_string()));
                    continue;
                }
            };
            let cname = m.names(dnsq::CNAME);
            let keys: Vec<String> = m.txt().into_iter().filter(|t| t.contains("p=")).collect();
            if is_probe {
                wildcard = keys.into_iter().next();
                let Some(w) = &wildcard else { continue };
                found += 1;
                let empty = tag(&tags(w), "p").is_none_or(str::is_empty);
                k.rec(format!("*._domainkey  wildcard: {w}"));
                if empty {
                    k.info("*._domainkey wildcard with an empty key: every selector is revoked, so this domain signs no mail");
                } else {
                    k.info("*._domainkey wildcard: every selector returns the same key");
                }
                continue;
            }
            // Selectors that only echo the wildcard add nothing.
            if cname.is_empty() && keys.first().is_some_and(|k| Some(k) == wildcard.as_ref()) {
                continue;
            }
            if keys.is_empty() {
                if let Some(target) = cname.last() {
                    found += 1;
                    k.rec(format!("{sel}  CNAME ➡ {target}  (no key)"));
                    keyless.push((sel, target.clone()));
                }
                continue;
            }
            found += 1;
            let key = &keys[0];
            let target = cname.last();
            let via = target.map(|t| format!(" (via {t})")).unwrap_or_default();
            // A CNAME out of the domain means a provider publishes the key;
            // Microsoft 365 tenants rotate theirs themselves.
            let fix = match target.filter(|t| !t.ends_with(&format!(".{d}"))) {
                Some(t) if t.ends_with(".onmicrosoft.com") => {
                    "rotate it in Microsoft Defender (Email authentication settings → DKIM → Rotate DKIM keys) to get 2048-bit".to_string()
                }
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
        for (sel, target) in keyless {
            // Microsoft 365 publishes two selectors and keys only the active
            // one between rotations.
            if active > 0 && target.ends_with(".onmicrosoft.com") {
                k.info(format!(
                    "{sel}: standby Microsoft 365 selector (no key until the next rotation)"
                ));
            } else {
                k.warn(format!(
                    "{sel}: CNAME to {target} but no key there; DKIM not enabled at the provider yet?"
                ));
            }
        }
        if revoked > 0 && active == 0 {
            k.warn("no active DKIM key found, only revoked ones; the current selector isn't in the probe list (check a message's s= tag)");
        }
        if failed.len() == sels.len() {
            // Usually one cause (SERVFAIL, timeout): say it once.
            let (sel, e) = &failed[0];
            return Err(io::Error::other(format!(
                "{e} (all {} selectors failed, starting with {sel})",
                failed.len()
            )));
        }
        if !failed.is_empty() {
            let list: Vec<String> = failed.iter().map(|(s, e)| format!("{s} ({e})")).collect();
            k.warn(format!(
                "{} selector lookup(s) failed, results may be incomplete: {}",
                failed.len(),
                list.join(", ")
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

/// MX host `h` matches an MTA-STS `mx:` pattern (exact, or `*.` one label).
fn sts_match(pattern: &str, h: &str) -> bool {
    let p = pattern.trim_end_matches('.').to_ascii_lowercase();
    match p.strip_prefix("*.") {
        Some(rest) => h.split_once('.').is_some_and(|(_, tail)| tail == rest),
        None => p == h,
    }
}

pub(super) fn mta_sts(c: &Client, d: &str, mx: &MxInfo) -> Check {
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

pub(super) fn tls_rpt(c: &Client, d: &str) -> Check {
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

pub(super) fn bimi(c: &Client, d: &str) -> Check {
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
                        .unwrap_or(l)
                        .to_ascii_lowercase()
                        .ends_with(".svg") =>
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

/// PTR names of an address and whether one resolves back to it.
type PtrResult = (Vec<String>, bool);

pub(super) fn mx_ptr(c: &Client, mx: &MxInfo) -> Check {
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
                Err(e) if hosted => k.info(format!("{ip}: {e}")),
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

pub(super) fn autodiscover(c: &Client, d: &str, mx: &MxInfo) -> Check {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
