//! Topology findings: what the configs and this machine's own view say
//! about how the site is put together, beyond any one card.
//!
//! - **Phones outside the voice VLAN.** A voice network is known (a
//!   config's network, VLAN or zone named for voice, a /mapnetwork zone
//!   for voice, or the voice VLAN the scanner's switch port announced) and
//!   a desk phone sits on another subnet.
//! - **Which WAN the LAN leaves by.** The scanner's public address (and
//!   its traceroute's first public hop) against the firewalls' WAN
//!   interfaces: the matching one is marked, and a backup WAN or no match
//!   at all is a finding.
//! - **Wi-Fi outside the firewall.** An access point or a Wi-Fi network
//!   whose subnet isn't one the firewall serves.
//!
//! Read from the converted JSON the Map already holds; nothing is sent.

use super::{
    B, Input, Link, Node, Sev, Zone, cidr, describe, describe_some, prefix, text, word_in,
};
use crate::net::mac::{self, Mac};
use serde_json::Value;
use std::collections::HashSet;
use std::net::Ipv4Addr;

/// A network as its address and prefix (any address on it).
type Net = (Ipv4Addr, u8);

/// Whether `ip` is on `net`.
fn on(ip: Ipv4Addr, net: Net) -> bool {
    super::on_net(ip, net)
}

/// "10.40.0.0/24": the network's own address, not the gateway's.
fn net_text((ip, bits): Net) -> String {
    let mask = u32::MAX.checked_shl(32 - u32::from(bits)).unwrap_or(0);
    format!("{}/{bits}", Ipv4Addr::from(u32::from(ip) & mask))
}

/// An address the internet routes: not private, shared (CGNAT),
/// loopback, link-local or special. The documentation ranges count as
/// public, as they stand in for public addresses in examples.
pub fn is_public(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || o[0] == 0
        || o[0] >= 240
        || (o[0] == 100 && (64..128).contains(&o[1])))
}

/// A name that says voice: "Voice", "VoIP", "Phones", "Telephony".
fn says_voice(name: &str) -> bool {
    ["voice", "voip", "phone", "phones", "telephony", "telephone"]
        .iter()
        .any(|w| word_in(name, w))
}

/// A card for a desk phone: its kind ends in PHONE (by its MAC's maker,
/// its web page or its name).
fn is_phone(n: &Node) -> bool {
    // "YEALINK PHONE", not "PHONE SYSTEM" (a PBX).
    !n.drawn && !n.gone && n.kind.split_whitespace().last() == Some("PHONE")
}

fn vlan_of(v: &Value, k: &str) -> Option<u16> {
    match v.get(k)? {
        Value::Number(n) => n.as_u64().and_then(|n| u16::try_from(n).ok()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|v| *v > 0 && *v < 4095)
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).map_or("", str::trim)
}

fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn addr(v: &Value, k: &str) -> Option<Ipv4Addr> {
    s(v, k)
        .parse()
        .ok()
        .filter(|a: &Ipv4Addr| !a.is_unspecified())
}

// ------------------------------------------------------------- networks

/// A LAN-side network a config names.
#[derive(Clone, Debug, Default)]
pub(super) struct Lan {
    pub name: String,
    pub vlan: Option<u16>,
    pub net: Option<Net>,
    /// UniFi's `_id`, which its Wi-Fi networks point at.
    pub id: String,
    pub site: Option<String>,
}

impl Lan {
    /// "VLAN 40 Voice (10.40.0.0/24)".
    fn line(&self) -> String {
        let mut l = match self.vlan {
            Some(v) => format!("VLAN {v} {}", self.name),
            None => self.name.clone(),
        };
        if let Some(n) = self.net {
            l += &format!(" ({})", net_text(n));
        }
        l.trim().to_string()
    }
}

/// A WAN interface on a firewall.
#[derive(Clone, Debug, Default)]
pub(super) struct Wan {
    /// "X1", "wan2", "WAN2".
    pub name: String,
    /// Its own address, and the public one it's NATed to when that's known.
    pub addrs: Vec<Ipv4Addr>,
    pub net: Option<Net>,
    pub gateway: Option<Ipv4Addr>,
    pub disabled: bool,
    /// Why it's a backup WAN, when the config says (or a default implies) so.
    pub backup: Option<String>,
}

impl Wan {
    /// Its address is the scanner's public one: its own, a NAT address in
    /// front of it, or on its block.
    fn holds(&self, ip: Ipv4Addr) -> bool {
        self.addrs.contains(&ip) || self.net.is_some_and(|n| is_public(n.0) && on(ip, n))
    }

    /// The traceroute's first public hop is its gateway or on its block.
    fn next_to(&self, hop: Ipv4Addr) -> bool {
        self.gateway == Some(hop) || self.holds(hop)
    }

    /// An address a public one can be compared with.
    fn comparable(&self) -> bool {
        self.disabled || self.addrs.iter().any(|a| is_public(*a))
    }

    fn line(&self) -> String {
        match self.addrs.first() {
            Some(a) => format!("{} {a}", self.name),
            None => self.name.clone(),
        }
    }
}

/// A firewall (or gateway) and what's on either side of it.
#[derive(Debug, Default)]
struct Fw {
    label: String,
    card: Option<usize>,
    lans: Vec<Lan>,
    wans: Vec<Wan>,
    /// Its WANs may be compared with what this machine sees: the sweep
    /// found its hardware (or, for Meraki, a WAN address matched).
    here: bool,
    /// From a Meraki export, which no other check ties to this site.
    meraki: bool,
}

/// An access point from a config, with no card of its own (Meraki MR).
struct Ap {
    name: String,
    ip: Ipv4Addr,
}

// ---------------------------------------------------------------- UniFi

/// A UniFi gateway: (site, MAC, name, [(group, WAN it last reported)]).
type UGateway = (Option<String>, Mac, String, Vec<(String, Wan)>);

