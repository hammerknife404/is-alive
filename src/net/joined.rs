//! Is another computer joined to Active Directory? Asked without
//! credentials: the first step of a sign-in, on SMB (445) or, failing
//! that, RDP with Network Level Authentication (3389). A Windows server
//! answers that step with an NTLM challenge naming itself and its domain
//! (computer name, domain, and both as DNS names), before any user name
//! or password is sent. The sign-in is never finished: nothing is sent
//! after the challenge, and no account is tried, so nothing can lock.
//!
//! What it can't see: an Entra ID (Azure AD) join, which shows only to
//! someone signed in. A computer that isn't in a domain is therefore
//! "Not AD-joined (Entra not checked)", never called a workgroup machine
//! as a fact.

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

/// The most hosts one run takes.
pub const MAX_HOSTS: usize = 32;
const CONNECT: Duration = Duration::from_millis(2000);
const REPLY: Duration = Duration::from_millis(3000);
/// The most a reply may claim to be long: a challenge is a few hundred
/// bytes, and the length comes from the other end.
const MAX_REPLY: usize = 64 * 1024;

pub const COLUMNS: &[&str] = &[
    "host",
    "computer",
    "domain",
    "dns_domain",
    "dns_name",
    "verdict",
    "port",
    "detail",
];

/// What a host's challenge named. Any of them may be missing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Names {
    /// The computer's short (NetBIOS) name.
    pub computer: Option<String>,
    /// Its domain's short name; its own name again when it has none.
    pub domain: Option<String>,
    /// The computer's DNS name.
    pub dns_name: Option<String>,
    /// Its domain's DNS name.
    pub dns_domain: Option<String>,
    /// The forest's DNS name.
    pub forest: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// In the Active Directory domain named.
    Ad(String),
    /// It answered and names no domain but itself.
    NotAd,
    /// It answered on a port, but not with its names.
    CantTell,
    /// Neither port answered.
    NoAnswer,
}

impl Verdict {
    pub fn text(&self) -> String {
        match self {
            Verdict::Ad(d) => format!("AD: {d}"),
            Verdict::NotAd => "Not AD-joined (Entra not checked)".into(),
            Verdict::CantTell => "Answers, but doesn't say".into(),
            Verdict::NoAnswer => "No answer on 445 or 3389".into(),
        }
    }
}

/// One host's answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// As typed.
    pub host: String,
    pub names: Names,
    pub verdict: Verdict,
    /// The port that named it (445 or 3389).
    pub port: Option<u16>,
    /// Why it couldn't tell, or what each port said.
    pub detail: String,
}

impl Answer {
    pub fn row(&self) -> Vec<String> {
        let s = |o: &Option<String>| o.clone().unwrap_or_default();
        vec![
            self.host.clone(),
            s(&self.names.computer),
            s(&self.names.domain),
            s(&self.names.dns_domain),
            s(&self.names.dns_name),
            self.verdict.text(),
            self.port.map(|p| p.to_string()).unwrap_or_default(),
            self.detail.clone(),
        ]
    }

    /// One line for Results and Copy.
    pub fn line(&self) -> String {
        let mut s = format!("{}: {}", self.host, self.verdict.text());
        if let Some(c) = &self.names.computer {
            s += &format!(" · computer {c}");
        }
        if let Some(p) = self.port {
            s += &format!(" · port {p}");
        }
        if !self.detail.is_empty() {
            s += &format!(" · {}", self.detail);
        }
        s
    }
}

