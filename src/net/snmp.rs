//! SNMP v1/v2c, read-only, for one device the technician picks (the
//! inspector's SNMP button): what it is (system group, ENTITY-MIB's
//! chassis), a printer's state and supplies, and which switch port it's
//! plugged into (LLDP). Never part of a sweep or Full scan, never sent
//! across a subnet: one address, UDP 161, when asked.
//!
//! A small BER encoder and decoder, no SNMP crate. Replies come from a
//! device on a client's network, so the decoder takes any bytes without
//! panicking: every length is checked against what's left, long-form
//! lengths are capped at 4 bytes, and it never recurses: SNMP's message
//! is a fixed shape (message, PDU, varbind list, varbind, value), parsed
//! level by level, and a constructed value inside a varbind is kept as an
//! opaque tag, not descended into. That fixed shape is its depth limit.

use std::io;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The only port an answer is taken from.
pub const PORT: u16 = 161;
/// How long one request waits for its answer.
pub const WAIT: Duration = Duration::from_secs(2);
/// One try and one retry per request.
const TRIES: usize = 2;
/// The whole read, every walk included.
pub const WHOLE: Duration = Duration::from_secs(30);
/// More varbinds than this in one reply isn't an answer to what was asked.
const MAX_BINDS: usize = 64;
/// Sub-identifiers in one OID.
const MAX_ARCS: usize = 128;
/// Rows a walk of LLDP's neighbour table or ENTITY-MIB reads at most.
pub const TABLE_ROWS: usize = 64;
/// Supplies a printer's walk reads at most.
const SUPPLY_ROWS: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Version {
    V1,
    V2c,
}

impl Version {
    fn wire(self) -> i64 {
        match self {
            Version::V1 => 0,
            Version::V2c => 1,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Version::V1 => "v1",
            Version::V2c => "v2c",
        }
    }
}

pub type Oid = Vec<u32>;

/// A varbind's value as it came. Counter32, Gauge32 and Counter64 are all
/// `Count`; anything else unknown keeps its tag.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Bytes(Vec<u8>),
    Null,
    Oid(Oid),
    Ip([u8; 4]),
    Count(u64),
    Ticks(u32),
    NoSuchObject,
    NoSuchInstance,
    EndOfMib,
    Other(u8),
}

impl Value {
    /// Not a value: v2c's exceptions, a NULL.
    fn missing(&self) -> bool {
        matches!(
            self,
            Value::Null | Value::NoSuchObject | Value::NoSuchInstance | Value::EndOfMib
        )
    }

    fn int(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Count(v) => i64::try_from(*v).ok(),
            Value::Ticks(v) => Some(*v as i64),
            _ => None,
        }
    }

    /// As text for a person: a string when it is one, else hex.
    pub fn text(&self) -> Option<String> {
        let s = match self {
            Value::Bytes(b) => text(b),
            Value::Int(v) => v.to_string(),
            Value::Count(v) => v.to_string(),
            Value::Ticks(v) => uptime(*v),
            Value::Oid(o) => dotted(o),
            Value::Ip(a) => std::net::Ipv4Addr::from(*a).to_string(),
            _ => return None,
        };
        (!s.is_empty()).then_some(s)
    }
}

/// Octets as text: a string when they're printable UTF-8 (trailing NULs,
/// which printers like, dropped), else hex pairs.
fn text(b: &[u8]) -> String {
    let b = &b[..b.len() - b.iter().rev().take_while(|&&c| c == 0).count()];
    match std::str::from_utf8(b) {
        Ok(s)
            if !s
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t')) =>
        {
            let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
            // Control and bidi characters dropped, 255 characters at most.
            crate::net::sanitize(&one)
        }
        _ => b
            .iter()
            .take(64)
            .map(|c| format!("{c:02x}"))
            .collect::<Vec<_>>()
            .join(":"),
    }
}

pub fn dotted(o: &[u32]) -> String {
    o.iter().map(u32::to_string).collect::<Vec<_>>().join(".")
}

/// sysUpTime (hundredths of a second) as "12 d 3 h", "3 h 4 min", "45 s".
pub fn uptime(ticks: u32) -> String {
    let s = ticks / 100;
    let (d, h, m) = (s / 86_400, s % 86_400 / 3600, s % 3600 / 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{s} s"),
        (0, 0, m) => format!("{m} min"),
        (0, h, m) => format!("{h} h {m} min"),
        (d, h, _) => format!("{d} d {h} h"),
    }
}

// ------------------------------------------------------------------ BER

const SEQ: u8 = 0x30;
const INT: u8 = 0x02;
const OCTETS: u8 = 0x04;
const NULL: u8 = 0x05;
const OID: u8 = 0x06;
pub const GET: u8 = 0xA0;
pub const GET_NEXT: u8 = 0xA1;
pub const RESPONSE: u8 = 0xA2;

fn put_len(out: &mut Vec<u8>, n: usize) {
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let b = (n as u64).to_be_bytes();
        let skip = b.iter().take_while(|&&x| x == 0).count();
        out.push(0x80 | (b.len() - skip) as u8);
        out.extend_from_slice(&b[skip..]);
    }
}

fn put(out: &mut Vec<u8>, tag: u8, content: &[u8]) {
    out.push(tag);
    put_len(out, content.len());
    out.extend_from_slice(content);
}

/// Two's complement in as few bytes as hold it.
fn int_bytes(v: i64) -> Vec<u8> {
    let b = v.to_be_bytes();
    let mut i = 0;
    while i < 7 && ((b[i] == 0 && b[i + 1] & 0x80 == 0) || (b[i] == 0xff && b[i + 1] & 0x80 != 0)) {
        i += 1;
    }
    b[i..].to_vec()
}

fn uint_bytes(v: u64) -> Vec<u8> {
    let mut b = v.to_be_bytes().to_vec();
    while b.len() > 1 && b[0] == 0 && b[1] & 0x80 == 0 {
        b.remove(0);
    }
    if b[0] & 0x80 != 0 {
        b.insert(0, 0);
    }
    b
}

fn base128(out: &mut Vec<u8>, v: u64) {
    let mut groups = vec![(v & 0x7f) as u8];
    let mut v = v >> 7;
    while v > 0 {
        groups.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.extend(groups.iter().rev());
}

fn oid_bytes(o: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    match o {
        [] => out.push(0),
        [a] => base128(&mut out, *a as u64 * 40),
        [a, b, rest @ ..] => {
            base128(&mut out, *a as u64 * 40 + *b as u64);
            for s in rest {
                base128(&mut out, *s as u64);
            }
        }
    }
    out
}

fn put_value(out: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Int(i) => put(out, INT, &int_bytes(*i)),
        Value::Bytes(b) => put(out, OCTETS, b),
        Value::Null => put(out, NULL, &[]),
        Value::Oid(o) => put(out, OID, &oid_bytes(o)),
        Value::Ip(a) => put(out, 0x40, a),
        Value::Count(c) => put(out, 0x46, &uint_bytes(*c)),
        Value::Ticks(t) => put(out, 0x43, &uint_bytes(*t as u64)),
        Value::NoSuchObject => put(out, 0x80, &[]),
        Value::NoSuchInstance => put(out, 0x81, &[]),
        Value::EndOfMib => put(out, 0x82, &[]),
        Value::Other(t) => put(out, *t, &[]),
    }
}

/// One SNMP message.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub version: i64,
    pub community: Vec<u8>,
    pub pdu: u8,
    pub id: i64,
    pub status: i64,
    pub index: i64,
    pub binds: Vec<(Oid, Value)>,
}

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut vbs = Vec::new();
        for (o, v) in &self.binds {
            let mut vb = Vec::new();
            put(&mut vb, OID, &oid_bytes(o));
            put_value(&mut vb, v);
            put(&mut vbs, SEQ, &vb);
        }
        let mut pdu = Vec::new();
        put(&mut pdu, INT, &int_bytes(self.id));
        put(&mut pdu, INT, &int_bytes(self.status));
        put(&mut pdu, INT, &int_bytes(self.index));
        put(&mut pdu, SEQ, &vbs);
        let mut msg = Vec::new();
        put(&mut msg, INT, &int_bytes(self.version));
        put(&mut msg, OCTETS, &self.community);
        put(&mut msg, self.pdu, &pdu);
        let mut out = Vec::new();
        put(&mut out, SEQ, &msg);
        out
    }

    /// A message from the wire, or None for anything that isn't one.
    /// Never panics.
    pub fn decode(b: &[u8]) -> Option<Message> {
        let mut top = Reader(b);
        let mut m = Reader(top.expect(SEQ)?);
        let version = int(m.expect(INT)?)?;
        let community = m.expect(OCTETS)?.to_vec();
        let (pdu, body) = m.next()?;
        if !(0xA0..=0xA8).contains(&pdu) {
            return None;
        }
        let mut p = Reader(body);
        let id = int(p.expect(INT)?)?;
        let status = int(p.expect(INT)?)?;
        let index = int(p.expect(INT)?)?;
        let mut list = Reader(p.expect(SEQ)?);
        let mut binds = Vec::new();
        while !list.0.is_empty() {
            if binds.len() >= MAX_BINDS {
                return None;
            }
            let mut vb = Reader(list.expect(SEQ)?);
            let oid = parse_oid(vb.expect(OID)?)?;
            let (tag, c) = vb.next()?;
            binds.push((oid, value(tag, c)));
        }
        Some(Message {
            version,
            community,
            pdu,
            id,
            status,
            index,
            binds,
        })
    }
}

