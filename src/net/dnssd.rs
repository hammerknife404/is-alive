//! DNS-SD (Bonjour) browse: what printers and other devices on the
//! scanner's own segment say they are. One multicast DNS question packet
//! goes to 224.0.0.251:5353 asking for the list of service types and for
//! the printer types, with the unicast-response (QU) bit set. It is sent
//! from an ordinary ephemeral port, so devices answer straight back to it
//! (RFC 6762, 5.4 and 6.7); nothing binds or listens on 5353, so it never
//! competes with the computer's own Bonjour or Avahi.
//!
//! Multicast DNS is link-local: routers don't pass it on, so only devices
//! on the scanner's own segment answer. A routed network's printers say
//! nothing here, which is not a sign there are none.
//!
//! What comes back is the device's own claim: its service types and, for
//! a printer, the make and model in its TXT record (`ty=`, `product=`),
//! its queue (`rp=`) and the page languages it takes (`pdl=`). Instance
//! names ("Front office printer", "Ana's MacBook") are not kept. Nothing
//! is ever sent to a printer's own ports (no PJL on 9100).
//!
//! A device that answers is a device: the sweep lists one that answered
//! here but not ping or ARP, when it's inside the swept range and on the
//! scanner's own segment.

use super::dnsq;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

/// The multicast DNS group and port.
pub const GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 251), 5353);

/// What's asked: every service type a device offers, the printer types
/// (IPP, IPP over TLS, raw "JetDirect" printing, LPD), whose TXT records
/// carry the model, and file shares and web pages, which bring in NASes,
/// computers and gear that don't list their types.
pub const ASKED: &[&str] = &[
    "_services._dns-sd._udp.local",
    "_ipp._tcp.local",
    "_ipps._tcp.local",
    "_pdl-datastream._tcp.local",
    "_printer._tcp.local",
    "_smb._tcp.local",
    "_http._tcp.local",
];

/// How long devices get to answer. Responders wait up to half a second
/// before answering a browse, longer when the segment is busy.
pub const WAIT: Duration = Duration::from_millis(2500);

const SERVICES: &str = "_services._dns-sd._udp.local";

/// Most answers read, and how long past `WAIT` the already-queued ones
/// may take: a device that keeps sending can't hold the sweep.
const MAX_ANSWERS: usize = 1024;
const DRAIN: Duration = Duration::from_millis(250);

/// Kept per device, so a chatty one can't grow a report.
const MAX_SERVICES: usize = 24;
const MAX_QUEUES: usize = 8;
const MAX_LANGUAGES: usize = 16;

/// What one device said over DNS-SD.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// Service types it offers, without ".local" ("_ipp._tcp"), sorted.
    pub services: Vec<String>,
    /// The printer's make and model (`ty=` in an IPP or LPD TXT record).
    pub ty: Option<String>,
    /// The PostScript product name (`product=`), without its brackets.
    pub product: Option<String>,
    /// Its print queues (`rp=`, "ipp/print"), sorted: a printer has one or
    /// two, a print server one per printer.
    pub rp: Vec<String>,
    /// The page languages its printer services take (`pdl=`, MIME types
    /// such as "application/pdf"), sorted.
    pub pdl: Vec<String>,
}

impl Found {
    /// Its model, as best said: `ty`, else `product`.
    pub fn model(&self) -> Option<&str> {
        self.ty.as_deref().or(self.product.as_deref())
    }
}

/// The browse: one packet, every type asked for, each question with the
/// QU bit (class IN | 0x8000).
pub fn query(id: u16) -> Vec<u8> {
    let mut q = id.to_be_bytes().to_vec();
    q.extend_from_slice(&[0, 0]);
    q.extend_from_slice(&(ASKED.len() as u16).to_be_bytes());
    q.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    for name in ASKED {
        // The names are fixed and valid.
        let _ = dnsq::put_name(&mut q, name);
        q.extend_from_slice(&dnsq::PTR.to_be_bytes());
        q.extend_from_slice(&0x8001u16.to_be_bytes());
    }
    q
}

/// A browse on its way: the socket answers come back to.
pub struct Browse {
    sock: UdpSocket,
    sent: Instant,
}

