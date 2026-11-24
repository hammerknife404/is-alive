//! SSDP (UPnP discovery): which devices on the scanner's own segment say
//! they're there. One M-SEARCH (`ST: ssdp:all`, `MX: 2`) goes to
//! 239.255.255.250:1900 from an ordinary ephemeral port, at the same
//! moment as the DNS-SD browse, and devices answer it with a unicast HTTP
//! reply straight back to that port (UPnP Device Architecture, 1.3.3).
//! Nothing binds or listens on 1900, so it never competes with the
//! computer's own SSDP service, and NOTIFY announcements aren't heard.
//!
//! SSDP is multicast on the local link (routers don't pass it on), so
//! only devices on the scanner's own segment answer.
//!
//! What's kept of a reply is the device's own claim: `SERVER` (its OS and
//! UPnP stack, "Linux/4.4 UPnP/1.0 Synology/DSM"), the `ST` search
//! targets it answered for (its device and service types), the UUID from
//! its `USN`, and `LOCATION` as text. The LOCATION URL is never fetched.

use super::dnssd;
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

/// The SSDP multicast group and port.
pub const GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);

/// Devices wait a random time up to MX (2 s) before answering.
pub const WAIT: Duration = Duration::from_millis(2500);

/// Kept per device, so a chatty one (a TV answers for every service it
/// has) can't grow a report.
const MAX_TYPES: usize = 24;
const MAX_UUIDS: usize = 4;
const MAX_LOCATIONS: usize = 4;

/// The search: every device and service type, answers within 2 s.
pub const SEARCH: &str = "M-SEARCH * HTTP/1.1\r\n\
HOST: 239.255.255.250:1900\r\n\
MAN: \"ssdp:discover\"\r\n\
MX: 2\r\n\
ST: ssdp:all\r\n\
\r\n";

/// What one device said over SSDP.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// Its `SERVER` header: OS, UPnP version and product. The first one
    /// given, when its replies disagree.
    pub server: Option<String>,
    /// The search targets it answered for (`upnp:rootdevice`,
    /// `urn:schemas-upnp-org:device:MediaRenderer:1`), sorted.
    pub types: Vec<String>,
    /// The UUIDs in its `USN` headers ("uuid:…"), sorted.
    pub uuids: Vec<String>,
    /// Its `LOCATION` URLs (its description document), as text only.
    pub locations: Vec<String>,
}

/// A search on its way: the socket answers come back to.
pub struct Search {
    sock: UdpSocket,
    sent: Instant,
}

/// Send the search to `to` (the multicast group; a test's own responder),
/// out of the card with address `via` when given.
pub fn send(via: Option<Ipv4Addr>, to: SocketAddrV4) -> io::Result<Search> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // An ephemeral port, never 1900.
    s.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)).into())?;
    if to.ip().is_multicast() {
        // UPnP Device Architecture 1.1: TTL 2 by default.
        s.set_multicast_ttl_v4(2)?;
        if let Some(v) = via {
            s.set_multicast_if_v4(&v)?;
        }
    }
    // Replies wait here while the ping phase runs. Best effort.
    let _ = s.set_recv_buffer_size(256 * 1024);
    let sock: UdpSocket = s.into();
    sock.send_to(SEARCH.as_bytes(), to)?;
    Ok(Search {
        sock,
        sent: Instant::now(),
    })
}

impl Search {
    /// Every reply that came within `wait` of sending, by the address it
    /// came from: the same hard deadline and answer cap as the DNS-SD
    /// browse.
    pub fn collect(self, wait: Duration) -> HashMap<Ipv4Addr, Found> {
        let mut out: HashMap<Ipv4Addr, Acc> = HashMap::new();
        dnssd::gather(&self.sock, self.sent, wait, |r, from| {
            if let Some(reply) = parse(r) {
                out.entry(from).or_default().add(reply);
            }
        });
        out.into_iter().map(|(ip, a)| (ip, a.done())).collect()
    }
}

/// One reply's headers that matter.
#[derive(Debug, Default, PartialEq)]
struct Reply {
    server: Option<String>,
    st: Option<String>,
    usn: Option<String>,
    location: Option<String>,
}

#[derive(Default)]
struct Acc {
    server: Option<String>,
    types: BTreeSet<String>,
    uuids: BTreeSet<String>,
    locations: BTreeSet<String>,
}

