//! Advanced IP Scanner's XML export, read as a scan source: what its alive
//! rows say about each host, and how many rows were dead or unknown.
//!
//! The file comes from a tech's laptop but it describes a client's network,
//! and the window takes whatever is dropped or picked: it's read with a size
//! cap, no DTD, a node limit and a row limit, and nothing in it can panic.
//!
//! Privacy: the `user` attribute (who is logged on) is never read, and
//! shares and printers are counted, never named.

use crate::net::mac::{self, Mac};
use std::collections::HashSet;
use std::net::Ipv4Addr;

/// The largest file read: a real export is about 1 KB a row, so this is
/// room for every row there can be.
pub const MAX_FILE: u64 = 64 << 20;

/// Rows read at most: as many addresses as a sweep may cover.
pub const MAX_ROWS: usize = 65_536;

/// Nodes the XML may hold: a row and its few children, with room to spare.
const MAX_NODES: u32 = 4_000_000;

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

fn read(doc: &roxmltree::Document) -> Result<Scan, String> {
    let mut scan = Scan::default();
    let mut seen: HashSet<Ipv4Addr> = HashSet::new();
    let mut all: Vec<u32> = Vec::new();
    let mut any = false;
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
        let Some(ip) = ip.filter(|ip| seen.insert(*ip)) else {
            scan.skipped += 1;
            continue;
        };
        all.push(ip.into());
        if let Some(d) = text(row.attribute("date"))
            && scan.date.as_ref().is_none_or(|have| d > *have)
        {
            scan.date = Some(d);
        }
        match row.attribute("status").map(str::trim) {
            Some(s) if s.eq_ignore_ascii_case("alive") => scan.hosts.push(host(&row, ip)),
            Some(s) if s.eq_ignore_ascii_case("dead") => scan.dead += 1,
            _ => scan.unknown += 1,
        }
    }
    if !any {
        return Err("no <row> elements: not an Advanced IP Scanner XML export".into());
    }
    scan.hosts.sort_by_key(|h| h.ip);
    scan.ranges = ranges(all);
    Ok(scan)
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

/// The addresses as sweep ranges, one per /24 they fall in: the lowest to
/// the highest there, or the whole /24 when that runs .1 to .254.
fn ranges(mut all: Vec<u32>) -> String {
    all.sort_unstable();
    all.dedup();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < all.len() {
        let net = all[i] & 0xffff_ff00;
        let mut j = i;
        while j + 1 < all.len() && all[j + 1] & 0xffff_ff00 == net {
            j += 1;
        }
        let (lo, hi) = (all[i] & 0xff, all[j] & 0xff);
        let base = Ipv4Addr::from(net);
        let o = base.octets();
        out.push(if lo <= 1 && hi >= 254 {
            format!("{base}/24")
        } else if lo == hi {
            format!("{}.{}.{}.{lo}", o[0], o[1], o[2])
        } else {
            format!("{}.{}.{}.{lo}-{hi}", o[0], o[1], o[2])
        });
        i = j + 1;
    }
    out.join(", ")
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
    }
}
