//! One small HTTP or HTTPS GET of a device's own page, to read what it
//! calls itself: the page's `<title>`, else its `Server` header. Read-only
//! and without logging in. A management page's self-signed certificate is
//! fine here: nothing but the request is sent.

use super::tls;
use rustls::StreamOwned;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Most of a page read: a title sits near the top.
const READ_MAX: usize = 32 * 1024;
/// Longest title kept.
const TITLE_MAX: usize = 80;

/// What a GET of one page answered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub status: u16,
    pub title: Option<String>,
    pub server: Option<String>,
    pub location: Option<String>,
    /// The `Date` header, as sent.
    pub date: Option<String>,
}

impl Page {
    /// What the device calls itself: its title, else its server's name.
    pub fn label(&self) -> Option<String> {
        self.title.clone().or_else(|| self.server.clone())
    }
}

/// GET `path` from `addr`, asking for `host`, over TLS when `https`.
pub fn get(
    addr: SocketAddr,
    host: &str,
    https: bool,
    path: &str,
    timeout: Duration,
) -> io::Result<Page> {
    let mut tcp = TcpStream::connect_timeout(&addr, timeout)?;
    tcp.set_read_timeout(Some(timeout))?;
    tcp.set_write_timeout(Some(timeout))?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: is-alive\r\nAccept: text/html\r\n\
         Connection: close\r\n\r\n"
    );
    let mut buf = Vec::new();
    if https {
        let (mut conn, _verdict) = tls::client(host)?;
        tls::handshake(&mut conn, &mut tcp)?;
        let mut s = StreamOwned::new(conn, tcp);
        s.write_all(req.as_bytes())?;
        read_some(&mut s, &mut buf);
    } else {
        tcp.write_all(req.as_bytes())?;
        read_some(&mut tcp, &mut buf);
    }
    parse(&buf).ok_or_else(|| io::Error::other("not an HTTP answer"))
}

/// Read until the end, `READ_MAX`, a timeout or an error, keeping what came.
fn read_some(r: &mut impl Read, buf: &mut Vec<u8>) {
    let mut chunk = [0u8; 4096];
    while buf.len() < READ_MAX {
        match r.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

/// Read an HTTP response's status, a few headers and the page's title.
pub fn parse(raw: &[u8]) -> Option<Page> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines
        .next()?
        .strip_prefix("HTTP/")?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    let mut page = Page {
        status,
        ..Page::default()
    };
    for l in lines {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        let v = Some(super::sanitize(v.trim())).filter(|v| !v.is_empty());
        match k.trim().to_ascii_lowercase().as_str() {
            "server" => page.server = v,
            "location" => page.location = v,
            "date" => page.date = v,
            _ => {}
        }
    }
    page.title = title(body);
    Some(page)
}

/// A page's `<title>`, entities decoded, on one line, cut to `TITLE_MAX`.
pub fn title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let start = open + lower[open..].find('>')? + 1;
    let end = start + lower[start..].find("</title")?;
    let raw = &html[start..end];
    let decoded = raw
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&nbsp;", " ");
    let one_line = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    let t: String = super::sanitize(&one_line).chars().take(TITLE_MAX).collect();
    Some(t).filter(|t| !t.is_empty())
}

/// Where a redirect from `addr` leads on the same device: (https, port,
/// path). None for another host, or nothing to follow.
pub fn same_device(location: &str, addr: SocketAddr, https: bool) -> Option<(bool, u16, String)> {
    if location.starts_with('/') {
        return Some((https, addr.port(), location.to_string()));
    }
    let (scheme, rest) = location.split_once("://")?;
    let tls = match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" => false,
        _ => return None,
    };
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (hostport, if tls { 443 } else { 80 }),
    };
    (host == addr.ip().to_string()).then(|| (tls, port, path.to_string()))
}

