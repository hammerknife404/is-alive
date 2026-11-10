//! The names devices give for themselves, for the hosts DNS has no name
//! for (most small sites have no reverse zone): NetBIOS, which Windows,
//! Samba, NAS boxes and many printers answer, and multicast DNS, which
//! Apple devices, printers, cast devices and Linux with Avahi answer. Each
//! is one UDP packet to the device and one back, sent from an ordinary
//! port: no admin rights, nothing sent to anyone else.
//!
//! NetBIOS also gives the device's MAC (Windows reports its adapter's;
//! Samba sends zeros, which are left out), so a Windows host past a router,
//! where ARP can't reach, still gets a MAC and a vendor.
//!
//! Everything in an answer is the device's own claim: names are cleaned
//! like any other text off the network before they're shown.

use super::dnsq;
use super::mac::Mac;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub const NETBIOS_PORT: u16 = 137;
pub const MDNS_PORT: u16 = 5353;

/// NBSTAT: the node status query (`nbtstat -A`).
const NBSTAT: u16 = 0x21;

/// What a device says over NetBIOS.
#[derive(Clone, Debug, PartialEq)]
pub struct NetBios {
    /// Its computer name ("FRONTDESK-PC").
    pub name: String,
    /// Its workgroup or domain, when it says.
    pub group: Option<String>,
    /// Its adapter's MAC, when it gives a real one.
    pub mac: Option<Mac>,
}

/// Ask `to` (a device's port 137) for its node status.
pub fn netbios(to: SocketAddr, wait: Duration) -> Option<NetBios> {
    let id = id();
    let answer = exchange(to, &nbstat_query(id), wait, |r| {
        r.get(..2) == Some(&id.to_be_bytes())
    })?;
    parse_nbstat(&answer, id)
}

/// Ask `to` (a device's port 5353) for the name of `ip`, as a one-shot
/// multicast DNS query sent straight to it (RFC 6762, 5.1 and 6.7: it
/// answers to the port asked from).
pub fn mdns(ip: Ipv4Addr, to: SocketAddr, wait: Duration) -> Option<String> {
    let want = dnsq::reverse_name(IpAddr::V4(ip));
    let client = dnsq::Client {
        server: to,
        timeout: wait,
        rd: false,
    };
    let m = client.once(&want, dnsq::PTR).ok()?;
    m.answers
        .iter()
        .filter(|rr| rr.rtype == dnsq::PTR && rr.name == want)
        .find_map(|rr| match &rr.data {
            dnsq::Data::Name(n) => Some(n.trim_end_matches('.').to_string()),
            _ => None,
        })
        .filter(|n| !n.is_empty())
}

/// Send `q` to `to` and wait up to `wait` for a datagram from it that
/// `ours` accepts.
fn exchange(
    to: SocketAddr,
    q: &[u8],
    wait: Duration,
    ours: impl Fn(&[u8]) -> bool,
) -> Option<Vec<u8>> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.send_to(q, to).ok()?;
    let end = Instant::now() + wait;
    let mut buf = vec![0u8; 2048];
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        sock.set_read_timeout(Some(left)).ok()?;
        let (n, from) = sock.recv_from(&mut buf).ok()?;
        if from.ip() == to.ip() && ours(&buf[..n]) {
            buf.truncate(n);
            return Some(buf);
        }
    }
}

fn id() -> u16 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.finish() as u16
}

/// A node status query for "*", the name every NetBIOS node answers to.
fn nbstat_query(id: u16) -> Vec<u8> {
    let mut q = id.to_be_bytes().to_vec();
    // Flags 0 (a query), one question.
    q.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    // "*" padded to 16 bytes with zeros, each byte as two letters A-P.
    q.push(32);
    let mut raw = [0u8; 16];
    raw[0] = b'*';
    for b in raw {
        q.push(b'A' + (b >> 4));
        q.push(b'A' + (b & 0x0f));
    }
    q.push(0);
    q.extend_from_slice(&NBSTAT.to_be_bytes());
    q.extend_from_slice(&[0, 1]); // class IN
    q
}

/// The offset after a (possibly compressed) name at `i`.
fn skip_name(r: &[u8], mut i: usize) -> Option<usize> {
    for _ in 0..64 {
        let len = *r.get(i)? as usize;
        if len & 0xc0 == 0xc0 {
            return Some(i + 2);
        }
        if len == 0 {
            return Some(i + 1);
        }
        i += 1 + len;
    }
    None
}

fn u16_at(r: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*r.get(i)?, *r.get(i + 1)?]))
}

