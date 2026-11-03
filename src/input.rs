//! What each text box accepts: how long it may get, which characters are
//! dropped as they're typed or pasted, and whether a value can be used,
//! with a plain message when it can't.
//!
//! Characters dropped everywhere: control characters (a single-line box
//! turns tabs and line breaks into spaces), the invisible controls that
//! flip the direction text is drawn in, which can make `moc.elpmaxe` read
//! as `example.com`, and other invisible characters, such as the tag
//! characters that can hide whole sentences. Names drop zero-width
//! characters and direction marks too; free text keeps the ones real
//! writing uses (joiners for emoji and Indic and Arabic scripts, word
//! breaks for Thai, the left-to-right and right-to-left marks).

use std::net::{IpAddr, Ipv6Addr};

/// The kind of thing a box holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// A host name or IP to probe (Ping, Port Check, Traceroute).
    Host,
    /// A name or IP to look up (DNS Lookup).
    Name,
    /// A domain to check (Advanced).
    Domain,
    /// A DNS server's IP, optionally with a port.
    Resolver,
    /// DKIM selectors, separated by commas or spaces.
    Selectors,
    /// Ports: lists and ranges.
    Ports,
    /// An IPv4 range to sweep.
    Range,
    /// A subnet, address and mask, or range (Subnet Calc).
    Subnet,
    /// One IP address.
    Address,
    /// A web link.
    Url,
    /// Free text on one line: a search, a filter, a site name.
    Text,
    /// The notes editor.
    Notes,
    /// Known problems in the site bundle.
    Problems,
    /// A note on a map card: a few lines.
    CardNote,
}

/// Longest host or domain name DNS can carry.
const NAME_MAX: usize = 253;
/// Longest single part of a name.
const LABEL_MAX: usize = 63;
/// Most DKIM selectors probed beyond the common ones.
pub const SELECTORS_MAX: usize = 20;

impl Field {
    /// Most characters the box takes.
    pub const fn max(self) -> usize {
        match self {
            Field::Host | Field::Name | Field::Domain | Field::Selectors | Field::Text => 255,
            Field::Resolver | Field::Range | Field::Address => 64,
            Field::Subnet => 100,
            Field::Ports => 1024,
            Field::Url => 2048,
            Field::Notes => 1_000_000,
            Field::Problems => 20_000,
            Field::CardNote => 2_000,
        }
    }

    /// What the text box itself is told it takes: room for the hidden
    /// characters a paste brings, so they don't use up `max` before `fit`
    /// takes them out and cuts what's left to `max`.
    pub const fn room(self) -> usize {
        self.max() * 4
    }

    fn multiline(self) -> bool {
        matches!(self, Field::Notes | Field::Problems | Field::CardNote)
    }

    /// Free text keeps the zero-width characters and direction marks real
    /// writing uses; a name or number never needs one.
    fn free(self) -> bool {
        matches!(
            self,
            Field::Text | Field::Notes | Field::Problems | Field::CardNote
        )
    }

    /// What `c` becomes in this box: itself, a space or line break, or nothing.
    fn map(self, c: char) -> Option<char> {
        match c {
            '\n' | '\t' if self.multiline() => Some(c),
            '\u{2028}' | '\u{2029}' if self.multiline() => Some('\n'),
            '\n' | '\t' | '\r' | '\u{2028}' | '\u{2029}' if !self.multiline() => Some(' '),
            // Zero-width space and joiners, left-to-right and right-to-left marks.
            '\u{200B}'..='\u{200F}' if self.free() => Some(c),
            c if hidden(c) => None,
            c => Some(c),
        }
    }

