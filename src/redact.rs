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
    /// Picked by the technician.
    Manual,
}

impl Kind {
    fn plural(self) -> &'static str {
        match self {
            Kind::Key => "keys or certificates",
            Kind::Password => "passwords",
            Kind::Token => "tokens",
            Kind::Ssn => "SSNs",
            Kind::Card => "card numbers",
            Kind::Patient => "patient details",
            Kind::Email => "emails",
            Kind::Phone => "phone numbers",
            Kind::Manual => "picked by hand",
        }
    }

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
            Kind::Manual => "by hand",
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
        // Words that are only a secret when given a value: "key: …".
        // ("password hunter2" and friends are read by `password_values`.)
        rule(
            Kind::Password,
            2,
            &format!(
                r"(?im)\b(secret|api[ _-]?key|key|token|community|pin|credentials?|p)\s*(?:\bis\b|\bwas\b|[:=])\s*{V}"
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

/// Words that mean a password follows, however it's phrased.
static PASSWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:wi-?fi[ -]?(?:key|pass(?:word)?)|wpa2?[ -]?key|pre-?shared[ -]?key|pass(?:word|wd|phrase|code)?|pwd|pw|psk)\b",
    )
    .expect("valid pattern")
});

/// Between "password" and its value: "her password **was** hunter2".
const LINKS: &[&str] = &["is", "was", "are", "were", ":", "=", "-", "->"];
/// Words to step over looking for the value.
const FILLER: &[&str] = &[
    "the",
    "a",
    "an",
    "her",
    "his",
    "my",
    "our",
    "their",
    "your",
    "its",
    "this",
    "that",
    "new",
    "old",
    "set",
    "changed",
    "reset",
    "updated",
    "rotated",
    "to",
    "for",
    "of",
    "on",
    "at",
    "as",
    "now",
    "still",
    "temp",
    "temporary",
    "default",
    "current",
    "same",
    "also",
    "just",
    "be",
    "been",
    "will",
];
/// Words that say the sentence is about passwords, not giving one.
const NOT_VALUE: &[&str] = &[
    "policy",
    "policies",
    "manager",
    "vault",
    "expired",
    "expires",
    "expiry",
    "expiration",
    "required",
    "requires",
    "requirement",
    "requirements",
    "complexity",
    "length",
    "rule",
    "rules",
    "done",
    "protected",
    "prompt",
    "field",
    "history",
    "hint",
    "recovery",
    "change",
    "changes",
    "works",
    "worked",
    "working",
    "fixed",
    "needed",
    "saved",
    "stored",
    "missing",
    "wrong",
    "incorrect",
    "invalid",
    "issue",
    "issues",
    "problem",
    "error",
    "less",
    "login",
    "sync",
];

/// Ordinary words: after "password" with no "is" or ":" they're the
/// sentence going on ("give it a password when prompted"), not a value.
const PLAIN: &[&str] = &[
    "when", "while", "if", "then", "and", "or", "but", "so", "because", "until", "before", "after",
    "once", "unless", "with", "without", "from", "into", "in", "by", "via", "per", "you", "we",
    "they", "he", "she", "it", "i", "me", "us", "them", "him", "someone", "user", "users",
    "anyone", "everyone", "nobody", "here", "there", "again", "below", "above", "later", "first",
    "too", "only", "every", "each", "all", "any", "some", "none", "no", "not", "yes", "which",
    "who", "what", "where", "why", "how", "should", "would", "could", "can", "must", "may",
    "might", "do", "does", "did", "has", "have", "had", "get", "gets", "got", "give", "gave",
    "given", "enter", "entered", "type", "typed", "use", "used", "using", "see", "sent", "send",
    "ask", "asked", "comes", "came", "pops", "shows", "shown", "box", "screen", "window", "dialog",
    "page", "box", "form", "please", "like", "unknown",
];

/// Shaped like a password: lowercase, a digit, and an uppercase letter or a
/// symbol ("Password11", "JBpass23$"). Serials, model numbers and hostnames
/// are uppercase or carry dots, colons and slashes, so they don't qualify.
fn password_shaped(t: &str) -> bool {
    let sym = |c: char| "!@#$%^&*+=?~".contains(c);
    t.chars().any(|c| c.is_ascii_lowercase())
        && t.chars().any(|c| c.is_ascii_digit())
        && t.chars().any(|c| c.is_ascii_uppercase() || sym(c))
        && !t.contains("://")
        && !t.starts_with("www.")
}

