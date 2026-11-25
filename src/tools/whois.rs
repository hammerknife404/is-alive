//! Registration lookup using the IANA WHOIS directory and registry referral.
use crate::job::{Job, pump};
use crate::widgets;
use eframe::egui;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Default)]
pub struct Whois {
    query: String,
    result: String,
    error: Option<String>,
    job: Option<Job<Result<String, String>>>,
}
impl Whois {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let running = pump(&mut self.job, |r| match r {
            Ok(text) => self.result = text,
            Err(e) => self.error = Some(e),
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Domain or IP");
            let field = widgets::add_text_edit(
                ui,
                egui::TextEdit::singleline(&mut self.query).hint_text("example.com"),
            );
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui
                .add_enabled(!running, egui::Button::new("Look up"))
                .clicked()
                || (enter && !running)
            {
                self.error = None;
                match crate::input::dns_name(&self.query) {
                    Ok(q) => {
                        self.result.clear();
                        self.job = Some(Job::spawn(ui.ctx(), move |tx| tx.send(lookup(&q))));
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            if running {
                widgets::spinner(ui);
                // The lookup ends by itself within its deadline; Stop lets
                // go of it now.
                if ui.button("■ Stop").clicked() {
                    self.job = None;
                    self.error = Some("Stopped".into());
                }
            }
        });
        widgets::toolbar_note(
            ui,
            "Registration records from IANA and the responsible registry. Some registries restrict WHOIS or redact contact information.",
        );
        widgets::error(ui, &self.error);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!self.result.is_empty(), |ui| {
                widgets::copy_button(ui, || self.result.clone());
                widgets::save_button(ui, "whois", || self.result.clone());
            });
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.monospace(&self.result);
        });
    }
}
/// Longest one server may take, from connecting to its last byte: a server
/// that sends a byte every few seconds can't hold Look up for hours.
const WHOLE: Duration = Duration::from_secs(20);

fn query(server: &str, name: &str) -> Result<String, String> {
    let timeout = Duration::from_secs(8);
    let deadline = std::time::Instant::now() + WHOLE;
    let addresses = (server, 43).to_socket_addrs().map_err(|e| e.to_string())?;
    let mut stream = addresses
        .take(4)
        .find_map(|a| TcpStream::connect_timeout(&a, timeout).ok())
        .ok_or_else(|| format!("Cannot reach {server}:43. WHOIS needs outbound TCP port 43."))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(format!("{name}\r\n").as_bytes())
        .map_err(|e| e.to_string())?;
    let bytes = read_reply(&mut stream, deadline, timeout).map_err(|e| format!("{server}: {e}"))?;
    if bytes.len() > 262144 {
        return Err("WHOIS response exceeds 256 KiB".into());
    }
    if bytes.is_empty() {
        return Err("WHOIS server returned no records".into());
    }
    // What a server sends is shown, copied and saved: no control or bidi
    // characters to rearrange or hide what's on screen.
    Ok(crate::net::sanitize_text(&String::from_utf8_lossy(&bytes)))
}
/// Read to the end, past 256 KiB, or until `deadline`: each read waits at
/// most `each`, and never past the deadline.
fn read_reply(
    stream: &mut TcpStream,
    deadline: std::time::Instant,
    each: Duration,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err("took too long to answer".into());
        }
        stream
            .set_read_timeout(Some(left.min(each)))
            .map_err(|e| e.to_string())?;
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(bytes),
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(e.to_string()),
        }
        if bytes.len() > 262144 {
            return Ok(bytes);
        }
    }
}

fn referral(text: &str) -> Option<&str> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        let host = value.trim();
        ((key.eq_ignore_ascii_case("refer") || key.eq_ignore_ascii_case("whois"))
            && host.contains('.')
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
        .then_some(host)
    })
}
fn lookup(name: &str) -> Result<String, String> {
    let directory = query("whois.iana.org", name)?;
    if let Some(server) = referral(&directory) {
        match query(server, name) {
            Ok(record) => Ok(format!("Query: {name}\nServer: {server}\n\n{record}")),
            Err(e) => Ok(format!(
                "IANA directory response\n\n{directory}\nRegistry lookup failed: {e}"
            )),
        }
    } else {
        Ok(format!("Server: whois.iana.org\n\n{directory}"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A directory's reply, damaged any way, gives a referral or None.
    #[test]
    fn damaged_replies_never_panic() {
        crate::hostile::each_text(
            "domain: COM\nrefer: whois.verisign-grs.com\nstatus: ACTIVE\n",
            |t| {
                let _ = referral(t);
            },
        );
    }

    /// A server that sends a byte every so often is given up on at the
    /// deadline, not after hours; what it sends loses its control and
    /// bidi characters but keeps its lines.
    #[test]
    fn a_dripping_server_is_cut_off() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                for _ in 0..100 {
                    if s.write_all(b"x").is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
        let mut s = TcpStream::connect(addr).unwrap();
        let t = std::time::Instant::now();
        let r = read_reply(
            &mut s,
            std::time::Instant::now() + Duration::from_millis(600),
            Duration::from_millis(300),
        );
        assert!(r.is_err() && t.elapsed() < Duration::from_secs(3), "{r:?}");
        assert_eq!(
            crate::net::sanitize_text("Domain: a.com\r\n\u{202E}evil\u{7}\tok\n"),
            "Domain: a.com\nevil\tok\n"
        );
    }
    #[test]
    fn directory_referrals() {
        assert_eq!(
            referral("refer: whois.verisign-grs.com\n"),
            Some("whois.verisign-grs.com")
        );
        assert_eq!(referral("whois: example.com:43"), None);
        assert_eq!(referral("refer: localhost"), None);
    }
}
