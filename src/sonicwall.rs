//! SonicWall settings export (.exp) or tech support report (.wri, see
//! `tsr`) → sanitized JSON for /mapnetwork, offline.
//!
//! An .exp is base64 of `key=value&key=value…` with URL-encoded values;
//! tables are flattened as `<field>_<index>` (`iface_name_3`, `policySrcZone_12`).
//! Output is built only from the fields named here, so admin and user
//! accounts, VPN and RADIUS secrets, SNMP communities and certificates are
//! never copied. Comments are kept (they say what a rule or port is for) unless
//! they look like they hold a credential.

mod tsr;

use serde_json::{Map, Value, json};
use std::cell::Cell;
use std::collections::BTreeMap;

const EXP: &str = "SonicWall settings export (.exp)";
const WRI: &str = "SonicWall tech support report (.wri)";

pub struct Report {
    /// Which of the two files it was.
    pub format: &'static str,
    pub keys: usize,
    /// Comments replaced because they look like they hold a credential.
    pub withheld: usize,
    pub used: usize,
    /// (table, rows) in output order.
    pub counts: Vec<(&'static str, usize)>,
    pub system: Vec<(String, String)>,
    pub json: String,
}

impl Report {
    pub fn count(&self, table: &str) -> usize {
        self.counts.iter().find(|c| c.0 == table).map_or(0, |c| c.1)
    }