    /// What each character of `s` becomes in this box. Free text keeps the
    /// tag characters of a flag like Scotland's (🏴, a short lower-case
    /// tag, the cancel tag); tag characters anywhere else go.
    fn each(self, s: &str) -> impl Iterator<Item = Option<char>> + '_ {
        let mut flag = 0;
        let mut prev = '\0';
        s.char_indices().map(move |(i, c)| {
            if flag == 0 && prev == '\u{1F3F4}' && self.free() {
                flag = flag_tags(&s[i..]);
            }
            prev = c;
            if flag > 0 {
                flag -= 1;
                Some(c)
            } else {
                self.map(c)
            }
        })
    }

    /// `s` with the characters this box drops taken out, cut to `max`.
    pub fn clean(self, s: &str) -> String {
        self.each(s).flatten().take(self.max()).collect()
    }

    /// How many characters of `s`, cleaned and not yet cut, come from the
    /// first `n` of `s`: where a cursor at `n` lands once `s` is cleaned.
    pub fn kept_before(self, s: &str, n: usize) -> usize {
        self.each(s).take(n).flatten().count()
    }

    /// How many characters of `s` this box drops that someone would want
    /// told about: a line break's carriage return (Windows) goes quietly.
    pub fn dropped(self, s: &str) -> usize {
        s.chars()
            .zip(self.each(s))
            .filter(|&(c, m)| c != '\r' && m.is_none())
            .count()
    }

    /// `s` as the box keeps it after an edit that left the cursor at `at`
    /// of the cleaned text: cleaned, and when that's over `max`, cut just
    /// before the cursor (the end of what was typed or pasted, as the box
    /// itself would), or at the end when there isn't that much before it.
    /// Also gives the part of the cleaned text that was cut.
    pub fn fit(self, s: &str, at: usize) -> (String, std::ops::Range<usize>) {
        let mut cs: Vec<char> = self.each(s).flatten().collect();
        let n = cs.len();
        let over = n.saturating_sub(self.max());
        let cut = if over == 0 {
            n..n
        } else if (over..=n).contains(&at) {
            at - over..at
        } else {
            self.max()..n
        };
        cs.drain(cut.clone());
        (cs.into_iter().collect(), cut)
    }
}

/// How many characters at the start of `s` are a flag's tag: two to seven
/// lower-case letters or digits as tag characters, then the cancel tag.
/// 0 when they aren't one.
fn flag_tags(s: &str) -> usize {
    let n = s
        .chars()
        .take_while(|c| matches!(c, '\u{E0030}'..='\u{E0039}' | '\u{E0061}'..='\u{E007A}'))
        .count();
    let cancel = s.chars().nth(n) == Some('\u{E007F}');
    if (2..=7).contains(&n) && cancel {
        n + 1
    } else {
        0
    }
}

/// Characters no box keeps (free text keeps a few, see `Field::map`):
/// controls, direction marks, embeddings, overrides and isolates,
/// zero-width spaces and joiners, invisible fillers and format characters,
/// line and paragraph separators, tag characters, and the byte-order mark.
pub fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
        )
}

/// One line of text read from a file or the network, safe to show: hidden
/// characters dropped, at most `max` characters.
pub fn line(s: &str, max: usize) -> String {
    Field::Text
        .each(s)
        .flatten()
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A text file of ours read back (settings, notes): at most `max` bytes,
/// anything that isn't UTF-8 replaced. None when it can't be read.
pub fn read_capped(path: &std::path::Path, max: u64) -> Option<String> {
    use std::io::Read;
    let mut data = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(max)
        .read_to_end(&mut data)
        .ok()?;
    Some(String::from_utf8_lossy(&data).into_owned())
}

/// A host to probe, as typed: what to resolve, and a port if one came with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// An IP literal (no brackets) or an ASCII host name.
    pub host: String,
    pub port: Option<u16>,
}

/// Read a host the way people paste one: `example.com`, `10.0.0.1`,
/// `fe80::1%eth0`, `[::1]:443`, `host:8443` or a whole link
/// (`https://user@host:8443/path`). Unicode names are turned into the
/// `xn--` form DNS uses; a trailing dot stays.
pub fn host(s: &str) -> Result<Target, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("enter a host".into());
    }
    let s = authority(s);
    if let Some((ip, _)) = s.split_once('/')
        && ip.parse::<IpAddr>().is_ok()
    {
        return Err(format!(
            "'{}' is a subnet: Subnet Sweep pings a range",
            short(s)
        ));
    }
    let (h, port) = split_port(s)?;
    if h.is_empty() {
        return Err("enter a host".into());
    }
    if let Some(ip) = ip_literal(h) {
        return Ok(Target { host: ip, port });
    }
    let host = name(h, false).map_err(|e| format!("'{}' isn't a host name: {e}", short(h)))?;
    Ok(Target { host, port })
}