/// What a UniFi backup says about networks, WANs and Wi-Fi, gathered as
/// the Map reads it (`parse_unifi` hands each document here).
#[derive(Default)]
pub(super) struct UnifiTopo {
    lans: Vec<Lan>,
    /// (site, group "WAN"/"WAN2", its WAN from networkconf, failover only).
    wan_confs: Vec<(Option<String>, String, Wan, bool)>,
    gateways: Vec<UGateway>,
    /// (SSID, its network's `_id`, site).
    wlans: Vec<(String, String, Option<String>)>,
}

impl UnifiTopo {
    pub fn append(&mut self, o: UnifiTopo) {
        self.lans.extend(o.lans);
        self.wan_confs.extend(o.wan_confs);
        self.gateways.extend(o.gateways);
        self.wlans.extend(o.wlans);
    }
}

/// One document of a UniFi export, from collection `coll`.
pub(super) fn unifi_doc(t: &mut UnifiTopo, coll: &str, d: &Value) {
    let site = text(d, "site_id");
    match coll {
        "networkconf" if s(d, "purpose") == "wan" => {
            let group = text(d, "wan_networkgroup").unwrap_or_else(|| "WAN".into());
            let ip = addr(d, "wan_ip");
            let bits = text(d, "wan_netmask").and_then(|m| prefix(&m));
            let wan = Wan {
                name: text(d, "name").unwrap_or_else(|| group.clone()),
                addrs: ip.into_iter().collect(),
                net: ip.zip(bits),
                gateway: addr(d, "wan_gateway"),
                disabled: d.get("enabled").and_then(Value::as_bool) == Some(false),
                backup: None,
            };
            // UniFi's default is failover: WAN2 waits for WAN.
            let failover = !matches!(s(d, "wan_load_balance_type"), "weighted");
            t.wan_confs.push((site, group, wan, failover));
        }
        "networkconf" => {
            let enabled = d.get("vlan_enabled").and_then(Value::as_bool) != Some(false);
            t.lans.push(Lan {
                name: text(d, "name").unwrap_or_default(),
                vlan: vlan_of(d, "vlan").filter(|_| enabled),
                net: text(d, "ip_subnet").as_deref().and_then(cidr),
                id: text(d, "_id").unwrap_or_default(),
                site,
            });
        }
        "wlanconf" => {
            if let (Some(ssid), Some(net)) = (text(d, "name"), text(d, "networkconf_id")) {
                t.wlans.push((ssid, net, site));
            }
        }
        "device" if super::is_unifi_gateway(s(d, "type")) => {
            let Some(m) = mac::parse_mac(s(d, "mac")) else {
                return;
            };
            let mut wans = Vec::new();
            for (k, group) in [("wan1", "WAN"), ("wan2", "WAN2")] {
                let Some(w) = d.get(k) else { continue };
                let ip = addr(w, "ip");
                let bits = text(w, "netmask").and_then(|m| prefix(&m));
                if ip.is_none() {
                    continue;
                }
                wans.push((
                    group.to_string(),
                    Wan {
                        name: group.into(),
                        addrs: ip.into_iter().collect(),
                        net: ip.zip(bits),
                        gateway: addr(w, "gateway"),
                        ..Default::default()
                    },
                ));
            }
            let name = text(d, "name").unwrap_or_else(|| "UniFi gateway".into());
            t.gateways.push((site, m, name, wans));
        }
        _ => {}
    }
}

/// Each UniFi gateway as a firewall: its site's networks behind it, its
/// WANs from the networks' WAN settings and what the device last reported.
fn unifi_fws(b: &B, t: &UnifiTopo, here: bool) -> Vec<Fw> {
    let mut out: Vec<Fw> = Vec::new();
    for (site, m, name, seen) in &t.gateways {
        if out
            .iter()
            .any(|f| f.card.is_some() && f.card == b.by_mac.get(m).copied())
        {
            continue;
        }
        let same = |s: &Option<String>| site.is_none() || s.is_none() || s == site;
        let mut wans: Vec<Wan> = Vec::new();
        for (_, group, conf, failover) in t.wan_confs.iter().filter(|c| same(&c.0)) {
            let mut w = conf.clone();
            if let Some((_, dev)) = seen.iter().find(|(g, _)| g == group) {
                for a in &dev.addrs {
                    if !w.addrs.contains(a) {
                        w.addrs.push(*a);
                    }
                }
                w.net = w.net.or(dev.net);
                w.gateway = w.gateway.or(dev.gateway);
            }
            if group != "WAN" && *failover {
                w.backup = Some(format!(
                    "({group}, a failover WAN in UniFi: it carries traffic only while WAN is down)"
                ));
            }
            wans.push(w);
        }
        for (group, dev) in seen {
            if !t.wan_confs.iter().any(|c| same(&c.0) && &c.1 == group) {
                wans.push(dev.clone());
            }
        }
        out.push(Fw {
            label: name.clone(),
            card: b.by_mac.get(m).copied(),
            lans: t
                .lans
                .iter()
                .filter(|l| same(&l.site) && l.net.is_some())
                .cloned()
                .collect(),
            wans,
            here,
            meraki: false,
        });
    }
    out
}

// ------------------------------------------------------------ SonicWall

