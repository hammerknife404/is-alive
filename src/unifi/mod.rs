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
use std::collections::{BTreeMap, BTreeSet};

/// Most JSON the kept collections may come to. A big controller's devices,
/// networks and clients are tens of MB; the events and stats that make a
/// backup large are left behind, not kept.
const KEPT_MAX: usize = 256 << 20;

/// Sites, devices, networks, switch port profiles, WLANs.
/// Largest file Config Export will read.
pub const MAX_INPUT: usize = container::MAX_UNPACKED;
pub use container::{zip_all, zip_find};

/// A stored zip of `files`, for other modules' tests.
#[cfg(test)]
pub fn container_test_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    container::tests::zip(files, false)
}

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

    /// From its label (the settings file's `unifi_clients=`).
    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.label() == s)
    }

    pub fn hint(self) -> &'static str {
        match self {
            Clients::None => {
                "Leave the UniFi client table out, and leave phones', laptops' and PCs' names out \
of the sweep inventory and a Meraki export's DHCP reservations (their MAC and address stay)."
            }
            Clients::Anonymized => {
                "Replace device names, which are often people's names, with a token like \
client-1a2b3c4d made from the MAC, so a device reads the same in every file: every name in the \
UniFi client table, and phones', laptops' and PCs' in the sweep inventory and Meraki DHCP \
reservations. MAC, vendor and switch port stay (what the skills need to spot unadopted gear); \
notes and email addresses are dropped."
            }
            Clients::Full => {
                "Keep device names everywhere. Notes, email addresses and phone numbers are still \
dropped: nothing reads them."
            }
        }
    }

    /// What a device's `name` becomes under this setting where a file
    /// mixes clients with gear (the sweep inventory, Meraki DHCP
    /// reservations): a name on anything that doesn't `look like a client`
    /// stays, as do all with Full; otherwise it's the `token` of `key`
    /// (its MAC as `aa:bb:cc:dd:ee:ff`, else its address) or nothing.
    pub fn device_name(self, name: Option<&str>, key: &str, client: bool) -> Option<String> {
        let name = name?;
        match self {
            Clients::Full => Some(name.to_string()),
            _ if !client => Some(name.to_string()),
            Clients::Anonymized => Some(token(key)),
            Clients::None => None,
        }
    }
}

/// Fields of a client record that name a person or hold their details.
const PERSONAL_NAMES: &[&str] = &["name", "hostname", "display_name", "local_dns_record"];

/// Drop a client's notes, email and phone fields in every mode: no skill
/// reads them, and they're where a person's details end up (minimum
/// necessary). Returns whether anything went.
fn drop_details(doc: &mut Map<String, Value>) -> bool {
    let before = doc.len();
    doc.retain(|k, _| {
        let k = k.to_ascii_lowercase();
        k != "note" && !k.contains("email") && !k.contains("phone")
    });
    doc.len() != before
}

/// The name a client goes by when names are anonymized: `client-` and a
/// hash of its MAC as UniFi writes it (`aa:bb:cc:dd:ee:ff`). The MAC is
/// kept anyway, so the token reveals nothing new, and the sweep inventory
/// uses the same one, so a device reads the same in both files.
pub fn token(mac: &str) -> String {
    format!(
        "client-{:08x}",
        mac.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32)
            .wrapping_mul(0x0100_0193))
    )
}

/// Replace a client's names with its `token`. Returns whether anything
/// changed.
fn anonymize(doc: &mut Map<String, Value>) -> bool {
    let mac = doc.get("mac").and_then(Value::as_str).unwrap_or_default();
    let token = token(mac);
    let mut changed = false;
    for k in PERSONAL_NAMES {
        if let Some(v) = doc.get_mut(*k)
            && v.is_string()
        {
            *v = Value::String(token.clone());
            changed = true;
        }
    }
    drop_details(doc) || changed
}

