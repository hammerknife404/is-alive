//! Advanced IP Scanner's XML export, read as a scan source: what its alive
//! rows say about each host, and how many rows were dead or unknown.
//!
//! The file comes from a tech's laptop but it describes a client's network,
//! and the window takes whatever is dropped or picked: it's read with a size
//! cap, no DTD, a node limit and a row limit, and nothing in it can panic.
//! Before the XML parser sees it, one linear pass refuses nesting deeper than
//! `MAX_DEPTH` (the parser recurses, and a job thread's stack is small) and
//! elements with more than `MAX_ATTRS` attributes (its duplicate check is
//! quadratic in them).
//!
//! Privacy: the `user` attribute (who is logged on) is never read, and
//! shares and printers are counted, never named.

use crate::net::mac::{self, Mac};
use std::collections::HashMap;
use std::net::Ipv4Addr;

/// The largest file read: a real export is about 1 KB a row, so this is
/// room for every row there can be.
pub const MAX_FILE: u64 = 64 << 20;

/// Rows read at most: as many addresses as a sweep may cover.
pub const MAX_ROWS: usize = 65_536;

/// Nodes the XML may hold: a row and its few children, with room to spare.
const MAX_NODES: u32 = 4_000_000;

/// The deepest nesting read: an export is rows › row › share, three.
const MAX_DEPTH: usize = 16;

/// The most attributes one element may have: a row has about 30.
const MAX_ATTRS: usize = 128;

/// The longest text kept from any attribute, in characters.
const MAX_TEXT: usize = 160;

/// Ports Advanced IP Scanner checks, from its has_* flags: FTP, HTTP,
/// HTTPS, RDP and HTTP on 8080.
pub const PORTS: &[u16] = &[21, 80, 443, 3389, 8080];

/// One alive row: what Advanced IP Scanner found on the host.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub ip: Option<Ipv4Addr>,
    pub mac: Option<Mac>,
    /// Its `name`: the name the scanner resolved (usually DNS).
    pub name: Option<String>,
    /// Its `manufacturer`: the scanner's own maker table.
    pub maker: Option<String>,
    pub netbios_name: Option<String>,
    pub netbios_group: Option<String>,
    pub os: Option<String>,
    /// Its web page's title.
    pub web: Option<String>,
    /// The TLS or SSL version its HTTPS answered with ("TLSv1.2").
    pub tls: Option<String>,
    /// The rest of `https_version`: the HTTPS server's banner.
    pub https: Option<String>,
    pub ftp: Option<String>,
    pub rdp: Option<String>,
    /// Ports its has_* flags say answered, ascending.
    pub open: Vec<u16>,
    /// The row carries the has_* flags: its services were checked.
    pub checked: bool,
    /// How many `share` and `printer` children it has. Their names are
    /// never read.
    pub shares: usize,
    pub printers: usize,
}

/// What a file held.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scan {
    /// The alive rows, in address order, one per address.
    pub hosts: Vec<Row>,
    pub dead: usize,
    /// Rows whose status was unknown (or anything but alive and dead).
    pub unknown: usize,
    /// Rows with no IPv4 address to read, or a second row for one address.
    pub skipped: usize,
    /// More rows than `MAX_ROWS`: the rest weren't read.
    pub cut: bool,
    /// The newest `date` on a row, as the scanner wrote it.
    pub date: Option<String>,
    /// Every address a row named, alive or not: what the scan covered,
    /// as sweep ranges (`192.0.2.0/24`, `198.51.100.10-40`).
    pub ranges: String,
}

impl Scan {
    /// Rows read, whatever their status.
    pub fn rows(&self) -> usize {
        self.hosts.len() + self.dead + self.unknown + self.skipped
    }
}

