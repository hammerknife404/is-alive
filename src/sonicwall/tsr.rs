//! SonicWall tech support report (.wri) → the same JSON an .exp gives.
//!
//! A TSR is a readable report in `#Area : Name_START` … `_END` sections.
//! Only the sections read here are touched: system status (model, serial,
//! firmware), interfaces, zones, address objects and groups, NAT policies
//! and access rules. Users, VPN, RADIUS/LDAP, SNMP, certificates, licences,
//! registration codes, logs and diagnostics are skipped whole, and each
//! section is read field by field, so nothing else in it is copied.

use super::{Report, WRI, clean_comment, mac, obj, s};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::cell::Cell;
use std::sync::LazyLock;

const INTERFACES: &str = "Network : Interfaces";

/// The report's text, if `data` is one: plain ASCII, or UTF-16 with a BOM.
fn text(data: &[u8]) -> Option<String> {
    let t = match data {
        [0xff, 0xfe, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xfe, 0xff, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => {
            let marker = format!("#{INTERFACES}_START");
            if !data.windows(marker.len()).any(|w| w == marker.as_bytes()) {
                return None;
            }
            String::from_utf8_lossy(data).into_owned()
        }
    };
    t.contains(&format!("#{INTERFACES}_START")).then_some(t)
}

fn utf16(b: &[u8], word: fn([u8; 2]) -> u16) -> String {
    let w: Vec<u16> = b.as_chunks::<2>().0.iter().map(|&c| word(c)).collect();
    String::from_utf16_lossy(&w)
}

pub fn sniff(data: &[u8]) -> bool {
    text(data).is_some()
}

/// The first `#<name>_START` … `#<name>_END` section; the report repeats
/// some tables further down, in its debug dump.
fn section<'a>(t: &'a str, name: &str) -> &'a str {
    let (start, end) = (format!("#{name}_START"), format!("#{name}_END"));
    let Some(a) = t.find(&start).map(|i| i + start.len()) else {
        return "";
    };
    let b = t[a..].find(&end).map_or(t.len(), |i| a + i);
    &t[a..b]
}

/// `Key   : value` → ("Key", "value").
fn kv(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once(':')?;
    Some((k.trim(), v.trim()))
}

fn ip(v: &str) -> Option<&str> {
    (v.parse::<std::net::Ipv4Addr>().is_ok() && v != "0.0.0.0").then_some(v)
}

/// A default object is written `X0 IP(LAN Primary IP)`: name, then its
/// display name. A custom name with a space before a bracket is left alone.
fn object_name(v: &str) -> &str {
    match v.find('(') {
        Some(i) if v.ends_with(')') && i > 0 && !v[..i].ends_with(' ') => &v[..i],
        _ => v,
    }
}

fn system(t: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in section(t, "System : Status").lines() {
        let Some((k, v)) = kv(line) else { continue };
        let key = match k {
            "Model" => "model",
            "Serial number" => "serialNumber",
            "Firmware Version" => "firmwareVersion",
            _ => continue,
        };
        if !v.is_empty() && !out.iter().any(|(k, _)| k == key) {
            out.push((key.to_string(), v.to_string()));
        }
    }
    out
}

#[derive(Default)]
struct Iface {
    name: String,
    zone: Option<String>,
    ip: Option<String>,
    mask: Option<String>,
    gateway: Option<String>,
    mac: Option<String>,
    mgmt: Vec<&'static str>,
    disabled: bool,
    comment: Option<String>,
}

