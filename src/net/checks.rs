//! What's between this computer and the internet that a ping or a port
//! can't show: whether a captive portal answers in the internet's place,
//! whether HTTPS is inspected (a firewall or security software
//! decrypting it and re-signing every site's certificate), and whether
//! this computer's clock is right (Active Directory sign-in fails past five
//! minutes off; TLS and sign-in codes before that).

use super::{tls, web};
use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Fail,
    /// It couldn't be checked from here.
    Unknown,
}

impl Status {
    pub fn tag(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
            Status::Unknown => "unknown",
        }
    }
}

/// One check's verdict.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub check: &'static str,
    pub status: Status,
    pub detail: String,
}

// ------------------------------------------------------------- portal

/// Cloudflare's connectivity check: plain HTTP, answered with an empty
/// 204 No Content. A portal (or a filter's block page) answers anything
/// else: its sign-in page, or a redirect to it.
pub const PORTAL_HOST: &str = "cp.cloudflare.com";
pub const PORTAL_PATH: &str = "/generate_204";

/// What the connectivity check got.
#[derive(Clone, Debug, PartialEq)]
pub enum Portal {
    /// 204, as Cloudflare sends it.
    Open,
    /// Something else answered: its status, where it redirected to (host
    /// only) and its page's title.
    Intercepted {
        status: u16,
        to: Option<String>,
        title: Option<String>,
    },
    /// No answer (no internet, HTTP blocked): nothing to judge from.
    Unreached(String),
}

impl Portal {
    pub fn intercepted(&self) -> bool {
        matches!(self, Portal::Intercepted { .. })
    }
}

/// The host a `Location` points to, else the path it gives.
fn location_host(loc: &str) -> String {
    let rest = loc.split_once("://").map_or(loc, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    if host.is_empty() { loc } else { host }.to_string()
}

/// Read a reply to the connectivity check.
pub fn portal_from(page: &web::Page) -> Portal {
    if page.status == 204 {
        return Portal::Open;
    }
    Portal::Intercepted {
        status: page.status,
        to: page.location.as_deref().map(location_host),
        title: page.title.clone(),
    }
}

/// One plain-HTTP GET of the connectivity check.
pub fn portal(timeout: Duration) -> Portal {
    let addr = match (PORTAL_HOST, 80).to_socket_addrs() {
        Ok(mut a) => match a.find(SocketAddr::is_ipv4) {
            Some(a) => a,
            None => return Portal::Unreached("no IPv4 address".into()),
        },
        Err(e) => return Portal::Unreached(e.to_string()),
    };
    match web::get(addr, PORTAL_HOST, false, PORTAL_PATH, timeout) {
        Ok(page) => portal_from(&page),
        Err(e) => Portal::Unreached(e.to_string()),
    }
}

/// The captive portal check.
pub fn judge_portal(p: &Portal) -> Item {
    let check = "Captive portal";
    match p {
        Portal::Open => Item {
            check,
            status: Status::Ok,
            detail: format!(
                "None: http://{PORTAL_HOST}{PORTAL_PATH} came back 204 No Content, as it should."
            ),
        },
        Portal::Intercepted { status, to, title } => {
            let mut what = format!("HTTP {status}");
            if let Some(t) = to {
                what += &format!(", redirected to {t}");
            }
            if let Some(t) = title {
                what += &format!(", page \"{t}\"");
            }
            Item {
                check,
                status: Status::Fail,
                detail: format!(
                    "Something answered in place of {PORTAL_HOST} ({what}): a captive portal \
                     (a guest network's sign-in or terms page) or a web filter's block page. \
                     Open a browser and sign in or accept the terms, then run Full scan again: \
                     until then the internet-side checks are judging the portal, not the \
                     internet."
                ),
            }
        }
        Portal::Unreached(e) => Item {
            check,
            status: Status::Unknown,
            detail: format!(
                "{PORTAL_HOST} didn't answer over HTTP ({e}), so it couldn't be checked."
            ),
        },
    }
}

// ------------------------------------------------------------- HTTPS

/// Sites with certificates from three different public CAs: one issuer on
/// two of them is a re-signing middlebox, not a coincidence.
pub const SITES: [&str; 3] = ["www.cloudflare.com", "github.com", "www.microsoft.com"];

/// Products that inspect HTTPS, as their certificates name them.
const INSPECTORS: &[&str] = &[
    "sonicwall",
    "fortinet",
    "fortigate",
    "sophos",
    "zscaler",
    "palo alto",
    "netskope",
    "forcepoint",
    "barracuda",
    "watchguard",
    "untangle",
    "check point",
    "checkpoint",
    "umbrella",
    "blue coat",
    "bluecoat",
    "smoothwall",
    "lightspeed",
    "securly",
    "goguardian",
    "iboss",
    "cyberoam",
    "kaspersky",
    "eset",
    "avast",
    "bitdefender",
    "norton",
    "mcafee",
    "trend micro",
    "webroot",
    "dpi-ssl",
    "dpi ssl",
    "ssl inspection",
    "fiddler",
    "mitmproxy",
    "burp",
    "squid",
];

/// What one site presented: its certificate's issuer, and the issuer at
/// the top of the chain sent; whether this computer trusts it.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub site: String,
    pub issuer: String,
    pub top: String,
    pub trusted: bool,
}

