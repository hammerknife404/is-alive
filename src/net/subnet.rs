//! Subnet calculator maths for IPv4 and IPv6: what a block holds, splitting
//! it into smaller ones, and the fewest blocks that cover a range. Addresses
//! are handled as u128 with a width of 32 or 128 bits, so both families
//! share one code path.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// What the user typed: a block (address and prefix) or a range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Block(Block),
    Range(IpAddr, IpAddr),
}

/// An address and a prefix length. The address may be any address in the
/// block, not only its network address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub ip: IpAddr,
    pub prefix: u8,
}

const HINT: &str = "add a prefix (/24) or a mask (255.255.255.0)";

/// Accepts `10.0.0.0/22`, `10.0.0.5/255.255.252.0`, `10.0.0.5 255.255.252.0`,
/// a wildcard mask (`10.0.0.0 0.0.3.255`, as in an ACL), `2001:db8::/48`,
/// and ranges `10.0.0.1-10.0.0.99` or `10.0.0.1-99`.
pub fn parse(spec: &str) -> Result<Input, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(HINT.into());
    }
    if let Some((a, b)) = spec.split_once('-') {
        let a = addr(a)?;
        let b = match (a, b.trim().parse::<u8>()) {
            (IpAddr::V4(v4), Ok(last)) => {
                let o = v4.octets();
                IpAddr::V4(Ipv4Addr::new(o[0], o[1], o[2], last))
            }
            _ => addr(b)?,
        };
        if a.is_ipv4() != b.is_ipv4() {
            return Err("the range mixes IPv4 and IPv6".into());
        }
        if num(a) > num(b) {
            return Err("range start is after range end".into());
        }
        return Ok(Input::Range(a, b));
    }
    let (ip, suffix) = match spec.split_once('/') {
        Some((ip, s)) => (ip, Some(s.trim())),
        None => match spec.split_once(char::is_whitespace) {
            Some((ip, s)) => (ip, Some(s.trim())),
            None => (spec, None),
        },
    };
    let ip = addr(ip)?;
    let w = width(ip);
    let prefix = match suffix {
        None => return Err(HINT.into()),
        Some(s) => match s.parse::<u8>() {
            Ok(p) if u32::from(p) <= w => p,
            Ok(_) => return Err(format!("prefix must be 0 to {w}")),
            Err(_) => {
                let m: Ipv4Addr = s
                    .parse()
                    .map_err(|_| format!("'{s}' isn't a prefix length or a mask"))?;
                if !ip.is_ipv4() {
                    return Err("an IPv6 block takes a prefix length (/64), not a mask".into());
                }
                mask_prefix(u32::from(m))
                    .or_else(|| mask_prefix(!u32::from(m)))
                    .ok_or_else(|| format!("{m} isn't a valid netmask or wildcard mask"))?
            }
        },
    };
    Ok(Input::Block(Block { ip, prefix }))
}

fn addr(s: &str) -> Result<IpAddr, String> {
    let s = s.trim();
    s.parse().map_err(|_| format!("'{s}' isn't an IP address"))
}

/// Prefix length of a contiguous netmask (ones then zeros).
fn mask_prefix(m: u32) -> Option<u8> {
    let p = m.leading_ones();
    (m.checked_shl(p).unwrap_or(0) == 0).then_some(p as u8)
}

fn width(ip: IpAddr) -> u32 {
    if ip.is_ipv4() { 32 } else { 128 }
}

fn num(ip: IpAddr) -> u128 {
    match ip {
        IpAddr::V4(a) => u32::from(a).into(),
        IpAddr::V6(a) => a.into(),
    }
}

fn ip_of(n: u128, v6: bool) -> IpAddr {
    if v6 {
        IpAddr::V6(Ipv6Addr::from(n))
    } else {
        IpAddr::V4(Ipv4Addr::from(n as u32))
    }
}

/// Every bit of an address `w` bits wide.
fn all(w: u32) -> u128 {
    u128::MAX.checked_shr(128 - w).unwrap_or(0)
}

fn mask(prefix: u8, w: u32) -> u128 {
    all(w) ^ all(w).checked_shr(prefix.into()).unwrap_or(0)
}

/// Everything the calculator shows for one block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub ip: IpAddr,
    pub prefix: u8,
    pub network: IpAddr,
    /// The broadcast address on IPv4; the block's last address on IPv6.
    pub last: IpAddr,
    pub mask: IpAddr,
    pub wildcard: IpAddr,
    pub first_host: IpAddr,
    pub last_host: IpAddr,
    /// Addresses in the block: 2 to this power.
    pub host_bits: u32,
    /// Hosts that can be given addresses (IPv4 leaves out network and
    /// broadcast below /31); None when the count is too large to hold.
    pub usable: Option<u128>,
    /// Which special range this is (private, CGNAT, link-local…).
    pub kind: String,
    pub prev: Option<IpAddr>,
    pub next: Option<IpAddr>,
}