fn interfaces(t: &str, withheld: &Cell<usize>) -> Vec<Value> {
    let mut all: Vec<Iface> = Vec::new();
    // Inside the block's [IPv4 Settings]: the address and the management
    // the firewall enforces for IPv4.
    let mut v4 = false;
    for line in section(t, INTERFACES).lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            v4 = trimmed == "[IPv4 Settings]";
            continue;
        }
        let Some((k, v)) = kv(line) else { continue };
        if k == "Interface Name" {
            all.push(Iface {
                name: v.to_string(),
                ..Default::default()
            });
            v4 = false;
            continue;
        }
        let Some(i) = all.last_mut() else { continue };
        let first = |slot: &mut Option<String>, val: Option<&str>| {
            if slot.is_none() {
                *slot = val.filter(|v| !v.is_empty()).map(String::from);
            }
        };
        match k {
            "Comment" => first(&mut i.comment, Some(v)),
            "Config MAC" => first(&mut i.mac, Some(mac(v).as_str())),
            // "LAN    Handle: 1      Type  : 1"
            "Zone" => first(&mut i.zone, v.split("Handle:").next().map(str::trim)),
            "IP Address" if v4 => first(&mut i.ip, ip(v)),
            "Network Mask" if v4 => first(&mut i.mask, ip(v)),
            "Gateway IP" if v4 => first(&mut i.gateway, ip(v)),
            "Port Shutdown" if v4 => i.disabled = v == "1",
            _ if v4 && k.starts_with("Interface ") && k.ends_with(" Management") => {
                let proto = &k["Interface ".len()..k.len() - " Management".len()];
                let known = ["http", "https", "ssh", "ping", "snmp"]
                    .into_iter()
                    .find(|p| p.eq_ignore_ascii_case(proto));
                if let Some(p) = known
                    && v == "Yes"
                    && !i.mgmt.contains(&p)
                {
                    i.mgmt.push(p);
                }
            }
            _ => {}
        }
    }
    all.into_iter()
        .map(|i| {
            // Sub-interfaces are named "<parent>:V<tag>".
            let (parent, vlan) = match i.name.split_once(":V") {
                Some((p, tag)) if tag.chars().all(|c| c.is_ascii_digit()) => {
                    (Some(p.to_string()), Some(tag.to_string()))
                }
                _ => (None, None),
            };
            obj(vec![
                ("name", s(Some(&i.name))),
                ("zone", s(i.zone.as_deref())),
                (
                    "type",
                    Some(json!(if vlan.is_some() { "vlan" } else { "physical" })),
                ),
                ("vlan", s(vlan.as_deref())),
                ("parent", s(parent.as_deref())),
                ("ip", s(i.ip.as_deref())),
                ("mask", s(i.mask.as_deref())),
                ("gateway", s(i.gateway.as_deref())),
                ("mac", s(i.mac.as_deref())),
                ("management", (!i.mgmt.is_empty()).then(|| json!(i.mgmt))),
                ("disabled", i.disabled.then_some(json!(true))),
                (
                    "comment",
                    i.comment.as_deref().map(|c| clean_comment(c, withheld)),
                ),
            ])
        })
        .collect()
}

fn zones(t: &str) -> Vec<Value> {
    let mut out: Vec<Map<String, Value>> = Vec::new();
    for line in section(t, "Network : Zones").lines() {
        match kv(line) {
            Some(("Name", v)) => {
                let mut m = Map::new();
                m.insert("name".into(), json!(object_name(v)));
                out.push(m);
            }
            Some(("Security Type", v)) => {
                if let Some(m) = out.last_mut() {
                    m.entry("security_type").or_insert(json!(v));
                }
            }
            _ => {}
        }
    }
    out.into_iter().map(Value::Object).collect()
}

/// `-------Name-------`, the head of each object and group.
static BANNER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-{5,}(.+?)-{5,}\s*$").expect("valid pattern"));