/// Send the browse to `to` (the multicast group; a test's own responder),
/// out of the card with address `via` when given.
pub fn send(via: Option<Ipv4Addr>, to: SocketAddrV4) -> io::Result<Browse> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)).into())?;
    if to.ip().is_multicast() {
        // RFC 6762, 11: multicast DNS goes out with TTL 255.
        s.set_multicast_ttl_v4(255)?;
        if let Some(v) = via {
            s.set_multicast_if_v4(&v)?;
        }
    }
    // Answers wait here while the ping phase runs: room for a busy segment
    // (Windows' default buffer is small). Best effort.
    let _ = s.set_recv_buffer_size(256 * 1024);
    let sock: UdpSocket = s.into();
    sock.send_to(&query(super::names::id()), to)?;
    Ok(Browse {
        sock,
        sent: Instant::now(),
    })
}

impl Browse {
    /// Every answer that came within `wait` of sending (those that came
    /// while the caller was busy are still in the socket), by device. A
    /// record about another host is moved to it only when that host is on
    /// `lan` (the scanner's own segment: address and prefix length).
    ///
    /// Also returns the addresses that answered for themselves: the UDP
    /// source of an answer with a record of its own. Only those are
    /// devices heard from; an address named in another device's A record
    /// (a sleep proxy's, or a forged list of hundreds) may label a host
    /// the sweep found some other way, never add one.
    pub fn collect(
        self,
        wait: Duration,
        lan: Option<(Ipv4Addr, u32)>,
    ) -> (HashMap<Ipv4Addr, Found>, HashSet<Ipv4Addr>) {
        let mut out: HashMap<Ipv4Addr, Acc> = HashMap::new();
        let mut answered = HashSet::new();
        gather(&self.sock, self.sent, wait, |r, from| {
            if read(r, from, lan, &mut out) {
                answered.insert(from);
            }
        });
        let found = out.into_iter().map(|(ip, a)| (ip, a.done())).collect();
        (found, answered)
    }
}

