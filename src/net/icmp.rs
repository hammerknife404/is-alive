//! ICMP echo. On Windows this uses the IP Helper API (IcmpSendEcho2 /
//! Icmp6SendEcho2), which needs no admin rights and no raw sockets.
//! On Linux it uses unprivileged ICMP datagram ("ping") sockets, falling
//! back to the system `ping` when `net.ipv4.ping_group_range` excludes
//! the user. Other platforms shell out to `ping`.

use std::net::IpAddr;

#[derive(Clone, Debug, PartialEq)]
pub enum EchoStatus {
    Reply,
    TtlExpired,
    Timeout,
    Unreachable(String),
    Error(String),
}

#[derive(Clone, Debug)]
pub struct EchoResult {
    /// Address that answered (target on Reply, router on TtlExpired/Unreachable).
    pub from: Option<IpAddr>,
    pub rtt_ms: f64,
    pub ttl: Option<u8>,
    pub status: EchoStatus,
}

impl EchoResult {
    pub fn is_reply(&self) -> bool {
        self.status == EchoStatus::Reply
    }
}

pub fn echo(dest: IpAddr, ttl: u8, timeout_ms: u32, size: u16) -> EchoResult {
    imp::echo(dest, ttl, timeout_ms, size)
}

#[cfg(windows)]
mod imp {
    use super::{EchoResult, EchoStatus};
    use std::mem::{size_of, zeroed};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::ptr::{null, null_mut, read_unaligned};
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        ICMP_ECHO_REPLY, ICMPV6_ECHO_REPLY_LH, IP_OPTION_INFORMATION, Icmp6CreateFile,
        Icmp6SendEcho2, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho2,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET6, SOCKADDR_IN6};

    fn map_status(code: u32) -> EchoStatus {
        match code {
            0 => EchoStatus::Reply,
            11010 => EchoStatus::Timeout,
            11013 | 11014 => EchoStatus::TtlExpired,
            11002 => EchoStatus::Unreachable("destination net unreachable".into()),
            11003 => EchoStatus::Unreachable("destination host unreachable".into()),
            11004 => EchoStatus::Unreachable("destination protocol unreachable".into()),
            11005 => EchoStatus::Unreachable("destination port unreachable".into()),
            11009 => EchoStatus::Error("packet too big".into()),
            11050 => EchoStatus::Error("general failure".into()),
            other => EchoStatus::Error(format!("ICMP status {other}")),
        }
    }

    fn reply_buf(size: u16, header: usize) -> Vec<u64> {
        // Room for the reply struct, echoed payload, and an ICMP error body.
        let bytes = header + size as usize + 8 + 128;
        vec![0u64; bytes.div_ceil(8)]
    }

    pub fn echo(dest: IpAddr, ttl: u8, timeout_ms: u32, size: u16) -> EchoResult {
        let payload = vec![b'a'; size as usize];
        let opts = IP_OPTION_INFORMATION {
            Ttl: ttl,
            Tos: 0,
            Flags: 0,
            OptionsSize: 0,
            OptionsData: null_mut(),
        };
        match dest {
            IpAddr::V4(v4) => unsafe { echo_v4(v4, &opts, &payload, timeout_ms) },
            IpAddr::V6(v6) => unsafe { echo_v6(v6, &opts, &payload, timeout_ms) },
        }
    }

    unsafe fn echo_v4(
        dest: Ipv4Addr,
        opts: &IP_OPTION_INFORMATION,
        payload: &[u8],
        timeout_ms: u32,
    ) -> EchoResult {
        unsafe {
            let h = IcmpCreateFile();
            if h == INVALID_HANDLE_VALUE {
                return fail(format!("IcmpCreateFile failed ({})", GetLastError()));
            }
            let mut buf = reply_buf(payload.len() as u16, size_of::<ICMP_ECHO_REPLY>());
            let start = Instant::now();
            let n = IcmpSendEcho2(
                h,
                null_mut(),
                None,
                null(),
                u32::from_ne_bytes(dest.octets()),
                payload.as_ptr().cast(),
                payload.len() as u16,
                opts,
                buf.as_mut_ptr().cast(),
                (buf.len() * 8) as u32,
                timeout_ms,
            );
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let err = if n == 0 { GetLastError() } else { 0 };
            IcmpCloseHandle(h);

            let reply: ICMP_ECHO_REPLY = read_unaligned(buf.as_ptr().cast());
            let code = if n == 0 && reply.Address == 0 {
                err
            } else {
                reply.Status
            };
            let status = map_status(code);
            let from = (reply.Address != 0)
                .then(|| IpAddr::V4(Ipv4Addr::from(reply.Address.to_ne_bytes())));
            let ttl = (status == EchoStatus::Reply).then_some(reply.Options.Ttl);
            EchoResult {
                from,
                rtt_ms: elapsed,
                ttl,
                status,
            }
        }
    }

    unsafe fn echo_v6(
        dest: Ipv6Addr,
        opts: &IP_OPTION_INFORMATION,
        payload: &[u8],
        timeout_ms: u32,
    ) -> EchoResult {
        unsafe {
            let h = Icmp6CreateFile();
            if h == INVALID_HANDLE_VALUE {
                return fail(format!("Icmp6CreateFile failed ({})", GetLastError()));
            }
            let mut src: SOCKADDR_IN6 = zeroed();
            src.sin6_family = AF_INET6;
            let mut dst: SOCKADDR_IN6 = zeroed();
            dst.sin6_family = AF_INET6;
            dst.sin6_addr.u.Byte = dest.octets();

            let mut buf = reply_buf(payload.len() as u16, size_of::<ICMPV6_ECHO_REPLY_LH>());
            let start = Instant::now();
            let n = Icmp6SendEcho2(
                h,
                null_mut(),
                None,
                null(),
                &src,
                &dst,
                payload.as_ptr().cast(),
                payload.len() as u16,
                opts,
                buf.as_mut_ptr().cast(),
                (buf.len() * 8) as u32,
                timeout_ms,
            );
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let err = if n == 0 { GetLastError() } else { 0 };
            IcmpCloseHandle(h);

            let reply: ICMPV6_ECHO_REPLY_LH = read_unaligned(buf.as_ptr().cast());
            let mut bytes = [0u8; 16];
            let words = reply.Address.sin6_addr;
            for (i, w) in words.iter().enumerate() {
                bytes[i * 2..i * 2 + 2].copy_from_slice(&w.to_ne_bytes());
            }
            let addr = Ipv6Addr::from(bytes);
            let code = if n == 0 && addr.is_unspecified() {
                err
            } else {
                reply.Status
            };
            let from = (!addr.is_unspecified()).then_some(IpAddr::V6(addr));
            EchoResult {
                from,
                rtt_ms: elapsed,
                ttl: None,
                status: map_status(code),
            }
        }
    }

    fn fail(msg: String) -> EchoResult {
        EchoResult {
            from: None,
            rtt_ms: 0.0,
            ttl: None,
            status: EchoStatus::Error(msg),
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{EchoResult, EchoStatus};
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
    use std::time::{Duration, Instant};

    /// Set once ping sockets are refused; later probes go straight to `ping`.
    static NO_PING_SOCKET: AtomicBool = AtomicBool::new(false);
    static SEQ: AtomicU16 = AtomicU16::new(1);

    pub fn echo(dest: IpAddr, ttl: u8, timeout_ms: u32, size: u16) -> EchoResult {
        if !NO_PING_SOCKET.load(Ordering::Relaxed) {
            match socket_echo(dest, ttl, timeout_ms, size) {
                Ok(r) => return r,
                Err(e) if matches!(e.raw_os_error(), Some(libc::EACCES | libc::EPERM)) => {
                    NO_PING_SOCKET.store(true, Ordering::Relaxed);
                }
                Err(e) => return fail(e.to_string()),
            }
        }
        super::cmd::echo(dest, ttl, timeout_ms, size)
    }

    fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
        if r < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(r)
        }
    }

    fn setopt(fd: &OwnedFd, level: i32, name: i32, val: i32) -> io::Result<()> {
        cvt(unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                level,
                name,
                (&val as *const i32).cast(),
                size_of::<i32>() as libc::socklen_t,
            )
        })
        .map(drop)
    }

    fn socket_echo(dest: IpAddr, ttl: u8, timeout_ms: u32, size: u16) -> io::Result<EchoResult> {
        let v6 = dest.is_ipv6();
        let (domain, proto) = if v6 {
            (libc::AF_INET6, libc::IPPROTO_ICMPV6)
        } else {
            (libc::AF_INET, libc::IPPROTO_ICMP)
        };
        let fd =
            cvt(unsafe { libc::socket(domain, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, proto) })?;
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        if v6 {
            setopt(&fd, libc::IPPROTO_IPV6, libc::IPV6_UNICAST_HOPS, ttl as i32)?;
            setopt(&fd, libc::IPPROTO_IPV6, libc::IPV6_RECVERR, 1)?;
            setopt(&fd, libc::IPPROTO_IPV6, libc::IPV6_RECVHOPLIMIT, 1)?;
        } else {
            setopt(&fd, libc::IPPROTO_IP, libc::IP_TTL, ttl as i32)?;
            setopt(&fd, libc::IPPROTO_IP, libc::IP_RECVERR, 1)?;
            setopt(&fd, libc::IPPROTO_IP, libc::IP_RECVTTL, 1)?;
        }

        // The kernel fills in the identifier and checksum.
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut pkt = vec![b'a'; 8 + size as usize];
        pkt[..8].fill(0);
        pkt[0] = if v6 { 128 } else { 8 };
        pkt[6..8].copy_from_slice(&seq.to_be_bytes());

        let (addr, alen) = sockaddr(dest);
        let start = Instant::now();
        let sent = unsafe {
            libc::sendto(
                fd.as_raw_fd(),
                pkt.as_ptr().cast(),
                pkt.len(),
                0,
                (&addr as *const libc::sockaddr_storage).cast(),
                alen,
            )
        };
        if sent < 0 {
            let e = io::Error::last_os_error();
            return match e.raw_os_error() {
                Some(libc::EACCES | libc::EPERM) => Err(e),
                Some(libc::ENETUNREACH | libc::EHOSTUNREACH) => Ok(EchoResult {
                    from: None,
                    rtt_ms: 0.0,
                    ttl: None,
                    status: EchoStatus::Unreachable(e.to_string().to_lowercase()),
                }),
                _ => Ok(fail(format!("send: {e}"))),
            };
        }

        let deadline = start + Duration::from_millis(timeout_ms as u64);
        let mut buf = vec![0u8; pkt.len() + 512];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(timeout(start));
            }
            let mut pfd = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let n = unsafe { libc::poll(&mut pfd, 1, left.as_millis().max(1) as i32) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Ok(fail(format!("poll: {e}")));
            }
            if n == 0 {
                return Ok(timeout(start));
            }
            if pfd.revents & libc::POLLERR != 0 {
                if let Some(r) = read_error(&fd, v6, start) {
                    return Ok(r);
                }
                continue;
            }
            if let Some(r) = read_reply(&fd, &mut buf, v6, seq, dest, start) {
                return Ok(r);
            }
        }
    }

    fn rtt(start: Instant) -> f64 {
        start.elapsed().as_secs_f64() * 1000.0
    }

    /// Echo replies arrive as bare ICMP (no IP header). Anything that is not
    /// our sequence number is a stale reply and is skipped.
    fn read_reply(
        fd: &OwnedFd,
        buf: &mut [u8],
        v6: bool,
        seq: u16,
        dest: IpAddr,
        start: Instant,
    ) -> Option<EchoResult> {
        let mut cbuf = [0u64; 16];
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        };
        let mut msg: libc::msghdr = unsafe { zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = size_of_val(&cbuf) as _;
        let n = unsafe { libc::recvmsg(fd.as_raw_fd(), &mut msg, libc::MSG_DONTWAIT) };
        let elapsed = rtt(start);
        if n < 8 {
            return None;
        }
        let reply_type = if v6 { 129 } else { 0 };
        if buf[0] != reply_type || u16::from_be_bytes([buf[6], buf[7]]) != seq {
            return None;
        }
        let mut ttl = None;
        unsafe {
            let mut c = libc::CMSG_FIRSTHDR(&msg);
            while !c.is_null() {
                let (lvl, ty) = ((*c).cmsg_level, (*c).cmsg_type);
                if (lvl == libc::IPPROTO_IP && ty == libc::IP_TTL)
                    || (lvl == libc::IPPROTO_IPV6 && ty == libc::IPV6_HOPLIMIT)
                {
                    let v = std::ptr::read_unaligned(libc::CMSG_DATA(c).cast::<i32>());
                    ttl = u8::try_from(v).ok();
                }
                c = libc::CMSG_NXTHDR(&msg, c);
            }
        }
        Some(EchoResult {
            from: Some(dest),
            rtt_ms: elapsed,
            ttl,
            status: EchoStatus::Reply,
        })
    }

    /// ICMP errors (TTL exceeded, unreachable) land on the error queue via
    /// IP_RECVERR, with the sending router as the offender address.
    fn read_error(fd: &OwnedFd, v6: bool, start: Instant) -> Option<EchoResult> {
        let mut data = [0u8; 576];
        let mut cbuf = [0u64; 64];
        let mut iov = libc::iovec {
            iov_base: data.as_mut_ptr().cast(),
            iov_len: data.len(),
        };
        let mut msg: libc::msghdr = unsafe { zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = size_of_val(&cbuf) as _;
        let n = unsafe {
            libc::recvmsg(
                fd.as_raw_fd(),
                &mut msg,
                libc::MSG_ERRQUEUE | libc::MSG_DONTWAIT,
            )
        };
        let elapsed = rtt(start);
        if n < 0 {
            return None;
        }
        unsafe {
            let mut c = libc::CMSG_FIRSTHDR(&msg);
            while !c.is_null() {
                let (lvl, ty) = ((*c).cmsg_level, (*c).cmsg_type);
                if (lvl == libc::IPPROTO_IP && ty == libc::IP_RECVERR)
                    || (lvl == libc::IPPROTO_IPV6 && ty == libc::IPV6_RECVERR)
                {
                    let ee_ptr = libc::CMSG_DATA(c).cast::<libc::sock_extended_err>();
                    let ee = std::ptr::read_unaligned(ee_ptr);
                    let from = offender(ee_ptr);
                    let status = classify(&ee, v6);
                    return Some(EchoResult {
                        from,
                        rtt_ms: elapsed,
                        ttl: None,
                        status,
                    });
                }
                c = libc::CMSG_NXTHDR(&msg, c);
            }
        }
        None
    }

    unsafe fn offender(ee: *const libc::sock_extended_err) -> Option<IpAddr> {
        unsafe {
            let sa = libc::SO_EE_OFFENDER(ee);
            match (*sa).sa_family as i32 {
                libc::AF_INET => {
                    let s = std::ptr::read_unaligned(sa.cast::<libc::sockaddr_in>());
                    let ip = Ipv4Addr::from(u32::from_be(s.sin_addr.s_addr));
                    (!ip.is_unspecified()).then_some(IpAddr::V4(ip))
                }
                libc::AF_INET6 => {
                    let s = std::ptr::read_unaligned(sa.cast::<libc::sockaddr_in6>());
                    let ip = Ipv6Addr::from(s.sin6_addr.s6_addr);
                    (!ip.is_unspecified()).then_some(IpAddr::V6(ip))
                }
                _ => None,
            }
        }
    }

    fn classify(ee: &libc::sock_extended_err, v6: bool) -> EchoStatus {
        let unreach = |s: &str| EchoStatus::Unreachable(s.into());
        match (ee.ee_origin, v6, ee.ee_type, ee.ee_code) {
            (libc::SO_EE_ORIGIN_ICMP, false, 11, _) | (libc::SO_EE_ORIGIN_ICMP6, true, 3, _) => {
                EchoStatus::TtlExpired
            }
            (libc::SO_EE_ORIGIN_ICMP, false, 3, code) => match code {
                0 | 6 | 11 => unreach("destination net unreachable"),
                1 | 7 | 12 => unreach("destination host unreachable"),
                2 => unreach("destination protocol unreachable"),
                3 => unreach("destination port unreachable"),
                4 => EchoStatus::Error("packet too big".into()),
                9 | 10 | 13 => unreach("communication administratively prohibited"),
                c => unreach(&format!("destination unreachable (code {c})")),
            },
            (libc::SO_EE_ORIGIN_ICMP6, true, 1, code) => match code {
                0 => unreach("no route to destination"),
                1 => unreach("communication administratively prohibited"),
                3 => unreach("destination host unreachable"),
                4 => unreach("destination port unreachable"),
                c => unreach(&format!("destination unreachable (code {c})")),
            },
            (libc::SO_EE_ORIGIN_ICMP6, true, 2, _) => EchoStatus::Error("packet too big".into()),
            (libc::SO_EE_ORIGIN_LOCAL, ..) => unreach(
                &io::Error::from_raw_os_error(ee.ee_errno as i32)
                    .to_string()
                    .to_lowercase(),
            ),
            (_, _, t, c) => EchoStatus::Error(format!("ICMP type {t} code {c}")),
        }
    }

    fn sockaddr(ip: IpAddr) -> (libc::sockaddr_storage, libc::socklen_t) {
        let mut ss: libc::sockaddr_storage = unsafe { zeroed() };
        let len = match ip {
            IpAddr::V4(v4) => {
                let sin = libc::sockaddr_in {
                    sin_family: libc::AF_INET as libc::sa_family_t,
                    sin_port: 0,
                    sin_addr: libc::in_addr {
                        s_addr: u32::from(v4).to_be(),
                    },
                    sin_zero: [0; 8],
                };
                unsafe { std::ptr::write((&mut ss as *mut libc::sockaddr_storage).cast(), sin) };
                size_of::<libc::sockaddr_in>()
            }
            IpAddr::V6(v6) => {
                let mut sin6: libc::sockaddr_in6 = unsafe { zeroed() };
                sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                sin6.sin6_addr.s6_addr = v6.octets();
                unsafe { std::ptr::write((&mut ss as *mut libc::sockaddr_storage).cast(), sin6) };
                size_of::<libc::sockaddr_in6>()
            }
        };
        (ss, len as libc::socklen_t)
    }

    fn timeout(start: Instant) -> EchoResult {
        EchoResult {
            from: None,
            rtt_ms: rtt(start),
            ttl: None,
            status: EchoStatus::Timeout,
        }
    }

    fn fail(msg: String) -> EchoResult {
        EchoResult {
            from: None,
            rtt_ms: 0.0,
            ttl: None,
            status: EchoStatus::Error(msg),
        }
    }
}