/// Each SonicWall export as a firewall. Two files of one box land on one
/// card: the one opened later wins.
fn sonic_fws(b: &B, input: &Input, here: bool) -> Vec<Fw> {
    let mut out: Vec<Fw> = Vec::new();
    for f in &input.sonicwall {
        let Ok(v) = serde_json::from_str::<Value>(f.json) else {
            continue;
        };
        let sys = v.get("system").and_then(Value::as_object);
        let pick = |words: &[&str]| {
            sys.and_then(|m| {
                m.iter()
                    .find(|(k, _)| {
                        let k = k.to_ascii_lowercase();
                        words.iter().any(|w| k.contains(w))
                    })
                    .and_then(|(_, v)| v.as_str())
                    .map(str::to_string)
            })
        };
        let label = pick(&["firewallname", "prodname"])
            .or_else(|| pick(&["model"]))
            .unwrap_or_else(|| "SonicWall".into());
        let mut fw = Fw {
            label,
            here,
            ..Default::default()
        };
        for i in list(&v, "interfaces") {
            let ip = addr(i, "ip");
            let bits = text(i, "mask").and_then(|m| prefix(&m));
            let card = ip
                .and_then(|ip| b.by_ip.get(&ip))
                .or_else(|| mac::parse_mac(s(i, "mac")).and_then(|m| b.by_mac.get(&m)))
                .copied()
                .filter(|&c| b.nodes[c].zone == Zone::Gateway);
            fw.card = fw.card.or(card);
            let (name, zone) = (s(i, "name"), s(i, "zone"));
            if zone.to_ascii_lowercase().contains("wan") {
                fw.wans.push(Wan {
                    name: name.to_string(),
                    addrs: ip.into_iter().collect(),
                    net: ip.zip(bits),
                    gateway: addr(i, "gateway"),
                    disabled: i.get("disabled").and_then(Value::as_bool) == Some(true),
                    backup: None,
                });
            } else if let (Some(ip), Some(bits)) = (ip, bits) {
                // A zone or comment naming voice names the network.
                let comment = s(i, "comment");
                let name = if says_voice(zone) || !says_voice(comment) {
                    format!("{name} {zone}")
                } else {
                    format!("{name} {zone} ({comment})")
                };
                fw.lans.push(Lan {
                    name: name.trim().to_string(),
                    vlan: vlan_of(i, "vlan"),
                    net: Some((ip, bits)),
                    ..Default::default()
                });
            }
        }
        // SonicOS fails over in its load-balancing group's order, the
        // first WAN (X1) first by default. The export doesn't hold that
        // order, so the rest are called backups only as a default.
        let live: Vec<usize> = (0..fw.wans.len())
            .filter(|&k| !fw.wans[k].disabled && !fw.wans[k].addrs.is_empty())
            .collect();
        if let [first, rest @ ..] = live.as_slice() {
            let primary = fw.wans[*first].name.clone();
            for &k in rest {
                fw.wans[k].backup = Some(format!(
                    "(SonicOS fails over in its Failover & LB group's order, {primary} first by \
                     default; the export doesn't hold that order, so check it there)"
                ));
            }
        }
        match out
            .iter_mut()
            .find(|o| o.card.is_some() && o.card == fw.card)
        {
            Some(o) => *o = fw,
            None => out.push(fw),
        }
    }
    out
}

// --------------------------------------------------------------- Meraki

/// A Meraki security appliance: MX, Z (teleworker) or vMX.
fn is_mx(model: &str) -> bool {
    let m = model.to_ascii_uppercase();
    m.starts_with("MX") || m.starts_with('Z') || m.starts_with("VMX")
}

/// Each Meraki network with an appliance as a firewall, its access points,
/// and every device MAC (for whether the sweep is at this site).
fn meraki_fws(b: &B, input: &Input) -> (Vec<Fw>, Vec<Ap>) {
    let mut fws: Vec<Fw> = Vec::new();
    let mut aps: Vec<Ap> = Vec::new();
    let swept: HashSet<Mac> = input.sweep.iter().filter_map(|h| h.mac).collect();
    for f in &input.meraki {
        let Ok(v) = serde_json::from_str::<Value>(f.json) else {
            continue;
        };
        let devices = list(&v, "devices");
        // Networks by name; one with no name is the file's only one.
        let mut nets: Vec<String> = Vec::new();
        for row in devices
            .iter()
            .chain(list(&v, "vlans"))
            .chain(list(&v, "uplinks"))
        {
            let n = s(row, "network").to_string();
            if !nets.contains(&n) {
                nets.push(n);
            }
        }
        for net in nets {
            let mine = |row: &&Value| s(row, "network") == net;
            let mx: Vec<&Value> = devices
                .iter()
                .filter(mine)
                .filter(|d| is_mx(s(d, "model")) || s(d, "productType") == "appliance")
                .collect();
            for d in devices.iter().filter(mine) {
                let wireless = s(d, "productType") == "wireless"
                    || ["MR", "CW"].iter().any(|p| s(d, "model").starts_with(p));
                if let (true, Some(ip)) = (wireless, addr(d, "lanIp")) {
                    aps.push(Ap {
                        name: match s(d, "name") {
                            "" => s(d, "model").to_string(),
                            n => n.to_string(),
                        },
                        ip,
                    });
                }
            }
            let lans: Vec<Lan> = list(&v, "vlans")
                .iter()
                .filter(mine)
                .map(|l| Lan {
                    name: s(l, "name").to_string(),
                    vlan: vlan_of(l, "id"),
                    net: text(l, "subnet")
                        .or_else(|| text(l, "cidr"))
                        .as_deref()
                        .and_then(cidr),
                    ..Default::default()
                })
                .collect();
            let status: Vec<&Value> = list(&v, "uplink_status").iter().filter(mine).collect();
            let mut wans: Vec<Wan> = Vec::new();
            for u in list(&v, "uplinks").iter().filter(mine) {
                let net = text(u, "address").as_deref().and_then(cidr);
                wans.push(Wan {
                    name: s(u, "interface").to_string(),
                    addrs: net.map(|n| n.0).into_iter().collect(),
                    net,
                    gateway: addr(u, "gateway"),
                    disabled: u.get("enabled").and_then(Value::as_bool) == Some(false),
                    backup: None,
                });
            }
            for st in &status {
                let name = s(st, "interface");
                let at = match wans.iter().position(|w| w.name == name) {
                    Some(k) => k,
                    None => {
                        wans.push(Wan {
                            name: name.to_string(),
                            ..Default::default()
                        });
                        wans.len() - 1
                    }
                };
                let w = &mut wans[at];
                for a in [addr(st, "ip"), addr(st, "publicIp")].into_iter().flatten() {
                    if !w.addrs.contains(&a) {
                        w.addrs.push(a);
                    }
                }
                w.gateway = w.gateway.or(addr(st, "gateway"));
            }
            for d in &mx {
                for (k, name) in [("wan1Ip", "wan1"), ("wan2Ip", "wan2")] {
                    if let (Some(a), Some(w)) =
                        (addr(d, k), wans.iter_mut().find(|w| w.name == name))
                        && !w.addrs.contains(&a)
                    {
                        w.addrs.push(a);
                    }
                }
            }
            // wan1 is the primary uplink. Both active is load balancing:
            // neither is a backup then.
            let active = |name: &str| {
                status
                    .iter()
                    .any(|st| s(st, "interface") == name && s(st, "status") == "active")
            };
            let balanced = active("wan1") && active("wan2");
            if wans.iter().any(|w| w.name == "wan1") && !balanced {
                let wan1 = status
                    .iter()
                    .find(|st| s(st, "interface") == "wan1")
                    .map(|st| s(st, "status"))
                    .filter(|st| !st.is_empty());
                for w in wans.iter_mut().filter(|w| w.name != "wan1") {
                    w.backup = Some(match wan1 {
                        Some(st) => format!("(the MX's secondary uplink; wan1 reads \"{st}\")"),
                        None => "(the MX's secondary uplink)".into(),
                    });
                }
            }
            if mx.is_empty() && wans.is_empty() {
                continue;
            }
            let macs: Vec<Mac> = devices
                .iter()
                .filter(mine)
                .filter_map(|d| mac::parse_mac(s(d, "mac")))
                .collect();
            let card = mx
                .iter()
                .filter_map(|d| mac::parse_mac(s(d, "mac")))
                .find_map(|m| b.by_mac.get(&m))
                .or_else(|| {
                    lans.iter()
                        .filter_map(|l| l.net)
                        .find_map(|n| b.by_ip.get(&n.0))
                })
                .copied()
                .filter(|&c| b.nodes[c].zone == Zone::Gateway);
            let here = macs.iter().any(|m| swept.contains(m))
                || input.wan.is_some_and(|w| wans.iter().any(|x| x.holds(w)));
            let label = mx
                .first()
                .map(|d| match s(d, "name") {
                    "" => format!("Meraki {}", s(d, "model")),
                    n => n.to_string(),
                })
                .unwrap_or_else(|| "Meraki MX".into());
            fws.push(Fw {
                label: if net.is_empty() || label.contains(&net) {
                    label
                } else {
                    format!("{label} ({net})")
                },
                card,
                lans,
                wans,
                here,
                meraki: true,
            });
        }
    }
    (fws, aps)
}