/// What a device's web page calls it: the first of its open web ports
/// that answers with a title (or a server name), following one redirect
/// on the device itself. HTTPS first.
pub fn identify(addr: std::net::IpAddr, open: &[u16], timeout: Duration) -> Option<String> {
    const ORDER: [(u16, bool); 6] = [
        (443, true),
        (8443, true),
        (4433, true),
        (80, false),
        (8080, false),
        (5000, false),
    ];
    let host = addr.to_string();
    // Two ports at most: a device's page is on its first or second.
    for (port, https) in ORDER.into_iter().filter(|(p, _)| open.contains(p)).take(2) {
        let at = SocketAddr::new(addr, port);
        let Ok(page) = get(at, &host, https, "/", timeout) else {
            continue;
        };
        if page.title.is_none()
            && (300..400).contains(&page.status)
            && let Some((tls, p, path)) = page
                .location
                .as_deref()
                .and_then(|l| same_device(l, at, https))
            && let Ok(next) = get(SocketAddr::new(addr, p), &host, tls, &path, timeout)
            && let Some(l) = next.label()
        {
            return Some(l);
        }
        if let Some(l) = page.label() {
            return Some(l);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pages_title_and_headers_are_read() {
        let raw = b"HTTP/1.1 200 OK\r\nServer: lighttpd/1.4\r\nDate: Wed, 11 Nov 2026 06:00:00 GMT\r\n\
                    Content-Type: text/html\r\n\r\n<html><head>\n<TITLE>\n  UniFi &amp; Network\n</TITLE>";
        let p = parse(raw).unwrap();
        assert_eq!(p.status, 200);
        assert_eq!(p.title.as_deref(), Some("UniFi & Network"));
        assert_eq!(p.server.as_deref(), Some("lighttpd/1.4"));
        assert_eq!(p.date.as_deref(), Some("Wed, 11 Nov 2026 06:00:00 GMT"));
        assert_eq!(p.label().as_deref(), Some("UniFi & Network"));
        // No title: the server's name; an empty one isn't a title.
        let p =
            parse(b"HTTP/1.0 200 OK\r\nServer: HP HTTP Server\r\n\r\n<title> </title>").unwrap();
        assert_eq!(p.label().as_deref(), Some("HP HTTP Server"));
        assert!(parse(b"SSH-2.0-OpenSSH_9.6\r\n").is_none());
        // Cut to a line of TITLE_MAX characters, controls taken out.
        let long = format!("<title>{}\u{7}</title>", "x".repeat(200));
        assert_eq!(title(&long).unwrap().chars().count(), TITLE_MAX);
    }

    #[test]
    fn redirects_are_followed_only_on_the_same_device() {
        let at: SocketAddr = "192.0.2.10:80".parse().unwrap();
        assert_eq!(
            same_device("/login.html", at, false),
            Some((false, 80, "/login.html".into()))
        );
        assert_eq!(
            same_device("https://192.0.2.10:8443/manage", at, false),
            Some((true, 8443, "/manage".into()))
        );
        assert_eq!(
            same_device("https://192.0.2.10", at, false),
            Some((true, 443, "/".into()))
        );
        assert_eq!(same_device("https://cloud.example.com/", at, false), None);
        assert_eq!(same_device("ftp://192.0.2.10/", at, false), None);
    }

    /// A real server on this machine: a page with a title, and one that
    /// sends you to another page on the same device.
    #[test]
    fn a_device_is_named_by_its_page() {
        use std::net::TcpListener;
        let serve = |answer: &'static str| {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = l.local_addr().unwrap().port();
            std::thread::spawn(move || {
                for s in l.incoming().take(2).flatten() {
                    let mut s = s;
                    let mut b = [0u8; 1024];
                    let n = s.read(&mut b).unwrap_or(0);
                    let req = String::from_utf8_lossy(&b[..n]);
                    let reply = if req.starts_with("GET /login") {
                        "HTTP/1.1 200 OK\r\n\r\n<title>SonicWall - Authentication</title>"
                    } else {
                        answer
                    };
                    let _ = s.write_all(reply.as_bytes());
                }
            });
            port
        };
        let t = Duration::from_secs(2);
        let ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        let page = serve("HTTP/1.1 200 OK\r\n\r\n<title>Synology DiskStation</title>");
        let got = get(SocketAddr::new(ip, page), "127.0.0.1", false, "/", t).unwrap();
        assert_eq!(got.title.as_deref(), Some("Synology DiskStation"));
        let moved = serve("HTTP/1.1 302 Found\r\nLocation: /login\r\n\r\n");
        let at = SocketAddr::new(ip, moved);
        let first = get(at, "127.0.0.1", false, "/", t).unwrap();
        let (tls, port, path) = same_device(first.location.as_deref().unwrap(), at, false).unwrap();
        let next = get(SocketAddr::new(ip, port), "127.0.0.1", tls, &path, t).unwrap();
        assert_eq!(next.label().as_deref(), Some("SonicWall - Authentication"));
    }
}
