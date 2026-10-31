//! Cisco Meraki MX → sanitized JSON for /mapnetwork, offline.
//!
//! Meraki keeps no config file on the box: its config is what the Dashboard
//! API returns. So this reads that JSON, as tools/meraki-export.ps1 saves it
//! (one file) or as a backup script saves it (a zip, one file per call).
//! Tables are found by their shape, not their file names: an array whose
//! objects have `applianceIp` and `subnet` is VLANs wherever it sits.
//! Output is built only from the fields named here, so pre-shared keys,
//! VPN, RADIUS and SNMP secrets and PPPoE passwords are never copied.

use crate::sonicwall::clean_comment;
use serde_json::{Map, Value, json};
use std::cell::Cell;

const FORMAT: &str = "Meraki Dashboard API JSON";

/// Most files read from one zip.
const MAX_FILES: usize = 2000;

pub struct Report {
    pub files: usize,
    /// Comments and names replaced because they look like they hold a credential.
    pub withheld: usize,
    /// (table, rows) in output order.
    pub counts: Vec<(&'static str, usize)>,
    pub system: Vec<(String, String)>,
    pub json: String,
    /// Secret values left behind (pre-shared keys, VPN and RADIUS secrets,
    /// SNMP communities, PPPoE passwords), held in memory only so notes can
    /// be checked for them. Never written, copied or bundled.
    pub secrets: Vec<String>,
}

impl Report {
    pub fn count(&self, table: &str) -> usize {
        self.counts.iter().find(|c| c.0 == table).map_or(0, |c| c.1)
    }

