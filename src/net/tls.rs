//! TLS certificates: what a server presents, read and checked. Who it's
//! for, who issued it, when it expires, the chain it sent, and whether this
//! computer trusts it for the name asked for (the system's own certificate
//! store, through the platform verifier).
//!
//! The handshake always runs to the end, whatever the verdict, so a
//! self-signed or expired certificate can still be read. Nothing is sent
//! after it: the connection closes once the certificate is in.

use super::sanitize;
use chrono::{DateTime, Utc};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned,
};
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Fewer days left than this, and it's flagged: a certificate that renews
/// itself (Let's Encrypt and the like) does so with about 30 days to go, so
/// one under that has stopped renewing.
pub const SOON_DAYS: i64 = 30;

/// One DER element: its tag, its body and what follows it.
pub fn tlv(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *b.first()?;
    let l = *b.get(1)? as usize;
    let (len, h) = if l < 0x80 {
        (l, 2)
    } else {
        let n = l & 0x7f;
        if n == 0 || n > 3 {
            return None;
        }
        let len = b
            .get(2..2 + n)?
            .iter()
            .fold(0usize, |a, &x| a << 8 | x as usize);
        (len, 2 + n)
    };
    Some((tag, b.get(h..h + len)?, &b[h + len..]))
}

/// Runs the platform verifier but always lets the handshake finish,
/// keeping its verdict, so an invalid certificate can still be inspected.
#[derive(Debug)]
pub struct Recording {
    inner: Arc<dyn ServerCertVerifier>,
    verdict: Mutex<Option<rustls::Error>>,
}

