//! UniFi backup → sanitized JSON for the unifidump skill, entirely offline.
//!
//! Output is the mongoexport-style array unifidump's `extract.py` already
//! reads: `{"__cmd":"select","collection":…}` markers, each followed by that
//! collection's documents. Only the collections the summary uses are kept
//! (admins, events, stats, settings and the rest never leave the backup),
//! and every password, key and secret field inside them is removed.

mod bson;
mod container;

use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Sites, devices, networks, switch port profiles, WLANs.
pub const CONFIG: &[&str] = &["site", "device", "networkconf", "portconf", "wlanconf"];
/// Client (station) table: hostnames, MACs, last IPs.
pub const CLIENTS: &str = "user";

/// How the client table goes out.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Clients {
    #[default]
    None,
    /// MAC, vendor and switch port kept; names replaced with a token.
    Anonymized,
    Full,
}

impl Clients {
    pub const ALL: [Clients; 3] = [Clients::None, Clients::Anonymized, Clients::Full];

    pub fn label(self) -> &'static str {
        match self {
            Clients::None => "none",
            Clients::Anonymized => "anonymized",
            Clients::Full => "full",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Clients::None => "Leave the client table out.",
            Clients::Anonymized => {
                "Keep each client's MAC, vendor and switch port (what unifidump needs to spot \
unadopted gear) but replace device names, which are often people's names, with a token \
like client-1a2b3c4d. Notes and email addresses are dropped."
            }
            Clients::Full => "Keep the client table as is, names included.",
        }
    }
}

/// Fields of a client record that name a person or hold their details.
const PERSONAL_NAMES: &[&str] = &["name", "hostname", "display_name", "local_dns_record"];

/// Replace a client's names with a stable token from its MAC (the MAC is
/// kept anyway, so the token reveals nothing new) and drop notes, email and
/// phone fields. Returns whether anything changed.
fn anonymize(doc: &mut Map<String, Value>) -> bool {
    let mac = doc.get("mac").and_then(Value::as_str).unwrap_or_default();
    let token = format!(
        "client-{:08x}",
        mac.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32)
            .wrapping_mul(0x0100_0193))
    );
    let mut changed = false;
    for k in PERSONAL_NAMES {
        if let Some(v) = doc.get_mut(*k)
            && v.is_string()
        {
            *v = Value::String(token.clone());
            changed = true;
        }
    }
    let before = doc.len();
    doc.retain(|k, _| {
        let k = k.to_ascii_lowercase();
        k != "note" && !k.contains("email") && !k.contains("phone")
    });
    changed || doc.len() != before
}

/// Field-name fragments that mark a secret. UniFi prefixes stored secrets
/// with `x_` (x_passphrase, x_authkey, x_wan_password, …); the rest catch
/// RADIUS secrets, PPSK lists, VPN keys and certificates by name.
const SECRET: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "secret",
    "psk",
    "preshared",
    "private",
    "authkey",
    "auth_key",
    "api_key",
    "token",
    "credential",
    "cert",
    "hash",
    "wep_key",
    "iapp_key",
    "ssh_key",
];

fn is_secret(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.starts_with("x_")
        || k.ends_with("key")
        || k.ends_with("keys")
        || SECRET.iter().any(|s| k.contains(s))
}

/// Key or certificate material in a value whatever the field is called,
/// e.g. an OpenVPN client config pasted into a network's settings.
fn holds_secret(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.contains("-----BEGIN") || s.contains("<key>") || s.contains("PRIVATE KEY")
    })
}

