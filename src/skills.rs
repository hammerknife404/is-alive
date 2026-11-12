//! The /unifidump and /mapnetwork skills, embedded (from `skills/`, which
//! `scripts/sync-skills.py` copies from the source skills and scrubs).
//!
//! A Site Bundle carries them so a Claude chat without the skills installed
//! can still follow them, and their two worked example maps open in the Map
//! tab as examples, with a home network and a blank map built from the
//! skill's template.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/skills.rs"));

/// The skills a bundle carries, by folder name.
pub const NAMES: [&str; 2] = ["unifidump", "mapnetwork"];

/// The example maps: (menu label, file under skills/, or `HOME` or
/// `BLANK`). The blank one stays last.
pub const EXAMPLES: [(&str, &str); 4] = [
    (
        "Acme Distribution: one campus, HA firewalls, dual WAN",
        "mapnetwork/assets/example-Acme_Distribution_LLC.dc.html",
    ),
    (
        "Contoso Health: 23 clinics, 81 devices",
        "mapnetwork/assets/example-Contoso_Health.dc.html",
    ),
    (HOME_LABEL, HOME),
    (BLANK_LABEL, BLANK),
];

/// The home example: not a file under skills/ but built from the
/// /mapnetwork template like the blank map. A small network to learn the
/// Map on, with links assumed (a switch nothing can scan) and the issues
/// a home or small office usually has.
pub const HOME: &str = "Home_network.dc.html";
pub const HOME_LABEL: &str = "Home: a router, mesh Wi-Fi and the usual issues";

