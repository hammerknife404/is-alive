//! Wake-on-LAN: the "magic packet", six 0xFF bytes and then the MAC 16
//! times, sent by UDP to port 9 as a broadcast. A sleeping or switched-off
//! computer whose network card is set up for it wakes when the packet
//! reaches it. It has no IP while asleep, so the packet goes to everything
//! on the segment and the card picks out its own MAC.
//!
//! Only the MAC goes in the packet; nothing comes back. Whether it woke is
//! for a Ping a minute later to say.

use super::mac::Mac;
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

/// The port every Wake-on-LAN tool sends to ("discard"). Cards don't
/// listen on a port: they read every frame, so any port works, but 9 is
/// what firewalls and relays expect.
pub const PORT: u16 = 9;

/// The magic packet for `mac`.
pub fn packet(mac: &Mac) -> [u8; 102] {
    let mut p = [0xFF; 102];
    for copy in p[6..].as_chunks_mut::<6>().0 {
        *copy = *mac;
    }
    p
}

/// One send: from this computer's address `from`, to `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Send {
    pub from: Ipv4Addr,
    pub to: Ipv4Addr,
}

/// Where the packets go, given this computer's IPv4 networks (address and
/// mask) and the card's last known IP, if any.
///
/// - **On one of this computer's networks:** that network's broadcast
///   address (the one Linux routes out of the right card) and
///   255.255.255.255 (which Windows sends out of the card it's bound to).
/// - **Anywhere else, or no IP known:** 255.255.255.255 out of every
///   network, and the IP itself, which only helps if the router still has
///   its MAC. `local` is false then, so the user is told.
pub fn plan(nets: &[(Ipv4Addr, Ipv4Addr)], ip: Option<Ipv4Addr>) -> (Vec<Send>, bool) {
    let nets: Vec<(Ipv4Addr, Ipv4Addr)> = nets
        .iter()
        .copied()
        .filter(|(a, _)| !a.is_loopback() && !a.is_unspecified())
        .collect();
    let home = ip.and_then(|ip| {
        nets.iter()
            .copied()
            .find(|&(a, m)| u32::from(a) & u32::from(m) == u32::from(ip) & u32::from(m))
    });
    let mut out = Vec::new();
    if let Some((from, mask)) = home {
        // A /31 or /32 (a VPN link) has no broadcast address of its own.
        if mask.to_bits().leading_ones() < 31 {
            let to = Ipv4Addr::from(u32::from(from) | !u32::from(mask));
            out.push(Send { from, to });
        }
        out.push(Send {
            from,
            to: Ipv4Addr::BROADCAST,
        });
        return (out, true);
    }
    for &(from, _) in &nets {
        out.push(Send {
            from,
            to: Ipv4Addr::BROADCAST,
        });
    }
    if let Some(ip) = ip {
        out.push(Send {
            from: Ipv4Addr::UNSPECIFIED,
            to: ip,
        });
    }
    (out, false)
}

/// What was sent: how many packets, and whether the card's IP is on one
/// of this computer's networks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sent {
    pub packets: usize,
    pub local: bool,
}

/// This computer's IPv4 networks: address and mask.
fn networks() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(a) => Some((a.ip, a.netmask)),
            _ => None,
        })
        .collect()
}

/// Wake `mac`, last seen at `ip`. An error only if no packet went out.
pub fn wake(mac: &Mac, ip: Option<Ipv4Addr>) -> io::Result<Sent> {
    let (sends, local) = plan(&networks(), ip);
    let p = packet(mac);
    let mut packets = 0;
    let mut last = None;
    for s in sends {
        let r = UdpSocket::bind(SocketAddrV4::new(s.from, 0)).and_then(|sock| {
            sock.set_broadcast(true)?;
            sock.send_to(&p, SocketAddrV4::new(s.to, PORT))
        });
        match r {
            Ok(_) => packets += 1,
            Err(e) => last = Some(e),
        }
    }
    match (packets, last) {
        (0, Some(e)) => Err(e),
        (0, None) => Err(io::Error::other("this computer has no IPv4 network")),
        _ => Ok(Sent { packets, local }),
    }
}

/// The toast after a wake: what was sent, and what to do next.
pub fn report(mac: &Mac, r: &io::Result<Sent>) -> (String, bool) {
    let m = super::mac::fmt(mac);
    match r {
        Ok(s) if s.local => (
            format!("Wake-on-LAN sent to {m}. Give it a minute, then Ping it."),
            true,
        ),
        Ok(_) => (
            format!(
                "Wake-on-LAN sent to {m}, but it isn't on this computer's network: \
                 it only gets there if the router passes it on."
            ),
            true,
        ),
        Err(e) => (format!("Couldn't send Wake-on-LAN to {m} ({e})"), false),
    }
}

/// Wake each of `cards` (MAC, last known IP) off the UI thread; a toast
/// says what went out.
pub fn wake_all(cards: Vec<(Mac, Option<Ipv4Addr>)>) {
    std::thread::spawn(move || {
        let toast = match cards.as_slice() {
            [(mac, ip)] => report(mac, &wake(mac, *ip)),
            _ => {
                let rs: Vec<io::Result<Sent>> =
                    cards.iter().map(|(mac, ip)| wake(mac, *ip)).collect();
                report_all(&rs)
            }
        };
        crate::widgets::toast(toast.0, toast.1);
    });
}

/// The toast after waking several cards.
fn report_all(rs: &[io::Result<Sent>]) -> (String, bool) {
    let sent = rs.iter().filter(|r| r.is_ok()).count();
    let far = rs.iter().filter(|r| matches!(r, Ok(s) if !s.local)).count();
    let mut t = format!("Wake-on-LAN sent to {sent} of {} cards.", rs.len());
    if far > 0 {
        t.push_str(&format!(
            " {far} aren't on this computer's network: those only get it if the router passes it on."
        ));
    }
    if sent > 0 {
        t.push_str(" Give them a minute, then Ping all.");
    }
    (t, sent > 0)
}