/// HTTPS inspection, judged from what each site presented (Err: not
/// reached). Behind a captive portal the sites' certificates are the
/// portal's own, so a portal is reported instead of inspection.
pub fn judge_https(seen: &[Result<Seen, String>], portal: &Portal) -> Item {
    let check = "HTTPS inspection";
    let r = judge_issuers(seen);
    if portal.intercepted() && r.status != Status::Ok {
        let fail = seen.iter().filter(|s| s.is_err()).count();
        let s = seen.iter().filter_map(|s| s.as_ref().ok()).next();
        let what = match s {
            Some(s) => format!("{}'s certificate came from \"{}\"", s.site, s.issuer),
            None => format!("{fail} of the test sites didn't answer"),
        };
        return Item {
            check,
            status: Status::Unknown,
            detail: format!(
                "Not judged: a captive portal is answering in the internet's place ({what}, \
                 most likely the portal's own). That isn't HTTPS inspection, and there's no \
                 certificate to install: sign in to the portal, then run Full scan again."
            ),
        };
    }
    r
}

fn judge_issuers(seen: &[Result<Seen, String>]) -> Item {
    let check = "HTTPS inspection";
    let got: Vec<&Seen> = seen.iter().filter_map(|s| s.as_ref().ok()).collect();
    if got.is_empty() {
        return Item {
            check,
            status: Status::Unknown,
            detail: "None of the test sites answered over HTTPS (no internet, or HTTPS blocked)."
                .into(),
        };
    }
    let named = |s: &Seen| {
        let both = format!("{} {}", s.issuer, s.top).to_ascii_lowercase();
        INSPECTORS.iter().any(|w| both.contains(w))
    };
    let shared = got
        .iter()
        .copied()
        .find(|a| got.iter().filter(|b| b.issuer == a.issuer).count() > 1);
    let by = got.iter().copied().find(|s| named(s)).or(shared);
    if let Some(s) = by {
        let how = if named(s) {
            "not by a public CA".to_string()
        } else {
            let n = got.iter().filter(|b| b.issuer == s.issuer).count();
            format!("the same issuer signed {n} unrelated sites")
        };
        let mut detail = format!(
            "HTTPS is inspected here: {}'s certificate came from \"{}\" ({how}). A firewall or \
             security software decrypts HTTPS and re-signs it; apps that pin their certificates \
             (some VPN clients, updaters, banking) can fail.",
            s.site, s.issuer
        );
        let status = if s.trusted {
            Status::Warn
        } else {
            detail += " This computer doesn't trust that issuer, so browsers show certificate \
                       errors: install the inspection certificate, or exclude this computer.";
            Status::Fail
        };
        return Item {
            check,
            status,
            detail,
        };
    }
    if let Some(s) = got.iter().find(|s| !s.trusted) {
        return Item {
            check,
            status: Status::Warn,
            detail: format!(
                "Not inspected, but {}'s certificate (from \"{}\") isn't trusted here: check \
                 this computer's clock and its certificate store.",
                s.site, s.issuer
            ),
        };
    }
    let issuers: Vec<String> = got
        .iter()
        .map(|s| format!("{} by {}", s.site, s.issuer))
        .collect();
    Item {
        check,
        status: Status::Ok,
        detail: format!(
            "Not inspected: each site's own certificate came through ({}).",
            issuers.join("; ")
        ),
    }
}

