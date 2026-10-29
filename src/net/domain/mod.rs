//! Domain and mail DNS health checks. Each check validates, not just detects:
//! most real problems are records that exist but are broken.

mod blacklist;
mod mail;
mod smtp;
mod zone;

use super::dnsq::{self, Client};
use blacklist::blacklists;
use mail::{autodiscover, bimi, dkim, dmarc, mta_sts, mx, mx_ptr, spf, tls_rpt};
use smtp::smtp;
use std::io;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;
use zone::{caa, dnssec, expiry, nameservers};

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
pub const NAMES: [&str; 15] = [
    "MX",
    "SPF",
    "DMARC",
    "DKIM",
    "Nameservers",
    "SMTP (port 25)",
    "Blacklists",
    "MTA-STS",
    "TLS-RPT",
    "BIMI",
    "DNSSEC",
    "CAA",
    "MX reverse DNS",
    "Autodiscover",
    "Domain expiry",
];
pub const CORE: usize = 7;

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
    let jobs: [CheckFn; 14] = [
        |c, d, _, _| spf(c, d),
        |c, d, _, _| dmarc(c, d),
        |c, d, sels, _| dkim(c, d, sels),
        |c, d, _, _| nameservers(c, d),
        |c, _, _, mx| smtp(c, mx),
        |c, d, _, mx| blacklists(c, d, mx, super::hostinfo::wan()),
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
    /// (MX host, CNAME target) for MX hosts that are aliases.
    pub aliases: Vec<(String, String)>,
}

/// MX host suffixes of Microsoft 365 and Google Workspace / Gmail.
const M365_MX: &[&str] = &[".mail.protection.outlook.com"];
const GOOGLE_MX: &[&str] = &[".google.com", ".googlemail.com"];
/// Hosted inbound filters: Cloudflare Email Security, Mimecast, Proofpoint,
/// Barracuda.
const FILTER_MX: &[&str] = &[
    ".cf-emailsecurity.net",
    ".mimecast.com",
    ".pphosted.com",
    ".ppe-hosted.com",
    ".barracudanetworks.com",
    ".mx.cloudflare.net", // Cloudflare Email Routing
];

fn ends_any(h: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|s| h.ends_with(s))
}

impl MxInfo {
    /// The names an MX host goes by: itself and its CNAME target, if any.
    fn names<'a>(&'a self, h: &'a str) -> impl Iterator<Item = &'a str> {
        std::iter::once(h).chain(
            self.aliases
                .iter()
                .filter(move |(a, _)| a == h)
                .map(|(_, t)| t.as_str()),
        )
    }
    fn any(&self, suffixes: &[&str]) -> bool {
        self.hosts
            .iter()
            .any(|h| self.names(h).any(|n| ends_any(n, suffixes)))
    }
    fn m365(&self) -> bool {
        self.any(M365_MX)
    }
    /// Every MX is a hosted mail/filtering service, whose inbound relays
    /// don't send the domain's mail, so their reverse DNS doesn't matter here.
    fn hosted(&self) -> bool {
        !self.hosts.is_empty()
            && self.hosts.iter().all(|h| {
                self.names(h).any(|n| {
                    ends_any(n, M365_MX) || ends_any(n, GOOGLE_MX) || ends_any(n, FILTER_MX)
                })
            })
    }
    fn google(&self) -> bool {
        self.any(GOOGLE_MX)
    }
}