/// Read a node status answer: the computer name (the first unique name
/// with suffix 0x00), the workgroup (the first group name with it), and
/// the MAC that follows the names.
fn parse_nbstat(r: &[u8], id: u16) -> Option<NetBios> {
    if r.len() < 12 || r[..2] != id.to_be_bytes() || r[2] & 0x80 == 0 {
        return None;
    }
    let (qd, an) = (u16_at(r, 4)?, u16_at(r, 6)?);
    if an == 0 {
        return None;
    }
    let mut i = 12;
    for _ in 0..qd {
        i = skip_name(r, i)? + 4;
    }
    i = skip_name(r, i)?;
    if u16_at(r, i)? != NBSTAT {
        return None;
    }
    // Type, class, TTL, then the data's length.
    let len = u16_at(r, i + 8)? as usize;
    let data = r.get(i + 10..i + 10 + len)?;
    let count = *data.first()? as usize;
    let names = data.get(1..1 + 18 * count)?;
    let mut name = None;
    let mut group = None;
    for e in names.as_chunks::<18>().0 {
        let (suffix, flags) = (e[15], u16::from_be_bytes([e[16], e[17]]));
        if suffix != 0x00 {
            continue;
        }
        let text = super::sanitize(String::from_utf8_lossy(&e[..15]).trim_end());
        if text.is_empty() {
            continue;
        }
        if flags & 0x8000 != 0 {
            group.get_or_insert(text);
        } else {
            name.get_or_insert(text);
        }
    }
    let mac = data
        .get(1 + 18 * count..1 + 18 * count + 6)
        .map(|m| [m[0], m[1], m[2], m[3], m[4], m[5]])
        .filter(|m| m.iter().any(|&b| b != 0) && m.iter().any(|&b| b != 0xff));
    Some(NetBios {
        name: name?,
        group,
        mac,
    })
}