/// Field-name fragments that mark a secret. UniFi prefixes stored secrets
/// with `x_` (x_passphrase, x_authkey, x_wan_password, …); the rest catch
/// RADIUS secrets, PPSK lists, VPN keys and certificates by name, and the
/// account names that go with them (a PPPoE login is the ISP's account
/// ID, half the credential): SonicWall's converter drops them too.
const SECRET: &[&str] = &[
    "username",
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

/// Marks of key material inside a value, matched without case: PEM
/// blocks, an OpenVPN config's inline key blocks, and a WireGuard
/// config's `PrivateKey =` / `PresharedKey =` lines.
const KEY_MARKS: &[&str] = &[
    "-----begin",
    "private key",
    "private_key",
    "privatekey",
    "presharedkey",
    "<key>",
    "<secret>",
    "<tls-auth>",
    "<tls-crypt>",
];

/// Key or certificate material in a value whatever the field is called:
/// an OpenVPN or WireGuard config pasted or uploaded into a network's
/// settings (as text or base64), or a bare 32-byte key in base64 (how
/// WireGuard writes its keys). The decision firmware fields don't get to
/// make by being named something new.
fn holds_secret(v: &Value) -> bool {
    v.as_str().is_some_and(text_holds_secret)
}

fn text_holds_secret(s: &str) -> bool {
    use base64::Engine;
    let marked = |t: &str| {
        let t = t.to_ascii_lowercase();
        KEY_MARKS.iter().any(|m| t.contains(m))
    };
    if marked(s) {
        return true;
    }
    let b64: String = s.split_whitespace().collect();
    if b64.len() < 44
        || !b64
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_'))
    {
        return false;
    }
    let engines = [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::URL_SAFE,
    ];
    let Some(raw) = engines.iter().find_map(|e| e.decode(&b64).ok()) else {
        return false;
    };
    // A bare key: 32 bytes is a WireGuard private or preshared key.
    raw.len() == 32 || std::str::from_utf8(&raw).is_ok_and(marked)
}

/// Drop secret fields at any depth; count each by `collection.path.field`,
/// and keep the dropped values in `secrets` (memory only) so notes can be
/// checked for them.
fn redact(
    v: &mut Value,
    path: &str,
    removed: &mut BTreeMap<String, usize>,
    secrets: &mut BTreeSet<String>,
) {
    match v {
        Value::Object(m) => {
            m.retain(|k, v| {
                let keep = !is_secret(k) && !holds_secret(v);
                if !keep {
                    *removed.entry(format!("{path}.{k}")).or_default() += 1;
                    strings(v, secrets);
                }
                keep
            });
            for (k, child) in m.iter_mut() {
                redact(child, &format!("{path}.{k}"), removed, secrets);
            }
        }
        Value::Array(a) => {
            // A list of strings (certificates, peer configs) is checked
            // item by item, like a field's value.
            a.retain(|c| {
                let keep = !holds_secret(c);
                if !keep {
                    *removed.entry(format!("{path}[]")).or_default() += 1;
                    strings(c, secrets);
                }
                keep
            });
            a.iter_mut().for_each(|c| redact(c, path, removed, secrets));
        }
        _ => {}
    }
}

/// Every string worth checking notes for, at any depth (a PPSK list holds
/// one password per entry).
fn strings(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(s) if crate::redact::secret_value(s) => {
            out.insert(s.trim().to_string());
        }
        Value::Array(a) => a.iter().for_each(|c| strings(c, out)),
        Value::Object(m) => m.values().for_each(|c| strings(c, out)),
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
    /// The secret values that were stripped, held in memory only so notes
    /// can be checked for them. Never written, copied or bundled.
    pub secrets: Vec<String>,
}

impl Report {
    pub fn count(&self, coll: &str) -> usize {
        self.kept.iter().filter(|k| k.0 == coll).map(|k| k.1).sum()
    }

    pub fn summary(&self) -> String {
        let mut s = format!("Format     {}\n", self.layers.join(" ➡ "));
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
    let mut secrets = BTreeSet::new();
    let mut sites = Vec::new();
    let mut current: Option<String> = None;
    let mut kept_bytes = 0usize;

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
                if c == CLIENTS {
                    if clients == Clients::Anonymized && anonymize(&mut doc) {
                        anonymized += 1;
                    }
                    drop_details(&mut doc);
                }
                let mut v = Value::Object(std::mem::take(&mut doc));
                redact(&mut v, c, &mut redacted, &mut secrets);
                let line = v.to_string();
                kept_bytes += line.len();
                if kept_bytes > KEPT_MAX {
                    return Err(format!(
                        "the collections kept come to more than {} MB: far more than a controller's \
devices, networks and clients, so this isn't a backup this reads",
                        KEPT_MAX >> 20
                    ));
                }
                out.push(line);
                // A document before any collection marker (a damaged or
                // hand-made dump) is still kept; it just isn't counted.
                if let Some(k) = kept.last_mut() {
                    k.1 += 1;
                }
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
        secrets: secrets.into_iter().collect(),
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
                   "config_network": {"type": "pppoe", "password": "p", "username": "acct-0042"},
                   "wan_username": "acct-0042",
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

    /// Key material is found by what it is, not only by the field's
    /// name: a WireGuard config as text or base64 under a name no list
    /// knows, lower-case PEM, a bare base64 key, and certificates in a
    /// list. Ordinary long values stay.
    #[test]
    fn finds_keys_whatever_the_field_is_called() {
        use base64::Engine;
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let conf = "[Interface]\nPrivateKey = cGxhY2Vob2xkZXIta2V5LW5vdC1yZWFsLTAwMDAwMDA=\nAddress = 10.8.0.2/32\n";
        let sel = |c: &str| json!({"__cmd": "select", "collection": c});
        let r = convert(
            backup(&[
                sel("site"),
                json!({"_id": "s1", "name": "default"}),
                sel("networkconf"),
                json!({"name": "Branch VPN", "purpose": "wireguard-client",
                       "wireguard_client_configuration_file": conf,
                       "uploaded_profile": b64(conf.as_bytes()),
                       "peer_blob": b64(&[7u8; 32]),
                       "ca_list": ["-----begin certificate-----\nMIIB\n-----end certificate-----", "keep-me"],
                       "description": "Main-Office-Building-Second-Floor-Network-Closet-A",
                       "site_id": "5f0c6b2e8a1d4c0012345678"}),
            ]),
            Clients::None,
        )
        .unwrap();
        for (i, gone) in [
            "PrivateKey",
            "cGxhY2Vob2xkZXIta2V5",
            "uploaded_profile",
            "peer_blob",
            "begin certificate",
            "wireguard_client_configuration_file",
        ]
        .iter()
        .enumerate()
        {
            // By number: a failing test never prints a secret.
            assert!(!r.json.contains(*gone), "fixture secret #{i} leaked");
        }
        for kept in [
            "Branch VPN",
            "keep-me",
            "Network-Closet-A",
            "5f0c6b2e8a1d4c0012345678",
        ] {
            assert!(r.json.contains(kept), "{kept} should stay");
        }
        assert_eq!(r.redacted.get("networkconf.ca_list[]"), Some(&1));
    }

    #[test]
    fn device_names_by_setting() {
        let mac = "00:12:f0:00:00:01";
        for c in Clients::ALL {
            assert_eq!(
                c.device_name(None, mac, true),
                None,
                "no name, nothing to keep"
            );
            assert_eq!(
                c.device_name(Some("SW-1"), mac, false).as_deref(),
                Some("SW-1")
            );
        }
        assert_eq!(
            Clients::Full
                .device_name(Some("Jane"), mac, true)
                .as_deref(),
            Some("Jane")
        );
        assert_eq!(
            Clients::Anonymized.device_name(Some("Jane"), mac, true),
            Some(token(mac))
        );
        assert_eq!(Clients::None.device_name(Some("Jane"), mac, true), None);
        // The UniFi table's token for the same MAC.
        let mut doc = Map::new();
        doc.insert("mac".into(), json!(mac));
        doc.insert("name".into(), json!("Jane"));
        anonymize(&mut doc);
        assert_eq!(doc["name"], json!(token(mac)));
    }

    #[test]
    fn keeps_config_and_strips_secrets() {
        let r = convert(sample(), Clients::Full).unwrap();
        let docs: Vec<Value> = serde_json::from_str(&r.json).unwrap();
        let text = r.json.as_str();
        for (i, secret) in [
            "hunter2",
            "$6$abc",
            "x_authkey",
            "radius_secret",
            "\"p\"",
            "root",
            "PRIVATE KEY",
            "peer_public_key",
            "acct-0042",
        ]
        .iter()
        .enumerate()
        {
            // By number: a failing test never prints a secret.
            assert!(!text.contains(*secret), "fixture secret #{i} leaked");
        }
        assert_eq!(docs[0], json!({"__cmd": "select", "collection": "site"}));
        assert!(text.contains("port_overrides") && text.contains("Janes-iPhone"));
        // Full keeps names, but a client's note and email never go out.
        assert!(!text.contains("front office") && !text.contains("jane@example.com"));
        assert!(text.contains("\"noted\":true"));
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
        assert!(r.summary().contains("Secret fields removed: 10"));
        // What was stripped is kept in memory for Notes, not in the output.
        assert!(
            r.secrets.contains(&"hunter2".to_string()),
            "the admin password isn't held"
        );
        assert!(r.secrets.iter().all(|s| !r.json.contains(s.as_str())));
        assert!(!r.secrets.iter().any(|s| s.starts_with('$')));
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

    /// Zero trust for input files: truncated, bit-flipped, random and
    /// deeply nested data is an error, never a panic or a hang.
    #[test]
    fn hostile_input_never_panics() {
        let good = sample();
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for n in (0..good.len()).step_by(29) {
            let _ = convert(good[..n].to_vec(), Clients::Full);
        }
        for _ in 0..300 {
            let mut d = good.clone();
            for _ in 0..3 {
                let i = rnd() as usize % d.len();
                d[i] ^= (rnd() as u8) | 1;
            }
            let _ = convert(d, Clients::Anonymized);
        }
        for len in [0, 1, 16, 17, 512, 4096] {
            let d: Vec<u8> = (0..len).map(|_| rnd() as u8).collect();
            assert!(convert(d, Clients::None).is_err());
        }
        // A BSON stream mangled after decryption.
        let stream: Vec<u8> = [
            json!({"__cmd": "select", "collection": "device"}),
            json!({"a": {"b": [1, "x"]}}),
        ]
        .iter()
        .flat_map(encode)
        .collect();
        for _ in 0..500 {
            let mut d = stream.clone();
            let i = rnd() as usize % d.len();
            d[i] = rnd() as u8;
            for doc in bson::documents(&d).take(8).flatten() {
                let _ = bson::decode(doc);
            }
        }
        // Deeper than any controller nests: refused, not recursed into.
        let mut deep = json!(1);
        for _ in 0..200 {
            deep = json!({ "a": deep });
        }
        let bytes = encode(&deep);
        assert!(bson::decode(&bytes).unwrap_err().contains("too deep"));
    }
}