/// A name or IP to look up in DNS. Underscores (`_dmarc.example.com`) and a
/// leading `*` label are allowed here, as DNS records use them.
pub fn dns_name(s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("enter a name or IP".into());
    }
    let s = if s.contains("://") {
        split_port(authority(s))?.0
    } else {
        s
    };
    let bare = s.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<IpAddr>().is_ok() {
        return Ok(bare.to_string());
    }
    name(s, true).map_err(|e| format!("'{}' isn't a name DNS can look up: {e}", short(s)))
}

/// Extra DKIM selectors from a list like `s1024, mykey selector2`. A
/// trailing `._domainkey` is dropped, as people paste the record's name.
pub fn selectors(s: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for part in s.split([',', ' ', ';']).map(str::trim) {
        let sel = part.trim_end_matches('.');
        let sel = sel.strip_suffix("._domainkey").unwrap_or(sel);
        if sel.is_empty() {
            continue;
        }
        let ok = sel.len() <= 200
            && sel.split('.').all(|l| {
                !l.is_empty()
                    && l.len() <= LABEL_MAX
                    && l.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            });
        if !ok {
            return Err(format!(
                "'{}' isn't a DKIM selector: letters, digits, '-', '_' and '.' only",
                short(sel)
            ));
        }
        if !out.iter().any(|o| o.eq_ignore_ascii_case(sel)) {
            out.push(sel.to_string());
        }
    }
    if out.len() > SELECTORS_MAX {
        return Err(format!("at most {SELECTORS_MAX} extra DKIM selectors"));
    }
    Ok(out)
}

/// The host and port of a link, or `s` itself when it isn't one.
fn authority(s: &str) -> &str {
    let Some((_, rest)) = s.split_once("://") else {
        return s;
    };
    let a = rest.split(['/', '?', '#']).next().unwrap_or("");
    a.rsplit_once('@').map_or(a, |(_, h)| h)
}

/// `[v6]:port`, `name:port` or `v4:port` into host and port; a bare IPv6
/// address (many colons) has no port.
fn split_port(s: &str) -> Result<(&str, Option<u16>), String> {
    let port = |p: &str| {
        p.parse::<u16>()
            .ok()
            .filter(|&p| p != 0)
            .map(Some)
            .ok_or_else(|| format!("'{}' isn't a port (1 to 65535)", short(p)))
    };
    if let Some(rest) = s.strip_prefix('[') {
        let (h, after) = rest
            .split_once(']')
            .ok_or("an IPv6 address in brackets needs the closing ]")?;
        return match after {
            "" => Ok((h, None)),
            a => match a.strip_prefix(':') {
                Some(p) => Ok((h, port(p)?)),
                None => Err(format!("unexpected '{}' after ]", short(a))),
            },
        };
    }
    match s.split_once(':') {
        Some((h, p)) if !p.contains(':') => Ok((h, port(p)?)),
        _ => Ok((s, None)),
    }
}

/// An IP literal, IPv6 with a zone (`fe80::1%eth0`) too, as text to resolve.
fn ip_literal(s: &str) -> Option<String> {
    if s.parse::<IpAddr>().is_ok() {
        return Some(s.to_string());
    }
    let (ip, zone) = s.split_once('%')?;
    let zone_ok = !zone.is_empty()
        && zone.len() <= 32
        && zone
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    (ip.parse::<Ipv6Addr>().is_ok() && zone_ok).then(|| s.to_string())
}

