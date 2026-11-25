//! Hear the switch's LLDP or CDP announcement on the wired adapter, for
//! "Which port am I on?". Receive only: nothing is sent on the wire. Tried
//! in order, each only if it can work here:
//!
//! - Windows: Npcap's wpcap.dll, loaded at run time from System32\Npcap
//!   by its full path (no dependency on it; never from PATH), capturing
//!   with a filter for the two multicast addresses.
//! - Linux: a raw AF_PACKET socket (needs CAP_NET_RAW), joined to the two
//!   multicast addresses with a kernel filter for them; else what lldpd
//!   heard (`lldpcli`), when it's already running; else what
//!   systemd-networkd heard (`networkctl lldp`), when it manages the link.
//!
//! When none can, the outcome says what would make one work. Never
//! "install or start lldpd": lldpd announces this computer on every port
//! it runs on, and an install starts it for good.

use super::lldp::{self, Neighbor};
use super::netconf::{self, Wired};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How long to listen: LLDP's default interval is 30 s and CDP's 60 s,
/// so this hears either at least once.
pub const LISTEN: Duration = Duration::from_secs(65);

/// How a listen ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The switch announced itself.
    Heard {
        n: Box<Neighbor>,
        adapter: String,
        /// How: "Npcap", "raw socket", "lldpd", "systemd-networkd".
        via: &'static str,
        secs: u64,
        mac: Option<[u8; 6]>,
    },
    /// Listened the whole time, heard nothing.
    Silent {
        adapter: String,
        via: &'static str,
        secs: u64,
        mac: Option<[u8; 6]>,
    },
    /// No wired adapter, or none with a link.
    NoWire(String),
    /// The capture broke off (the adapter went away mid-listen).
    Lost {
        adapter: String,
        via: &'static str,
        why: String,
        mac: Option<[u8; 6]>,
    },
    /// Nothing here can listen; what would make it work.
    Cannot {
        adapter: String,
        fix: String,
        mac: Option<[u8; 6]>,
    },
    Stopped,
}

impl Outcome {
    /// The wired adapter's MAC, for asking a switch over SNMP.
    pub fn mac(&self) -> Option<[u8; 6]> {
        match self {
            Outcome::Heard { mac, .. }
            | Outcome::Silent { mac, .. }
            | Outcome::Lost { mac, .. }
            | Outcome::Cannot { mac, .. } => *mac,
            _ => None,
        }
    }
}

/// Words for an outcome that heard nothing.
pub fn silent_text(adapter: &str, secs: u64) -> String {
    format!(
        "Nothing heard on {adapter} in {secs} s. An unmanaged switch doesn't announce itself, and a \
         managed one can have LLDP and CDP turned off. If you know the switch's address, ask it over \
         SNMP which port has this computer's MAC."
    )
}

/// Listen on the wired adapter until the first announcement, `LISTEN`
/// or `cancel`. `status` is told what it's doing ("Listening on eth0
/// (raw socket)").
pub fn listen(lan: Option<Ipv4Addr>, cancel: &AtomicBool, status: &dyn Fn(String)) -> Outcome {
    let all = netconf::wired(lan);
    let Some(w) = netconf::pick_wired(&all) else {
        return Outcome::NoWire(
            "No wired network adapter found. Plug the cable into this computer's Ethernet port \
             (or a USB Ethernet adapter): Wi-Fi never carries the switch's announcements."
                .into(),
        );
    };
    if !w.up {
        return Outcome::NoWire(format!(
            "{} has no link: check the cable, and that the port's light comes on. With no link \
             there's nothing to hear (a dead drop, or a port that's shut down).",
            w.name
        ));
    }
    let start = Instant::now();
    let deadline = start + LISTEN;
    let (via, got) = match backend(w, deadline, cancel, status) {
        Ok(x) => x,
        Err(fix) => {
            return Outcome::Cannot {
                adapter: w.name.clone(),
                fix,
                mac: w.mac,
            };
        }
    };
    if cancel.load(Ordering::Relaxed) {
        return Outcome::Stopped;
    }
    let secs = start.elapsed().as_secs();
    match got {
        Err(why) => Outcome::Lost {
            adapter: w.name.clone(),
            via,
            why,
            mac: w.mac,
        },
        Ok(Some(n)) => Outcome::Heard {
            n: Box::new(n),
            adapter: w.name.clone(),
            via,
            secs,
            mac: w.mac,
        },
        Ok(None) => Outcome::Silent {
            adapter: w.name.clone(),
            via,
            secs,
            mac: w.mac,
        },
    }
}