// ------------------------------------------------------------- findings

/// Every topology finding the data supports, on the map being built.
/// `local`: what this machine sees belongs to this map; `internet`: its
/// card.
pub(super) fn findings(
    b: &mut B,
    input: &Input,
    unifi: Option<&UnifiTopo>,
    local: bool,
    internet: usize,
) {
    let mut fws = sonic_fws(b, input, local);
    if fws.is_empty()
        && let Some(t) = unifi
    {
        fws.extend(unifi_fws(b, t, local));
    }
    let (meraki, meraki_aps) = meraki_fws(b, input);
    let meraki_here = meraki.iter().any(|f| f.here);
    fws.extend(meraki);
    voice(b, input, &fws, unifi);
    leaves_by(b, input, &fws, internet);
    wifi_outside(b, &fws, unifi, if meraki_here { &meraki_aps } else { &[] });
}

/// Phones on another subnet than the voice network.
fn voice(b: &mut B, input: &Input, fws: &[Fw], unifi: Option<&UnifiTopo>) {
    let lans = fws
        .iter()
        .filter(|f| f.here || !f.meraki)
        .flat_map(|f| f.lans.iter())
        .chain(unifi.into_iter().flat_map(|t| t.lans.iter()));
    let mut voice: Vec<(String, Net)> = Vec::new();
    for l in lans {
        let Some(net) = l.net else { continue };
        let by_name = says_voice(&l.name);
        let by_port = input.voice_vlan.is_some() && l.vlan == input.voice_vlan;
        if !(by_name || by_port) || voice.iter().any(|v| net_text(v.1) == net_text(net)) {
            continue;
        }
        let mut line = l.line();
        if by_port && !by_name {
            line += ", the voice VLAN the scanner's switch port announces";
        }
        voice.push((line, net));
    }
    phones_outside(b, &voice, &[]);
}

/// A /mapnetwork map's voice zones: a zone named for voice, with the
/// subnet its caption gives ("VLAN 40 · 10.40.0.0/24", "10.40.0.x").
pub(super) fn voice_on_map(b: &mut B, doc: &crate::mapnetwork::Doc) {
    let mut voice: Vec<(String, Net)> = Vec::new();
    let mut zones: Vec<String> = Vec::new();
    for z in doc.zones.values() {
        let (name, caption) = (s(z, "name"), s(z, "vlan"));
        if !says_voice(name) && !says_voice(caption) {
            continue;
        }
        zones.push(name.to_string());
        let line = match caption {
            "" => name.to_string(),
            c => format!("{name} ({c})"),
        };
        for w in caption.split(|c: char| c.is_whitespace() || c == ',' || c == '·') {
            let net = cidr(w).or_else(|| {
                let w = w.strip_suffix(".x")?;
                let base: Ipv4Addr = format!("{w}.0").parse().ok()?;
                Some((base, 24))
            });
            if let Some(n) = net {
                voice.push((line.clone(), n));
            }
        }
    }
    phones_outside(b, &voice, &zones);
}