    pub fn summary(&self) -> String {
        let mut s = format!("Format     {}\n", self.format);
        for (k, v) in &self.system {
            let k = match k.to_ascii_lowercase() {
                l if l.contains("serial") => "Serial",
                l if l.contains("firmware") => "Firmware",
                l if l.contains("romversion") => "ROM",
                l if l.contains("model") || l.contains("prodname") => "Model",
                l if l.contains("firewallname") => "Name",
                _ => k.as_str(),
            };
            s += &format!("{k:<10} {v}\n");
        }
        s += &format!(
            "Output     {:.1} KB, JSON for /mapnetwork\n\nKept\n",
            self.json.len() as f64 / 1024.0
        );
        for (t, n) in &self.counts {
            s += &format!("  {t:<18}{n:>6}\n");
        }
        if self.format == WRI {
            s += "\nRead from the report: system status (model, serial, firmware), interfaces,\n\
zones, address objects and groups, NAT policies and access rules, field by field.\n\
Users, VPN, RADIUS/LDAP, SNMP, certificates, licences, registration codes, logs\n\
and diagnostics are skipped. Comments are kept unless they mention a password,\n\
key or similar.\n";
        } else {
            s += &format!(
                "\nSettings in the export: {}; {} copied, {} left behind.\n\
Only the fields listed above are read: accounts, passwords, VPN and RADIUS secrets,\n\
SNMP communities and certificates never leave the file. Comments are kept\n\
unless they mention a password, key or similar.\n",
                self.keys,
                self.used,
                self.keys - self.used
            );
        }
        if self.withheld > 0 {
            s += &format!(
                "{} comment(s) withheld or redacted: a password, key, email, phone or similar.\n",
                self.withheld
            );
        }
        s
    }
}

/// True when `data` is an .exp or a tech support report.
pub fn sniff(data: &[u8]) -> bool {
    decode(data).is_some() || tsr::sniff(data)
}

/// An .exp's key=value text (base64 or already decoded).
fn decode(data: &[u8]) -> Option<String> {
    let trimmed = data.trim_ascii();
    let text = if trimmed
        .iter()
        .all(|&b| is_b64(b) || b.is_ascii_whitespace())
    {
        String::from_utf8(base64(trimmed)?).ok()?
    } else {
        String::from_utf8(trimmed.to_vec()).ok()?
    };
    let pairs = text
        .split(['&', '\n'])
        .take(20)
        .filter(|p| {
            p.split_once('=').is_some_and(|(k, _)| {
                !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
        })
        .count();
    (pairs >= 10).then_some(text)
}

fn is_b64(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')
}

fn base64(s: &[u8]) -> Option<Vec<u8>> {
    let val = |b: u8| match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &b in s.iter().filter(|b| !b.is_ascii_whitespace()) {
        if b == b'=' {
            break;
        }
        acc = acc << 6 | val(b)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Settings, plus which keys the output used.
struct Settings {
    map: BTreeMap<String, String>,
    used: std::cell::RefCell<std::collections::HashSet<String>>,
    /// Comments not copied because they look like they hold a credential.
    withheld: std::cell::Cell<usize>,
}

impl Settings {
    fn parse(text: &str) -> Self {
        let map = text
            .split(['&', '\n', '\r'])
            .filter_map(|p| p.split_once('='))
            .filter(|(k, _)| !k.is_empty())
            .map(|(k, v)| (k.to_string(), unescape(v)))
            .collect();
        Self {
            map,
            used: Default::default(),
            withheld: Default::default(),
        }
    }

    /// Non-empty value of `key`.
    fn get(&self, key: &str) -> Option<&str> {
        let v = self.map.get(key)?;
        self.used.borrow_mut().insert(key.to_string());
        Some(v.as_str()).filter(|v| !v.is_empty())
    }

    fn at(&self, stem: &str, i: u32) -> Option<&str> {
        self.get(&format!("{stem}_{i}"))
    }

    /// A free-text comment, withheld if it looks like it holds a credential,
    /// and otherwise put through the same redaction as tech notes (emails,
    /// phone numbers, tokens…).
    fn comment(&self, stem: &str, i: u32) -> Option<Value> {
        Some(clean_comment(self.at(stem, i)?, &self.withheld))
    }

    /// Indices present for `stem` (keys `stem_<n>`), ascending.
    fn indices(&self, stem: &str) -> Vec<u32> {
        let p = format!("{stem}_");
        let mut v: Vec<u32> = self
            .map
            .range(p.clone()..)
            .take_while(|(k, _)| k.starts_with(&p))
            .filter_map(|(k, _)| k[p.len()..].parse().ok())
            .collect();
        v.sort_unstable();
        v
    }
}

/// Words that suggest someone typed a credential into a free-text field.
const CREDENTIAL: &[&str] = &[
    "pass",
    "pwd",
    "psk",
    "secret",
    "community",
    "token",
    "apikey",
    "api key",
];

/// A free-text comment, withheld if it looks like it holds a credential,
/// and otherwise put through the same redaction as tech notes; `withheld`
/// counts either.
fn clean_comment(v: &str, withheld: &Cell<usize>) -> Value {
    if credential_like(v) {
        withheld.set(withheld.get() + 1);
        return json!("[withheld: may contain a credential]");
    }
    let (clean, hits) = crate::redact::sanitize(v);
    if !hits.is_empty() {
        withheld.set(withheld.get() + 1);
    }
    json!(clean)
}

fn credential_like(v: &str) -> bool {
    let mut v = v.to_ascii_lowercase();
    for benign in ["bypass", "passthr", "pass-thr", "pass thr", "passive"] {
        v = v.replace(benign, "");
    }
    CREDENTIAL.iter().any(|w| v.contains(w))
}

/// An address that means "nothing set".
fn ip(v: Option<&str>) -> Option<&str> {
    v.filter(|v| *v != "0.0.0.0")
}

/// "C0EAE4E44748" → "c0:ea:e4:e4:47:48".
fn mac(v: &str) -> String {
    let h: String = v.chars().filter(char::is_ascii_hexdigit).collect();
    if h.len() != 12 {
        return v.to_string();
    }
    (0..6)
        .map(|i| h[i * 2..i * 2 + 2].to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(":")
}

fn obj(pairs: Vec<(&str, Option<Value>)>) -> Value {
    Value::Object(
        pairs
            .into_iter()
            .filter_map(|(k, v)| Some((k.to_string(), v?)))
            .collect::<Map<_, _>>(),
    )
}

fn s(v: Option<&str>) -> Option<Value> {
    v.map(|v| Value::String(v.to_string()))
}

fn interfaces(c: &Settings) -> Vec<Value> {
    c.indices("iface_name")
        .into_iter()
        .filter_map(|i| {
            let name = c.at("iface_name", i)?;
            let kind = c.at("iface_phys_type", i).map(|t| match t {
                "0" => "physical".to_string(),
                "2" => "vlan".to_string(),
                t => format!("type {t}"),
            });
            let vlan = c.at("iface_vlan_tag", i).filter(|t| *t != "0");
            let parent = c
                .at("iface_vlan_parent", i)
                .and_then(|p| p.parse::<u32>().ok())
                .and_then(|p| c.at("iface_name", p))
                .filter(|_| vlan.is_some());
            let (ip_addr, mask) = [("lan", "lan"), ("static", "static"), ("dhcp", "dhcp")]
                .iter()
                .find_map(|(a, m)| {
                    let a = ip(c.at(&format!("iface_{a}_ip"), i))?;
                    Some((a, c.at(&format!("iface_{m}_mask"), i)))
                })
                .unzip();
            let gateway = [
                "iface_static_gateway",
                "iface_dhcp_gateway",
                "iface_lan_default_gw",
            ]
            .iter()
            .find_map(|k| ip(c.at(k, i)));
            let mgmt: Vec<Value> = ["http", "https", "ssh", "ping", "snmp"]
                .iter()
                .filter(|p| c.at(&format!("iface_{p}_mgmt"), i) == Some("1"))
                .map(|p| json!(p))
                .collect();
            let secondary = ip(c.at("secSubnetIp", i))
                .map(|a| json!(format!("{a}/{}", c.at("secSubnetMask", i).unwrap_or("?"))));
            Some(obj(vec![
                ("name", s(Some(name))),
                ("zone", s(c.at("interface_Zone", i))),
                ("type", kind.map(Value::String)),
                ("vlan", s(vlan)),
                ("parent", s(parent)),
                ("ip", s(ip_addr)),
                ("mask", s(mask.flatten())),
                ("gateway", s(gateway)),
                ("secondary_subnet", secondary),
                ("mac", c.at("eth_mac", i).map(|m| json!(mac(m)))),
                (
                    "management",
                    (!mgmt.is_empty()).then_some(Value::Array(mgmt)),
                ),
                (
                    "disabled",
                    (c.at("iface_port_disabled", i) == Some("1")).then_some(json!(true)),
                ),
                ("comment", c.comment("iface_comment", i)),
            ]))
        })
        .collect()
}

/// Rows of a table: `fields` are (output name, key stem); the first is the
/// row's identity and must be set.
fn table(c: &Settings, fields: &[(&str, &str)]) -> Vec<Value> {
    c.indices(fields[0].1)
        .into_iter()
        .filter_map(|i| {
            c.at(fields[0].1, i)?;
            Some(obj(fields
                .iter()
                .map(|(name, stem)| match *name {
                    "comment" => (*name, c.comment(stem, i)),
                    _ => (*name, s(c.at(stem, i))),
                })
                .collect()))
        })
        .collect()
}

/// Group → members from `<p>_atomToGrp_N` / `<p>_grpToGrp_N` pairs.
fn groups(c: &Settings, p: &str) -> Value {
    let (member, group) = (format!("{p}_atomToGrp"), format!("{p}_grpToGrp"));
    let mut g: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for i in c.indices(&member) {
        if let (Some(m), Some(grp)) = (c.at(&member, i), c.at(&group, i)) {
            g.entry(grp.to_string()).or_default().push(json!(m));
        }
    }
    json!(g)
}

fn address_objects(c: &Settings) -> Vec<Value> {
    c.indices("addrObjId")
        .into_iter()
        .filter_map(|i| {
            let name = c.at("addrObjId", i)?;
            let t = c.at("addrObjType", i);
            let kind = t.map(|t| match t {
                "1" => "host".to_string(),
                "2" => "range".to_string(),
                "4" => "network".to_string(),
                "8" => "group".to_string(),
                t => format!("type {t}"),
            });
            let group = t == Some("8");
            Some(obj(vec![
                ("name", s(Some(name))),
                ("type", kind.map(Value::String)),
                ("zone", s(c.at("addrObjZone", i))),
                ("ip1", s(ip(c.at("addrObjIp1", i)).filter(|_| !group))),
                ("ip2", s(ip(c.at("addrObjIp2", i)).filter(|_| !group))),
            ]))
        })
        .collect()
}

/// Scalar system facts, whatever this firmware calls them.
fn system(c: &Settings) -> Vec<(String, String)> {
    const WANT: &[&str] = &[
        "model",
        "firmware",
        "prodname",
        "romversion",
        "firewallname",
        // Serial is the strongest key for matching documentation (IT Glue).
        "serial",
    ];
    c.map
        .iter()
        .filter(|(k, _)| {
            let k = k.to_ascii_lowercase();
            !k.contains('_')
                && WANT.iter().any(|w| k.contains(w))
                && !credential_like(&k)
                && !k.contains("key")
                && !k.contains("user")
        })
        .filter_map(|(k, _)| Some((k.clone(), c.get(k)?.to_string())))
        .collect()
}

pub fn convert(data: &[u8]) -> Result<Report, String> {
    let Some(text) = decode(data) else {
        return if tsr::sniff(data) {
            tsr::convert(data)
        } else {
            Err("not a SonicWall settings export (.exp) or tech support report (.wri)".into())
        };
    };
    let format = EXP;
    let c = Settings::parse(&text);

    let ifaces = interfaces(&c);
    if ifaces.is_empty() {
        return Err("export has no interfaces (iface_name_N): is this a SonicWall .exp?".into());
    }
    let zones = table(
        &c,
        &[
            ("name", "zoneObjId"),
            ("security_type", "zoneObjSecType"),
            ("comment", "zoneObjComment"),
        ],
    );
    let addresses = address_objects(&c);
    let fqdn = table(
        &c,
        &[
            ("name", "addrObjFqdnId"),
            ("fqdn", "addrObjFqdn"),
            ("zone", "addrObjFqdnZone"),
        ],
    );
    let address_groups = groups(&c, "addro");
    let services = table(
        &c,
        &[
            ("name", "svcObjId"),
            ("type", "svcObjType"),
            ("ip_type", "svcObjIpType"),
            ("port1", "svcObjPort1"),
            ("port2", "svcObjPort2"),
        ],
    );
    let service_groups = groups(&c, "so");
    let rules = table(
        &c,
        &[
            ("action", "policyAction"),
            ("src_zone", "policySrcZone"),
            ("dst_zone", "policyDstZone"),
            ("src", "policySrcNet"),
            ("dst", "policyDstNet"),
            ("src_service", "policySrcSvc"),
            ("service", "policyDstSvc"),
            ("enabled", "policyEnabled"),
            ("comment", "policyComment"),
        ],
    );
    let nat = table(
        &c,
        &[
            ("enabled", "natPolicyEnabled"),
            ("orig_src", "natPolicyOrigSrc"),
            ("orig_dst", "natPolicyOrigDst"),
            ("orig_service", "natPolicyOrigSvc"),
            ("trans_src", "natPolicyTransSrc"),
            ("trans_dst", "natPolicyTransDst"),
            ("trans_service", "natPolicyTransSvc"),
            ("in_iface", "natPolicySrcIface"),
            ("out_iface", "natPolicyDstIface"),
            ("reflexive", "natPolicyReflexive"),
            ("comment", "natPolicyComment"),
        ],
    );
    let system = system(&c);

    let counts = vec![
        ("interfaces", ifaces.len()),
        ("zones", zones.len()),
        ("address objects", addresses.len() + fqdn.len()),
        (
            "address groups",
            address_groups.as_object().map_or(0, Map::len),
        ),
        ("service objects", services.len()),
        (
            "service groups",
            service_groups.as_object().map_or(0, Map::len),
        ),
        ("access rules", rules.len()),
        ("NAT rules", nat.len()),
    ];
    let v = json!({
        "source": format!("is-alive {} {format}", crate::VERSION),
        "system": system.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>(),
        "notes": [
            "Read from the firewall's own config, so this is enforced state, not observation.",
            "Interface mac is the port's own address: match it against /unifidump's discovered_devices and uplink MACs.",
            "A rule or NAT field that is missing, or a NAT interface of -1, means Any. Codes are SonicOS's own: rule action 2 = allow, 0 = deny, 1 = discard (verify on the firewall).",
            "Accounts, passwords, VPN/RADIUS secrets, SNMP communities and certificates were not copied.",
        ],
        "interfaces": ifaces,
        "zones": zones,
        "address_objects": addresses,
        "fqdn_objects": fqdn,
        "address_groups": address_groups,
        "service_objects": services,
        "service_groups": service_groups,
        "access_rules": rules,
        "nat_rules": nat,
    });
    let used = c.used.borrow().len();
    Ok(Report {
        format,
        keys: c.map.len(),
        withheld: c.withheld.get(),
        used,
        counts,
        system,
        json: serde_json::to_string_pretty(&v).unwrap_or_default() + "\n",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(data: &[u8]) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in data.chunks(3) {
            let n = c.iter().fold(0u32, |a, &b| a << 8 | b as u32) << (8 * (3 - c.len()));
            for i in 0..=c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            }
            out.extend(std::iter::repeat_n('=', 3 - c.len()));
        }
        out
    }

    const SAMPLE: &str = "iface_ifnum_0=0&iface_name_0=X0&iface_phys_type_0=0&interface_Zone_0=LAN&\
iface_comment_0=Default%20LAN&iface_lan_ip_0=172.16.0.1&iface_lan_mask_0=255.255.255.0&\
iface_vlan_tag_0=0&iface_vlan_parent_0=-1&iface_https_mgmt_0=1&iface_ping_mgmt_0=1&eth_mac_0=C0EAE4E44748&\
iface_name_1=X1&iface_phys_type_1=0&interface_Zone_1=WAN&iface_static_ip_1=203.0.113.2&\
iface_static_mask_1=255.255.255.248&iface_static_gateway_1=203.0.113.1&iface_lan_ip_1=0.0.0.0&\
iface_pppoe_username_1=isp-account&\
iface_name_2=X0%3aV40&iface_phys_type_2=2&interface_Zone_2=Voice&iface_lan_ip_2=10.40.0.1&\
iface_lan_mask_2=255.255.255.0&iface_vlan_tag_2=40&iface_vlan_parent_2=0&\
addrObjId_1=Server&addrObjType_1=1&addrObjZone_1=LAN&addrObjIp1_1=172.16.0.10&addrObjIp2_1=255.255.255.255&\
addrObjId_2=Servers&addrObjType_2=8&addrObjZone_2=LAN&addrObjIp1_2=0.0.0.0&addrObjIp2_2=0.0.0.0&\
addro_atomToGrp_1=Server&addro_grpToGrp_1=Servers&\
policyAction_0=2&policySrcZone_0=LAN&policyDstZone_0=WAN&policySrcNet_0=&policyDstNet_0=Servers&\
policyDstSvc_0=HTTPS&policyEnabled_0=1&policyComment_0=web%20out&\
policyAction_1=2&policySrcZone_1=WAN&policyDstZone_1=LAN&policyComment_1=vpn%20pass%20is%20Hunter2&\
firmwareUpdatePassword=FwPass&\
natPolicyOrigSrc_0=&natPolicyTransSrc_0=X1%20IP&natPolicyEnabled_0=1&\
userObjName_1=jsmith&userObjPass_1=S3cretPass&ipsecSharedSecret_1=TunnelPSK&\
snmpCommunity=private&adminName=admin&adminPassword=AdminPass&shortProdName=TZ%20470&\
firmwareVersion=SonicOS%207.0.1&serialNumber=2CB8EDDA6E30&&";

    #[test]
    fn decodes_base64_and_plain() {
        let enc = b64(SAMPLE.as_bytes());
        assert_eq!(decode(enc.as_bytes()).as_deref(), Some(SAMPLE));
        let wrapped: String = enc
            .as_bytes()
            .chunks(76)
            .map(|c| String::from_utf8_lossy(c) + "\r\n")
            .collect();
        assert_eq!(decode(wrapped.as_bytes()).as_deref(), Some(SAMPLE));
        assert!(decode(SAMPLE.replace('&', "\n").as_bytes()).is_some());
        assert!(decode(b"\x8b\x1f\x00binary").is_none());
        assert!(decode(b"hello world").is_none());
    }

    #[test]
    fn builds_tables_without_secrets() {
        let r = convert(b64(SAMPLE.as_bytes()).as_bytes()).unwrap();
        for secret in [
            "S3cretPass",
            "TunnelPSK",
            "AdminPass",
            "isp-account",
            "jsmith",
            "\"private\"",
            "Hunter2",
            "FwPass",
        ] {
            assert!(!r.json.contains(secret), "{secret} leaked");
        }
        let v: Value = serde_json::from_str(&r.json).unwrap();
        let x0 = &v["interfaces"][0];
        assert_eq!(x0["mac"], "c0:ea:e4:e4:47:48");
        assert_eq!(x0["ip"], "172.16.0.1");
        assert_eq!(x0["management"], json!(["https", "ping"]));
        assert_eq!(x0["comment"], "Default LAN");
        let x1 = &v["interfaces"][1];
        assert_eq!(
            (&x1["ip"], &x1["gateway"]),
            (&json!("203.0.113.2"), &json!("203.0.113.1"))
        );
        let v40 = &v["interfaces"][2];
        assert_eq!(
            (&v40["name"], &v40["vlan"], &v40["parent"]),
            (&json!("X0:V40"), &json!("40"), &json!("X0"))
        );
        assert_eq!(v["address_objects"][1]["type"], "group");
        assert!(v["address_objects"][1].get("ip1").is_none());
        assert_eq!(v["address_groups"]["Servers"], json!(["Server"]));
        let rule = &v["access_rules"][0];
        assert_eq!(rule["action"], "2");
        assert!(rule.get("src").is_none());
        assert_eq!(rule["dst"], "Servers");
        assert_eq!(v["nat_rules"][0]["trans_src"], "X1 IP");
        assert_eq!(v["system"]["shortProdName"], "TZ 470");
        assert_eq!(v["system"]["serialNumber"], "2CB8EDDA6E30");
        assert_eq!((r.count("interfaces"), r.count("access rules")), (3, 2));
        assert_eq!(v["access_rules"][0]["comment"], "web out");
        assert_eq!(r.withheld, 1);
        assert!(r.summary().contains("1 comment(s) withheld"));
        assert!(!credential_like("Passthrough to DMZ, passive FTP"));
        assert!(credential_like("wifi PSK: abc"));
        assert!(r.used < r.keys);
        assert!(r.summary().contains("left behind"));
    }

    #[test]
    fn rejects_non_exports() {
        assert!(convert(b"a=1&b=2").is_err());
        let no_ifaces: String = (0..12).map(|i| format!("k{i}=v&")).collect();
        assert!(convert(no_ifaces.as_bytes()).is_err());
    }

    #[test]
    fn hostile_input_never_panics() {
        let good = b64(SAMPLE.as_bytes()).into_bytes();
        let mut seed = 0xD1B5_4A32_D192_ED03_u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for n in (0..good.len()).step_by(7) {
            let _ = convert(&good[..n]);
            let _ = convert(&SAMPLE.as_bytes()[..n.min(SAMPLE.len())]);
        }
        for _ in 0..300 {
            let mut d = SAMPLE.as_bytes().to_vec();
            for _ in 0..4 {
                let i = rnd() as usize % d.len();
                let palette = b"%=&_0123456789aZ\xff";
                d[i] = palette[rnd() as usize % palette.len()];
            }
            let _ = convert(&d);
        }
    }
}
