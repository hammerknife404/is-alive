//! ICMP echo. On Windows this uses the IP Helper API (IcmpSendEcho2 /
//! Icmp6SendEcho2), which needs no admin rights and no raw sockets.
//! Other platforms shell out to the system `ping` so the app can be
//! developed and tested off Windows.

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

#[cfg(not(windows))]
mod imp {
    use super::{EchoResult, EchoStatus};
    use std::net::IpAddr;
    use std::process::Command;
    use std::time::Instant;

    pub fn echo(dest: IpAddr, ttl: u8, timeout_ms: u32, size: u16) -> EchoResult {
        let secs = timeout_ms.div_ceil(1000).max(1).to_string();
        let start = Instant::now();
        let out = Command::new("ping")
            .args(["-n", "-c", "1", "-W", &secs, "-t", &ttl.to_string()])
            .args(["-s", &size.to_string(), &dest.to_string()])
            .output();
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

    fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
        let i = line.find(key)? + key.len();
        line[i..]
            .split(|c: char| c.is_whitespace() || c == ':')
            .next()
    }

    fn parse(out: &str, dest: IpAddr, elapsed: f64) -> EchoResult {
        for line in out.lines() {
            if let Some(t) = field(line, "time=") {
                return EchoResult {
                    from: Some(dest),
                    rtt_ms: t.parse().unwrap_or(elapsed),
                    ttl: field(line, "ttl=").and_then(|v| v.parse().ok()),
                    status: EchoStatus::Reply,
                };
            }
            if line.starts_with("From ") {
                let from = field(line, "From ").and_then(|a| a.parse().ok());
                let status = if line.contains("Time to live exceeded") {
                    EchoStatus::TtlExpired
                } else {
                    let msg = line.split_once("icmp_seq=").map_or(line, |(_, r)| r);
                    let msg = msg.split_once(' ').map_or(msg, |(_, r)| r);
                    EchoStatus::Unreachable(msg.to_lowercase())
                };
                return EchoResult {
                    from,
                    rtt_ms: elapsed,
                    ttl: None,
                    status,
                };
            }
        }
        EchoResult {
            from: None,
            rtt_ms: elapsed,
            ttl: None,
            status: EchoStatus::Timeout,
        }
    }
}