impl Acc {
    fn add(&mut self, r: Reply) {
        if self.server.is_none() {
            self.server = r.server;
        }
        if let Some(t) = r.st
            && self.types.len() < MAX_TYPES
        {
            self.types.insert(t);
        }
        if let Some(u) = r.usn.as_deref().and_then(uuid)
            && self.uuids.len() < MAX_UUIDS
        {
            self.uuids.insert(u);
        }
        if let Some(l) = r.location
            && self.locations.len() < MAX_LOCATIONS
        {
            self.locations.insert(l);
        }
    }

    fn done(self) -> Found {
        Found {
            server: self.server,
            types: self.types.into_iter().collect(),
            uuids: self.uuids.into_iter().collect(),
            locations: self.locations.into_iter().collect(),
        }
    }
}

/// A header value off the wire: cleaned, kept short, None when empty.
fn value(v: &str, max: usize) -> Option<String> {
    let v: String = super::sanitize(v.trim()).chars().take(max).collect();
    (!v.is_empty()).then_some(v)
}

/// "uuid:2f402f80-da50-11e1-9b23-00000000a1b2::upnp:rootdevice" → the
/// part before "::".
fn uuid(usn: &str) -> Option<String> {
    let u = usn.split("::").next()?.trim();
    // `get`, not slicing: byte 5 may be inside a character.
    (u.len() > 5 && u.get(..5)?.eq_ignore_ascii_case("uuid:")).then(|| value(u, 80))?
}