/// The finding: phones whose address is on none of the `voice` networks
/// (or, with no subnet known, whose map zone isn't one of `zones`).
fn phones_outside(b: &mut B, voice: &[(String, Net)], zones: &[String]) {
    if voice.is_empty() && zones.is_empty() {
        return;
    }
    let out: Vec<usize> = (0..b.nodes.len())
        .filter(|&i| {
            let n = &b.nodes[i];
            // A /mapnetwork map that put it in a voice zone placed it there.
            let in_zone = zones.iter().any(|z| {
                n.details
                    .iter()
                    .any(|d| d.starts_with(&format!("Zone on the map: {z}")))
            });
            if !is_phone(n) || in_zone {
                return false;
            }
            match n.ip {
                Some(ip) if !voice.is_empty() => !voice.iter().any(|v| on(ip, v.1)),
                _ => voice.is_empty(),
            }
        })
        .collect();
    if out.is_empty() {
        return;
    }
    let mut names: Vec<&str> = Vec::new();
    for (l, _) in voice {
        if !names.contains(&l.as_str()) {
            names.push(l);
        }
    }
    if names.is_empty() {
        names = zones.iter().map(String::as_str).collect();
    }
    for &i in &out {
        let n = &mut b.nodes[i];
        let flag = "Outside the voice VLAN".to_string();
        if !n.flags.contains(&flag) {
            n.flags.push(flag);
        }
    }
    let text = format!(
        "{} phone(s) outside the voice network ({}): {}. Most often plugged into a data \
         port, or behind an unmanaged switch, so they get no voice VLAN tag and no LLDP-MED \
         voice policy: move them to a port with the voice VLAN, or set the VLAN on the phone.",
        out.len(),
        names.join("; "),
        describe_some(&b.nodes, &out)
    );
    // A map is-alive saved holds the finding already: once is enough.
    if !b.findings.iter().any(|f| f.1 == text) {
        b.finding(Sev::Med, text, &out);
    }
}

/// Which WAN this LAN leaves by: marked on the firewall's card, with a
/// finding when it's a backup WAN or none of them.
fn leaves_by(b: &mut B, input: &Input, fws: &[Fw], internet: usize) {
    let here: Vec<&Fw> = fws
        .iter()
        .filter(|f| f.here && !f.wans.is_empty())
        .collect();
    if here.is_empty() {
        return;
    }
    let (public, hop) = (input.wan, input.trace_hop.filter(|h| is_public(*h)));
    let find = |test: &dyn Fn(&Wan) -> bool| {
        here.iter().find_map(|f| {
            f.wans
                .iter()
                .find(|w| !w.disabled && test(w))
                .map(|w| (*f, w))
        })
    };
    let by_ip = public.and_then(|p| find(&|w| w.holds(p)));
    let by_hop = hop.and_then(|h| find(&|w| w.next_to(h)));
    let found = by_ip
        .map(|m| {
            (
                m,
                format!(
                    "the scanner's public address {}",
                    public.unwrap_or(m.1.addrs[0])
                ),
            )
        })
        .or_else(|| {
            by_hop.map(|m| {
                (
                    m,
                    format!(
                        "the traceroute's first public hop {}",
                        hop.unwrap_or(m.1.addrs[0])
                    ),
                )
            })
        });
    if let Some(((f, w), why)) = found {
        let line = format!("This LAN leaves by {} ({why})", w.line());
        let at = f.card.unwrap_or(internet);
        let n = &mut b.nodes[at];
        n.detail(line.clone());
        if f.card.is_some()
            && let Some(up) = &mut n.up
            && up.0 == internet
            && up.1 == Link::Backbone
        {
            up.2 = Some(format!("LAN leaves by {}", w.name));
        }
        if at != internet {
            b.nodes[internet].detail(format!("{line}, on {}", f.label));
        }
        if let Some(reason) = &w.backup {
            let primary: Vec<String> = f
                .wans
                .iter()
                .filter(|x| x.backup.is_none() && !x.disabled)
                .map(Wan::line)
                .collect();
            let cards: Vec<usize> = f.card.into_iter().chain([internet]).collect();
            if let Some(c) = f.card {
                b.nodes[c]
                    .flags
                    .push(format!("Running on backup WAN {}", w.name));
            }
            b.finding(
                Sev::Med,
                format!(
                    "This LAN leaves by {}'s {}, its backup WAN {reason}: {} may be down, \
                     failing or not preferred. Check the primary link and the failover settings.",
                    f.label,
                    w.line(),
                    match primary.as_slice() {
                        [] => "the primary".to_string(),
                        p => format!("the primary ({})", p.join(", ")),
                    }
                ),
                &cards,
            );
        }
        return;
    }
    let Some(public) = public else { return };
    // Only when every WAN's address is known and public: a WAN on DHCP
    // with no address in the config, or behind the ISP's router (double
    // NAT), could be the one.
    if !here.iter().all(|f| f.wans.iter().all(Wan::comparable)) {
        return;
    }
    let all: Vec<String> = here
        .iter()
        .flat_map(|f| {
            f.wans
                .iter()
                .filter(|w| !w.disabled)
                .map(move |w| format!("{} {}", f.label, w.line()))
        })
        .collect();
    let cards: Vec<usize> = here
        .iter()
        .filter_map(|f| f.card)
        .chain([internet])
        .collect();
    let hop_note = hop
        .map(|h| format!(" (the traceroute's first public hop, {h}, is on none of them either)"))
        .unwrap_or_default();
    b.finding(
        Sev::Med,
        format!(
            "The scanner's public address {public} is on none of the firewall's WANs ({}){hop_note}: \
             this LAN leaves by another router or the ISP's gateway, not through the firewall, \
             or the scanner itself uses a VPN or cloud proxy. Find the second way out.",
            all.join(", ")
        ),
        &cards,
    );
}

