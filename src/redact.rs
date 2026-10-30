//! Redaction for free text the technician writes (tech notes, known
//! problems): anything that looks like a credential, key or personal
//! identifier is replaced before the text is saved, copied or bundled.
//! Zero trust: when in doubt it redacts, and the Notes tab highlights every
//! hit so the technician sees exactly what will go.

use regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Kind {
    Key,
    Password,
    Token,
    Ssn,
    Card,
    Patient,
    Email,
    Phone,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Key => "key or certificate",
            Kind::Password => "password",
            Kind::Token => "token",
            Kind::Ssn => "SSN",
            Kind::Card => "card number",
            Kind::Patient => "patient detail",
            Kind::Email => "email",
            Kind::Phone => "phone",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// Byte range in the text.
    pub span: Range<usize>,
    pub kind: Kind,
}

struct Rule {
    kind: Kind,
    re: Regex,
    /// Capture group holding what to redact (0: the whole match).
    group: usize,
}

fn rule(kind: Kind, group: usize, re: &str) -> Rule {
    Rule {
        kind,
        re: Regex::new(re).expect("valid pattern"),
        group,
    }
}

/// What `sanitize` leaves behind; never redacted again.
static MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[REDACTED: [^\]\n]*\]").expect("valid pattern"));

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    // A value: quoted, or a run of non-space characters.
    const V: &str = r#"("[^"\n]*"|'[^'\n]*'|\S+)"#;
    vec![
        rule(
            Kind::Key,
            0,
            r"(?s)-----BEGIN [A-Z0-9 ]+-----.*?(?:-----END [A-Z0-9 ]+-----|\z)",
        ),
        // Words that are a password whatever follows: "password hunter2".
        rule(
            Kind::Password,
            2,
            &format!(
                r"(?im)\b(pass(?:word|wd|phrase|code)?|pwd|pw|psk|pre-?shared[ -]?key|wpa2?[ -]?key|wi-?fi[ -]?(?:key|pass(?:word)?))\b\s*(?:is\b|[:=]|-)?\s*{V}"
            ),
        ),
        // Words that are only a secret when given a value: "key: …".
        rule(
            Kind::Password,
            2,
            &format!(
                r"(?im)\b(secret|api[ _-]?key|key|token|community|pin|credentials?|p)\s*(?:\bis\b|[:=])\s*{V}"
            ),
        ),
        // user:password@host in a URL.
        rule(Kind::Password, 1, r"://[^\s:/@]+:([^\s@/]+)@"),
        // Well-known key shapes, then anything long and random-looking.
        rule(
            Kind::Token,
            0,
            r"\b(?:AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abpr]-[A-Za-z0-9-]{10,}|sk-[A-Za-z0-9_-]{20,})\b",
        ),
        rule(Kind::Token, 0, r"\b[A-Za-z0-9+/_-]{32,}={0,2}"),
        rule(Kind::Ssn, 0, r"\b\d{3}-\d{2}-\d{4}\b"),
        rule(Kind::Card, 0, r"\b(?:\d[ -]?){12,18}\d\b"),
        // HIPAA identifiers: the rest of the line after the label.
        rule(
            Kind::Patient,
            2,
            r"(?im)\b(patient(?:'s)?(?:\s+name)?|pt\.?\s*name|member\s*id|insurance\s*id|subscriber\s*id)\s*[:#=]\s*([^\n]+)",
        ),
        rule(
            Kind::Patient,
            2,
            r"(?im)\b(mrn|medical\s+record(?:\s+(?:no\.?|number|#))?|dob|d\.o\.b\.|date\s+of\s+birth|birth\s*date|ssn|social\s+security(?:\s+(?:no\.?|number|#))?)\b\s*[:#=-]?\s*([^\n]+)",
        ),
        rule(
            Kind::Email,
            0,
            r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}\b",
        ),
        rule(
            Kind::Phone,
            0,
            r"(?:\+?1[ .-]?)?(?:\(\d{3}\)|\b\d{3})[ .-]?\d{3}[ .-]\d{4}\b",
        ),
    ]
});

/// Luhn check, so serials and part numbers aren't taken for card numbers.
fn luhn(s: &str) -> bool {
    let digits: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let d = d * 2;
                if d > 9 { d - 9 } else { d }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// A long run that has letters and digits: a key, not a word or a number.
fn random_looking(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit()) && s.chars().any(|c| c.is_ascii_alphabetic())
}

/// Everything that would be redacted, in order, without overlaps.
pub fn find(text: &str) -> Vec<Hit> {
    let done: Vec<Range<usize>> = MARKER.find_iter(text).map(|m| m.range()).collect();
    let mut hits: Vec<Hit> = Vec::new();
    for r in RULES.iter() {
        for c in r.re.captures_iter(text) {
            let Some(m) = c.get(r.group) else { continue };
            let s = m.as_str();
            if done.iter().any(|d| m.start() < d.end && d.start < m.end()) {
                continue;
            }
            // The token this match sits in, for the URL check.
            let token_start = text[..m.start()]
                .rfind(char::is_whitespace)
                .map_or(0, |i| i + 1);
            let keep = match r.kind {
                Kind::Card => luhn(s),
                Kind::Token if s.len() >= 32 && !s.starts_with("AKIA") => random_looking(s),
                Kind::Password | Kind::Patient => !s.trim().is_empty(),
                // user:password@host is a URL login, handled above.
                Kind::Email => !text[token_start..m.start()].contains("://"),
                _ => true,
            };
            if keep {
                hits.push(Hit {
                    span: m.range(),
                    kind: r.kind,
                });
            }
        }
    }
    // Earliest first; on overlap the first (wider, by rule order) wins.
    hits.sort_by_key(|h| (h.span.start, std::cmp::Reverse(h.span.end)));
    let mut out: Vec<Hit> = Vec::new();
    for h in hits {
        match out.last_mut() {
            Some(last) if h.span.start < last.span.end => {
                last.span.end = last.span.end.max(h.span.end);
            }
            _ => out.push(h),
        }
    }
    out
}

