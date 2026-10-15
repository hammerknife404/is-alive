//! IPv4 range parsing for subnet sweeps.

use std::net::Ipv4Addr;

pub const MAX_HOSTS: usize = 65_536;

/// The host's subnet on its default-route interface, as CIDR. Networks
/// wider than /24 are narrowed to the host's /24 so a first sweep stays quick.
pub fn local_subnet() -> Option<String> {
    // Connecting a UDP socket only does a route lookup; nothing is sent.
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    let std::net::IpAddr::V4(ip) = sock.local_addr().ok()?.ip() else {
        return None;
    };
    let mask = if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .find_map(|i| match i.addr {
            if_addrs::IfAddr::V4(a) if a.ip == ip => Some(a.netmask),
            _ => None,
        })?;
    Some(subnet_cidr(ip, mask))
}

fn subnet_cidr(ip: Ipv4Addr, mask: Ipv4Addr) -> String {
    let bits = u32::from(mask).leading_ones().clamp(24, 32);
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
        assert_eq!(
            subnet_cidr(ip, Ipv4Addr::new(255, 255, 255, 0)),
            "10.1.2.0/24"
        );
        assert_eq!(
            subnet_cidr(ip, Ipv4Addr::new(255, 255, 255, 240)),
            "10.1.2.0/28"
        );
        assert_eq!(
            subnet_cidr(ip, Ipv4Addr::new(255, 255, 0, 0)),
            "10.1.2.0/24"
        );
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
