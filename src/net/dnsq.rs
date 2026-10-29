//! Minimal DNS client that asks a chosen server directly (UDP, TCP on
//! truncation) instead of going through the system resolver, so answers show
//! what the internet sees rather than hosts-file or split-DNS overrides.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

pub const A: u16 = 1;
pub const NS: u16 = 2;
pub const CNAME: u16 = 5;
pub const SOA: u16 = 6;
pub const PTR: u16 = 12;
pub const MX: u16 = 15;
pub const TXT: u16 = 16;
pub const AAAA: u16 = 28;
pub const SRV: u16 = 33;
const OPT: u16 = 41;
pub const DS: u16 = 43;
pub const DNSKEY: u16 = 48;
pub const CAA: u16 = 257;

pub const NOERROR: u8 = 0;
pub const SERVFAIL: u8 = 2;
pub const NXDOMAIN: u8 = 3;

#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    A(Ipv4Addr),
    Aaaa(Ipv6Addr),
    /// NS, CNAME and PTR targets.
    Name(String),
    Mx(u16, String),
    /// Character-strings joined, as SPF/DMARC/DKIM readers expect.
    Txt(String),
    Soa {
        mname: String,
        serial: u32,
    },
    Caa {
        flags: u8,
        tag: String,
        value: String,
    },
    Ds {
        key_tag: u16,
        alg: u8,
        digest_type: u8,
    },
    Dnskey {
        flags: u16,
        alg: u8,
    },
    Srv {
        prio: u16,
        weight: u16,
        port: u16,
        target: String,
    },
    Other,
}

#[derive(Clone, Debug)]
pub struct Rr {
    pub name: String,
    pub rtype: u16,
    pub data: Data,
}

#[derive(Clone, Debug, Default)]
pub struct Msg {
    pub rcode: u8,
    /// Authoritative answer.
    pub aa: bool,
    /// Resolver validated the answer with DNSSEC.
    pub ad: bool,
    pub answers: Vec<Rr>,
    pub authority: Vec<Rr>,
}

impl Msg {
    pub fn of(&self, rtype: u16) -> impl Iterator<Item = &Data> {
        self.answers
            .iter()
            .filter(move |r| r.rtype == rtype)
            .map(|r| &r.data)
    }