/// Passwords with no word to announce them: a line that is one
/// password-shaped token (a login written as user, then password, on lines
/// of their own), and anywhere, a token strong enough to be nothing else:
/// eight or more characters with upper, lower, digit and symbol
/// ("vpn: jsmith / Spring26$").
fn bare_passwords(text: &str) -> Vec<Range<usize>> {
    let sym = |c: char| "!@#$%^&*+=?~".contains(c);
    let mut out = Vec::new();
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let lead = body.len() - body.trim_start().len();
        let token = body.trim();
        let alone = !token.is_empty()
            && !token.contains(char::is_whitespace)
            && (6..=64).contains(&token.len())
            && !token.contains(['.', ':', '/', '\\', '='])
            // "USW-24-PoE" is a model; a hyphenated password has a symbol too.
            && (!token.contains('-') || token.chars().any(|c| "!@#$%^&*+?~".contains(c)))
            && password_shaped(token);
        if alone {
            let at = line_start + lead;
            out.push(at..at + token.len());
        } else {
            let mut at = line_start;
            for word in body.split(char::is_whitespace) {
                let t = word.trim_matches(|c: char| ",;()[]{}\"'".contains(c));
                let start = at + word.find(t).unwrap_or(0);
                let email = t
                    .split_once('@')
                    .is_some_and(|(_, d)| d.contains('.') && !d.ends_with('.'));
                if t.len() >= 8
                    && t.chars().any(|c| c.is_ascii_uppercase())
                    && t.chars().any(sym)
                    && password_shaped(t)
                    // "key=value" is read by the rules that know the key.
                    && !t.contains('=')
                    && !email
                {
                    out.push(start..start + t.len());
                }
                at += word.len() + 1;
            }
        }
        line_start += line.len();
    }
    out
}

/// The value after each password word, read like a sentence: after a link
/// ("is", "was", ":", "=") the first real word; with no link, the first word
/// that isn't filler, unless the sentence is about passwords ("password
/// policy", "password reset done") or just goes on ("password when
/// prompted"). Stops at the end of the line or sentence.
fn password_values(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for m in PASSWORD.find_iter(text) {
        let line_end = text[m.end()..]
            .find('\n')
            .map_or(text.len(), |i| m.end() + i);
        // Tokens up to the line or sentence end; a quoted value is one token,
        // and ":x" / "=x" give up their separator.
        let mut toks: Vec<(Range<usize>, bool)> = Vec::new();
        let mut i = m.end();
        while i < line_end && toks.len() < 12 {
            let rest = &text[i..line_end];
            let skip = rest.len() - rest.trim_start().len();
            i += skip;
            if i >= line_end {
                break;
            }
            let rest = &text[i..line_end];
            let first = rest.chars().next().expect("non-empty");
            if matches!(first, ':' | '=') && !rest.starts_with("->") {
                toks.push((i..i + 1, true));
                i += 1;
                continue;
            }
            let len = if matches!(first, '"' | '\'') {
                rest[1..].find(first).map_or(rest.len(), |j| j + 2)
            } else {
                rest.find(char::is_whitespace).unwrap_or(rest.len())
            };
            let word = rest[..len]
                .trim_matches(|c: char| ".,;:)(!?".contains(c))
                .to_lowercase();
            toks.push((
                i..i + len,
                LINKS.contains(&word.as_str()) || LINKS.contains(&&rest[..len]),
            ));
            i += len;
            // A sentence ends at ". " (a password can end in "!", so not there).
            if rest[..len].ends_with(['.', ';', '?']) {
                break;
            }
        }
        let word = |r: &Range<usize>| {
            text[r.clone()]
                .trim_matches(|c: char| ".,;:)(!?".contains(c))
                .to_lowercase()
        };
        let pick = match toks.iter().position(|t| t.1) {
            Some(link) => toks[link + 1..]
                .iter()
                .find(|t| !t.1 && !FILLER.contains(&word(&t.0).as_str())),
            None => toks
                .iter()
                .find(|t| !FILLER.contains(&word(&t.0).as_str()))
                .filter(|t| {
                    let w = word(&t.0);
                    !NOT_VALUE.contains(&w.as_str()) && !PLAIN.contains(&w.as_str())
                }),
        };
        if let Some((r, _)) = pick {
            // "hunter2." keeps its full stop out of the redaction.
            let v = &text[r.clone()];
            let end = r.start + v.trim_end_matches(['.', ',', ';', ')']).len();
            if end > r.start {
                out.push(r.start..end);
            }
        }
    }
    out
}

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