/// A and AAAA addresses of `host`, plus the final CNAME target if it is an alias.
fn addrs(c: &Client, host: &str) -> io::Result<(Vec<IpAddr>, Option<String>)> {
    let a = c.ask(host, dnsq::A)?;
    let cname = a.names(dnsq::CNAME).pop();
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
pub(crate) fn zone_of(c: &Client, name: &str) -> io::Result<String> {
    let m = c.ask(name, dnsq::SOA)?;
    // An alias answers with its target's SOA, but the name itself is
    // published in the zone above it (mail.yahoo.com lives in yahoo.com).
    if m.answers
        .iter()
        .any(|r| r.rtype == dnsq::CNAME && r.name == name)
        && let Some(p) = parent(name)
    {
        return zone_of(c, p);
    }
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

/// HTTPS GET (MTA-STS policy, RDAP) with the OS trust store and proxy.
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
                    // Wildcard: *.<parent> answers for any name under it.
                    let wc = format!("*.{}", name.split_once('.').map_or("", |p| p.1));
                    let hit: Vec<_> = zone.iter().filter(|r| r.0 == wc && r.1 == qtype).collect();
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
                // Names with a "fail" label answer SERVFAIL; "bogus.*" names act
                // like a broken DNSSEC zone: SERVFAIL unless CD is set.
                let bogus = labels[0] == "bogus" && q[3] & 0x10 == 0;
                let rcode = if labels.iter().any(|l| l == "fail") || bogus {
                    2
                } else {
                    0
                };
                if rcode != 0 {
                    ans.clear();
                }
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

    fn soa(mname: &str) -> Vec<u8> {
        let mut v = wire(mname);
        v.extend(wire("hostmaster.test"));
        v.extend([0, 0, 0, 1].repeat(5)); // serial, refresh, retry, expire, minimum
        v
    }

    fn mx_to(host: &str) -> Vec<u8> {
        let mut v = vec![0, 10];
        v.extend(wire(host));
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
            // DKIM wildcard revoking every selector (example.com style).
            ("*._domainkey.wild.test", dnsq::TXT, txt("v=DKIM1; p=")),
            // Microsoft 365: selector1 standby, selector2 active 1024-bit.
            (
                "selector1._domainkey.m365.test",
                dnsq::CNAME,
                wire("selector1-m365-test._domainkey.m365test.onmicrosoft.com"),
            ),
            (
                "selector2._domainkey.m365.test",
                dnsq::CNAME,
                wire("selector2-m365-test._domainkey.m365test.onmicrosoft.com"),
            ),
            (
                "selector2-m365-test._domainkey.m365test.onmicrosoft.com",
                dnsq::TXT,
                txt(&format!("v=DKIM1; k=rsa; p={key}")),
            ),
            // A CNAMEd name: its zone is the parent, not the target's zone.
            ("alias.zone.test", dnsq::CNAME, wire("host.other.test")),
            ("zone.test", dnsq::SOA, soa("ns.zone.test")),
            ("host.other.test", dnsq::SOA, soa("ns.other.test")),
            // MX that is a CNAME to a hosted filter.
            ("ali.test", dnsq::MX, mx_to("mx.ali.test")),
            (
                "mx.ali.test",
                dnsq::CNAME,
                wire("mxa.global.inbound.cf-emailsecurity.net"),
            ),
            (
                "mxa.global.inbound.cf-emailsecurity.net",
                dnsq::A,
                vec![127, 0, 0, 7],
            ),
            ("bogus.test", dnsq::SOA, soa("ns.bogus.test")),
            // example.test's MX IP (127.0.0.9) is on Spamhaus ZEN.
            ("9.0.0.127.zen.spamhaus.org", dnsq::A, vec![127, 0, 0, 2]),
            // A Spamhaus PBL entry for an SPF-authorized address.
            ("5.2.0.192.zen.spamhaus.org", dnsq::A, vec![127, 0, 0, 11]),
            ("pbl.test", dnsq::TXT, txt("v=spf1 ip4:192.0.2.5 -all")),
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
            aliases: vec![],
        };
        assert_eq!(mx_ptr(&c, &hosted).status(), Status::Info);

        let w = dkim(&c, "wild.test", &[]);
        assert_eq!(w.status(), Status::Info, "{w:?}");
        assert!(has(&w, Status::Info, "signs no mail"), "{w:?}");
        assert!(
            !w.findings.iter().any(|f| f.1.starts_with("selector1")),
            "{w:?}"
        );

        let ms = dkim(&c, "m365.test", &[]);
        assert!(
            has(&ms, Status::Info, "selector1: standby Microsoft 365"),
            "{ms:?}"
        );
        assert!(
            has(
                &ms,
                Status::Warn,
                "selector2: 1024-bit key; rotate it in Microsoft Defender"
            ),
            "{ms:?}"
        );

        // Every selector failing the same way is one line, not thirteen.
        let bad = dkim(&c, "x.fail.test", &[]);
        assert_eq!(bad.findings.len(), 1, "{bad:?}");
        assert!(
            has(
                &bad,
                Status::Fail,
                "(all 13 selectors failed, starting with"
            ),
            "{bad:?}"
        );

        assert_eq!(zone_of(&c, "alias.zone.test").unwrap(), "zone.test");

        let (am, ainfo) = mx(&c, "ali.test");
        assert!(
            has(
                &am,
                Status::Warn,
                "CNAME (to mxa.global.inbound.cf-emailsecurity.net)"
            ),
            "{am:?}"
        );
        assert_eq!(mx_ptr(&c, &ainfo).status(), Status::Info);

        let bl = blacklists(&c, d, &info, None);
        assert!(
            has(
                &bl,
                Status::Fail,
                "127.0.0.9 (MX) is listed on Spamhaus ZEN (code 127.0.0.2)"
            ),
            "{bl:?}"
        );
        let none = MxInfo::default();
        let pbl = blacklists(&c, "pbl.test", &none, None);
        assert!(
            has(&pbl, Status::Warn, "but SPF authorizes it to send"),
            "{pbl:?}"
        );
        let clean = blacklists(
            &c,
            "pbl.test",
            &none,
            Some(std::net::Ipv4Addr::new(198, 51, 100, 7)),
        );
        assert!(has(&clean, Status::Warn, "Spamhaus PBL"), "{clean:?}");
        assert!(
            clean
                .records
                .iter()
                .any(|r| r.starts_with("198.51.100.7 (this network's WAN)  clean")),
            "{clean:?}"
        );

        let ds = dnssec(&c, "bogus.test");
        assert!(has(&ds, Status::Fail, "bogus"), "{ds:?}");

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