/// `data` (a whole file, or rows pasted on their own) read as an Advanced
/// IP Scanner export. An error says why it isn't one.
pub fn parse(data: &[u8]) -> Result<Scan, String> {
    if data.len() as u64 > MAX_FILE {
        return Err(format!(
            "{} MB is larger than any Advanced IP Scanner export",
            data.len() >> 20
        ));
    }
    let text = decode(data);
    shape(&text)?;
    let opt = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_NODES,
    };
    let doc = match roxmltree::Document::parse_with_options(&text, opt) {
        Ok(d) => d,
        // Rows pasted on their own have no root: give them one.
        Err(first) => {
            let wrapped = format!("<rows>{}</rows>", strip_prolog(&text));
            match roxmltree::Document::parse_with_options(&wrapped, opt) {
                Ok(d) => return read(&d),
                Err(_) => return Err(format!("not XML that reads: {first}")),
            }
        }
    };
    read(&doc)
}

/// A row's status as the scanner wrote it.
#[derive(Clone, Copy, PartialEq)]
enum Status {
    Alive,
    Dead,
    Unknown,
}

fn read(doc: &roxmltree::Document) -> Result<Scan, String> {
    let mut scan = Scan::default();
    let mut status: HashMap<Ipv4Addr, Status> = HashMap::new();
    let mut any = false;
    // The newest date that reads (its time in seconds, and as written);
    // else the first row's, as written.
    let mut newest: Option<(i64, String)> = None;
    let mut first_date: Option<String> = None;
    for row in doc.descendants().filter(|n| n.has_tag_name("row")) {
        any = true;
        if scan.rows() >= MAX_ROWS {
            scan.cut = true;
            break;
        }
        let ip = row
            .attribute("ip")
            .and_then(|s| s.trim().parse::<Ipv4Addr>().ok())
            .filter(|ip| !ip.is_unspecified() && !ip.is_broadcast());
        let Some(ip) = ip else {
            scan.skipped += 1;
            continue;
        };
        let now = match row.attribute("status").map(str::trim) {
            Some(s) if s.eq_ignore_ascii_case("alive") => Status::Alive,
            Some(s) if s.eq_ignore_ascii_case("dead") => Status::Dead,
            _ => Status::Unknown,
        };
        match status.get(&ip).copied() {
            // A second row for one address: one of them is skipped, and an
            // alive one wins over a dead or unknown one.
            Some(Status::Alive) => {
                scan.skipped += 1;
                continue;
            }
            Some(was) if now == Status::Alive => {
                scan.skipped += 1;
                match was {
                    Status::Dead => scan.dead -= 1,
                    _ => scan.unknown -= 1,
                }
            }
            Some(_) => {
                scan.skipped += 1;
                continue;
            }
            None => {}
        }
        status.insert(ip, now);
        if let Some(d) = text(row.attribute("date")) {
            match when(&d) {
                Some(t) if newest.as_ref().is_none_or(|(have, _)| t > *have) => {
                    newest = Some((t, d));
                }
                Some(_) => {}
                None if first_date.is_none() => first_date = Some(d),
                None => {}
            }
        }
        match now {
            Status::Alive => scan.hosts.push(host(&row, ip)),
            Status::Dead => scan.dead += 1,
            Status::Unknown => scan.unknown += 1,
        }
    }
    if !any {
        return Err("no <row> elements: not an Advanced IP Scanner XML export".into());
    }
    scan.date = newest.map(|n| n.1).or(first_date);
    scan.hosts.sort_by_key(|h| h.ip);
    scan.ranges = ranges(status.into_keys().map(u32::from).collect());
    Ok(scan)
}