/// (address objects, FQDN objects, groups → members).
fn addresses(t: &str) -> (Vec<Value>, Vec<Value>, Map<String, Value>) {
    let sec = section(t, "Network : Address Objects");
    let (mut objects, mut fqdn, mut groups) = (Vec::new(), Vec::new(), Map::new());
    let mut in_groups = false;
    let mut name: Option<String> = None;
    for line in sec.lines() {
        match line.trim() {
            "--Address Group Table--" => {
                in_groups = true;
                name = None;
                continue;
            }
            "--Address Object Table--" => {
                in_groups = false;
                name = None;
                continue;
            }
            _ => {}
        }
        if let Some(c) = BANNER.captures(line) {
            name = Some(object_name(c[1].trim()).to_string());
            if in_groups && let Some(n) = &name {
                groups.insert(n.clone(), json!([]));
            }
            continue;
        }
        let Some(n) = &name else { continue };
        if in_groups {
            // "   member: Name:X0 Subnet Handle:12"
            if let Some(rest) = line.trim().strip_prefix("member: Name:") {
                let member = rest.rsplit_once(" Handle:").map_or(rest, |(m, _)| m).trim();
                if let Some(Value::Array(a)) = groups.get_mut(n) {
                    a.push(json!(member));
                }
            }
            continue;
        }
        let Some((k, v)) = kv(line) else { continue };
        let pair = |v: &str| {
            let (a, b) = v.split_once(" - ").unwrap_or((v, ""));
            (a.trim().to_string(), b.trim().to_string())
        };
        let o = match k {
            "HOST" => Some(json!({ "name": n, "type": "host", "ip1": v })),
            "NETWORK" => {
                let (a, m) = pair(v);
                Some(json!({ "name": n, "type": "network", "ip1": a, "ip2": m }))
            }
            "RANGE" => {
                let (a, b) = pair(v);
                Some(json!({ "name": n, "type": "range", "ip1": a, "ip2": b }))
            }
            "FQDN" => {
                fqdn.push(json!({ "name": n, "fqdn": v }));
                name = None;
                None
            }
            _ => None,
        };
        if let Some(o) = o {
            objects.push(o);
            name = None;
        }
    }
    (objects, fqdn, groups)
}

/// "Rule 12 LAN -> WAN Allow Service Any -> HTTPS (Enabled)".
static RULE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^Rule (\d+) (.+?) -> (.+?) (Allow|Deny|Discard) Service (.+?) -> (.+) \((Enabled|Disabled)\)\s*$",
    )
    .expect("valid pattern")
});
/// "       IP: Any -> All X0 Management IP  Iface: …".
static RULE_IP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*IP: (.+?) -> (.+?)(?:\s{2,}Iface:.*)?\s*$").expect("valid pattern")
});

fn access_rules(t: &str, withheld: &Cell<usize>) -> Vec<Value> {
    let mut out: Vec<Map<String, Value>> = Vec::new();
    for line in section(t, "Firewall : Access Rules").lines() {
        if let Some(c) = RULE.captures(line) {
            let mut m = Map::new();
            for (k, i) in [
                ("src_zone", 2),
                ("dst_zone", 3),
                ("src_service", 5),
                ("service", 6),
            ] {
                m.insert(k.into(), json!(c[i].trim()));
            }
            m.insert("action".into(), json!(c[4].to_lowercase()));
            m.insert("enabled".into(), json!(&c[7] == "Enabled"));
            out.push(m);
            continue;
        }
        let Some(m) = out.last_mut() else { continue };
        if !m.contains_key("src")
            && let Some(c) = RULE_IP.captures(line)
        {
            m.insert("src".into(), json!(c[1].trim()));
            m.insert("dst".into(), json!(c[2].trim()));
        } else if let Some(("Comment", v)) = kv(line)
            && !v.is_empty()
            && !m.contains_key("comment")
        {
            m.insert("comment".into(), clean_comment(v, withheld));
        }
    }
    out.into_iter().map(Value::Object).collect()
}