impl Recording {
    /// The verifier's complaint, if it had one.
    pub fn verdict(&self) -> Option<rustls::Error> {
        self.verdict
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}

impl ServerCertVerifier for Recording {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let r = self
            .inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp, now);
        *self.verdict.lock().unwrap_or_else(|e| e.into_inner()) = r.err();
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(m, c, d)
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// A client connection to `name` (a host name, or an IP, which is then
/// what the certificate must be valid for), checked against this
/// computer's certificate store.
pub fn client(name: &str) -> io::Result<(ClientConnection, Arc<Recording>)> {
    client_with(name, None)
}

/// As `client`, trusting only `roots` when given (tests: their own CA).
fn client_with(
    name: &str,
    roots: Option<Arc<dyn ServerCertVerifier>>,
) -> io::Result<(ClientConnection, Arc<Recording>)> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let inner: Arc<dyn ServerCertVerifier> = match roots {
        Some(v) => v,
        None => Arc::new(
            rustls_platform_verifier::Verifier::new(provider.clone())
                .map_err(|e| io::Error::other(format!("certificate store: {e}")))?,
        ),
    };
    let verifier = Arc::new(Recording {
        inner,
        verdict: Mutex::new(None),
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(verifier.clone())
        .with_no_client_auth();
    let name = match name.parse::<IpAddr>() {
        Ok(ip) => ServerName::IpAddress(ip.into()),
        Err(_) => ServerName::try_from(name.to_string()).map_err(io::Error::other)?,
    };
    let conn = ClientConnection::new(Arc::new(config), name).map_err(io::Error::other)?;
    Ok((conn, verifier))
}

/// Run the handshake to the end over `tcp`.
pub fn handshake(conn: &mut ClientConnection, tcp: &mut TcpStream) -> io::Result<()> {
    while conn.is_handshaking() {
        conn.complete_io(tcp).map_err(|e| {
            if matches!(
                e.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) {
                io::Error::new(io::ErrorKind::TimedOut, "no TLS reply (timed out)")
            } else {
                io::Error::other(format!(
                    "TLS handshake failed: {e} (servers offering only TLS 1.0/1.1 end up here)"
                ))
            }
        })?;
    }
    Ok(())
}

/// "TLS 1.3, TLS13_AES_256_GCM_SHA384" for a finished handshake.
pub fn negotiated(conn: &ClientConnection) -> (String, String) {
    let v = match conn.protocol_version() {
        Some(rustls::ProtocolVersion::TLSv1_3) => "TLS 1.3".to_string(),
        Some(rustls::ProtocolVersion::TLSv1_2) => "TLS 1.2".to_string(),
        Some(v) => format!("{v:?}"),
        None => "?".into(),
    };
    let c = conn
        .negotiated_cipher_suite()
        .map(|c| format!("{:?}", c.suite()))
        .unwrap_or_else(|| "?".into());
    (v, c)
}

/// What one certificate says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cert {
    /// Its subject's common name (or organisation).
    pub subject: String,
    /// Its issuer's common name (or organisation).
    pub issuer: String,
    /// The names and addresses it's valid for (subject alternative names).
    pub names: Vec<String>,
    pub not_before: Option<DateTime<Utc>>,
    pub not_after: Option<DateTime<Utc>>,
    /// Issued by itself: subject and issuer are the same name.
    pub self_signed: bool,
}

/// Read a DER certificate. None if it isn't one.
pub fn parse(der: &[u8]) -> Option<Cert> {
    let (_, cert, _) = tlv(der)?;
    let (_, tbs, _) = tlv(cert)?;
    let mut rest = tbs;
    let mut fields = Vec::new();
    while !rest.is_empty() {
        let (tag, body, r) = tlv(rest)?;
        fields.push((tag, body));
        rest = r;
    }
    // [0] version is optional; then serial, signature, issuer, validity,
    // subject, key, and the [3] extensions.
    let skip = usize::from(fields.first()?.0 == 0xa0);
    let issuer = fields.get(skip + 2)?.1;
    let validity = fields.get(skip + 3)?.1;
    let subject = fields.get(skip + 4)?.1;
    let (tag_b, before, after) = tlv(validity)?;
    let (tag_a, after, _) = tlv(after)?;
    let names = fields
        .iter()
        .skip(skip + 6)
        .find(|(t, _)| *t == 0xa3)
        .map(|(_, e)| alt_names(e).unwrap_or_default())
        .unwrap_or_default();
    Some(Cert {
        subject: name_field(subject).unwrap_or_else(|| "?".into()),
        issuer: name_field(issuer).unwrap_or_else(|| "?".into()),
        names,
        not_before: parse_time(tag_b, before),
        not_after: parse_time(tag_a, after),
        self_signed: subject == issuer,
    })
}

/// The DNS names and IPs in the subject-alternative-names extension.
fn alt_names(ext: &[u8]) -> Option<Vec<String>> {
    const SAN: &[u8] = &[0x55, 0x1d, 0x11];
    let (_, mut list, _) = tlv(ext)?;
    while !list.is_empty() {
        let (_, e, r) = tlv(list)?;
        list = r;
        let (_, oid, rest) = tlv(e)?;
        if oid != SAN {
            continue;
        }
        // An optional "critical" BOOLEAN, then the OCTET STRING.
        let (mut t, mut v, r) = tlv(rest)?;
        if t == 0x01 {
            (t, v, _) = tlv(r)?;
        }
        if t != 0x04 {
            return None;
        }
        let (_, mut gn, _) = tlv(v)?;
        let mut out = Vec::new();
        while !gn.is_empty() {
            let (tag, body, r) = tlv(gn)?;
            gn = r;
            match (tag, body.len()) {
                (0x82, _) => out.push(sanitize(&String::from_utf8_lossy(body))),
                (0x87, 4) => out.push(IpAddr::from(<[u8; 4]>::try_from(body).ok()?).to_string()),
                (0x87, 16) => out.push(IpAddr::from(<[u8; 16]>::try_from(body).ok()?).to_string()),
                _ => {}
            }
        }
        return Some(out);
    }
    Some(Vec::new())
}

/// The CN of an X.509 Name, falling back to O.
fn name_field(name: &[u8]) -> Option<String> {
    const CN: &[u8] = &[0x55, 0x04, 0x03];
    const O: &[u8] = &[0x55, 0x04, 0x0a];
    let mut found: Vec<(&[u8], String)> = Vec::new();
    let mut rest = name;
    while !rest.is_empty() {
        let (_, set, r) = tlv(rest)?;
        rest = r;
        let (_, atv, _) = tlv(set)?;
        let (_, oid, v) = tlv(atv)?;
        let (_, value, _) = tlv(v)?;
        found.push((oid, sanitize(&String::from_utf8_lossy(value))));
    }
    [CN, O].iter().find_map(|want| {
        found
            .iter()
            .find(|(o, _)| o == want)
            .map(|(_, v)| v.clone())
    })
}

/// UTCTime (0x17, YYMMDDHHMMSSZ) or GeneralizedTime (0x18, YYYYMMDDHHMMSSZ).
fn parse_time(tag: u8, t: &[u8]) -> Option<DateTime<Utc>> {
    let s = std::str::from_utf8(t).ok()?;
    let fmt = match tag {
        0x17 => "%y%m%d%H%M%SZ",
        0x18 => "%Y%m%d%H%M%SZ",
        _ => return None,
    };
    chrono::NaiveDateTime::parse_from_str(s, fmt)
        .ok()
        .map(|n| n.and_utc())
}

/// Why this computer doesn't accept the certificate.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// Issued by a CA it doesn't know: self-signed, a private CA, or a
    /// missing intermediate.
    Untrusted,
    /// Not valid for the name or address asked for.
    Name,
    Expired,
    NotYetValid,
    Revoked,
    Other(String),
}