/// The text with every hit replaced by `[REDACTED: kind]`. Line breaks
/// inside a hit are kept, so line N of the result is line N of the input.
pub fn sanitize(text: &str) -> (String, Vec<Hit>) {
    let hits = find(text);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for h in &hits {
        out += &text[at..h.span.start];
        out += &format!("[REDACTED: {}]", h.kind.label());
        out.extend(text[h.span.clone()].chars().filter(|&c| c == '\n'));
        at = h.span.end;
    }
    out += &text[at..];
    (out, hits)
}

/// "3 redacted: 2 passwords, 1 email".
pub fn summary(hits: &[Hit]) -> String {
    let mut counts: Vec<(Kind, usize)> = Vec::new();
    for h in hits {
        match counts.iter_mut().find(|c| c.0 == h.kind) {
            Some(c) => c.1 += 1,
            None => counts.push((h.kind, 1)),
        }
    }
    counts.sort();
    let parts: Vec<String> = counts
        .iter()
        .map(|(k, n)| format!("{n} {}", k.label()))
        .collect();
    format!("{} redacted: {}", hits.len(), parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red(t: &str) -> String {
        sanitize(t).0
    }

    #[test]
    fn credentials() {
        assert_eq!(
            red("admin password: hunter2!"),
            "admin password: [REDACTED: password]"
        );
        assert_eq!(red("Password hunter2"), "Password [REDACTED: password]");
        assert_eq!(
            red("wifi password is \"Spring 2026!\""),
            "wifi password is [REDACTED: password]"
        );
        assert_eq!(red("PSK=abc123xyz"), "PSK=[REDACTED: password]");
        assert_eq!(
            red("snmp community: public"),
            "snmp community: [REDACTED: password]"
        );
        assert_eq!(
            red("u: admin p: Tr0ub4dor"),
            "u: admin p: [REDACTED: password]"
        );
        assert_eq!(
            red("https://bob:s3cret@nas.local/x"),
            "https://bob:[REDACTED: password]@nas.local/x"
        );
        // Not credentials.
        assert_eq!(
            red("the key issue is the uplink"),
            "the key issue is the uplink"
        );
        assert_eq!(red("reset the token ring"), "reset the token ring");
    }

    #[test]
    fn keys_and_tokens() {
        let pem =
            "before\n-----BEGIN PRIVATE KEY-----\nMIIEv\nabc\n-----END PRIVATE KEY-----\nafter";
        let (s, h) = sanitize(pem);
        assert_eq!(s, "before\n[REDACTED: key or certificate]\n\n\n\nafter");
        assert_eq!(s.lines().count(), pem.lines().count(), "line numbers kept");
        assert_eq!(h.len(), 1);
        assert!(red("api AKIAIOSFODNN7EXAMPLE here").contains("[REDACTED: token]"));
        assert!(red("ghp_abcdefghijklmnopqrstuvwxyz0123456789").contains("REDACTED"));
        assert!(red("k 3f9a1c0e7b2d4a6f8e0c1b3d5f7a9c2e4b6d8f0a").contains("REDACTED"));
        // Long but not random: a path or a word run stays.
        assert_eq!(
            red("see /var/log/unifi/server.log"),
            "see /var/log/unifi/server.log"
        );
        assert_eq!(
            red("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn personal_identifiers() {
        assert_eq!(red("SSN 123-45-6789"), "SSN [REDACTED: SSN]");
        assert_eq!(red("call 123-45-6789"), "call [REDACTED: SSN]");
        assert_eq!(
            red("card 4111 1111 1111 1111 ok"),
            "card [REDACTED: card number] ok"
        );
        assert_eq!(
            red("serial 1234 5678 9012 3456"),
            "serial 1234 5678 9012 3456",
            "fails Luhn"
        );
        assert_eq!(
            red("Patient: Jane Q. Doe\nnext"),
            "Patient: [REDACTED: patient detail]\nnext"
        );
        assert_eq!(red("DOB 01/02/1980"), "DOB [REDACTED: patient detail]");
        assert_eq!(red("MRN# 00123456"), "MRN# [REDACTED: patient detail]");
        assert_eq!(red("patient portal is down"), "patient portal is down");
        assert_eq!(
            red("mail jane.doe@clinic.org now"),
            "mail [REDACTED: email] now"
        );
        assert_eq!(red("(555) 123-4567"), "[REDACTED: phone]");
        assert_eq!(red("+1 555.123.4567"), "[REDACTED: phone]");
    }

    #[test]
    fn network_facts_survive() {
        let t = "gw 10.0.2.1, AP 80:2a:a8:40:69:ec on port 24, VLAN 30, fw 6.6.77.15402, 2026-10-30 07:30, \
uplink fe80::1, id 677d5c3b0b20927705d44671, U7LR";
        assert_eq!(red(t), t);
    }

    #[test]
    fn sanitizing_twice_changes_nothing() {
        let t = "password: x1\nPatient: Bob\nkey = abc";
        let once = red(t);
        assert_eq!(red(&once), once);
        assert_eq!(
            summary(&sanitize(t).1),
            "3 redacted: 2 password, 1 patient detail"
        );
    }
}