/// What a backend heard: the switch, nothing by the deadline, or why the
/// capture broke off.
type Heard = Result<Option<Neighbor>, String>;

/// Whether a captured frame is the switch's: not one this computer sent
/// (lldpd here announces too).
fn from_switch(frame: &[u8], own: Option<[u8; 6]>) -> Option<Neighbor> {
    if own.is_some_and(|m| frame.get(6..12) == Some(&m[..])) {
        return None;
    }
    lldp::parse_frame(frame)
}

/// A frame and the capture are read in these slices, so Stop stays quick.
#[cfg(any(windows, target_os = "linux"))]
const SLICE: Duration = Duration::from_millis(500);

// ------------------------------------------------------------- Linux

#[cfg(target_os = "linux")]
type Found = Result<(&'static str, Heard), String>;

#[cfg(target_os = "linux")]
fn backend(w: &Wired, deadline: Instant, cancel: &AtomicBool, status: &dyn Fn(String)) -> Found {
    let raw = match raw::open(w) {
        Ok(sock) => {
            status(format!("Listening on {} (raw socket)", w.name));
            return Ok(("raw socket", raw::listen(&sock, w.mac, deadline, cancel)));
        }
        Err(e) => e,
    };
    // What the daemons on this computer already heard, asked again every
    // few seconds until something comes.
    let lldpcli = super::system_tool("lldpcli");
    let lldpd = lldpcli.as_ref().and_then(|p| ask_lldpd(p, &w.dev).ok());
    let networkd = networkd_manages(w)
        .then(|| super::system_tool("networkctl"))
        .flatten();
    if lldpd.is_none() && networkd.is_none() {
        return Err(linux_fix(w, &raw));
    }
    let via = if lldpd.is_some() {
        "lldpd"
    } else {
        "systemd-networkd"
    };
    status(format!("Asking {via} what it heard on {}", w.name));
    loop {
        let n = match (&lldpcli, &networkd) {
            (Some(p), _) if lldpd.is_some() => ask_lldpd(p, &w.dev).ok().flatten(),
            (_, Some(p)) => ask_networkd(p, &w.dev),
            _ => None,
        };
        if n.is_some() {
            return Ok((via, Ok(n)));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || cancel.load(Ordering::Relaxed) {
            return Ok((via, Ok(None)));
        }
        let until = Instant::now() + left.min(Duration::from_secs(3));
        while Instant::now() < until && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// What would make listening work: the raw socket's permission. Never
/// "start lldpd": it would announce this computer to the switch.
#[cfg(target_os = "linux")]
fn linux_fix(w: &Wired, raw: &std::io::Error) -> String {
    let fix = if raw.kind() != std::io::ErrorKind::PermissionDenied {
        format!("a raw socket on it failed: {raw}")
    } else if std::env::var_os("APPIMAGE").is_some() {
        "run is-alive with sudo (an AppImage can't be given the CAP_NET_RAW capability)".into()
    } else {
        "give is-alive the CAP_NET_RAW capability (sudo setcap cap_net_raw+ep on its program \
         file), or run it with sudo"
            .into()
    };
    format!(
        "Can't listen on {} here: {fix}. Then press Listen again. (If lldpd or systemd-networkd \
         is already running here, is-alive reads what it heard instead.)",
        w.name
    )
}

/// lldpd's neighbour on `dev`: Err when lldpcli can't reach lldpd.
#[cfg(target_os = "linux")]
fn ask_lldpd(lldpcli: &std::path::Path, dev: &str) -> Result<Option<Neighbor>, ()> {
    let out = super::output_within(
        std::process::Command::new(lldpcli).args(["-f", "json", "show", "neighbors"]),
        Duration::from_secs(5),
    )
    .map_err(|_| ())?;
    if !out.status.success() {
        return Err(());
    }
    Ok(lldp::parse_lldpcli(
        &String::from_utf8_lossy(&out.stdout),
        Some(dev),
    ))
}

/// Whether systemd-networkd manages the link (its state file says so).
#[cfg(target_os = "linux")]
fn networkd_manages(w: &Wired) -> bool {
    std::fs::read_to_string(format!("/run/systemd/netif/links/{}", w.index))
        .is_ok_and(|s| s.lines().any(|l| l.trim() == "ADMIN_STATE=configured"))
}

#[cfg(target_os = "linux")]
fn ask_networkd(networkctl: &std::path::Path, dev: &str) -> Option<Neighbor> {
    let out = super::output_within(
        std::process::Command::new(networkctl).args(["--no-pager", "lldp", dev]),
        Duration::from_secs(5),
    )
    .ok()?;
    lldp::parse_networkctl(&String::from_utf8_lossy(&out.stdout), Some(dev))
}

#[cfg(target_os = "linux")]
mod raw {
    use super::*;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    /// Classic BPF: accept a frame to either multicast address, drop the rest.
    fn filter() -> [libc::sock_filter; 9] {
        let op = |code: u16, jt: u8, jf: u8, k: u32| libc::sock_filter { code, jt, jf, k };
        const LD_W: u16 = 0x20;
        const LD_H: u16 = 0x28;
        const JEQ: u16 = 0x15;
        const RET: u16 = 0x06;
        [
            op(LD_W, 0, 0, 0),
            op(JEQ, 0, 2, 0x0180_c200),
            op(LD_H, 0, 0, 4),
            op(JEQ, 4, 3, 0x000e),
            op(JEQ, 0, 2, 0x0100_0ccc),
            op(LD_H, 0, 0, 4),
            op(JEQ, 1, 0, 0xcccc),
            op(RET, 0, 0, 0),
            op(RET, 0, 0, 0xffff),
        ]
    }

    fn check(r: libc::c_int) -> io::Result<()> {
        if r < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// A raw socket on the adapter, filtered and joined to both addresses.
    pub fn open(w: &Wired) -> io::Result<OwnedFd> {
        let proto = (libc::ETH_P_ALL as u16).to_be();
        let fd = unsafe {
            libc::socket(
                libc::AF_PACKET,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                i32::from(proto),
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let sock = unsafe { OwnedFd::from_raw_fd(fd) };
        let f = filter();
        let prog = libc::sock_fprog {
            len: f.len() as u16,
            filter: f.as_ptr().cast_mut(),
        };
        let set = |level, name, v: *const libc::c_void, len: usize| unsafe {
            check(libc::setsockopt(
                sock.as_raw_fd(),
                level,
                name,
                v,
                len as libc::socklen_t,
            ))
        };
        set(
            libc::SOL_SOCKET,
            libc::SO_ATTACH_FILTER,
            (&raw const prog).cast(),
            size_of::<libc::sock_fprog>(),
        )?;
        let mut sll: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        sll.sll_family = libc::AF_PACKET as u16;
        sll.sll_protocol = proto;
        sll.sll_ifindex = w.index as i32;
        check(unsafe {
            libc::bind(
                sock.as_raw_fd(),
                (&raw const sll).cast(),
                size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        })?;
        // Joining a link-layer multicast address only opens the adapter's
        // own filter to it: nothing goes out on the wire.
        for mac in [lldp::LLDP_MAC, lldp::CDP_MAC] {
            let mut mr: libc::packet_mreq = unsafe { std::mem::zeroed() };
            mr.mr_ifindex = w.index as i32;
            mr.mr_type = libc::PACKET_MR_MULTICAST as u16;
            mr.mr_alen = 6;
            mr.mr_address[..6].copy_from_slice(&mac);
            set(
                libc::SOL_PACKET,
                libc::PACKET_ADD_MEMBERSHIP,
                (&raw const mr).cast(),
                size_of::<libc::packet_mreq>(),
            )?;
        }
        let tv = libc::timeval {
            tv_sec: 0,
            tv_usec: SLICE.as_micros() as libc::suseconds_t,
        };
        set(
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&raw const tv).cast(),
            size_of::<libc::timeval>(),
        )?;
        Ok(sock)
    }

    pub fn listen(
        sock: &OwnedFd,
        own: Option<[u8; 6]>,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Heard {
        let mut buf = vec![0u8; 9216];
        while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
            let n = unsafe { libc::recv(sock.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
            if n < 0 {
                let e = io::Error::last_os_error();
                match e.kind() {
                    io::ErrorKind::WouldBlock
                    | io::ErrorKind::TimedOut
                    | io::ErrorKind::Interrupted => {}
                    // The adapter went down or away (unplugged, a USB adapter pulled).
                    _ => return Err(format!("the capture stopped: {e}")),
                }
                continue;
            }
            if n == 0 {
                continue;
            }
            if let Some(found) = from_switch(&buf[..n as usize], own) {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    #[test]
    fn the_filter_jumps_land_on_returns() {
        let f = filter();
        for (i, op) in f.iter().enumerate() {
            if op.code == 0x15 {
                for j in [op.jt, op.jf] {
                    assert!(i + 1 + usize::from(j) < f.len());
                }
            }
        }
        // Run it by hand on the two addresses and one other.
        let run = |dst: [u8; 6]| {
            let mut a = 0u32;
            let mut pc = 0;
            loop {
                let op = f[pc];
                match op.code {
                    0x20 => a = u32::from_be_bytes([dst[0], dst[1], dst[2], dst[3]]),
                    0x28 => a = u32::from(u16::from_be_bytes([dst[4], dst[5]])),
                    0x15 => {
                        pc += 1 + usize::from(if a == op.k { op.jt } else { op.jf });
                        continue;
                    }
                    _ => return op.k,
                }
                pc += 1;
            }
        };
        assert!(run(lldp::LLDP_MAC) > 0);
        assert!(run(lldp::CDP_MAC) > 0);
        assert_eq!(run([0x01, 0x80, 0xc2, 0, 0, 0x02]), 0);
        assert_eq!(run([0xff; 6]), 0);
    }
}

// ----------------------------------------------------------- Windows

#[cfg(windows)]
fn backend(
    w: &Wired,
    deadline: Instant,
    cancel: &AtomicBool,
    status: &dyn Fn(String),
) -> Result<(&'static str, Heard), String> {
    let lib = npcap::Npcap::load().ok_or_else(|| {
        format!(
            "Can't listen on {} here: install Npcap (npcap.com, the capture driver Wireshark \
             uses), then press Listen again. is-alive only listens through it.",
            w.name
        )
    })?;
    let cap = lib.open(&w.dev).map_err(|e| npcap_fix(&w.name, &e))?;
    // The one change to how the adapter receives, for this capture only.
    cap.all_multicast();
    status(format!("Listening on {} (Npcap)", w.name));
    Ok(("Npcap", cap.listen(w.mac, deadline, cancel)))
}

/// Why Npcap couldn't open the adapter, in words: admin-only mode said
/// so, else Npcap's own error.
#[cfg_attr(not(windows), allow(dead_code))]
fn npcap_fix(adapter: &str, err: &str) -> String {
    let e = err.to_ascii_lowercase();
    if e.contains("access is denied") || e.contains("error code 5") || e.contains("(5)") {
        format!(
            "Npcap is installed to let only administrators capture, so it refused to open \
             {adapter}. Run is-alive as administrator, or reinstall Npcap without \
             \"Restrict Npcap driver's access to Administrators only\"."
        )
    } else {
        format!("Npcap couldn't open {adapter}: {err}")
    }
}

#[cfg(windows)]
mod npcap {
    use super::*;
    use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
    use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

    #[repr(C)]
    struct BpfProgram {
        len: c_uint,
        insns: *mut c_void,
    }

    /// pcap_pkthdr on Windows: a timeval of two 32-bit longs.
    #[repr(C)]
    struct PktHdr {
        sec: i32,
        usec: i32,
        caplen: u32,
        len: u32,
    }

    type OpenLive =
        unsafe extern "C" fn(*const c_char, c_int, c_int, c_int, *mut c_char) -> *mut c_void;
    type Compile =
        unsafe extern "C" fn(*mut c_void, *mut BpfProgram, *const c_char, c_int, u32) -> c_int;
    type SetFilter = unsafe extern "C" fn(*mut c_void, *mut BpfProgram) -> c_int;
    type FreeCode = unsafe extern "C" fn(*mut BpfProgram);
    type NextEx = unsafe extern "C" fn(*mut c_void, *mut *mut PktHdr, *mut *const u8) -> c_int;
    type Close = unsafe extern "C" fn(*mut c_void);
    type GetErr = unsafe extern "C" fn(*mut c_void) -> *const c_char;
    type OidSet = unsafe extern "C" fn(*mut c_void, u32, *const c_void, *mut usize) -> c_int;

    pub struct Npcap {
        dll: HMODULE,
        open_live: OpenLive,
        compile: Compile,
        set_filter: SetFilter,
        free_code: FreeCode,
        next_ex: NextEx,
        close: Close,
        get_err: GetErr,
        /// Not in every Npcap: optional.
        oid_set: Option<OidSet>,
    }

    impl Drop for Npcap {
        fn drop(&mut self) {
            unsafe { FreeLibrary(self.dll) };
        }
    }

    pub struct Capture<'a> {
        lib: &'a Npcap,
        p: *mut c_void,
        /// `all_multicast` took: put the filter back before closing.
        widened: std::cell::Cell<bool>,
    }

    impl Drop for Capture<'_> {
        fn drop(&mut self) {
            if self.widened.get() {
                // DIRECTED | MULTICAST | BROADCAST: Npcap's own filter for
                // a capture that isn't promiscuous.
                self.packet_filter(0x1 | 0x2 | 0x8);
            }
            unsafe { (self.lib.close)(self.p) };
        }
    }

    impl Npcap {
        /// System32\Npcap\wpcap.dll by its full path (Packet.dll beside it
        /// loads from there too); None when Npcap isn't installed.
        pub fn load() -> Option<Npcap> {
            let mut dir = [0u16; 260];
            let n = unsafe { GetSystemDirectoryW(dir.as_mut_ptr(), dir.len() as u32) } as usize;
            if n == 0 || n >= dir.len() {
                return None;
            }
            let mut path: Vec<u16> = dir[..n].to_vec();
            path.extend("\\Npcap\\wpcap.dll".encode_utf16());
            path.push(0);
            let dll = unsafe {
                LoadLibraryExW(
                    path.as_ptr(),
                    std::ptr::null_mut(),
                    LOAD_WITH_ALTERED_SEARCH_PATH,
                )
            };
            if dll.is_null() {
                return None;
            }
            macro_rules! f {
                ($name:literal, $t:ty) => {
                    match unsafe { GetProcAddress(dll, concat!($name, "\0").as_ptr()) } {
                        Some(p) => unsafe {
                            std::mem::transmute::<unsafe extern "system" fn() -> isize, $t>(p)
                        },
                        None => {
                            unsafe { FreeLibrary(dll) };
                            return None;
                        }
                    }
                };
            }
            let oid_set = unsafe { GetProcAddress(dll, c"pcap_oid_set_request".as_ptr().cast()) }
                .map(|p| unsafe {
                    std::mem::transmute::<unsafe extern "system" fn() -> isize, OidSet>(p)
                });
            Some(Npcap {
                oid_set,
                open_live: f!("pcap_open_live", OpenLive),
                compile: f!("pcap_compile", Compile),
                set_filter: f!("pcap_setfilter", SetFilter),
                free_code: f!("pcap_freecode", FreeCode),
                next_ex: f!("pcap_next_ex", NextEx),
                close: f!("pcap_close", Close),
                get_err: f!("pcap_geterr", GetErr),
                dll,
            })
        }

        /// The adapter by its GUID, filtered to the two multicast addresses.
        pub fn open(&self, guid: &str) -> Result<Capture<'_>, String> {
            let dev = format!("\\Device\\NPF_{guid}\0");
            let mut err = [0 as c_char; 512];
            // Not promiscuous: the adapter keeps its own receive filter
            // (but see `all_multicast`).
            let p = unsafe {
                (self.open_live)(
                    dev.as_ptr().cast(),
                    2048,
                    0,
                    SLICE.as_millis() as c_int,
                    err.as_mut_ptr(),
                )
            };
            if p.is_null() {
                let e = unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy();
                return Err(crate::net::sanitize(&e));
            }
            let cap = Capture {
                lib: self,
                p,
                widened: std::cell::Cell::new(false),
            };
            let expr = c"ether dst 01:80:c2:00:00:0e or ether dst 01:00:0c:cc:cc:cc";
            let mut prog = BpfProgram {
                len: 0,
                insns: std::ptr::null_mut(),
            };
            let ok = unsafe { (self.compile)(p, &mut prog, expr.as_ptr(), 1, 0xffff_ffff) } == 0
                && unsafe { (self.set_filter)(p, &mut prog) } == 0;
            if !prog.insns.is_null() {
                unsafe { (self.free_code)(&mut prog) };
            }
            if !ok {
                let e = unsafe { CStr::from_ptr((self.get_err)(p)) }.to_string_lossy();
                return Err(crate::net::sanitize(&e));
            }
            Ok(cap)
        }
    }

    impl Capture<'_> {
        /// The only change to how the adapter receives, and only while this
        /// capture is open: ask Npcap to let all multicast frames through to
        /// it (NDIS_PACKET_TYPE_ALL_MULTICAST, with directed, listed
        /// multicast and broadcast as before), since the adapter otherwise
        /// drops multicast to addresses no program asked for, and CDP's
        /// 01:00:0c:cc:cc:cc (and LLDP's, without Windows' LLDP driver) is
        /// one. Only for the listen: dropping the capture (the first
        /// announcement, 65 s, Stop) sets the filter back first, and Npcap
        /// drops a capture's filter when it closes anyway. Not promiscuous:
        /// frames to other computers' addresses still aren't received.
        /// Best effort: an Npcap without pcap_oid_set_request, or one that
        /// refuses, listens as it is. (The owner chose this over
        /// promiscuous capture; removing this call removes the change.)
        pub fn all_multicast(&self) {
            // DIRECTED | MULTICAST | ALL_MULTICAST | BROADCAST.
            if self.packet_filter(0x1 | 0x2 | 0x4 | 0x8) {
                self.widened.set(true);
            }
        }

        /// Set this capture's NDIS packet filter; whether Npcap took it.
        fn packet_filter(&self, filter: u32) -> bool {
            const OID_GEN_CURRENT_PACKET_FILTER: u32 = 0x0001_010e;
            let Some(set) = self.lib.oid_set else {
                return false;
            };
            let mut len = size_of::<u32>();
            let r = unsafe {
                set(
                    self.p,
                    OID_GEN_CURRENT_PACKET_FILTER,
                    (&raw const filter).cast(),
                    &mut len,
                )
            };
            r == 0
        }

        pub fn listen(
            &self,
            own: Option<[u8; 6]>,
            deadline: Instant,
            cancel: &AtomicBool,
        ) -> Heard {
            while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
                let mut h: *mut PktHdr = std::ptr::null_mut();
                let mut d: *const u8 = std::ptr::null();
                match unsafe { (self.lib.next_ex)(self.p, &mut h, &mut d) } {
                    1 if !h.is_null() && !d.is_null() => {
                        let len = unsafe { (*h).caplen }.min(2048) as usize;
                        let frame = unsafe { std::slice::from_raw_parts(d, len) };
                        if let Some(n) = from_switch(frame, own) {
                            return Ok(Some(n));
                        }
                    }
                    0 | 1 => {}
                    // -1: the adapter went away (unplugged, a USB adapter
                    // pulled) or the driver failed; -2 can't happen live.
                    _ => {
                        let e =
                            unsafe { CStr::from_ptr((self.lib.get_err)(self.p)) }.to_string_lossy();
                        return Err(format!("the capture stopped: {}", crate::net::sanitize(&e)));
                    }
                }
            }
            Ok(None)
        }
    }
}

// -------------------------------------------------------------- other

#[cfg(not(any(windows, target_os = "linux")))]
fn backend(
    w: &Wired,
    _: Instant,
    _: &AtomicBool,
    _: &dyn Fn(String),
) -> Result<(&'static str, Heard), String> {
    Err(format!(
        "Listening on {} isn't supported on this system.",
        w.name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npcap_in_admin_only_mode_says_so() {
        let e = npcap_fix("Ethernet", "Error opening adapter: Access is denied.  (5)");
        assert!(e.contains("only administrators"), "{e}");
        let e = npcap_fix(
            "Ethernet",
            "Error opening adapter: The system cannot find the device specified. (20)",
        );
        assert!(e.starts_with("Npcap couldn't open Ethernet"), "{e}");
    }

    #[test]
    fn our_own_announcements_are_not_the_switch() {
        let f = lldp::sample::lldp();
        let src: [u8; 6] = f[6..12].try_into().unwrap();
        assert!(from_switch(&f, None).is_some());
        assert!(from_switch(&f, Some([0x02, 0, 0, 0, 0, 1])).is_some());
        assert_eq!(from_switch(&f, Some(src)), None);
    }

    #[test]
    fn the_adapter_with_the_lan_and_a_link_is_picked() {
        let w = |name: &str, up, lan| Wired {
            name: name.into(),
            up,
            lan,
            ..Default::default()
        };
        let all = [
            w("eth0", false, false),
            w("eth1", true, false),
            w("eth2", true, true),
        ];
        assert_eq!(netconf::pick_wired(&all).unwrap().name, "eth2");
        assert_eq!(netconf::pick_wired(&all[..2]).unwrap().name, "eth1");
        assert_eq!(netconf::pick_wired(&all[..1]).unwrap().name, "eth0");
        assert_eq!(netconf::pick_wired(&[]), None);
    }
}