/// One linear pass over the markup before the parser sees it: nesting
/// deeper than `MAX_DEPTH`, or an element with more than `MAX_ATTRS`
/// attributes, is refused. Comments, CDATA, processing instructions and
/// quoted values are stepped over, so a `>` inside one doesn't count.
fn shape(text: &str) -> Result<(), String> {
    let b = text.as_bytes();
    let skip_to = |from: usize, end: &[u8]| -> usize {
        b[from.min(b.len())..]
            .windows(end.len())
            .position(|w| w == end)
            .map_or(b.len(), |p| from + p + end.len())
    };
    let (mut i, mut depth) = (0, 0usize);
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &b[i + 1..];
        if rest.starts_with(b"!--") {
            i = skip_to(i + 4, b"-->");
        } else if rest.starts_with(b"![CDATA[") {
            i = skip_to(i + 9, b"]]>");
        } else if rest.starts_with(b"?") || rest.starts_with(b"!") {
            i = skip_to(i + 1, b">");
        } else if rest.starts_with(b"/") {
            depth = depth.saturating_sub(1);
            i = skip_to(i + 1, b">");
        } else {
            // A start tag: its attributes counted by their `=`, outside
            // quoted values.
            let (mut j, mut quote, mut attrs) = (i + 1, None, 0);
            while j < b.len() {
                match (quote, b[j]) {
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), _) => {}
                    (None, c @ (b'"' | b'\'')) => quote = Some(c),
                    (None, b'=') => attrs += 1,
                    (None, b'>') => break,
                    _ => {}
                }
                j += 1;
            }
            if attrs > MAX_ATTRS {
                return Err(format!(
                    "an element with more than {MAX_ATTRS} attributes: not an Advanced IP \
                     Scanner export"
                ));
            }
            if b.get(j.wrapping_sub(1)) != Some(&b'/') {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(format!(
                        "nested more than {MAX_DEPTH} deep: not an Advanced IP Scanner export"
                    ));
                }
            }
            i = j + 1;
        }
    }
    Ok(())
}

/// A row's `date` ("2026-11-24 13:05:10 UTC-05:00", the offset optional)
/// as seconds since 1970 in UTC, to find the newest; None if it doesn't
/// read that way.
fn when(d: &str) -> Option<i64> {
    let (date, rest) = d.trim().split_once(' ')?;
    let mut ymd = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (ymd.next()??, ymd.next()??, ymd.next()??);
    let (time, zone) = rest.split_once(' ').unwrap_or((rest, ""));
    let mut hms = time.split(':').map(|x| x.parse::<i64>().ok());
    let (h, mi, sec) = (
        hms.next()??,
        hms.next()??,
        hms.next().flatten().unwrap_or(0),
    );
    if !(1..=9999).contains(&y)
        || !(1..=12).contains(&m)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&h)
        || !(0..=59).contains(&mi)
        || !(0..=60).contains(&sec)
    {
        return None;
    }
    let off = match zone.trim().strip_prefix("UTC").or(Some(zone.trim())) {
        Some("") => 0,
        Some(z) => {
            let (sign, hhmm) = match z.as_bytes().first()? {
                b'+' => (1, &z[1..]),
                b'-' => (-1, &z[1..]),
                _ => return None,
            };
            let (oh, om) = hhmm.split_once(':').unwrap_or((hhmm, "0"));
            sign * (oh.parse::<i64>().ok()?.min(14) * 3600 + om.parse::<i64>().ok()?.min(59) * 60)
        }
        None => 0,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec - off)
}

/// One alive row. `user` is never read; shares and printers only counted.
fn host(row: &roxmltree::Node, ip: Ipv4Addr) -> Row {
    let at = |k: &str| text(row.attribute(k));
    let flag = |k: &str| row.attribute(k).map(|v| v.trim() == "1");
    let mut open = Vec::new();
    if flag("has_ftp") == Some(true) {
        open.push(21);
    }
    if flag("has_http") == Some(true) {
        open.push(if flag("is_http8080") == Some(true) {
            8080
        } else {
            80
        });
    }
    if flag("has_https") == Some(true) {
        open.push(443);
    }
    if flag("has_rdp") == Some(true) {
        open.push(3389);
    }
    open.sort_unstable();
    let checked = ["has_http", "has_https", "has_ftp", "has_rdp"]
        .iter()
        .any(|k| flag(k).is_some());
    let (tls, https) = split_https(row.attribute("https_version"));
    let count = |tag: &str| {
        row.children()
            .filter(|c| c.has_tag_name(tag))
            .take(MAX_ROWS)
            .count()
    };
    Row {
        ip: Some(ip),
        mac: row
            .attribute("mac")
            .and_then(|m| mac::parse_mac(m.trim()))
            .filter(|m| m.iter().any(|&b| b != 0) && m[0] & 0x01 == 0),
        name: at("name"),
        maker: at("manufacturer"),
        netbios_name: at("netbiosname"),
        netbios_group: at("netbiosgroup"),
        os: at("os_version"),
        // Its page's title only: the HTTPS banner is kept as https.
        web: at("http_title").or_else(|| at("http_title_full")),
        tls,
        https,
        ftp: at("ftp_version"),
        rdp: at("rdp_version"),
        open,
        checked,
        shares: count("share"),
        printers: count("printer"),
    }
}