/// What's left to read; each `next` takes one tag-length-value off it.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn next(&mut self) -> Option<(u8, &'a [u8])> {
        let (&tag, rest) = self.0.split_first()?;
        // High tag numbers aren't SNMP.
        if tag & 0x1f == 0x1f {
            return None;
        }
        let (&first, mut rest) = rest.split_first()?;
        let len = if first < 0x80 {
            first as usize
        } else {
            // Indefinite (0x80) isn't allowed in SNMP; more than 4 length
            // bytes is more than any datagram.
            let n = (first & 0x7f) as usize;
            if n == 0 || n > 4 || rest.len() < n {
                return None;
            }
            let len = rest[..n].iter().fold(0usize, |a, &x| (a << 8) | x as usize);
            rest = &rest[n..];
            len
        };
        if len > rest.len() {
            return None;
        }
        let (c, r) = rest.split_at(len);
        self.0 = r;
        Some((tag, c))
    }

    fn expect(&mut self, tag: u8) -> Option<&'a [u8]> {
        self.next().filter(|(t, _)| *t == tag).map(|(_, c)| c)
    }
}

/// A signed INTEGER of 1 to 8 bytes.
fn int(c: &[u8]) -> Option<i64> {
    if c.is_empty() || c.len() > 8 {
        return None;
    }
    let start = if c[0] & 0x80 != 0 { -1i64 } else { 0 };
    Some(c.iter().fold(start, |a, &x| (a << 8) | x as i64))
}

/// An unsigned one (Counter64 can take 9 bytes, the first a 0).
fn uint(c: &[u8]) -> Option<u64> {
    let c = &c[c.iter().take_while(|&&x| x == 0).count().min(c.len())..];
    if c.len() > 8 {
        return None;
    }
    Some(c.iter().fold(0u64, |a, &x| (a << 8) | x as u64))
}

fn parse_oid(c: &[u8]) -> Option<Oid> {
    let mut arcs = Vec::new();
    let mut v: u64 = 0;
    let mut open = false;
    for &b in c {
        v = (v << 7) | (b & 0x7f) as u64;
        if v > u32::MAX as u64 + 80 {
            return None;
        }
        open = b & 0x80 != 0;
        if !open {
            if arcs.is_empty() {
                let (a, b) = match v {
                    0..40 => (0, v),
                    40..80 => (1, v - 40),
                    _ => (2, v - 80),
                };
                arcs.push(a as u32);
                arcs.push(u32::try_from(b).ok()?);
            } else {
                arcs.push(u32::try_from(v).ok()?);
            }
            if arcs.len() > MAX_ARCS {
                return None;
            }
            v = 0;
        }
    }
    (!open && !arcs.is_empty()).then_some(arcs)
}

fn value(tag: u8, c: &[u8]) -> Value {
    let v = match tag {
        INT => int(c).map(Value::Int),
        OCTETS | 0x44 => Some(Value::Bytes(c.to_vec())),
        NULL => Some(Value::Null),
        OID => parse_oid(c).map(Value::Oid),
        0x40 => <[u8; 4]>::try_from(c).ok().map(Value::Ip),
        0x41 | 0x42 | 0x46 | 0x47 => uint(c).map(Value::Count),
        0x43 => uint(c).map(|t| Value::Ticks(t.min(u32::MAX as u64) as u32)),
        0x80 => Some(Value::NoSuchObject),
        0x81 => Some(Value::NoSuchInstance),
        0x82 => Some(Value::EndOfMib),
        _ => None,
    };
    v.unwrap_or(Value::Other(tag))
}

// --------------------------------------------------------------- client

/// Why an exchange gave nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    /// No answer from the address and port asked, after the retry.
    Silent,
    /// The device said nothing listens on its UDP 161 (ICMP port unreachable).
    Refused,
    /// The read's own time limit.
    Late,
    Stopped,
    Io(String),
}

/// One device's agent: requests go to `to` only, and only its answers to
/// them, by request ID, from that address and port, are taken.
pub struct Agent<'a> {
    sock: UdpSocket,
    to: SocketAddr,
    version: Version,
    community: &'a [u8],
    id: i64,
    deadline: Instant,
    cancel: &'a AtomicBool,
}

impl<'a> Agent<'a> {
    pub fn new(
        to: SocketAddr,
        version: Version,
        community: &'a [u8],
        deadline: Instant,
        cancel: &'a AtomicBool,
    ) -> io::Result<Self> {
        let any: SocketAddr = match to {
            SocketAddr::V4(_) => (std::net::Ipv4Addr::UNSPECIFIED, 0).into(),
            SocketAddr::V6(_) => (std::net::Ipv6Addr::UNSPECIFIED, 0).into(),
        };
        let sock = UdpSocket::bind(any)?;
        // Connected: the system drops datagrams from anywhere else, and
        // reports a port unreachable as a refusal.
        sock.connect(to)?;
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.subsec_nanos() as i64);
        Ok(Self {
            sock,
            to,
            version,
            community,
            id: (seed & 0x3fff_ffff).max(1),
            deadline,
            cancel,
        })
    }

    fn exchange(&mut self, pdu: u8, oids: &[Oid]) -> Result<Message, Fail> {
        let mut buf = vec![0u8; 65_536];
        // Every try's ID: a late answer to the first counts while the
        // retry waits.
        let mut sent = Vec::with_capacity(TRIES);
        for _ in 0..TRIES {
            // Nothing is sent once the read's time is up.
            if Instant::now() >= self.deadline {
                return Err(Fail::Late);
            }
            self.id = if self.id >= 0x7fff_ffff {
                1
            } else {
                self.id + 1
            };
            sent.push(self.id);
            let req = Message {
                version: self.version.wire(),
                community: self.community.to_vec(),
                pdu,
                id: self.id,
                status: 0,
                index: 0,
                binds: oids.iter().map(|o| (o.clone(), Value::Null)).collect(),
            };
            match self.sock.send(&req.encode()) {
                Ok(_) => {}
                Err(e) if refused(&e) => return Err(Fail::Refused),
                Err(e) => return Err(Fail::Io(e.to_string())),
            }
            let until = (Instant::now() + WAIT).min(self.deadline);
            loop {
                if self.cancel.load(Ordering::Relaxed) {
                    return Err(Fail::Stopped);
                }
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let _ = self
                    .sock
                    .set_read_timeout(Some(left.min(Duration::from_millis(200))));
                match self.sock.recv_from(&mut buf) {
                    Ok((n, from)) => {
                        if from != self.to {
                            continue;
                        }
                        if let Some(m) = Message::decode(&buf[..n])
                            && m.pdu == RESPONSE
                            && sent.contains(&m.id)
                            && m.version == self.version.wire()
                        {
                            return Ok(m);
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) => {}
                    Err(e) if refused(&e) => return Err(Fail::Refused),
                    Err(e) => return Err(Fail::Io(e.to_string())),
                }
            }
            if Instant::now() >= self.deadline {
                return Err(Fail::Late);
            }
        }
        Err(Fail::Silent)
    }

    /// The values of `oids` that the device has: one request, or one per
    /// OID when the device refuses the lot (v1 fails a whole request for
    /// one name it doesn't know; a reply too big fails it too).
    pub fn get(&mut self, oids: &[Oid]) -> Result<Vec<(Oid, Value)>, Fail> {
        let m = self.exchange(GET, oids)?;
        if m.status == 0 {
            return Ok(keep(oids, m.binds));
        }
        if oids.len() == 1 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for o in oids {
            let m = self.exchange(GET, std::slice::from_ref(o))?;
            if m.status == 0 {
                out.extend(keep(std::slice::from_ref(o), m.binds));
            }
        }
        Ok(out)
    }

    /// GETNEXT from `base` while the answers stay under it, at most `cap`
    /// rows. Ends early, keeping what it has and saying why in `cut`, at
    /// the read's time limit or if the device stops answering; with nothing
    /// read yet, either is an error, never an empty table.
    pub fn walk(&mut self, base: &[u32], cap: usize) -> Result<Walk, Fail> {
        self.walk_to(base, cap, |_| false)
    }

    /// `walk`, ending at (and keeping) the first row whose value `found`
    /// picks.
    pub fn walk_to(
        &mut self,
        base: &[u32],
        cap: usize,
        found: impl Fn(&Value) -> bool,
    ) -> Result<Walk, Fail> {
        let mut at = base.to_vec();
        let mut w = Walk::default();
        loop {
            if w.rows.len() >= cap {
                w.cut = Some(Cut::Cap);
                break;
            }
            let m = match self.exchange(GET_NEXT, std::slice::from_ref(&at)) {
                Ok(m) => m,
                Err(Fail::Late) if !w.rows.is_empty() => {
                    w.cut = Some(Cut::Late);
                    break;
                }
                Err(Fail::Silent) if !w.rows.is_empty() => {
                    w.cut = Some(Cut::Silent);
                    break;
                }
                Err(e) => return Err(e),
            };
            // v1 ends a walk with noSuchName.
            if m.status != 0 {
                break;
            }
            let Some((oid, v)) = m.binds.into_iter().next() else {
                break;
            };
            // Past the subtree, the end of the MIB, or an agent going
            // backwards (which would loop forever).
            if !oid.starts_with(base) || v.missing() || oid <= at {
                break;
            }
            at.clone_from(&oid);
            let stop = found(&v);
            w.rows.push((oid[base.len()..].to_vec(), v));
            if stop {
                break;
            }
        }
        Ok(w)
    }
}