/// Drop secret fields at any depth; count each by `collection.path.field`.
fn redact(v: &mut Value, path: &str, removed: &mut BTreeMap<String, usize>) {
    match v {
        Value::Object(m) => {
            m.retain(|k, v| {
                let keep = !is_secret(k) && !holds_secret(v);
                if !keep {
                    *removed.entry(format!("{path}.{k}")).or_default() += 1;
                }
                keep
            });
            for (k, child) in m.iter_mut() {
                redact(child, &format!("{path}.{k}"), removed);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|c| redact(c, path, removed)),
        _ => {}
    }
}

pub struct Report {
    pub layers: Vec<String>,
    pub version: Option<String>,
    pub sites: Vec<String>,
    /// (collection, documents) in output order.
    pub kept: Vec<(String, usize)>,
    pub dropped: BTreeMap<String, usize>,
    /// `collection.field` → times removed.
    pub redacted: BTreeMap<String, usize>,
    pub clients: Clients,
    /// Client records whose names were replaced.
    pub anonymized: usize,
    pub json: String,
}

impl Report {
    pub fn count(&self, coll: &str) -> usize {
        self.kept.iter().filter(|k| k.0 == coll).map(|k| k.1).sum()
    }

    pub fn summary(&self) -> String {
        let mut s = format!("Format     {}\n", self.layers.join(" → "));
        if let Some(v) = &self.version {
            s += &format!("Network    {v}\n");
        }
        s += &format!(
            "Sites      {} ({})\n",
            self.sites.len(),
            self.sites.join(", ")
        );
        s += &format!(
            "Output     {:.1} KB, mongoexport JSON for unifidump\n\nKept\n",
            self.json.len() as f64 / 1024.0
        );
        for c in CONFIG {
            s += &format!("  {c:<14}{:>7}\n", self.count(c));
        }
        s += &match self.clients {
            Clients::None => format!("  {CLIENTS:<14}{:>7}  (left out)\n", 0),
            Clients::Anonymized => format!(
                "  {CLIENTS:<14}{:>7}  (anonymized: {} renamed)\n",
                self.count(CLIENTS),
                self.anonymized
            ),
            Clients::Full => format!("  {CLIENTS:<14}{:>7}\n", self.count(CLIENTS)),
        };
        s += &format!(
            "\nLeft in the backup: {} collection(s), {} document(s)\n",
            self.dropped.len(),
            self.dropped.values().sum::<usize>()
        );
        for (c, n) in &self.dropped {
            s += &format!("  {c:<28}{n:>7}\n");
        }
        s += &format!(
            "\nSecret fields removed: {}\n",
            self.redacted.values().sum::<usize>()
        );
        for (f, n) in &self.redacted {
            s += &format!("  {f:<44}{n:>5}\n");
        }
        s
    }
}

/// Decrypt and unpack a backup, keep the config collections (plus clients
/// if asked) and strip secrets.
pub fn convert(data: Vec<u8>, clients: Clients) -> Result<Report, String> {
    let dump = container::open(data)?;
    let keep = |c: &str| CONFIG.contains(&c) || (clients != Clients::None && c == CLIENTS);
    let mut anonymized = 0;

    let mut out: Vec<String> = Vec::new();
    let mut kept: Vec<(String, usize)> = Vec::new();
    let mut dropped = BTreeMap::new();
    let mut redacted = BTreeMap::new();
    let mut sites = Vec::new();
    let mut current: Option<String> = None;

    for raw in bson::documents(&dump.bson) {
        let raw = raw?;
        let wanted = current.as_deref().is_some_and(keep);
        if bson::has_cmd(raw) || wanted {
            let mut doc = bson::decode(raw)?;
            if doc.get("__cmd").and_then(Value::as_str) == Some("select") {
                let c = doc
                    .get("collection")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if keep(&c) {
                    out.push(Value::Object(doc).to_string());
                    kept.push((c.clone(), 0));
                }
                current = Some(c);
                continue;
            }
            if wanted {
                let c = current.as_deref().unwrap_or_default();
                if c == "site" {
                    sites.push(site_name(&doc));
                }
                if c == CLIENTS && clients == Clients::Anonymized && anonymize(&mut doc) {
                    anonymized += 1;
                }
                let mut v = Value::Object(std::mem::take(&mut doc));
                redact(&mut v, c, &mut redacted);
                out.push(v.to_string());
                kept.last_mut().expect("marker precedes rows").1 += 1;
                continue;
            }
        }
        let c = current
            .clone()
            .unwrap_or_else(|| "(before any collection)".into());
        *dropped.entry(c).or_default() += 1;
    }
    if kept.is_empty() {
        return Err("backup holds none of the expected collections (site, device, …)".into());
    }
    Ok(Report {
        layers: dump.layers,
        version: dump.version,
        sites,
        kept,
        dropped,
        redacted,
        clients,
        anonymized,
        json: format!("[\n{}\n]\n", out.join(",\n")),
    })
}

fn site_name(doc: &Map<String, Value>) -> String {
    ["desc", "name"]
        .iter()
        .find_map(|k| doc.get(*k).and_then(Value::as_str))
        .unwrap_or("?")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::tests::encode;
    use container::tests::{gz, unf, zip};
    use serde_json::json;

    fn backup(docs: &[Value]) -> Vec<u8> {
        let stream: Vec<u8> = docs.iter().flat_map(encode).collect();
        unf(zip(
            &[("version", b"9.0.114"), ("db.gz", &gz(&stream))],
            false,
        ))
    }

    fn sample() -> Vec<u8> {
        let sel = |c: &str| json!({"__cmd": "select", "collection": c});
        backup(&[
            sel("admin"),
            json!({"name": "root", "x_shadow": "$6$abc"}),
            sel("site"),
            json!({"_id": "s1", "name": "default", "desc": "Main Office"}),
            sel("device"),
            json!({"name": "SW1", "mac": "aa", "x_authkey": "k", "x_ssh_hostkey": "h",
                   "config_network": {"type": "pppoe", "password": "p"},
                   "port_overrides": [{"port_idx": 1, "portconf_id": "pc1"}]}),
            sel("wlanconf"),
            json!({"name": "Staff", "security": "wpapsk", "x_passphrase": "hunter2",
                   "private_preshared_keys": [{"password": "a"}], "radius_secret": "r"}),
            sel("networkconf"),
            json!({"name": "VPN", "purpose": "vpn-client", "vlan": 50,
                   "openvpn_configuration": "client\n<key>\n-----BEGIN PRIVATE KEY-----\nMIIE\n</key>",
                   "wireguard_client_peer_public_key": "pub"}),
            sel("user"),
            json!({"mac": "bb", "hostname": "Janes-iPhone", "name": "Jane's phone", "oui": "Apple",
                   "note": "Jane Doe, front office", "email": "jane@example.com",
                   "last_uplink_name": "SW1", "noted": true}),
            sel("event"),
            json!({"msg": "x"}),
            json!({"msg": "y"}),
        ])
    }

    #[test]
    fn keeps_config_and_strips_secrets() {
        let r = convert(sample(), Clients::Full).unwrap();
        let docs: Vec<Value> = serde_json::from_str(&r.json).unwrap();
        let text = r.json.as_str();
        for secret in [
            "hunter2",
            "$6$abc",
            "x_authkey",
            "radius_secret",
            "\"p\"",
            "root",
            "PRIVATE KEY",
            "peer_public_key",
        ] {
            assert!(!text.contains(secret), "{secret} leaked");
        }
        assert_eq!(docs[0], json!({"__cmd": "select", "collection": "site"}));
        assert!(text.contains("port_overrides") && text.contains("Janes-iPhone"));
        assert_eq!(r.sites, ["Main Office"]);
        assert_eq!(r.version.as_deref(), Some("9.0.114"));
        assert_eq!((r.count("device"), r.count("user")), (1, 1));
        assert_eq!(r.dropped.get("event"), Some(&2));
        assert_eq!(r.dropped.get("admin"), Some(&1));
        assert_eq!(r.redacted.get("device.config_network.password"), Some(&1));
        assert_eq!(r.redacted.get("wlanconf.private_preshared_keys"), Some(&1));
        assert_eq!(
            r.redacted.get("networkconf.openvpn_configuration"),
            Some(&1)
        );
        assert!(text.contains("\"purpose\":\"vpn-client\""));
        assert!(r.summary().contains("Secret fields removed: 8"));
    }

    #[test]
    fn clients_anonymized() {
        let r = convert(sample(), Clients::Anonymized).unwrap();
        assert!(!r.json.contains("Janes-iPhone"));
        assert!(!r.json.contains("jane@example.com"));
        let docs: Vec<Value> = serde_json::from_str(&r.json).unwrap();
        let user = docs.iter().find(|d| d["mac"] == "bb").unwrap();
        assert!(user["hostname"].as_str().unwrap().starts_with("client-"));
        assert_eq!(user["hostname"], user["name"]);
        assert_eq!(
            (&user["oui"], &user["last_uplink_name"]),
            (&json!("Apple"), &json!("SW1"))
        );
        assert_eq!(r.anonymized, 1);
        assert!(r.summary().contains("anonymized: 1 renamed"));
        // Same MAC, same token, so exports can be compared.
        let again = convert(sample(), Clients::Anonymized).unwrap();
        assert_eq!(again.json, r.json);
    }

    #[test]
    fn clients_optional() {
        let r = convert(sample(), Clients::None).unwrap();
        assert!(!r.json.contains("Janes-iPhone"));
        assert_eq!(r.dropped.get("user"), Some(&1));
        assert!(r.summary().contains("(left out)"));
    }

    #[test]
    fn no_config_is_an_error() {
        let b = backup(&[
            json!({"__cmd": "select", "collection": "event"}),
            json!({"m": 1}),
        ]);
        assert!(convert(b, Clients::Full).is_err());
    }
}