fn nat_rules(t: &str, withheld: &Cell<usize>) -> Vec<Value> {
    let mut out: Vec<Map<String, Value>> = Vec::new();
    for line in section(t, "Network : NAT Policies").lines() {
        let Some((k, v)) = kv(line) else { continue };
        if k == "Index" {
            out.push(Map::new());
            continue;
        }
        let Some(m) = out.last_mut() else { continue };
        let key = match k {
            "Name" => "name",
            "Original Source" => "orig_src",
            "Translated Source" => "trans_src",
            "Original Destination" => "orig_dst",
            "Translated Destination" => "trans_dst",
            "Original Service" => "orig_service",
            "Translated Service" => "trans_service",
            "Inbound Interface" => "in_iface",
            "Outbound Interface" => "out_iface",
            "Enable NAT Policy" => {
                m.entry("enabled").or_insert(json!(v == "1"));
                continue;
            }
            "Comment" if !v.is_empty() => {
                if !m.contains_key("comment") {
                    m.insert("comment".into(), clean_comment(v, withheld));
                }
                continue;
            }
            _ => continue,
        };
        if !v.is_empty() {
            m.entry(key).or_insert(json!(v));
        }
    }
    out.into_iter()
        .filter(|m| !m.is_empty())
        .map(Value::Object)
        .collect()
}