fn refused(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset
    )
}

/// The varbinds that answer what was asked, with a value.
fn keep(asked: &[Oid], binds: Vec<(Oid, Value)>) -> Vec<(Oid, Value)> {
    binds
        .into_iter()
        .filter(|(o, v)| asked.contains(o) && !v.missing())
        .collect()
}

/// A walk's rows (each OID after the walked base) and why it stopped short.
#[derive(Default, Debug)]
pub struct Walk {
    pub rows: Vec<(Oid, Value)>,
    pub cut: Option<Cut>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cut {
    Cap,
    Late,
    Silent,
}

// -------------------------------------------------------------- reading

// Table entries (the table's OID and .1); a column is entry.N.
const SYS: [u32; 7] = [1, 3, 6, 1, 2, 1, 1];
const ENT_PHYSICAL: [u32; 11] = [1, 3, 6, 1, 2, 1, 47, 1, 1, 1, 1];
const HR_PRINTER: [u32; 10] = [1, 3, 6, 1, 2, 1, 25, 3, 5, 1];
const PRT_SUPPLIES: [u32; 10] = [1, 3, 6, 1, 2, 1, 43, 11, 1, 1];
const LLDP_REM: [u32; 10] = [1, 0, 8802, 1, 1, 2, 1, 4, 1, 1];
const LLDP_LOC_PORT: [u32; 10] = [1, 0, 8802, 1, 1, 2, 1, 3, 7, 1];

/// A column of a table, from its entry's OID.
fn col(entry: &[u32], column: u32) -> Oid {
    let mut o = entry.to_vec();
    o.push(column);
    o
}

fn sys(n: u32) -> Oid {
    let mut o = SYS.to_vec();
    o.extend([n, 0]);
    o
}

/// The system group.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct System {
    pub descr: Option<String>,
    pub object_id: Option<String>,
    pub uptime: Option<u32>,
    pub name: Option<String>,
    pub location: Option<String>,
    pub contact: Option<String>,
}

/// ENTITY-MIB's first chassis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chassis {
    pub model: Option<String>,
    /// entPhysicalFirmwareRev.
    pub firmware: Option<String>,
    /// entPhysicalSoftwareRev, kept only when it has no firmware revision.
    pub software: Option<String>,
    pub serial: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Printer {
    /// hrPrinterStatus.
    pub status: Option<i64>,
    /// hrPrinterDetectedErrorState, decoded.
    pub errors: Vec<&'static str>,
    pub supplies: Vec<Supply>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Supply {
    pub name: String,
    pub level: Option<i64>,
    pub max: Option<i64>,
}

/// One LLDP neighbour: the device at the far end of one of its ports.
#[derive(Clone, Debug, PartialEq)]
pub struct Neighbour {
    /// Its own port the neighbour was heard on.
    pub local_port: String,
    pub sys_name: Option<String>,
    pub port_id: Option<String>,
    pub port_desc: Option<String>,
}

/// What one read found.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub target: IpAddr,
    pub version: Version,
    pub system: System,
    pub chassis: Option<Chassis>,
    pub printer: Option<Printer>,
    pub lldp: Vec<Neighbour>,
    /// What was cut short and why.
    pub notes: Vec<String>,
}

/// How a read ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Read(Box<Reading>),
    /// No answer to v2c or v1.
    Silent(IpAddr),
    /// Port unreachable.
    Refused(IpAddr),
    Failed(IpAddr, String),
    Stopped(IpAddr),
}

/// One part of a read after the system group.
type Step = fn(&mut Agent, &mut Reading) -> Result<(), Fail>;

/// Read `ip` with `community`: the system group, then ENTITY-MIB, a
/// printer's state, and LLDP. v2c first; v1 if v2c gets no answer.
pub fn read(ip: IpAddr, community: &[u8], cancel: &AtomicBool) -> Outcome {
    read_at(SocketAddr::new(ip, PORT), community, cancel, WHOLE)
}

/// `read`, at any port (tests run an agent on a high one).
pub fn read_at(to: SocketAddr, community: &[u8], cancel: &AtomicBool, whole: Duration) -> Outcome {
    let ip = to.ip();
    let deadline = Instant::now() + whole;
    let oids: Vec<Oid> = (1..=6).map(sys).collect();
    let mut found = None;
    for version in [Version::V2c, Version::V1] {
        let mut a = match Agent::new(to, version, community, deadline, cancel) {
            Ok(a) => a,
            Err(e) => return Outcome::Failed(ip, e.to_string()),
        };
        match a.get(&oids) {
            Ok(got) => {
                found = Some((a, got));
                break;
            }
            Err(Fail::Silent) => continue,
            Err(Fail::Refused) => return Outcome::Refused(ip),
            Err(Fail::Stopped) => return Outcome::Stopped(ip),
            Err(Fail::Late) => return Outcome::Silent(ip),
            Err(Fail::Io(e)) => return Outcome::Failed(ip, e),
        }
    }
    let Some((mut a, got)) = found else {
        return Outcome::Silent(ip);
    };
    let at = |n: u32| got.iter().find(|(o, _)| *o == sys(n)).map(|(_, v)| v);
    let mut r = Reading {
        target: ip,
        version: a.version,
        system: System {
            descr: at(1).and_then(Value::text),
            object_id: at(2).and_then(Value::text),
            uptime: at(3).and_then(|v| match v {
                Value::Ticks(t) => Some(*t),
                _ => None,
            }),
            name: at(5).and_then(Value::text),
            location: at(6).and_then(Value::text),
            contact: at(4).and_then(Value::text),
        },
        chassis: None,
        printer: None,
        lldp: Vec::new(),
        notes: Vec::new(),
    };
    let steps: [(&str, Step); 3] = [
        ("ENTITY-MIB", chassis),
        ("the printer MIB", printer),
        ("LLDP", lldp),
    ];
    for (n, (what, step)) in steps.iter().enumerate() {
        let why = match step(&mut a, &mut r) {
            Ok(()) => continue,
            Err(Fail::Stopped) => return Outcome::Stopped(ip),
            Err(Fail::Late) => format!(
                "Stopped at the {} s limit while reading {what}.",
                whole.as_secs()
            ),
            Err(Fail::Silent) => format!("It stopped answering while {what} was read."),
            Err(Fail::Refused) => format!("It refused UDP 161 while {what} was read."),
            Err(Fail::Io(e)) => format!("Reading {what}: {e}"),
        };
        let rest: Vec<&str> = steps[n + 1..].iter().map(|(w, _)| *w).collect();
        r.notes.push(if rest.is_empty() {
            why
        } else {
            format!("{why} Not read: {}.", rest.join(", "))
        });
        break;
    }
    Outcome::Read(Box::new(r))
}

/// Say in the reading that a walk stopped short. `cap` is None for a walk
/// whose cap isn't news (one that wants the first row, or follows a walk
/// whose cap was already said).
fn note_cut(r: &mut Reading, what: &str, w: &Walk, cap: Option<usize>) {
    match w.cut {
        Some(Cut::Cap) => {
            if let Some(cap) = cap {
                r.notes.push(format!("{what}: the first {cap} rows only."));
            }
        }
        Some(Cut::Late) => r
            .notes
            .push(format!("{what}: cut short at the time limit.")),
        Some(Cut::Silent) => r
            .notes
            .push(format!("{what}: cut short, it stopped answering.")),
        None => {}
    }
}

/// entPhysicalClass chassis.
const CLASS_CHASSIS: i64 = 3;

/// ENTITY-MIB's first entry of class chassis: its model, firmware (or
/// software, when it has no firmware revision) and serial. The walk ends at
/// that entry, so a big stack's modules and ports don't eat the read's time.
fn chassis(a: &mut Agent, r: &mut Reading) -> Result<(), Fail> {
    let class = a.walk_to(&col(&ENT_PHYSICAL, 5), TABLE_ROWS, |v| {
        v.int() == Some(CLASS_CHASSIS)
    })?;
    let Some(idx) = class
        .rows
        .iter()
        .find(|(_, v)| v.int() == Some(CLASS_CHASSIS))
        .map(|(i, _)| i.clone())
    else {
        if class.cut == Some(Cut::Cap) {
            r.notes.push(format!(
                "ENTITY-MIB: no chassis in its first {TABLE_ROWS} entries."
            ));
        } else {
            note_cut(r, "ENTITY-MIB", &class, None);
        }
        return Ok(());
    };
    let of = |c: u32| {
        let mut o = col(&ENT_PHYSICAL, c);
        o.extend(&idx);
        o
    };
    // Model name, firmware, software and serial revisions.
    let got = a.get(&[of(13), of(9), of(10), of(11)])?;
    let at = |c: u32| {
        got.iter()
            .find(|(o, _)| *o == of(c))
            .and_then(|(_, v)| v.text())
    };
    let firmware = at(9);
    let c = Chassis {
        model: at(13),
        software: if firmware.is_none() { at(10) } else { None },
        firmware,
        serial: at(11),
    };
    if c != Chassis::default() {
        r.chassis = Some(c);
    }
    Ok(())
}

