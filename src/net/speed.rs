//! Speed test against Cloudflare's speed test service: parallel HTTPS
//! downloads and uploads for throughput, plus the latency maths (jitter,
//! loss, bufferbloat grade, VoIP MOS) for the probes the tool sends
//! before and during the transfers.

use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const HOST: &str = "speed.cloudflare.com";
pub const BASE: &str = "https://speed.cloudflare.com";
/// The file "Download a file" starts with (and the Speed Test's default):
/// Cloudflare's own download, served from the nearest data centre. Each
/// stream fetches it again as soon as it ends, so 25 MB never runs out, and
/// it stays under the size Cloudflare refuses (see DOWN_BYTES).
pub const DEFAULT_FILE: &str = "https://speed.cloudflare.com/__down?bytes=25000000";
/// Latency target when downloading a file: Cloudflare's resolver, nearby
/// on any line, so idle latency, jitter and bufferbloat read the local
/// line rather than the path to wherever the file lives.
pub const FILE_PROBE: &str = "1.1.1.1";
/// Per request. Cloudflare answers 403 to large ones (100 MB was refused in
/// the field; others report 50 MB working), so stay well under that; each
/// stream asks again as soon as one ends. Runs are cut at the deadline.
const DOWN_BYTES: u64 = 25_000_000;
const UP_BYTES: u64 = 25_000_000;
const CHUNK: usize = 64 * 1024;

/// VoIP targets (Cisco / ITU-T G.114): above them calls degrade.
pub const VOIP_LATENCY_MS: f64 = 150.0;
pub const VOIP_JITTER_MS: f64 = 30.0;
pub const VOIP_LOSS_PCT: f64 = 1.0;

/// Where a test runs.
#[derive(Clone, PartialEq, Debug)]
pub enum Server {
    /// Cloudflare's speed test: download, upload and where it ran.
    Cloudflare,
    /// Any file served over HTTP(S), downloaded again and again. Download
    /// only: plain file hosts don't take uploads.
    File(String),
    /// Any URL that takes a POST and throws the body away (your own
    /// server, a LibreSpeed backend's empty.php). Upload only.
    Upload(String),
}

impl Server {
    /// The host name, for latency probes and the report.
    pub fn host(&self) -> Result<String, String> {
        match self {
            Server::Cloudflare => Ok(HOST.into()),
            Server::File(url) | Server::Upload(url) => file_host(url),
        }
    }

    /// Where latency is probed.
    pub fn probe_host(&self) -> Result<String, String> {
        match self {
            Server::Cloudflare => Ok(HOST.into()),
            Server::File(url) | Server::Upload(url) => file_host(url).map(|_| FILE_PROBE.into()),
        }
    }

    pub fn uploads(&self) -> bool {
        matches!(self, Server::Cloudflare | Server::Upload(_))
    }

    pub fn downloads(&self) -> bool {
        !matches!(self, Server::Upload(_))
    }

    /// The URL each stream fetches or posts to.
    pub fn url(&self, dir: Dir) -> String {
        match (self, dir) {
            (Server::Cloudflare, Dir::Down) => format!("{BASE}/__down?bytes={DOWN_BYTES}"),
            (Server::Cloudflare, Dir::Up) => format!("{BASE}/__up"),
            (Server::File(url) | Server::Upload(url), _) => url.trim().to_string(),
        }
    }
}

/// Host of an http(s) URL, or why it can't be used.
pub fn file_host(url: &str) -> Result<String, String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or("the URL must start with https:// or http://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        return Err("the URL has no host".into());
    }
    Ok(host.to_string())
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dir {
    Down,
    Up,
}