impl Problem {
    fn from(e: rustls::Error) -> Self {
        match e {
            rustls::Error::InvalidCertificate(c) => match c {
                CertificateError::UnknownIssuer => Problem::Untrusted,
                CertificateError::NotValidForName
                | CertificateError::NotValidForNameContext { .. } => Problem::Name,
                CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
                    Problem::Expired
                }
                CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                    Problem::NotYetValid
                }
                CertificateError::Revoked => Problem::Revoked,
                other => Problem::Other(innermost(&format!("{other:?}"))),
            },
            other => Problem::Other(other.to_string()),
        }
    }
}

/// "Other(OtherError(CaUsedAsEndEntity))" → "CaUsedAsEndEntity": the
/// verifier's own word for it, without the wrapping.
fn innermost(debug: &str) -> String {
    let inner = debug.trim_end_matches(')');
    inner.rsplit('(').next().unwrap_or(inner).to_string()
}

/// What a TLS server presented.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    /// The name (or IP) it was asked for.
    pub name: String,
    /// "TLS 1.3"
    pub version: String,
    pub cipher: String,
    /// As sent: the server's own certificate first.
    pub chain: Vec<Cert>,
    /// The verifier's complaint; None when trusted for `name`.
    pub problem: Option<Problem>,
}

/// How a certificate reads at a glance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Level {
    Good,
    Warn,
    Bad,
}

impl Info {
    pub fn leaf(&self) -> Option<&Cert> {
        self.chain.first()
    }

    /// Whole days until the server's certificate expires; negative once it
    /// has.
    pub fn days_left(&self, now: DateTime<Utc>) -> Option<i64> {
        let end = self.leaf()?.not_after?;
        let d = end - now;
        Some(if d.num_seconds() < 0 {
            -((-d).num_days()).max(1)
        } else {
            d.num_days()
        })
    }