/// hrPrinterDetectedErrorState's bits, first byte's high bit first.
const PRINTER_ERRORS: [&str; 16] = [
    "low paper",
    "no paper",
    "low toner",
    "no toner",
    "door open",
    "jammed",
    "offline",
    "service requested",
    "input tray missing",
    "output tray missing",
    "marker supply missing",
    "output near full",
    "output full",
    "input tray empty",
    "overdue preventive maintenance",
    "",
];

pub fn printer_errors(b: &[u8]) -> Vec<&'static str> {
    (0..16)
        .filter(|&bit| {
            b.get(bit / 8)
                .is_some_and(|byte| byte & (0x80 >> (bit % 8)) != 0)
        })
        .map(|bit| PRINTER_ERRORS[bit])
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn printer_status(s: i64) -> &'static str {
    match s {
        3 => "idle",
        4 => "printing",
        5 => "warming up",
        2 => "unknown",
        _ => "other",
    }
}

/// A printer (Host Resources' printer table answers): its status, errors
/// and supplies.
/// The reading keeps each part as it's read, so a read cut short shows
/// what came (and its note says what didn't).
fn printer(a: &mut Agent, r: &mut Reading) -> Result<(), Fail> {
    let status = a.walk(&col(&HR_PRINTER, 1), 1)?;
    note_cut(r, "The printer's status", &status, None);
    let Some((idx, st)) = status.rows.first().cloned() else {
        return Ok(());
    };
    r.printer = Some(Printer {
        status: st.int(),
        errors: Vec::new(),
        supplies: Vec::new(),
    });
    let mut err_oid = col(&HR_PRINTER, 2);
    err_oid.extend(&idx);
    if let Some((_, Value::Bytes(b))) = a.get(&[err_oid])?.first()
        && let Some(p) = r.printer.as_mut()
    {
        p.errors = printer_errors(b);
    }
    let names = a.walk(&col(&PRT_SUPPLIES, 6), SUPPLY_ROWS)?;
    note_cut(r, "Supplies", &names, Some(SUPPLY_ROWS));
    let rows: Vec<(Oid, Supply)> = names
        .rows
        .iter()
        .filter_map(|(i, v)| {
            Some((
                i.clone(),
                Supply {
                    name: v.text()?,
                    level: None,
                    max: None,
                },
            ))
        })
        .collect();
    if rows.is_empty() {
        return Ok(());
    }
    let ids: Vec<Oid> = rows.iter().map(|(i, _)| i.clone()).collect();
    if let Some(p) = r.printer.as_mut() {
        p.supplies = rows.into_iter().map(|(_, s)| s).collect();
    }
    let find = |w: &Walk, i: &Oid| {
        w.rows
            .iter()
            .find(|(o, _)| o == i)
            .and_then(|(_, v)| v.int())
    };
    // Same cap as the names, already said.
    let max = a.walk(&col(&PRT_SUPPLIES, 8), SUPPLY_ROWS)?;
    note_cut(r, "Supplies' capacities", &max, None);
    if let Some(p) = r.printer.as_mut() {
        for (s, i) in p.supplies.iter_mut().zip(&ids) {
            s.max = find(&max, i);
        }
    }
    let level = a.walk(&col(&PRT_SUPPLIES, 9), SUPPLY_ROWS)?;
    note_cut(r, "Supplies' levels", &level, None);
    if let Some(p) = r.printer.as_mut() {
        for (s, i) in p.supplies.iter_mut().zip(&ids) {
            s.level = find(&level, i);
        }
    }
    Ok(())
}

impl Supply {
    /// "45%", "some left", "unknown".
    pub fn left(&self) -> String {
        match (self.level, self.max) {
            (Some(-3), _) => "some left".into(),
            (Some(l), Some(m)) if l >= 0 && m > 0 => {
                // i128: an 8-byte INTEGER times 100 doesn't fit an i64.
                let (l, m) = (i128::from(l.min(m)), i128::from(m));
                format!("{}%", (l * 100 + m / 2) / m)
            }
            (Some(l), _) if l >= 0 => format!("level {l}"),
            _ => "unknown".into(),
        }
    }
}

/// LLDP's neighbour table: who is at the far end of each of its ports.
///
/// Each neighbour is in the reading from its first walk on, and filled in
/// as the rest comes, so a read cut short keeps what it heard.
fn lldp(a: &mut Agent, r: &mut Reading) -> Result<(), Fail> {
    let port_id = a.walk(&col(&LLDP_REM, 7), TABLE_ROWS)?;
    note_cut(r, "LLDP", &port_id, Some(TABLE_ROWS));
    // Index: time mark, local port number, neighbour index.
    let mut ids: Vec<(Oid, u32)> = Vec::new();
    let first = r.lldp.len();
    for (i, v) in &port_id.rows {
        let Some(&p) = i.get(1) else { continue };
        ids.push((i.clone(), p));
        r.lldp.push(Neighbour {
            local_port: format!("port {p}"),
            sys_name: None,
            port_id: v.text(),
            port_desc: None,
        });
    }
    if ids.is_empty() {
        return Ok(());
    }
    let find = |w: &Walk, i: &Oid| {
        w.rows
            .iter()
            .find(|(o, _)| o == i)
            .and_then(|(_, v)| v.text())
    };
    // Same cap as the port IDs, already said.
    let port_desc = a.walk(&col(&LLDP_REM, 8), TABLE_ROWS)?;
    note_cut(r, "LLDP's port descriptions", &port_desc, None);
    for (n, (i, _)) in r.lldp[first..].iter_mut().zip(&ids) {
        n.port_desc = find(&port_desc, i);
    }
    let name = a.walk(&col(&LLDP_REM, 9), TABLE_ROWS)?;
    note_cut(r, "LLDP's names", &name, None);
    for (n, (i, _)) in r.lldp[first..].iter_mut().zip(&ids) {
        n.sys_name = find(&name, i);
    }
    let mut ports: Vec<u32> = ids.iter().map(|(_, p)| *p).collect();
    ports.sort_unstable();
    ports.dedup();
    for p in ports {
        let (mut desc, mut id) = (col(&LLDP_LOC_PORT, 4), col(&LLDP_LOC_PORT, 3));
        desc.push(p);
        id.push(p);
        let got = a.get(&[desc.clone(), id.clone()])?;
        let at = |o: &Oid| got.iter().find(|(g, _)| g == o).and_then(|(_, v)| v.text());
        if let Some(own) = at(&desc).or_else(|| at(&id)) {
            for (n, _) in r.lldp[first..]
                .iter_mut()
                .zip(&ids)
                .filter(|(_, (_, q))| *q == p)
            {
                n.local_port.clone_from(&own);
            }
        }
    }
    Ok(())
}

