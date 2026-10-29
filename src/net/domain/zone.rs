//! Zone and registration checks: nameservers, DNSSEC, CAA and domain expiry.

use super::dnsq::{self, Client, Data};
use super::{Check, addrs, http_get, parent, zone_of};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::OnceLock;

fn norm_set(v: &[String]) -> Vec<String> {
    let mut v: Vec<String> = v
        .iter()
        .map(|s| s.trim_end_matches('.').to_ascii_lowercase())
        .collect();
    v.sort();
    v.dedup();
    v
}

pub(super) fn nameservers(c: &Client, d: &str) -> Check {
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

pub(super) fn dnssec(c: &Client, d: &str) -> Check {
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

pub(super) fn caa(c: &Client, d: &str) -> Check {
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

/// TLD (lowercase) to RDAP base URL, from the IANA bootstrap file.
fn rdap_services(bootstrap: &str) -> Option<HashMap<String, String>> {
    let j: serde_json::Value = serde_json::from_str(bootstrap).ok()?;
    let mut m = HashMap::new();
    for s in j["services"].as_array()? {
        let (Some(tlds), Some(url)) = (s[0].as_array(), s[1][0].as_str()) else {
            continue;
        };
        for t in tlds.iter().filter_map(|t| t.as_str()) {
            m.entry(t.to_ascii_lowercase())
                .or_insert_with(|| url.to_string());
        }
    }
    // An empty map would be cached and make every TLD read "no RDAP".
    (!m.is_empty()).then_some(m)
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

pub(super) fn expiry(d: &str) -> Check {
    let mut k = Check::default();
    let tld = d.rsplit('.').next().unwrap_or(d);
    // The IANA bootstrap file changes rarely; fetch it once per session, and
    // cache it only once it parses (a captive portal page must not stick).
    static SERVICES: OnceLock<HashMap<String, String>> = OnceLock::new();
    let services = match SERVICES.get() {
        Some(s) => s,
        None => match http_get("https://data.iana.org/rdap/dns.json", true) {
            Ok(b) => match rdap_services(&b) {
                Some(s) => SERVICES.get_or_init(|| s),
                None => {
                    k.warn("the IANA RDAP registry answer couldn't be parsed");
                    return k;
                }
            },
            Err(e) => {
                k.warn(format!("couldn't reach the IANA RDAP registry: {e}"));
                return k;
            }
        },
    };
    let Some(base) = services.get(&tld.to_ascii_lowercase()) else {
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
    fn rdap_bootstrap_and_record() {
        let boot = r#"{"services":[[["net","com"],["https://rdap.verisign.com/com/v1/"]]]}"#;
        let m = rdap_services(boot).unwrap();
        assert_eq!(
            m.get("com").map(String::as_str),
            Some("https://rdap.verisign.com/com/v1/")
        );
        assert_eq!(m.get("org"), None);
        assert!(rdap_services("<html>captive portal</html>").is_none());
        assert!(rdap_services(r#"{"services":[]}"#).is_none());
        let body = r#"{"status":["client transfer prohibited"],
            "events":[{"eventAction":"registration","eventDate":"1997-09-15T04:00:00Z"},
                      {"eventAction":"expiration","eventDate":"2028-09-14T04:00:00Z"}],
            "entities":[{"roles":["registrar"],"vcardArray":["vcard",[["version",{},"text","4.0"],["fn",{},"text","MarkMonitor Inc."]]]}]}"#;
        let r = parse_rdap(body).unwrap();
        assert_eq!(r.expires.as_deref(), Some("2028-09-14T04:00:00Z"));
        assert_eq!(r.registrar.as_deref(), Some("MarkMonitor Inc."));
        assert_eq!(r.status, vec!["client transfer prohibited"]);
    }
}