    pub fn txt(&self) -> Vec<String> {
        self.of(TXT)
            .filter_map(|d| match d {
                Data::Txt(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    /// NS, CNAME or PTR targets of the given type.
    pub fn names(&self, rtype: u16) -> Vec<String> {
        self.of(rtype)
            .filter_map(|d| match d {
                Data::Name(n) => Some(n.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn ips(&self) -> Vec<IpAddr> {
        self.answers
            .iter()
            .filter_map(|r| match r.data {
                Data::A(ip) => Some(IpAddr::V4(ip)),
                Data::Aaaa(ip) => Some(IpAddr::V6(ip)),
                _ => None,
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct Client {
    pub server: SocketAddr,
    pub timeout: Duration,
    /// Recursion desired. Off when asking an authoritative server directly.
    pub rd: bool,
}

impl Client {
    pub fn new(server: SocketAddr) -> Self {
        Self {
            server,
            timeout: Duration::from_secs(2),
            rd: true,
        }
    }

    /// Non-recursive client for asking an authoritative server.
    pub fn direct(ip: IpAddr) -> Self {
        Self {
            server: SocketAddr::new(ip, 53),
            timeout: Duration::from_secs(3),
            rd: false,
        }
    }

    /// Query; SERVFAIL and other server errors (but not NXDOMAIN) are errors.
    pub fn ask(&self, name: &str, rtype: u16) -> io::Result<Msg> {
        let m = self.query(name, rtype, false)?;
        match m.rcode {
            NOERROR | NXDOMAIN => Ok(m),
            SERVFAIL => Err(io::Error::other(format!("SERVFAIL for {name}"))),
            rc => Err(io::Error::other(format!("rcode {rc} for {name}"))),
        }
    }

    /// Raw query. `cd` (checking disabled) asks a validating resolver to skip
    /// DNSSEC validation, which tells bogus signatures apart from outages.
    pub fn query(&self, name: &str, rtype: u16, cd: bool) -> io::Result<Msg> {
        let id = random_id();
        let q = encode_query(id, name, rtype, self.rd, cd)?;
        let r = match self.udp(&q, id) {
            Ok(r) => r,
            // One retry: a single lost datagram shouldn't fail a check.
            Err(e) if e.kind() == io::ErrorKind::TimedOut => self.udp(&q, id)?,
            Err(e) => return Err(e),
        };
        let m = if r.len() > 2 && r[2] & 0x02 != 0 {
            self.tcp(&q, id)?
        } else {
            r
        };
        parse(&m, id)
    }

    fn udp(&self, q: &[u8], id: u16) -> io::Result<Vec<u8>> {
        let bind = if self.server.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let sock = UdpSocket::bind(bind)?;
        sock.connect(self.server)?;
        sock.send(q)?;
        let end = Instant::now() + self.timeout;
        let mut buf = vec![0u8; 4096];
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(timed_out(self.server));
            }
            sock.set_read_timeout(Some(left))?;
            let n = match sock.recv(&mut buf) {
                Ok(n) => n,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(timed_out(self.server));
                }
                Err(e) => return Err(e),
            };
            // Ignore stray datagrams that aren't our answer.
            if n >= 12 && buf[..2] == id.to_be_bytes() {
                buf.truncate(n);
                return Ok(buf);
            }
        }
    }

    fn tcp(&self, q: &[u8], id: u16) -> io::Result<Vec<u8>> {
        let mut s = TcpStream::connect_timeout(&self.server, self.timeout)?;
        s.set_read_timeout(Some(self.timeout))?;
        s.set_write_timeout(Some(self.timeout))?;
        let mut framed = (q.len() as u16).to_be_bytes().to_vec();
        framed.extend_from_slice(q);
        s.write_all(&framed)?;
        let mut len = [0u8; 2];
        s.read_exact(&mut len)?;
        let mut buf = vec![0u8; u16::from_be_bytes(len) as usize];
        s.read_exact(&mut buf)?;
        if buf.len() < 12 || buf[..2] != id.to_be_bytes() {
            return Err(io::Error::other("mismatched TCP answer"));
        }
        Ok(buf)
    }
}

fn timed_out(server: SocketAddr) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("no answer from {}", server.ip()),
    )
}

fn random_id() -> u16 {
    let mut h = RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.finish() as u16
}

/// Query with EDNS0 (4096-byte UDP, DO bit so DNSSEC records come back).
fn encode_query(id: u16, name: &str, rtype: u16, rd: bool, cd: bool) -> io::Result<Vec<u8>> {
    let mut q = id.to_be_bytes().to_vec();
    q.push(if rd { 0x01 } else { 0x00 });
    q.push(if cd { 0x10 } else { 0x00 });
    q.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 1]); // 1 question, 1 additional (OPT)
    encode_name(&mut q, name)?;
    q.extend_from_slice(&rtype.to_be_bytes());
    q.extend_from_slice(&[0, 1]); // class IN
    // OPT: root name, type, UDP size, ext-rcode/version, DO flag, no options.
    q.push(0);
    q.extend_from_slice(&OPT.to_be_bytes());
    q.extend_from_slice(&4096u16.to_be_bytes());
    q.extend_from_slice(&[0, 0, 0x80, 0, 0, 0]);
    Ok(q)
}

fn encode_name(q: &mut Vec<u8>, name: &str) -> io::Result<()> {
    let name = name.trim_end_matches('.');
    if name.len() > 253 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "name too long"));
    }
    for label in name.split('.').filter(|l| !l.is_empty()) {
        if label.len() > 63 || !label.is_ascii() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid label '{label}'"),
            ));
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    Ok(())
}

fn bad() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "malformed DNS answer")
}