/// What a sysObjectID's enterprise number says, for the common ones.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Enterprise {
    /// The device's maker.
    Maker(&'static str),
    /// Agent or switch software that many makers ship: it names the
    /// software, never who made the device.
    Software(&'static str),
}

/// What `object_id`'s enterprise number names, for the common ones.
pub fn enterprise(object_id: &str) -> Option<Enterprise> {
    let n: u32 = object_id
        .strip_prefix("1.3.6.1.4.1.")?
        .split('.')
        .next()?
        .parse()
        .ok()?;
    let software = match n {
        311 => Some("Microsoft's SNMP service (Windows; not the maker)"),
        6876 => Some("VMware ESXi (the hypervisor's agent; not the maker)"),
        8072 => Some("Net-SNMP (agent software many makers use; not the maker)"),
        4413 => Some("Broadcom FASTPATH (switch software several makers use; not the maker)"),
        _ => None,
    };
    if let Some(s) = software {
        return Some(Enterprise::Software(s));
    }
    Some(Enterprise::Maker(match n {
        9 => "Cisco",
        11 => "HP",
        43 => "3Com",
        171 => "D-Link",
        253 => "Xerox",
        367 => "Ricoh",
        641 => "Lexmark",
        674 => "Dell",
        1347 => "Kyocera",
        1602 => "Canon",
        1916 => "Extreme",
        2011 => "Huawei",
        2435 => "Brother",
        2636 => "Juniper",
        3375 => "F5",
        4526 => "Netgear",
        6574 => "Synology",
        8741 => "SonicWall",
        11863 => "TP-Link",
        12356 => "Fortinet",
        14179 => "Cisco (wireless)",
        14823 => "Aruba",
        14988 => "MikroTik",
        18334 => "Konica Minolta",
        24681 => "QNAP",
        25461 => "Palo Alto Networks",
        29671 => "Cisco Meraki",
        30065 => "Arista",
        41112 => "Ubiquiti",
        47196 => "HPE Aruba",
        _ => return None,
    }))
}

// ---------------------------------------------------------------- words

/// Said when nothing answered: the four reasons look the same from here.
pub const SILENT_WHY: &str = "SNMP may be off, the community may be wrong, the device may \
answer only some addresses (an ACL by source IP), or it may speak SNMPv3 only. No answer looks \
the same for all four, so this can't tell which.";

impl Outcome {
    pub fn target(&self) -> IpAddr {
        match self {
            Outcome::Read(r) => r.target,
            Outcome::Silent(ip)
            | Outcome::Refused(ip)
            | Outcome::Failed(ip, _)
            | Outcome::Stopped(ip) => *ip,
        }
    }

    /// One line on why there's no reading; None when there is one.
    pub fn problem(&self) -> Option<String> {
        Some(match self {
            Outcome::Read(_) => return None,
            Outcome::Silent(ip) => format!(
                "No SNMP answer from {ip} on UDP 161 (v2c, then v1; {} s each, tried twice). {SILENT_WHY}",
                WAIT.as_secs()
            ),
            Outcome::Refused(ip) => format!(
                "{ip} answered that nothing takes SNMP on UDP 161 (ICMP port unreachable): its \
                 SNMP agent is off, or its own firewall rejects it."
            ),
            Outcome::Failed(ip, e) => format!("SNMP to {ip} couldn't be sent: {e}"),
            Outcome::Stopped(ip) => format!("The SNMP read of {ip} was stopped."),
        })
    }

    /// (section, field, value) rows, as Results logs them: no community,
    /// and no sysContact (a person's name, email or phone, often), which
    /// shows in the inspector only (`screen_rows`).
    pub fn rows(&self) -> Vec<(&'static str, String, String)> {
        self.rows_with(false)
    }

    /// `rows` with sysContact, for the inspector. Never logged.
    pub fn screen_rows(&self) -> Vec<(&'static str, String, String)> {
        self.rows_with(true)
    }

    fn rows_with(&self, contact: bool) -> Vec<(&'static str, String, String)> {
        let Outcome::Read(r) = self else {
            let why = match self {
                Outcome::Silent(_) => "no answer",
                Outcome::Refused(_) => "port unreachable",
                Outcome::Stopped(_) => "stopped",
                _ => "error",
            };
            return vec![("snmp", "answer".into(), why.into())];
        };
        let mut v = vec![("snmp", "version".into(), r.version.name().into())];
        let s = &r.system;
        let mut add = |sec: &'static str, f: &str, val: &Option<String>| {
            if let Some(x) = val {
                v.push((sec, f.to_string(), x.clone()));
            }
        };
        add("system", "name", &s.name);
        add("system", "description", &s.descr);
        add("system", "object_id", &s.object_id);
        match s.object_id.as_deref().and_then(enterprise) {
            Some(Enterprise::Maker(m)) => add("system", "maker", &Some(m.into())),
            Some(Enterprise::Software(w)) => add("system", "agent", &Some(w.into())),
            None => {}
        }
        add("system", "uptime", &s.uptime.map(uptime));
        add("system", "location", &s.location);
        if contact {
            add("system", "contact", &s.contact);
        }
        if let Some(c) = &r.chassis {
            add("chassis", "model", &c.model);
            add("chassis", "firmware", &c.firmware);
            add("chassis", "software", &c.software);
            add("chassis", "serial", &c.serial);
        }
        if let Some(p) = &r.printer {
            add(
                "printer",
                "status",
                &p.status.map(|s| printer_status(s).to_string()),
            );
            let errs = if p.errors.is_empty() {
                "none".to_string()
            } else {
                p.errors.join(", ")
            };
            v.push(("printer", "errors".into(), errs));
            for s in &p.supplies {
                v.push(("supply", s.name.clone(), s.left()));
            }
        }
        for n in &r.lldp {
            let far = [
                n.sys_name.clone().unwrap_or_else(|| "(no name)".into()),
                n.port_id.clone().unwrap_or_default(),
                n.port_desc
                    .clone()
                    .filter(|d| Some(d) != n.port_id.as_ref())
                    .map(|d| format!("({d})"))
                    .unwrap_or_default(),
            ]
            .iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
            v.push(("lldp", n.local_port.clone(), far));
        }
        for n in &r.notes {
            v.push(("note", "cut".into(), n.clone()));
        }
        v
    }

    /// The run's text for Results. No community.
    pub fn report(&self) -> String {
        let ip = self.target();
        if let Some(p) = self.problem() {
            return format!("SNMP read of {ip}\n{p}\n");
        }
        let Outcome::Read(r) = self else {
            return String::new();
        };
        let mut s = format!("SNMP read of {ip} ({})\n", r.version.name());
        for (sec, field, value) in self.rows().into_iter().skip(1) {
            let (k, v) = pair(sec, &field, &value);
            s += &format!("{k}: {v}\n");
        }
        // sysContact isn't logged, so it doesn't count here.
        let logged = System {
            contact: None,
            ..r.system.clone()
        };
        if logged == System::default() && r.chassis.is_none() {
            s += if r.system.contact.is_some() {
                "It answered, but with only its contact (sysContact) of the system group, which isn't logged.\n"
            } else {
                "It answered, but with none of the system group.\n"
            };
        }
        s
    }
}

/// A row of `Outcome::rows` as a label and its value, for the inspector
/// and the report: "Supply" and "Black Toner: 45%", "LLDP" and "eth0 →
/// SW-OFFICE-1 Gi1/0/14" (its own port, then who's at the far end).
pub fn pair(section: &str, field: &str, value: &str) -> (String, String) {
    let k = |s: &str| s.to_string();
    match (section, field) {
        ("system", "object_id") => (k("Object ID"), k(value)),
        ("system", "agent") => (k("Agent software"), k(value)),
        ("printer", "status") => (k("Printer"), k(value)),
        ("printer", "errors") => (k("Errors"), k(value)),
        ("supply", f) => (k("Supply"), format!("{f}: {value}")),
        ("lldp", f) => (k("LLDP"), format!("{f} → {value}")),
        ("note", _) => (k("Note"), k(value)),
        (_, f) => {
            let mut c = f.chars();
            let label = c
                .next()
                .map(|h| h.to_uppercase().chain(c).collect())
                .unwrap_or_default();
            (label, k(value))
        }
    }
}

