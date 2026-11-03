//! IPv4 range parsing for subnet sweeps.

use std::net::Ipv4Addr;

pub const MAX_HOSTS: usize = 65_536;

/// The host's subnet on its default-route interface, as CIDR. Networks
/// wider than /22 (1022 hosts) are narrowed to the host's /22 so a sweep stays quick.
pub fn local_subnet() -> Option<String> {
    let (ip, bits) = super::hostinfo::lan()?;
    Some(subnet_cidr(ip, bits))
}

pub fn subnet_cidr(ip: Ipv4Addr, prefix: u32) -> String {
    let bits = prefix.clamp(22, 32);
    let net = u32::from(ip) & (u32::MAX << (32 - bits));
    format!("{}/{bits}", Ipv4Addr::from(net))
}

/// Examples of what a sweep takes, for the box's hint and tooltip.
pub const EXAMPLES: &str = "10.10.10.0/24 192.168.100.1-50 172.20.5.12 10.0.0.0/28";

/// One or more targets: `10.0.0.0/24`, `10.0.0.0/255.255.255.0` or
/// `10.0.0.0 255.255.255.0`, `10.0.0.1-254`, `10.0.0.10-10.0.1.20` (or
/// `… to …`), `10.0.0.*`, or a single IP. Separated by spaces, commas,
/// semicolons, `|` or new lines; spaces around `-` and `/` and a pasted
/// en dash are fine. CIDR blocks of /30 and larger exclude the network and
/// broadcast addresses. Overlaps are swept once; addresses come back in order.
pub fn parse_range(spec: &str) -> Result<Vec<Ipv4Addr>, String> {
    let spans = spans(spec)?;
    Ok(spans
        .into_iter()
        .flat_map(|(a, z)| (a..=z).map(Ipv4Addr::from))
        .collect())
}

/// How many targets `spec` lists and how many addresses they come to
/// (overlaps counted once), without listing them.
pub fn count(spec: &str) -> Result<(usize, usize), String> {
    let parts = tokens(spec).len();
    let n = spans(spec)?
        .iter()
        .map(|&(a, z)| (z - a) as usize + 1)
        .sum();
    Ok((parts, n))
}

/// The targets as address spans, sorted and merged where they overlap or
/// touch.
fn spans(spec: &str) -> Result<Vec<(u32, u32)>, String> {
    let toks = tokens(spec);
    if toks.is_empty() {
        return Err(format!("nothing to sweep: enter a range, e.g. {EXAMPLES}"));
    }
    let mut v = Vec::with_capacity(toks.len());
    for t in &toks {
        let (a, z) = one(t).map_err(|e| {
            if toks.len() > 1 {
                format!("{t}: {e}")
            } else {
                e
            }
        })?;
        v.push((a, z));
    }
    v.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(v.len());
    for (a, z) in v {
        match merged.last_mut() {
            Some(last) if a <= last.1.saturating_add(1) => last.1 = last.1.max(z),
            _ => merged.push((a, z)),
        }
    }
    let n: usize = merged.iter().map(|&(a, z)| (z - a) as usize + 1).sum();
    if n > MAX_HOSTS {
        return Err(format!(
            "{n} addresses in all; the limit is {MAX_HOSTS} (a /16)"
        ));
    }
    Ok(merged)
}

/// `spec` split into one string per target: separators become spaces,
/// spaces around `-` and `/` go, and `ip mask` or `ip to ip` pairs join.
fn tokens(spec: &str) -> Vec<String> {
    let spaced: String = spec
        .chars()
        .map(|c| match c {
            ',' | ';' | '|' => ' ',
            // A dash pasted from a document or a mail.
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            c if c.is_whitespace() => ' ',
            c => c,
        })
        .collect();
    let words: Vec<&str> = spaced.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for w in words {
        let joins = |t: &str| t.starts_with(['-', '/']) || t.eq_ignore_ascii_case("to");
        let open = |t: &str| t.ends_with(['-', '/']);
        match out.last_mut() {
            // `a - b`, `a -b`, `a- b`, `a / 24`, `a to b`.
            Some(last) if open(last) || joins(w) => {
                if w.eq_ignore_ascii_case("to") {
                    last.push('-');
                } else {
                    last.push_str(w);
                }
            }
            // `10.0.0.0 255.255.255.0`: an address, then a mask.
            Some(last) if is_plain_ip(last) && mask_bits(w).is_some() => {
                last.push('/');
                last.push_str(w);
            }
            _ => out.push(w.to_string()),
        }
    }
    out
}

