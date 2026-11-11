//! What's between this computer and the internet that a ping or a port
//! can't show: whether HTTPS is inspected (a firewall or security software
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
/// reached).
pub fn judge_https(seen: &[Result<Seen, String>]) -> Item {
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
/// as seconds since 1970.
fn ntp_time(b: &[u8]) -> f64 {
    let s = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64;
    let f = u32::from_be_bytes([b[4], b[5], b[6], b[7]]) as f64 / 4_294_967_296.0;
    s + f - NTP_EPOCH
}

/// The offset from a server's reply sent at `t1` and read at `t4`
/// (seconds since 1970): + when this computer is fast.
pub fn sntp_offset(reply: &[u8], t1: f64, t4: f64) -> Option<f64> {
    if reply.len() < 48 || reply[0] & 0x07 != 4 || reply[1] == 0 {
        // Not a server's answer, or a "kiss of death" (stratum 0).
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
    sock.set_read_timeout(Some(timeout))?;
    let mut req = [0u8; 48];
    req[0] = 0x23; // version 4, client
    let t1 = now();
    sock.send_to(&req, addr)?;
    let mut buf = [0u8; 68];
    let (n, _) = sock.recv_from(&mut buf)?;
    let t4 = now();
    sntp_offset(&buf[..n], t1, t4).ok_or_else(|| io::Error::other("not an NTP answer"))
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
        let r = judge_https(&own);
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
        let r = judge_https(&dpi);
        assert_eq!(r.status, Status::Warn);
        assert!(r.detail.contains("SonicWall Firewall DPI-SSL") && r.detail.contains("public CA"));
        // An unknown product, but one issuer on two unrelated sites, and
        // not trusted here: browsers will complain.
        let odd = [
            seen("www.cloudflare.com", "Acme Gateway CA", false),
            seen("github.com", "Acme Gateway CA", false),
            Err("refused".into()),
        ];
        let r = judge_https(&odd);
        assert_eq!(r.status, Status::Fail);
        assert!(
            r.detail.contains("same issuer signed 2 unrelated sites"),
            "{}",
            r.detail
        );
        assert!(r.detail.contains("doesn't trust"));
        // Untrusted but each its own CA: the clock or the store, not inspection.
        let r = judge_https(&[seen("github.com", "Sectigo", false)]);
        assert_eq!(r.status, Status::Warn);
        assert!(r.detail.contains("clock"));
        // Nothing reached.
        assert_eq!(judge_https(&[Err("x".into())]).status, Status::Unknown);
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
        println!("{:#?}", judge_https(&seen));
        let off = offset(Duration::from_secs(4));
        println!("{:#?}", judge_clock(off.as_ref(), None));
    }

    /// A server 2 s ahead of us, answering at once.
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
        let mut reply = vec![0u8; 48];
        reply[0] = 0x24; // version 4, server
        reply[1] = 2; // stratum
        reply[32..40].copy_from_slice(&ts(t1 + 2.05));
        reply[40..48].copy_from_slice(&ts(t1 + 2.05));
        let o = sntp_offset(&reply, t1, t4).unwrap();
        assert!((o + 2.0).abs() < 0.01, "{o}: this computer is 2 s slow");
        reply[1] = 0;
        assert_eq!(sntp_offset(&reply, t1, t4), None, "kiss of death");
        assert_eq!(sntp_offset(&reply[..20], t1, t4), None);
    }
}