/// A search reply: "HTTP/1.1 200 OK" and its headers. None for anything
/// else (another computer's M-SEARCH, a NOTIFY, noise).
fn parse(r: &[u8]) -> Option<Reply> {
    let text = String::from_utf8_lossy(r);
    let mut lines = text.split('\n').map(|l| l.trim_end_matches('\r'));
    let status = lines.next()?;
    let mut words = status.split_ascii_whitespace();
    let (ver, code) = (words.next()?, words.next()?);
    if !ver.to_ascii_uppercase().starts_with("HTTP/1.") || code != "200" {
        return None;
    }
    let mut out = Reply::default();
    for line in lines.take(64) {
        if line.is_empty() {
            break;
        }
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let slot = match k.trim().to_ascii_uppercase().as_str() {
            "SERVER" => &mut out.server,
            "ST" => &mut out.st,
            "USN" => &mut out.usn,
            "LOCATION" => &mut out.location,
            _ => continue,
        };
        if slot.is_none() {
            *slot = value(v, 160);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reply as a NAS sends it.
    const REPLY: &str = "HTTP/1.1 200 OK\r\n\
CACHE-CONTROL: max-age=1800\r\n\
DATE: Tue, 24 Nov 2026 10:00:00 GMT\r\n\
EXT:\r\n\
LOCATION: http://192.0.2.60:5000/ssdp/desc-DSM-eth0.xml\r\n\
SERVER: Synology/DSM/192.0.2.60\r\n\
ST: upnp:rootdevice\r\n\
USN: uuid:73796e6f-6c6f-6779-0000-00000000a1b2::upnp:rootdevice\r\n\
\r\n";

    #[test]
    fn the_search_asks_for_everything_once() {
        assert!(SEARCH.starts_with("M-SEARCH * HTTP/1.1\r\n"));
        assert!(SEARCH.contains("\r\nST: ssdp:all\r\n"));
        assert!(SEARCH.contains("\r\nMX: 2\r\n"));
        assert!(SEARCH.contains("\r\nMAN: \"ssdp:discover\"\r\n"));
        assert!(SEARCH.ends_with("\r\n\r\n"));
        assert_eq!(GROUP.port(), 1900);
    }

    #[test]
    fn a_reply_gives_its_server_type_uuid_and_location() {
        let r = parse(REPLY.as_bytes()).unwrap();
        assert_eq!(r.server.as_deref(), Some("Synology/DSM/192.0.2.60"));
        assert_eq!(r.st.as_deref(), Some("upnp:rootdevice"));
        assert_eq!(
            r.location.as_deref(),
            Some("http://192.0.2.60:5000/ssdp/desc-DSM-eth0.xml")
        );
        let mut a = Acc::default();
        a.add(r);
        // Lower-case headers and bare line feeds read the same.
        let other = REPLY
            .replace("\r\n", "\n")
            .replace(
                "ST: upnp:rootdevice",
                "st: urn:schemas-upnp-org:device:Basic:1",
            )
            .replace("SERVER:", "server:");
        a.add(parse(other.as_bytes()).unwrap());
        let f = a.done();
        assert_eq!(
            f.types,
            ["upnp:rootdevice", "urn:schemas-upnp-org:device:Basic:1"]
        );
        assert_eq!(f.uuids, ["uuid:73796e6f-6c6f-6779-0000-00000000a1b2"]);
        assert_eq!(f.locations.len(), 1);
        assert_eq!(f.server.as_deref(), Some("Synology/DSM/192.0.2.60"));
    }

    #[test]
    fn requests_and_errors_are_not_replies() {
        assert_eq!(parse(SEARCH.as_bytes()), None);
        assert_eq!(
            parse(b"NOTIFY * HTTP/1.1\r\nNT: upnp:rootdevice\r\n\r\n"),
            None
        );
        assert_eq!(parse(b"HTTP/1.1 404 Not Found\r\n\r\n"), None);
        assert_eq!(parse(b""), None);
        assert_eq!(uuid("not-a-uuid"), None);
        assert_eq!(uuid("uuid:"), None);
        assert_eq!(uuid("uui\u{e9}:x"), None);
    }

    /// Whatever a device sends, reading it can't panic, and what's kept
    /// stays small and clean.
    #[test]
    fn hostile_replies_cant_panic() {
        crate::hostile::each(REPLY.as_bytes(), |r| {
            if let Some(r) = parse(r) {
                let mut a = Acc::default();
                a.add(r);
                a.done();
            }
        });
        crate::hostile::each_text(REPLY, |t| {
            let _ = parse(t.as_bytes());
        });
        let mut long = String::from("HTTP/1.1 200 OK\r\nSERVER: ");
        long += &"x\u{202e}".repeat(5000);
        long += "\r\n\r\n";
        let r = parse(long.as_bytes()).unwrap();
        let s = r.server.unwrap();
        assert!(s.chars().count() <= 160 && !s.contains('\u{202e}'));
        // A device answering for a thousand types keeps a few.
        let mut a = Acc::default();
        for i in 0..1000 {
            a.add(Reply {
                st: Some(format!("urn:x:{i}")),
                usn: Some(format!("uuid:{i}0000::x")),
                location: Some(format!("http://192.0.2.1/{i}")),
                ..Default::default()
            });
        }
        let f = a.done();
        assert!(f.types.len() <= MAX_TYPES);
        assert!(f.uuids.len() <= MAX_UUIDS && f.locations.len() <= MAX_LOCATIONS);
    }

    /// Over the wire on loopback: the search goes out from an ephemeral
    /// port (never 1900) and the reply sent back to that port is read.
    #[test]
    fn a_search_reads_the_replies_sent_back_to_its_port() {
        let responder = UdpSocket::bind("127.0.0.1:0").unwrap();
        let at = match responder.local_addr().unwrap() {
            SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let s = send(None, at).unwrap();
        let mut buf = [0u8; 1500];
        responder
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let (n, back) = responder.recv_from(&mut buf).unwrap();
        assert_ne!(back.port(), 1900, "asked from an ordinary port");
        assert_eq!(&buf[..n], SEARCH.as_bytes());
        responder.send_to(REPLY.as_bytes(), back).unwrap();
        let found = s.collect(Duration::from_millis(500));
        let f = &found[&Ipv4Addr::LOCALHOST];
        assert_eq!(f.server.as_deref(), Some("Synology/DSM/192.0.2.60"));
    }

    /// A device that never stops sending can't hold the sweep.
    #[test]
    fn a_flood_of_replies_is_cut_off() {
        let flood = UdpSocket::bind("127.0.0.1:0").unwrap();
        let at = match flood.local_addr().unwrap() {
            SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let s = send(None, at).unwrap();
        let mut buf = [0u8; 1500];
        let (_, back) = flood.recv_from(&mut buf).unwrap();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let d = done.clone();
        let t = std::thread::spawn(move || {
            while !d.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = flood.send_to(REPLY.as_bytes(), back);
            }
        });
        let t0 = Instant::now();
        let found = s.collect(Duration::from_millis(200));
        let took = t0.elapsed();
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        t.join().unwrap();
        assert!(took < Duration::from_secs(2), "took {took:?}");
        assert!(found.contains_key(&Ipv4Addr::LOCALHOST));
    }
}