pub fn convert(data: &[u8]) -> Result<Report, String> {
    let t = text(data).ok_or("not a SonicWall tech support report")?;
    let withheld = Cell::new(0);
    let ifaces = interfaces(&t, &withheld);
    if ifaces.is_empty() {
        return Err("tech support report has no interfaces section".into());
    }
    let zones = zones(&t);
    let (addresses, fqdn, groups) = addresses(&t);
    let rules = access_rules(&t, &withheld);
    let nat = nat_rules(&t, &withheld);
    let system = system(&t);
    let counts = vec![
        ("interfaces", ifaces.len()),
        ("zones", zones.len()),
        ("address objects", addresses.len() + fqdn.len()),
        ("address groups", groups.len()),
        ("access rules", rules.len()),
        ("NAT rules", nat.len()),
    ];
    let v = json!({
        "source": format!("is-alive {} {WRI}", crate::VERSION),
        "system": system.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>(),
        "notes": [
            "Read from the firewall's tech support report, so this is enforced config, not observation.",
            "Interface mac is the port's own address: match it against /unifidump's discovered_devices and uplink MACs.",
            "Rule action is as the report words it (allow, deny, discard). Service objects aren't read from a report; rules name their services.",
            "Accounts, passwords, VPN/RADIUS/LDAP settings, SNMP, certificates and registration codes were not copied.",
        ],
        "interfaces": ifaces,
        "zones": zones,
        "address_objects": addresses,
        "fqdn_objects": fqdn,
        "address_groups": groups,
        "access_rules": rules,
        "nat_rules": nat,
    });
    Ok(Report {
        format: WRI,
        keys: 0,
        withheld: withheld.get(),
        used: 0,
        counts,
        system,
        json: serde_json::to_string_pretty(&v).unwrap_or_default() + "\n",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like a SonicOS 7 report; every value is made up.
    const REPORT: &str = "#version 2.0\n\n#System : Status_START\n#Blade_1_STATUS_START\n\
--System Information--\nModel                     : TZ 370\n\
Serial number             : 0040-1000-0001\nFirmware Version          : SonicOS 7.3.3-7014\n\
Authentication Code       : ABCD-EFGH\nRegistration Code         : SECRETREG\n\
#Blade_1_STATUS_END\n#System : Status_END\n\n\
#System : Administration_START\nAdmin Name : admin\nAdmin Password : Hunter2Admin\n#System : Administration_END\n\
#Network : Interfaces_START\n#Blade_1_INTERFACES_START\n\
Interface Name                                  : X0                              \n\
Portshield VLAN: \t\t\t\t2\n\
Comment                                         : Default LAN                     \n\
Config MAC                                      : 00:40:10:00:00:01\n\
Zone  : LAN    Handle: 1      Type  : 1     \n\n[IPv4 Settings]\n\
IP Address                                      : 10.20.0.1\n\
Network Mask                                    : 255.255.255.0\n\
Interface http Management                       : No                              \n\
Interface https Management                      : Yes                             \n\
Interface ping Management                       : Yes                             \n\
Port Shutdown                                   : 0\n\n[IPv6 Settings]\n\
Interface SSH Management                        : Yes                             \n\
Interface Name                                  : X0:V70                          \n\
Comment                                         : Guest wifi pass is Sunshine1     \n\
Config MAC                                      : 00:40:10:00:00:01\n\
Zone  : Guest WLAN Handle: 3      Type  : 2     \n\n[IPv4 Settings]\n\
IP Address                                      : 172.16.70.1\n\
Network Mask                                    : 255.255.255.0\n\n\
Interface Name                                  : X1                              \n\
Comment                                         : ISP fiber                       \n\
Config MAC                                      : 00:40:10:00:00:02\n\
Zone  : WAN    Handle: 2      Type  : 0     \n\n[IPv4 Settings]\n\
IP Address                                      : 203.0.113.2\n\
Network Mask                                    : 255.255.255.248\n\
Gateway IP                                      : 203.0.113.1\n\
Interface https Management                      : Yes                             \n\n\
Interface Name                                  : X5                              \n\
Zone  : Unassigned Handle: 0      Type  : 0     \n\n[IPv4 Settings]\n\
IP Address                                      : 0.0.0.0\n\
#Blade_1_INTERFACES_END\n#Network : Interfaces_END\n\n\
#Network : Zones_START\n--------------------- LAN(LAN) ---------------------\nGeneral Settings\n\
Name:                                 LAN(LAN)\nSecurity Type:                        Trusted\n\
Name:                                 Guest WLAN\nSecurity Type:                        Wireless\n\
#Network : Zones_END\n\n\
#Network : Address Objects_START\n\n--Address Group Table--\n\n-------LAN Subnets-------\n\
UUID: d4f1\n   member: Name:X0 Subnet Handle:12\n\
  Group (Member of):  Firewalled Subnets,   Jane.Doe12345,   \n\n\
--Address Object Table--\n\n-------X0 Subnet(LAN Primary Subnet)-------\n\
NETWORK: 10.20.0.0 - 255.255.255.0\nNetwork Size: 254\n\
  Group (Member of):  LAN Subnets,   Jane.Doe12345,   \n\n\
-------File Server-------\nHOST: 10.20.0.10\n\n-------Old box (2019)-------\nHOST: 10.20.0.11\n\n\
-------TimeClock-------\nRANGE: 198.51.100.81 - 198.51.100.84\n\n\
-------sendgrid-------\nFQDN: smtp.example.net\nHOST(S) 0xffff 24: 192.0.2.1 TTL=90,\n\
#Network : Address Objects_END\n\n\
#Network : NAT Policies_START\nIP Version                  : IPv4\nIndex                       : 1\n\
Name                        :Default NAT Policy\nOriginal Source             : ANY\n\
Translated Source           : ORIGINAL\nOriginal Destination        : X1 IP\n\
Translated Destination      : File Server\nOriginal Service            : HTTPS\n\
Translated Service          : ORIGINAL\nInbound Interface           : X1\nOutbound Interface          : Any\n\
Comment                     : Web to the file server\nEnable NAT Policy           : 1\n\
#Network : NAT Policies_END\n\n\
#Users : Local Users_START\nUser: jsmith Password: S3cretPass\n#Users : Local Users_END\n\
#Firewall : Access Rules_START\n#Blade_1_ACCESS_RULES_START\n\
Rule 1 LAN -> WAN Allow Service Any -> HTTPS (Enabled)\n\
       IP: Any -> Any  Iface: Any (ffffffff) -> Any (ffffffff) \n\
Policy Name:               Default Access Rule\nComment:                   web out\n\n\
Rule 2 Guest WLAN -> Guest WLAN Deny Service Any -> SSLVPN (Disabled)\n\
       IP: Guest WLAN Subnets -> All X0 Management IP  Iface: Any (ffffffff) -> Any (ffffffff) \n\
Comment:                   \n\
#Firewall : Access Rules_END\n\
#Debug Information_START:\n#Network : Interfaces_START\nInterface Name : X9\n#Network : Interfaces_END\n";

    #[test]
    fn reads_a_sonicos_7_report() {
        assert!(sniff(REPORT.as_bytes()));
        let r = convert(REPORT.as_bytes()).unwrap();
        for secret in [
            "Hunter2Admin",
            "SECRETREG",
            "ABCD-EFGH",
            "S3cretPass",
            "Sunshine1",
            "Jane.Doe",
        ] {
            assert!(!r.json.contains(secret), "{secret} leaked");
        }
        let v: Value = serde_json::from_str(&r.json).unwrap();
        assert_eq!(v["system"]["model"], "TZ 370");
        assert_eq!(v["system"]["serialNumber"], "0040-1000-0001");
        let ifs = v["interfaces"].as_array().unwrap();
        // The debug dump's repeat of the table isn't read twice.
        assert_eq!(ifs.len(), 4);
        let x0 = &ifs[0];
        assert_eq!(x0["zone"], "LAN");
        assert_eq!(x0["ip"], "10.20.0.1");
        assert_eq!(x0["mac"], "00:40:10:00:00:01");
        // IPv6's SSH management is a different setting.
        assert_eq!(x0["management"], json!(["https", "ping"]));
        assert_eq!(x0["comment"], "Default LAN");
        let v70 = &ifs[1];
        assert_eq!(
            (&v70["zone"], &v70["vlan"], &v70["parent"], &v70["type"]),
            (
                &json!("Guest WLAN"),
                &json!("70"),
                &json!("X0"),
                &json!("vlan")
            )
        );
        assert_eq!(v70["comment"], "[withheld: may contain a credential]");
        assert_eq!(ifs[2]["gateway"], "203.0.113.1");
        assert!(ifs[3].get("ip").is_none());
        assert_eq!(
            v["zones"],
            json!([
                {"name": "LAN", "security_type": "Trusted"},
                {"name": "Guest WLAN", "security_type": "Wireless"}
            ])
        );
        let objs = v["address_objects"].as_array().unwrap();
        assert_eq!(objs[0]["name"], "X0 Subnet");
        assert_eq!(objs[0]["ip2"], "255.255.255.0");
        assert_eq!(objs[2]["name"], "Old box (2019)");
        assert_eq!(objs[3]["type"], "range");
        assert_eq!(v["fqdn_objects"][0]["fqdn"], "smtp.example.net");
        assert_eq!(v["address_groups"]["LAN Subnets"], json!(["X0 Subnet"]));
        let rules = v["access_rules"].as_array().unwrap();
        assert_eq!(rules[0]["action"], "allow");
        assert_eq!(rules[0]["service"], "HTTPS");
        assert_eq!(rules[0]["comment"], "web out");
        assert_eq!(rules[1]["src_zone"], "Guest WLAN");
        assert_eq!(rules[1]["dst"], "All X0 Management IP");
        assert_eq!(rules[1]["enabled"], false);
        assert!(rules[1].get("comment").is_none());
        let nat = &v["nat_rules"][0];
        assert_eq!(nat["trans_dst"], "File Server");
        assert_eq!(nat["enabled"], true);
        assert_eq!(r.withheld, 1);
        assert_eq!(r.count("access rules"), 2);
        assert!(r.summary().contains(".wri"));
    }

    #[test]
    fn utf16_and_junk() {
        let mut wide = vec![0xff, 0xfe];
        wide.extend(REPORT.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(convert(&wide).unwrap().count("interfaces"), 4);
        assert!(!sniff(b"#System : Status_START\nModel : TZ\n"));
        assert!(convert(b"#Network : Interfaces_START\n#Network : Interfaces_END").is_err());
        for n in (0..REPORT.len()).step_by(41) {
            let _ = convert(&REPORT.as_bytes()[..n]);
        }
    }
}