/// The home map's site data: fictional, in the template's own format.
const HOME_DATA: &str = r#"    // HOME NETWORK: a fictional house. Dashed wired links are inferred (the
    // switch behind the TV is unmanaged: nothing to scan); dashed Wi-Fi
    // links are wireless.
    this.META = {
      title: 'Home — Network',
      subtitle: '192.168.1.0/24 · ISP modem, Wi-Fi router, one mesh node · a few clients mapped · fictional'
    };

    this.ZONES = {
      edge:    { name:'Internet & routers', vlan:'WAN · NAT',          dark:'#5b9dff', light:'#2563c9' },
      gear:    { name:'Switch & mesh',      vlan:'wired · backhaul',   dark:'#a98bff', light:'#6d44d9' },
      family:  { name:'Family clients',     vlan:'main Wi-Fi',         dark:'#5fcf8a', light:'#1c9b54' },
      devices: { name:'NAS & printer',      vlan:'main Wi-Fi · wired', dark:'#e0b34d', light:'#a9790c' },
      iot:     { name:'Smart home · IoT',   vlan:'main Wi-Fi · not isolated', dark:'#f0944a', light:'#cf5d18' },
      unknown: { name:'Unidentified',       vlan:'no name',            dark:'#8a93a3', light:'#5d6878' }
    };

    this.NODES = [
      { id:'internet', zone:'edge', kind:'WAN', label:'Internet', sub:'203.0.113.24',
        details:['Public address 203.0.113.24, from the ISP', 'Fibre, 500/50 Mbps plan'],
        flags:[], chips:[] },
      { id:'isp', zone:'edge', kind:'MODEM · ROUTER', label:'ISP modem', sub:'192.168.0.1', ip:'192.168.0.1',
        details:['Supplied by the ISP, left in router mode: it does NAT too',
                 'Its own Wi-Fi is still on, on the same channel as the Wi-Fi router'],
        flags:['Double NAT with the Wi-Fi router behind it', 'Its own Wi-Fi is still on'],
        chips:['NAT'] },
      { id:'router', zone:'edge', kind:'WI-FI ROUTER', label:'Wi-Fi router', sub:'192.168.1.1', ip:'192.168.1.1',
        details:['WAN 192.168.0.20, from the ISP modem: double NAT', 'DHCP for 192.168.1.0/24',
                 'Firmware 1.0.4 from 2021, auto-update off', 'UPnP on; WPS on',
                 'Port 5000 forwarded to the NAS', 'Admin password: still the one on its sticker'],
        flags:['Default admin password', 'Firmware from 2021, auto-update off', 'WPS on',
               'Forwards port 5000 from the internet to the NAS'],
        chips:['UPnP'] },
      { id:'mesh', zone:'gear', kind:'MESH NODE', label:'Mesh node · upstairs', sub:'192.168.1.2', ip:'192.168.1.2',
        details:['Wireless backhaul to the router at -78 dBm', 'Serves the bedrooms and the doorbell'],
        flags:['Weak link to the router (-78 dBm)'],
        chips:[] },
      { id:'switch', zone:'gear', kind:'SWITCH · UNMANAGED', label:'Switch behind the TV', sub:'no address',
        details:['5 ports, unmanaged: nothing to scan, so its links are assumed',
                 'The TV and the console share one router port through it'],
        flags:[], chips:['unmanaged'] },
      { id:'nas', zone:'devices', kind:'NAS', label:'NAS', sub:'192.168.1.20', ip:'192.168.1.20',
        details:['Photos, and the laptops’ backups', 'Admin page on port 5000, forwarded from the internet',
                 'No copy of it anywhere else'],
        flags:['Admin page reachable from the internet', 'The only copy of the backups'],
        chips:['port 5000'] },
      { id:'printer', zone:'devices', kind:'PRINTER', label:'Printer', sub:'192.168.1.30', ip:'192.168.1.30',
        details:['On the main Wi-Fi', 'Wi-Fi Direct on, with its default password'],
        flags:['Wi-Fi Direct on, default password'],
        chips:['Wi-Fi'] },
      { id:'tv', zone:'family', kind:'SMART TV', label:'Living room TV', sub:'192.168.1.40', ip:'192.168.1.40',
        details:['Wired, behind the switch (assumed)'],
        flags:[], chips:['wired'] },
      { id:'console', zone:'family', kind:'GAME CONSOLE', label:'Game console', sub:'192.168.1.41', ip:'192.168.1.41',
        details:['Wired, behind the switch (assumed)', 'NAT type: strict, from the double NAT'],
        flags:['NAT type strict: it can’t host online games'],
        chips:['UPnP'] },
      { id:'family', zone:'family', kind:'PHONES · TABLETS', label:'Phones & tablets', sub:'5 devices on Wi-Fi',
        details:['3 phones and 2 tablets', 'On the main Wi-Fi, with the smart home devices'],
        flags:[], chips:['5 devices'] },
      { id:'laptop', zone:'family', kind:'LAPTOP', label:'Work laptop', sub:'192.168.1.50', ip:'192.168.1.50',
        details:['Managed by an employer; VPN to the office', 'On the same network as the smart home devices'],
        flags:['Shares a network with the IoT devices'],
        chips:['VPN'] },
      { id:'doorbell', zone:'iot', kind:'DOORBELL CAMERA', label:'Doorbell', sub:'192.168.1.60', ip:'192.168.1.60',
        details:['On the main Wi-Fi through the mesh node', 'Drops off a few times a day while the mesh link is weak'],
        flags:['Drops off Wi-Fi a few times a day'],
        chips:['2.4 GHz'] },
      { id:'thermostat', zone:'iot', kind:'THERMOSTAT', label:'Thermostat', sub:'192.168.1.65', ip:'192.168.1.65',
        details:['On the main Wi-Fi through the mesh node'],
        flags:[], chips:['2.4 GHz'] },
      { id:'plugs', zone:'iot', kind:'IOT · SMART PLUGS', label:'Smart plugs ×4', sub:'192.168.1.70–73',
        details:['Four plugs on the main Wi-Fi', 'Run from the maker’s cloud; can reach every device here'],
        flags:[], chips:['2.4 GHz', '×4'] },
      { id:'unknown', zone:'unknown', kind:'UNKNOWN DEVICE', label:'Unknown device', sub:'192.168.1.77', ip:'192.168.1.77',
        details:['Answers ping; no name, no open ports', 'Its MAC is randomised, so no vendor'],
        flags:['Nobody has said what this is'],
        chips:['random MAC'] }
    ];

    this.EDGES = [
      { id:'e1',  from:'internet', to:'isp',        cls:'backbone' },
      { id:'e2',  from:'isp',      to:'router',     cls:'wired', label:'LAN 1 → WAN' },
      { id:'e3',  from:'router',   to:'mesh',       cls:'wifi',  dashed:true, label:'backhaul' },
      { id:'e4',  from:'router',   to:'switch',     cls:'wired', dashed:true, label:'assumed' },
      { id:'e5',  from:'switch',   to:'tv',         cls:'wired', dashed:true },
      { id:'e6',  from:'switch',   to:'console',    cls:'wired', dashed:true },
      { id:'e7',  from:'router',   to:'nas',        cls:'wired', label:'LAN 2' },
      { id:'e8',  from:'router',   to:'printer',    cls:'wifi',  dashed:true },
      { id:'e9',  from:'router',   to:'plugs',      cls:'wifi',  dashed:true },
      { id:'e10', from:'router',   to:'unknown',    cls:'wifi',  dashed:true },
      { id:'e11', from:'mesh',     to:'family',     cls:'wifi',  dashed:true },
      { id:'e12', from:'mesh',     to:'laptop',     cls:'wifi',  dashed:true },
      { id:'e13', from:'mesh',     to:'doorbell',   cls:'wifi',  dashed:true },
      { id:'e14', from:'mesh',     to:'thermostat', cls:'wifi',  dashed:true }
    ];

    this.ACTIONS = [
      { id:'a1',  sev:'high', text:'The router forwards port 5000 to the NAS admin page: remove the forward and reach the NAS through its maker’s app or a VPN.', nodes:['router','nas'] },
      { id:'a2',  sev:'high', text:'The Wi-Fi router still uses the admin password on its sticker: set a new one.', nodes:['router'] },
      { id:'a3',  sev:'med',  text:'The NAS holds the only copy of the backups: add a USB drive or a cloud copy.', nodes:['nas'] },
      { id:'a4',  sev:'med',  text:'Double NAT: the ISP modem routes too. Put it in bridge mode (or the Wi-Fi router in access-point mode) and turn its Wi-Fi off.', nodes:['isp','router','console'] },
      { id:'a5',  sev:'med',  text:'Smart plugs, the doorbell and the thermostat share the network with the laptops: move them to a guest or IoT Wi-Fi.', nodes:['plugs','doorbell','thermostat','laptop'] },
      { id:'a6',  sev:'med',  text:'Router firmware is from 2021 with auto-update off: update it, and turn auto-update on.', nodes:['router'] },
      { id:'a7',  sev:'med',  text:'The upstairs mesh node barely reaches the router (-78 dBm): move it a room closer, or run a cable to it.', nodes:['mesh','doorbell'] },
      { id:'a8',  sev:'low',  text:'WPS is on: turn it off; nothing needs it once devices have joined.', nodes:['router'] },
      { id:'a9',  sev:'low',  text:'The printer’s Wi-Fi Direct uses its default password: turn Wi-Fi Direct off.', nodes:['printer'] },
      { id:'a10', sev:'low',  text:'Unknown device at 192.168.1.77: find out whose it is, then name it or block it.', nodes:['unknown'] },
      { id:'a11', sev:'low',  text:'The switch behind the TV is unmanaged: the dashed links through it are assumed, not seen.', nodes:['switch','tv','console'] }
    ];

    this.BANDS = [];