/// The hosts in what was typed: names or addresses, split on spaces,
/// commas, semicolons and new lines, each once, at most `MAX_HOSTS`. An
/// error names what isn't one (a range or a subnet is never expanded:
/// this asks the computers you name, not a network).
pub fn hosts(text: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for h in text
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .map(str::trim)
        .filter(|h| !h.is_empty())
    {
        if h.contains('/')
            || h.contains("..")
            || (h.contains('-') && h.parse::<IpAddr>().is_err() && range_like(h))
        {
            return Err(format!(
                "{h} is a range: name the computers one by one (up to {MAX_HOSTS})"
            ));
        }
        let ok = h.len() <= 253
            && (h.parse::<IpAddr>().is_ok()
                || h.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_'));
        if !ok {
            return Err(format!("{h} isn't a computer's name or address"));
        }
        if !out.iter().any(|o| o.eq_ignore_ascii_case(h)) {
            out.push(h.to_string());
        }
    }
    if out.len() > MAX_HOSTS {
        return Err(format!(
            "{} computers named: {MAX_HOSTS} at most in one run",
            out.len()
        ));
    }
    Ok(out)
}

/// "10.0.0.1-20": digits and dots either side of a dash.
fn range_like(h: &str) -> bool {
    h.split_once('-').is_some_and(|(a, b)| {
        let num = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.');
        num(a) && num(b)
    })
}

/// What the names say. A domain member's challenge gives its domain's
/// short name (not its own) and a DNS domain; a computer in no domain
/// names itself as its domain.
pub fn verdict(n: &Names) -> Verdict {
    let same = |a: &Option<String>, b: &Option<String>| match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    };
    let Some(domain) = n.domain.as_deref().filter(|d| !d.is_empty()) else {
        return if n.computer.is_some() {
            Verdict::NotAd
        } else {
            Verdict::CantTell
        };
    };
    let dns = n.dns_domain.as_deref().filter(|d| !d.is_empty());
    if same(&n.domain, &n.computer) || same(&n.dns_domain, &n.dns_name) {
        return Verdict::NotAd;
    }
    match dns {
        Some(d) => Verdict::Ad(d.to_lowercase()),
        // A short domain name and no DNS one: a pre-2000 domain, or a
        // Samba workgroup. Not something to call Active Directory.
        None => {
            let _ = domain;
            Verdict::NotAd
        }
    }
}

/// Ask `host`: SMB first, then RDP.
pub fn ask(host: &str) -> Answer {
    let mut a = Answer {
        host: host.to_string(),
        names: Names::default(),
        verdict: Verdict::NoAnswer,
        port: None,
        detail: String::new(),
    };
    let ip = match super::resolve(host, false) {
        Ok(ip) => ip,
        Err(e) => {
            a.verdict = Verdict::CantTell;
            a.detail = format!("name not found: {e}");
            return a;
        }
    };
    let mut said = Vec::new();
    let mut answered = false;
    for port in [445u16, 3389] {
        // Shut, filtered or not there: the next port.
        let mut tcp = match connect(ip, port) {
            Ok(t) => t,
            Err(e) => {
                said.push(format!("{port}: {}", closed(&e)));
                continue;
            }
        };
        answered = true;
        let names = if port == 445 {
            smb_exchange(&mut tcp)
        } else {
            rdp(&mut tcp, host)
        };
        match names {
            Ok(n) if n != Names::default() => {
                a.verdict = verdict(&n);
                a.names = n;
                a.port = Some(port);
                a.detail.clear();
                return a;
            }
            Ok(_) => said.push(format!("{port}: no names in its reply")),
            Err(e) => said.push(format!("{port}: {e}")),
        }
    }
    a.verdict = if answered {
        Verdict::CantTell
    } else {
        Verdict::NoAnswer
    };
    a.detail = said.join("; ");
    a
}

/// Why a port didn't open, in a word.
fn closed(e: &io::Error) -> &'static str {
    match e.kind() {
        io::ErrorKind::ConnectionRefused => "closed",
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "no reply",
        _ => "unreachable",
    }
}

fn connect(ip: IpAddr, port: u16) -> io::Result<TcpStream> {
    let tcp = TcpStream::connect_timeout(&SocketAddr::new(ip, port), CONNECT)?;
    tcp.set_read_timeout(Some(REPLY))?;
    tcp.set_write_timeout(Some(REPLY))?;
    let _ = tcp.set_nodelay(true);
    Ok(tcp)
}

