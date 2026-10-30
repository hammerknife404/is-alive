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
    pub clients: bool,
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
        for c in CONFIG.iter().chain(self.clients.then_some(&CLIENTS)) {
            s += &format!("  {c:<14}{:>7}\n", self.count(c));
        }
        if !self.clients {
            s += &format!("  {CLIENTS:<14}{:>7}  (left out)\n", 0);
        }
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
pub fn convert(data: Vec<u8>, clients: bool) -> Result<Report, String> {
    let dump = container::open(data)?;
    let keep = |c: &str| CONFIG.contains(&c) || (clients && c == CLIENTS);

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
            json!({"mac": "bb", "hostname": "Janes-iPhone", "oui": "Apple"}),
            sel("event"),
            json!({"msg": "x"}),
            json!({"msg": "y"}),
        ])
    }

    #[test]
    fn keeps_config_and_strips_secrets() {
        let r = convert(sample(), true).unwrap();
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
    fn clients_optional() {
        let r = convert(sample(), false).unwrap();
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
        assert!(convert(b, true).is_err());
    }
}