/// Where earlier redactions left their markers.
pub fn markers(text: &str) -> Vec<Range<usize>> {
    MARKER.find_iter(text).map(|m| m.range()).collect()
}

/// The technician's own calls: values to redact that nothing detects, and
/// detected values that are fine to keep.
#[derive(Clone, Debug, Default, PartialEq, Hash)]
pub struct Overrides {
    pub redact: Vec<String>,
    pub allow: Vec<String>,
}

impl Overrides {
    /// Redact `value` everywhere it appears (and stop allowing it).
    pub fn add_redact(&mut self, value: &str) -> bool {
        let v = value.trim();
        if v.chars().count() < 2 {
            return false;
        }
        self.allow.retain(|a| a != v);
        if !self.redact.iter().any(|r| r == v) {
            self.redact.push(v.to_string());
        }
        true
    }

    /// Keep `value` as written, wherever it appears.
    pub fn add_allow(&mut self, value: &str) {
        let v = value.trim();
        if v.is_empty() {
            return;
        }
        self.redact.retain(|r| r != v);
        if !self.allow.iter().any(|a| a == v) {
            self.allow.push(v.to_string());
        }
    }
}

/// Everything that would be redacted, in order, without overlaps.
pub fn find(text: &str) -> Vec<Hit> {
    find_with(text, &Overrides::default())
}

/// As `find`, with the technician's overrides: their picks are added,
/// whatever they allowed is left out.
pub fn find_with(text: &str, ov: &Overrides) -> Vec<Hit> {
    let mut hits: Vec<Hit> = detect(text)
        .into_iter()
        .filter(|h| !ov.allow.iter().any(|a| a == text[h.span.clone()].trim()))
        .collect();
    let done: Vec<Range<usize>> = MARKER.find_iter(text).map(|m| m.range()).collect();
    for v in ov.redact.iter().filter(|v| !v.is_empty()) {
        for (i, _) in text.match_indices(v.as_str()) {
            let span = i..i + v.len();
            if !done
                .iter()
                .any(|d| span.start < d.end && d.start < span.end)
            {
                hits.push(Hit {
                    span,
                    kind: Kind::Manual,
                });
            }
        }
    }
    merge(hits)
}

