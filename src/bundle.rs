//! Site bundle: one zip of the sanitized exports plus a manifest, laid out
//! for the unifidump and mapnetwork skills. Nothing here touches a raw
//! backup; every part is something the app already sanitized.

use crate::net::hostinfo::HostInfo;
use flate2::{Compression, Crc, write::DeflateEncoder};
use serde_json::{Value, json};
use std::io::Write;

pub const UNIFI: &str = "unifi.unifidump.json";
pub const SONICWALL: &str = "sonicwall.json";
pub const INVENTORY: &str = "inventory.json";
pub const RESULTS: &str = "results.json";

/// One file in the bundle.
pub struct Part {
    pub file: &'static str,
    /// Where it came from: the backup or export's file name, or a range.
    pub source: String,
    pub skill: &'static str,
    pub what: &'static str,
    pub summary: Value,
    pub data: String,
}

pub struct Bundle {
    pub site: String,
    pub notes: String,
    pub clients: crate::unifi::Clients,
    pub scanner: HostInfo,
    pub parts: Vec<Part>,
}

impl Bundle {
    fn has(&self, file: &str) -> bool {
        self.parts.iter().any(|p| p.file == file)
    }

    fn notes(&self) -> Option<&str> {
        Some(self.notes.trim()).filter(|n| !n.is_empty())
    }

    pub fn manifest(&self) -> Value {
        let files: Vec<Value> = self
            .parts
            .iter()
            .map(|p| {
                json!({
                    "file": p.file,
                    "source": p.source,
                    "for": p.skill,
                    "what": p.what,
                    "summary": p.summary,
                })
            })
            .collect();
        json!({
            "kind": "is-alive site bundle",
            "format": 1,
            "created_by": format!("is-alive {}", crate::VERSION),
            "created": crate::log::now(),
            "site": self.site.trim(),
            "notes": self.notes(),
            "clients": match self.clients {
                crate::unifi::Clients::None => "not included",
                crate::unifi::Clients::Anonymized =>
                    "anonymized: MAC, vendor and switch port kept; device names replaced with client-<hash> tokens; notes and email dropped",
                crate::unifi::Clients::Full => "included as exported by the controller",
            },
            "scanner": {
                "lan": self.scanner.lan.map(|(ip, bits)| format!("{ip}/{bits}")),
                "gateway": self.scanner.gateway.map(|g| g.to_string()),
                "wan": self.scanner.wan.map(|w| w.to_string()),
            },
            "files": files,
            "mapnetwork_preflight": {
                "site_name": self.site.trim(),
                "scan": self.has(INVENTORY).then_some(INVENTORY),
                "firewall_config": self.has(SONICWALL).then_some(SONICWALL),
                "known_problems": self.notes(),
                "map_clients": false,
                "note": "Answers /mapnetwork's pre-build question. Treat it as answered; don't ask again.",
            },
            "privacy": "Built from sanitized exports: passwords, keys, secrets and certificates were removed on the technician's machine before this bundle was made.",
        })
    }

    /// Message to paste into Claude alongside the bundle.
    pub fn kickoff(&self, file_name: &str) -> String {
        let site = self.site.trim();
        let site = if site.is_empty() { "this site" } else { site };
        let mut s = format!(
            "Attached: {file_name}, an is-alive site bundle for {site}. Unzip it and read manifest.json first.\n\n"
        );
        let mut step = 1;
        if self.has(UNIFI) {
            s += &format!("{step}. Run /unifidump on {UNIFI}.\n");
            step += 1;
        }
        let mut inputs = Vec::new();
        if self.has(UNIFI) {
            inputs.push("the unifidump summary".to_string());
        }
        for f in [SONICWALL, INVENTORY, RESULTS] {
            if self.has(f) {
                inputs.push(f.to_string());
            }
        }
        if !inputs.is_empty() {
            let verb = if step == 1 { "Run" } else { "Then" };
            s += &format!(
                "{step}. {verb} /mapnetwork for {site} (new map), using {}. manifest.json's \
mapnetwork_preflight answers your pre-build question: infrastructure only, no extra inputs coming. \
Don't ask it again.\n",
                join(&inputs)
            );
        }
        if let Some(n) = self.notes() {
            s += &format!("\nKnown problems from the technician: {n}\n");
        }
        s
    }