impl Block {
    pub fn v6(&self) -> bool {
        self.ip.is_ipv6()
    }

    fn width(&self) -> u32 {
        width(self.ip)
    }

    fn net(&self) -> u128 {
        num(self.ip) & mask(self.prefix, self.width())
    }

    fn size_bits(&self) -> u32 {
        self.width() - u32::from(self.prefix)
    }

    /// The block's last address.
    fn end(&self) -> u128 {
        self.net() | (all(self.width()) & !mask(self.prefix, self.width()))
    }

    /// Network address and prefix, as `10.0.0.0/22`.
    pub fn cidr(&self) -> String {
        format!("{}/{}", ip_of(self.net(), self.v6()), self.prefix)
    }

    /// Whether `ip` falls inside the block; and if so, its offset from the
    /// network address.
    pub fn offset(&self, ip: IpAddr) -> Option<u128> {
        (ip.is_ipv6() == self.v6() && num(ip) & mask(self.prefix, self.width()) == self.net())
            .then(|| num(ip) - self.net())
    }

    pub fn info(&self) -> Info {
        let (w, v6) = (self.width(), self.v6());
        let (net, end) = (self.net(), self.end());
        let m = mask(self.prefix, w);
        let bits = self.size_bits();
        let total = 1u128.checked_shl(bits);
        // IPv4 keeps the network and broadcast addresses back, except on
        // /31 (point-to-point, RFC 3021) and /32. IPv6 has no broadcast.
        let (first_host, last_host, usable) = if !v6 && bits >= 2 {
            (net + 1, end - 1, total.map(|t| t - 2))
        } else {
            (net, end, total)
        };
        let step = total;
        let prev = step.and_then(|s| net.checked_sub(s)).map(|n| ip_of(n, v6));
        let next = end
            .checked_add(1)
            .filter(|n| *n <= all(w))
            .map(|n| ip_of(n, v6));
        Info {
            ip: self.ip,
            prefix: self.prefix,
            network: ip_of(net, v6),
            last: ip_of(end, v6),
            mask: ip_of(m, v6),
            wildcard: ip_of(all(w) & !m, v6),
            first_host: ip_of(first_host, v6),
            last_host: ip_of(last_host, v6),
            host_bits: bits,
            usable,
            kind: kind(net, end, v6),
            prev,
            next,
        }
    }

    /// How many /`to` blocks this one splits into (2 to this power).
    pub fn split_bits(&self, to: u8) -> u32 {
        u32::from(to.saturating_sub(self.prefix))
    }

    /// The `i`th /`to` block inside this one.
    pub fn sub(&self, to: u8, i: u128) -> Block {
        let size_bits = self.width() - u32::from(to);
        let n = self.net() + (i << size_bits);
        Block {
            ip: ip_of(n, self.v6()),
            prefix: to,
        }
    }
}

/// The fewest CIDR blocks that exactly cover `a` to `b` (inclusive), as a
/// firewall address group would need them.
pub fn cover(a: IpAddr, b: IpAddr) -> Vec<Block> {
    let (v6, w) = (a.is_ipv6(), width(a));
    let (mut start, end) = (num(a), num(b));
    let mut out = Vec::new();
    while start <= end {
        // The largest aligned block at `start` that doesn't pass `end`.
        let mut bits = start.trailing_zeros().min(w);
        while bits > 0 && start + (all(bits)) > end {
            bits -= 1;
        }
        out.push(Block {
            ip: ip_of(start, v6),
            prefix: (w - bits) as u8,
        });
        match (start + all(bits)).checked_add(1) {
            Some(n) if n <= all(w) => start = n,
            _ => break,
        }
    }
    out
}

/// Addresses from `a` to `b` inclusive, as text (it can exceed u128 only for
/// the whole IPv6 space).
pub fn range_len(a: IpAddr, b: IpAddr) -> String {
    match (num(b) - num(a)).checked_add(1) {
        Some(n) => group(n),
        None => "2^128".into(),
    }
}

/// A count of 2^`bits` addresses, written out up to 2^64 and as a power above.
pub fn count(bits: u32, n: Option<u128>) -> String {
    match n {
        Some(n) if bits <= 64 => group(n),
        _ => format!("2^{bits}"),
    }
}

/// 1234567 → "1,234,567".
pub fn group(n: u128) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// An IPv4 mask in binary, octet by octet: `11111111.11111111.11111100.00000000`.
pub fn binary(ip: IpAddr) -> Option<String> {
    match ip {
        IpAddr::V4(a) => Some(
            a.octets()
                .iter()
                .map(|o| format!("{o:08b}"))
                .collect::<Vec<_>>()
                .join("."),
        ),
        IpAddr::V6(_) => None,
    }
}