/// What `site` presents over HTTPS from here.
pub fn present(site: &str, timeout: Duration) -> Result<Seen, String> {
    let addr = (site, 443)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .find(SocketAddr::is_ipv4)
        .ok_or("no IPv4 address")?;
    let info = tls::probe(site, addr, timeout).map_err(|e| e.to_string())?;
    let leaf = info.chain.first().ok_or("no certificate")?;
    let top = info.chain.last().unwrap_or(leaf);
    Ok(Seen {
        site: site.to_string(),
        issuer: leaf.issuer.clone(),
        top: top.issuer.clone(),
        trusted: info.problem.is_none(),
    })
}

// ------------------------------------------------------------- clock

/// Servers asked for the time, in turn.
const NTP: [&str; 3] = ["time.cloudflare.com", "time.windows.com", "pool.ntp.org"];
/// Seconds from 1900 (NTP's epoch) to 1970.
const NTP_EPOCH: f64 = 2_208_988_800.0;

/// How far this computer's clock is off (seconds, + fast), and how it
/// was measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Offset {
    pub secs: f64,
    pub from: String,
}

/// The clock, judged from its offset (None: nothing could be asked) and
/// where this computer says it syncs from.
pub fn judge_clock(offset: Option<&Offset>, source: Option<&str>) -> Item {
    let check = "Clock";
    let synced = source
        .map(|s| format!(" It syncs to {s}."))
        .unwrap_or_default();
    let Some(o) = offset else {
        return Item {
            check,
            status: Status::Unknown,
            detail: format!(
                "No time server or HTTPS site answered, so the clock couldn't be checked.{synced}"
            ),
        };
    };
    let off = o.secs.abs();
    let way = if o.secs > 0.0 { "fast" } else { "slow" };
    let size = if off < 1.0 {
        "within a second".to_string()
    } else if off < 120.0 {
        format!("{off:.0} s {way}")
    } else {
        format!("{:.0} min {way}", off / 60.0)
    };
    let (status, then) = if off <= 30.0 {
        (Status::Ok, "")
    } else if off <= 300.0 {
        (
            Status::Warn,
            " It's drifting: sign-in codes and some TLS checks start failing; past 5 minutes \
             Active Directory sign-in (Kerberos) fails too.",
        )
    } else {
        (
            Status::Fail,
            " Past 5 minutes Active Directory sign-in (Kerberos) fails, as do sign-in codes \
             and TLS checks: fix the time source.",
        )
    };
    Item {
        check,
        status,
        detail: format!(
            "This computer's clock is {size} (measured {}).{then}{synced}",
            o.from
        ),
    }
}

/// Seconds since 1970 now.
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// An NTP timestamp (bytes 0..8: seconds since 1900, then the fraction)
/// as seconds since 1970. The seconds wrap on 7 Feb 2036: a value from
/// the first half of the range is read as the next era (2036 on), since
/// nothing this computer asks about is from before 1968.
fn ntp_time(b: &[u8]) -> f64 {
    let mut s = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64;
    if s < 2_147_483_648.0 {
        s += 4_294_967_296.0;
    }
    let f = u32::from_be_bytes([b[4], b[5], b[6], b[7]]) as f64 / 4_294_967_296.0;
    s + f - NTP_EPOCH
}

