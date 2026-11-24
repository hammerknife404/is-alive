//! Path MTU to the internet: pings to 1.1.1.1 with Don't Fragment set, a
//! binary search between 1400 and 1500 bytes for the largest packet that
//! gets through. A router that can't pass a packet should answer
//! "fragmentation needed" (path MTU discovery works); one that drops it
//! silently is a black hole, and TCP connections that send full-size
//! packets stall through it.

use super::checks::{Item, Status};
use super::icmp::{self, EchoStatus};
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

pub const TARGET: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
/// The search's bounds, as whole IP packets: Ethernet's 1500 down to below
/// common VPN and tunnel sizes.
pub const HIGH: u16 = 1500;
pub const LOW: u16 = 1400;
/// IPv4 (20) and ICMP (8) headers: the payload is the size less these.
const HEADERS: u16 = 28;
/// A packet every path passes: 84 bytes, like a plain ping.
pub const BASELINE: u16 = 84;
/// 1.1.1.1 rate-limits ICMP: a size is lost only after this many tries.
const TRIES: usize = 3;
const WAIT_MS: u32 = 1000;
const GAP: Duration = Duration::from_millis(250);
/// A probe given up on because the run was stopped.
const STOPPED: &str = "stopped";

/// What one size got.
#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    Through,
    /// "Fragmentation needed": from a router (`from`), or this computer's
    /// own link (`from` None); with the MTU it named.
    TooBig {
        mtu: Option<u16>,
        from: Option<IpAddr>,
    },
    /// No answer at all, every try.
    Lost,
    Error(String),
}

/// One size, tried up to `tries` times by `send` (silence is tried again;
/// any answer is final). Gives up between tries once `stop` says so.
pub fn try_size(
    mut send: impl FnMut() -> icmp::EchoResult,
    tries: usize,
    stop: &dyn Fn() -> bool,
) -> Probe {
    let mut last_err = None;
    for i in 0..tries {
        if stop() {
            return Probe::Error(STOPPED.into());
        }
        if i > 0 {
            std::thread::sleep(GAP);
        }
        let r = send();
        if r.is_reply() {
            return Probe::Through;
        }
        if r.too_big() {
            return Probe::TooBig {
                mtu: r.mtu,
                from: r.from,
            };
        }
        match r.status {
            EchoStatus::Timeout => {}
            EchoStatus::Unreachable(m) | EchoStatus::Error(m) => last_err = Some(m),
            EchoStatus::TtlExpired => last_err = Some("TTL expired in transit".into()),
            EchoStatus::Reply => return Probe::Through,
        }
    }
    last_err.map_or(Probe::Lost, Probe::Error)
}

/// The sizes tried, in order: `HIGH` first (most paths pass it, and that
/// settles it), then `LOW`, then halving the gap between the largest size
/// that passed and the smallest that didn't, until they're one apart.
/// `probe` is one size with its retries, so a size only counts as failed
/// once every try is spent: one lost reply can't steer the search. Nothing
/// more is tried once `stop` says so.
pub fn search(mut probe: impl FnMut(u16) -> Probe, stop: &dyn Fn() -> bool) -> Vec<(u16, Probe)> {
    let mut tried = Vec::new();
    // Whether `size` passed; None once stopped.
    let mut go = |size: u16, tried: &mut Vec<(u16, Probe)>| {
        if stop() {
            return None;
        }
        let p = probe(size);
        let through = p == Probe::Through;
        let stopped = matches!(&p, Probe::Error(e) if e == STOPPED);
        tried.push((size, p));
        (!stopped).then_some(through)
    };
    if go(HIGH, &mut tried) != Some(false) || go(LOW, &mut tried) != Some(true) {
        return tried;
    }
    let (mut lo, mut hi) = (LOW, HIGH);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        match go(mid, &mut tried) {
            Some(true) => lo = mid,
            Some(false) => hi = mid,
            None => break,
        }
    }
    tried
}

/// Probe the baseline, then search for the largest size that passes;
/// nothing more is sent once `stop` says so (a dropped or replaced run).
pub fn measure(stop: &dyn Fn() -> bool) -> (Probe, Vec<(u16, Probe)>) {
    let send = |size: u16| move || icmp::echo_df(TARGET, WAIT_MS, size - HEADERS);
    let base = try_size(send(BASELINE), TRIES, stop);
    if base != Probe::Through {
        return (base, Vec::new());
    }
    let sizes = search(|s| try_size(send(s), TRIES, stop), stop);
    (base, sizes)
}