fn detect(text: &str) -> Vec<Hit> {
    let done: Vec<Range<usize>> = MARKER.find_iter(text).map(|m| m.range()).collect();
    let fresh = |r: &Range<usize>| !done.iter().any(|d| r.start < d.end && d.start < r.end);
    let mut hits: Vec<Hit> = password_values(text)
        .into_iter()
        .chain(bare_passwords(text))
        .filter(|r| fresh(r))
        .map(|span| Hit {
            span,
            kind: Kind::Password,
        })
        .collect();
    for r in RULES.iter() {
        for c in r.re.captures_iter(text) {
            let Some(m) = c.get(r.group) else { continue };
            let s = m.as_str();
            if !fresh(&m.range()) {
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
    merge(hits)
}

/// Earliest first; on overlap the first (wider, by rule order) wins.
fn merge(mut hits: Vec<Hit>) -> Vec<Hit> {
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
    sanitize_with(text, &Overrides::default())
}

/// As `sanitize`, with the technician's overrides.
pub fn sanitize_with(text: &str, ov: &Overrides) -> (String, Vec<Hit>) {
    let hits = find_with(text, ov);
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
        .map(|(k, n)| format!("{n} {}", if *n == 1 { k.label() } else { k.plural() }))
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
        // Read like a sentence: the value after "was", not "was".
        assert_eq!(
            red("There is no way her password was hunter2."),
            "There is no way her password was [REDACTED: password]."
        );
        let r = red("password for the router is Spring2026!");
        assert_eq!(r, "password for the router is [REDACTED: password]");
        assert_eq!(red("pw:x1 then"), "pw:[REDACTED: password] then");
        assert_eq!(
            red("the password -> Tr0ub4dor"),
            "the password -> [REDACTED: password]"
        );
        assert_eq!(
            red("set the new password to Winter25"),
            "set the new password to [REDACTED: password]"
        );
        assert_eq!(red("password reset done"), "password reset done");
        assert_eq!(
            red("password policy requires 12 chars"),
            "password policy requires 12 chars"
        );
        assert_eq!(
            red("Password reset.\nnext line"),
            "Password reset.\nnext line"
        );
        assert_eq!(
            red("the api key was abc123"),
            "the api key was [REDACTED: password]"
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

    /// A real-world note's shape (names and values made up).
    #[test]
    fn notes_as_techs_write_them() {
        let t = "jane Doe, NEW USER - also needs vpn Copy Sam Lee\n    \
jdoe@example.com\n    JDpass23$\n         VPN: jane.doe / JDpass23%\n\n\
DSmith-LAPTOP - Out - Dan Smith\ndsmith\nPassword11\nNeeds sage credentials\n\
DSmith\n\nGV Edge Recording Manager - export data from old install: System > Export - \
give it a password when prompt comes up. Import it the same - Enter password you gave during export.";
        let r = red(t);
        for gone in ["JDpass23$", "JDpass23%", "Password11", "jdoe@example.com"] {
            assert!(!r.contains(gone), "{gone} left in:\n{r}");
        }
        for kept in [
            "a password when prompt",
            "Enter password you gave",
            "jane.doe",
            "DSmith-LAPTOP",
            "dsmith",
            "Needs sage credentials",
        ] {
            assert!(r.contains(kept), "{kept} redacted:\n{r}");
        }
        assert_eq!(summary(&sanitize(t).1), "4 redacted: 3 passwords, 1 email");
    }

    #[test]
    fn bare_values_need_the_shape() {
        // Strong anywhere; weaker only alone on a line.
        assert_eq!(
            red("login jsmith / Spring26$ ok"),
            "login jsmith / [REDACTED: password] ok"
        );
        assert_eq!(red("Welcome1\n"), "[REDACTED: password]\n");
        for t in [
            "Win11",
            "Office 365 E3",
            "SN 2CB8EDA78330",
            "USW-24-PoE",
            "fw SonicOS-7.3.3",
            "C:\\Users\\jsmith1",
            "https://portal.example.com/a?B=1&c=Zz9!",
            "Fixed on Win11!",
            "dlee134",
            "50% done, R&D",
        ] {
            assert_eq!(red(t), t);
        }
    }

    #[test]
    fn overrides_redact_and_allow() {
        let t = "Lockbox code 4471 at the door. License Office365\nlockbox again: 4471";
        assert_eq!(
            red(t),
            "Lockbox code 4471 at the door. License Office365\nlockbox again: 4471"
        );
        let mut ov = Overrides::default();
        assert!(ov.add_redact(" 4471 "));
        assert!(!ov.add_redact("x"));
        let (clean, hits) = sanitize_with(t, &ov);
        assert_eq!(
            clean,
            "Lockbox code [REDACTED: by hand] at the door. License Office365\nlockbox again: [REDACTED: by hand]"
        );
        assert_eq!(summary(&hits), "2 redacted: 2 picked by hand");

        let t = "Office365\npassword: hunter2";
        assert_eq!(find(t).len(), 2);
        let mut ov = Overrides::default();
        ov.add_allow("Office365");
        let (clean, _) = sanitize_with(t, &ov);
        assert_eq!(clean, "Office365\npassword: [REDACTED: password]");
        // Redacting it again takes it off the allow list.
        ov.add_redact("Office365");
        assert!(ov.allow.is_empty());
        assert_eq!(sanitize_with(t, &ov).1.len(), 2);
        // Twice is still a no-op.
        let once = sanitize_with(t, &ov).0;
        assert_eq!(sanitize_with(&once, &ov).0, once);
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
            "3 redacted: 2 passwords, 1 patient detail"
        );
    }
}