/// HTTPS with the OS trust store and proxy, as the domain checks use.
/// Timeouts are per read/write so a long transfer isn't cut, but a stalled
/// one is.
pub fn agent(streams: usize) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_per_read(Some(Duration::from_secs(10)))
        .timeout_per_write(Some(Duration::from_secs(10)))
        // Every stream keeps its connection between requests.
        .max_idle_connections_per_host(streams.max(3))
        .user_agent(concat!("is-alive/", env!("CARGO_PKG_VERSION")))
        // Statuses are read here, so a refusal can say why.
        .http_status_as_error(false)
        .build()
        .into()
}

/// Why a response can't be measured, or None if it can: a refusal, with
/// the reason when the headers give it away, or a web page where a file
/// was expected.
fn refusal(resp: &ureq::http::Response<ureq::Body>, dir: Dir) -> Option<String> {
    let header = |k: &str| {
        resp.headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let code = resp.status().as_u16();
    if !resp.status().is_success() {
        let why = if header("cf-mitigated") == "challenge" {
            ": the site's Cloudflare bot check refuses anything but a browser; use a host that \
serves the file directly"
        } else if code == 429 {
            ": too many requests; the server is rate-limiting"
        } else {
            ""
        };
        return Some(format!("http status: {code}{why}"));
    }
    if dir == Dir::Down && header("content-type").starts_with("text/html") {
        return Some(
            "the URL is a web page, not a file: open it in a browser, copy the actual download \
link (it usually ends in .bin, .zip or .dat) and use that"
                .into(),
        );
    }
    None
}

/// Where the test ran, from Cloudflare's `/meta`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    /// Cloudflare data centre (IATA code) and its city.
    pub colo: Option<String>,
    pub city: Option<String>,
    pub isp: Option<String>,
    pub asn: Option<u64>,
}

/// Best effort: None if the service doesn't answer or the shape changed.
/// The client IP it also returns is not kept.
pub fn meta(agent: &ureq::Agent, base: &str) -> Option<Meta> {
    let mut r = agent
        .get(format!("{base}/meta"))
        .config()
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .call()
        .ok()?;
    let body = r
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .ok()?;
    parse_meta(&body)
}

fn parse_meta(body: &str) -> Option<Meta> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let text = |x: &serde_json::Value| x.as_str().map(super::sanitize).filter(|s| !s.is_empty());
    // `colo` has been both "IAD" and {"iata": "IAD", "city": …}.
    let colo = &v["colo"];
    let m = Meta {
        colo: text(colo).or_else(|| text(&colo["iata"])),
        city: text(&v["city"]).or_else(|| text(&colo["city"])),
        isp: text(&v["asOrganization"]),
        asn: v["asn"].as_u64(),
    };
    (m != Meta::default()).then_some(m)
}

/// Parallel streams moving data until stopped; `bytes` counts what has
/// gone through so far.
pub struct Transfer {
    bytes: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    err: Arc<Mutex<Option<String>>>,
    threads: Vec<JoinHandle<()>>,
}

impl Transfer {
    /// `streams` connections moving data to or from `url` (see `Server::url`).
    pub fn start(agent: &ureq::Agent, url: &str, dir: Dir, streams: usize) -> Self {
        let bytes = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let err = Arc::new(Mutex::new(None));
        let threads = (0..streams.max(1))
            .map(|_| {
                let (agent, bytes, stop, err) =
                    (agent.clone(), bytes.clone(), stop.clone(), err.clone());
                let url = url.to_string();
                std::thread::spawn(move || {
                    let s = Stream {
                        agent,
                        bytes: &bytes,
                        stop: &stop,
                        err: &err,
                    };
                    match dir {
                        Dir::Down => s.download(&url),
                        Dir::Up => s.upload(&url),
                    }
                })
            })
            .collect();
        Self {
            bytes,
            stop,
            err,
            threads,
        }
    }

    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// The last error any stream hit, if one did.
    pub fn error(&self) -> Option<String> {
        self.err.lock().ok()?.clone()
    }