/// The offset from a server's reply to a request carrying `origin` in its
/// transmit timestamp, sent at `t1` and read at `t4` (seconds since 1970),
/// positive when this computer is fast. A reply that doesn't echo `origin`
/// isn't an answer to this request; a server that says it isn't
/// synchronized (leap indicator 3, stratum 16 or more) has no time worth
/// comparing.
pub fn sntp_offset(reply: &[u8], origin: &[u8; 8], t1: f64, t4: f64) -> Option<f64> {
    if reply.len() < 48 || reply[0] & 0x07 != 4 || reply[1] == 0 {
        // Not a server's answer, or a "kiss of death" (stratum 0).
        return None;
    }
    if reply[0] >> 6 == 3 || reply[1] >= 16 || reply[24..32] != origin[..] {
        return None;
    }
    let t2 = ntp_time(&reply[32..40]);
    let t3 = ntp_time(&reply[40..48]);
    // Theirs minus ours, so negative when ours is ahead: flip it.
    Some(-(((t2 - t1) + (t3 - t4)) / 2.0))
}

/// Ask one NTP server (SNTP, one packet each way).
fn sntp(server: &str, timeout: Duration) -> io::Result<f64> {
    let addr = (server, 123)
        .to_socket_addrs()?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| io::Error::other("no IPv4 address"))?;
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    // Connected: datagrams from anywhere else are dropped by the system.
    sock.connect(addr)?;
    sock.set_read_timeout(Some(timeout))?;
    let mut req = [0u8; 48];
    req[0] = 0x23; // version 4, client
    // A transmit timestamp no one else can guess, echoed back by the server
    // as its originate timestamp (RFC 4330 §5).
    let origin = {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(now().to_bits());
        h.finish().to_be_bytes()
    };
    req[40..48].copy_from_slice(&origin);
    let t1 = now();
    sock.send(&req)?;
    let mut buf = [0u8; 68];
    let n = sock.recv(&mut buf)?;
    let t4 = now();
    sntp_offset(&buf[..n], &origin, t1, t4)
        .ok_or_else(|| io::Error::other("not an answer from a synchronized NTP server"))
}