fn u16_at(r: &[u8], i: usize) -> io::Result<u16> {
    r.get(i..i + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or_else(bad)
}

/// Read a possibly compressed name at `i`; returns it and the offset after it.
fn read_name(r: &[u8], mut i: usize) -> io::Result<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut end = None;
    for _ in 0..128 {
        let len = *r.get(i).ok_or_else(bad)? as usize;
        if len & 0xc0 == 0xc0 {
            end.get_or_insert(i + 2);
            i = (u16_at(r, i)? & 0x3fff) as usize;
            continue;
        }
        if len == 0 {
            let name = labels.join(".").to_ascii_lowercase();
            return Ok((name, end.unwrap_or(i + 1)));
        }
        let l = r.get(i + 1..i + 1 + len).ok_or_else(bad)?;
        labels.push(super::sanitize(&String::from_utf8_lossy(l)));
        i += 1 + len;
    }
    Err(bad()) // pointer loop
}

fn parse(r: &[u8], id: u16) -> io::Result<Msg> {
    if r.len() < 12 || r[..2] != id.to_be_bytes() {
        return Err(bad());
    }
    let mut m = Msg {
        rcode: r[3] & 0x0f,
        aa: r[2] & 0x04 != 0,
        ad: r[3] & 0x20 != 0,
        ..Default::default()
    };
    let (qd, an, ns) = (u16_at(r, 4)?, u16_at(r, 6)?, u16_at(r, 8)?);
    let mut i = 12;
    for _ in 0..qd {
        i = read_name(r, i)?.1 + 4;
    }
    for n in 0..an as usize + ns as usize {
        let (name, j) = read_name(r, i)?;
        let rtype = u16_at(r, j)?;
        let len = u16_at(r, j + 8)? as usize;
        let start = j + 10;
        let rd = r.get(start..start + len).ok_or_else(bad)?;
        let data = parse_data(r, start, rd, rtype).unwrap_or(Data::Other);
        let rr = Rr { name, rtype, data };
        if n < an as usize {
            m.answers.push(rr);
        } else {
            m.authority.push(rr);
        }
        i = start + len;
    }
    Ok(m)
}

fn parse_data(r: &[u8], start: usize, d: &[u8], rtype: u16) -> io::Result<Data> {
    Ok(match rtype {
        A if d.len() == 4 => Data::A(Ipv4Addr::new(d[0], d[1], d[2], d[3])),
        AAAA if d.len() == 16 => {
            Data::Aaaa(Ipv6Addr::from(<[u8; 16]>::try_from(d).map_err(|_| bad())?))
        }
        NS | CNAME | PTR => Data::Name(read_name(r, start)?.0),
        MX => Data::Mx(u16_at(d, 0)?, read_name(r, start + 2)?.0),
        TXT => {
            let mut s = Vec::new();
            let mut k = 0;
            while k < d.len() {
                let n = d[k] as usize;
                s.extend_from_slice(d.get(k + 1..k + 1 + n).ok_or_else(bad)?);
                k += 1 + n;
            }
            Data::Txt(super::sanitize_long(&String::from_utf8_lossy(&s)))
        }
        SOA => {
            let (mname, j) = read_name(r, start)?;
            let j = read_name(r, j)?.1; // rname
            let b = r.get(j..j + 4).ok_or_else(bad)?;
            Data::Soa {
                mname,
                serial: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            }
        }
        CAA => {
            let tl = *d.get(1).ok_or_else(bad)? as usize;
            let tag = d.get(2..2 + tl).ok_or_else(bad)?;
            Data::Caa {
                flags: d[0],
                tag: String::from_utf8_lossy(tag).to_ascii_lowercase(),
                value: super::sanitize(&String::from_utf8_lossy(&d[2 + tl..])),
            }
        }
        DS if d.len() >= 4 => Data::Ds {
            key_tag: u16_at(d, 0)?,
            alg: d[2],
            digest_type: d[3],
        },
        DNSKEY if d.len() >= 4 => Data::Dnskey {
            flags: u16_at(d, 0)?,
            alg: d[3],
        },
        SRV => Data::Srv {
            prio: u16_at(d, 0)?,
            weight: u16_at(d, 2)?,
            port: u16_at(d, 4)?,
            target: read_name(r, start + 6)?.0,
        },
        _ => Data::Other,
    })
}