/// A fictional printer's reading (documentation address, made-up serial),
/// for the scene tour and tests.
pub fn sample(ip: IpAddr) -> Reading {
    Reading {
        target: ip,
        version: Version::V2c,
        system: System {
            descr: Some("HP ETHERNET MULTI-ENVIRONMENT,ROM none,JETDIRECT,JD153".into()),
            object_id: Some("1.3.6.1.4.1.11.2.3.9.1".into()),
            uptime: Some(105_264_000),
            name: Some("PRN-FRONTDESK".into()),
            location: Some("Front desk".into()),
            contact: Some("IT desk, ext. 100".into()),
        },
        chassis: Some(Chassis {
            model: Some("HP LaserJet Pro M404".into()),
            firmware: Some("20231115".into()),
            software: None,
            serial: Some("EXAMPLE0123".into()),
        }),
        printer: Some(Printer {
            status: Some(3),
            errors: vec!["low toner"],
            supplies: vec![Supply {
                name: "Black Cartridge HP 58A".into(),
                level: Some(12),
                max: Some(100),
            }],
        }),
        lldp: vec![Neighbour {
            local_port: "eth0".into(),
            sys_name: Some("SW-OFFICE-1".into()),
            port_id: Some("Gi1/0/14".into()),
            port_desc: Some("Front desk printer".into()),
        }],
        notes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    fn oid(s: &str) -> Oid {
        s.split('.').map(|p| p.parse().unwrap()).collect()
    }

    fn reply(binds: Vec<(Oid, Value)>) -> Message {
        Message {
            version: 1,
            community: b"public".to_vec(),
            pdu: RESPONSE,
            id: 77,
            status: 0,
            index: 0,
            binds,
        }
    }

    #[test]
    fn messages_round_trip() {
        let m = reply(vec![
            (sys(1), Value::Bytes(b"Example switch".to_vec())),
            (sys(2), Value::Oid(oid("1.3.6.1.4.1.32473.1"))),
            (sys(3), Value::Ticks(4_000_000_000)),
            (oid("1.3.6.1.2.1.2.2.1.10.1"), Value::Count(u64::MAX)),
            (oid("1.3.6.1.2.1.4.20.1.1.1"), Value::Ip([192, 0, 2, 1])),
            (oid("1.3.6.1.2.1.43.11.1.1.9.1.1"), Value::Int(-3)),
            (oid("1.3.6.1.2.1.1.9.0"), Value::NoSuchObject),
            (oid("2.999.4294967295"), Value::EndOfMib),
        ]);
        assert_eq!(Message::decode(&m.encode()), Some(m));
        for v in [
            0,
            1,
            -1,
            127,
            128,
            -128,
            -129,
            i64::MAX,
            i64::MIN,
            0x7fff_ffff,
        ] {
            assert_eq!(int(&int_bytes(v)), Some(v), "{v}");
        }
    }

    /// A GetRequest as net-snmp's snmpget sends it (sysDescr.0, v2c,
    /// community "public", request-id 0x1a2b).
    #[test]
    fn encodes_a_get_as_snmpget_does() {
        let m = Message {
            version: 1,
            community: b"public".to_vec(),
            pdu: GET,
            id: 0x1a2b,
            status: 0,
            index: 0,
            binds: vec![(sys(1), Value::Null)],
        };
        let want = [
            0x30, 0x27, 0x02, 0x01, 0x01, 0x04, 0x06, b'p', b'u', b'b', b'l', b'i', b'c', 0xa0,
            0x1a, 0x02, 0x02, 0x1a, 0x2b, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00, 0x30, 0x0e, 0x30,
            0x0c, 0x06, 0x08, 0x2b, 0x06, 0x01, 0x02, 0x01, 0x01, 0x01, 0x00, 0x05, 0x00,
        ];
        assert_eq!(m.encode(), want);
    }

    #[test]
    fn hostile_replies_never_panic() {
        let good = reply(
            (1..=6)
                .map(|n| (sys(n), Value::Bytes(b"Example value".to_vec())))
                .chain([(sys(3), Value::Ticks(12345))])
                .collect(),
        )
        .encode();
        crate::hostile::each(&good, |b| {
            if let Some(m) = Message::decode(b) {
                for (_, v) in &m.binds {
                    let _ = v.text();
                }
                let _ = printer_errors(&m.community);
            }
        });
    }

    #[test]
    fn rejects_what_isnt_a_message() {
        // Indefinite length, five length bytes, a length past the end.
        assert!(Message::decode(&[0x30, 0x80, 0x00, 0x00]).is_none());
        assert!(Message::decode(&[0x30, 0x85, 1, 0, 0, 0, 0]).is_none());
        assert!(Message::decode(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff]).is_none());
        // An OID whose sub-identifier never ends, or overflows 32 bits.
        assert!(parse_oid(&[0x2b, 0x86]).is_none());
        assert!(parse_oid(&[0x2b, 0x90, 0x80, 0x80, 0x80, 0x00]).is_none());
        assert_eq!(
            parse_oid(&[0x2b, 0x8f, 0xff, 0xff, 0xff, 0x7f]),
            Some(vec![1, 3, u32::MAX])
        );
        // More sub-identifiers than any OID has.
        let mut long = vec![0x2b];
        long.extend([1u8; 200]);
        assert!(parse_oid(&long).is_none());
        // Too many varbinds.
        let many = reply(
            (0..100)
                .map(|n| (oid(&format!("1.3.6.{n}")), Value::Null))
                .collect(),
        );
        assert!(Message::decode(&many.encode()).is_none());
        // A deeply nested constructed value is kept as a tag, not walked into.
        let mut deep = Vec::new();
        for _ in 0..1000 {
            let mut d = Vec::new();
            put(&mut d, SEQ, &deep);
            deep = d;
        }
        let mut vb = Vec::new();
        put(&mut vb, OID, &oid_bytes(&sys(1)));
        vb.extend(&deep);
        let mut list = Vec::new();
        put(&mut list, SEQ, &vb);
        let mut pdu = Vec::new();
        for _ in 0..3 {
            put(&mut pdu, INT, &[0]);
        }
        put(&mut pdu, SEQ, &list);
        let mut msg = Vec::new();
        put(&mut msg, INT, &[1]);
        put(&mut msg, OCTETS, b"public");
        put(&mut msg, RESPONSE, &pdu);
        let mut whole = Vec::new();
        put(&mut whole, SEQ, &msg);
        let m = Message::decode(&whole).unwrap();
        assert_eq!(m.binds, [(sys(1), Value::Other(SEQ))]);
    }

    #[test]
    fn printer_errors_are_decoded_high_bit_first() {
        assert_eq!(printer_errors(&[0x20]), ["low toner"]);
        assert_eq!(
            printer_errors(&[0x04, 0x80]),
            ["jammed", "input tray missing"]
        );
        assert!(printer_errors(&[]).is_empty());
        assert!(printer_errors(&[0, 0x01]).is_empty());
    }

    #[test]
    fn supplies_say_whats_left() {
        let s = |level, max| Supply {
            name: "Toner".into(),
            level,
            max,
        };
        assert_eq!(s(Some(45), Some(100)).left(), "45%");
        assert_eq!(s(Some(450), Some(1000)).left(), "45%");
        assert_eq!(s(Some(-3), Some(-2)).left(), "some left");
        assert_eq!(s(Some(-2), Some(100)).left(), "unknown");
        assert_eq!(s(Some(7), Some(-2)).left(), "level 7");
        assert_eq!(s(None, None).left(), "unknown");
    }

    #[test]
    fn text_values_read_as_people_would() {
        assert_eq!(text(b"Black Toner\0\0"), "Black Toner");
        assert_eq!(text(b"a\r\n  b"), "a b");
        assert_eq!(text(&[0x00, 0x1b, 0xff]), "00:1b:ff");
        assert_eq!(uptime(4_500), "45 s");
        assert_eq!(uptime(100 * 3700), "1 h 1 min");
        assert_eq!(uptime(100 * 86_400 * 12 + 100 * 3600 * 3), "12 d 3 h");
        assert_eq!(
            enterprise("1.3.6.1.4.1.11.2.3.9.1"),
            Some(Enterprise::Maker("HP"))
        );
        assert_eq!(enterprise("1.3.6.1.4.1.32473.1"), None);
        assert_eq!(enterprise("garbage"), None);
    }

    /// A pretend agent on 127.0.0.1: answers GET and GETNEXT from `mib`
    /// for `community`, only as `version`. `drop_first` requests go
    /// unanswered; `decoy` sends a wrong-ID reply before each real one;
    /// `late_first` answers the very first request after its retry has
    /// gone out, and never the retry; `answer_only` (when not 0) answers
    /// that many requests, then goes quiet.
    #[derive(Default)]
    struct Fake {
        mib: BTreeMap<Oid, Value>,
        community: &'static [u8],
        version: i64,
        drop_first: usize,
        decoy: bool,
        late_first: bool,
        answer_only: usize,
    }

    fn serve(f: Fake) -> (SocketAddr, Arc<AtomicUsize>) {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        let seen = Arc::new(AtomicUsize::new(0));
        let count = seen.clone();
        sock.set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok((n, from)) = sock.recv_from(&mut buf) {
                let k = count.fetch_add(1, Ordering::Relaxed);
                let Some(req) = Message::decode(&buf[..n]) else {
                    continue;
                };
                if k < f.drop_first || req.community != f.community || req.version != f.version {
                    continue;
                }
                if (f.late_first && k == 1) || (f.answer_only > 0 && k >= f.answer_only) {
                    continue;
                }
                if f.late_first && k == 0 {
                    // Past WAIT: the client has sent its retry by now.
                    std::thread::sleep(WAIT + Duration::from_millis(500));
                }
                let mut status = 0;
                let mut index = 0;
                let mut binds = Vec::new();
                for (i, (o, _)) in req.binds.iter().enumerate() {
                    let hit = if req.pdu == GET_NEXT {
                        f.mib
                            .range::<Oid, _>((
                                std::ops::Bound::Excluded(o),
                                std::ops::Bound::Unbounded,
                            ))
                            .next()
                            .map(|(k, v)| (k.clone(), v.clone()))
                    } else {
                        f.mib.get(o).map(|v| (o.clone(), v.clone()))
                    };
                    match (hit, f.version) {
                        (Some(b), _) => binds.push(b),
                        (None, 0) => {
                            status = 2;
                            index = i as i64 + 1;
                            binds.push((o.clone(), Value::Null));
                        }
                        (None, _) if req.pdu == GET_NEXT => {
                            binds.push((o.clone(), Value::EndOfMib))
                        }
                        (None, _) => binds.push((o.clone(), Value::NoSuchObject)),
                    }
                }
                let m = Message {
                    pdu: RESPONSE,
                    status,
                    index,
                    binds,
                    ..req
                };
                if f.decoy {
                    let mut d = m.clone();
                    d.id = m.id.wrapping_add(1);
                    d.binds = vec![(sys(5), Value::Bytes(b"DECOY".to_vec()))];
                    let _ = sock.send_to(&d.encode(), from);
                }
                let _ = sock.send_to(&m.encode(), from);
            }
        });
        (addr, seen)
    }

    fn printer_mib() -> BTreeMap<Oid, Value> {
        let b = |s: &str| Value::Bytes(s.as_bytes().to_vec());
        let mut m = BTreeMap::new();
        m.insert(sys(1), b("Example printer"));
        m.insert(sys(2), Value::Oid(oid("1.3.6.1.4.1.11.2.3.9.1")));
        m.insert(sys(3), Value::Ticks(360_000));
        m.insert(sys(5), b("PRN-EXAMPLE"));
        m.insert(sys(6), b("Room 1"));
        // ENTITY-MIB: a module (9) first, then the chassis (3) at index 2.
        m.insert(oid("1.3.6.1.2.1.47.1.1.1.1.5.1"), Value::Int(9));
        m.insert(oid("1.3.6.1.2.1.47.1.1.1.1.5.2"), Value::Int(3));
        m.insert(
            oid("1.3.6.1.2.1.47.1.1.1.1.13.2"),
            b("Example LaserPrint 400"),
        );
        m.insert(oid("1.3.6.1.2.1.47.1.1.1.1.11.2"), b("EXAMPLE0001"));
        m.insert(oid("1.3.6.1.2.1.25.3.5.1.1.1"), Value::Int(3));
        m.insert(oid("1.3.6.1.2.1.25.3.5.1.2.1"), Value::Bytes(vec![0x24]));
        for (i, (name, max, level)) in [("Black Toner\0", 1000, 450), ("Drum", -2, -3)]
            .into_iter()
            .enumerate()
        {
            let i = i as u32 + 1;
            m.insert(oid(&format!("1.3.6.1.2.1.43.11.1.1.6.1.{i}")), b(name));
            m.insert(
                oid(&format!("1.3.6.1.2.1.43.11.1.1.8.1.{i}")),
                Value::Int(max),
            );
            m.insert(
                oid(&format!("1.3.6.1.2.1.43.11.1.1.9.1.{i}")),
                Value::Int(level),
            );
        }
        // LLDP: one neighbour on local port 1.
        m.insert(oid("1.0.8802.1.1.2.1.3.7.1.4.1"), b("eth0"));
        m.insert(oid("1.0.8802.1.1.2.1.4.1.1.7.0.1.1"), b("Gi1/0/7"));
        m.insert(oid("1.0.8802.1.1.2.1.4.1.1.8.0.1.1"), b("Printer port"));
        m.insert(oid("1.0.8802.1.1.2.1.4.1.1.9.0.1.1"), b("SW-EXAMPLE"));
        m
    }

    fn quiet() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn reads_a_printer_over_v2c() {
        let (addr, _) = serve(Fake {
            mib: printer_mib(),
            community: b"s3cret-example",
            version: 1,
            drop_first: 0,
            decoy: true,
            ..Fake::default()
        });
        let out = read_at(addr, b"s3cret-example", &quiet(), Duration::from_secs(20));
        let Outcome::Read(r) = &out else {
            panic!("{out:?}")
        };
        assert_eq!(r.version, Version::V2c);
        // The decoy (wrong request ID) was ignored.
        assert_eq!(r.system.name.as_deref(), Some("PRN-EXAMPLE"));
        assert_eq!(r.system.uptime, Some(360_000));
        let c = r.chassis.as_ref().unwrap();
        assert_eq!(c.model.as_deref(), Some("Example LaserPrint 400"));
        assert_eq!(c.serial.as_deref(), Some("EXAMPLE0001"));
        let p = r.printer.as_ref().unwrap();
        assert_eq!(p.status, Some(3));
        assert_eq!(p.errors, ["low toner", "jammed"]);
        assert_eq!(p.supplies.len(), 2);
        assert_eq!(p.supplies[0].name, "Black Toner");
        assert_eq!(p.supplies[0].left(), "45%");
        assert_eq!(p.supplies[1].left(), "some left");
        assert_eq!(
            r.lldp,
            [Neighbour {
                local_port: "eth0".into(),
                sys_name: Some("SW-EXAMPLE".into()),
                port_id: Some("Gi1/0/7".into()),
                port_desc: Some("Printer port".into()),
            }]
        );
        let text = out.report();
        assert!(text.contains("Maker: HP"), "{text}");
        assert!(
            text.contains("LLDP: eth0 → SW-EXAMPLE Gi1/0/7 (Printer port)"),
            "{text}"
        );
        // The community is never in what's logged.
        assert!(!text.contains("s3cret"));
        assert!(
            out.rows()
                .iter()
                .all(|(_, f, v)| !f.contains("s3cret") && !v.contains("s3cret"))
        );
    }

    #[test]
    fn falls_back_to_v1_and_retries_a_lost_request() {
        let (addr, seen) = serve(Fake {
            mib: printer_mib(),
            community: b"public",
            version: 0,
            drop_first: 0,
            decoy: false,
            ..Fake::default()
        });
        let out = read_at(addr, b"public", &quiet(), Duration::from_secs(30));
        let Outcome::Read(r) = &out else {
            panic!("{out:?}")
        };
        assert_eq!(r.version, Version::V1);
        // v1 fails a GET with one unknown name (sysContact): the others still come.
        assert_eq!(r.system.contact, None);
        assert_eq!(r.system.location.as_deref(), Some("Room 1"));
        assert_eq!(r.printer.as_ref().unwrap().supplies.len(), 2);
        assert!(seen.load(Ordering::Relaxed) > 2);

        // The first request lost: the retry gets it.
        let (addr, _) = serve(Fake {
            mib: printer_mib(),
            community: b"public",
            version: 1,
            drop_first: 1,
            decoy: false,
            ..Fake::default()
        });
        let out = read_at(addr, b"public", &quiet(), Duration::from_secs(30));
        assert!(matches!(out, Outcome::Read(_)), "{out:?}");
    }

    #[test]
    fn a_wrong_community_reads_as_no_answer() {
        let (addr, seen) = serve(Fake {
            mib: printer_mib(),
            community: b"right",
            version: 1,
            drop_first: 0,
            decoy: false,
            ..Fake::default()
        });
        let t = Instant::now();
        let out = read_at(addr, b"bad-guess-77", &quiet(), Duration::from_secs(30));
        assert_eq!(out, Outcome::Silent(addr.ip()));
        // v2c twice, v1 twice, 2 s each.
        assert_eq!(seen.load(Ordering::Relaxed), 4);
        assert!(t.elapsed() < Duration::from_secs(10));
        let p = out.problem().unwrap();
        assert!(
            p.contains("can't tell which") && p.contains("SNMPv3"),
            "{p}"
        );
        assert!(!out.report().contains("bad-guess"));
    }

    #[test]
    fn answers_from_another_port_are_ignored() {
        // The real agent never answers; another socket replies to every
        // request with the right ID, from the wrong port.
        let agent = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = agent.local_addr().unwrap();
        agent
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        std::thread::spawn(move || {
            let liar = UdpSocket::bind("127.0.0.1:0").unwrap();
            let mut buf = [0u8; 2048];
            while let Ok((n, from)) = agent.recv_from(&mut buf) {
                if let Some(req) = Message::decode(&buf[..n]) {
                    let m = Message {
                        pdu: RESPONSE,
                        binds: vec![(sys(5), Value::Bytes(b"SPOOF".to_vec()))],
                        ..req
                    };
                    let _ = liar.send_to(&m.encode(), from);
                }
            }
        });
        let out = read_at(addr, b"public", &quiet(), Duration::from_secs(20));
        assert_eq!(out, Outcome::Silent(addr.ip()));
    }

    #[test]
    fn a_closed_port_is_said_plainly() {
        // Nothing listens: Linux and Windows report the port unreachable.
        let addr = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let out = read_at(addr, b"public", &quiet(), Duration::from_secs(10));
        if cfg!(any(target_os = "linux", windows)) {
            assert_eq!(out, Outcome::Refused(addr.ip()));
        }
    }

    #[test]
    fn walks_stop_at_their_cap_and_on_an_agent_going_backwards() {
        let mut mib = BTreeMap::new();
        for i in 0..200u32 {
            mib.insert(
                oid(&format!("1.0.8802.1.1.2.1.4.1.1.7.0.{i}.1")),
                Value::Bytes(b"p".to_vec()),
            );
        }
        let (addr, _) = serve(Fake {
            mib,
            community: b"public",
            version: 1,
            drop_first: 0,
            decoy: false,
            ..Fake::default()
        });
        let cancel = quiet();
        let mut a = Agent::new(
            addr,
            Version::V2c,
            b"public",
            Instant::now() + Duration::from_secs(20),
            &cancel,
        )
        .unwrap();
        let w = a.walk(&col(&LLDP_REM, 7), TABLE_ROWS).unwrap();
        assert_eq!(w.rows.len(), TABLE_ROWS);
        assert_eq!(w.cut, Some(Cut::Cap));

        // An agent that answers every GETNEXT with the same OID.
        let agent = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = agent.local_addr().unwrap();
        agent
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while let Ok((n, from)) = agent.recv_from(&mut buf) {
                if let Some(req) = Message::decode(&buf[..n]) {
                    let m = Message {
                        pdu: RESPONSE,
                        binds: vec![(oid("1.3.6.1.2.1.25.3.5.1.1.1"), Value::Int(3))],
                        ..req
                    };
                    let _ = agent.send_to(&m.encode(), from);
                }
            }
        });
        let mut a = Agent::new(
            addr,
            Version::V2c,
            b"public",
            Instant::now() + Duration::from_secs(20),
            &cancel,
        )
        .unwrap();
        let w = a.walk(&col(&HR_PRINTER, 1), 50).unwrap();
        assert_eq!(w.rows.len(), 1);
    }

    #[test]
    fn the_sample_is_fictional() {
        let ip: IpAddr = "192.0.2.40".parse().unwrap();
        let out = Outcome::Read(Box::new(sample(ip)));
        let t = out.report();
        assert!(t.starts_with("SNMP read of 192.0.2.40 (v2c)"), "{t}");
        assert!(t.contains("Serial: EXAMPLE0123"), "{t}");
        assert!(t.contains("Supply: Black Cartridge HP 58A: 12%"), "{t}");
    }

    #[test]
    fn a_late_answer_to_the_first_try_still_counts() {
        // The first GET's answer comes after the retry went out, and the
        // retry is never answered: the first answer is the read.
        let (addr, _) = serve(Fake {
            mib: printer_mib(),
            community: b"public",
            version: 1,
            late_first: true,
            ..Fake::default()
        });
        let out = read_at(addr, b"public", &quiet(), Duration::from_secs(30));
        let Outcome::Read(r) = &out else {
            panic!("{out:?}")
        };
        assert_eq!(r.version, Version::V2c);
        assert_eq!(r.system.name.as_deref(), Some("PRN-EXAMPLE"));
    }

    #[test]
    fn a_walk_at_the_time_limit_is_an_error_not_an_empty_table() {
        let (addr, seen) = serve(Fake {
            mib: printer_mib(),
            community: b"public",
            version: 1,
            ..Fake::default()
        });
        let cancel = quiet();
        // The read's time is up before each part starts.
        for step in [chassis as Step, printer, lldp] {
            let mut a = Agent::new(addr, Version::V2c, b"public", Instant::now(), &cancel).unwrap();
            let mut r = sample(addr.ip());
            r.chassis = None;
            r.printer = None;
            r.lldp.clear();
            assert_eq!(step(&mut a, &mut r), Err(Fail::Late));
            assert_eq!((r.chassis, r.printer, r.lldp.len()), (None, None, 0));
        }
        assert_eq!(seen.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_read_cut_short_says_what_wasnt_read() {
        // It answers the system group, then nothing more.
        let (addr, _) = serve(Fake {
            mib: printer_mib(),
            community: b"public",
            version: 1,
            answer_only: 1,
            ..Fake::default()
        });
        let out = read_at(addr, b"public", &quiet(), Duration::from_secs(30));
        let Outcome::Read(r) = &out else {
            panic!("{out:?}")
        };
        assert_eq!(r.system.name.as_deref(), Some("PRN-EXAMPLE"));
        assert_eq!(
            r.notes,
            ["It stopped answering while ENTITY-MIB was read. Not read: the printer MIB, LLDP."]
        );
        assert!(out.report().contains("Note: It stopped answering"));
    }

    #[test]
    fn a_printer_cut_short_keeps_its_supplies_and_says_so() {
        // The system group (1 request), ENTITY-MIB (2 GETNEXTs to the
        // chassis, 1 GET), the printer's status (1), errors (1) and the two
        // supplies' names (3): then quiet, before their capacities.
        let (addr, _) = serve(Fake {
            mib: printer_mib(),
            community: b"public",
            version: 1,
            answer_only: 9,
            ..Fake::default()
        });
        let out = read_at(addr, b"public", &quiet(), Duration::from_secs(30));
        let Outcome::Read(r) = &out else {
            panic!("{out:?}")
        };
        assert!(r.chassis.is_some());
        assert_eq!(
            r.notes,
            ["It stopped answering while the printer MIB was read. Not read: LLDP."]
        );
        assert!(
            out.report()
                .contains("Note: It stopped answering while the printer MIB was read."),
            "{}",
            out.report()
        );
        let p = r.printer.clone().unwrap();
        assert_eq!(p.errors, ["low toner", "jammed"]);
        assert_eq!(p.status, Some(3));
        let names: Vec<&str> = p.supplies.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Black Toner", "Drum"]);
        assert!(p.supplies.iter().all(|s| s.left() == "unknown"));
    }

    #[test]
    fn the_chassis_walk_stops_at_the_first_chassis() {
        let b = |s: &str| Value::Bytes(s.as_bytes().to_vec());
        let mut mib = BTreeMap::new();
        // The chassis first, then a big stack's modules and ports.
        mib.insert(oid("1.3.6.1.2.1.47.1.1.1.1.5.1"), Value::Int(3));
        for i in 2..200u32 {
            mib.insert(
                oid(&format!("1.3.6.1.2.1.47.1.1.1.1.5.{i}")),
                Value::Int(10),
            );
        }
        mib.insert(oid("1.3.6.1.2.1.47.1.1.1.1.9.1"), b("FW 2.4.1"));
        mib.insert(oid("1.3.6.1.2.1.47.1.1.1.1.10.1"), b("OS 7.0"));
        mib.insert(oid("1.3.6.1.2.1.47.1.1.1.1.13.1"), b("EX-SWITCH-48"));
        let (addr, seen) = serve(Fake {
            mib: mib.clone(),
            community: b"public",
            version: 1,
            ..Fake::default()
        });
        let cancel = quiet();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut a = Agent::new(addr, Version::V2c, b"public", deadline, &cancel).unwrap();
        let mut r = sample(addr.ip());
        r.chassis = None;
        r.notes.clear();
        chassis(&mut a, &mut r).unwrap();
        // One GETNEXT to find it, one GET for its values.
        assert_eq!(seen.load(Ordering::Relaxed), 2);
        let c = r.chassis.unwrap();
        assert_eq!(c.model.as_deref(), Some("EX-SWITCH-48"));
        // The firmware revision, and no software beside it.
        assert_eq!(c.firmware.as_deref(), Some("FW 2.4.1"));
        assert_eq!(c.software, None);
        assert!(r.notes.is_empty());

        // No firmware revision: the software revision, labelled as such.
        mib.remove(&oid("1.3.6.1.2.1.47.1.1.1.1.9.1"));
        let (addr, _) = serve(Fake {
            mib,
            community: b"public",
            version: 1,
            ..Fake::default()
        });
        let mut a = Agent::new(addr, Version::V2c, b"public", deadline, &cancel).unwrap();
        r.chassis = None;
        chassis(&mut a, &mut r).unwrap();
        let c = r.chassis.clone().unwrap();
        assert_eq!((c.firmware, c.software.as_deref()), (None, Some("OS 7.0")));
        let out = Outcome::Read(Box::new(r));
        assert!(
            out.report().contains("Software: OS 7.0"),
            "{}",
            out.report()
        );
    }

    #[test]
    fn no_chassis_in_the_capped_walk_is_said() {
        let mut mib = BTreeMap::new();
        for i in 1..200u32 {
            mib.insert(
                oid(&format!("1.3.6.1.2.1.47.1.1.1.1.5.{i}")),
                Value::Int(10),
            );
        }
        let (addr, _) = serve(Fake {
            mib,
            community: b"public",
            version: 1,
            ..Fake::default()
        });
        let cancel = quiet();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut a = Agent::new(addr, Version::V2c, b"public", deadline, &cancel).unwrap();
        let mut r = sample(addr.ip());
        r.chassis = None;
        r.notes.clear();
        chassis(&mut a, &mut r).unwrap();
        assert_eq!(r.chassis, None);
        assert_eq!(r.notes, ["ENTITY-MIB: no chassis in its first 64 entries."]);
    }

    #[test]
    fn huge_supply_levels_dont_overflow() {
        let s = |level, max| Supply {
            name: "Toner".into(),
            level,
            max,
        };
        assert_eq!(s(Some(i64::MAX), Some(i64::MAX)).left(), "100%");
        assert_eq!(s(Some(i64::MAX / 2), Some(i64::MAX)).left(), "50%");
        assert_eq!(s(Some(i64::MAX), Some(1)).left(), "100%");
    }

    #[test]
    fn agent_software_isnt_called_the_maker() {
        for (id, says) in [
            ("1.3.6.1.4.1.311.1.1.3.1.2", "Microsoft's SNMP service"),
            ("1.3.6.1.4.1.6876.4.1", "VMware"),
            ("1.3.6.1.4.1.8072.3.2.10", "Net-SNMP"),
            ("1.3.6.1.4.1.4413.1.1", "Broadcom FASTPATH"),
        ] {
            let Some(Enterprise::Software(w)) = enterprise(id) else {
                panic!("{id}")
            };
            assert!(w.starts_with(says) && w.contains("not the maker"), "{w}");
            let mut r = sample("192.0.2.41".parse().unwrap());
            r.system.object_id = Some(id.into());
            let out = Outcome::Read(Box::new(r));
            assert!(out.rows().iter().all(|(_, f, _)| f != "maker"));
            assert!(out.report().contains(&format!("Agent software: {says}")));
        }
    }

    #[test]
    fn the_contact_shows_on_screen_and_is_never_logged() {
        let out = Outcome::Read(Box::new(sample("192.0.2.40".parse().unwrap())));
        let on_screen = out.screen_rows();
        assert!(
            on_screen
                .iter()
                .any(|(s, f, v)| *s == "system" && f == "contact" && v == "IT desk, ext. 100")
        );
        assert!(
            out.rows()
                .iter()
                .all(|(_, f, v)| f != "contact" && !v.contains("IT desk"))
        );
        assert!(!out.report().contains("IT desk"));
        // Only the contact differs.
        assert_eq!(on_screen.len(), out.rows().len() + 1);
    }

    #[test]
    fn a_contact_alone_isnt_called_none_of_the_system_group() {
        let mut r = sample("192.0.2.42".parse().unwrap());
        r.system = System {
            contact: Some("Example admin".into()),
            ..System::default()
        };
        r.chassis = None;
        let text = Outcome::Read(Box::new(r.clone())).report();
        assert!(
            text.contains("only its contact") && !text.contains("Example admin"),
            "{text}"
        );
        r.system.contact = None;
        let text = Outcome::Read(Box::new(r)).report();
        assert!(text.contains("none of the system group"), "{text}");
    }

    #[test]
    fn long_text_is_cut_at_the_shared_limit() {
        assert_eq!(text(&[b'a'; 600]).chars().count(), 255);
    }
}