fn said(what: &str) -> io::Error {
    io::Error::other(what.to_string())
}

// ---------------------------------------------------------------- NTLM

/// "NTLMSSP\0", then the message's type.
const NTLMSSP: &[u8] = b"NTLMSSP\0";

/// NTLM's first message (NEGOTIATE): which options the client takes,
/// no domain, no workstation, no name. Asking for the target's
/// information is what makes the reply carry its names.
fn ntlm_negotiate() -> Vec<u8> {
    // Unicode, OEM, request target, NTLM, always sign, extended session
    // security, version, 128-bit, 56-bit.
    const FLAGS: u32 = 0x0000_0001
        | 0x0000_0002
        | 0x0000_0004
        | 0x0000_0200
        | 0x0000_8000
        | 0x0008_0000
        | 0x0200_0000
        | 0x2000_0000
        | 0x8000_0000;
    let mut m = NTLMSSP.to_vec();
    m.extend(1u32.to_le_bytes());
    m.extend(FLAGS.to_le_bytes());
    // Domain and workstation: empty, both placed at the message's end.
    for _ in 0..2 {
        m.extend(0u16.to_le_bytes());
        m.extend(0u16.to_le_bytes());
        m.extend(40u32.to_le_bytes());
    }
    // Version: 10.0, build 19041, NTLM revision 15.
    m.extend([10, 0]);
    m.extend(19041u16.to_le_bytes());
    m.extend([0, 0, 0, 15]);
    m
}

/// The names in NTLM's second message (CHALLENGE), wherever it starts
/// in `reply` (it comes wrapped by SMB, SPNEGO or CredSSP). Every length
/// and offset in it is the server's: each is checked against the bytes
/// there are.
pub fn challenge_names(reply: &[u8]) -> Option<Names> {
    let at = reply.windows(NTLMSSP.len()).position(|w| w == NTLMSSP)?;
    let m = &reply[at..];
    let u16_at = |i: usize| Some(u16::from_le_bytes(m.get(i..i + 2)?.try_into().ok()?) as usize);
    let u32_at = |i: usize| Some(u32::from_le_bytes(m.get(i..i + 4)?.try_into().ok()?) as usize);
    if u32_at(8)? != 2 {
        return None;
    }
    let field = |i: usize| -> Option<&[u8]> {
        let (len, off) = (u16_at(i)?, u32_at(i + 4)?);
        m.get(off..off.checked_add(len)?)
    };
    let mut n = Names::default();
    // TargetInfo: pairs of (id, length, value) up to id 0.
    let mut info = field(40).unwrap_or_default();
    while info.len() >= 4 {
        let id = u16::from_le_bytes([info[0], info[1]]);
        let len = u16::from_le_bytes([info[2], info[3]]) as usize;
        let Some(value) = info.get(4..4 + len) else {
            break;
        };
        let text = utf16(value);
        match id {
            0 => break,
            1 => n.computer = text,
            2 => n.domain = text,
            3 => n.dns_name = text,
            4 => n.dns_domain = text,
            5 => n.forest = text,
            _ => {}
        }
        info = &info[4 + len..];
    }
    // No target information (an old server): the target name alone is
    // its domain, or itself.
    if n == Names::default() {
        n.domain = field(12).and_then(utf16);
    }
    Some(n)
}