/// A host (`dns` false) or DNS name, checked part by part and returned
/// lower-case in ASCII. A host keeps a trailing dot (an absolute name, no
/// search domains added); a DNS name drops it. A host's ASCII parts are
/// letters, digits, '-' and '_'; a DNS name's may hold any printable ASCII
/// but '.', as DNS itself allows (`0/26.2.0.192.in-addr.arpa`, `Office
/// Printer._ipp._tcp.local`). An error says what's wrong ("it has a space").
fn name(s: &str, dns: bool) -> Result<String, String> {
    // Fullwidth letters and ideographic full stops, as an IME types them,
    // read as ASCII the way IDNA maps them; lower-casing the whole name
    // turns a Kelvin sign into a plain k.
    let s: String = s
        .chars()
        .map(|c| match c {
            '\u{3002}' | '\u{FF0E}' | '\u{FF61}' => '.',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            c => c,
        })
        .collect::<String>()
        .to_lowercase();
    let (s, absolute) = match s.strip_suffix('.') {
        Some(s) => (s, true),
        None => (s.as_str(), false),
    };
    if s.is_empty() {
        return Err("it's empty".into());
    }
    let ok = |c: char| {
        if c.is_ascii() {
            c.is_ascii_alphanumeric()
                || matches!(c, '.' | '-' | '_' | '*')
                || (dns && (' '..='~').contains(&c))
        } else {
            !c.is_whitespace() && !hidden(c)
        }
    };
    if let Some(c) = s.chars().find(|&c| !ok(c)) {
        return Err(match c {
            c if c.is_whitespace() => "it has a space".into(),
            c => format!("it has '{c}' in it"),
        });
    }
    let mut out = Vec::new();
    for (i, label) in s.split('.').enumerate() {
        if label.is_empty() {
            return Err("it has an empty part (..)".into());
        }
        if label.contains('*') && !(dns && i == 0 && label == "*") {
            return Err("'*' can only be a whole first part (*.example.com)".into());
        }
        if !dns && (label.starts_with('-') || label.ends_with('-')) {
            return Err("a part starts or ends with '-'".into());
        }
        let ascii = if label.is_ascii() {
            label.to_string()
        } else {
            // An accent pasted as its own character after the letter (as
            // macOS can) would be written in DNS as a different name.
            let mut prev = ' ';
            for c in label.chars() {
                if prev.is_ascii_alphabetic() && ('\u{0300}'..='\u{036F}').contains(&c) {
                    return Err(format!(
                        "the accent on '{prev}' is a separate character: retype the letter with its accent"
                    ));
                }
                prev = c;
            }
            punycode(label)
                .map(|p| format!("xn--{p}"))
                .ok_or("it can't be written in DNS")?
        };
        if ascii.len() > LABEL_MAX {
            return Err(format!("a part is longer than {LABEL_MAX} characters"));
        }
        out.push(ascii);
    }
    let mut joined = out.join(".");
    if joined.len() > NAME_MAX {
        return Err(format!("it's longer than {NAME_MAX} characters"));
    }
    if absolute && !dns {
        joined.push('.');
    }
    Ok(joined)
}