fn is_plain_ip(s: &str) -> bool {
    s.parse::<Ipv4Addr>().is_ok()
}

/// The prefix length of a dotted netmask (`255.255.255.0` is 24). Only
/// masks starting 255. count, so no sweepable address is taken for one.
fn mask_bits(s: &str) -> Option<u32> {
    let m = u32::from(s.parse::<Ipv4Addr>().ok()?);
    let bits = m.leading_ones();
    (m >> 24 == 255 && m.checked_shl(bits).unwrap_or(0) == 0).then_some(bits)
}

/// One target as its first and last address.
fn one(spec: &str) -> Result<(u32, u32), String> {
    let ip = |s: &str| {
        s.trim()
            .parse::<Ipv4Addr>()
            .map_err(|_| format!("invalid IPv4 address '{}'", s.trim()))
    };
    // `10.0.0.*`, `10.0.*.*`: the block those octets span.
    let stars = spec.split('.').rev().take_while(|o| *o == "*").count();
    let spec = if (1..=2).contains(&stars) && spec.split('.').count() == 4 {
        let base = spec
            .split('.')
            .map(|o| if o == "*" { "0" } else { o })
            .collect::<Vec<_>>()
            .join(".");
        format!("{base}/{}", 32 - 8 * stars)
    } else {
        spec.to_string()
    };
    let spec = spec.as_str();

    let (start, end) = if let Some((base, bits)) = spec.split_once('/') {
        let bits: u32 = match bits.trim().parse::<u32>() {
            Ok(b) if b <= 32 => b,
            _ => mask_bits(bits.trim()).ok_or("invalid prefix length")?,
        };
        let mask = if bits == 0 {
            0
        } else {
            u32::MAX << (32 - bits)
        };
        let net = u32::from(ip(base)?) & mask;
        let bcast = net | !mask;
        if bits <= 30 {
            (net + 1, bcast - 1)
        } else {
            (net, bcast)
        }
    } else if let Some((a, b)) = spec.split_once('-') {
        let a = ip(a)?;
        let b = match b.trim().parse::<u8>() {
            Ok(last) => {
                let o = a.octets();
                Ipv4Addr::new(o[0], o[1], o[2], last)
            }
            Err(_) => ip(b)?,
        };
        (u32::from(a), u32::from(b))
    } else {
        let a = u32::from(ip(spec)?);
        (a, a)
    };

    if start > end {
        return Err("range start is after range end".into());
    }
    let count = (end - start) as usize + 1;
    if count > MAX_HOSTS {
        return Err(format!("range has {count} hosts; limit is {MAX_HOSTS}"));
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnet_cidr_narrows_wide_networks() {
        let ip = Ipv4Addr::new(10, 1, 2, 3);
        assert_eq!(subnet_cidr(ip, 24), "10.1.2.0/24");
        assert_eq!(subnet_cidr(ip, 28), "10.1.2.0/28");
        assert_eq!(subnet_cidr(ip, 23), "10.1.2.0/23");
        assert_eq!(subnet_cidr(ip, 16), "10.1.0.0/22");
    }

    #[test]
    fn cidr() {
        let v = parse_range("192.168.1.77/24").unwrap();
        assert_eq!(v.len(), 254);
        assert_eq!(v[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(*v.last().unwrap(), Ipv4Addr::new(192, 168, 1, 254));
        assert_eq!(parse_range("10.0.0.0/31").unwrap().len(), 2);
        assert_eq!(parse_range("10.0.0.5/32").unwrap().len(), 1);
        assert!(parse_range("10.0.0.0/8").is_err());
    }

    #[test]
    fn dashes() {
        assert_eq!(parse_range("10.0.0.10-20").unwrap().len(), 11);
        assert_eq!(parse_range("10.0.0.250-10.0.1.5").unwrap().len(), 12);
        assert!(parse_range("10.0.0.20-10").is_err());
        assert_eq!(
            parse_range("10.1.2.3").unwrap(),
            vec![Ipv4Addr::new(10, 1, 2, 3)]
        );
    }

    #[test]
    fn several_targets_with_any_common_separator() {
        let want = 254 + 50 + 1 + 14;
        for spec in [
            "10.10.10.0/24 192.168.100.1-50 172.20.5.12 10.0.0.0/28",
            "10.10.10.0/24, 192.168.100.1-50, 172.20.5.12, 10.0.0.0/28",
            "10.10.10.0/24;192.168.100.1-50;172.20.5.12;10.0.0.0/28",
            "10.10.10.0/24\n192.168.100.1-50\r\n172.20.5.12\t10.0.0.0/28",
            "  10.10.10.0 / 24 ,192.168.100.1 - 50 | 172.20.5.12 ;; 10.0.0.0 /28  ",
            "10.10.10.0 255.255.255.0 192.168.100.1 to 192.168.100.50 172.20.5.12 10.0.0.0/255.255.255.240",
            "10.10.10.* 192.168.100.1\u{2013}50 172.20.5.12 10.0.0.0/28",
        ] {
            let v = parse_range(spec).unwrap_or_else(|e| panic!("{spec}: {e}"));
            assert_eq!(v.len(), want, "{spec}");
            assert_eq!(count(spec).unwrap(), (4, want), "{spec}");
            assert!(v.windows(2).all(|w| w[0] < w[1]), "{spec}: in order");
            assert!(v.contains(&Ipv4Addr::new(172, 20, 5, 12)));
            assert!(v.contains(&Ipv4Addr::new(192, 168, 100, 50)));
            assert!(!v.contains(&Ipv4Addr::new(192, 168, 100, 51)));
        }
    }

    #[test]
    fn overlaps_are_swept_once() {
        let v = parse_range("10.0.0.0/24 10.0.0.10-20 10.0.0.5").unwrap();
        assert_eq!(v.len(), 254);
        assert_eq!(count("10.0.0.0/24 10.0.0.10-20").unwrap(), (2, 254));
        // Two /16s that are the same /16 fit; two different ones don't.
        assert_eq!(
            parse_range("10.1.0.0/16 10.1.0.0/16").unwrap().len(),
            65_534
        );
        let e = parse_range("10.1.0.0/16 10.2.0.0/16").unwrap_err();
        assert!(e.contains("131068 addresses in all"), "{e}");
    }

    #[test]
    fn a_bad_target_is_named() {
        let e = parse_range("10.0.0.0/24 10.0.0.300 10.0.1.1").unwrap_err();
        assert!(e.starts_with("10.0.0.300:"), "{e}");
        let e = parse_range("10.0.0.0/24 10.0.0.50-10").unwrap_err();
        assert!(e.contains("10.0.0.50-10") && e.contains("after"), "{e}");
        // One target alone says what's wrong without repeating it.
        assert_eq!(
            parse_range("10.0.0.300").unwrap_err(),
            "invalid IPv4 address '10.0.0.300'"
        );
        assert!(
            parse_range("   ")
                .unwrap_err()
                .starts_with("nothing to sweep")
        );
        assert!(parse_range(",;").is_err());
    }

    #[test]
    fn masks_and_wildcards() {
        assert_eq!(mask_bits("255.255.255.0"), Some(24));
        assert_eq!(mask_bits("255.255.252.0"), Some(22));
        assert_eq!(mask_bits("255.255.255.255"), Some(32));
        assert_eq!(mask_bits("255.0.255.0"), None);
        assert_eq!(mask_bits("10.0.0.1"), None);
        // Two plain addresses stay two targets.
        assert_eq!(parse_range("10.0.0.1 10.0.0.9").unwrap().len(), 2);
        assert_eq!(parse_range("10.0.*.*").unwrap().len(), 65_534);
        assert!(parse_range("10.*.*.*").is_err());
        assert!(parse_range("10.0.0.0/255.0.255.0").is_err());
    }
}