/// UTF-16LE text, cleaned of anything that isn't printable; None when
/// nothing is left.
fn utf16(b: &[u8]) -> Option<String> {
    let units: Vec<u16> = b
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let s: String = String::from_utf16_lossy(&units)
        .chars()
        .filter(|c| !c.is_control() && *c != char::REPLACEMENT_CHARACTER)
        .take(255)
        .collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

// ----------------------------------------------------------------- DER

/// A DER element: tag, length, contents.
fn der(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![tag];
    match body.len() {
        n if n < 0x80 => v.push(n as u8),
        n if n <= 0xff => v.extend([0x81, n as u8]),
        n => v.extend([0x82, (n >> 8) as u8, n as u8]),
    }
    v.extend(body);
    v
}

/// SPNEGO's first token offering NTLM, around NTLM's first message.
fn spnego_init(ntlm: &[u8]) -> Vec<u8> {
    // 1.3.6.1.5.5.2 (SPNEGO) and 1.3.6.1.4.1.311.2.2.10 (NTLMSSP).
    const SPNEGO: &[u8] = &[0x06, 0x06, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x02];
    const NTLM: &[u8] = &[
        0x06, 0x0a, 0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x02, 0x02, 0x0a,
    ];
    let mechs = der(0xa0, &der(0x30, NTLM));
    let token = der(0xa2, &der(0x04, ntlm));
    let init = der(0xa0, &der(0x30, &[mechs, token].concat()));
    der(0x60, &[SPNEGO, &init].concat())
}

// ----------------------------------------------------------------- SMB

const SMB2: &[u8] = b"\xfeSMB";
const MORE_PROCESSING: u32 = 0xC000_0016;

/// An SMB2 header for `command`, message `id`.
fn smb_header(command: u16, id: u64) -> Vec<u8> {
    let mut h = SMB2.to_vec();
    h.extend(64u16.to_le_bytes()); // structure size
    h.extend(0u16.to_le_bytes()); // credit charge
    h.extend(0u32.to_le_bytes()); // status
    h.extend(command.to_le_bytes());
    h.extend(1u16.to_le_bytes()); // credits asked for
    h.extend(0u32.to_le_bytes()); // flags
    h.extend(0u32.to_le_bytes()); // next command
    h.extend(id.to_le_bytes());
    h.extend(0u32.to_le_bytes()); // process
    h.extend(0u32.to_le_bytes()); // tree
    h.extend(0u64.to_le_bytes()); // session
    h.extend([0u8; 16]); // signature
    h
}

/// With the 4-byte length SMB over TCP puts before each message.
fn framed(m: Vec<u8>) -> Vec<u8> {
    let mut v = (m.len() as u32).to_be_bytes().to_vec();
    v.extend(m);
    v
}

fn smb_negotiate() -> Vec<u8> {
    // SMB 2.0.2, 2.1, 3.0 and 3.0.2: none needs the negotiate contexts
    // 3.1.1 does, and every Windows since Vista speaks one of them.
    const DIALECTS: [u16; 4] = [0x0202, 0x0210, 0x0300, 0x0302];
    let mut m = smb_header(0, 0);
    m.extend(36u16.to_le_bytes());
    m.extend((DIALECTS.len() as u16).to_le_bytes());
    m.extend(1u16.to_le_bytes()); // signing enabled
    m.extend(0u16.to_le_bytes());
    m.extend(0u32.to_le_bytes()); // capabilities
    m.extend(*b"is-alive-domain?"); // client GUID: any 16 bytes
    m.extend(0u64.to_le_bytes()); // start time
    for d in DIALECTS {
        m.extend(d.to_le_bytes());
    }
    framed(m)
}

fn smb_session_setup(token: &[u8]) -> Vec<u8> {
    let mut m = smb_header(1, 1);
    m.extend(25u16.to_le_bytes());
    m.push(0); // flags
    m.push(1); // signing enabled
    m.extend(0u32.to_le_bytes()); // capabilities
    m.extend(0u32.to_le_bytes()); // channel
    m.extend(88u16.to_le_bytes()); // where the token starts: 64 + 24
    m.extend((token.len() as u16).to_le_bytes());
    m.extend(0u64.to_le_bytes()); // previous session
    m.extend(token);
    framed(m)
}

/// One SMB message: its length, then that many bytes.
fn smb_read(tcp: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    tcp.read_exact(&mut len)?;
    let n = u32::from_be_bytes([0, len[1], len[2], len[3]]) as usize;
    if !(64..=MAX_REPLY).contains(&n) {
        return Err(said("not an SMB reply"));
    }
    let mut m = vec![0u8; n];
    tcp.read_exact(&mut m)?;
    Ok(m)
}

fn smb_status(m: &[u8]) -> io::Result<u32> {
    if m.get(..4) != Some(SMB2) {
        // "\xffSMB": it speaks only SMB 1, which this doesn't.
        return Err(said(if m.get(..4) == Some(b"\xffSMB") {
            "SMB 1 only"
        } else {
            "not an SMB reply"
        }));
    }
    m.get(8..12)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| said("short SMB reply"))
}