/// For tests: a node status answer as a Windows PC sends it.
#[cfg(test)]
fn nbstat_answer(id: u16, names: &[(&str, u8, bool)], mac: Mac) -> Vec<u8> {
    let mut r = id.to_be_bytes().to_vec();
    // A response, authoritative; no questions, one answer.
    r.extend_from_slice(&[0x84, 0x00, 0, 0, 0, 1, 0, 0, 0, 0]);
    let q = nbstat_query(0);
    r.extend_from_slice(&q[12..12 + 34]); // the name asked about
    r.extend_from_slice(&NBSTAT.to_be_bytes());
    r.extend_from_slice(&[0, 1, 0, 0, 0, 0]);
    let mut data = vec![names.len() as u8];
    for (n, suffix, group) in names {
        let mut padded = [b' '; 15];
        padded[..n.len()].copy_from_slice(n.as_bytes());
        data.extend_from_slice(&padded);
        data.push(*suffix);
        data.extend_from_slice(&(if *group { 0x8400u16 } else { 0x0400 }).to_be_bytes());
    }
    data.extend_from_slice(&mac);
    data.extend_from_slice(&[0; 40]); // the rest of the statistics
    r.extend_from_slice(&(data.len() as u16).to_be_bytes());
    r.extend_from_slice(&data);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    const PC: &[(&str, u8, bool)] = &[
        ("FRONTDESK-PC", 0x00, false),
        ("HARBORDENTAL", 0x00, true),
        ("FRONTDESK-PC", 0x20, false),
        ("HARBORDENTAL", 0x1e, true),
    ];
    const MAC: Mac = [0x00, 0x15, 0x5d, 0x01, 0x02, 0x03];

    #[test]
    fn the_query_asks_for_star_by_node_status() {
        let q = nbstat_query(0xbeef);
        assert_eq!(&q[..2], &[0xbe, 0xef]);
        // "*" then fifteen zeros, as nbtstat -A sends it.
        assert_eq!(&q[13..45], b"CKAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        assert_eq!(q.len(), 12 + 34 + 4);
        assert_eq!(&q[46..], &[0, 0x21, 0, 1]);
    }

    #[test]
    fn a_windows_answer_gives_name_workgroup_and_mac() {
        let r = nbstat_answer(7, PC, MAC);
        assert_eq!(
            parse_nbstat(&r, 7),
            Some(NetBios {
                name: "FRONTDESK-PC".into(),
                group: Some("HARBORDENTAL".into()),
                mac: Some(MAC),
            })
        );
        // Another query's answer, or a query, isn't ours.
        assert_eq!(parse_nbstat(&r, 8), None);
        assert_eq!(parse_nbstat(&nbstat_query(7), 7), None);
    }

    #[test]
    fn samba_zeros_and_odd_names_are_left_out() {
        // Samba's MAC is zeros; a group-only answer has no computer name.
        let r = nbstat_answer(1, &[("NAS01", 0x00, false)], [0; 6]);
        assert_eq!(parse_nbstat(&r, 1).unwrap().mac, None);
        let r = nbstat_answer(1, &[("WORKGROUP", 0x00, true)], MAC);
        assert_eq!(parse_nbstat(&r, 1), None);
        // Control characters in a name are taken out.
        let r = nbstat_answer(1, &[("EVIL\u{7}PC", 0x00, false)], MAC);
        assert_eq!(parse_nbstat(&r, 1).unwrap().name, "EVILPC");
    }

    #[test]
    fn short_or_broken_answers_are_nothing() {
        let r = nbstat_answer(3, PC, MAC);
        for cut in 0..r.len() - 40 {
            // Any answer cut short of its MAC is refused or has no MAC; none panics.
            if let Some(nb) = parse_nbstat(&r[..cut], 3) {
                assert_eq!(nb.mac, None);
            }
        }
        // Header 12, name 34, type, class and TTL 8: the data's length at
        // 54, its name count at 56.
        let mut lies = r.clone();
        lies[54] = 0xff; // a length far past the end
        assert_eq!(parse_nbstat(&lies, 3), None);
        let mut count = r;
        count[56] = 200; // more names than there are
        assert_eq!(parse_nbstat(&count, 3), None);
    }

    /// A device on this machine answering node status on a port of its own.
    #[test]
    fn netbios_over_the_wire() {
        let dev = UdpSocket::bind("127.0.0.1:0").unwrap();
        let at = dev.local_addr().unwrap();
        let t = thread::spawn(move || {
            let mut buf = [0u8; 512];
            let (n, from) = dev.recv_from(&mut buf).unwrap();
            let id = u16::from_be_bytes([buf[0], buf[1]]);
            assert_eq!(&buf[13..15], b"CK");
            // A stray datagram first: ignored.
            dev.send_to(&nbstat_answer(id ^ 1, PC, MAC), from).unwrap();
            dev.send_to(&nbstat_answer(id, PC, MAC), from).unwrap();
            n
        });
        let nb = netbios(at, Duration::from_secs(2)).unwrap();
        assert_eq!(nb.name, "FRONTDESK-PC");
        assert_eq!(nb.mac, Some(MAC));
        assert_eq!(t.join().unwrap(), 50);
        // No answer: nothing, after the wait.
        let quiet = UdpSocket::bind("127.0.0.1:0").unwrap();
        let start = Instant::now();
        assert_eq!(
            netbios(quiet.local_addr().unwrap(), Duration::from_millis(200)),
            None
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    /// A device answering a one-shot multicast DNS query for its own address.
    #[test]
    fn mdns_over_the_wire() {
        let ip = Ipv4Addr::new(192, 168, 1, 20);
        let dev = UdpSocket::bind("127.0.0.1:0").unwrap();
        let at = dev.local_addr().unwrap();
        let t = thread::spawn(move || {
            let mut buf = [0u8; 512];
            let (n, from) = dev.recv_from(&mut buf).unwrap();
            let q = &buf[..n];
            // Echo the id and question (RFC 6762, 6.7), then answer with a
            // compression pointer to the question's name, cache-flush set.
            let qend = 12 + skip_name(q, 12).unwrap() - 12 + 4;
            let mut r = q[..2].to_vec();
            r.extend_from_slice(&[0x84, 0x00, 0, 1, 0, 1, 0, 0, 0, 0]);
            r.extend_from_slice(&q[12..qend]);
            r.extend_from_slice(&[0xc0, 12, 0, 12, 0x80, 1, 0, 0, 0, 120]);
            let target = b"\x0bBrother-MFC\x05local\x00";
            r.extend_from_slice(&(target.len() as u16).to_be_bytes());
            r.extend_from_slice(target);
            dev.send_to(&r, from).unwrap();
        });
        assert_eq!(
            mdns(ip, at, Duration::from_secs(2)).as_deref(),
            Some("brother-mfc.local")
        );
        t.join().unwrap();
        // An answer about another address isn't taken.
        let dev = UdpSocket::bind("127.0.0.1:0").unwrap();
        let at = dev.local_addr().unwrap();
        let t = thread::spawn(move || {
            let mut buf = [0u8; 512];
            let (n, from) = dev.recv_from(&mut buf).unwrap();
            let q = &buf[..n];
            let mut r = q[..2].to_vec();
            r.extend_from_slice(&[0x84, 0x00, 0, 0, 0, 1, 0, 0, 0, 0]);
            r.extend_from_slice(b"\x0299\x011\x03168\x03192\x07in-addr\x04arpa\x00");
            r.extend_from_slice(&[0, 12, 0, 1, 0, 0, 0, 120]);
            let target = b"\x05other\x05local\x00";
            r.extend_from_slice(&(target.len() as u16).to_be_bytes());
            r.extend_from_slice(target);
            dev.send_to(&r, from).unwrap();
        });
        assert_eq!(mdns(ip, at, Duration::from_secs(2)), None);
        t.join().unwrap();
    }
}