/// `https_version` ("Tunnel is TLSv1: Web-Server httpd 3.0"): the TLS or
/// SSL version, and the server banner after it.
fn split_https(v: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(v) = v else { return (None, None) };
    let tls = v
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .find(|w| {
            let w = w.to_ascii_lowercase();
            (w.starts_with("tlsv") || w.starts_with("sslv"))
                && w.len() <= 8
                && w[4..].chars().next().is_some_and(|c| c.is_ascii_digit())
        });
    let banner = match v.split_once(':') {
        Some((_, b)) if tls.is_some() => text(Some(b)),
        _ => text(Some(v)).filter(|_| tls.is_none()),
    };
    (tls.map(str::to_string), banner)
}

/// An attribute as text to keep: trimmed, without controls or the
/// characters that reorder text, cut at `MAX_TEXT`; None when empty.
fn text(v: Option<&str>) -> Option<String> {
    let s: String = v?
        .chars()
        .filter(|&c| {
            !c.is_control()
                && !matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
        })
        .take(MAX_TEXT)
        .collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// The bytes as text: UTF-16 with its byte-order mark, else UTF-8 (bad
/// sequences replaced).
fn decode(data: &[u8]) -> String {
    let utf16 = |be: bool| {
        let units: Vec<u16> = data[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                if be {
                    u16::from_be_bytes([c[0], c[1]])
                } else {
                    u16::from_le_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match data {
        [0xff, 0xfe, ..] => utf16(false),
        [0xfe, 0xff, ..] => utf16(true),
        [0xef, 0xbb, 0xbf, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(data).into_owned(),
    }
}

/// `text` without its `<?xml …?>` prolog, so rows can be wrapped.
fn strip_prolog(text: &str) -> &str {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    match t.strip_prefix("<?xml").and_then(|r| r.split_once("?>")) {
        Some((_, rest)) => rest,
        None => t,
    }
}

/// Most ranges written per /24 before they're summarised per /16.
const MAX_RANGES: usize = 32;

/// The addresses as sweep ranges: one per /24 they fall in, the lowest to
/// the highest there (the whole /24 when that runs .1 to .254). Past
/// `MAX_RANGES` of those, one span per /16, lowest to highest; past that
/// many /16s, one span from the lowest address to the highest.
fn ranges(mut all: Vec<u32>) -> String {
    all.sort_unstable();
    all.dedup();
    let groups = |mask: u32| -> Vec<(u32, u32)> {
        let mut v: Vec<(u32, u32)> = Vec::new();
        for &a in &all {
            match v.last_mut() {
                Some(g) if g.0 & mask == a & mask => g.1 = a,
                _ => v.push((a, a)),
            }
        }
        v
    };
    let span = |(lo, hi): (u32, u32)| {
        if lo == hi {
            Ipv4Addr::from(lo).to_string()
        } else {
            format!("{}-{}", Ipv4Addr::from(lo), Ipv4Addr::from(hi))
        }
    };
    let by24 = groups(0xffff_ff00);
    if by24.len() <= MAX_RANGES {
        return by24
            .into_iter()
            .map(|(lo, hi)| {
                let base = Ipv4Addr::from(lo & 0xffff_ff00);
                let o = base.octets();
                let (l, h) = (lo & 0xff, hi & 0xff);
                if l <= 1 && h >= 254 {
                    format!("{base}/24")
                } else if l == h {
                    format!("{}.{}.{}.{l}", o[0], o[1], o[2])
                } else {
                    format!("{}.{}.{}.{l}-{h}", o[0], o[1], o[2])
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
    }
    let by16 = groups(0xffff_0000);
    if by16.len() <= MAX_RANGES {
        return by16.into_iter().map(span).collect::<Vec<_>>().join(", ");
    }
    match (all.first(), all.last()) {
        (Some(&lo), Some(&hi)) => span((lo, hi)),
        _ => String::new(),
    }
}

/// A fictional export (documentation addresses, made-up names, the
/// 00:11:32 prefix the tests use) for the screenshot tour and the tests. Its
/// logged-on users and share names are there to prove they never get in.
pub const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rows>
<row status="alive" name="gw.example.lan" ip="192.0.2.1" mac="00:11:32:00:00:01" manufacturer="Example Networks" netbiosname="" netbiosgroup="" os_version="" user="" date="2026-11-24 13:05:10 UTC-05:00" has_http="0" is_http8080="0" http_title="" http_title_full="" has_https="1" https_version="Tunnel is TLSv1.2: Web-Server httpd 3.0" https_version_full="" has_ftp="0" ftp_version="" has_rdp="0" rdp_version="" device_type="" sIsDNSScan="0" expanded="0"/>
<row status="alive" name="frontdesk.example.lan" ip="192.0.2.20" mac="00-11-32-00-00-20" manufacturer="" netbiosname="FRONTDESK-PC" netbiosgroup="EXAMPLE" os_version="Windows 11 Pro" user="EXAMPLE\jdoe" date="2026-11-24 13:05:12 UTC-05:00" has_http="0" is_http8080="0" http_title="" has_https="0" https_version="" has_ftp="0" has_rdp="1" rdp_version="RDP 10.0" sIsDNSScan="1" expanded="1"><share name="Payroll$"/><share name="Scans"/><printer name="Front Desk HP"/></row>
<row status="alive" name="" ip="192.0.2.40" mac="00:11:32:00:00:40" manufacturer="" netbiosname="NAS01" netbiosgroup="WORKGROUP" os_version="" user="" date="2026-11-24 13:05:11 UTC-05:00" has_http="1" is_http8080="1" http_title="Example NAS login" has_https="0" has_ftp="1" ftp_version="220 Example FTP ready" has_rdp="0"/>
<row status="dead" name="" ip="192.0.2.2" mac="" user="EXAMPLE\ghost" date="2026-11-24 13:05:10 UTC-05:00"/>
<row status="unknown" name="" ip="192.0.2.254" mac="" user="" date="2026-11-24 13:05:10 UTC-05:00"/>
<row status="unknown" name="" ip="192.0.2.3" mac="" date="2026-11-24 13:05:10 UTC-05:00"/>
</rows>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alive_rows_become_hosts_and_the_rest_are_counted() {
        let s = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(s.hosts.len(), 3);
        assert_eq!((s.dead, s.unknown, s.skipped, s.cut), (1, 2, 0, false));
        assert_eq!(s.date.as_deref(), Some("2026-11-24 13:05:12 UTC-05:00"));
        assert_eq!(s.ranges, "192.0.2.0/24");
        let gw = &s.hosts[0];
        assert_eq!(gw.ip, Some(Ipv4Addr::new(192, 0, 2, 1)));
        assert_eq!(gw.mac, Some([0x00, 0x11, 0x32, 0, 0, 1]));
        assert_eq!(gw.maker.as_deref(), Some("Example Networks"));
        assert_eq!(
            (gw.tls.as_deref(), gw.https.as_deref()),
            (Some("TLSv1.2"), Some("Web-Server httpd 3.0"))
        );
        assert_eq!((gw.open.as_slice(), gw.checked), (&[443][..], true));
        assert_eq!(gw.netbios_name, None, "empty attributes are nothing");
        let pc = &s.hosts[1];
        assert_eq!(
            pc.mac,
            Some([0x00, 0x11, 0x32, 0, 0, 0x20]),
            "dashes read too"
        );
        assert_eq!(pc.netbios_name.as_deref(), Some("FRONTDESK-PC"));
        assert_eq!(pc.netbios_group.as_deref(), Some("EXAMPLE"));
        assert_eq!(pc.os.as_deref(), Some("Windows 11 Pro"));
        assert_eq!(pc.open, [3389]);
        assert_eq!(pc.rdp.as_deref(), Some("RDP 10.0"));
        assert_eq!((pc.shares, pc.printers), (2, 1));
        let nas = &s.hosts[2];
        assert_eq!(nas.open, [21, 8080], "HTTP on 8080");
        assert_eq!(nas.web.as_deref(), Some("Example NAS login"));
        assert_eq!(nas.ftp.as_deref(), Some("220 Example FTP ready"));
        // Who's logged on and the share and printer names never get in.
        let all = format!("{s:?}");
        for never in ["jdoe", "ghost", "Payroll", "Scans", "Front Desk HP"] {
            assert!(!all.contains(never), "{never}");
        }
    }

    /// Rows pasted on their own (as they end up in the site notes), with
    /// or without the prolog, read the same as the whole file.
    #[test]
    fn bare_rows_read_too() {
        let rows: String = SAMPLE
            .lines()
            .filter(|l| l.starts_with("<row "))
            .collect::<Vec<_>>()
            .join("\n");
        let whole = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(parse(rows.as_bytes()).unwrap(), whole);
        let with_prolog = format!("<?xml version=\"1.0\"?>\n{rows}");
        assert_eq!(parse(with_prolog.as_bytes()).unwrap(), whole);
        // One row: a document of its own.
        let one = rows.lines().next().unwrap();
        assert_eq!(parse(one.as_bytes()).unwrap().hosts.len(), 1);
    }

    #[test]
    fn utf16_with_its_mark_reads() {
        let mut data = vec![0xff, 0xfe];
        for u in SAMPLE.encode_utf16() {
            data.extend(u.to_le_bytes());
        }
        assert_eq!(parse(&data).unwrap().hosts.len(), 3);
    }

    #[test]
    fn what_isnt_an_export_says_so() {
        assert!(parse(b"").is_err());
        assert!(parse(b"hello").is_err());
        assert!(parse(b"<html><body>hi</body></html>").is_err());
        // A DTD (where entity bombs live) is refused outright.
        let bomb = "<?xml version=\"1.0\"?><!DOCTYPE r [<!ENTITY a \"aaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;\">]><rows><row status=\"alive\" ip=\"192.0.2.9\" name=\"&b;\"/></rows>";
        assert!(parse(bomb.as_bytes()).is_err());
        let big = vec![b' '; MAX_FILE as usize + 1];
        assert!(parse(&big).unwrap_err().contains("larger"));
    }

    #[test]
    fn odd_rows_are_skipped_or_cleaned() {
        let s = parse(
            "<rows>\
             <row status='alive' ip='not an ip'/>\
             <row status='alive' ip='2001:db8::1'/>\
             <row status='alive' ip=' 192.0.2.7 ' mac='ff:ff:ff:ff:ff:ff' name='a\u{202e}b&#9;c'/>\
             <row status='alive' ip='192.0.2.7' name='again'/>\
             <row status='ALIVE' ip='192.0.2.8' mac='00:00:00:00:00:00' has_http='1'/>\
             <row ip='198.51.100.10'/><row status='dead' ip='198.51.100.40'/>\
             </rows>"
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(s.skipped, 3);
        assert_eq!(s.hosts.len(), 2);
        assert_eq!(s.hosts[0].name.as_deref(), Some("abc"));
        assert_eq!(s.hosts[0].mac, None, "broadcast isn't a host's MAC");
        assert_eq!(s.hosts[1].mac, None, "nor all zeros");
        assert_eq!(s.hosts[1].open, [80]);
        assert_eq!((s.unknown, s.dead), (1, 1));
        assert_eq!(s.ranges, "192.0.2.7-8, 198.51.100.10-40");
        let long = "x".repeat(5000);
        let s =
            parse(format!("<row status='alive' ip='192.0.2.5' os_version='{long}'/>").as_bytes())
                .unwrap();
        assert_eq!(s.hosts[0].os.as_ref().unwrap().len(), MAX_TEXT);
    }

    #[test]
    fn more_rows_than_a_sweep_covers_stop_at_the_cap() {
        let mut x = String::from("<rows>");
        for i in 0..(MAX_ROWS as u32 + 10) {
            let ip = Ipv4Addr::from(0x0a00_0000 + i);
            x += &format!("<row status='dead' ip='{ip}'/>");
        }
        x += "</rows>";
        let s = parse(x.as_bytes()).unwrap();
        assert!(s.cut);
        assert_eq!(s.rows(), MAX_ROWS);
    }

    #[test]
    fn https_versions_split_into_tls_and_banner() {
        let t = |v: &str| split_https(Some(v));
        assert_eq!(
            t("Tunnel is TLSv1: Web-Server httpd 3.0"),
            (Some("TLSv1".into()), Some("Web-Server httpd 3.0".into()))
        );
        assert_eq!(t("SSLv3"), (Some("SSLv3".into()), None));
        assert_eq!(t("nginx"), (None, Some("nginx".into())));
        assert_eq!(t(""), (None, None));
        assert_eq!(t("TLSv"), (None, Some("TLSv".into())));
    }

    /// Deep nesting is refused before the parser (which recurses) sees
    /// it, on a stack as small as a job thread's: 4,000 levels used to
    /// abort the app.
    #[test]
    fn deep_nesting_is_refused_on_a_small_stack() {
        for depth in [17, 4_000, 200_000] {
            let x = "<a>".repeat(depth) + &"</a>".repeat(depth);
            let r = std::thread::Builder::new()
                .stack_size(2 << 20)
                .spawn(move || parse(x.as_bytes()))
                .unwrap()
                .join()
                .unwrap();
            assert!(r.unwrap_err().contains("nested"), "{depth}");
        }
        // Self-closed elements, comments and quoted `>` don't count.
        let ok = "<rows><!-- <a><a> --><row status='alive' ip='192.0.2.9' name='a>b'/>\
                  <![CDATA[<a><a><a>]]></rows>";
        assert_eq!(parse(ok.as_bytes()).unwrap().hosts.len(), 1);
        let ok = "<a>".repeat(MAX_DEPTH - 1)
            + "<row status='alive' ip='192.0.2.9'/>"
            + &"</a>".repeat(MAX_DEPTH - 1);
        assert_eq!(parse(ok.as_bytes()).unwrap().hosts.len(), 1);
    }

    /// An element with thousands of attributes (quadratic for the parser:
    /// 80,000 took seconds) is refused at once.
    #[test]
    fn too_many_attributes_are_refused() {
        let mut row = String::from("<row status='alive' ip='192.0.2.9'");
        for i in 0..80_000 {
            row += &format!(" a{i}='x'");
        }
        row += "/>";
        let t = std::time::Instant::now();
        assert!(parse(row.as_bytes()).unwrap_err().contains("attributes"));
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        // A real row's 30-odd, and an `=` inside a value, are fine.
        assert!(parse(SAMPLE.as_bytes()).is_ok());
        let eq = format!(
            "<row status='alive' ip='192.0.2.9' name='{}'/>",
            "=".repeat(500)
        );
        assert!(parse(eq.as_bytes()).is_ok());
    }

    /// Two rows for one address: one is skipped, and the alive one wins
    /// whichever comes first.
    #[test]
    fn a_second_row_for_an_address_is_skipped_alive_winning() {
        let s = parse(
            b"<rows><row status='dead' ip='192.0.2.5'/><row status='alive' ip='192.0.2.5' name='late'/>\
              <row status='alive' ip='192.0.2.6' name='first'/><row status='unknown' ip='192.0.2.6'/></rows>",
        )
        .unwrap();
        assert_eq!(s.hosts.len(), 2);
        assert_eq!(s.hosts[0].name.as_deref(), Some("late"));
        assert_eq!((s.dead, s.unknown, s.skipped), (0, 0, 2));
    }

    /// The scan's date is the newest by time, not by text: a later hour in
    /// another offset, a day past the 9th; one that won't read only when
    /// none do.
    #[test]
    fn the_scan_date_is_the_newest_by_time() {
        let rows = |dates: &[&str]| {
            let mut x = String::from("<rows>");
            for (i, d) in dates.iter().enumerate() {
                x += &format!("<row status='alive' ip='192.0.2.{}' date='{d}'/>", i + 1);
            }
            parse((x + "</rows>").as_bytes()).unwrap().date
        };
        assert_eq!(
            rows(&[
                "2026-11-24 13:05:10 UTC-05:00",
                "2026-11-24 14:00:00 UTC+02:00"
            ])
            .as_deref(),
            Some("2026-11-24 13:05:10 UTC-05:00"),
            "14:00 at +2 is 12:00 UTC, before 18:05"
        );
        assert_eq!(
            rows(&["2026-11-9 23:00:00", "2026-11-10 01:00:00"]).as_deref(),
            Some("2026-11-10 01:00:00")
        );
        assert_eq!(rows(&["yesterday", "later"]).as_deref(), Some("yesterday"));
        assert_eq!(
            rows(&["yesterday", "2026-01-01 00:00:00"]).as_deref(),
            Some("2026-01-01 00:00:00")
        );
        assert_eq!(when("2026-11-24 13:05:10 UTC"), when("2026-11-24 13:05:10"));
        assert_eq!(when("1970-01-01 00:00:00 UTC+01:00"), Some(-3600));
        for bad in [
            "",
            " ",
            "2026-13-01 00:00:00",
            "2026-11-24",
            "2026-11-24 25:00:00",
            "2026-11-24 13:05:10 EST",
        ] {
            assert_eq!(when(bad), None, "{bad}");
        }
    }

    /// Past 32 /24s the ranges are summarised per /16, and past 32 of
    /// those as one span: never a list as long as the file.
    #[test]
    fn ranges_past_32_networks_are_summarised() {
        let a = |x: [u8; 4]| u32::from(Ipv4Addr::from(x));
        assert_eq!(
            ranges(vec![a([10, 0, 0, 5]), a([10, 0, 1, 9])]),
            "10.0.0.5, 10.0.1.9"
        );
        let many: Vec<u32> = (0..40u8)
            .map(|n| a([10, 1, n, 7]))
            .chain([a([10, 2, 0, 1])])
            .collect();
        assert_eq!(ranges(many), "10.1.0.7-10.1.39.7, 10.2.0.1");
        let huge: Vec<u32> = (0..40u8).map(|n| a([10, n, 0, 1])).collect();
        assert_eq!(ranges(huge), "10.0.0.1-10.39.0.1");
        assert_eq!(ranges(vec![]), "");
    }

    /// Hostile files: the sample cut, corrupted and noise, and text with
    /// characters that trip byte slicing. None may panic.
    #[test]
    fn hostile_files_never_panic() {
        crate::hostile::each(SAMPLE.as_bytes(), |d| {
            let _ = parse(d);
        });
        crate::hostile::each_text(SAMPLE, |t| {
            let _ = parse(t.as_bytes());
        });
        for v in [
            "TLSv1:",
            ":",
            "TLSv\u{e9}",
            "sslv1.2.3.4.5",
            "\u{1f511}TLSv1",
        ] {
            let _ = split_https(Some(v));
        }
        let _ = decode(&[0xff, 0xfe, 0x41]);
        let _ = decode(&[0xfe, 0xff]);
        let _ = ranges(vec![0, u32::MAX, 255, 256]);
        for d in [
            "99999999999999999-1-1 0:0:0",
            "2026-1-1 -5:0:0",
            "2026-\u{e9}-1 0:0:0",
            "-9999999999-1-1 0:0:0",
            "2026-11-24 13:05:10 UTC+",
            "2026-11-24 13:05:10 UTC-99:99",
            "x y z",
        ] {
            let _ = when(d);
        }
        for t in [
            "<",
            "<a",
            "<a '",
            "</",
            "<!--",
            "<![CDATA[",
            "<?",
            "<a/",
            "<<<>>>",
        ] {
            let _ = shape(t);
        }
    }
}