    /// The verdict: how it reads and the words for it, worst first.
    pub fn verdict(&self, now: DateTime<Utc>) -> (Level, Vec<String>) {
        let mut out: Vec<(Level, String)> = Vec::new();
        let leaf = self.leaf();
        match self.days_left(now) {
            Some(d) if d < 0 => out.push((Level::Bad, format!("Expired {} ago", days(-d)))),
            Some(d) if d < SOON_DAYS => out.push((Level::Warn, format!("Expires in {}", days(d)))),
            _ => {}
        }
        if leaf.and_then(|c| c.not_before).is_some_and(|b| b > now) {
            out.push((Level::Bad, "Not valid yet".into()));
        }
        match &self.problem {
            // Dates are read above, from the certificate itself.
            None | Some(Problem::Expired | Problem::NotYetValid) => {}
            // However the verifier words it (an unknown issuer; a CA's
            // certificate used as a server's, as appliances' often are).
            Some(Problem::Untrusted | Problem::Other(_)) if leaf.is_some_and(|c| c.self_signed) => {
                out.push((Level::Warn, "Self-signed: not trusted".into()))
            }
            Some(Problem::Untrusted) => out.push((
                Level::Warn,
                "Not trusted: an issuer this computer doesn't know, or an intermediate missing"
                    .into(),
            )),
            Some(Problem::Name) => out.push((Level::Warn, format!("Not valid for {}", self.name))),
            Some(Problem::Revoked) => out.push((Level::Bad, "Revoked".into())),
            Some(Problem::Other(e)) => out.push((Level::Warn, format!("Not trusted ({e})"))),
        }
        if out.is_empty() {
            return (Level::Good, vec!["Trusted".into()]);
        }
        out.sort_by_key(|(l, _)| matches!(l, Level::Warn));
        let level = out[0].0;
        (level, out.into_iter().map(|(_, t)| t).collect())
    }

    /// One line for a table cell: the worst of the verdict, then whom it's
    /// for and who issued it.
    pub fn summary(&self, now: DateTime<Utc>) -> (Level, String) {
        let (level, words) = self.verdict(now);
        let head = match (level, self.days_left(now)) {
            (Level::Good, Some(d)) => format!("{} left", days(d)),
            _ => words[0].clone(),
        };
        let who = self.leaf().map_or(String::new(), |c| {
            if c.self_signed {
                format!(" · {}", c.subject)
            } else {
                format!(" · {} · {}", c.subject, c.issuer)
            }
        });
        (level, format!("{head}{who}"))
    }

    /// Every detail, as (label, text) rows: the certificate window, the
    /// report and the copy.
    pub fn details(&self, now: DateTime<Utc>) -> Vec<(&'static str, String)> {
        let mut rows = vec![("Status", self.verdict(now).1.join("; "))];
        if let Some(c) = self.leaf() {
            rows.push(("Issued to", c.subject.clone()));
            if !c.names.is_empty() {
                const MAX: usize = 20;
                let mut n = c
                    .names
                    .iter()
                    .take(MAX)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                if c.names.len() > MAX {
                    n.push_str(&format!(" and {} more", c.names.len() - MAX));
                }
                rows.push(("Valid for", n));
            }
            rows.push((
                "Issued by",
                if c.self_signed {
                    format!("{} (itself)", c.issuer)
                } else {
                    c.issuer.clone()
                },
            ));
            let day = |t: Option<DateTime<Utc>>| {
                t.map_or("?".into(), |t| t.format("%Y-%m-%d").to_string())
            };
            let left = match self.days_left(now) {
                Some(d) if d >= 0 => format!(" ({} left)", days(d)),
                Some(d) => format!(" (expired {} ago)", days(-d)),
                None => String::new(),
            };
            rows.push((
                "Valid",
                format!("{} to {}{left}", day(c.not_before), day(c.not_after)),
            ));
        }
        rows.push(("Protocol", format!("{}, {}", self.version, self.cipher)));
        let chain = self
            .chain
            .iter()
            .map(|c| format!("{} ← {}", c.subject, c.issuer))
            .collect::<Vec<_>>();
        rows.push((
            "Chain sent",
            format!(
                "{} certificate{}: {}",
                chain.len(),
                if chain.len() == 1 { "" } else { "s" },
                chain.join(" · ")
            ),
        ));
        rows
    }
}

/// "12 days", "1 day", "3 years".
fn days(d: i64) -> String {
    match d {
        1 => "1 day".into(),
        // Rounded: a ten-year certificate read a day in is "10 years".
        d if d >= 730 => format!("{} years", (d + 182) / 365),
        d => format!("{d} days"),
    }
}