/// The first 40 characters, for quoting a value in a message.
fn short(s: &str) -> String {
    match s.char_indices().nth(40) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// RFC 3492 Punycode of one label (without the `xn--`). None only when
/// the label is too long to encode, far past what DNS allows anyway.
fn punycode(input: &str) -> Option<String> {
    const BASE: u32 = 36;
    const TMIN: u32 = 1;
    const TMAX: u32 = 26;
    let digit = |d: u32| -> char {
        if d < 26 {
            (b'a' + d as u8) as char
        } else {
            (b'0' + (d - 26) as u8) as char
        }
    };
    let adapt = |delta: u32, points: u32, first: bool| {
        let mut delta = if first { delta / 700 } else { delta / 2 };
        delta += delta / points;
        let mut k = 0;
        while delta > ((BASE - TMIN) * TMAX) / 2 {
            delta /= BASE - TMIN;
            k += BASE;
        }
        k + (BASE - TMIN + 1) * delta / (delta + 38)
    };
    let cps: Vec<u32> = input.chars().map(u32::from).collect();
    let mut out: String = input.chars().filter(char::is_ascii).collect();
    let basic = out.len() as u32;
    let mut h = basic;
    if basic > 0 {
        out.push('-');
    }
    let (mut n, mut delta, mut bias) = (128u32, 0u32, 72u32);
    while (h as usize) < cps.len() {
        let m = cps.iter().copied().filter(|&c| c >= n).min()?;
        delta = delta.checked_add((m - n).checked_mul(h + 1)?)?;
        n = m;
        for &c in &cps {
            if c < n {
                delta = delta.checked_add(1)?;
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = if k <= bias {
                        TMIN
                    } else if k >= bias + TMAX {
                        TMAX
                    } else {
                        k - bias
                    };
                    if q < t {
                        break;
                    }
                    out.push(digit(t + (q - t) % (BASE - t)));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                out.push(digit(q));
                bias = adapt(delta, h + 1, h == basic);
                delta = 0;
                h += 1;
            }
        }
        delta = delta.checked_add(1)?;
        n += 1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(host: &str, port: Option<u16>) -> Target {
        Target {
            host: host.into(),
            port,
        }
    }

    #[test]
    fn hosts_people_type_are_read() {
        assert_eq!(host(" 10.0.0.1 "), Ok(t("10.0.0.1", None)));
        assert_eq!(host("Example.COM"), Ok(t("example.com", None)));
        // A trailing dot keeps the name absolute: no search domains.
        assert_eq!(host("nas."), Ok(t("nas.", None)));
        assert_eq!(host("Example.COM."), Ok(t("example.com.", None)));
        assert_eq!(host("fe80::1%eth0"), Ok(t("fe80::1%eth0", None)));
        assert_eq!(host("2001:db8::1"), Ok(t("2001:db8::1", None)));
        assert_eq!(host("[::1]"), Ok(t("::1", None)));
        assert_eq!(host("[::1]:443"), Ok(t("::1", Some(443))));
        assert_eq!(host("[fe80::1%en0]:22"), Ok(t("fe80::1%en0", Some(22))));
        assert_eq!(host("nas:5001"), Ok(t("nas", Some(5001))));
        assert_eq!(host("10.0.0.1:8443"), Ok(t("10.0.0.1", Some(8443))));
        assert_eq!(
            host("https://admin@fw.example.net:4433/login?x=1"),
            Ok(t("fw.example.net", Some(4433)))
        );
        assert_eq!(host("http://[2001:db8::5]/"), Ok(t("2001:db8::5", None)));
        assert_eq!(host("printer_2.lan"), Ok(t("printer_2.lan", None)));
        assert_eq!(host("xn--bcher-kva.de"), Ok(t("xn--bcher-kva.de", None)));
        assert_eq!(host("Bücher.de"), Ok(t("xn--bcher-kva.de", None)));
    }

    #[test]
    fn unicode_names_are_mapped_like_idna() {
        // Combining marks (the virama) belong to many scripts' names.
        assert_eq!(
            host(
                "\u{0939}\u{093F}\u{0928}\u{094D}\u{0926}\u{0940}.\u{092D}\u{093E}\u{0930}\u{0924}"
            ),
            Ok(t("xn--j2bd4cyah0f.xn--h2brj9c", None))
        );
        // A Kelvin sign lower-cases to a plain k: no xn-- for ASCII.
        assert_eq!(host("\u{212A}elvin.com"), Ok(t("kelvin.com", None)));
        // Fullwidth letters and an ideographic full stop, as an IME types them.
        assert_eq!(
            host("\u{FF25}\u{FF38}\u{FF21}\u{FF2D}\u{FF30}\u{FF2C}\u{FF25}\u{3002}com"),
            Ok(t("example.com", None))
        );
        // A decomposed ü (as macOS can paste it) would be a different name.
        assert!(host("bu\u{308}cher.de").unwrap_err().contains("accent"));
    }

    #[test]
    fn junk_hosts_say_why() {
        assert_eq!(host("   ").unwrap_err(), "enter a host");
        assert_eq!(
            host("exa mple.com").unwrap_err(),
            "'exa mple.com' isn't a host name: it has a space"
        );
        assert!(host("a..b").unwrap_err().contains("empty part"));
        assert!(host("10.0.0.0/24").unwrap_err().contains("Subnet Sweep"));
        assert!(host("-oProxyCommand=x").is_err());
        assert!(host("host;rm -rf").is_err());
        assert!(host("evil.com/../x").is_err());
        assert!(host("host:0").unwrap_err().contains("port"));
        assert!(host("host:99999").unwrap_err().contains("port"));
        assert!(host("[::1").unwrap_err().contains("]"));
        assert!(host("*.example.com").is_err());
        assert!(host(&format!("{}.com", "a".repeat(64))).is_err());
        let long = vec!["abcdefghi"; 30].join(".");
        assert!(host(&long).unwrap_err().contains("253"));
        assert!(host("fe80::1%eth 0").is_err());
    }

    #[test]
    fn dns_names_allow_underscores_and_wildcards() {
        assert_eq!(
            dns_name("_dmarc.Example.com."),
            Ok("_dmarc.example.com".into())
        );
        assert_eq!(
            dns_name("_sip._tcp.example.com"),
            Ok("_sip._tcp.example.com".into())
        );
        assert_eq!(dns_name("*.example.com"), Ok("*.example.com".into()));
        assert_eq!(dns_name("8.8.8.8"), Ok("8.8.8.8".into()));
        assert_eq!(dns_name("[2001:db8::1]"), Ok("2001:db8::1".into()));
        assert_eq!(
            dns_name("https://www.example.com/a"),
            Ok("www.example.com".into())
        );
        assert_eq!(
            dns_name("\u{4F8B}\u{3048}.\u{30C6}\u{30B9}\u{30C8}"),
            Ok("xn--r8jz45g.xn--zckzah".into())
        );
        assert!(dns_name("a.*.example.com").is_err());
        assert!(dns_name("").is_err());
        assert!(dns_name("a..b").is_err());
        assert_eq!(dns_name("nas."), Ok("nas".into()));
    }

    #[test]
    fn dns_names_take_what_dns_carries() {
        // RFC 2317 classless reverse zones and DNS-SD instance names.
        assert_eq!(
            dns_name("0/26.2.0.192.in-addr.arpa"),
            Ok("0/26.2.0.192.in-addr.arpa".into())
        );
        assert_eq!(
            dns_name("Office Printer._ipp._tcp.example.com"),
            Ok("office printer._ipp._tcp.example.com".into())
        );
        assert_eq!(dns_name("-x-.example.com"), Ok("-x-.example.com".into()));
        // A host is stricter.
        assert!(host("0/26.example.com").is_err());
        assert!(host("<script>").is_err());
    }

    #[test]
    fn punycode_matches_rfc_3492() {
        assert_eq!(punycode("bücher").as_deref(), Some("bcher-kva"));
        assert_eq!(punycode("münchen").as_deref(), Some("mnchen-3ya"));
        assert_eq!(punycode("παράδειγμα").as_deref(), Some("hxajbheg2az3al"));
        assert_eq!(punycode("\u{4F8B}\u{3048}").as_deref(), Some("r8jz45g"));
    }

    #[test]
    fn selectors_are_dns_labels() {
        assert_eq!(
            selectors("s1024, mykey;selector2._domainkey  google"),
            Ok(vec![
                "s1024".into(),
                "mykey".into(),
                "selector2".into(),
                "google".into()
            ])
        );
        assert_eq!(selectors(""), Ok(vec![]));
        assert_eq!(selectors("a, A"), Ok(vec!["a".into()]));
        assert_eq!(selectors("k1.2024"), Ok(vec!["k1.2024".into()]));
        assert!(selectors("bad/one").is_err());
        assert!(selectors("ä").is_err());
        assert!(selectors("k1.2024/x").unwrap_err().contains("'.'"));
        let many: Vec<String> = (0..21).map(|i| format!("s{i}")).collect();
        assert!(selectors(&many.join(",")).unwrap_err().contains("20"));
    }

    #[test]
    fn hidden_characters_are_dropped() {
        // A right-to-left override that would draw this as "example.com".
        assert_eq!(
            Field::Host.clean("\u{202E}moc.elpmaxe\u{202C}"),
            "moc.elpmaxe"
        );
        assert_eq!(Field::Host.clean("a\u{200B}b\u{FEFF}c\0"), "abc");
        assert_eq!(Field::Host.clean("host\r\n"), "host  ");
        assert_eq!(Field::Ports.clean("22,\t80"), "22, 80");
        assert_eq!(Field::Text.clean("x\u{2066}y\u{2069}"), "xy");
        // Emoji and scripts keep their joiners in free text.
        assert_eq!(Field::Text.clean("👩\u{200D}💻"), "👩\u{200D}💻");
        assert_eq!(Field::Host.clean("a\u{200D}b"), "ab");
        assert_eq!(Field::Notes.clean("a\r\n\tb\x1b[31m\u{202E}"), "a\n\tb[31m");
        // The marks and word breaks real writing uses stay in free text.
        let rtl = "c\u{200F}\u{05E2}\u{05D1}\u{200E}x";
        assert_eq!(Field::Notes.clean(rtl), rtl);
        assert_eq!(Field::Text.clean("a\u{200B}b"), "a\u{200B}b");
        assert_eq!(Field::Host.clean("a\u{200F}b"), "ab");
        // Other invisible characters, and tags that can hide a sentence.
        for c in [
            '\u{061C}',
            '\u{00AD}',
            '\u{180E}',
            '\u{034F}',
            '\u{3164}',
            '\u{FFF9}',
            '\u{E0001}',
            '\u{E0041}',
            '\u{E007F}',
        ] {
            assert_eq!(Field::Text.clean(&format!("a{c}b")), "ab", "{c:?}");
            assert_eq!(Field::Notes.clean(&format!("a{c}b")), "ab", "{c:?}");
        }
        // Scotland's flag keeps its tags; a black flag doesn't excuse others.
        let scotland = "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}";
        assert_eq!(Field::Notes.clean(scotland), scotland);
        let smuggled = "\u{1F3F4}\u{E0049}\u{E0020}\u{E0061}\u{E007F}";
        assert_eq!(Field::Notes.clean(smuggled), "\u{1F3F4}");
        assert_eq!(Field::Host.clean(scotland), "\u{1F3F4}");
        // Line and paragraph separators.
        assert_eq!(Field::Text.clean("a\u{2028}b"), "a b");
        assert_eq!(Field::Notes.clean("a\u{2029}b"), "a\nb");
    }

    /// A card's note: a few lines, hidden characters out, 2,000 at most.
    #[test]
    fn card_notes_keep_lines_and_drop_hidden_characters() {
        let k = Field::CardNote;
        assert_eq!(
            k.clean("a\r\n\tb\u{202E}c\u{200B}d\u{7}"),
            "a\n\tbc\u{200B}d"
        );
        assert_eq!(k.clean("x\u{2028}y"), "x\ny");
        assert_eq!(k.clean("👩\u{200D}💻"), "👩\u{200D}💻");
        assert_eq!(k.clean(&"n".repeat(5000)).chars().count(), 2_000);
        assert_eq!(k.room(), 8_000);
    }

    #[test]
    fn clean_cuts_at_the_limit() {
        let s = "x".repeat(5000);
        assert_eq!(Field::Host.clean(&s).chars().count(), 255);
        assert_eq!(Field::Url.clean(&s).chars().count(), 2048);
        assert_eq!(Field::Ports.clean(&s).chars().count(), 1024);
        assert_eq!(Field::Host.kept_before(&s, 4000), 4000);
    }

    #[test]
    fn fit_cuts_what_was_just_added() {
        // Hidden characters don't use up the limit: 20 go, all 255 fit.
        let paste = format!("{}{}", "\u{200B}".repeat(20), "y".repeat(255));
        let (s, cut) = Field::Host.fit(&paste, 255);
        assert_eq!((s.len(), cut), (255, 255..255));
        // Over the limit: what's before the cursor goes, what's after stays.
        let (s, cut) = Field::Host.fit(&format!("{}abcEND", "x".repeat(253)), 256);
        assert_eq!(cut, 252..256);
        assert!(s.ends_with("xEND"));
        assert_eq!(s.chars().count(), 255);
        // No room before the cursor: cut at the end.
        let (s, cut) = Field::Host.fit(&"z".repeat(300), 3);
        assert_eq!((s.len(), cut), (255, 255..300));
    }

    #[test]
    fn the_cursor_stays_put_when_characters_go() {
        let s = "a\u{202E}b\u{200B}c";
        assert_eq!(Field::Host.kept_before(s, 0), 0);
        assert_eq!(Field::Host.kept_before(s, 2), 1);
        assert_eq!(Field::Host.kept_before(s, 5), 3);
        assert_eq!(Field::Host.dropped(s), 2);
        assert_eq!(Field::Notes.dropped("a\r\nb"), 0);
    }

    #[test]
    fn lines_from_files_are_safe_to_show() {
        assert_eq!(line(" 9.0.114\n\0", 64), "9.0.114");
        assert_eq!(line("v\u{202E}1", 64), "v1");
        assert_eq!(line(&"9".repeat(100), 8), "99999999");
    }
}