/// How far off this computer's clock is: from an NTP server, else (UDP 123
/// blocked) from an HTTPS site's Date header, to the second.
pub fn offset(timeout: Duration) -> Option<Offset> {
    for s in NTP {
        if let Ok(secs) = sntp(s, timeout) {
            return Some(Offset {
                secs,
                from: format!("against {s}"),
            });
        }
    }
    for site in SITES {
        let Ok(addr) = (site, 443)
            .to_socket_addrs()
            .map(|mut a| a.find(SocketAddr::is_ipv4))
        else {
            continue;
        };
        let Some(addr) = addr else { continue };
        let t0 = now();
        let Ok(page) = web::get(addr, site, true, "/", timeout) else {
            continue;
        };
        let t1 = now();
        let Some(date) = page
            .date
            .as_deref()
            .and_then(|d| chrono::DateTime::parse_from_rfc2822(d).ok())
        else {
            continue;
        };
        // The header is to the second: compare with the middle of the wait.
        let secs = (t0 + t1) / 2.0 - date.timestamp() as f64;
        return Some(Offset {
            secs,
            from: format!("from {site}'s HTTPS Date header, to the second; NTP didn't answer"),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(site: &str, issuer: &str, trusted: bool) -> Result<Seen, String> {
        Ok(Seen {
            site: site.into(),
            issuer: issuer.into(),
            top: issuer.into(),
            trusted,
        })
    }

    #[test]
    fn https_is_inspected_when_a_middlebox_signs_the_sites() {
        // Each site's own CA: not inspected.
        let own = [
            seen("www.cloudflare.com", "WE1", true),
            seen(
                "github.com",
                "Sectigo ECC Domain Validation Secure Server CA",
                true,
            ),
            seen(
                "www.microsoft.com",
                "Microsoft Azure RSA TLS Issuing CA 07",
                true,
            ),
        ];
        let r = judge_https(&own, &Portal::Open);
        assert_eq!(r.status, Status::Ok, "{}", r.detail);
        assert!(r.detail.contains("github.com by Sectigo"));
        // A SonicWall re-signing, trusted (its CA installed): a warning.
        let dpi = [
            seen("www.cloudflare.com", "SonicWall Firewall DPI-SSL", true),
            Err("timed out".into()),
            seen(
                "www.microsoft.com",
                "Microsoft Azure RSA TLS Issuing CA 07",
                true,
            ),
        ];
        let r = judge_https(&dpi, &Portal::Open);
        assert_eq!(r.status, Status::Warn);
        assert!(r.detail.contains("SonicWall Firewall DPI-SSL") && r.detail.contains("public CA"));
        // An unknown product, but one issuer on two unrelated sites, and
        // not trusted here: browsers will complain.
        let odd = [
            seen("www.cloudflare.com", "Acme Gateway CA", false),
            seen("github.com", "Acme Gateway CA", false),
            Err("refused".into()),
        ];
        let r = judge_https(&odd, &Portal::Open);
        assert_eq!(r.status, Status::Fail);
        assert!(
            r.detail.contains("same issuer signed 2 unrelated sites"),
            "{}",
            r.detail
        );
        assert!(r.detail.contains("doesn't trust"));
        // Untrusted but each its own CA: the clock or the store, not inspection.
        let r = judge_https(&[seen("github.com", "Sectigo", false)], &Portal::Open);
        assert_eq!(r.status, Status::Warn);
        assert!(r.detail.contains("clock"));
        // Nothing reached.
        assert_eq!(
            judge_https(&[Err("x".into())], &Portal::Unreached("x".into())).status,
            Status::Unknown
        );
    }

    /// Behind a captive portal every test site shows the portal's own
    /// certificate: one issuer on three unrelated sites, untrusted. That's
    /// the portal, not inspection, and there's no certificate to install.
    #[test]
    fn a_captive_portal_isnt_https_inspection() {
        let portal_seen = [
            seen("www.cloudflare.com", "guest.example.net", false),
            seen("github.com", "guest.example.net", false),
            seen("www.microsoft.com", "guest.example.net", false),
        ];
        // Without the portal check this reads as inspection.
        let r = judge_https(&portal_seen, &Portal::Unreached("timed out".into()));
        assert_eq!(r.status, Status::Fail);
        let page = web::parse(
            b"HTTP/1.1 302 Found\r\nLocation: https://guest.example.net/login?u=x\r\n\r\n",
        )
        .unwrap();
        let portal = portal_from(&page);
        assert_eq!(
            portal,
            Portal::Intercepted {
                status: 302,
                to: Some("guest.example.net".into()),
                title: None
            }
        );
        let p = judge_portal(&portal);
        assert_eq!(p.status, Status::Fail);
        assert!(p.detail.contains("captive portal") && p.detail.contains("guest.example.net"));
        let r = judge_https(&portal_seen, &portal);
        assert_ne!(r.status, Status::Fail);
        assert!(r.detail.contains("captive portal"), "{}", r.detail);
        assert!(!r.detail.contains("HTTPS is inspected"), "{}", r.detail);
        assert!(!r.detail.contains("install the inspection certificate"));
        // The real thing: 204 No Content.
        let ok = web::parse(b"HTTP/1.1 204 No Content\r\nServer: cloudflare\r\n\r\n").unwrap();
        assert_eq!(portal_from(&ok), Portal::Open);
        assert_eq!(judge_portal(&Portal::Open).status, Status::Ok);
        // A portal that lets HTTPS through untouched: the sites are judged.
        let own = [seen(
            "github.com",
            "Sectigo ECC Domain Validation Secure Server CA",
            true,
        )];
        assert_eq!(judge_https(&own, &portal).status, Status::Ok);
        // A 200 sign-in page names itself.
        let login =
            web::parse(b"HTTP/1.1 200 OK\r\n\r\n<html><title>Guest Wi-Fi terms</title></html>")
                .unwrap();
        assert!(
            judge_portal(&portal_from(&login))
                .detail
                .contains("Guest Wi-Fi terms")
        );
        assert_eq!(location_host("/login"), "/login");
    }

    #[test]
    fn the_clock_is_judged_by_how_far_off_it_is() {
        let at = |secs: f64| Offset {
            secs,
            from: "against time.example".into(),
        };
        let r = judge_clock(Some(&at(0.4)), Some("time.windows.com"));
        assert_eq!(r.status, Status::Ok);
        assert!(
            r.detail.contains("within a second") && r.detail.contains("syncs to time.windows.com")
        );
        assert_eq!(judge_clock(Some(&at(-12.0)), None).status, Status::Ok);
        let r = judge_clock(Some(&at(95.0)), None);
        assert_eq!(r.status, Status::Warn);
        assert!(r.detail.contains("95 s fast"));
        let r = judge_clock(Some(&at(-420.0)), None);
        assert_eq!(r.status, Status::Fail);
        assert!(r.detail.contains("7 min slow") && r.detail.contains("Kerberos"));
        assert_eq!(judge_clock(None, None).status, Status::Unknown);
    }

    /// Against the real internet: what this network does to HTTPS, and how
    /// far off this computer's clock is. `cargo test -- --ignored live`
    #[test]
    #[ignore = "needs the internet"]
    fn live_checks() {
        let seen: Vec<_> = SITES
            .iter()
            .map(|s| present(s, Duration::from_secs(5)))
            .collect();
        println!("{seen:#?}");
        let portal = portal(Duration::from_secs(4));
        println!("{:#?}", judge_portal(&portal));
        println!("{:#?}", judge_https(&seen, &portal));
        let off = offset(Duration::from_secs(4));
        println!("{:#?}", judge_clock(off.as_ref(), None));
    }

    /// A server 2 s ahead of us, answering at once.
    #[test]
    fn hostile_ntp_replies_never_panic() {
        let mut good = vec![0x24, 1, 0, 0xec];
        good.extend(std::iter::repeat_n(0x5a, 44));
        crate::hostile::each(&good, |r| {
            let _ = sntp_offset(r, &[0; 8], 1_800_000_000.0, 1_800_000_000.1);
        });
    }

    #[test]
    fn an_ntp_reply_gives_the_offset() {
        let ts = |t: f64| {
            let n = t + NTP_EPOCH;
            let s = n.trunc() as u32;
            let f = ((n.fract()) * 4_294_967_296.0) as u32;
            let mut b = s.to_be_bytes().to_vec();
            b.extend(f.to_be_bytes());
            b
        };
        let t1 = 1_800_000_000.0;
        let t4 = t1 + 0.1;
        let origin = [7, 1, 2, 3, 4, 5, 6, 8];
        let mut reply = vec![0u8; 48];
        reply[0] = 0x24; // version 4, server
        reply[1] = 2; // stratum
        reply[24..32].copy_from_slice(&origin);
        reply[32..40].copy_from_slice(&ts(t1 + 2.05));
        reply[40..48].copy_from_slice(&ts(t1 + 2.05));
        let o = sntp_offset(&reply, &origin, t1, t4).unwrap();
        assert!((o + 2.0).abs() < 0.01, "{o}: this computer is 2 s slow");
        // Not this request's answer: someone else's, or a guess.
        assert_eq!(sntp_offset(&reply, &[0; 8], t1, t4), None);
        // Unsynchronized: leap indicator 3, or stratum 16.
        let mut unsynced = reply.clone();
        unsynced[0] |= 0xC0;
        assert_eq!(sntp_offset(&unsynced, &origin, t1, t4), None);
        unsynced = reply.clone();
        unsynced[1] = 16;
        assert_eq!(sntp_offset(&unsynced, &origin, t1, t4), None);
        reply[1] = 0;
        assert_eq!(sntp_offset(&reply, &origin, t1, t4), None, "kiss of death");
        assert_eq!(sntp_offset(&reply[..20], &origin, t1, t4), None);
    }

    /// Past 7 Feb 2036 NTP's seconds wrap to 0: read as the next era, not
    /// 136 years back.
    #[test]
    fn ntp_time_after_2036() {
        // 2040-01-01 00:00:00 UTC, as NTP sends it (seconds wrapped).
        let unix = 2_208_988_800.0_f64;
        let ntp_secs = ((unix + NTP_EPOCH) as u64 % (1u64 << 32)) as u32;
        let mut b = ntp_secs.to_be_bytes().to_vec();
        b.extend([0; 4]);
        assert_eq!(ntp_time(&b), unix);
        // And today's stays today's.
        let now = 1_800_000_000.0_f64;
        let mut b = ((now + NTP_EPOCH) as u32).to_be_bytes().to_vec();
        b.extend([0; 4]);
        assert_eq!(ntp_time(&b), now);
    }
}
