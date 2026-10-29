//! DNS blocklists (RBL/DNSBL): is a sending IP, or the domain itself, listed?
//! A listing is an A record under the list's zone; the 127.0.0.x code says why.

use super::dnsq::{self, Client};
use super::{Check, MxInfo, txt_with};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

struct List {
    zone: &'static str,
    name: &'static str,
    /// Widely used for rejecting mail: a listing here is a FAIL, not a WARN.
    major: bool,
    /// Lists domains (URIs in mail bodies) rather than sending IPs.
    domain: bool,
}

const LISTS: &[List] = &[
    List {
        zone: "zen.spamhaus.org",
        name: "Spamhaus ZEN",
        major: true,
        domain: false,
    },
    List {
        zone: "b.barracudacentral.org",
        name: "Barracuda",
        major: true,
        domain: false,
    },
    List {
        zone: "bl.spamcop.net",
        name: "SpamCop",
        major: true,
        domain: false,
    },
    List {
        zone: "psbl.surriel.com",
        name: "PSBL",
        major: false,
        domain: false,
    },
    List {
        zone: "bl.mailspike.net",
        name: "Mailspike",
        major: false,
        domain: false,
    },
    List {
        zone: "dnsbl-1.uceprotect.net",
        name: "UCEPROTECT L1",
        major: false,
        domain: false,
    },
    List {
        zone: "ix.dnsbl.manitu.net",
        name: "NiX Spam",
        major: false,
        domain: false,
    },
    List {
        zone: "dnsbl.dronebl.org",
        name: "DroneBL",
        major: false,
        domain: false,
    },
    List {
        zone: "dbl.spamhaus.org",
        name: "Spamhaus DBL",
        major: true,
        domain: true,
    },
    List {
        zone: "multi.uribl.com",
        name: "URIBL",
        major: true,
        domain: true,
    },
    List {
        zone: "multi.surbl.org",
        name: "SURBL",
        major: true,
        domain: true,
    },
];

#[derive(Debug, PartialEq)]
enum Answer {
    Clean,
    Listed(Ipv4Addr),
    /// Spamhaus PBL: an end-user/dynamic range, not evidence of spam.
    Policy,
    /// The list declined the query (public or high-volume resolver).
    Refused,
    /// A non-127/8 answer: a resolver rewriting NXDOMAIN, not a listing.
    Bogus,
}

/// What a list's answer means. Spamhaus signals refusals with 127.255.255.x,
/// URIBL and SURBL with 127.0.0.1; listings are other 127.0.0.x codes.
fn classify(list: &List, ips: &[IpAddr]) -> Answer {
    let Some(ip) = ips.iter().find_map(|ip| match ip {
        IpAddr::V4(v4) => Some(*v4),
        IpAddr::V6(_) => None,
    }) else {
        return Answer::Clean;
    };
    let o = ip.octets();
    if o[0] != 127 {
        return Answer::Bogus;
    }
    if o[1] == 255 && o[2] == 255 {
        return Answer::Refused;
    }
    if list.domain && ip == Ipv4Addr::new(127, 0, 0, 1) {
        return Answer::Refused;
    }
    if list.zone == "zen.spamhaus.org" && matches!(o[3], 10 | 11) {
        return Answer::Policy;
    }
    Answer::Listed(ip)
}

/// Query `name` on `c`; NXDOMAIN or an empty answer is clean.
fn ask(c: &Client, list: &List, name: &str) -> Result<Answer, String> {
    let m = c.ask(name, dnsq::A).map_err(|e| e.to_string())?;
    Ok(classify(list, &m.ips()))
}

/// Look up one name on a list: via the chosen resolver, then, if the list
/// refuses it (Spamhaus and URIBL block public resolvers), via this
/// machine's own DNS server.
fn lookup(c: &Client, list: &List, name: &str) -> Result<Answer, String> {
    let a = ask(c, list, name)?;
    if a != Answer::Refused {
        return Ok(a);
    }
    match dnsq::system_servers().first() {
        Some(&ip) if SocketAddr::new(ip, 53) != c.server => {
            ask(&Client::new(SocketAddr::new(ip, 53)), list, name)
        }
        _ => Ok(Answer::Refused),
    }
}

fn reversed(ip: Ipv4Addr) -> String {
    let o = ip.octets();
    format!("{}.{}.{}.{}", o[3], o[2], o[1], o[0])
}

/// SPF ip4: entries that are single addresses (ranges can't be checked as one).
fn spf_ip4(record: &str) -> Vec<Ipv4Addr> {
    record
        .split_whitespace()
        .filter_map(|t| {
            let a = t
                .trim_start_matches(['+', '-', '~', '?'])
                .strip_prefix("ip4:")?;
            a.strip_suffix("/32").unwrap_or(a).parse().ok()
        })
        .collect()
}