    /// Stop every stream, waiting up to `grace` for them to let go of the
    /// link. One stuck on a stalled read finishes on its own later.
    pub fn stop(self, grace: Duration) {
        self.stop.store(true, Ordering::Relaxed);
        let end = Instant::now() + grace;
        while Instant::now() < end && self.threads.iter().any(|t| !t.is_finished()) {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

struct Stream<'a> {
    agent: ureq::Agent,
    bytes: &'a AtomicU64,
    stop: &'a AtomicBool,
    err: &'a Mutex<Option<String>>,
}

impl Stream<'_> {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn failed(&self, e: impl std::fmt::Display) {
        if self.stopped() {
            return;
        }
        if let Ok(mut slot) = self.err.lock() {
            *slot = Some(e.to_string());
        }
        // Don't hammer a service that refuses us.
        let end = Instant::now() + Duration::from_millis(500);
        while !self.stopped() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn download(&self, url: &str) {
        let mut buf = vec![0u8; CHUNK];
        while !self.stopped() {
            let mut resp = match self.agent.get(url).call() {
                Ok(r) => r,
                Err(e) => {
                    self.failed(e);
                    continue;
                }
            };
            if let Some(why) = refusal(&resp, Dir::Down) {
                self.failed(why);
                continue;
            }
            let mut body = resp.body_mut().as_reader();
            while !self.stopped() {
                match body.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        self.bytes.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    Err(e) => {
                        self.failed(e);
                        break;
                    }
                }
            }
        }
    }

    fn upload(&self, url: &str) {
        while !self.stopped() {
            let mut src = Source {
                left: UP_BYTES,
                bytes: self.bytes,
                stop: self.stop,
            };
            let sent = self
                .agent
                .post(url)
                .header("content-type", "application/octet-stream")
                .header("content-length", UP_BYTES)
                .send(ureq::SendBody::from_reader(&mut src));
            match sent {
                Ok(mut r) => {
                    if let Some(why) = refusal(&r, Dir::Up) {
                        self.failed(why);
                    }
                    let _ = r.body_mut().with_config().limit(64 * 1024).read_to_vec();
                }
                Err(e) => self.failed(e),
            }
        }
    }
}

/// Upload body: zeros, counted as the HTTP client takes them. Once the
/// socket buffer has filled (well inside the warm-up) that is the rate they
/// leave at. Stopping fails the read, which abandons the request.
struct Source<'a> {
    left: u64,
    bytes: &'a AtomicU64,
    stop: &'a AtomicBool,
}

impl Read for Source<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(io::Error::other("stopped"));
        }
        let n = buf.len().min(CHUNK).min(self.left as usize);
        buf[..n].fill(0);
        self.left -= n as u64;
        self.bytes.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Seconds at the start of a transfer left out of its result: TCP slow
/// start and the buffers filling, not the line's rate.
pub fn warmup(secs: f64) -> f64 {
    (secs * 0.2).clamp(1.0, 3.0)
}

pub fn mbps(bytes: u64, secs: f64) -> f64 {
    if secs <= 0.0 {
        0.0
    } else {
        bytes as f64 * 8.0 / secs / 1e6
    }
}

/// "412 Mbps", "38.2 Mbps", "1.25 Gbps".
pub fn fmt_rate(mbps: f64) -> String {
    if mbps >= 1000.0 {
        format!("{:.2} Gbps", mbps / 1000.0)
    } else if mbps >= 100.0 {
        format!("{mbps:.0} Mbps")
    } else if mbps >= 10.0 {
        format!("{mbps:.1} Mbps")
    } else {
        format!("{mbps:.2} Mbps")
    }
}

pub fn fmt_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else {
        format!("{:.1} MB", b / 1e6)
    }
}

/// Latency figures for one phase's probes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Latency {
    pub sent: usize,
    pub lost: usize,
    pub avg: f64,
    pub min: f64,
    pub max: f64,
    /// Mean difference between consecutive replies, as VoIP tools and
    /// RFC 3550 (unsmoothed) measure it.
    pub jitter: f64,
}

