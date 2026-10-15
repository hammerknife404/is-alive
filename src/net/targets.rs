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

/// Accepts `10.0.0.0/24`, `10.0.0.1-254`, `10.0.0.10-10.0.1.20` or a single IP.
/// CIDR blocks of /30 and larger exclude the network and broadcast addresses.
pub fn parse_range(spec: &str) -> Result<Vec<Ipv4Addr>, String> {
    let spec = spec.trim();
    let ip = |s: &str| {
        s.trim()
            .parse::<Ipv4Addr>()
            .map_err(|_| format!("invalid IPv4 address '{}'", s.trim()))
    };

    let (start, end) = if let Some((base, bits)) = spec.split_once('/') {
        let bits: u32 = bits
            .trim()
            .parse()
            .ok()
            .filter(|b| *b <= 32)
            .ok_or("invalid prefix length")?;
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
    Ok((start..=end).map(Ipv4Addr::from).collect())
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
}