    /// The bundle as a zip: manifest first, then each part.
    pub fn zip(&self) -> std::io::Result<Vec<u8>> {
        let manifest = serde_json::to_string_pretty(&self.manifest()).unwrap_or_default() + "\n";
        let mut files: Vec<(&str, &[u8])> = vec![("manifest.json", manifest.as_bytes())];
        files.extend(self.parts.iter().map(|p| (p.file, p.data.as_bytes())));
        zip(&files)
    }
}

fn join(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// File-name-safe slug of a site name, e.g. "McClendon Baptist" → "mcclendon-baptist".
pub fn slug(site: &str) -> String {
    let s: String = site
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() { "site".into() } else { s }
}

/// Minimal deflate zip writer (UTF-8 names, no zip64; bundles are small).
fn zip(files: &[(&str, &[u8])]) -> std::io::Result<Vec<u8>> {
    let (time, date) = dos_now();
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data)?;
        let packed = enc.finish()?;
        let mut crc = Crc::new();
        crc.update(data);
        let offset = out.len() as u32;
        let head = |sig: &[u8], central: bool| {
            let mut h = sig.to_vec();
            if central {
                h.extend(20u16.to_le_bytes()); // made by
            }
            h.extend(20u16.to_le_bytes()); // needed to extract
            h.extend(0x0800u16.to_le_bytes()); // UTF-8 names
            h.extend(8u16.to_le_bytes()); // deflate
            h.extend(time.to_le_bytes());
            h.extend(date.to_le_bytes());
            h.extend(crc.sum().to_le_bytes());
            h.extend((packed.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0u16.to_le_bytes()); // extra
            if central {
                h.extend([0u8; 6]); // comment length, disk, internal attrs
                h.extend(0u32.to_le_bytes()); // external attrs
                h.extend(offset.to_le_bytes());
            }
            h.extend(name.as_bytes());
            h
        };
        out.extend(head(b"PK\x03\x04", false));
        out.extend(&packed);
        central.extend(head(b"PK\x01\x02", true));
    }
    let cd_offset = out.len() as u32;
    let n = files.len() as u16;
    out.extend(&central);
    out.extend(b"PK\x05\x06");
    out.extend([0u8; 4]);
    out.extend(n.to_le_bytes());
    out.extend(n.to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(cd_offset.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    Ok(out)
}

/// Local time in DOS format (time, date).
fn dos_now() -> (u16, u16) {
    use chrono::{Datelike, Timelike};
    let t = chrono::Local::now();
    let time = ((t.hour() << 11) | (t.minute() << 5) | (t.second() / 2)) as u16;
    let date = ((((t.year() - 1980).max(0) as u32) << 9) | (t.month() << 5) | t.day()) as u16;
    (time, date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unifi::Clients;
    use std::io::Read;

    fn part(file: &'static str, data: &str) -> Part {
        Part {
            file,
            source: "src".into(),
            skill: "/mapnetwork",
            what: "test",
            summary: json!({}),
            data: data.into(),
        }
    }

    fn bundle(parts: Vec<Part>) -> Bundle {
        Bundle {
            site: " McClendon Baptist ".into(),
            notes: "Closet 2 switch flaky".into(),
            clients: Clients::Anonymized,
            scanner: HostInfo::default(),
            parts,
        }
    }

    /// Read every entry back from the local headers.
    fn unzip(z: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut p = 0;
        while z[p..].starts_with(b"PK\x03\x04") {
            let u16at = |i: usize| u16::from_le_bytes([z[i], z[i + 1]]) as usize;
            let u32at = |i: usize| u32::from_le_bytes(z[i..i + 4].try_into().unwrap()) as usize;
            let (csize, crc) = (u32at(p + 18), u32at(p + 14) as u32);
            let name_len = u16at(p + 26);
            let name = String::from_utf8(z[p + 30..p + 30 + name_len].to_vec()).unwrap();
            let start = p + 30 + name_len + u16at(p + 28);
            let mut data = Vec::new();
            flate2::read::DeflateDecoder::new(&z[start..start + csize])
                .read_to_end(&mut data)
                .unwrap();
            let mut c = Crc::new();
            c.update(&data);
            assert_eq!(c.sum(), crc, "{name} crc");
            out.push((name, data));
            p = start + csize;
        }
        assert!(z[p..].starts_with(b"PK\x01\x02"));
        // Walk the central directory from the end record, as zip readers do.
        let end = z.len() - 22;
        assert!(z[end..].starts_with(b"PK\x05\x06"));
        let u16at = |i: usize| u16::from_le_bytes([z[i], z[i + 1]]) as usize;
        let u32at = |i: usize| u32::from_le_bytes(z[i..i + 4].try_into().unwrap()) as usize;
        let (count, mut c) = (u16at(end + 10), u32at(end + 16));
        assert_eq!(c, p);
        for (name, _) in &out {
            assert!(
                z[c..].starts_with(b"PK\x01\x02"),
                "central record for {name}"
            );
            let n = u16at(c + 28);
            assert_eq!(&z[c + 46..c + 46 + n], name.as_bytes());
            assert!(z[u32at(c + 42)..].starts_with(b"PK\x03\x04"));
            c += 46 + n + u16at(c + 30) + u16at(c + 32);
        }
        assert_eq!((count, c), (out.len(), end));
        out
    }

    #[test]
    fn zip_round_trips() {
        let b = bundle(vec![part(UNIFI, "[1]"), part(INVENTORY, "{\"hosts\":[]}")]);
        let files = unzip(&b.zip().unwrap());
        let names: Vec<_> = files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["manifest.json", UNIFI, INVENTORY]);
        assert_eq!(files[1].1, b"[1]");
        let m: Value = serde_json::from_slice(&files[0].1).unwrap();
        assert_eq!(m["site"], "McClendon Baptist");
        assert_eq!(m["mapnetwork_preflight"]["scan"], INVENTORY);
        assert_eq!(m["mapnetwork_preflight"]["firewall_config"], Value::Null);
        assert_eq!(
            m["mapnetwork_preflight"]["known_problems"],
            "Closet 2 switch flaky"
        );
        assert!(m["clients"].as_str().unwrap().starts_with("anonymized"));
        assert_eq!(m["files"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn kickoff_names_only_whats_there() {
        let full = bundle(vec![
            part(UNIFI, ""),
            part(SONICWALL, ""),
            part(INVENTORY, ""),
        ]);
        let k = full.kickoff("x.bundle.zip");
        assert!(k.contains("1. Run /unifidump on unifi.unifidump.json"));
        assert!(k.contains(
            "2. Then /mapnetwork for McClendon Baptist (new map), using the unifidump summary, \
sonicwall.json and inventory.json"
        ));
        assert!(k.contains("Known problems from the technician: Closet 2 switch flaky"));

        let fw_only = bundle(vec![part(SONICWALL, "")]).kickoff("y.zip");
        assert!(!fw_only.contains("unifidump"));
        assert!(
            fw_only.contains(
                "1. Run /mapnetwork for McClendon Baptist (new map), using sonicwall.json."
            )
        );
    }

    #[test]
    fn slugs() {
        assert_eq!(
            slug(" McClendon Baptist Church "),
            "mcclendon-baptist-church"
        );
        assert_eq!(slug("A&B / C"), "a-b-c");
        assert_eq!(slug("  "), "site");
    }
}