/// (first, prefix, name) of the special-purpose ranges worth naming.
const V4: &[([u8; 4], u8, &str)] = &[
    ([0, 0, 0, 0], 8, "\"This network\" (RFC 791)"),
    ([10, 0, 0, 0], 8, "Private (RFC 1918)"),
    ([100, 64, 0, 0], 10, "Shared / carrier-grade NAT (RFC 6598)"),
    ([127, 0, 0, 0], 8, "Loopback"),
    ([169, 254, 0, 0], 16, "Link-local (APIPA)"),
    ([172, 16, 0, 0], 12, "Private (RFC 1918)"),
    ([192, 0, 2, 0], 24, "Documentation (TEST-NET-1)"),
    ([192, 168, 0, 0], 16, "Private (RFC 1918)"),
    ([198, 18, 0, 0], 15, "Benchmarking (RFC 2544)"),
    ([198, 51, 100, 0], 24, "Documentation (TEST-NET-2)"),
    ([203, 0, 113, 0], 24, "Documentation (TEST-NET-3)"),
    ([224, 0, 0, 0], 4, "Multicast"),
    ([240, 0, 0, 0], 4, "Reserved (class E)"),
];

const V6: &[(&str, u8, &str)] = &[
    ("::", 128, "Unspecified"),
    ("::1", 128, "Loopback"),
    ("::ffff:0:0", 96, "IPv4-mapped"),
    ("64:ff9b::", 96, "NAT64 (RFC 6052)"),
    ("2001:db8::", 32, "Documentation"),
    ("fc00::", 7, "Unique local (ULA, private)"),
    ("fe80::", 10, "Link-local"),
    ("ff00::", 8, "Multicast"),
    ("2000::", 3, "Global unicast (public)"),
];