    pub fn summary(&self) -> String {
        let mut s = format!("Format     {FORMAT}\n");
        for (k, v) in &self.system {
            s += &format!("{k:<10} {v}\n");
        }
        s += &format!(
            "Output     {:.1} KB, JSON for /mapnetwork\n\nKept\n",
            self.json.len() as f64 / 1024.0
        );
        for (t, n) in &self.counts {
            s += &format!("  {t:<18}{n:>6}\n");
        }
        s += &format!(
            "\nRead from {} file(s) by shape: organization, networks, devices, VLANs,\n\
appliance ports, firewall, L7, port forwarding and NAT rules, static routes,\n\
VPN, uplinks and appliance SSIDs. Pre-shared keys, VPN, RADIUS and SNMP\n\
secrets and PPPoE passwords never leave the file. Rule comments are kept\n\
unless they mention a password, key or similar.\n",
            self.files
        );
        if self.withheld > 0 {
            s += &format!(
                "{} comment(s) withheld or redacted: a password, key, email, phone or similar.\n",
                self.withheld
            );
        }
        s
    }
}

/// The JSON documents in `data`: the file itself, or each .json in a zip,
/// with the folder it sat in (often the network's name).
fn documents(data: &[u8]) -> Vec<(Option<String>, String, Value)> {
    // Windows PowerShell 5.1 writes UTF-8 with a BOM.
    let bom = |b: &[u8]| b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(b).to_vec();
    let parse = |b: &[u8]| serde_json::from_slice::<Value>(&bom(b)).ok();
    let data = &bom(data)[..];
    let first = data.iter().find(|b| !b.is_ascii_whitespace());
    if matches!(first, Some(b'{' | b'[')) {
        return parse(data)
            .map(|v| vec![(None, String::new(), v)])
            .unwrap_or_default();
    }
    crate::unifi::zip_all(
        data,
        |n| n.to_ascii_lowercase().ends_with(".json"),
        MAX_FILES,
    )
    .unwrap_or_default()
    .into_iter()
    .filter_map(|(name, b)| {
        let mut parts: Vec<&str> = name.split('/').filter(|p| !p.is_empty()).collect();
        let file = parts.pop()?;
        let stem = file.rsplit_once('.').map_or(file, |s| s.0).to_string();
        Some((parts.last().map(|d| d.to_string()), stem, parse(&b)?))
    })
    .collect()
}

/// True when `data` is Meraki API JSON (or a zip of it) with MX config in it.
pub fn sniff(data: &[u8]) -> bool {
    let first = data
        .strip_prefix(b"\xEF\xBB\xBF")
        .unwrap_or(data)
        .iter()
        .find(|b| !b.is_ascii_whitespace());
    if !matches!(first, Some(b'{' | b'[' | b'P')) {
        return false;
    }
    let w = read(&documents(data));
    w.found()
}

pub fn convert(data: &[u8]) -> Result<Report, String> {
    let docs = documents(data);
    if docs.is_empty() {
        return Err("no JSON found: is this a Meraki Dashboard API export?".into());
    }
    let mut w = read(&docs);
    if !w.found() {
        return Err(
            "no Meraki MX config found (VLANs, firewall rules, ports, uplinks or MX devices)"
                .into(),
        );
    }
    w.name_networks();

    let mut secrets = Vec::new();
    for (_, _, v) in &docs {
        secrets_in(v, &mut secrets);
    }
    secrets.sort();
    secrets.dedup();

    let mut system: Vec<(String, String)> = Vec::new();
    if let Some(o) = &w.org {
        system.push(("Org".into(), o.clone()));
    }
    let mx: Vec<&Value> = w.tables[DEVICES]
        .iter()
        .filter(|d| is_mx(s(d, "model")))
        .collect();
    for d in &mx {
        let mut line = format!("{} {}", s(d, "model"), s(d, "serial"));
        if !s(d, "firmware").is_empty() {
            line += &format!(", {}", s(d, "firmware"));
        }
        if !s(d, "name").is_empty() {
            line += &format!(" ({})", s(d, "name"));
        }
        system.push(("Appliance".into(), line));
    }
    if !w.networks.is_empty() {
        system.push(("Networks".into(), w.networks.len().to_string()));
    }

    let mut counts = vec![("networks", w.networks.len())];
    counts.extend(
        TABLES
            .iter()
            .map(|(_, label)| *label)
            .zip(w.tables.iter().map(Vec::len)),
    );
    let mut out = Map::new();
    out.insert(
        "source".into(),
        json!(format!("is-alive {} {FORMAT}", crate::VERSION)),
    );
    out.insert(
        "system".into(),
        Value::Object(system.iter().map(|(k, v)| (k.clone(), json!(v))).collect()),
    );
    out.insert("notes".into(), json!([
        "Read from the Meraki Dashboard's config (the API), so this is enforced state, not observation.",
        "A device's mac is its own address: match it against /unifidump's discovered_devices and uplink MACs.",
        "Firewall rules are in order; Meraki ends each set with an allow-any default rule. direction: outbound = the MX's L3 rules, inbound = inbound rules, cellular = cellular failover rules.",
        "Pre-shared keys, VPN, RADIUS and SNMP secrets and PPPoE passwords were not copied.",
    ]));
    out.insert("networks".into(), Value::Array(w.networks.clone()));
    for ((key, _), rows) in TABLES.iter().zip(w.tables.iter_mut()) {
        out.insert((*key).into(), Value::Array(std::mem::take(rows)));
    }
    Ok(Report {
        files: docs.len(),
        withheld: w.withheld.get(),
        counts,
        system,
        json: serde_json::to_string_pretty(&Value::Object(out)).unwrap_or_default() + "\n",
        secrets,
    })
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// Security appliances: MX, Z (teleworker) and vMX.
fn is_mx(model: &str) -> bool {
    let m = model.to_ascii_uppercase();
    m.starts_with("MX") || m.starts_with("Z") || m.starts_with("VMX")
}

// Output tables, in order: (JSON key, label in the summary).
const DEVICES: usize = 0;
const VLANS: usize = 1;
const PORTS: usize = 2;
const RULES: usize = 3;
const L7: usize = 4;
const FORWARDS: usize = 5;
const ONE_TO_ONE: usize = 6;
const ONE_TO_MANY: usize = 7;
const ROUTES: usize = 8;
const S2S: usize = 9;
const PEERS: usize = 10;
const UPLINKS: usize = 11;
const UPLINK_STATUS: usize = 12;
const SSIDS: usize = 13;
const TABLES: [(&str, &str); 14] = [
    ("devices", "devices"),
    ("vlans", "VLANs"),
    ("appliance_ports", "appliance ports"),
    ("firewall_rules", "firewall rules"),
    ("l7_rules", "L7 rules"),
    ("port_forwards", "port forwards"),
    ("one_to_one_nat", "1:1 NAT"),
    ("one_to_many_nat", "1:many NAT"),
    ("static_routes", "static routes"),
    ("site_to_site_vpn", "site-to-site VPN"),
    ("vpn_peers", "VPN peers"),
    ("uplinks", "uplinks"),
    ("uplink_status", "uplink status"),
    ("appliance_ssids", "appliance SSIDs"),
];

#[derive(Default)]
struct Walk {
    org: Option<String>,
    networks: Vec<Value>,
    tables: [Vec<Value>; 14],
    withheld: Cell<usize>,
}

fn read(docs: &[(Option<String>, String, Value)]) -> Walk {
    let mut w = Walk::default();
    for (dir, stem, v) in docs {
        w.walk(v, stem, "", dir.as_deref(), 0);
    }
    w
}

impl Walk {
    /// Anything Meraki-specific: the device list alone could be anyone's.
    fn found(&self) -> bool {
        self.tables
            .iter()
            .enumerate()
            .any(|(i, t)| i != DEVICES && !t.is_empty())
            || self.tables[DEVICES].iter().any(|d| is_mx(s(d, "model")))
    }

    fn walk(&mut self, v: &Value, key: &str, parent: &str, net: Option<&str>, depth: u8) {
        if depth > 32 {
            return;
        }
        match v {
            Value::Array(a) => {
                if let Some(kind) = a.iter().find_map(item_kind) {
                    for x in a {
                        if item_kind(x) == Some(kind) {
                            self.item(kind, x, key, parent, net);
                        }
                    }
                } else {
                    for x in a {
                        self.walk(x, key, parent, net, depth + 1);
                    }
                }
            }
            Value::Object(o) => {
                if is_network(o) {
                    self.networks.push(Value::Object(pick(
                        o,
                        &["id", "name", "productTypes", "timeZone", "tags"],
                    )));
                    let name = s(v, "name").to_string();
                    for (k, c) in o {
                        self.walk(c, k, key, Some(&name), depth + 1);
                    }
                    return;
                }
                if is_org(o) {
                    self.org = Some(s(v, "name").to_string());
                    return;
                }
                if let Some(kind) = item_kind(v) {
                    // A lone row: a one-item list unrolled on the way out.
                    self.item(kind, v, key, parent, net);
                    return;
                }
                if self.object(o, net) {
                    return;
                }
                for (k, c) in o {
                    self.walk(c, k, key, net, depth + 1);
                }
            }
            _ => {}
        }
    }

    /// Objects that are a whole table's worth: uplink settings, site-to-site
    /// VPN, a single LAN.
    fn object(&mut self, o: &Map<String, Value>, net: Option<&str>) -> bool {
        let v = Value::Object(o.clone());
        if let Some(Value::Object(ifaces)) = o.get("interfaces")
            && ifaces
                .keys()
                .any(|k| k.starts_with("wan") || k == "cellular")
        {
            for (name, i) in ifaces {
                let ipv4 = i.pointer("/svis/ipv4").unwrap_or(&Value::Null);
                let mut row = with_network(net);
                row.insert("interface".into(), json!(name));
                if !s(&v, "serial").is_empty() {
                    row.insert("device".into(), json!(s(&v, "serial")));
                }
                copy(&mut row, i, &["enabled"]);
                if i.pointer("/vlanTagging/enabled") == Some(&json!(true)) {
                    row.insert(
                        "vlan".into(),
                        i.pointer("/vlanTagging/vlanId")
                            .cloned()
                            .unwrap_or(Value::Null),
                    );
                }
                copy(&mut row, ipv4, &["assignmentMode", "address", "gateway"]);
                if let Some(dns) = ipv4.pointer("/nameservers/addresses") {
                    row.insert("dns".into(), dns.clone());
                }
                if i.pointer("/pppoe/enabled") == Some(&json!(true)) {
                    row.insert("pppoe".into(), json!(true));
                }
                self.tables[UPLINKS].push(Value::Object(row));
            }
            return true;
        }
        if matches!(s(&v, "mode"), "hub" | "spoke" | "none")
            && (o.contains_key("subnets") || o.contains_key("hubs"))
        {
            let mut row = with_network(net);
            copy(&mut row, &v, &["mode"]);
            let hubs: Vec<Value> = list(&v, "hubs")
                .iter()
                .map(|h| json!({"hub": s(h, "hubId"), "default_route": h.get("useDefaultRoute")}))
                .collect();
            let subnets: Vec<Value> = list(&v, "subnets")
                .iter()
                .map(|n| json!({"subnet": s(n, "localSubnet"), "in_vpn": n.get("useVpn")}))
                .collect();
            row.insert("hubs".into(), Value::Array(hubs));
            row.insert("subnets".into(), Value::Array(subnets));
            self.tables[S2S].push(Value::Object(row));
            return true;
        }
        false
    }

    fn item(&mut self, kind: Kind, v: &Value, key: &str, parent: &str, net: Option<&str>) {
        let mut row = with_network(net);
        let comment = |row: &mut Map<String, Value>, field: &str| {
            let c = s(v, field);
            if !c.is_empty() {
                row.insert(field.into(), clean_comment(c, &self.withheld));
            }
        };
        let (table, fields): (usize, &[&str]) = match kind {
            Kind::Device => (
                DEVICES,
                &[
                    "name",
                    "serial",
                    "model",
                    "mac",
                    "lanIp",
                    "wan1Ip",
                    "wan2Ip",
                    "publicIp",
                    "firmware",
                    "productType",
                    "networkId",
                    "tags",
                ],
            ),
            Kind::Vlan => {
                let fixed: Vec<Value> = v
                    .get("fixedIpAssignments")
                    .and_then(Value::as_object)
                    .map(|m| {
                        m.iter()
                            .map(|(mac, a)| json!({"mac": mac, "ip": s(a, "ip"), "name": s(a, "name")}))
                            .collect()
                    })
                    .unwrap_or_default();
                if !fixed.is_empty() {
                    row.insert("fixed_ips".into(), Value::Array(fixed));
                }
                if v.get("id").is_none() {
                    row.insert("id".into(), json!("single LAN"));
                }
                (
                    VLANS,
                    &[
                        "id",
                        "name",
                        "subnet",
                        "cidr",
                        "applianceIp",
                        "groupPolicyId",
                        "dhcpHandling",
                        "dhcpRelayServerIps",
                        "dhcpLeaseTime",
                        "dnsNameservers",
                        "reservedIpRanges",
                        "vpnNatSubnet",
                    ],
                )
            }
            Kind::Port => (
                PORTS,
                &[
                    "number",
                    "enabled",
                    "type",
                    "vlan",
                    "allowedVlans",
                    "dropUntaggedTraffic",
                    "accessPolicy",
                ],
            ),
            Kind::Rule => {
                let k = format!("{parent} {key}").to_ascii_lowercase();
                let dir = if k.contains("inbound") {
                    "inbound"
                } else if k.contains("cellular") {
                    "cellular"
                } else {
                    "outbound"
                };
                row.insert("direction".into(), json!(dir));
                comment(&mut row, "comment");
                (
                    RULES,
                    &[
                        "policy",
                        "protocol",
                        "srcCidr",
                        "srcPort",
                        "destCidr",
                        "destPort",
                        "syslogEnabled",
                    ],
                )
            }
            Kind::L7 => (L7, &["policy", "type", "value"]),
            Kind::Forward => {
                comment(&mut row, "name");
                (
                    FORWARDS,
                    &[
                        "protocol",
                        "publicPort",
                        "lanIp",
                        "localPort",
                        "allowedIps",
                        "uplink",
                    ],
                )
            }
            Kind::OneToOne => {
                comment(&mut row, "name");
                (
                    ONE_TO_ONE,
                    &["publicIp", "lanIp", "uplink", "allowedInbound"],
                )
            }
            Kind::OneToMany => {
                let rules: Vec<Value> = list(v, "portRules")
                    .iter()
                    .map(|r| {
                        let mut m = Map::new();
                        if !s(r, "name").is_empty() {
                            m.insert("name".into(), clean_comment(s(r, "name"), &self.withheld));
                        }
                        copy(
                            &mut m,
                            r,
                            &[
                                "protocol",
                                "publicPort",
                                "localIp",
                                "localPort",
                                "allowedIps",
                            ],
                        );
                        Value::Object(m)
                    })
                    .collect();
                row.insert("port_rules".into(), Value::Array(rules));
                (ONE_TO_MANY, &["publicIp", "uplink"])
            }
            Kind::Route => (ROUTES, &["name", "subnet", "gatewayIp", "enabled"]),
            Kind::Peer => (
                PEERS,
                &[
                    "name",
                    "publicIp",
                    "privateSubnets",
                    "ikeVersion",
                    "ipsecPoliciesPreset",
                    "networkTags",
                ],
            ),
            Kind::UplinkStatus => {
                for u in list(v, "uplinks") {
                    let mut r = with_network(net);
                    copy(&mut r, v, &["serial", "model", "networkId"]);
                    copy(
                        &mut r,
                        u,
                        &[
                            "interface",
                            "status",
                            "ip",
                            "gateway",
                            "publicIp",
                            "primaryDns",
                            "secondaryDns",
                            "ipAssignedBy",
                        ],
                    );
                    self.tables[UPLINK_STATUS].push(Value::Object(r));
                }
                return;
            }
            Kind::Ssid => (
                SSIDS,
                &[
                    "number",
                    "name",
                    "enabled",
                    "authMode",
                    "encryptionMode",
                    "wpaEncryptionMode",
                    "defaultVlanId",
                    "visible",
                ],
            ),
        };
        copy(&mut row, v, fields);
        self.tables[table].push(Value::Object(row));
    }

    /// Devices name their network by id: add its name.
    fn name_networks(&mut self) {
        let names: Vec<(String, String)> = self
            .networks
            .iter()
            .map(|n| (s(n, "id").to_string(), s(n, "name").to_string()))
            .collect();
        for table in &mut self.tables {
            for row in table.iter_mut() {
                let Some(o) = row.as_object_mut() else {
                    continue;
                };
                if o.contains_key("network") {
                    continue;
                }
                let id = o.get("networkId").and_then(Value::as_str).unwrap_or("");
                if let Some((_, name)) = names.iter().find(|(i, _)| i == id) {
                    o.insert("network".into(), json!(name));
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Device,
    Vlan,
    Port,
    Rule,
    L7,
    Forward,
    OneToOne,
    OneToMany,
    Route,
    Peer,
    UplinkStatus,
    Ssid,
}

/// What one row is, by the fields the API gives it.
fn item_kind(v: &Value) -> Option<Kind> {
    let o = v.as_object()?;
    let has = |k: &str| o.contains_key(k);
    Some(if has("uplinks") && has("serial") {
        Kind::UplinkStatus
    } else if has("serial") && has("model") {
        Kind::Device
    } else if has("applianceIp") && (has("subnet") || has("cidr")) {
        Kind::Vlan
    } else if has("destCidr") && has("srcCidr") && has("policy") {
        Kind::Rule
    } else if has("policy") && has("type") && has("value") {
        Kind::L7
    } else if has("lanIp") && has("publicPort") {
        Kind::Forward
    } else if has("publicIp") && has("portRules") {
        Kind::OneToMany
    } else if has("publicIp") && has("lanIp") {
        Kind::OneToOne
    } else if has("publicIp") && has("privateSubnets") {
        Kind::Peer
    } else if has("gatewayIp") && has("subnet") {
        Kind::Route
    } else if has("number") && has("authMode") {
        Kind::Ssid
    } else if has("number")
        && (has("allowedVlans") || has("accessPolicy") || has("dropUntaggedTraffic"))
    {
        Kind::Port
    } else {
        return None;
    })
}

fn is_network(o: &Map<String, Value>) -> bool {
    let id = o.get("id").and_then(Value::as_str).unwrap_or("");
    (id.starts_with("L_") || id.starts_with("N_"))
        && o.contains_key("name")
        && (o.contains_key("productTypes") || o.contains_key("timeZone"))
}

fn is_org(o: &Map<String, Value>) -> bool {
    o.contains_key("id")
        && o.contains_key("name")
        && o.get("url")
            .and_then(Value::as_str)
            .is_some_and(|u| u.contains("meraki.com"))
}

fn with_network(net: Option<&str>) -> Map<String, Value> {
    let mut m = Map::new();
    if let Some(n) = net {
        m.insert("network".into(), json!(n));
    }
    m
}

fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Copy `keys` from `v`, never one that names a credential.
fn copy(row: &mut Map<String, Value>, v: &Value, keys: &[&str]) {
    for k in keys {
        if let Some(x) = v.get(*k)
            && !x.is_null()
            && !secret_key(k)
        {
            row.insert((*k).into(), x.clone());
        }
    }
}

fn pick(o: &Map<String, Value>, keys: &[&str]) -> Map<String, Value> {
    let mut m = Map::new();
    copy(&mut m, &Value::Object(o.clone()), keys);
    m
}

/// A setting that holds a credential.
fn secret_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    [
        "secret",
        "psk",
        "password",
        "passphrase",
        "community",
        "presharedkey",
        "apikey",
        "privatekey",
    ]
    .iter()
    .any(|w| k.contains(w))
}

/// Every credential's value, for Notes to redact.
fn secrets_in(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(o) => {
            for (k, x) in o {
                match x {
                    Value::String(t) if secret_key(k) && crate::redact::secret_value(t) => {
                        out.push(t.trim().to_string());
                    }
                    _ => secrets_in(x, out),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| secrets_in(x, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What tools/meraki-export.ps1 writes, trimmed.
    const EXPORT: &str = r#"{
      "source": "meraki-export.ps1",
      "organization": {"id": "123", "name": "Harbor Dental", "url": "https://n1.meraki.com/o/x/manage/organization/overview"},
      "devices": [
        {"name": "Front MX", "serial": "Q2XX-AAAA-0001", "model": "MX68", "mac": "e0:55:3d:00:00:01",
         "lanIp": null, "wan1Ip": "203.0.113.42", "firmware": "MX 18.211", "networkId": "L_1", "productType": "appliance"},
        {"name": "Lobby AP", "serial": "Q2XX-AAAA-0002", "model": "MR36", "mac": "e0:55:3d:00:00:02",
         "lanIp": "10.20.0.21", "networkId": "L_1", "productType": "wireless"}
      ],
      "networks": [{
        "id": "L_1", "name": "Main Office", "productTypes": ["appliance", "wireless"], "timeZone": "America/Chicago",
        "appliance": {
          "vlans": [
            {"id": 1, "name": "Data", "subnet": "10.20.0.0/24", "applianceIp": "10.20.0.1", "dhcpHandling": "Run a DHCP server",
             "fixedIpAssignments": {"00:11:22:33:44:55": {"ip": "10.20.0.30", "name": "Printer"}}},
            {"id": 40, "name": "Cameras", "subnet": "10.20.4.0/24", "applianceIp": "10.20.4.1"}
          ],
          "ports": [{"number": 3, "enabled": true, "type": "trunk", "vlan": 1, "allowedVlans": "all", "dropUntaggedTraffic": false}],
          "l3FirewallRules": {"rules": [
            {"comment": "Cameras stay local", "policy": "deny", "protocol": "any", "srcPort": "Any", "srcCidr": "10.20.4.0/24", "destPort": "Any", "destCidr": "Any", "syslogEnabled": false},
            {"comment": "temp: wifi password is Harbor2024!", "policy": "allow", "protocol": "tcp", "srcPort": "Any", "srcCidr": "Any", "destPort": "443", "destCidr": "Any"},
            {"comment": "Default rule", "policy": "allow", "protocol": "Any", "srcPort": "Any", "srcCidr": "Any", "destPort": "Any", "destCidr": "Any"}
          ]},
          "inboundFirewallRules": {"rules": [
            {"comment": "Default rule", "policy": "allow", "protocol": "Any", "srcPort": "Any", "srcCidr": "Any", "destPort": "Any", "destCidr": "Any"}
          ]},
          "portForwardingRules": {"rules": [
            {"name": "RDP to server", "lanIp": "10.20.1.10", "publicPort": "3389", "localPort": "3389", "allowedIps": ["any"], "protocol": "tcp", "uplink": "internet1"}
          ]},
          "oneToOneNatRules": {"rules": [{"name": "PACS", "publicIp": "203.0.113.43", "lanIp": "10.20.1.12", "uplink": "internet1", "allowedInbound": []}]},
          "staticRoutes": [{"name": "Lab", "subnet": "192.168.50.0/24", "gatewayIp": "10.20.0.254", "enabled": true}],
          "siteToSiteVpn": {"mode": "spoke", "hubs": [{"hubId": "L_9", "useDefaultRoute": false}], "subnets": [{"localSubnet": "10.20.0.0/24", "useVpn": true}]},
          "ssids": [{"number": 0, "name": "Harbor", "enabled": true, "authMode": "psk", "psk": "SuperSecretWifi1"}]
        },
        "uplinkSettings": [{"serial": "Q2XX-AAAA-0001", "interfaces": {
          "wan1": {"enabled": true, "vlanTagging": {"enabled": false}, "svis": {"ipv4": {"assignmentMode": "static", "address": "203.0.113.42/29", "gateway": "203.0.113.41", "nameservers": {"addresses": ["1.1.1.1"]}}},
                   "pppoe": {"enabled": false, "authentication": {"enabled": false, "username": "u", "password": "PppoeSecret99"}}},
          "wan2": {"enabled": false}
        }}]
      }],
      "thirdPartyVPNPeers": {"peers": [{"name": "Lab peer", "publicIp": "198.51.100.7", "privateSubnets": ["172.16.0.0/16"], "secret": "IkeSecret#77", "ikeVersion": "2"}]},
      "uplinkStatuses": [{"networkId": "L_1", "serial": "Q2XX-AAAA-0001", "model": "MX68", "uplinks": [{"interface": "wan1", "status": "active", "ip": "203.0.113.42", "publicIp": "203.0.113.42"}]}]
    }"#;

    #[test]
    fn reads_an_export_without_secrets() {
        assert!(sniff(EXPORT.as_bytes()));
        let bom = [b"\xEF\xBB\xBF".as_slice(), EXPORT.as_bytes()].concat();
        assert!(sniff(&bom) && convert(&bom).is_ok());
        let r = convert(EXPORT.as_bytes()).unwrap();
        for (t, n) in [
            ("devices", 2),
            ("VLANs", 2),
            ("appliance ports", 1),
            ("firewall rules", 4),
            ("port forwards", 1),
            ("1:1 NAT", 1),
            ("static routes", 1),
            ("site-to-site VPN", 1),
            ("VPN peers", 1),
            ("uplinks", 2),
            ("uplink status", 1),
            ("appliance SSIDs", 1),
            ("networks", 1),
        ] {
            assert_eq!(r.count(t), n, "{t}");
        }
        for secret in [
            "SuperSecretWifi1",
            "PppoeSecret99",
            "IkeSecret#77",
            "Harbor2024",
        ] {
            assert!(!r.json.contains(secret), "{secret} leaked");
        }
        for secret in ["SuperSecretWifi1", "PppoeSecret99", "IkeSecret#77"] {
            assert!(r.secrets.contains(&secret.to_string()), "{secret} not held");
        }
        assert_eq!(r.withheld, 1);
        let v: Value = serde_json::from_str(&r.json).unwrap();
        assert_eq!(v["firewall_rules"][3]["direction"], "inbound");
        assert_eq!(v["firewall_rules"][0]["network"], "Main Office");
        assert_eq!(v["vlans"][0]["fixed_ips"][0]["name"], "Printer");
        assert_eq!(v["devices"][1]["network"], "Main Office");
        assert_eq!(v["uplinks"][0]["address"], "203.0.113.42/29");
        assert!(
            r.summary()
                .contains("MX68 Q2XX-AAAA-0001, MX 18.211 (Front MX)")
        );
        assert!(r.summary().contains("Harbor Dental"));
    }

    #[test]
    fn reads_a_zip_of_one_file_per_call() {
        let vlans =
            br#"[{"id": 1, "name": "Data", "subnet": "10.0.0.0/24", "applianceIp": "10.0.0.1"}]"#;
        let rules = br#"{"rules": [{"comment": "x", "policy": "allow", "protocol": "any", "srcPort": "Any", "srcCidr": "Any", "destPort": "Any", "destCidr": "Any"}]}"#;
        let inbound = br#"{"rules": [{"policy": "deny", "protocol": "any", "srcPort": "Any", "srcCidr": "Any", "destPort": "Any", "destCidr": "Any"}]}"#;
        let zip = crate::unifi::container_test_zip(&[
            ("backup/Branch/vlans.json", vlans),
            ("backup/Branch/l3FirewallRules.json", rules),
            ("backup/Branch/inboundFirewallRules.json", inbound),
            ("backup/readme.txt", b"hi"),
        ]);
        assert!(sniff(&zip));
        let r = convert(&zip).unwrap();
        assert_eq!(r.files, 3);
        let v: Value = serde_json::from_str(&r.json).unwrap();
        assert_eq!(v["vlans"][0]["network"], "Branch");
        assert_eq!(v["firewall_rules"][1]["direction"], "inbound");
    }

    #[test]
    fn other_json_is_not_meraki() {
        assert!(!sniff(
            br#"{"devices": [{"serial": "x", "model": "USW-24"}], "name": "n"}"#
        ));
        assert!(!sniff(b"{\"a\": 1}"));
        assert!(!sniff(b"not json"));
        assert!(convert(b"{\"a\": 1}").is_err());
    }
}