/// Read what comes back to `sock` within `wait` of `sent`, handing each
/// datagram and its sender to `f`. A hostile segment can't hold the
/// sweep: at most `MAX_ANSWERS` are read, and past `wait` only what is
/// already queued, for `DRAIN` at most. Shared with the SSDP search.
pub(super) fn gather(
    sock: &UdpSocket,
    sent: Instant,
    wait: Duration,
    mut f: impl FnMut(&[u8], Ipv4Addr),
) {
    let mut buf = vec![0u8; 9000];
    let end = sent + wait;
    // Hard stop, whatever keeps arriving.
    let stop = Instant::now().max(end) + DRAIN;
    for _ in 0..MAX_ANSWERS {
        if Instant::now() >= stop {
            break;
        }
        let left = end.saturating_duration_since(Instant::now());
        // Past the end: only what's already waiting.
        let r = if left.is_zero() {
            sock.set_nonblocking(true)
                .and_then(|_| sock.recv_from(&mut buf))
        } else {
            sock.set_read_timeout(Some(left))
                .and_then(|_| sock.recv_from(&mut buf))
        };
        match r {
            Ok((n, SocketAddr::V4(from))) => f(&buf[..n], *from.ip()),
            Ok(_) => {}
            Err(_) if left.is_zero() => break,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            // ICMP port unreachable and the like (Windows reports it on
            // the next read): keep listening, without spinning.
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

#[derive(Default)]
struct Acc {
    services: BTreeSet<String>,
    /// Every model each of its printer services gave.
    ty: BTreeSet<String>,
    product: BTreeSet<String>,
    rp: BTreeSet<String>,
    pdl: BTreeSet<String>,
}

/// The one value given, or None when there were none or they disagree
/// (a computer or print server sharing several printers).
fn only(v: BTreeSet<String>) -> Option<String> {
    (v.len() == 1).then(|| v.into_iter().next()).flatten()
}

impl Acc {
    fn done(self) -> Found {
        Found {
            services: self.services.into_iter().take(MAX_SERVICES).collect(),
            ty: only(self.ty),
            product: only(self.product),
            rp: self.rp.into_iter().take(MAX_QUEUES).collect(),
            pdl: self.pdl.into_iter().take(MAX_LANGUAGES).collect(),
        }
    }
}

/// "_ipp._tcp.local" → "_ipp._tcp"; None for anything that isn't a
/// service type.
fn service_type(name: &str) -> Option<String> {
    let t = name.strip_suffix(".local")?;
    let mut parts = t.split('.');
    let (a, b) = (parts.next()?, parts.next()?);
    (parts.next().is_none() && a.starts_with('_') && a.len() > 1 && (b == "_tcp" || b == "_udp"))
        .then(|| format!("{a}.{b}"))
}

/// The type an instance name ends in: "x._ipp._tcp.local" → "_ipp._tcp".
fn instance_type(name: &str) -> Option<String> {
    let t = name.strip_suffix(".local")?;
    let mut it = t.rsplitn(3, '.');
    let (proto, svc) = (it.next()?, it.next()?);
    it.next()?;
    service_type(&format!("{svc}.{proto}.local"))
}

/// A TXT record's `key=value` strings.
fn txt_pairs(rd: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut k = 0;
    while k < rd.len() {
        let n = rd[k] as usize;
        let Some(s) = rd.get(k + 1..k + 1 + n) else {
            break;
        };
        let s = String::from_utf8_lossy(s);
        if let Some((key, v)) = s.split_once('=') {
            out.push((key.to_ascii_lowercase(), v.to_string()));
        }
        k += 1 + n;
    }
    out
}

/// A model string off the wire: cleaned, brackets gone, kept short.
fn model(v: &str) -> Option<String> {
    let v = v
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    let v: String = super::sanitize(v).chars().take(80).collect();
    (!v.is_empty() && v != "Unknown").then_some(v)
}

/// A queue name off the wire ("ipp/print"): cleaned and kept short.
fn queue(v: &str) -> Option<String> {
    let v: String = super::sanitize(v.trim()).chars().take(80).collect();
    (!v.is_empty()).then_some(v)
}

/// The languages in a `pdl=` list: each cleaned, kept short.
fn languages(v: &str) -> impl Iterator<Item = String> + '_ {
    v.split(',')
        .map(|l| {
            super::sanitize(l.trim())
                .chars()
                .take(60)
                .collect::<String>()
        })
        .filter(|l| !l.is_empty())
        .take(MAX_LANGUAGES)
}

/// Fold one answer from `from` into `out`. A record about another host
/// (a sleep proxy answering for a sleeping printer) goes to that host's
/// address when the answer gives it and it's on `lan` (any, without one).
/// True when the answer gave `from` a record of its own.
fn read(
    r: &[u8],
    from: Ipv4Addr,
    lan: Option<(Ipv4Addr, u32)>,
    out: &mut HashMap<Ipv4Addr, Acc>,
) -> bool {
    let on_lan = |a: Ipv4Addr| {
        lan.is_none_or(|(me, bits)| {
            let mask = u32::MAX.checked_shl(32 - bits.min(32)).unwrap_or(0);
            u32::from(me) & mask == u32::from(a) & mask
        })
    };
    let Ok(rrs) = dnsq::records(r) else {
        return false;
    };
    let mut own = false;
    // Instance → its host's name; host name → address.
    let mut target: HashMap<&str, &str> = HashMap::new();
    let mut addr: HashMap<&str, Ipv4Addr> = HashMap::new();
    for (rr, _) in &rrs {
        match &rr.data {
            dnsq::Data::Srv { target: t, .. } => {
                target.insert(&rr.name, t);
            }
            dnsq::Data::A(a) if rr.rtype == dnsq::A => {
                addr.entry(&rr.name).or_insert(*a);
            }
            _ => {}
        }
    }
    let owner = |instance: &str| -> Ipv4Addr {
        target
            .get(instance)
            .and_then(|t| addr.get(t))
            .copied()
            .filter(|&a| !a.is_unspecified() && !a.is_loopback() && on_lan(a))
            .unwrap_or(from)
    };
    for (rr, rd) in &rrs {
        match rr.rtype {
            dnsq::PTR if rr.name == SERVICES => {
                if let dnsq::Data::Name(n) = &rr.data
                    && let Some(t) = service_type(n)
                {
                    out.entry(from).or_default().services.insert(t);
                    own = true;
                }
            }
            dnsq::PTR => {
                if let (Some(t), dnsq::Data::Name(inst)) = (service_type(&rr.name), &rr.data) {
                    let at = owner(inst);
                    own |= at == from;
                    out.entry(at).or_default().services.insert(t);
                }
            }
            dnsq::SRV | dnsq::TXT => {
                let Some(t) = instance_type(&rr.name) else {
                    continue;
                };
                let at = owner(&rr.name);
                own |= at == from;
                let acc = out.entry(at).or_default();
                acc.services.insert(t);
                if rr.rtype == dnsq::TXT {
                    for (k, v) in txt_pairs(rd) {
                        match k.as_str() {
                            "ty" => acc.ty.extend(model(&v)),
                            "product" => acc.product.extend(model(&v)),
                            "rp" if acc.rp.len() < MAX_QUEUES => acc.rp.extend(queue(&v)),
                            "pdl" if acc.pdl.len() < MAX_LANGUAGES => acc.pdl.extend(languages(&v)),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    own
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A response as a printer sends it: its service types, and its IPP
    /// instance with SRV, TXT and A records.
    fn answer(id: u16, host_ip: [u8; 4]) -> Vec<u8> {
        fn name(r: &mut Vec<u8>, n: &str) {
            dnsq::put_name(r, n).unwrap();
        }
        fn rr(r: &mut Vec<u8>, n: &str, t: u16, data: &[u8]) {
            name(r, n);
            r.extend_from_slice(&t.to_be_bytes());
            r.extend_from_slice(&[0x80, 0x01, 0, 0, 0, 10]);
            r.extend_from_slice(&(data.len() as u16).to_be_bytes());
            r.extend_from_slice(data);
        }
        let mut r = id.to_be_bytes().to_vec();
        r.extend_from_slice(&[0x84, 0x00, 0, 0, 0, 3, 0, 0, 0, 3]);
        let mut n = Vec::new();
        name(&mut n, "_ipp._tcp.local");
        rr(&mut r, SERVICES, dnsq::PTR, &n);
        let mut n = Vec::new();
        name(&mut n, "_pdl-datastream._tcp.local");
        rr(&mut r, SERVICES, dnsq::PTR, &n);
        let inst = "Front office._ipp._tcp.local";
        let mut n = Vec::new();
        name(&mut n, inst);
        rr(&mut r, "_ipp._tcp.local", dnsq::PTR, &n);
        let mut srv = vec![0, 0, 0, 0, 0x02, 0x77];
        name(&mut srv, "npi0a1b2c.local");
        rr(&mut r, inst, dnsq::SRV, &srv);
        let mut txt = Vec::new();
        for s in [
            "txtvers=1",
            "ty=HP LaserJet Pro M404\u{202e}",
            "product=(HP LaserJet Pro M404)",
            "rp=ipp/print",
            "pdl=application/pdf, image/urf,application/vnd.hp-PCL,",
            "note=",
        ] {
            txt.push(s.len() as u8);
            txt.extend_from_slice(s.as_bytes());
        }
        rr(&mut r, inst, dnsq::TXT, &txt);
        rr(&mut r, "npi0a1b2c.local", dnsq::A, &host_ip);
        r
    }

    #[test]
    fn the_browse_asks_every_type_with_the_qu_bit() {
        let q = query(0x1234);
        assert_eq!(&q[..2], &[0x12, 0x34]);
        assert_eq!(u16::from_be_bytes([q[4], q[5]]) as usize, ASKED.len());
        // Every question: PTR, class IN with the unicast-response bit.
        let mut i = 12;
        for _ in ASKED {
            while q[i] != 0 {
                i += 1 + q[i] as usize;
            }
            assert_eq!(&q[i + 1..i + 5], &[0, 12, 0x80, 0x01]);
            i += 5;
        }
        assert_eq!(i, q.len());
        assert!(!ASKED.iter().any(|a| a.contains("9100")));
    }

    #[test]
    fn a_printer_says_its_services_and_model() {
        let from = Ipv4Addr::new(192, 0, 2, 40);
        let mut out = HashMap::new();
        read(&answer(7, [192, 0, 2, 40]), from, None, &mut out);
        let f = out.remove(&from).unwrap().done();
        assert_eq!(f.services, ["_ipp._tcp", "_pdl-datastream._tcp"]);
        // Cleaned: no direction-changing character from the wire.
        assert_eq!(f.ty.as_deref(), Some("HP LaserJet Pro M404"));
        assert_eq!(f.product.as_deref(), Some("HP LaserJet Pro M404"));
        assert_eq!(f.model(), Some("HP LaserJet Pro M404"));
        assert_eq!(f.rp, ["ipp/print"]);
        assert_eq!(
            f.pdl,
            ["application/pdf", "application/vnd.hp-PCL", "image/urf"]
        );
    }

    #[test]
    fn file_shares_and_web_pages_are_asked_for_too() {
        for t in ["_smb._tcp.local", "_http._tcp.local", "_ipp._tcp.local"] {
            assert!(ASKED.contains(&t), "{t}");
        }
    }

    /// Whatever a device on the segment sends, reading it can't panic, and
    /// what's kept stays small.
    #[test]
    fn hostile_answers_cant_panic() {
        let from = Ipv4Addr::new(192, 0, 2, 40);
        let lan = Some((from, 24));
        crate::hostile::each(&answer(7, [192, 0, 2, 40]), |r| {
            let mut out = HashMap::new();
            read(r, from, lan, &mut out);
            for (_, a) in out {
                let f = a.done();
                assert!(f.services.len() <= MAX_SERVICES);
                assert!(f.rp.len() <= MAX_QUEUES && f.pdl.len() <= MAX_LANGUAGES);
            }
        });
        // A direction-changing character in a queue is cleaned away, and a
        // language list far too long is cut short.
        let mut out = HashMap::new();
        let mut a = answer(7, [192, 0, 2, 40]);
        let at = a.windows(12).position(|w| w == b"rp=ipp/print").unwrap();
        a[at + 3..at + 12].copy_from_slice(b"\xe2\x80\xaexxxxxx");
        read(&a, from, lan, &mut out);
        let f = out.remove(&from).unwrap().done();
        assert!(f.rp.iter().all(|q| !q.contains('\u{202e}')));
        assert!(languages(&"x,".repeat(500)).count() <= MAX_LANGUAGES);
        assert!(languages("a".repeat(500).as_str()).all(|l| l.len() <= 60));
    }

    #[test]
    fn a_sleep_proxy_answer_goes_to_the_printer_it_speaks_for() {
        let proxy = Ipv4Addr::new(192, 0, 2, 9);
        let mut out = HashMap::new();
        let lan = Some((proxy, 24));
        read(&answer(7, [192, 0, 2, 41]), proxy, lan, &mut out);
        let printer = out.remove(&Ipv4Addr::new(192, 0, 2, 41)).unwrap().done();
        assert_eq!(printer.ty.as_deref(), Some("HP LaserJet Pro M404"));
        // The proxy's own list of types stays with the proxy.
        assert!(out.contains_key(&proxy));
        // An A record off the scanner's segment isn't believed: it stays
        // with the device that sent it.
        let mut out = HashMap::new();
        read(&answer(7, [10, 9, 9, 41]), proxy, lan, &mut out);
        assert!(!out.contains_key(&Ipv4Addr::new(10, 9, 9, 41)));
        assert!(out[&proxy].ty.contains("HP LaserJet Pro M404"));
    }

    #[test]
    fn a_host_sharing_different_printers_gets_no_model() {
        let from = Ipv4Addr::new(192, 0, 2, 7);
        let mut out = HashMap::new();
        read(&answer(1, [192, 0, 2, 7]), from, None, &mut out);
        let mut other = answer(2, [192, 0, 2, 7]);
        // Same shape, another model: a print server's second queue.
        let at = other
            .windows(12)
            .position(|w| w == b"LaserJet Pro")
            .unwrap();
        other[at..at + 12].copy_from_slice(b"OfficeJet 80");
        read(&other, from, None, &mut out);
        let f = out.remove(&from).unwrap().done();
        assert_eq!(f.ty, None);
        assert!(f.services.contains(&"_ipp._tcp".to_string()));
    }

    #[test]
    fn junk_and_questions_are_ignored() {
        let mut out = HashMap::new();
        let from = Ipv4Addr::new(192, 0, 2, 1);
        read(&[0u8; 5], from, None, &mut out);
        read(&query(1), from, None, &mut out);
        let mut cut = answer(1, [192, 0, 2, 1]);
        cut.truncate(cut.len() - 3);
        read(&cut, from, None, &mut out);
        assert!(out.values().all(|a| a.ty.is_empty()));
        assert_eq!(
            instance_type("a.b._ipp._tcp.local").as_deref(),
            Some("_ipp._tcp")
        );
        assert_eq!(
            service_type("_ipp._tcp.local").as_deref(),
            Some("_ipp._tcp")
        );
        assert_eq!(service_type("printer.local"), None);
        assert_eq!(model("(Unknown)"), None);
    }

    /// A device that never stops sending can't hold the sweep: collect
    /// returns soon after its wait.
    #[test]
    fn a_flood_of_answers_is_cut_off() {
        let flood = UdpSocket::bind("127.0.0.1:0").unwrap();
        let at = match flood.local_addr().unwrap() {
            SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let b = send(None, at).unwrap();
        let mut buf = [0u8; 1500];
        let (_, back) = flood.recv_from(&mut buf).unwrap();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let d = done.clone();
        let t = std::thread::spawn(move || {
            let p = answer(1, [127, 0, 0, 1]);
            while !d.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = flood.send_to(&p, back);
            }
        });
        let t0 = Instant::now();
        let (found, _) = b.collect(Duration::from_millis(200), None);
        let took = t0.elapsed();
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        t.join().unwrap();
        assert!(took < Duration::from_secs(2), "took {took:?}");
        assert!(found.contains_key(&Ipv4Addr::LOCALHOST));
    }

    /// Over the wire on loopback: the browse goes out from an ephemeral
    /// port (never 5353) and the answer sent back to that port is read.
    #[test]
    fn a_browse_reads_the_answers_sent_back_to_its_port() {
        let responder = UdpSocket::bind("127.0.0.1:0").unwrap();
        let at = match responder.local_addr().unwrap() {
            SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let b = send(None, at).unwrap();
        let mut buf = [0u8; 1500];
        responder
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let (n, back) = responder.recv_from(&mut buf).unwrap();
        assert_ne!(back.port(), 5353, "asked from an ordinary port");
        assert_eq!(u16::from_be_bytes([buf[4], buf[5]]) as usize, ASKED.len());
        let _ = n;
        responder.send_to(&answer(1, [127, 0, 0, 1]), back).unwrap();
        let (found, answered) = b.collect(Duration::from_millis(500), None);
        let f = &found[&Ipv4Addr::LOCALHOST];
        assert_eq!(f.model(), Some("HP LaserJet Pro M404"));
        assert!(answered.contains(&Ipv4Addr::LOCALHOST));
    }

    /// One answer naming the addresses of many instances' hosts: the
    /// sender answered for none of its own, so it adds no device; the
    /// addresses named may only label hosts found another way.
    #[test]
    fn a_packet_naming_many_owners_answers_for_none_of_them() {
        fn name(r: &mut Vec<u8>, n: &str) {
            dnsq::put_name(r, n).unwrap();
        }
        fn rr(r: &mut Vec<u8>, n: &str, t: u16, data: &[u8]) {
            name(r, n);
            r.extend_from_slice(&t.to_be_bytes());
            r.extend_from_slice(&[0x80, 0x01, 0, 0, 0, 10]);
            r.extend_from_slice(&(data.len() as u16).to_be_bytes());
            r.extend_from_slice(data);
        }
        let n = 100u8;
        let mut r = vec![0, 1, 0x84, 0, 0, 0, 0, 2 * n, 0, 0, 0, 0];
        for i in 0..n {
            let inst = format!("p{i}._ipp._tcp.local");
            let host = format!("h{i}.local");
            let mut srv = vec![0, 0, 0, 0, 0x02, 0x77];
            name(&mut srv, &host);
            rr(&mut r, &inst, dnsq::SRV, &srv);
            rr(&mut r, &host, dnsq::A, &[192, 0, 2, 100 + i]);
        }
        let from = Ipv4Addr::new(192, 0, 2, 9);
        let mut out = HashMap::new();
        assert!(!read(&r, from, Some((from, 24)), &mut out));
        assert!(out.len() >= n as usize, "the owners may be labelled");
        assert!(!out.contains_key(&from));

        // Over the wire: nothing counts as having answered.
        let responder = UdpSocket::bind("127.0.0.1:0").unwrap();
        let at = match responder.local_addr().unwrap() {
            SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let b = send(None, at).unwrap();
        let mut buf = [0u8; 1500];
        let (_, back) = responder.recv_from(&mut buf).unwrap();
        responder.send_to(&r, back).unwrap();
        let (_, answered) = b.collect(Duration::from_millis(300), None);
        assert!(answered.is_empty(), "{answered:?}");
    }
}