pub(super) fn blacklists(c: &Client, d: &str, mx: &MxInfo, wan: Option<Ipv4Addr>) -> Check {
    Check::run(|k| {
        // Which IPs send this domain's mail is unknowable from DNS, so check
        // the likely ones: self-hosted MX, SPF-listed addresses, and the
        // public IP of the network running the check.
        let mut ips: Vec<(Ipv4Addr, Vec<&str>)> = Vec::new();
        let mut add = |ip: Ipv4Addr, why: &'static str| match ips.iter_mut().find(|e| e.0 == ip) {
            Some(e) => e.1.push(why),
            None => ips.push((ip, vec![why])),
        };
        if mx.hosted() {
            k.info("MX is a hosted mail service: its IPs are the provider's to keep clean, not checked");
        } else {
            for ip in &mx.ips {
                if let IpAddr::V4(v4) = ip {
                    add(*v4, "MX");
                }
            }
        }
        let spf = txt_with(c, d, "v=spf1").unwrap_or_default();
        for ip in spf.first().map(|r| spf_ip4(r)).unwrap_or_default() {
            add(ip, "SPF");
        }
        if let Some(w) = wan {
            add(w, "this network's WAN");
        }
        ips.truncate(12);

        // Every (IP, list) and (domain, list) pair, in parallel.
        let ip_lists: Vec<&List> = LISTS.iter().filter(|l| !l.domain).collect();
        let dom_lists: Vec<&List> = LISTS.iter().filter(|l| l.domain).collect();
        let (ip_results, dom_results) = std::thread::scope(|s| {
            let ip_hs: Vec<_> = ips
                .iter()
                .flat_map(|(ip, _)| ip_lists.iter().map(move |l| (*ip, *l)))
                .map(|(ip, l)| {
                    s.spawn(move || (ip, l, lookup(c, l, &format!("{}.{}", reversed(ip), l.zone))))
                })
                .collect();
            let dom_hs: Vec<_> = dom_lists
                .iter()
                .map(|&l| s.spawn(move || (l, lookup(c, l, &format!("{d}.{}", l.zone)))))
                .collect();
            (
                ip_hs
                    .into_iter()
                    .filter_map(|h| h.join().ok())
                    .collect::<Vec<_>>(),
                dom_hs
                    .into_iter()
                    .filter_map(|h| h.join().ok())
                    .collect::<Vec<_>>(),
            )
        });

        let mut refused: Vec<&str> = Vec::new();
        let mut clean_ips = 0;
        for (ip, why) in &ips {
            let label = format!("{ip} ({})", why.join(", "));
            let mut hits = Vec::new();
            for (_, l, r) in ip_results.iter().filter(|r| r.0 == *ip) {
                match r {
                    Ok(Answer::Listed(code)) => {
                        hits.push(l.name);
                        let msg = format!("{label} is listed on {} (code {code})", l.name);
                        if l.major {
                            k.fail(format!("{msg}: mail from it is likely rejected"));
                        } else {
                            k.warn(format!("{msg}: some receivers use this list"));
                        }
                    }
                    Ok(Answer::Policy) => {
                        hits.push("Spamhaus PBL");
                        // An end-user range is fine unless SPF says it sends mail.
                        let msg =
                            format!("{label} is on the Spamhaus PBL (end-user/dynamic range)");
                        if why.contains(&"SPF") {
                            k.warn(format!("{msg}, but SPF authorizes it to send: receivers using ZEN reject direct mail from it"));
                        } else {
                            k.info(format!("{msg}: fine unless it sends mail directly"));
                        }
                    }
                    Ok(Answer::Refused) => refused.push(l.name),
                    Ok(Answer::Clean | Answer::Bogus) | Err(_) => {}
                }
            }
            if hits.is_empty() {
                clean_ips += 1;
                k.rec(format!("{label}  clean"));
            } else {
                k.rec(format!("{label}  listed: {}", hits.join(", ")));
            }
        }
        let mut dom_hits = Vec::new();
        for (l, r) in &dom_results {
            match r {
                Ok(Answer::Listed(code)) => {
                    dom_hits.push(l.name);
                    k.fail(format!(
                        "{d} is listed on {} (code {code}): mail mentioning it gets junked or rejected",
                        l.name
                    ));
                }
                Ok(Answer::Refused) => refused.push(l.name),
                _ => {}
            }
        }
        k.rec(if dom_hits.is_empty() {
            format!("{d}  clean")
        } else {
            format!("{d}  listed: {}", dom_hits.join(", "))
        });
        refused.sort_unstable();
        refused.dedup();
        if !refused.is_empty() {
            k.info(format!(
                "couldn't check {}: they refuse public resolvers; set Resolver to your ISP's DNS to include them",
                refused.join(", ")
            ));
        }
        if clean_ips == ips.len() && dom_hits.is_empty() {
            k.pass(format!(
                "{} IP(s) and the domain are not listed on {} blocklists",
                ips.len(),
                LISTS.len()
            ));
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Vec<IpAddr> {
        vec![s.parse().unwrap()]
    }

    #[test]
    fn classifies_list_answers() {
        let zen = &LISTS[0];
        let uribl = LISTS.iter().find(|l| l.zone == "multi.uribl.com").unwrap();
        assert_eq!(classify(zen, &[]), Answer::Clean);
        assert_eq!(
            classify(zen, &ip("127.0.0.2")),
            Answer::Listed(Ipv4Addr::new(127, 0, 0, 2))
        );
        assert_eq!(classify(zen, &ip("127.0.0.11")), Answer::Policy);
        assert_eq!(classify(zen, &ip("127.255.255.254")), Answer::Refused);
        assert_eq!(classify(uribl, &ip("127.0.0.1")), Answer::Refused);
        assert_eq!(classify(zen, &ip("203.0.113.9")), Answer::Bogus);
        assert_eq!(reversed(Ipv4Addr::new(192, 0, 2, 5)), "5.2.0.192");
    }

    #[test]
    fn reads_spf_single_addresses() {
        let rec = "v=spf1 ip4:192.0.2.1 +ip4:192.0.2.2/32 ip4:198.51.100.0/24 include:x -all";
        assert_eq!(
            spf_ip4(rec),
            [Ipv4Addr::new(192, 0, 2, 1), Ipv4Addr::new(192, 0, 2, 2)]
        );
    }
}