/// Whether a card's MAC can be woken: a real card's address. A private
/// (locally administered) one belongs to a phone, a laptop's per-network
/// address or a virtual NIC, none of which wake this way.
pub fn wakeable(mac: &Mac) -> bool {
    !super::mac::local_admin(mac) && mac[0] & 0x01 == 0 && mac.iter().any(|&b| b != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: Mac = [0x00, 0x11, 0x32, 0xAB, 0xCD, 0xEF];

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn the_packet_is_six_ff_then_the_mac_sixteen_times() {
        let p = packet(&MAC);
        assert_eq!(p.len(), 102);
        assert_eq!(&p[..6], &[0xFF; 6]);
        for k in 0..16 {
            assert_eq!(&p[6 + k * 6..12 + k * 6], &MAC, "copy {k}");
        }
    }

    #[test]
    fn on_this_network_it_goes_to_its_broadcast_from_its_card() {
        let nets = [
            (ip("127.0.0.1"), ip("255.0.0.0")),
            (ip("10.8.0.2"), ip("255.255.255.0")),
            (ip("192.168.1.20"), ip("255.255.255.0")),
        ];
        let (sends, local) = plan(&nets, Some(ip("192.168.1.77")));
        assert!(local);
        let from = ip("192.168.1.20");
        assert_eq!(
            sends,
            [
                Send {
                    from,
                    to: ip("192.168.1.255")
                },
                Send {
                    from,
                    to: Ipv4Addr::BROADCAST
                },
            ]
        );
    }

    #[test]
    fn a_wider_mask_gives_its_own_broadcast() {
        let nets = [(ip("172.16.4.9"), ip("255.255.252.0"))];
        let (sends, _) = plan(&nets, Some(ip("172.16.6.1")));
        assert_eq!(sends[0].to, ip("172.16.7.255"));
    }

    #[test]
    fn a_point_to_point_link_has_no_broadcast_of_its_own() {
        let nets = [(ip("10.9.0.5"), ip("255.255.255.255"))];
        let (sends, local) = plan(&nets, Some(ip("10.9.0.5")));
        assert!(local);
        assert_eq!(
            sends,
            [Send {
                from: ip("10.9.0.5"),
                to: Ipv4Addr::BROADCAST
            }]
        );
    }

    #[test]
    fn elsewhere_it_goes_out_of_every_card_and_to_the_ip() {
        let nets = [
            (ip("127.0.0.1"), ip("255.0.0.0")),
            (ip("192.168.1.20"), ip("255.255.255.0")),
            (ip("10.8.0.2"), ip("255.255.255.0")),
        ];
        let (sends, local) = plan(&nets, Some(ip("10.20.0.5")));
        assert!(!local);
        let to: Vec<Ipv4Addr> = sends.iter().map(|s| s.to).collect();
        assert_eq!(
            to,
            [Ipv4Addr::BROADCAST, Ipv4Addr::BROADCAST, ip("10.20.0.5")]
        );
        assert!(sends.iter().all(|s| !s.from.is_loopback()));
    }

    #[test]
    fn no_ip_known_sends_only_broadcasts() {
        let nets = [(ip("192.168.1.20"), ip("255.255.255.0"))];
        let (sends, local) = plan(&nets, None);
        assert!(!local);
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].to, Ipv4Addr::BROADCAST);
    }

    #[test]
    fn private_and_broken_macs_are_not_offered() {
        assert!(wakeable(&MAC));
        // Locally administered: a phone's private address.
        assert!(!wakeable(&[0xDA, 0x11, 0x32, 0xAB, 0xCD, 0xEF]));
        // Multicast, and all zeros (an incomplete ARP entry).
        assert!(!wakeable(&[0x01, 0x00, 0x5E, 0x00, 0x00, 0x01]));
        assert!(!wakeable(&[0; 6]));
    }

    #[test]
    fn the_report_says_what_to_do_next() {
        let ok = Ok(Sent {
            packets: 2,
            local: true,
        });
        let (t, good) = report(&MAC, &ok);
        assert!(good && t.contains("00:11:32:ab:cd:ef") && t.contains("Ping"));
        let far = Ok(Sent {
            packets: 3,
            local: false,
        });
        assert!(report(&MAC, &far).0.contains("router"));
        let err = Err(io::Error::other("no route"));
        let (t, good) = report(&MAC, &err);
        assert!(!good && t.contains("no route"));
    }

    #[test]
    fn several_cards_report_in_one_toast() {
        let rs = [
            Ok(Sent {
                packets: 2,
                local: true,
            }),
            Ok(Sent {
                packets: 3,
                local: false,
            }),
            Err(io::Error::other("x")),
        ];
        let (t, ok) = report_all(&rs);
        assert!(ok);
        assert!(t.starts_with("Wake-on-LAN sent to 2 of 3 cards."), "{t}");
        assert!(t.contains("1 aren't on this computer's network"), "{t}");
        let (_, ok) = report_all(&[Err(io::Error::other("x"))]);
        assert!(!ok);
    }

    /// Sends a real packet on loopback and reads it back: the bytes on
    /// the wire are the packet.
    #[test]
    fn what_goes_out_is_the_packet() {
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        let to = match rx.local_addr().unwrap() {
            std::net::SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        tx.send_to(&packet(&MAC), to).unwrap();
        let mut buf = [0u8; 200];
        rx.set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let n = rx.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], &packet(&MAC)[..]);
    }
}