/// Name to query for a PTR record of `ip`.
pub fn reverse_name(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0])
        }
        IpAddr::V6(v6) => {
            let mut s = String::new();
            for b in v6.octets().iter().rev() {
                s += &format!("{:x}.{:x}.", b & 0x0f, b >> 4);
            }
            s + "ip6.arpa"
        }
    }
}

impl std::fmt::Display for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Data::A(ip) => write!(f, "{ip}"),
            Data::Aaaa(ip) => write!(f, "{ip}"),
            Data::Name(n) => write!(f, "{n}"),
            Data::Mx(pref, host) => write!(f, "{pref} {host}"),
            Data::Txt(t) => write!(f, "\"{t}\""),
            Data::Soa { mname, serial } => write!(f, "{mname}  serial {serial}"),
            Data::Caa { flags, tag, value } => write!(f, "{flags} {tag} \"{value}\""),
            Data::Ds {
                key_tag,
                alg,
                digest_type,
            } => write!(f, "tag {key_tag}  alg {alg}  digest {digest_type}"),
            Data::Dnskey { flags, alg } => {
                let role = if flags & 1 == 1 { "KSK" } else { "ZSK" };
                write!(f, "{role}  alg {alg}")
            }
            Data::Srv {
                prio,
                weight,
                port,
                target,
            } => write!(f, "{prio} {weight} {port} {target}"),
            Data::Other => write!(f, "(unparsed)"),
        }
    }
}

/// Record types offered in the DNS Lookup tab, by name.
pub const TYPES: &[(&str, u16)] = &[
    ("A", A),
    ("AAAA", AAAA),
    ("MX", MX),
    ("TXT", TXT),
    ("CNAME", CNAME),
    ("NS", NS),
    ("SOA", SOA),
    ("SRV", SRV),
    ("CAA", CAA),
    ("PTR", PTR),
    ("DS", DS),
    ("DNSKEY", DNSKEY),
];

pub fn type_name(rtype: u16) -> String {
    TYPES
        .iter()
        .find(|t| t.1 == rtype)
        .map_or_else(|| format!("TYPE{rtype}"), |t| t.0.to_string())
}

/// The DNS servers this machine is configured to use.
pub fn system_servers() -> Vec<IpAddr> {
    let mut v = imp_servers();
    v.dedup();
    v
}

#[cfg(unix)]
fn imp_servers() -> Vec<IpAddr> {
    std::fs::read_to_string("/etc/resolv.conf")
        .map(|t| parse_resolv_conf(&t))
        .unwrap_or_default()
}

#[cfg(any(unix, test))]
fn parse_resolv_conf(t: &str) -> Vec<IpAddr> {
    t.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            (w.next()? == "nameserver").then(|| w.next()?.split('%').next()?.parse().ok())?
        })
        .collect()
}

#[cfg(windows)]
fn imp_servers() -> Vec<IpAddr> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{FIXED_INFO_W2KSP1, GetNetworkParams};
    // First call reports the size needed (ERROR_BUFFER_OVERFLOW).
    let mut len = 0u32;
    unsafe { GetNetworkParams(std::ptr::null_mut(), &mut len) };
    let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
    let info = buf.as_mut_ptr().cast::<FIXED_INFO_W2KSP1>();
    if unsafe { GetNetworkParams(info, &mut len) } != 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut node = unsafe { std::ptr::addr_of!((*info).DnsServerList) };
    while !node.is_null() {
        let raw = unsafe { (*node).IpAddress.String };
        let text: Vec<u8> = raw
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as u8)
            .collect();
        if let Ok(ip) = String::from_utf8_lossy(&text).trim().parse() {
            out.push(ip);
        }
        node = unsafe { (*node).Next };
    }
    out
}