/// Connect to `addr`, run the handshake as for `name`, and read what the
/// server presents.
pub fn probe(name: &str, addr: SocketAddr, timeout: Duration) -> io::Result<Info> {
    probe_with(name, addr, timeout, None)
}

fn probe_with(
    name: &str,
    addr: SocketAddr,
    timeout: Duration,
    roots: Option<Arc<dyn ServerCertVerifier>>,
) -> io::Result<Info> {
    let mut tcp = TcpStream::connect_timeout(&addr, timeout)?;
    tcp.set_read_timeout(Some(timeout))?;
    tcp.set_write_timeout(Some(timeout))?;
    let (mut conn, verifier) = client_with(name, roots)?;
    handshake(&mut conn, &mut tcp)?;
    let (version, cipher) = negotiated(&conn);
    let chain = conn
        .peer_certificates()
        .unwrap_or_default()
        .iter()
        .map(|c| {
            parse(c.as_ref()).unwrap_or_else(|| Cert {
                subject: "(unreadable)".into(),
                ..Cert::default()
            })
        })
        .collect();
    let problem = verifier.verdict().map(Problem::from);
    // Say goodbye; nothing else is sent.
    conn.send_close_notify();
    let _ = StreamOwned::new(conn, tcp).flush();
    Ok(Info {
        name: name.to_string(),
        version,
        cipher,
        chain,
        problem,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, date_time_ymd,
    };
    use rustls::pki_types::PrivateKeyDer;
    use std::net::TcpListener;

    /// A test CA and a server certificate it issued for `names`, valid
    /// from `from` to `to` (y, m, d).
    struct Pki {
        ca: Vec<u8>,
        leaf: Vec<u8>,
        key: Vec<u8>,
    }

    fn pki(names: &[&str], from: (i32, u8, u8), to: (i32, u8, u8)) -> Pki {
        pki_from("is-alive Test CA", names, from, to)
    }

    fn pki_from(ca_name: &str, names: &[&str], from: (i32, u8, u8), to: (i32, u8, u8)) -> Pki {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca = CertificateParams::default();
        ca.distinguished_name = rcgen::DistinguishedName::new();
        ca.distinguished_name.push(DnType::CommonName, ca_name);
        ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca.not_before = date_time_ymd(2025, 1, 1);
        ca.not_after = date_time_ymd(2045, 1, 1);
        let ca_cert = ca.self_signed(&ca_key).unwrap();
        let issuer = Issuer::new(ca, ca_key);

        let key = KeyPair::generate().unwrap();
        let mut p = CertificateParams::new(names.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .unwrap();
        p.distinguished_name = rcgen::DistinguishedName::new();
        p.distinguished_name.push(DnType::CommonName, names[0]);
        p.distinguished_name
            .push(DnType::OrganizationName, "Test Org");
        p.not_before = date_time_ymd(from.0, from.1, from.2);
        p.not_after = date_time_ymd(to.0, to.1, to.2);
        let leaf = p.signed_by(&key, &issuer).unwrap();
        Pki {
            ca: ca_cert.der().to_vec(),
            leaf: leaf.der().to_vec(),
            key: key.serialize_der(),
        }
    }

    /// A TLS server on loopback presenting `chain`, for one connection.
    fn serve(chain: Vec<Vec<u8>>, key: Vec<u8>) -> SocketAddr {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                chain.into_iter().map(CertificateDer::from).collect(),
                PrivateKeyDer::try_from(key).unwrap(),
            )
            .unwrap();
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut tcp, _) = l.accept().unwrap();
            let mut conn = rustls::ServerConnection::new(Arc::new(config)).unwrap();
            while conn.is_handshaking() {
                if conn.complete_io(&mut tcp).is_err() {
                    return;
                }
            }
            let _ = conn.complete_io(&mut tcp);
        });
        addr
    }

    /// Trusts only `ca`, as the platform store trusts its roots.
    fn trusting(ca: &[u8]) -> Arc<dyn ServerCertVerifier> {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(ca.to_vec())).unwrap();
        rustls::client::WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::new(rustls::crypto::ring::default_provider()),
        )
        .build()
        .unwrap()
    }

    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
    }

    const SECS: Duration = Duration::from_secs(5);

    #[test]
    fn reads_a_certificate() {
        let p = pki(
            &["test.example", "www.test.example", "127.0.0.1", "::1"],
            (2025, 1, 1),
            (2030, 6, 1),
        );
        let c = parse(&p.leaf).unwrap();
        assert_eq!(c.subject, "test.example");
        assert_eq!(c.issuer, "is-alive Test CA");
        assert_eq!(
            c.names,
            ["test.example", "www.test.example", "127.0.0.1", "::1"]
        );
        assert_eq!(c.not_before.unwrap().format("%F").to_string(), "2025-01-01");
        assert_eq!(c.not_after.unwrap().format("%F").to_string(), "2030-06-01");
        assert!(!c.self_signed);
        let ca = parse(&p.ca).unwrap();
        assert!(ca.self_signed && ca.names.is_empty());
    }

    #[test]
    fn reads_the_smtp_test_certificate_too() {
        let pem = include_str!("domain/testdata/mx-cert.pem");
        let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap();
        let c = parse(&der).unwrap();
        assert_eq!(c.subject, "mx.example.test");
        assert_eq!(c.not_after.unwrap().format("%F").to_string(), "2030-01-01");
    }

    #[test]
    fn hostile_certificates_never_panic() {
        let p = pki(
            &["test.example", "127.0.0.1", "::1"],
            (2025, 1, 1),
            (2030, 6, 1),
        );
        crate::hostile::each(&p.leaf, |der| {
            let _ = parse(der);
        });
    }

    #[test]
    fn rubbish_is_not_a_certificate() {
        assert!(parse(b"").is_none());
        assert!(parse(&[0x30, 0x03, 0x02, 0x01, 0x00]).is_none());
        assert!(parse(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff]).is_none());
    }

    #[test]
    fn a_trusted_server_with_its_chain() {
        let p = pki(&["test.example", "127.0.0.1"], (2025, 1, 1), (2045, 1, 1));
        let addr = serve(vec![p.leaf.clone(), p.ca.clone()], p.key);
        let i = probe_with("test.example", addr, SECS, Some(trusting(&p.ca))).unwrap();
        assert_eq!(i.problem, None);
        assert!(i.version.starts_with("TLS 1."), "{}", i.version);
        assert_eq!(i.chain.len(), 2);
        assert_eq!(i.chain[1].subject, "is-alive Test CA");
        let now = at(2026, 11, 9);
        let (level, line) = i.summary(now);
        assert_eq!(level, Level::Good);
        assert_eq!(line, "18 years left · test.example · is-alive Test CA");
        let d = i.details(now);
        assert_eq!(d[0], ("Status", "Trusted".to_string()));
        assert!(
            d.iter()
                .any(|(k, v)| *k == "Chain sent" && v.starts_with("2 certificates"))
        );
    }

    #[test]
    fn the_wrong_name_is_flagged() {
        let p = pki(&["test.example"], (2025, 1, 1), (2045, 1, 1));
        let addr = serve(vec![p.leaf.clone()], p.key);
        let i = probe_with("127.0.0.1", addr, SECS, Some(trusting(&p.ca))).unwrap();
        assert_eq!(i.problem, Some(Problem::Name));
        let (level, words) = i.verdict(at(2026, 11, 9));
        assert_eq!(level, Level::Warn);
        assert_eq!(words, ["Not valid for 127.0.0.1"]);
    }

    #[test]
    fn an_unknown_issuer_is_flagged() {
        let p = pki(&["test.example"], (2025, 1, 1), (2045, 1, 1));
        // A CA of another name: one of the same name is taken for the
        // issuer, and fails on its signature instead.
        let other = pki_from("Another CA", &["x.example"], (2025, 1, 1), (2045, 1, 1));
        let addr = serve(vec![p.leaf.clone()], p.key);
        let i = probe_with("test.example", addr, SECS, Some(trusting(&other.ca))).unwrap();
        assert_eq!(i.problem, Some(Problem::Untrusted));
        assert!(i.verdict(at(2026, 11, 9)).1[0].starts_with("Not trusted"));
    }

    #[test]
    fn expiry_reads_from_the_certificate() {
        let leaf = Cert {
            subject: "vpn.example".into(),
            issuer: "R11".into(),
            not_before: Some(at(2026, 9, 1)),
            not_after: Some(at(2026, 11, 30)),
            ..Cert::default()
        };
        let info = Info {
            name: "vpn.example".into(),
            version: "TLS 1.3".into(),
            cipher: "x".into(),
            chain: vec![leaf],
            problem: None,
        };
        // 21 days left: under SOON_DAYS.
        let (level, line) = info.summary(at(2026, 11, 9));
        assert_eq!(level, Level::Warn);
        assert_eq!(line, "Expires in 21 days · vpn.example · R11");
        // Plenty left.
        assert_eq!(info.summary(at(2026, 9, 2)).0, Level::Good);
        // Expired: the verifier says so too, but the dates are what's shown.
        let gone = Info {
            problem: Some(Problem::Expired),
            ..info.clone()
        };
        let (level, words) = gone.verdict(at(2026, 12, 3));
        assert_eq!(level, Level::Bad);
        assert_eq!(words, ["Expired 3 days ago"]);
        // Not yet valid.
        assert_eq!(info.verdict(at(2026, 8, 1)).1, ["Not valid yet"]);
    }

    #[test]
    fn a_self_signed_appliance_reads_as_such() {
        let leaf = Cert {
            subject: "UniFi".into(),
            issuer: "UniFi".into(),
            not_before: Some(at(2024, 1, 1)),
            not_after: Some(at(2034, 1, 1)),
            self_signed: true,
            ..Cert::default()
        };
        let info = Info {
            name: "10.0.0.2".into(),
            version: "TLS 1.2".into(),
            cipher: "x".into(),
            chain: vec![leaf],
            problem: Some(Problem::Untrusted),
        };
        let (level, line) = info.summary(at(2026, 11, 9));
        assert_eq!(level, Level::Warn);
        assert_eq!(line, "Self-signed: not trusted · UniFi");
        let d = info.details(at(2026, 11, 9));
        assert!(d.contains(&("Issued by", "UniFi (itself)".to_string())));
    }

    #[test]
    fn a_plain_text_service_is_no_tls() {
        // An SSH-like server: a banner, then it hangs up.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let _ = s.write_all(b"SSH-2.0-OpenSSH_9.6\r\n");
        });
        let e = probe("test.example", addr, SECS).unwrap_err();
        assert!(e.to_string().contains("TLS handshake failed"), "{e}");
    }

    #[test]
    fn a_self_signed_ca_used_as_a_server_reads_as_self_signed() {
        let leaf = Cert {
            subject: "UniFi".into(),
            issuer: "UniFi".into(),
            self_signed: true,
            ..Cert::default()
        };
        let info = Info {
            name: "10.0.0.2".into(),
            version: "TLS 1.3".into(),
            cipher: "x".into(),
            chain: vec![leaf],
            problem: Some(Problem::Other(innermost(
                "Other(OtherError(CaUsedAsEndEntity))",
            ))),
        };
        assert_eq!(
            info.verdict(at(2026, 11, 9)).1,
            ["Self-signed: not trusted"]
        );
        assert_eq!(innermost("BadSignature"), "BadSignature");
    }

    #[test]
    fn days_read_naturally() {
        assert_eq!(days(1), "1 day");
        assert_eq!(days(29), "29 days");
        assert_eq!(days(729), "729 days");
        assert_eq!(days(800), "2 years");
        assert_eq!(days(3649), "10 years");
    }
}