/// The verdict. `vpn`: the VPN adapter the route to 1.1.1.1 goes through.
pub fn judge(base: &Probe, sizes: &[(u16, Probe)], vpn: Option<&str>) -> Item {
    let check = "Path MTU";
    let via = vpn
        .map(|v| {
            format!(
                " The route to {TARGET} goes through a VPN adapter ({v}): its tunnel adds \
                 headers, so a path MTU under 1500 is expected."
            )
        })
        .unwrap_or_default();
    match base {
        Probe::Through => {}
        Probe::Error(e) => {
            return Item {
                check,
                status: Status::Unknown,
                detail: format!("Couldn't ping {TARGET} to measure it: {e}.{via}"),
            };
        }
        _ => {
            return Item {
                check,
                status: Status::Unknown,
                detail: format!(
                    "{TARGET} didn't answer even a small ping, so the path MTU couldn't be \
                     measured (ICMP blocked, or no internet).{via}"
                ),
            };
        }
    }
    let largest = sizes
        .iter()
        .filter(|(_, p)| *p == Probe::Through)
        .map(|(s, _)| *s)
        .max();
    let above = |n: u16| sizes.iter().filter(move |(s, _)| *s > n);
    let floor = largest.unwrap_or(BASELINE);
    if largest == Some(HIGH) {
        return Item {
            check,
            status: Status::Ok,
            detail: format!(
                "Full-size 1500-byte packets reach {TARGET} unfragmented: nothing on the way \
                 lowers the MTU.{via}"
            ),
        };
    }
    // What came back for the sizes that didn't pass.
    let told = above(floor).find_map(|(s, p)| match p {
        Probe::TooBig { mtu, from } => Some((*s, *mtu, *from)),
        _ => None,
    });
    let mut silent: Vec<u16> = above(floor)
        .filter(|(_, p)| *p == Probe::Lost)
        .map(|(s, _)| *s)
        .collect();
    silent.sort_unstable();
    let errors: Vec<String> = above(floor)
        .filter_map(|(s, p)| match p {
            Probe::Error(e) => Some(format!("{s}: {e}")),
            _ => None,
        })
        .collect();
    // Exact when the size one byte up was tried and didn't pass.
    let passes = match largest {
        Some(n) if above(n).any(|(s, _)| *s == n + 1) => {
            format!(
                "{n} bytes is the largest packet that passes ({} doesn't)",
                n + 1
            )
        }
        Some(n) => format!("{n}-byte packets pass"),
        None => format!("nothing from {LOW} bytes up passed (a {BASELINE}-byte ping did)"),
    };
    if let Some((size, mtu, from)) = told {
        let who = match from {
            Some(ip) => format!("{ip} answered"),
            // Its own link's MTU, or one a router reported on an earlier run
            // (the system keeps it for a while).
            None => "this computer wouldn't send it, from its link or a path MTU it had \
                     already learned"
                .into(),
        };
        let named = mtu.map(|m| format!(" (MTU {m})")).unwrap_or_default();
        // Silent at a size the reported MTU allows: a hop past that router
        // drops it without saying so. Discovery only half works.
        let hidden: Vec<u16> = silent
            .iter()
            .copied()
            .filter(|&s| mtu.is_some_and(|m| s <= m))
            .collect();
        if !hidden.is_empty() {
            return Item {
                check,
                status: Status::Warn,
                detail: format!(
                    "Silence above {floor} bytes: {passes}; at {size} bytes \"fragmentation \
                     needed\" came back ({who}{named}), but {hidden:?}-byte packets, within that \
                     MTU, vanish with no answer after {TRIES} tries each. A hop past it drops \
                     them without saying so: an MTU black hole further along. Connections that \
                     send full-size packets can stall; lower the MTU or clamp TCP MSS to \
                     {floor} or below.{via}"
                ),
            };
        }
        let extra = if silent.is_empty() {
            String::new()
        } else {
            format!(" Some sizes got no answer at all ({silent:?}), likely rate limiting.")
        };
        return Item {
            check,
            status: Status::Ok,
            detail: format!(
                "Path MTU found: {passes}; at {size} bytes \"fragmentation needed\" came back \
                 ({who}{named}), so path MTU discovery works and connections adjust on their \
                 own.{extra}{via}"
            ),
        };
    }
    if !silent.is_empty() {
        return Item {
            check,
            status: Status::Warn,
            detail: format!(
                "Silence above {floor} bytes: {passes}, but {silent:?}-byte packets vanish with \
                 no \"fragmentation needed\" coming back, after {TRIES} tries each. That's an \
                 MTU black hole: connections that send full-size packets can stall (pages that \
                 half-load, VPNs, file copies). Lower the MTU or clamp TCP MSS on the router or \
                 tunnel, or let ICMP \"fragmentation needed\" through the firewall.{via}"
            ),
        };
    }
    Item {
        check,
        status: Status::Unknown,
        detail: format!("{passes}; larger sizes failed: {}.{via}", errors.join("; ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::icmp::EchoResult;

    fn res(status: EchoStatus, mtu: Option<u16>) -> EchoResult {
        EchoResult {
            from: Some("192.0.2.1".parse().unwrap()),
            rtt_ms: 1.0,
            ttl: None,
            status,
            mtu,
        }
    }

    #[test]
    fn a_rate_limited_size_is_tried_again_before_its_lost() {
        let mut n = 0;
        let p = try_size(
            || {
                n += 1;
                if n < 3 {
                    res(EchoStatus::Timeout, None)
                } else {
                    res(EchoStatus::Reply, None)
                }
            },
            3,
            &|| false,
        );
        assert_eq!(p, Probe::Through);
        assert_eq!(n, 3);
        assert_eq!(
            try_size(|| res(EchoStatus::Timeout, None), 2, &|| false),
            Probe::Lost
        );
        // Stopped: nothing is sent.
        let mut sent = 0;
        let p = try_size(
            || {
                sent += 1;
                res(EchoStatus::Timeout, None)
            },
            3,
            &|| true,
        );
        assert!(matches!(p, Probe::Error(_)) && sent == 0);
        let big = try_size(
            || res(EchoStatus::Error("packet too big".into()), Some(1492)),
            3,
            &|| false,
        );
        assert_eq!(
            big,
            Probe::TooBig {
                mtu: Some(1492),
                from: Some("192.0.2.1".parse().unwrap())
            }
        );
    }

    #[test]
    fn fragmentation_needed_is_healthy_and_silence_is_a_black_hole() {
        let too_big = |m| Probe::TooBig {
            mtu: Some(m),
            from: Some("192.0.2.1".parse().unwrap()),
        };
        // Everything through.
        let all = vec![(HIGH, Probe::Through)];
        assert_eq!(judge(&Probe::Through, &all, None).status, Status::Ok);
        // PPPoE: 1500 told to shrink, 1492 passes.
        let pppoe = vec![
            (1500, too_big(1492)),
            (1492, Probe::Through),
            (1472, Probe::Through),
            (1400, Probe::Through),
        ];
        let r = judge(&Probe::Through, &pppoe, None);
        assert_eq!(r.status, Status::Ok);
        assert!(
            r.detail.contains("Path MTU found") && r.detail.contains("MTU 1492"),
            "{}",
            r.detail
        );
        // Black hole: nothing comes back above 1472.
        let hole = vec![
            (1500, Probe::Lost),
            (1492, Probe::Lost),
            (1472, Probe::Through),
            (1400, Probe::Through),
        ];
        let r = judge(&Probe::Through, &hole, None);
        assert_eq!(r.status, Status::Warn);
        assert!(
            r.detail.contains("Silence above 1472") && r.detail.contains("black hole"),
            "{}",
            r.detail
        );
        // Told to shrink to 1492, yet 1492 itself vanishes: a hop past the
        // router that answered is a black hole. Not "found".
        let half = vec![
            (1500, too_big(1492)),
            (1492, Probe::Lost),
            (1472, Probe::Through),
            (1400, Probe::Through),
        ];
        let r = judge(&Probe::Through, &half, None);
        assert_eq!(r.status, Status::Warn, "{}", r.detail);
        assert!(r.detail.contains("Silence above 1472") && r.detail.contains("[1492]"));
        // Through a VPN: said, whatever the result.
        let r = judge(&Probe::Through, &pppoe, Some("wg0"));
        assert!(r.detail.contains("VPN adapter (wg0)"));
        // 1.1.1.1 silent even for a small ping: not a black hole, unmeasured.
        let r = judge(&Probe::Lost, &[], None);
        assert_eq!(r.status, Status::Unknown);
        assert!(!r.detail.contains("black hole"));
    }

    /// A path: what a size gets, given the router's MTU and where (if
    /// anywhere) packets start vanishing with no answer.
    fn path(router: Option<u16>, hole_above: Option<u16>) -> impl Fn(u16) -> Probe {
        move |size| {
            if router.is_some_and(|m| size > m) {
                Probe::TooBig {
                    mtu: router,
                    from: Some("192.0.2.1".parse().unwrap()),
                }
            } else if hole_above.is_some_and(|h| size > h) {
                Probe::Lost
            } else {
                Probe::Through
            }
        }
    }

    #[test]
    fn the_search_names_the_exact_largest_size() {
        // Ethernet all the way: one probe settles it.
        let t = search(path(None, None), &|| false);
        assert_eq!(t, vec![(1500, Probe::Through)]);
        // PPPoE, the router says so: exactly 1492, and found.
        let t = search(path(Some(1492), None), &|| false);
        assert!(t.len() <= 9, "{t:?}");
        let r = judge(&Probe::Through, &t, None);
        assert_eq!(r.status, Status::Ok, "{}", r.detail);
        assert!(
            r.detail
                .contains("1492 bytes is the largest packet that passes (1493 doesn't)")
                && r.detail.contains("Path MTU found"),
            "{}",
            r.detail
        );
        // An odd size, silent above it: exact, and a black hole.
        let t = search(path(None, Some(1437)), &|| false);
        let r = judge(&Probe::Through, &t, None);
        assert_eq!(r.status, Status::Warn, "{}", r.detail);
        assert!(
            r.detail.contains("Silence above 1437")
                && r.detail.contains("1437 bytes is the largest")
                && r.detail.contains("black hole"),
            "{}",
            r.detail
        );
        // The router reports 1492, but a hop past it drops anything over
        // 1472 without a word: a silent size within the reported MTU is
        // still a black hole, not "found".
        let t = search(path(Some(1492), Some(1472)), &|| false);
        let r = judge(&Probe::Through, &t, None);
        assert_eq!(r.status, Status::Warn, "{}", r.detail);
        assert!(r.detail.contains("Silence above 1472"), "{}", r.detail);
        // Under 1400: the search stops at its floor and says so.
        let t = search(path(Some(1380), None), &|| false);
        assert_eq!(t.len(), 2);
        let r = judge(&Probe::Through, &t, None);
        assert!(
            r.detail.contains("nothing from 1400 bytes up"),
            "{}",
            r.detail
        );
        // Stopped: nothing is tried.
        assert!(search(path(None, None), &|| true).is_empty());
    }

    #[test]
    fn one_lost_reply_doesnt_steer_the_search() {
        // 1.1.1.1 drops the first try of every size (rate limiting); with
        // the retries each size is still judged on what it really gets.
        let real = path(Some(1492), None);
        let mut seen = std::collections::HashMap::<u16, usize>::new();
        let t = search(
            |size| {
                try_size(
                    || {
                        let n = seen.entry(size).or_default();
                        *n += 1;
                        if *n == 1 {
                            return res(EchoStatus::Timeout, None);
                        }
                        match real(size) {
                            Probe::Through => res(EchoStatus::Reply, None),
                            _ => res(EchoStatus::Error("packet too big".into()), Some(1492)),
                        }
                    },
                    3,
                    &|| false,
                )
            },
            &|| false,
        );
        let largest = t
            .iter()
            .filter(|(_, p)| *p == Probe::Through)
            .map(|(s, _)| *s)
            .max();
        assert_eq!(largest, Some(1492), "{t:?}");
        assert!(!t.iter().any(|(_, p)| *p == Probe::Lost), "{t:?}");
    }
}