#[cfg(not(any(unix, windows)))]
fn imp_servers() -> Vec<IpAddr> {
    Vec::new()
}

/// Parse "1.1.1.1", "1.1.1.1:53", "2606:4700::1111" or "[2606:4700::1111]:53".
pub fn parse_server(s: &str) -> Option<SocketAddr> {
    let s = s.trim();
    s.parse::<SocketAddr>().ok().or_else(|| {
        s.trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .ok()
            .map(|ip| SocketAddr::new(ip, 53))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(q: &[u8], rrs: &[u8], an: u16) -> Vec<u8> {
        // Drop the OPT record from the query and turn it into a response.
        let qlen = q.len() - 11;
        let mut r = q[..qlen].to_vec();
        r[2] |= 0x80;
        r[3] = 0x20; // AD
        r[6..8].copy_from_slice(&an.to_be_bytes());
        r[10..12].copy_from_slice(&[0, 0]);
        r.extend_from_slice(rrs);
        r
    }

    #[test]
    fn parses_mx_txt_and_compression() {
        let q = encode_query(9, "example.com", MX, true, false).unwrap();
        let mut rrs = vec![0xc0, 12, 0, 15, 0, 1, 0, 0, 0, 60, 0, 9, 0, 10, 4];
        rrs.extend_from_slice(b"mail");
        rrs.extend_from_slice(&[0xc0, 12]);
        rrs.extend_from_slice(&[0xc0, 12, 0, 16, 0, 1, 0, 0, 0, 60, 0, 8, 3]);
        rrs.extend_from_slice(b"v=s");
        rrs.push(3);
        rrs.extend_from_slice(b"pf1");
        let m = parse(&answer(&q, &rrs, 2), 9).unwrap();
        assert!(m.ad);
        assert_eq!(m.answers[0].data, Data::Mx(10, "mail.example.com".into()));
        assert_eq!(m.txt(), vec!["v=spf1".to_string()]);
        assert!(parse(&answer(&q, &rrs, 2), 10).is_err());
    }

    #[test]
    fn rejects_pointer_loops() {
        let q = encode_query(1, "a.b", A, true, false).unwrap();
        let rrs = [0xc0, 29, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0];
        let mut r = answer(&q, &rrs, 1);
        let at = r.len() - rrs.len();
        r[at + 1] = at as u8; // points at itself
        assert!(parse(&r, 1).is_err());
    }

    #[test]
    fn reverse_names() {
        assert_eq!(
            reverse_name("192.0.2.5".parse().unwrap()),
            "5.2.0.192.in-addr.arpa"
        );
        assert!(
            reverse_name("2001:db8::1".parse().unwrap())
                .starts_with("1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.8.b.d.0.1.0.0.2")
        );
    }

    #[test]
    fn resolv_conf() {
        let t = "# comment\nnameserver 127.0.0.53\nsearch lan\nnameserver fe80::1%eth0\n";
        assert_eq!(
            parse_resolv_conf(t),
            vec![
                "127.0.0.53".parse::<IpAddr>().unwrap(),
                "fe80::1".parse().unwrap()
            ]
        );
    }

    #[test]
    fn displays_records() {
        assert_eq!(
            Data::Mx(10, "mx.example".into()).to_string(),
            "10 mx.example"
        );
        assert_eq!(type_name(MX), "MX");
        assert_eq!(type_name(99), "TYPE99");
    }

    #[test]
    fn server_forms() {
        assert_eq!(parse_server("1.1.1.1").unwrap().port(), 53);
        assert_eq!(parse_server("9.9.9.9:5353").unwrap().port(), 5353);
        assert!(parse_server("[2606:4700::1111]").is_some());
        assert!(parse_server("dns.example").is_none());
    }
}