"#;

/// The blank example: not a file under skills/ but built from the
/// /mapnetwork template, its sample site swapped for the smallest map that
/// still draws (the Internet and one gateway at one site). A start for a
/// map drawn by hand, or a small map to try things on.
pub const BLANK: &str = "Blank_map.dc.html";
pub const BLANK_LABEL: &str = "Blank map: the Internet and one gateway";
/// The template the blank is built in.
const TEMPLATE: &str = "mapnetwork/assets/Site Map Template.dc.html";

/// The blank map's site data, in the template's own format.
const BLANK_DATA: &str = r#"    // BLANK MAP: the Internet and one gateway at one site. Add to it, or
    // replace this whole block, to map a network.
    this.META = {
      title: 'Blank map',
      subtitle: 'the Internet and one gateway · a template to start from'
    };

    this.SITES = {
      site: { name:'Site' }
    };

    this.ZONES = {
      core: { name:'Edge / routing', vlan:'WAN · firewall', dark:'#5b9dff', light:'#2563c9' }
    };

    this.NODES = [
      { id:'internet', zone:'core', kind:'WAN', label:'Internet', sub:'upstream',
        details:[], flags:[], chips:[] },
      { id:'gateway', site:'site', zone:'core', kind:'GATEWAY · FIREWALL', label:'Gateway', sub:'',
        details:[], flags:[], chips:[] }
    ];

    this.EDGES = [
      { id:'e1', from:'internet', to:'gateway', cls:'backbone' }
    ];

    this.ACTIONS = [];

    this.BANDS = [];
"#;