impl Latency {
    /// None until there is a reply.
    pub fn of(samples: &[Option<f64>]) -> Option<Self> {
        let got: Vec<f64> = samples.iter().flatten().copied().collect();
        if got.is_empty() {
            return None;
        }
        let jitter = if got.len() > 1 {
            got.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (got.len() - 1) as f64
        } else {
            0.0
        };
        Some(Self {
            sent: samples.len(),
            lost: samples.len() - got.len(),
            avg: got.iter().sum::<f64>() / got.len() as f64,
            min: got.iter().copied().fold(f64::INFINITY, f64::min),
            max: got.iter().copied().fold(0.0, f64::max),
            jitter,
        })
    }

    pub fn loss_pct(&self) -> f64 {
        100.0 * self.lost as f64 / self.sent.max(1) as f64
    }

    /// Estimated call quality for this latency, jitter and loss.
    pub fn mos(&self) -> f64 {
        mos(self.avg, self.jitter, self.loss_pct())
    }
}

/// Mean opinion score (1 to 4.5) from the simplified ITU-T G.107 E-model
/// that VoIP readiness tools use, with the round-trip time as measured.
pub fn mos(latency_ms: f64, jitter_ms: f64, loss_pct: f64) -> f64 {
    let effective = latency_ms + 2.0 * jitter_ms + 10.0;
    let r = if effective < 160.0 {
        93.2 - effective / 40.0
    } else {
        93.2 - (effective - 120.0) / 10.0
    };
    let r = (r - 2.5 * loss_pct).clamp(0.0, 100.0);
    1.0 + 0.035 * r + 0.000007 * r * (r - 60.0) * (100.0 - r)
}

/// G.107's satisfaction bands, by MOS.
pub fn mos_label(mos: f64) -> &'static str {
    match mos {
        m if m >= 4.34 => "Excellent",
        m if m >= 4.03 => "Good",
        m if m >= 3.6 => "Fair",
        m if m >= 3.1 => "Poor",
        _ => "Bad",
    }
}