#[cfg(all(not(windows), not(target_os = "linux")))]
mod imp {
    pub use super::cmd::echo;
}

/// Fallback: shell out to the system `ping`.
#[cfg(not(windows))]
mod cmd {
    use super::{EchoResult, EchoStatus};
    use std::net::IpAddr;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    pub fn echo(dest: IpAddr, ttl: u8, timeout_ms: u32, size: u16) -> EchoResult {
        let start = Instant::now();
        let Some(prog) = crate::net::system_tool(program(dest)) else {
            return EchoResult {
                from: None,
                rtt_ms: 0.0,
                ttl: None,
                status: EchoStatus::Error(format!("{} not found", program(dest))),
            };
        };
        // C locale: the output is parsed, and translations would break that.
        let child = Command::new(prog)
            .env("LC_ALL", "C")
            .args(args(dest, ttl, timeout_ms))
            .args(["-s", &size.to_string(), &dest.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        // Not every ping takes a per-reply timeout (BSD ping6 has none, iputils
        // rounds up to whole seconds), so enforce the deadline here.
        let deadline =
            Duration::from_millis(u64::from(timeout_ms.div_ceil(1000).max(1)) * 1000 + 500);
        let out = child.and_then(|mut c| {
            while c.try_wait()?.is_none() {
                if start.elapsed() >= deadline {
                    _ = c.kill();
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            c.wait_with_output()
        });
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        let out = match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
            Err(e) => {
                return EchoResult {
                    from: None,
                    rtt_ms: 0.0,
                    ttl: None,
                    status: EchoStatus::Error(format!("ping: {e}")),
                };
            }
        };
        parse(&out, dest, elapsed)
    }

    pub(super) fn program(dest: IpAddr) -> &'static str {
        // BSD/macOS keep a separate ping6; iputils' ping does both.
        if cfg!(target_os = "linux") || dest.is_ipv4() {
            "ping"
        } else {
            "ping6"
        }
    }

    /// TTL and timeout flags differ: iputils uses -t TTL / -W seconds, BSD and
    /// macOS use -m TTL / -W milliseconds (-t there is a total timeout), and
    /// BSD ping6 takes the hop limit as -h.
    pub(super) fn args(dest: IpAddr, ttl: u8, timeout_ms: u32) -> Vec<String> {
        let ttl = ttl.to_string();
        let mut a = vec!["-n".to_string(), "-c".into(), "1".into()];
        if cfg!(target_os = "linux") {
            let secs = timeout_ms.div_ceil(1000).max(1).to_string();
            a.extend(["-W".into(), secs, "-t".into(), ttl]);
        } else if dest.is_ipv4() {
            a.extend(["-W".into(), timeout_ms.to_string(), "-m".into(), ttl]);
        } else {
            a.extend(["-h".into(), ttl]);
        }
        a
    }

    fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
        let i = line.find(key)? + key.len();
        line[i..]
            .split(|c: char| c.is_whitespace() || c == ':')
            .next()
    }

    /// The address after `key`, e.g. "From 10.0.0.1 icmp_seq" (iputils) or
    /// "bytes from 10.0.0.1: Time to live exceeded" (BSD/macOS).
    fn addr(line: &str, key: &str) -> Option<IpAddr> {
        let i = line.find(key)? + key.len();
        let a = line[i..].split_whitespace().next()?;
        let a = a.strip_suffix([':', ',']).unwrap_or(a);
        a.parse().ok()
    }

    pub(super) fn parse(out: &str, dest: IpAddr, elapsed: f64) -> EchoResult {
        for line in out.lines() {
            if let Some(t) = field(line, "time=") {
                return EchoResult {
                    from: Some(dest),
                    rtt_ms: t.parse().unwrap_or(elapsed),
                    ttl: field(line, "ttl=").and_then(|v| v.parse().ok()),
                    status: EchoStatus::Reply,
                };
            }
            // An ICMP error: iputils "From <ip> icmp_seq=1 <msg>",
            // BSD/macOS "<n> bytes from <ip>: <msg>".
            let (key, msg) = if line.starts_with("From ") {
                let msg = line.split_once("icmp_seq=").map_or(line, |(_, r)| r);
                ("From ", msg.split_once(' ').map_or(msg, |(_, r)| r))
            } else if let Some((_, r)) = line.split_once(" bytes from ") {
                (" bytes from ", r.split_once(": ").map_or(r, |(_, m)| m))
            } else {
                continue;
            };
            let lower = msg.to_lowercase();
            let status = if lower.contains("time to live exceeded")
                || lower.contains("time exceeded")
                || lower.contains("hop limit")
            {
                EchoStatus::TtlExpired
            } else {
                EchoStatus::Unreachable(lower)
            };
            return EchoResult {
                from: addr(line, key),
                rtt_ms: elapsed,
                ttl: None,
                status,
            };
        }
        EchoResult {
            from: None,
            rtt_ms: elapsed,
            ttl: None,
            status: EchoStatus::Timeout,
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn iputils_ping_flags() {
        let ip = "192.0.2.1".parse().unwrap();
        assert_eq!(super::cmd::program(ip), "ping");
        assert_eq!(
            super::cmd::args(ip, 7, 1500),
            ["-n", "-c", "1", "-W", "2", "-t", "7"]
        );
    }

    #[test]
    fn ping_output_errors() {
        use super::EchoStatus;
        let dest = "192.0.2.1".parse().unwrap();
        let hop: std::net::IpAddr = "10.0.0.1".parse().unwrap();
        for out in [
            "From 10.0.0.1 icmp_seq=1 Time to live exceeded\n",
            "92 bytes from 10.0.0.1: Time to live exceeded\n",
        ] {
            let r = super::cmd::parse(out, dest, 5.0);
            assert_eq!(r.status, EchoStatus::TtlExpired, "{out}");
            assert_eq!(r.from, Some(hop), "{out}");
        }
        let v6 = super::cmd::parse(
            "From 2001:db8::1 icmp_seq=1 Time exceeded: Hop limit\n",
            dest,
            5.0,
        );
        assert_eq!(v6.status, EchoStatus::TtlExpired);
        assert_eq!(v6.from, "2001:db8::1".parse().ok());
        let un = super::cmd::parse(
            "From 10.0.0.1 icmp_seq=1 Destination Host Unreachable\n",
            dest,
            5.0,
        );
        assert_eq!(
            un.status,
            EchoStatus::Unreachable("destination host unreachable".into())
        );
        let ok = super::cmd::parse(
            "64 bytes from 192.0.2.1: icmp_seq=0 ttl=57 time=10.5 ms\n",
            dest,
            5.0,
        );
        assert!(ok.is_reply() && ok.ttl == Some(57) && ok.rtt_ms == 10.5);
    }

    #[test]
    fn ping_socket_loopback() {
        // Skip where ping sockets are disabled (net.ipv4.ping_group_range).
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, libc::IPPROTO_ICMP) };
        if fd < 0 {
            return;
        }
        unsafe { libc::close(fd) };
        let r = super::echo("127.0.0.1".parse().unwrap(), 64, 1000, 32);
        assert!(r.is_reply(), "{r:?}");
        assert!(r.ttl.is_some());
    }
}
