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
            let field =
                ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("example.com"));
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
fn query(server: &str, name: &str) -> Result<String, String> {
    let timeout = Duration::from_secs(8);
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
    let mut bytes = Vec::new();
    stream
        .take(262145)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 262144 {
        return Err("WHOIS response exceeds 256 KiB".into());
    }
    if bytes.is_empty() {
        return Err("WHOIS server returned no records".into());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
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