/// Names the special range the block `net`…`end` sits in, or the ones it
/// spans part of.
fn kind(net: u128, end: u128, v6: bool) -> String {
    let ranges: Vec<(u128, u128, &str)> = if v6 {
        V6.iter()
            .map(|(a, p, name)| {
                let b = Block {
                    ip: a.parse().expect("listed IPv6 range"),
                    prefix: *p,
                };
                (b.net(), b.end(), *name)
            })
            .collect()
    } else {
        V4.iter()
            .map(|(a, p, name)| {
                let b = Block {
                    ip: IpAddr::V4(Ipv4Addr::from(*a)),
                    prefix: *p,
                };
                (b.net(), b.end(), *name)
            })
            .collect()
    };
    // The narrowest listed range holding the whole block (the list runs
    // from wide to narrow only within a family's overlaps, so pick by size).
    if let Some((.., name)) = ranges
        .iter()
        .filter(|(a, b, _)| *a <= net && end <= *b)
        .min_by_key(|(a, b, _)| b - a)
    {
        return (*name).into();
    }
    let mut part: Vec<&str> = Vec::new();
    for (.., name) in ranges.iter().filter(|(a, b, _)| *a <= end && net <= *b) {
        if !part.contains(name) {
            part.push(name);
        }
    }
    match (part.len(), v6) {
        (0, false) => "Public".into(),
        (0, true) => "Reserved".into(),
        (1..=3, _) => format!("Mixed: includes {}", part.join(", ")),
        _ => "Mixed: public and special-use ranges".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(s: &str) -> Block {
        match parse(s).unwrap() {
            Input::Block(b) => b,
            r => panic!("not a block: {r:?}"),
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn v4_block() {
        let i = block("10.0.1.77/22").info();
        assert_eq!(i.network, ip("10.0.0.0"));
        assert_eq!(i.last, ip("10.0.3.255"));
        assert_eq!(i.mask, ip("255.255.252.0"));
        assert_eq!(i.wildcard, ip("0.0.3.255"));
        assert_eq!(i.first_host, ip("10.0.0.1"));
        assert_eq!(i.last_host, ip("10.0.3.254"));
        assert_eq!(i.usable, Some(1022));
        assert_eq!(i.kind, "Private (RFC 1918)");
        assert_eq!(i.prev, Some(ip("9.255.252.0")));
        assert_eq!(i.next, Some(ip("10.0.4.0")));
        assert_eq!(
            binary(i.mask).unwrap(),
            "11111111.11111111.11111100.00000000"
        );
    }

    #[test]
    fn masks_and_wildcards() {
        assert_eq!(block("192.168.1.9 255.255.255.0").prefix, 24);
        assert_eq!(block("192.168.1.9/255.255.255.128").prefix, 25);
        assert_eq!(block("10.0.0.0 0.0.3.255").prefix, 22);
        assert_eq!(block("10.0.0.0 0.0.0.0").prefix, 0);
        assert_eq!(block("10.0.0.0 255.255.255.255").prefix, 32);
        assert!(parse("10.0.0.0 255.0.255.0").is_err());
        assert!(parse("10.0.0.0/33").is_err());
        assert!(parse("10.0.0.1").is_err());
        assert!(parse("2001:db8:: 255.255.0.0").is_err());
        assert!(parse("nonsense/24").is_err());
    }

    #[test]
    fn edges() {
        let i = block("10.0.0.4/31").info();
        assert_eq!(
            (i.first_host, i.last_host),
            (ip("10.0.0.4"), ip("10.0.0.5"))
        );
        assert_eq!(i.usable, Some(2));
        let i = block("10.0.0.4/32").info();
        assert_eq!(i.usable, Some(1));
        assert_eq!(i.first_host, ip("10.0.0.4"));
        let i = block("0.0.0.0/0").info();
        assert_eq!(i.last, ip("255.255.255.255"));
        assert_eq!(i.usable, Some((1u128 << 32) - 2));
        assert_eq!((i.prev, i.next), (None, None));
        assert!(i.kind.starts_with("Mixed"));
        let i = block("255.255.255.0/24").info();
        assert_eq!(i.next, None);
        assert_eq!(block("8.8.8.0/24").info().kind, "Public");
        assert_eq!(
            block("100.100.0.0/16").info().kind,
            "Shared / carrier-grade NAT (RFC 6598)"
        );
    }

    #[test]
    fn v6_block() {
        let i = block("2001:db8:abcd:12::1/64").info();
        assert_eq!(i.network, ip("2001:db8:abcd:12::"));
        assert_eq!(i.last, ip("2001:db8:abcd:12:ffff:ffff:ffff:ffff"));
        assert_eq!(i.mask, ip("ffff:ffff:ffff:ffff::"));
        assert_eq!(i.usable, Some(1u128 << 64));
        assert_eq!(count(i.host_bits, i.usable), "18,446,744,073,709,551,616");
        assert_eq!(i.kind, "Documentation");
        assert_eq!(i.next, Some(ip("2001:db8:abcd:13::")));
        let i = block("::/0").info();
        assert_eq!(i.usable, None);
        assert_eq!(count(i.host_bits, i.usable), "2^128");
        assert_eq!(i.next, None);
        assert_eq!(
            block("fd12:3456::/48").info().kind,
            "Unique local (ULA, private)"
        );
        assert_eq!(block("::1/128").info().kind, "Loopback");
    }

    #[test]
    fn split() {
        let b = block("10.0.0.0/22");
        assert_eq!(b.split_bits(24), 2);
        assert_eq!(b.sub(24, 0).cidr(), "10.0.0.0/24");
        assert_eq!(b.sub(24, 3).cidr(), "10.0.3.0/24");
        let b = block("2001:db8::/48");
        assert_eq!(b.sub(64, 0xff).cidr(), "2001:db8:0:ff::/64");
    }

    #[test]
    fn contains() {
        let b = block("10.0.0.0/22");
        assert_eq!(b.offset(ip("10.0.3.7")), Some(775));
        assert_eq!(b.offset(ip("10.0.4.0")), None);
        assert_eq!(b.offset(ip("::1")), None);
    }

    #[test]
    fn ranges() {
        let Input::Range(a, b) = parse("10.0.0.1-10.0.0.10").unwrap() else {
            panic!()
        };
        let c: Vec<String> = cover(a, b).iter().map(Block::cidr).collect();
        assert_eq!(
            c,
            [
                "10.0.0.1/32",
                "10.0.0.2/31",
                "10.0.0.4/30",
                "10.0.0.8/31",
                "10.0.0.10/32"
            ]
        );
        assert_eq!(range_len(a, b), "10");
        assert_eq!(
            parse("10.0.0.1-99").unwrap(),
            Input::Range(a, ip("10.0.0.99"))
        );
        let full = cover(ip("0.0.0.0"), ip("255.255.255.255"));
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].cidr(), "0.0.0.0/0");
        let top = cover(ip("255.255.255.254"), ip("255.255.255.255"));
        assert_eq!(top[0].cidr(), "255.255.255.254/31");
        let all6 = cover(ip("::"), ip("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"));
        assert_eq!(all6[0].cidr(), "::/0");
        assert!(parse("10.0.0.9-10.0.0.1").is_err());
        assert!(parse("10.0.0.1-::1").is_err());
    }

    #[test]
    fn grouping() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1000), "1,000");
        assert_eq!(group(1234567), "1,234,567");
    }
}