/// Bufferbloat grade for the latency added under load, on the scale
/// Waveform's bufferbloat test popularised.
pub fn bloat_grade(added_ms: f64) -> &'static str {
    match added_ms {
        a if a < 5.0 => "A+",
        a if a < 30.0 => "A",
        a if a < 60.0 => "B",
        a if a < 200.0 => "C",
        a if a < 400.0 => "D",
        _ => "F",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// Plain-HTTP stand-in for the speed test service: `/__down?bytes=N`
    /// sends N zeros, `/__up` reads the body, `/meta` answers JSON.
    /// Keep-alive, one thread per connection.
    pub(crate) fn mock() -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut w = c.try_clone().unwrap();
                    let mut r = BufReader::new(c);
                    loop {
                        let mut line = String::new();
                        if r.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let mut len = 0u64;
                        loop {
                            let mut h = String::new();
                            if r.read_line(&mut h).unwrap_or(0) == 0 {
                                return;
                            }
                            if h.trim().is_empty() {
                                break;
                            }
                            if let Some((k, v)) = h.split_once(':')
                                && k.eq_ignore_ascii_case("content-length")
                            {
                                len = v.trim().parse().unwrap_or(0);
                            }
                        }
                        if io::copy(&mut (&mut r).take(len), &mut io::sink()).is_err() {
                            return;
                        }
                        let path = line.split_whitespace().nth(1).unwrap_or("");
                        let mut extra = "";
                        let (status, body): (&str, Vec<u8>) = if path == "/page" {
                            extra = "content-type: text/html; charset=utf-8\r\n";
                            ("200 OK", b"<html>download here</html>".to_vec())
                        } else if path == "/challenge" {
                            extra = "cf-mitigated: challenge\r\ncontent-type: text/html\r\n";
                            ("403 Forbidden", b"<html>just a moment</html>".to_vec())
                        } else if let Some(n) = path.strip_prefix("/__down?bytes=") {
                            ("200 OK", vec![0; n.parse::<usize>().unwrap().min(1 << 20)])
                        } else if path == "/__up" {
                            ("200 OK", Vec::new())
                        } else if path == "/meta" {
                            (
                                    "200 OK",
                                    br#"{"colo":"IAD","city":"Ashburn","asOrganization":"Example ISP","asn":64500,"clientIp":"192.0.2.1"}"#.to_vec(),
                                )
                        } else {
                            ("404 Not Found", Vec::new())
                        };
                        let head = format!(
                            "HTTP/1.1 {status}\r\n{extra}content-length: {}\r\n\r\n",
                            body.len()
                        );
                        if w.write_all(head.as_bytes()).is_err() || w.write_all(&body).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        base
    }

    #[test]
    fn transfers_count_bytes_both_ways() {
        let base = mock();
        let agent = agent(2);
        for dir in [Dir::Down, Dir::Up] {
            let url = match dir {
                Dir::Down => format!("{base}/__down?bytes={DOWN_BYTES}"),
                Dir::Up => format!("{base}/__up"),
            };
            let t = Transfer::start(&agent, &url, dir, 2);
            let end = Instant::now() + Duration::from_secs(5);
            while t.bytes() < 4 << 20 && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(t.bytes() >= 4 << 20, "{dir:?}: {} bytes", t.bytes());
            assert_eq!(t.error(), None, "{dir:?}");
            t.stop(Duration::from_secs(2));
        }
    }

    #[test]
    fn refused_transfer_reports_why() {
        let base = mock();
        let t = Transfer::start(&agent(1), &format!("{base}/nope"), Dir::Down, 1);
        // A plain file, fetched again and again.
        let f = Transfer::start(
            &agent(1),
            &format!("{base}/__down?bytes=65536"),
            Dir::Down,
            1,
        );
        let end = Instant::now() + Duration::from_secs(5);
        while f.bytes() < 4 * 65536 && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(f.bytes() >= 4 * 65536, "{} bytes", f.bytes());
        f.stop(Duration::from_secs(1));
        let end = Instant::now() + Duration::from_secs(5);
        while t.error().is_none() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(t.bytes(), 0);
        assert!(t.error().is_some_and(|e| e.contains("404")));
        t.stop(Duration::from_secs(1));
        // A page behind a bot check, and a page where a file should be.
        for (path, says) in [
            ("/challenge", "bot check"),
            ("/page", "web page, not a file"),
        ] {
            let t = Transfer::start(&agent(1), &format!("{base}{path}"), Dir::Down, 1);
            let end = Instant::now() + Duration::from_secs(5);
            while t.error().is_none() && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(t.bytes(), 0, "{path} counted as download");
            let e = t.error().unwrap_or_default();
            assert!(e.contains(says), "{path}: {e}");
            t.stop(Duration::from_secs(1));
        }
    }

    #[test]
    fn servers() {
        let cf = Server::Cloudflare;
        assert_eq!(cf.host().unwrap(), HOST);
        assert!(cf.uploads());
        assert!(cf.url(Dir::Down).ends_with("__down?bytes=25000000"));
        let f = Server::File(" https://files.example.net:8443/1GB.bin?x=1 ".into());
        assert_eq!(f.host().unwrap(), "files.example.net");
        assert_eq!(f.probe_host().unwrap(), FILE_PROBE);
        assert_eq!(cf.probe_host().unwrap(), HOST);
        assert!(Server::File("ftp://x".into()).probe_host().is_err());
        assert_eq!(file_host(DEFAULT_FILE).unwrap(), HOST);
        assert_eq!(
            DEFAULT_FILE,
            Server::Cloudflare.url(Dir::Down),
            "the same download the Cloudflare server uses"
        );
        assert!(!f.uploads());
        let u = Server::Upload(" https://speed.example.net/backend/empty.php ".into());
        assert!(u.uploads() && !u.downloads());
        assert!(cf.downloads() && f.downloads());
        assert_eq!(u.host().unwrap(), "speed.example.net");
        assert_eq!(u.probe_host().unwrap(), FILE_PROBE);
        assert_eq!(
            u.url(Dir::Up),
            "https://speed.example.net/backend/empty.php"
        );
        assert_eq!(
            f.url(Dir::Down),
            "https://files.example.net:8443/1GB.bin?x=1"
        );
        assert_eq!(file_host("http://[2001:db8::1]/f").unwrap(), "2001:db8::1");
        assert_eq!(file_host("http://u:p@10.0.0.5/f").unwrap(), "10.0.0.5");
        assert!(file_host("ftp://x/y").is_err());
        assert!(file_host("https:///y").is_err());
    }

    #[test]
    fn meta_from_service_and_both_shapes() {
        let m = meta(&agent(1), &mock()).unwrap();
        assert_eq!(m.colo.as_deref(), Some("IAD"));
        assert_eq!(m.isp.as_deref(), Some("Example ISP"));
        assert_eq!(m.asn, Some(64500));
        let m = parse_meta(r#"{"colo":{"iata":"FRA","city":"Frankfurt"}}"#).unwrap();
        assert_eq!(
            (m.colo.as_deref(), m.city.as_deref()),
            (Some("FRA"), Some("Frankfurt"))
        );
        assert_eq!(parse_meta(r#"{"other":1}"#), None);
        assert_eq!(parse_meta("<html>"), None);
    }

    #[test]
    fn latency_jitter_and_loss() {
        let l = Latency::of(&[Some(10.0), None, Some(14.0), Some(12.0), Some(12.0)]).unwrap();
        assert_eq!((l.sent, l.lost), (5, 1));
        assert_eq!((l.min, l.avg, l.max), (10.0, 12.0, 14.0));
        // |14-10| + |12-14| + |12-12| over 3 pairs; the lost probe is skipped.
        assert_eq!(l.jitter, 2.0);
        assert_eq!(l.loss_pct(), 20.0);
        assert_eq!(Latency::of(&[None, None]), None);
        assert_eq!(Latency::of(&[Some(5.0)]).unwrap().jitter, 0.0);
    }

    #[test]
    fn mos_follows_the_e_model() {
        // A clean line is as good as G.711 gets.
        let clean = mos(20.0, 2.0, 0.0);
        assert!((4.38..4.41).contains(&clean), "{clean}");
        assert_eq!(mos_label(clean), "Excellent");
        // Jitter counts double, loss heavily.
        assert!((mos(20.0, 40.0, 0.0) - mos(100.0, 0.0, 0.0)).abs() < 1e-9);
        assert_eq!(mos_label(mos(20.0, 2.0, 5.0)), "Fair");
        assert_eq!(mos_label(mos(400.0, 50.0, 0.0)), "Bad");
        assert_eq!(mos(5000.0, 0.0, 100.0), 1.0);
    }

    #[test]
    fn grades_and_formats() {
        assert_eq!(bloat_grade(2.0), "A+");
        assert_eq!(bloat_grade(45.0), "B");
        assert_eq!(bloat_grade(198.0), "C");
        assert_eq!(bloat_grade(900.0), "F");
        assert_eq!(fmt_rate(1250.0), "1.25 Gbps");
        assert_eq!(fmt_rate(412.3), "412 Mbps");
        assert_eq!(fmt_rate(38.24), "38.2 Mbps");
        assert_eq!(fmt_rate(4.5), "4.50 Mbps");
        assert_eq!(mbps(12_500_000, 10.0), 10.0);
        assert_eq!(warmup(10.0), 2.0);
        assert_eq!(warmup(5.0), 1.0);
    }
}