/// The names from an SMB sign-in's first step, over a connection to 445.
fn smb_exchange(tcp: &mut (impl Read + Write)) -> io::Result<Names> {
    tcp.write_all(&smb_negotiate())?;
    let m = smb_read(tcp)?;
    match smb_status(&m)? {
        0 => {}
        s => return Err(said(&format!("SMB refused to negotiate ({s:#010x})"))),
    }
    tcp.write_all(&smb_session_setup(&spnego_init(&ntlm_negotiate())))?;
    let m = smb_read(tcp)?;
    match smb_status(&m)? {
        MORE_PROCESSING => {}
        0xC000_00BB => return Err(said("SMB sign-in with NTLM is turned off there")),
        s => return Err(said(&format!("SMB didn't offer a challenge ({s:#010x})"))),
    }
    challenge_names(&m[64..]).ok_or_else(|| said("no NTLM challenge in the SMB reply"))
}

// ----------------------------------------------------------------- RDP

/// RDP's connection request asking for TLS and Network Level
/// Authentication (CredSSP), which is where the NTLM challenge is.
const RDP_REQUEST: [u8; 19] = [
    0x03, 0x00, 0x00, 0x13, // TPKT, 19 bytes
    0x0e, 0xe0, 0x00, 0x00, 0x00, 0x00, 0x00, // X.224 connection request
    0x01, 0x00, 0x08, 0x00, // negotiation request, 8 bytes
    0x03, 0x00, 0x00, 0x00, // TLS and CredSSP
];

/// What the server picked from `RDP_REQUEST`: Ok(true) when it will do
/// CredSSP.
fn rdp_picked(reply: &[u8]) -> io::Result<bool> {
    if reply.len() < 11 || reply[0] != 0x03 || reply[5] != 0xd0 {
        return Err(said("not an RDP reply"));
    }
    match (reply.get(11), reply.get(15)) {
        // Negotiation response: the protocol picked.
        (Some(0x02), Some(p)) => Ok(p & 0x0a != 0),
        // Negotiation failure.
        (Some(0x03), _) => Err(said("RDP there doesn't take Network Level Authentication")),
        // No negotiation at all: plain RDP security.
        _ => Ok(false),
    }
}

/// CredSSP's first message (TSRequest) around NTLM's first message.
fn ts_request(ntlm: &[u8]) -> Vec<u8> {
    let version = der(0xa0, &[0x02, 0x01, 0x06]);
    let token = der(0x30, &der(0xa0, &der(0x04, ntlm)));
    let tokens = der(0xa1, &der(0x30, &token));
    der(0x30, &[version, tokens].concat())
}

/// The names from an RDP sign-in's first step, over a connection to
/// 3389, when the server takes Network Level Authentication.
fn rdp(tcp: &mut TcpStream, host: &str) -> io::Result<Names> {
    tcp.write_all(&RDP_REQUEST)?;
    let mut reply = [0u8; 19];
    let n = read_some(tcp, &mut reply)?;
    if !rdp_picked(&reply[..n])? {
        return Err(said(
            "RDP answers without Network Level Authentication, so it names nothing before a sign-in",
        ));
    }
    // Its certificate is its own (self-signed on almost every PC):
    // nothing secret is sent, so it isn't checked.
    let (mut conn, _) = super::tls::client(host)?;
    super::tls::handshake(&mut conn, tcp)?;
    let mut tls = rustls::Stream::new(&mut conn, tcp);
    tls.write_all(&ts_request(&ntlm_negotiate()))?;
    tls.flush()?;
    let mut buf = vec![0u8; 4096];
    let n = read_some(&mut tls, &mut buf)?;
    challenge_names(&buf[..n]).ok_or_else(|| said("no NTLM challenge in the RDP reply"))
}