/// Access points and Wi-Fi networks on a subnet no firewall serves.
fn wifi_outside(b: &mut B, fws: &[Fw], unifi: Option<&UnifiTopo>, meraki_aps: &[Ap]) {
    let lans: Vec<Net> = fws
        .iter()
        .flat_map(|f| f.lans.iter().filter_map(|l| l.net))
        .chain(b.gws.iter().flat_map(|g| g.lans.iter().copied()))
        .collect();
    if lans.is_empty() {
        return;
    }
    // The networks on the WAN side: the ISP router's LAN, where a WAN
    // sits on a private block.
    let wan_side: Vec<(&str, &Wan, Net)> = fws
        .iter()
        .flat_map(|f| {
            f.wans
                .iter()
                .filter_map(move |w| Some((f.label.as_str(), w, w.net?)))
        })
        .collect();
    let behind = |ip: Ipv4Addr| lans.iter().any(|&n| on(ip, n));
    let wan_of = |ip: Ipv4Addr| wan_side.iter().find(|w| on(ip, w.2));
    let mut lines: Vec<String> = Vec::new();
    let mut cards: Vec<usize> = Vec::new();
    // Only past what's evidently another site: a source counts when some
    // of its devices are behind the firewall, or the address is on a WAN.
    let unifi_cards: Vec<usize> = (0..b.nodes.len())
        .filter(|&i| b.nodes[i].seen_by.contains(&"UniFi") && b.nodes[i].ip.is_some())
        .collect();
    let unifi_overlaps = unifi_cards
        .iter()
        .any(|&i| b.nodes[i].ip.is_some_and(behind));
    let why = |ip: Ipv4Addr| match wan_of(ip) {
        Some((fw, w, n)) => Some(format!(
            "on {fw}'s WAN side ({}'s network {}, the ISP router's LAN)",
            w.name,
            net_text(*n)
        )),
        None if !behind(ip) => Some("on a subnet no firewall interface serves".to_string()),
        None => None,
    };
    for &i in &unifi_cards {
        let n = &b.nodes[i];
        let Some(ip) = n.ip else { continue };
        if n.zone != Zone::Wireless || !(unifi_overlaps || wan_of(ip).is_some()) {
            continue;
        }
        if let Some(w) = why(ip) {
            lines.push(format!("{} {w}", describe(n)));
            cards.push(i);
        }
    }
    let meraki_overlaps = meraki_aps.iter().any(|a| behind(a.ip));
    for a in meraki_aps {
        if !(meraki_overlaps || wan_of(a.ip).is_some()) {
            continue;
        }
        if let Some(w) = why(a.ip) {
            lines.push(format!("{} ({}) {w}", a.name, a.ip));
        }
    }
    if let Some(t) = unifi {
        // With a UniFi gateway as the firewall its own networks are its
        // LANs; this is for a SonicWall or Meraki in front of UniFi Wi-Fi.
        for (ssid, id, _) in &t.wlans {
            let Some(net) = t.lans.iter().find(|l| &l.id == id).and_then(|l| l.net) else {
                continue;
            };
            if !(unifi_overlaps || wan_of(net.0).is_some()) {
                continue;
            }
            if let Some(w) = why(net.0) {
                let line = format!("Wi-Fi \"{ssid}\" ({}) {w}", net_text(net));
                if !lines.contains(&line) {
                    lines.push(line);
                }
            }
        }
    }
    if lines.is_empty() {
        return;
    }
    for &i in &cards {
        b.nodes[i].flags.push("Outside the firewall".into());
    }
    b.finding(
        Sev::High,
        format!(
            "Wi-Fi outside the firewall: {}. Its traffic reaches the internet without the \
             firewall's rules and filtering. Put it behind the firewall (a VLAN on a firewall \
             interface), or confirm the subnet is routed through it.",
            lines.join("; ")
        ),
        &cards,
    );
}

#[cfg(test)]
mod tests {
    use super::super::{Input, Loaded, build, one};
    use super::*;

    fn seen(ip: &str, mac: &str) -> super::super::Seen {
        super::super::Seen {
            ip: ip.parse().unwrap(),
            mac: mac::parse_mac(mac),
            name: None,
            rtt: Some(1.0),
            open: Vec::new(),
            checked: false,
            also: Vec::new(),
            web: None,
            multicast: None,
            dnssd: None,
            imported: None,
        }
    }