/// The blank map's page (see `in_template`).
fn blank() -> &'static [u8] {
    static PAGE: OnceLock<Vec<u8>> = OnceLock::new();
    PAGE.get_or_init(|| in_template(BLANK_DATA))
}

/// The home map's page (see `in_template`).
fn home() -> &'static [u8] {
    static PAGE: OnceLock<Vec<u8>> = OnceLock::new();
    PAGE.get_or_init(|| in_template(HOME_DATA))
}

/// The template with its site data block swapped for `data` (so the page
/// still opens in a browser, and /mapnetwork can fill it in), or just the
/// data if the template has lost its markers.
fn in_template(data: &str) -> Vec<u8> {
    let html = file(TEMPLATE)
        .map(|d| String::from_utf8_lossy(d).into_owned())
        .unwrap_or_default();
    let start = html
        .find("SITE DATA ====")
        .and_then(|s| Some(s + html[s..].find('\n')? + 1));
    let end = start.and_then(|s| {
        let e = s + html[s..].find("END SITE DATA")?;
        Some(html[..e].rfind('\n')? + 1)
    });
    match (start, end) {
        (Some(s), Some(e)) if s <= e => format!("{}{data}{}", &html[..s], &html[e..]),
        _ => format!(
            "<html><script>\n    /* ===================== SITE DATA ===================== */\n\
             {data}    /* =================== END SITE DATA =================== */\n\
             </script></html>\n"
        ),
    }
    .into_bytes()
}

/// An example's bytes, by its `EXAMPLES` file.
pub fn example_file(f: &str) -> Option<&'static [u8]> {
    match f {
        BLANK => Some(blank()),
        HOME => Some(home()),
        _ => file(f),
    }
}

/// True when `p` is the blank example opened (a map saved from it is a
/// plan).
pub fn is_blank(p: &Path) -> bool {
    example_path(EXAMPLES.len() - 1) == p
}

/// Where an example "is" for Config Export: not a real file, so a Reload
/// reads it from here again.
const EXAMPLE_DIR: &str = "(is-alive example)";

pub fn file(path: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(p, _)| *p == path).map(|(_, d)| *d)
}

/// The path Config Export opens example `i` under.
pub fn example_path(i: usize) -> PathBuf {
    let f = EXAMPLES[i].1;
    Path::new(EXAMPLE_DIR).join(f.rsplit('/').next().unwrap_or(f))
}

/// An example's bytes, if `p` is one of `example_path`'s.
pub fn example(p: &Path) -> Option<&'static [u8]> {
    if p.parent() != Some(Path::new(EXAMPLE_DIR)) {
        return None;
    }
    let name = p.file_name()?.to_str()?;
    EXAMPLES
        .iter()
        .find(|(_, f)| f.rsplit('/').next() == Some(name))
        .and_then(|(_, f)| example_file(f))
}

/// Total size of the skills' files.
pub fn bytes() -> usize {
    FILES.iter().map(|(_, d)| d.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_skills_and_both_examples_are_embedded() {
        for n in NAMES {
            assert!(file(&format!("{n}/SKILL.md")).is_some(), "{n}");
        }
        for i in 0..EXAMPLES.len() {
            let d = example(&example_path(i)).expect("example");
            assert!(crate::mapnetwork::sniff(d));
            assert!(crate::mapnetwork::parse(d).is_ok());
        }
        assert!(example(Path::new("/tmp/example-Contoso_Health.dc.html")).is_none());
    }

    #[test]
    fn the_blank_example_is_the_template_with_two_cards() {
        let i = EXAMPLES.iter().position(|e| e.1 == BLANK).unwrap();
        assert!(is_blank(&example_path(i)));
        assert!(!is_blank(&example_path(0)));
        let d = example(&example_path(i)).unwrap();
        let text = std::str::from_utf8(d).unwrap();
        // Built in the template (its engine kept, its sample site gone).
        let template = std::str::from_utf8(file(TEMPLATE).unwrap()).unwrap();
        assert!(
            text.len() > template.len() / 2,
            "the template lost its markers"
        );
        assert!(!text.contains("Acme Field Office"));
        let doc = crate::mapnetwork::parse(d).unwrap();
        assert_eq!(doc.title, "Blank map");
        assert_eq!(doc.nodes.len(), 2);
        assert_eq!(doc.edges.len(), 1);
        assert_eq!(doc.sites.len(), 1);
        assert!(doc.actions.is_empty());
    }
}