/// One read's worth: what the other end sent in reply (a challenge
/// arrives whole in one).
fn read_some(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    match r.read(buf)? {
        0 => Err(said("it closed the connection")),
        n => Ok(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An NTLM challenge as a Windows server sends it, naming these.
    fn challenge(pairs: &[(u16, &str)], target: &str) -> Vec<u8> {
        let wide = |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(u16::to_le_bytes).collect() };
        let mut info = Vec::new();
        for (id, v) in pairs {
            let v = wide(v);
            info.extend(id.to_le_bytes());
            info.extend((v.len() as u16).to_le_bytes());
            info.extend(v);
        }
        info.extend([0, 0, 0, 0]);
        let target = wide(target);
        let mut m = NTLMSSP.to_vec();
        m.extend(2u32.to_le_bytes());
        m.extend((target.len() as u16).to_le_bytes());
        m.extend((target.len() as u16).to_le_bytes());
        m.extend(56u32.to_le_bytes());
        m.extend(0xe28a_8215u32.to_le_bytes());
        m.extend([1, 2, 3, 4, 5, 6, 7, 8]); // the challenge itself
        m.extend([0u8; 8]);
        m.extend((info.len() as u16).to_le_bytes());
        m.extend((info.len() as u16).to_le_bytes());
        m.extend((56 + target.len() as u32).to_le_bytes());
        m.extend([10, 0, 0x61, 0x4a, 0, 0, 0, 15]);
        m.extend(target);
        m.extend(info);
        m
    }

    fn member() -> Vec<u8> {
        challenge(
            &[
                (2, "HARBOR"),
                (1, "FRONTDESK-PC"),
                (4, "harbor.example"),
                (3, "frontdesk-pc.harbor.example"),
                (5, "harbor.example"),
            ],
            "HARBOR",
        )
    }

    fn standalone() -> Vec<u8> {
        challenge(
            &[
                (2, "KIOSK-01"),
                (1, "KIOSK-01"),
                (4, "KIOSK-01"),
                (3, "KIOSK-01"),
            ],
            "KIOSK-01",
        )
    }

    #[test]
    fn a_domain_members_challenge_names_its_domain() {
        let n = challenge_names(&member()).unwrap();
        assert_eq!(n.computer.as_deref(), Some("FRONTDESK-PC"));
        assert_eq!(n.domain.as_deref(), Some("HARBOR"));
        assert_eq!(n.dns_name.as_deref(), Some("frontdesk-pc.harbor.example"));
        assert_eq!(n.dns_domain.as_deref(), Some("harbor.example"));
        assert_eq!(verdict(&n), Verdict::Ad("harbor.example".into()));
        assert_eq!(verdict(&n).text(), "AD: harbor.example");
        // Wrapped, as SMB and CredSSP send it: found wherever it starts.
        let wrapped = [&[0xa1, 0x81, 0x90, 0x30][..], &member()].concat();
        assert_eq!(challenge_names(&wrapped), Some(n));
    }

    /// A computer in no domain names itself as its domain. That is all
    /// that can be said: an Entra join doesn't show without signing in.
    #[test]
    fn a_computer_in_no_domain_is_not_called_a_workgroup_machine() {
        let n = challenge_names(&standalone()).unwrap();
        assert_eq!(verdict(&n), Verdict::NotAd);
        let text = verdict(&n).text();
        assert_eq!(text, "Not AD-joined (Entra not checked)");
        assert!(!text.to_lowercase().contains("workgroup"));
        // A short domain name with no DNS domain isn't called AD either.
        let samba = challenge_names(&challenge(&[(2, "WORKGROUP"), (1, "NAS")], "WORKGROUP"));
        assert_eq!(verdict(&samba.unwrap()), Verdict::NotAd);
        assert_eq!(verdict(&Names::default()), Verdict::CantTell);
    }

    /// Anything can be listening on 445 or 3389. Its reply cut short,
    /// corrupted or made of noise is "can't tell", never a panic.
    #[test]
    fn hostile_challenges_never_panic() {
        crate::hostile::each(&member(), |r| {
            if let Some(n) = challenge_names(r) {
                let _ = verdict(&n).text();
            }
            let _ = rdp_picked(r);
            let _ = smb_status(r);
            let mut io = Talk::new(r.to_vec());
            let _ = smb_exchange(&mut io);
        });
    }

    /// The other end of a connection: says `reply`, takes anything.
    struct Talk {
        reply: std::io::Cursor<Vec<u8>>,
        sent: Vec<u8>,
    }

    impl Talk {
        fn new(reply: Vec<u8>) -> Self {
            Self {
                reply: std::io::Cursor::new(reply),
                sent: Vec::new(),
            }
        }
    }

    impl Read for Talk {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.reply.read(buf)
        }
    }

    impl Write for Talk {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.sent.extend(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// An SMB2 reply: `status`, then `body` after the 64-byte header.
    fn smb_reply(status: u32, body: &[u8]) -> Vec<u8> {
        let mut m = smb_header(0, 0);
        m[8..12].copy_from_slice(&status.to_le_bytes());
        m.extend(body);
        framed(m)
    }

    #[test]
    fn the_smb_exchange_stops_at_the_challenge() {
        let server = [
            smb_reply(0, &[0u8; 64]),
            smb_reply(
                MORE_PROCESSING,
                &[&[9, 0, 0, 0, 72, 0, 0, 0][..], &member()].concat(),
            ),
        ]
        .concat();
        let mut io = Talk::new(server);
        let n = smb_exchange(&mut io).unwrap();
        assert_eq!(n.dns_domain.as_deref(), Some("harbor.example"));
        // Two messages went out, and no third: negotiate, then the first
        // step of a sign-in with no name in it.
        let first = u32::from_be_bytes(io.sent[..4].try_into().unwrap()) as usize;
        let second = &io.sent[4 + first..];
        let len = u32::from_be_bytes(second[..4].try_into().unwrap()) as usize;
        assert_eq!(
            second.len(),
            4 + len,
            "nothing after the sign-in's first step"
        );
        assert_eq!(&io.sent[4..8], SMB2);
        assert_eq!(u16::from_le_bytes([second[4 + 12], second[4 + 13]]), 1);
        let ntlm = ntlm_negotiate();
        assert!(second.windows(ntlm.len()).any(|w| w == ntlm));
        assert_eq!(ntlm.len(), 40);
        assert_eq!(u32::from_le_bytes(ntlm[8..12].try_into().unwrap()), 1);
        // A server that speaks only SMB 1, or refuses NTLM, says so.
        let old = framed([&b"\xffSMB"[..], &[0u8; 60]].concat());
        let e = smb_exchange(&mut Talk::new(old)).unwrap_err();
        assert!(e.to_string().contains("SMB 1"), "{e}");
        let off = [smb_reply(0, &[0u8; 64]), smb_reply(0xC000_00BB, &[])].concat();
        let e = smb_exchange(&mut Talk::new(off)).unwrap_err();
        assert!(e.to_string().contains("turned off"), "{e}");
    }

    #[test]
    fn rdp_says_whether_it_takes_network_level_authentication() {
        let reply = |kind: u8, picked: u8| {
            let mut r = vec![0x03, 0, 0, 0x13, 0x0e, 0xd0, 0, 0, 0x12, 0x34, 0];
            r.extend([kind, 0, 8, 0, picked, 0, 0, 0]);
            r
        };
        assert!(rdp_picked(&reply(2, 2)).unwrap(), "CredSSP");
        assert!(rdp_picked(&reply(2, 8)).unwrap(), "CredSSP with early auth");
        assert!(!rdp_picked(&reply(2, 1)).unwrap(), "TLS only");
        assert!(rdp_picked(&reply(3, 5)).is_err(), "negotiation failed");
        assert!(rdp_picked(b"SSH-2.0-OpenSSH").is_err());
        // The request: 19 bytes, asking for TLS and CredSSP.
        assert_eq!(RDP_REQUEST.len(), usize::from(RDP_REQUEST[3]));
        // CredSSP's first message is well-formed DER around the NTLM one.
        let ts = ts_request(&ntlm_negotiate());
        assert_eq!(ts[0], 0x30);
        assert_eq!(usize::from(ts[1]), ts.len() - 2);
        assert!(ts.windows(8).any(|w| w == NTLMSSP));
        let sp = spnego_init(&ntlm_negotiate());
        assert_eq!((sp[0], usize::from(sp[1])), (0x60, sp.len() - 2));
        // Lengths past 127 and 255 take the long forms.
        assert_eq!(der(4, &[0; 200])[..3], [4, 0x81, 200]);
        assert_eq!(der(4, &[0; 300])[..4], [4, 0x82, 1, 44]);
    }

    #[test]
    fn hosts_are_named_one_by_one() {
        assert_eq!(
            hosts("pc-01, 10.0.0.7\nsrv.harbor.example;PC-01").unwrap(),
            ["pc-01", "10.0.0.7", "srv.harbor.example"]
        );
        assert!(hosts("").unwrap().is_empty());
        for range in ["10.0.0.0/24", "10.0.0.1-20", "10.0.0.1..9"] {
            let e = hosts(range).unwrap_err();
            assert!(e.contains("range"), "{e}");
        }
        assert!(hosts("pc 01; rm -rf").is_ok(), "words are names");
        assert!(hosts("pc$(x)").is_err());
        // A dash in a name is a name.
        assert_eq!(hosts("front-desk-2").unwrap(), ["front-desk-2"]);
        let many: Vec<String> = (0..=MAX_HOSTS).map(|i| format!("pc{i}")).collect();
        assert!(hosts(&many.join(" ")).unwrap_err().contains("at most"));
        assert_eq!(hosts(&many[1..].join(" ")).unwrap().len(), MAX_HOSTS);
    }

    /// Against whatever answers on this machine's 445 (a Samba started
    /// by hand): `cargo test -- --ignored a_real_server --nocapture`.
    #[test]
    #[ignore]
    fn a_real_server_names_itself() {
        let a = ask("127.0.0.1");
        println!("{a:#?}\n{}", a.line());
        assert_eq!(a.port, Some(445), "{}", a.detail);
        assert!(a.names.computer.is_some());
    }

    #[test]
    fn nothing_listening_is_no_answer() {
        // A port nothing listens on, on this machine: both refuse.
        let a = ask("127.0.0.1");
        if a.port.is_none() && a.verdict == Verdict::NoAnswer {
            assert_eq!(a.verdict.text(), "No answer on 445 or 3389");
            assert_eq!(a.row().len(), COLUMNS.len());
        }
        let a = Answer {
            host: "pc-01".into(),
            names: challenge_names(&member()).unwrap(),
            verdict: Verdict::Ad("harbor.example".into()),
            port: Some(445),
            detail: String::new(),
        };
        assert_eq!(
            a.line(),
            "pc-01: AD: harbor.example · computer FRONTDESK-PC · port 445"
        );
        assert_eq!(a.row().len(), COLUMNS.len());
    }
}