    /// A SonicWall with a LAN, a voice VLAN and two WANs (X1, X2).
    const SONIC: &str = r#"{"system": {"firewallName": "Edge"},
      "interfaces": [
        {"name": "X0", "zone": "LAN", "ip": "192.0.2.1", "mask": "255.255.255.0", "mac": "c0:ea:e4:00:00:01"},
        {"name": "X1", "zone": "WAN", "ip": "203.0.113.2", "mask": "255.255.255.248", "gateway": "203.0.113.1"},
        {"name": "X2", "zone": "WAN", "ip": "198.51.100.10", "mask": "255.255.255.0", "gateway": "198.51.100.1"},
        {"name": "X0:V40", "zone": "Voice", "vlan": "40", "ip": "10.40.0.1", "mask": "255.255.255.0"}
      ]}"#;

    fn input<'a>(sweep: Vec<super::super::Seen>, files: &'a [Loaded<'a>]) -> Input<'a> {
        Input {
            sweep,
            gateway: Some("192.0.2.1".parse().unwrap()),
            this: Some("192.0.2.50".parse().unwrap()),
            sonicwall: files.to_vec(),
            ..Default::default()
        }
    }

    fn fw_mac() -> super::super::Seen {
        seen("192.0.2.1", "c0:ea:e4:00:00:01")
    }

    #[test]
    fn a_phone_on_the_data_lan_is_outside_the_voice_vlan() {
        let files = one(SONIC);
        // Yealink's registered prefix, on the data LAN; another on voice.
        let mut i = input(
            vec![
                fw_mac(),
                seen("192.0.2.77", "80:5e:c0:00:00:01"),
                seen("10.40.0.20", "80:5e:c0:00:00:02"),
            ],
            &files,
        );
        i.clients = super::super::Clients::All;
        let m = build(&i);
        let f = m
            .findings
            .iter()
            .find(|f| f.text.contains("outside the voice network"))
            .expect("a voice finding");
        assert_eq!(f.sev, Sev::Med);
        assert!(
            f.text.contains("VLAN 40 X0:V40 Voice (10.40.0.0/24)"),
            "{}",
            f.text
        );
        assert!(f.text.contains("192.0.2.77") && !f.text.contains("10.40.0.20"));
        assert!(f.text.contains("unmanaged switch") && f.text.contains("LLDP-MED"));
        assert_eq!(f.nodes.len(), 1);
    }

    #[test]
    fn no_voice_network_known_says_nothing() {
        let plain = SONIC.replace("\"Voice\"", "\"LAN\"");
        let files = one(&plain);
        let m = build(&input(
            vec![fw_mac(), seen("192.0.2.77", "80:5e:c0:00:00:01")],
            &files,
        ));
        assert!(!m.findings.iter().any(|f| f.text.contains("voice")));
    }

    #[test]
    fn the_switch_ports_voice_vlan_names_the_voice_network() {
        let plain = SONIC.replace("\"Voice\"", "\"LAN\"");
        let files = one(&plain);
        let mut i = input(
            vec![fw_mac(), seen("192.0.2.77", "80:5e:c0:00:00:01")],
            &files,
        );
        i.voice_vlan = Some(40);
        let m = build(&i);
        let f = m
            .findings
            .iter()
            .find(|f| f.text.contains("outside the voice network"))
            .expect("a voice finding");
        assert!(f.text.contains("switch port announces"), "{}", f.text);
    }

    #[test]
    fn the_wan_the_lan_leaves_by_is_marked() {
        let files = one(SONIC);
        let mut i = input(vec![fw_mac()], &files);
        i.wan = Some("203.0.113.2".parse().unwrap());
        let m = build(&i);
        let gw = m.nodes.iter().find(|n| n.id == "gateway").unwrap();
        assert_eq!(
            gw.up.as_ref().unwrap().2.as_deref(),
            Some("LAN leaves by X1")
        );
        assert!(
            gw.details
                .iter()
                .any(|d| d.starts_with("This LAN leaves by X1 203.0.113.2"))
        );
        assert!(
            !m.findings.iter().any(|f| f.text.contains("WAN")),
            "X1 is the primary"
        );
    }

    #[test]
    fn leaving_by_the_backup_wan_is_a_finding() {
        let files = one(SONIC);
        let mut i = input(vec![fw_mac()], &files);
        i.wan = Some("198.51.100.10".parse().unwrap());
        let m = build(&i);
        let f = m
            .findings
            .iter()
            .find(|f| f.text.contains("backup WAN"))
            .expect("a backup finding");
        assert!(f.text.contains("X2 198.51.100.10") && f.text.contains("X1 203.0.113.2"));
    }

    #[test]
    fn the_first_public_hop_places_the_wan_when_the_address_doesnt() {
        let files = one(SONIC);
        let mut i = input(vec![fw_mac()], &files);
        i.trace_hop = Some("203.0.113.1".parse().unwrap());
        let m = build(&i);
        let gw = m.nodes.iter().find(|n| n.id == "gateway").unwrap();
        assert_eq!(
            gw.up.as_ref().unwrap().2.as_deref(),
            Some("LAN leaves by X1")
        );
    }

    #[test]
    fn a_public_address_on_no_wan_is_a_finding() {
        let files = one(SONIC);
        let mut i = input(vec![fw_mac()], &files);
        i.wan = Some("203.0.113.99".parse().unwrap());
        let m = build(&i);
        let f = m
            .findings
            .iter()
            .find(|f| f.text.contains("on none of the firewall's WANs"))
            .expect("a finding");
        assert!(f.text.contains("another router or the ISP's gateway"));
        // Not when a WAN's address isn't known (DHCP): it could be that one.
        let dhcp = SONIC.replace(r#""ip": "198.51.100.10", "#, "");
        let files = one(&dhcp);
        let mut i = input(vec![fw_mac()], &files);
        i.wan = Some("203.0.113.99".parse().unwrap());
        assert!(
            !build(&i)
                .findings
                .iter()
                .any(|f| f.text.contains("none of"))
        );
        // Nor read far from the site: no sweep found the firewall.
        let files = one(SONIC);
        let mut i = input(Vec::new(), &files);
        i.wan = Some("203.0.113.99".parse().unwrap());
        assert!(
            !build(&i)
                .findings
                .iter()
                .any(|f| f.text.contains("none of"))
        );
    }

    const UNIFI: &str = r#"[
      {"__cmd": "select", "collection": "site"}, {"_id": "s1", "name": "default", "desc": "Main"},
      {"__cmd": "select", "collection": "device"},
      {"mac": "f0:9f:c2:00:00:01", "type": "uap", "model": "U6LR", "name": "AP-Lobby", "ip": "192.0.2.20", "site_id": "s1"},
      {"mac": "f0:9f:c2:00:00:02", "type": "uap", "model": "U6LR", "name": "AP-Yard", "ip": "192.168.1.20", "site_id": "s1"},
      {"__cmd": "select", "collection": "networkconf"},
      {"_id": "n1", "name": "Staff", "purpose": "vlan-only", "vlan": 10, "site_id": "s1"},
      {"_id": "n2", "name": "Guest", "purpose": "corporate", "ip_subnet": "192.168.1.1/24", "site_id": "s1"},
      {"__cmd": "select", "collection": "wlanconf"},
      {"name": "Guest Wi-Fi", "networkconf_id": "n2", "site_id": "s1"}
    ]"#;

    #[test]
    fn an_ap_on_the_isp_routers_lan_is_outside_the_firewall() {
        // X2's WAN is on the ISP router's LAN, 192.168.1.0/24.
        let sonic = SONIC.replace("198.51.100.10", "192.168.1.2").replace(
            r#""gateway": "198.51.100.1""#,
            r#""gateway": "192.168.1.1""#,
        );
        let files = one(&sonic);
        let mut i = input(vec![fw_mac()], &files);
        i.unifi = one(UNIFI);
        let m = build(&i);
        let f = m
            .findings
            .iter()
            .find(|f| f.text.starts_with("Wi-Fi outside the firewall"))
            .expect("a finding");
        assert_eq!(f.sev, Sev::High);
        assert!(
            f.text.contains("AP-Yard (192.168.1.20) on Edge's WAN side"),
            "{}",
            f.text
        );
        assert!(f.text.contains("Wi-Fi \"Guest Wi-Fi\" (192.168.1.0/24)"));
        assert!(!f.text.contains("AP-Lobby"));
        assert_eq!(f.nodes.len(), 1);
    }

    #[test]
    fn a_unifi_backup_of_another_site_isnt_outside_this_firewall() {
        // None of its devices is behind this firewall, nor on its WAN side.
        let files = one(SONIC);
        let mut i = input(vec![fw_mac()], &files);
        let other = UNIFI.replace("192.0.2.20", "172.16.9.20");
        i.unifi = one(&other);
        let m = build(&i);
        assert!(
            !m.findings
                .iter()
                .any(|f| f.text.starts_with("Wi-Fi outside"))
        );
    }

    const MERAKI: &str = r#"{"devices": [
        {"name": "MX-Edge", "model": "MX68", "mac": "e0:55:3d:00:00:01", "wan1Ip": "203.0.113.42", "network": "Branch", "productType": "appliance"},
        {"name": "MR-Hall", "model": "MR36", "mac": "e0:55:3d:00:00:02", "lanIp": "192.168.0.30", "network": "Branch", "productType": "wireless"},
        {"name": "MR-Desk", "model": "MR36", "mac": "e0:55:3d:00:00:03", "lanIp": "10.20.0.31", "network": "Branch", "productType": "wireless"}],
      "vlans": [{"id": 1, "name": "Data", "subnet": "10.20.0.0/24", "applianceIp": "10.20.0.1", "network": "Branch"},
                {"id": 30, "name": "VoIP", "subnet": "10.20.3.0/24", "applianceIp": "10.20.3.1", "network": "Branch"}],
      "uplinks": [{"interface": "wan1", "address": "203.0.113.42/29", "gateway": "203.0.113.41", "network": "Branch"},
                  {"interface": "wan2", "address": "198.51.100.7/24", "gateway": "198.51.100.1", "network": "Branch"}],
      "uplink_status": [{"interface": "wan1", "status": "failed", "network": "Branch"},
                        {"interface": "wan2", "status": "active", "ip": "198.51.100.7", "publicIp": "198.51.100.7", "network": "Branch"}]}"#;

    #[test]
    fn meraki_on_its_secondary_uplink_with_an_ap_outside() {
        let files = one(MERAKI);
        let i = Input {
            sweep: vec![
                seen("10.20.0.1", "e0:55:3d:00:00:01"),
                seen("10.20.0.60", "80:5e:c0:00:00:09"),
            ],
            gateway: Some("10.20.0.1".parse().unwrap()),
            wan: Some("198.51.100.7".parse().unwrap()),
            meraki: files.clone(),
            ..Default::default()
        };
        let m = build(&i);
        let texts: Vec<&str> = m.findings.iter().map(|f| f.text.as_str()).collect();
        assert!(
            texts
                .iter()
                .any(|t| t.contains("wan2 198.51.100.7, its backup WAN")
                    && t.contains("wan1 reads \"failed\"")),
            "{texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|t| t.contains("phone(s) outside the voice network (VLAN 30 VoIP"))
        );
        assert!(
            texts
                .iter()
                .any(|t| t.contains("MR-Hall (192.168.0.30) on a subnet no firewall"))
        );
        assert!(!texts.iter().any(|t| t.contains("MR-Desk")));
    }

    #[test]
    fn a_meraki_export_read_far_away_isnt_compared() {
        let files = one(MERAKI);
        let i = Input {
            sweep: vec![seen("192.0.2.9", "00:11:22:00:00:09")],
            wan: Some("192.0.2.200".parse().unwrap()),
            meraki: files.clone(),
            ..Default::default()
        };
        let m = build(&i);
        assert!(m.findings.is_empty(), "{:?}", m.findings);
    }

    #[test]
    fn a_mapnetwork_voice_zone_flags_phones_elsewhere() {
        let page = r#"<html><script>
// ==== SITE DATA ====
this.SITES = [];
this.ZONES = { data: { name: 'Data', vlan: 'VLAN 1 · 10.1.0.x' }, voice: { name: 'Voice', vlan: 'VLAN 40 · 10.40.0.0/24' } };
this.NODES = [
  { id: 'fw', zone: 'data', kind: 'FIREWALL', label: 'Edge', sub: '10.1.0.1' },
  { id: 'p1', zone: 'data', kind: 'YEALINK PHONE', label: 'Front desk', sub: '10.1.0.77' },
  { id: 'p2', zone: 'voice', kind: 'YEALINK PHONE', label: 'Back office', sub: '10.40.0.8' },
  { id: 'pbx', zone: 'data', kind: 'PHONE SYSTEM', label: 'PBX', sub: '10.1.0.5' },
  { id: 'p3', zone: 'voice', kind: 'PHONE', label: 'Annex', sub: '10.9.0.3' } ];
this.EDGES = [ { from: 'fw', to: 'p1' }, { from: 'fw', to: 'p2' }, { from: 'fw', to: 'pbx' }, { from: 'fw', to: 'p3' } ];
this.ACTIONS = [];
// ==== END SITE DATA ====
</script></html>"#;
        let doc = crate::mapnetwork::parse(page.as_bytes()).expect("a map");
        let m = build(&Input {
            mapnetwork: Some(&doc),
            ..Default::default()
        });
        let f = m
            .findings
            .iter()
            .find(|f| f.text.contains("outside the voice network"))
            .expect("a finding");
        assert!(f.text.contains("Front desk (10.1.0.77)") && !f.text.contains("Back office"));
        // Not a PBX, nor a phone the map itself put in the voice zone.
        assert!(
            !f.text.contains("PBX") && !f.text.contains("Annex"),
            "{}",
            f.text
        );
        // A map is-alive saved holds it as one of its own: not twice.
        let again = page.replace(
            "this.ACTIONS = [];",
            &format!("this.ACTIONS = [ {{ sev: 'med', text: {:?} }} ];", f.text),
        );
        let doc = crate::mapnetwork::parse(again.as_bytes()).expect("a map");
        let m = build(&Input {
            mapnetwork: Some(&doc),
            ..Default::default()
        });
        assert_eq!(m.findings.len(), 1, "{:?}", m.findings);
    }

    #[test]
    fn public_addresses() {
        for a in ["203.0.113.5", "198.51.100.1", "8.8.8.8"] {
            assert!(is_public(a.parse().unwrap()), "{a}");
        }
        for a in [
            "10.0.0.1",
            "192.168.1.1",
            "100.64.0.1",
            "169.254.1.1",
            "127.0.0.1",
        ] {
            assert!(!is_public(a.parse().unwrap()), "{a}");
        }
    }
}
