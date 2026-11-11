//! Network map: one picture of what the app knows about a site. The sweep,
//! a UniFi backup and a SonicWall export merge into a tree rooted at the
//! internet, in the spirit of /mapnetwork: roles as colours, infrastructure
//! first, clients only on request, and findings drawn from evidence. A link
//! the data doesn't show (a sweep can't see switch ports) is drawn as
//! assumed, never as fact.

pub(crate) mod colours;
pub mod compare;
pub mod export;
pub mod import;
pub mod layout;
pub mod legend;
pub mod runs;
pub mod view;

use crate::net::mac::{self, Mac};
use crate::theme::Theme;
use eframe::egui::Color32;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::net::Ipv4Addr;

/// Most cards on one map, as /mapnetwork: past this it stops being a map.
pub const BUDGET: usize = 120;
/// Clients shown under one switch or AP before the rest fold into a card.
const PER_UPLINK: usize = 10;
/// More: clients per uplink, and the most cards before that tightens.
const MORE_PER_UPLINK: usize = 30;
const MORE_BUDGET: usize = 400;
/// Clients a +N on a card adds each click.
pub const STEP: usize = 8;

/// Which end devices get cards: the level the toolbar steps through.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Clients {
    #[default]
    Off,
    /// Up to 10 per switch or AP, the rest in a +N more card, fewer per
    /// uplink on a big map.
    Some,
    /// Up to 30 per uplink, on a map of up to 400 cards.
    More,
    /// A card for every one.
    All,
}

impl Clients {
    pub const ALL: [Clients; 4] = [Clients::Off, Clients::Some, Clients::More, Clients::All];

    pub fn name(self) -> &'static str {
        match self {
            Clients::Off => "Off",
            Clients::Some => "Some",
            Clients::More => "More",
            Clients::All => "All",
        }
    }

    pub fn tip(self) -> &'static str {
        match self {
            Clients::Off => {
                "No end devices: a count on each switch or AP instead (+N on a card shows its \
                 clients a few at a time)"
            }
            Clients::Some => {
                "Up to 10 per switch or AP, the rest in a +N more card, fewer on a big map. \
                 Ones with a warning, printers, cameras, phones and named devices come first"
            }
            Clients::More => "Up to 30 per switch or AP, on a map of up to 400 cards",
            Clients::All => {
                "A card for every end device. A big site can mean hundreds of cards; Zones \
                 keeps them tidy"
            }
        }
    }

    /// Clients under each uplink, and the most cards on the map before
    /// that count tightens.
    fn caps(self) -> (usize, usize) {
        match self {
            Clients::Off => (0, BUDGET),
            Clients::Some => (PER_UPLINK, BUDGET),
            Clients::More => (MORE_PER_UPLINK, MORE_BUDGET),
            Clients::All => (usize::MAX, usize::MAX),
        }
    }
}

/// Role, which is the map's only colour channel.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Default)]
pub enum Zone {
    Internet,
    Gateway,
    Switch,
    Wireless,
    /// Network gear the sweep found by vendor or management port.
    Gear,
    Server,
    /// Printers and cameras.
    Device,
    Client,
    #[default]
    Unknown,
}

impl Zone {
    pub const ALL: [Zone; 9] = [
        Zone::Internet,
        Zone::Gateway,
        Zone::Switch,
        Zone::Wireless,
        Zone::Gear,
        Zone::Server,
        Zone::Device,
        Zone::Client,
        Zone::Unknown,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Zone::Internet => "Internet",
            Zone::Gateway => "Gateway / firewall",
            Zone::Switch => "Switches",
            Zone::Wireless => "Access points",
            Zone::Gear => "Network gear (inferred)",
            Zone::Server => "Servers / NAS",
            Zone::Device => "Printers / cameras",
            Zone::Client => "Clients",
            Zone::Unknown => "Unidentified",
        }
    }

    /// (dark, light) accent: /mapnetwork's palette, plus teal and slate.
    pub fn rgb(self, dark: bool) -> [u8; 3] {
        let (d, l) = match self {
            Zone::Internet => ([150, 160, 178], [84, 96, 116]),
            Zone::Gateway => ([0x5b, 0x9d, 0xff], [0x25, 0x63, 0xc9]),
            Zone::Switch => ([0xa9, 0x8b, 0xff], [0x6d, 0x44, 0xd9]),
            Zone::Wireless => ([0x5f, 0xcf, 0x8a], [0x1c, 0x9b, 0x54]),
            Zone::Gear => ([0x4f, 0xc3, 0xd9], [0x1f, 0x8a, 0xa0]),
            Zone::Server => ([0xe0, 0xb3, 0x4d], [0xa9, 0x79, 0x0c]),
            Zone::Device => ([0xf0, 0x94, 0x4a], [0xcf, 0x5d, 0x18]),
            Zone::Client => ([0x8f, 0xa8, 0xc8], [0x47, 0x65, 0x8c]),
            Zone::Unknown => ([0x8a, 0x93, 0xa3], [0x5d, 0x68, 0x78]),
        };
        if dark { d } else { l }
    }

    /// The five roles a theme menu shows as swatches, in /mapnetwork's
    /// zone order (core, server, trusted, iot, guest).
    pub const SWATCH: [Zone; 5] = [
        Zone::Gateway,
        Zone::Switch,
        Zone::Wireless,
        Zone::Device,
        Zone::Server,
    ];

    /// Its colour in a theme: /mapnetwork's zone colours for that theme
    /// (Miami's own neons), with gear, clients and the rest to match.
    pub fn colour_in(self, t: Theme) -> Color32 {
        // Internet, Gateway, Switch, Wireless, Gear, Server, Device, Client, Unknown.
        let hex: [u32; 9] = match t {
            Theme::Dark | Theme::Light => {
                let [r, g, b] = self.rgb(!t.is_light());
                return Color32::from_rgb(r, g, b);
            }
            Theme::Miami => [
                0xbeaae6, 0xff48aa, 0xb278ff, 0x28e2dc, 0x5ab4ff, 0xff9646, 0xffdc5a, 0x9aa0f0,
                0x9682b4,
            ],
            Theme::Slate => [
                0x8a94a8, 0x7aa7ff, 0xb8a2ff, 0x72d49c, 0x6fd3e0, 0xe6c36a, 0xf2a566, 0x9fb0cc,
                0x7f8899,
            ],
            Theme::Phosphor => [
                0x6f9c7c, 0x4fd1ff, 0xc792ff, 0x5cf28a, 0x3ff0d0, 0xf2e05c, 0xffb454, 0x8fd4a2,
                0x5f8c6c,
            ],
            Theme::Blueprint => [
                0x8fb0d6, 0x8cc4ff, 0xc7b3ff, 0x8ee6b0, 0x7fe0ea, 0xffe08a, 0xffbd7a, 0xc3d8f0,
                0x7c9dc4,
            ],
            Theme::Contrast => [
                0xd0d0d0, 0x6cb6ff, 0xd2a8ff, 0x56f08a, 0x4fe0f0, 0xffe066, 0xffa657, 0xffffff,
                0xb0b0b0,
            ],
            Theme::Rgb => [
                0xa58cd6, 0x00e5ff, 0xb14dff, 0x39ff14, 0xff00c8, 0xfff200, 0xff8a00, 0x7a9cff,
                0x8a6fc0,
            ],
            // Dashboard lamps; clients grey.
            Theme::Scanner => [
                0x8c8c92, 0xff2a1a, 0xffb000, 0x39e06a, 0x4fc3ff, 0xd27dff, 0xff7a1a, 0xa9a9b0,
                0x6a6a70,
            ],
            Theme::Paper => [
                0x6a5f4c, 0x2f5fb3, 0x7046c2, 0x2b7d4b, 0x1f7f8a, 0x94700f, 0xb8561c, 0x5a6f8c,
                0x8a7f6c,
            ],
        };
        let i = Zone::ALL.iter().position(|z| *z == self).unwrap_or(0);
        let h = hex[i];
        Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }
}

/// How a card hangs off its parent.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Link {
    /// Internet to the gateway.
    Backbone,
    Wired,
    Wireless,
    /// Not in the data: placed under the gateway because it answered there.
    Assumed,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Sev {
    High,
    Med,
    Low,
}

impl Sev {
    pub fn label(self) -> &'static str {
        match self {
            Sev::High => "high",
            Sev::Med => "med",
            Sev::Low => "low",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    /// Stable across rebuilds, so positions and the selection survive.
    pub id: String,
    pub zone: Zone,
    /// Small caps line at the top of the card, at most 18 characters.
    pub kind: String,
    pub label: String,
    pub sub: String,
    pub chips: Vec<String>,
    pub details: Vec<String>,
    pub flags: Vec<String>,
    pub ip: Option<Ipv4Addr>,
    pub mac: Option<Mac>,
    /// An end device: drawn only when clients are on.
    pub client: bool,
    pub seen_by: Vec<&'static str>,
    pub open: Vec<u16>,
    /// Its management ports were checked (a sweep's port pass reached it),
    /// so an empty `open` means none answered, not "not looked".
    pub ports_checked: bool,
    /// Parent, link and a label for the link (the parent's port). None only
    /// for the internet.
    pub up: Option<(usize, Link, Option<String>)>,
    /// Ranks clients within a reveal tier: (0 named, 1 vendor only, 2
    /// anonymous; then lower is seen more recently).
    rank: (u8, u64),
    /// The site it's at, when the source has sites (/mapnetwork maps).
    pub site: Option<String>,
    /// Pinned, then missing from the latest data: kept on the map, marked,
    /// for the user to review.
    pub gone: bool,
    /// Drawn by a person (on a plan or a training map), not discovered.
    /// Only a saved map's file carries it.
    pub drawn: bool,
    /// Its link up was drawn by a person, not discovered (the card itself
    /// may have been).
    pub drawn_up: bool,
    /// The technician's note on it, redacted: put on by the Map tab from
    /// the notes it keeps (never from the data), for the tooltip, search,
    /// Copy and the exports.
    pub note: String,
}

impl Node {
    pub fn parent(&self) -> Option<usize> {
        self.up.as_ref().map(|u| u.0)
    }

    pub fn link(&self) -> Option<Link> {
        self.up.as_ref().map(|u| u.1)
    }

    fn seen(&mut self, by: &'static str) {
        if !self.seen_by.contains(&by) {
            self.seen_by.push(by);
        }
    }

    fn chip(&mut self, c: impl Into<String>) {
        let c = c.into();
        if !self.chips.contains(&c) {
            self.chips.push(c);
        }
    }

    /// A line for Details, once: the same evidence twice says nothing new.
    fn detail(&mut self, line: String) {
        if !self.details.contains(&line) {
            self.details.push(line);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub sev: Sev,
    pub text: String,
    pub nodes: Vec<usize>,
}

#[derive(Default)]
pub struct Map {
    /// The map's own title (a /mapnetwork map's); empty for the rest.
    pub title: String,
    pub nodes: Vec<Node>,
    pub findings: Vec<Finding>,
    /// What the map was built from, for under the title.
    pub summary: String,
    /// Clients known but not drawn as their own card: in a count on their
    /// uplink, or in a +N more card.
    pub hidden: usize,
    /// Ids of the cards left out to keep the map readable (clients off, or
    /// past the per-uplink cap): still in the data, just not drawn.
    pub folded: HashSet<String>,
    /// Sites collapsed into one card, with the ids of the cards inside.
    pub closed: BTreeMap<String, Vec<String>>,
    /// Each uplink with clients, by id: (clients drawn, clients known).
    pub under: HashMap<String, (usize, usize)>,
    /// Clients per uplink the level came to (fewer than asked on a big map).
    pub cap: usize,
    /// A sweep was loaded but left off: it found nothing the data knows
    /// (another network).
    pub sweep_apart: bool,
}

impl Map {
    pub fn children(&self) -> Vec<Vec<usize>> {
        let mut c = vec![Vec::new(); self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            if let Some(p) = n.parent() {
                c[p].push(i);
            }
        }
        c
    }

    /// Every site named on a card, in the order first seen.
    pub fn sites(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for s in self.nodes.iter().filter_map(|n| n.site.as_ref()) {
            if !v.contains(s) {
                v.push(s.clone());
            }
        }
        v
    }

    /// Give every card but the internet a site: its own, else its
    /// parent's, else (hung straight off the internet) the first one below
    /// it, else `default`.
    pub fn fill_sites(&mut self, default: &str) {
        let kids = self.children();
        let mut order: Vec<usize> = (0..self.nodes.len()).collect();
        order.sort_by_key(|&i| self.path(i).len());
        for i in order {
            let n = &self.nodes[i];
            if n.zone == Zone::Internet || n.site.is_some() {
                continue;
            }
            let site = n
                .parent()
                .filter(|&p| self.nodes[p].zone != Zone::Internet)
                .and_then(|p| self.nodes[p].site.clone())
                .or_else(|| site_below(&self.nodes, &kids, i))
                .unwrap_or_else(|| default.to_string());
            self.nodes[i].site = Some(site);
        }
    }

    /// " · N clients not drawn", for under the title, when any aren't.
    fn hidden_note(&self) -> String {
        match self.hidden {
            0 => String::new(),
            1 => " · 1 client not drawn".into(),
            n => format!(" · {n} clients not drawn"),
        }
    }

    /// Whether the card `id` is inside a collapsed site.
    pub fn tucked(&self, id: &str) -> bool {
        self.closed.values().any(|v| v.iter().any(|s| s == id))
    }

    /// The id of a collapsed site's card.
    pub fn site_id(site: &str) -> String {
        format!("site:{site}")
    }

    /// Fold each site in `close` into one card that hangs where the site
    /// does, with everything below the site hanging off it. The cards inside
    /// are listed in its details and their findings point at it.
    pub fn close_sites(&mut self, close: &HashSet<String>) {
        for site in self.sites() {
            if !close.contains(&site) {
                continue;
            }
            let inside: Vec<usize> = (0..self.nodes.len())
                .filter(|&i| {
                    let n = &self.nodes[i];
                    n.zone != Zone::Internet && n.site.as_ref() == Some(&site)
                })
                .collect();
            if !inside.is_empty() {
                self.fold_site(&site, &inside);
            }
        }
    }

    fn fold_site(&mut self, site: &str, inside: &[usize]) {
        let n = self.nodes.len();
        let mut member = vec![false; n];
        for &i in inside {
            member[i] = true;
        }
        // It hangs where the site's top card does.
        let up = inside
            .iter()
            .filter(|&&i| self.nodes[i].parent().is_none_or(|p| !member[p]))
            .min_by_key(|&&i| self.path(i).len())
            .and_then(|&i| self.nodes[i].up.clone())
            .filter(|u| !member[u.0])
            .unwrap_or((0, Link::Assumed, None));
        let zone = inside
            .iter()
            .map(|&i| self.nodes[i].zone)
            .min_by_key(|&z| layout::zone_order(z))
            .unwrap_or(Zone::Unknown);
        let mut roles: Vec<(Zone, usize)> = Vec::new();
        for &i in inside {
            let z = self.nodes[i].zone;
            match roles.iter_mut().find(|r| r.0 == z) {
                Some(r) => r.1 += 1,
                None => roles.push((z, 1)),
            }
        }
        roles.sort_by_key(|r| layout::zone_order(r.0));
        let flags: usize = inside.iter().map(|&i| self.nodes[i].flags.len()).sum();
        let mut card = Node {
            id: Self::site_id(site),
            zone,
            kind: "SITE".into(),
            label: site.to_string(),
            sub: format!(
                "{} card{} inside",
                inside.len(),
                if inside.len() == 1 { "" } else { "s" }
            ),
            chips: vec!["collapsed".into()],
            site: Some(site.to_string()),
            up: Some(up),
            ..Default::default()
        };
        card.details = roles
            .iter()
            .map(|(z, c)| format!("{}: {c}", z.name()))
            .collect();
        card.details.push("Inside:".into());
        for &i in inside.iter().take(60) {
            let c = &self.nodes[i];
            card.details.push(
                [c.label.as_str(), c.sub.as_str()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" · "),
            );
        }
        if inside.len() > 60 {
            card.details
                .push(format!("…and {} more", inside.len() - 60));
        }
        if flags > 0 {
            card.flags.push(format!(
                "{flags} flag{} on cards inside: expand the site to see them",
                if flags == 1 { "" } else { "s" }
            ));
        }
        let ids: Vec<String> = inside.iter().map(|&i| self.nodes[i].id.clone()).collect();

        // Reindex: the cards inside become the site's card, where the
        // first of them was, so the site keeps its place in the sidebar
        // and the layout.
        let first = inside.iter().copied().min().unwrap_or(n);
        let mut map_to = vec![usize::MAX; n];
        let mut out = Vec::with_capacity(n - inside.len() + 1);
        let mut card = Some(card);
        for (i, node) in std::mem::take(&mut self.nodes).into_iter().enumerate() {
            if i == first {
                out.extend(card.take());
            }
            if !member[i] {
                map_to[i] = out.len();
                out.push(node);
            }
        }
        let at = first.min(out.len());
        out.extend(card.take());
        for (i, m) in member.iter().enumerate() {
            if *m {
                map_to[i] = at;
            }
        }
        for node in &mut out {
            if let Some(u) = &mut node.up {
                u.0 = map_to[u.0];
            }
        }
        self.nodes = out;
        for f in &mut self.findings {
            let mut v: Vec<usize> = Vec::new();
            for &k in &f.nodes {
                let k = map_to[k];
                if !v.contains(&k) {
                    v.push(k);
                }
            }
            f.nodes = v;
        }
        self.closed.insert(site.to_string(), ids);
    }

    /// Path from the internet down to `i`.
    pub fn path(&self, mut i: usize) -> Vec<usize> {
        let mut p = vec![i];
        while let Some(u) = self.nodes.get(i).and_then(Node::parent) {
            if p.contains(&u) {
                break;
            }
            p.push(u);
            i = u;
        }
        p.reverse();
        p
    }
}

/// The site of the first card below `i` that has one.
fn site_below(nodes: &[Node], kids: &[Vec<usize>], i: usize) -> Option<String> {
    let mut seen = vec![false; nodes.len()];
    let mut stack = kids[i].clone();
    while let Some(k) = stack.pop() {
        if std::mem::replace(&mut seen[k], true) {
            continue;
        }
        if let Some(s) = &nodes[k].site {
            return Some(s.clone());
        }
        stack.extend(kids[k].iter().rev());
    }
    None
}

/// A host from the sweep.
#[derive(Clone, Debug)]
pub struct Seen {
    pub ip: Ipv4Addr,
    pub mac: Option<Mac>,
    pub name: Option<String>,
    /// None: answered ARP, not ping.
    pub rtt: Option<f64>,
    pub open: Vec<u16>,
    /// Its management ports were checked (`open` then says what answered).
    pub checked: bool,
}

/// A backup or export open in Config Export: its file's name and the JSON
/// it was converted to.
#[derive(Clone, Copy, Debug, Default)]
pub struct Loaded<'a> {
    /// The file's name, which tells two controllers' sites of one name
    /// (or two firewalls of one name) apart.
    pub name: &'a str,
    pub json: &'a str,
}

/// One backup or export, for tests here and elsewhere.
#[cfg(test)]
pub(crate) fn one(json: &str) -> Vec<Loaded<'_>> {
    vec![Loaded { name: "", json }]
}

#[derive(Default)]
pub struct Input<'a> {
    pub sweep: Vec<Seen>,
    /// The swept range, once a sweep has finished.
    pub range: Option<String>,
    pub ports_checked: bool,
    pub gateway: Option<Ipv4Addr>,
    pub this: Option<Ipv4Addr>,
    pub wan: Option<Ipv4Addr>,
    /// Every UniFi backup open, in the order opened. Where two know the
    /// same device or client (one controller's backups from two days),
    /// the one opened later wins.
    pub unifi: Vec<Loaded<'a>>,
    /// Every SonicWall export or report open, in the order opened. Each
    /// firewall is its own gateway; the one opened last takes whatever
    /// can't be placed under another.
    pub sonicwall: Vec<Loaded<'a>>,
    /// A /mapnetwork map: when loaded, it is the map.
    pub mapnetwork: Option<&'a crate::mapnetwork::Doc>,
    pub clients: Clients,
    /// Uplinks stepped on their own (+N or − on the card, Show all), by
    /// id: how many clients each shows, whatever the level.
    pub reveal: Option<&'a HashMap<String, usize>>,
}

impl Input<'_> {
    pub fn is_empty(&self) -> bool {
        self.sweep.is_empty()
            && self.unifi.is_empty()
            && self.sonicwall.is_empty()
            && self.mapnetwork.is_none()
    }
}

// ---------------------------------------------------------------- sources

struct UDevice {
    mac: Mac,
    name: String,
    kind: String,
    model: String,
    ip: Option<Ipv4Addr>,
    version: Option<String>,
    serial: Option<String>,
    eol: bool,
    disabled: bool,
    ports: usize,
    /// (uplink MAC, port on the uplink, wireless).
    up: Option<(Mac, Option<i64>, bool)>,
    /// The controller site it belongs to (the site's `_id`).
    site_id: Option<String>,
    /// The backup it came from: its place in `Input::unifi`.
    file: usize,
}

struct UClient {
    mac: Mac,
    name: Option<String>,
    oui: Option<String>,
    ip: Option<Ipv4Addr>,
    wired: bool,
    guest: bool,
    blocked: bool,
    up: Option<(Mac, Option<i64>)>,
    last_seen: i64,
    site_id: Option<String>,
}

/// A UniFi network.
struct UNet {
    /// "VLAN 10 Voice (10.0.10.1/24)", for the gateway's details.
    line: String,
    site_id: Option<String>,
    /// Its gateway address and prefix, from "10.0.10.1/24".
    gw: Option<(Ipv4Addr, u8)>,
}

#[derive(Default)]
struct Unifi {
    devices: Vec<UDevice>,
    clients: Vec<UClient>,
    networks: Vec<UNet>,
    sites: Vec<String>,
    /// (site `_id`, name), to name each device's site.
    site_ids: Vec<(String, String)>,
}

/// "10.0.0.1/24" as its address and prefix.
fn cidr(s: &str) -> Option<(Ipv4Addr, u8)> {
    let (ip, bits) = s.trim().split_once('/')?;
    let bits: u8 = bits.parse().ok().filter(|b| *b <= 32)?;
    Some((ip.parse().ok()?, bits))
}

/// A mask as a prefix: "255.255.255.0" or "/24" (or "24").
fn prefix(mask: &str) -> Option<u8> {
    let m = mask.trim().trim_start_matches('/');
    if let Ok(bits) = m.parse::<u8>() {
        return (bits <= 32).then_some(bits);
    }
    let m = u32::from(m.parse::<Ipv4Addr>().ok()?);
    // Contiguous ones only.
    (m.leading_ones() + m.trailing_zeros() == 32).then_some(m.leading_ones() as u8)
}

/// Whether `ip` is on the network `net`/`bits`.
fn on_net(ip: Ipv4Addr, (net, bits): (Ipv4Addr, u8)) -> bool {
    let mask = u32::MAX.checked_shl(32 - u32::from(bits)).unwrap_or(0);
    u32::from(ip) & mask == u32::from(net) & mask
}

/// A file's name without its last extension, to tell two sites (or
/// firewalls) of one name apart; `n` (counting from 1) when it has none.
fn file_tag(name: &str, n: usize) -> String {
    std::path::Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| n.to_string())
}

/// What sets each of `tags` apart: the words (split at `_`, `-`, `.` and
/// spaces) they all start or end with left off, so "acme-north_backup_
/// 2026-11-10" and "acme-south_backup_2026-11-10" are "north" and "south".
/// A part with no letters keeps the word before it ("14-05", not "05").
/// A tag no different from the others stays whole.
fn differing(tags: &[&str]) -> Vec<String> {
    let words: Vec<Vec<&str>> = tags
        .iter()
        .map(|t| t.split_inclusive(['_', '-', '.', ' ']).collect())
        .collect();
    let shortest = words.iter().map(Vec::len).min().unwrap_or(0);
    let mut lead = 0;
    while lead < shortest && words.iter().all(|w| w[lead] == words[0][lead]) {
        lead += 1;
    }
    let from_end = |w: &Vec<&str>, k: usize| w[w.len() - 1 - k].to_string();
    let mut tail = 0;
    while tail < shortest - lead
        && words
            .iter()
            .all(|w| from_end(w, tail) == from_end(&words[0], tail))
    {
        tail += 1;
    }
    let trim = |s: &str| s.trim_matches(['_', '-', '.', ' ']).to_string();
    words
        .iter()
        .zip(tags)
        .map(|(w, tag)| {
            let mut from = lead;
            let to = w.len() - tail;
            if from > 0 && !w[from..to].concat().chars().any(char::is_alphabetic) {
                from -= 1;
            }
            match trim(&w[from..to].concat()) {
                s if s.is_empty() => tag.to_string(),
                s => s,
            }
        })
        .collect()
}

/// Every name in `names` once: one used more than once gets what sets its
/// source's tag apart ("Default (north)"), and a number if that's not
/// enough.
fn tell_apart(names: &[(String, String)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (n, tag) in names {
        let group: Vec<&str> = names
            .iter()
            .filter(|(m, _)| m == n)
            .map(|(_, t)| t.as_str())
            .collect();
        let mut name = if group.len() > 1 {
            let at = group.iter().position(|t| t == tag).unwrap_or(0);
            format!("{n} ({})", differing(&group)[at])
        } else {
            n.clone()
        };
        let base = name.clone();
        let mut k = 2;
        while out.contains(&name) {
            name = format!("{base} {k}");
            k += 1;
        }
        out.push(name);
    }
    out
}

/// Drop from each list what a later list holds again (by `key`), so the
/// one opened later wins; within one list, nothing is dropped.
fn later_wins<T, K: std::hash::Hash + Eq>(lists: Vec<Vec<T>>, key: impl Fn(&T) -> K) -> Vec<T> {
    let mut later: HashSet<K> = HashSet::new();
    let mut kept: Vec<Vec<T>> = Vec::new();
    for list in lists.into_iter().rev() {
        let mine: Vec<K> = list.iter().map(&key).collect();
        kept.push(
            list.into_iter()
                .filter(|x| !later.contains(&key(x)))
                .collect(),
        );
        later.extend(mine);
    }
    kept.into_iter().rev().flatten().collect()
}

/// Every UniFi backup open, as one.
fn read_unifi(files: &[Loaded]) -> Option<Unifi> {
    let all = files.iter().map(|f| parse_unifi(f.json)).collect();
    let names: Vec<&str> = files.iter().map(|f| f.name).collect();
    merge_unifi(all, &names)
}

/// Several backups as one. Devices and clients go by MAC and networks by
/// site and line, the later backup winning (one controller's backups from
/// two days). Sites go by `_id`, which two backups of one controller
/// share; a site name two controllers both use ("Default") gets its
/// backup's file name, so their devices don't share a box. `names`: each
/// backup's file name. One backup is read as it is.
fn merge_unifi(mut all: Vec<Unifi>, names: &[&str]) -> Option<Unifi> {
    if all.len() <= 1 {
        return all.pop();
    }
    // Each site once: (key, newest name, its backup).
    let mut sites: Vec<(String, String, usize)> = Vec::new();
    for (k, u) in all.iter_mut().enumerate() {
        let mut own = u.site_ids.clone();
        for s in &u.sites {
            if !u.site_ids.iter().any(|(_, n)| n == s) {
                own.push((format!("{k}/{s}"), s.clone()));
            }
        }
        // At a backup of one site, what names no site of its own is there.
        if let [(only, _)] = own.as_slice() {
            let known = |id: &Option<String>| id.as_ref().is_some_and(|id| id == only);
            for d in u.devices.iter_mut().filter(|d| !known(&d.site_id)) {
                d.site_id = Some(only.clone());
            }
            for c in u.clients.iter_mut().filter(|c| !known(&c.site_id)) {
                c.site_id = Some(only.clone());
            }
            for n in u.networks.iter_mut().filter(|n| !known(&n.site_id)) {
                n.site_id = Some(only.clone());
            }
        }
        for d in &mut u.devices {
            d.file = k;
        }
        for (id, name) in own {
            match sites.iter_mut().find(|s| s.0 == id) {
                Some(s) => (s.1, s.2) = (name, k),
                None => sites.push((id, name, k)),
            }
        }
    }
    let tagged: Vec<(String, String)> = sites
        .iter()
        .map(|(_, name, k)| {
            let tag = file_tag(names.get(*k).copied().unwrap_or_default(), k + 1);
            (name.clone(), tag)
        })
        .collect();
    let shown = tell_apart(&tagged);
    let mut devices = Vec::new();
    let mut clients = Vec::new();
    let mut networks = Vec::new();
    for u in all {
        devices.push(u.devices);
        clients.push(u.clients);
        networks.push(u.networks);
    }
    Some(Unifi {
        devices: later_wins(devices, |d| d.mac),
        clients: later_wins(clients, |c| c.mac),
        networks: later_wins(networks, |n| (n.site_id.clone(), n.line.clone())),
        site_ids: sites
            .iter()
            .zip(&shown)
            .map(|(s, name)| (s.0.clone(), name.clone()))
            .collect(),
        sites: shown,
    })
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn ipv4(v: &Value, k: &str) -> Option<Ipv4Addr> {
    text(v, k)?.parse().ok()
}

/// mongoexport-style array from Config Export: `{"__cmd":"select"}` markers
/// followed by that collection's documents.
fn parse_unifi(json: &str) -> Unifi {
    let mut u = Unifi::default();
    let Ok(Value::Array(docs)) = serde_json::from_str::<Value>(json) else {
        return u;
    };
    let mut coll = String::new();
    for d in &docs {
        if d.get("__cmd").and_then(Value::as_str) == Some("select") {
            coll = text(d, "collection").unwrap_or_default();
            continue;
        }
        match coll.as_str() {
            "site" => {
                if let Some(name) = text(d, "desc").or_else(|| text(d, "name")) {
                    if let Some(id) = text(d, "_id") {
                        u.site_ids.push((id, name.clone()));
                    }
                    u.sites.push(name);
                }
            }
            "device" => {
                let Some(m) = text(d, "mac").and_then(|m| mac::parse_mac(&m)) else {
                    continue;
                };
                let kind = text(d, "type").unwrap_or_else(|| "device".into());
                let up = d.get("last_uplink").and_then(|l| {
                    let um = mac::parse_mac(&text(l, "uplink_mac")?)?;
                    Some((
                        um,
                        l.get("uplink_remote_port").and_then(Value::as_i64),
                        text(l, "type").as_deref() == Some("wireless"),
                    ))
                });
                u.devices.push(UDevice {
                    mac: m,
                    // UniFi leaves some names unset (mesh APs): type + MAC tail.
                    name: text(d, "name")
                        .unwrap_or_else(|| format!("{kind}-{}", &mac::fmt(&m)[12..])),
                    model: text(d, "model").unwrap_or_default(),
                    kind,
                    ip: ipv4(d, "ip"),
                    version: text(d, "version"),
                    serial: text(d, "serial"),
                    eol: d.get("model_in_eol").and_then(Value::as_bool) == Some(true),
                    disabled: d.get("disabled").and_then(Value::as_bool) == Some(true),
                    ports: d
                        .get("port_table")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len),
                    up,
                    site_id: text(d, "site_id"),
                    file: 0,
                });
            }
            "user" => {
                let Some(m) = text(d, "mac").and_then(|m| mac::parse_mac(&m)) else {
                    continue;
                };
                let up = text(d, "last_uplink_mac")
                    .and_then(|um| mac::parse_mac(&um))
                    .map(|um| (um, d.get("last_uplink_remote_port").and_then(Value::as_i64)));
                u.clients.push(UClient {
                    mac: m,
                    name: text(d, "name").or_else(|| text(d, "hostname")),
                    oui: text(d, "oui"),
                    ip: ipv4(d, "fixed_ip").or_else(|| ipv4(d, "last_ip")),
                    wired: d.get("is_wired").and_then(Value::as_bool) == Some(true),
                    guest: d.get("is_guest").and_then(Value::as_bool) == Some(true),
                    blocked: d.get("blocked").and_then(Value::as_bool) == Some(true),
                    up,
                    last_seen: d.get("last_seen").and_then(Value::as_i64).unwrap_or(0),
                    site_id: text(d, "site_id"),
                });
            }
            "networkconf" => {
                let name = text(d, "name").unwrap_or_default();
                let subnet = text(d, "ip_subnet");
                let line = match (d.get("vlan").and_then(Value::as_i64), &subnet) {
                    (Some(v), Some(s)) => format!("VLAN {v} {name} ({s})"),
                    (Some(v), None) => format!("VLAN {v} {name}"),
                    (None, Some(s)) => format!("{name} ({s}, untagged)"),
                    (None, None) => continue,
                };
                u.networks.push(UNet {
                    line,
                    site_id: text(d, "site_id"),
                    gw: subnet.as_deref().and_then(cidr),
                });
            }
            _ => {}
        }
    }
    u
}

struct Iface {
    name: String,
    zone: String,
    ip: Option<Ipv4Addr>,
    mask: Option<String>,
    mac: Option<Mac>,
    mgmt: Vec<String>,
}

/// A device the firewall heard on one of its ports (LLDP or LLTD).
struct Heard {
    iface: String,
    /// LLTD: a Windows host. LLDP: network gear, which names its port.
    lltd: bool,
    mac: Option<Mac>,
    ip: Option<Ipv4Addr>,
    name: Option<String>,
    description: Option<String>,
    port: Option<String>,
    caps: String,
}

#[derive(Default)]
struct Sonic {
    model: Option<String>,
    firmware: Option<String>,
    serial: Option<String>,
    name: Option<String>,
    ifaces: Vec<Iface>,
    heard: Vec<Heard>,
}

fn parse_sonicwall(json: &str) -> Option<Sonic> {
    let v: Value = serde_json::from_str(json).ok()?;
    let mut s = Sonic::default();
    for n in v
        .get("neighbors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        s.heard.push(Heard {
            iface: text(n, "interface").unwrap_or_default(),
            lltd: text(n, "protocol").as_deref() == Some("LLTD"),
            mac: text(n, "mac").and_then(|m| mac::parse_mac(&m)),
            ip: ipv4(n, "ip"),
            name: text(n, "name"),
            description: text(n, "description"),
            port: text(n, "port"),
            caps: text(n, "capabilities").unwrap_or_default(),
        });
    }
    if let Some(sys) = v.get("system").and_then(Value::as_object) {
        for (k, val) in sys {
            let (k, val) = (k.to_ascii_lowercase(), val.as_str().unwrap_or_default());
            if val.is_empty() {
                continue;
            }
            let slot = if k.contains("serial") {
                &mut s.serial
            } else if k.contains("model") {
                &mut s.model
            } else if k.contains("firmware") || k.contains("romversion") {
                &mut s.firmware
            } else if k.contains("firewallname") || k.contains("prodname") {
                &mut s.name
            } else {
                continue;
            };
            slot.get_or_insert_with(|| val.to_string());
        }
    }
    for i in v.get("interfaces").and_then(Value::as_array)? {
        s.ifaces.push(Iface {
            name: text(i, "name").unwrap_or_default(),
            zone: text(i, "zone").unwrap_or_default(),
            ip: ipv4(i, "ip").filter(|ip| !ip.is_unspecified()),
            mask: text(i, "mask"),
            mac: text(i, "mac").and_then(|m| mac::parse_mac(&m)),
            mgmt: i
                .get("management")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    Some(s)
}

/// A firewall from one or more of the exports open.
struct Firewall<'a> {
    s: Sonic,
    /// The file it was read from last.
    file: &'a str,
    /// That file's place in `Input::sonicwall`.
    last: usize,
}

impl Sonic {
    /// The same box: one serial, or an interface MAC in common.
    fn same_box(&self, o: &Sonic) -> bool {
        if let (Some(a), Some(b)) = (&self.serial, &o.serial) {
            return a == b;
        }
        self.ifaces
            .iter()
            .filter_map(|f| f.mac)
            .any(|m| o.ifaces.iter().any(|g| g.mac == Some(m)))
    }

    /// Its LAN-side networks: every interface with an address and mask
    /// that isn't on a WAN.
    fn lans(&self) -> Vec<(Ipv4Addr, u8)> {
        self.ifaces
            .iter()
            .filter(|f| !f.zone.to_ascii_lowercase().contains("wan"))
            .filter_map(|f| Some((f.ip?, prefix(f.mask.as_deref()?)?)))
            .collect()
    }
}

/// Every SonicWall export open, one per firewall, in the order first
/// opened. A settings export and a tech support report of one box are one
/// firewall: the later file's config, with what either heard.
fn read_sonicwall<'a>(files: &[Loaded<'a>]) -> Vec<Firewall<'a>> {
    let mut out: Vec<Firewall> = Vec::new();
    for (k, f) in files.iter().enumerate() {
        let Some(mut s) = parse_sonicwall(f.json) else {
            continue;
        };
        match out.iter_mut().find(|o| o.s.same_box(&s)) {
            Some(o) => {
                let old = std::mem::take(&mut o.s.heard);
                for h in old {
                    if !s
                        .heard
                        .iter()
                        .any(|n| (n.mac, n.ip, &n.iface) == (h.mac, h.ip, &h.iface))
                    {
                        s.heard.push(h);
                    }
                }
                *o = Firewall {
                    s,
                    file: f.name,
                    last: k,
                };
            }
            None => out.push(Firewall {
                s,
                file: f.name,
                last: k,
            }),
        }
    }
    out
}

// ---------------------------------------------------------- classification

const GEAR: &[(&str, &str)] = &[
    ("ubiquiti", "UBIQUITI"),
    ("meraki", "MERAKI"),
    ("cisco", "CISCO"),
    ("aruba", "ARUBA"),
    ("hewlett packard enterprise", "HPE"),
    ("juniper", "JUNIPER"),
    ("routerboard", "MIKROTIK"),
    ("mikrotik", "MIKROTIK"),
    ("fortinet", "FORTINET"),
    ("sonicwall", "SONICWALL"),
    ("ruckus", "RUCKUS"),
    ("cambium", "CAMBIUM"),
    ("extreme networks", "EXTREME"),
    ("brocade", "BROCADE"),
    ("zyxel", "ZYXEL"),
    ("draytek", "DRAYTEK"),
    ("watchguard", "WATCHGUARD"),
    ("palo alto", "PALO ALTO"),
    ("sophos", "SOPHOS"),
    ("allied telesis", "ALLIED TELESIS"),
    ("netgear", "NETGEAR"),
    ("tp-link", "TP-LINK"),
    ("d-link", "D-LINK"),
    ("engenius", "ENGENIUS"),
    ("peplink", "PEPLINK"),
    ("cradlepoint", "CRADLEPOINT"),
    ("arista", "ARISTA"),
];
const NAS: &[&str] = &[
    "synology",
    "qnap",
    "asustor",
    "buffalo",
    "western digital",
    "drobo",
    "terramaster",
    "netapp",
];
const PRINTERS: &[&str] = &[
    "brother", "canon", "epson", "lexmark", "xerox", "kyocera", "ricoh", "konica", "zebra",
];
const CAMERAS: &[&str] = &[
    "hikvision",
    "dahua",
    "axis comm",
    "amcrest",
    "reolink",
    "hanwha",
    "uniview",
    "vivotek",
    "avigilon",
    "mobotix",
    "verkada",
];

fn has(vendor: Option<&str>, words: &[&str]) -> bool {
    let v = vendor.unwrap_or_default().to_ascii_lowercase();
    words.iter().any(|w| v.contains(w))
}

fn gear_brand(vendor: Option<&str>) -> Option<&'static str> {
    let v = vendor?.to_ascii_lowercase();
    GEAR.iter().find(|(k, _)| v.contains(k)).map(|g| g.1)
}

/// First word of a registry name, for a card's kind line.
fn short_vendor(v: &str) -> String {
    v.split([' ', ',', '.'])
        .find(|w| !w.is_empty())
        .unwrap_or(v)
        .to_uppercase()
}

/// Upper case, at most 18 characters, cut at a word when one is close.
fn caps(s: &str) -> String {
    let s = s.trim().to_uppercase();
    let Some((i, next)) = s.char_indices().nth(18) else {
        return s;
    };
    let head = &s[..i];
    match head.rfind(' ') {
        Some(sp) if next != ' ' && sp >= 8 => head[..sp].to_string(),
        _ => head.trim_end().to_string(),
    }
}

/// Makers of the network cards in laptops, desktops and phones: a host
/// with one is someone's computer or phone, not gear. (Quanta Computer
/// only: Quanta Network Systems makes switches.)
const CLIENT_NICS: &[&str] = &[
    "apple",
    "samsung electronics",
    "intel corporate",
    "realtek",
    "liteon",
    "azurewave",
    "hon hai",
    "chicony",
    "murata",
    "microsoft",
    "lcfc",
    "wistron",
    "quanta computer",
    "compal",
    "pegatron",
    "huawei device",
    "motorola mobility",
    "oneplus",
    "xiaomi communications",
    "lenovo",
];

/// Whether a swept host's management ports are worth a look (the sweep's
/// Smart check), from its MAC: yes for gear, NAS, printers and cameras
/// (their admin pages, a telnet left open) and for a maker this list
/// doesn't know or no MAC at all (ports are what tell its role then); no
/// for a private MAC or a laptop or phone network card's maker.
pub fn ports_worth_checking(vendor: Option<&str>, private: bool) -> bool {
    if private {
        return false;
    }
    let infra = gear_brand(vendor).is_some()
        || has(vendor, NAS)
        || has(vendor, PRINTERS)
        || has(vendor, CAMERAS);
    infra || vendor.is_none() || !has(vendor, CLIENT_NICS)
}

/// Role of a host the sweep found and nothing else describes.
fn classify(vendor: Option<&str>, private: bool, open: &[u16]) -> (Zone, String, bool) {
    let p = |x| open.contains(&x);
    if let Some(b) = gear_brand(vendor) {
        return (Zone::Gear, caps(&format!("{b} DEVICE")), false);
    }
    if p(23) || p(8291) || p(541) || p(4433) {
        return (Zone::Gear, "NETWORK DEVICE".into(), false);
    }
    if has(vendor, NAS) || p(5000) {
        let k = vendor.map_or("NAS".into(), |v| caps(&format!("{} NAS", short_vendor(v))));
        return (Zone::Server, k, false);
    }
    if has(vendor, PRINTERS) || p(9100) {
        return (Zone::Device, "PRINTER".into(), true);
    }
    if has(vendor, CAMERAS) || p(554) {
        return (Zone::Device, "CAMERA".into(), true);
    }
    let kind = match vendor {
        Some(v) => caps(&short_vendor(v)),
        None if private => "PRIVATE MAC".into(),
        None => "HOST".into(),
    };
    (Zone::Client, kind, true)
}

/// Role of a device the firewall heard, from its LLDP capabilities and
/// description; an LLTD talker is a Windows host.
fn heard_role(h: &Heard) -> (Zone, String) {
    if h.lltd {
        return (Zone::Client, "WINDOWS HOST".into());
    }
    let can = h.caps.to_ascii_lowercase();
    let model = h
        .description
        .as_deref()
        .and_then(|d| d.split([',', ';']).next())
        .map(str::trim)
        .filter(|m| !m.is_empty() && m.len() <= 24);
    let vendor = h.mac.as_ref().and_then(mac::vendor);
    let named = |word: &str| {
        caps(&match model {
            Some(m) => format!("{word} {m}"),
            None => word.to_string(),
        })
    };
    if can.contains("wlan") || can.contains("access point") {
        (Zone::Wireless, named("AP"))
    } else if can.contains("bridge") || can.contains("mac-relay") || can.contains("switch") {
        (Zone::Switch, named("SWITCH"))
    } else if can.contains("telephone") || can.contains("phone") {
        (Zone::Client, named("PHONE"))
    } else if can.contains("router") {
        (Zone::Gear, named("ROUTER"))
    } else {
        let word = vendor.map_or("NETWORK DEVICE".into(), |v| {
            format!("{} DEVICE", short_vendor(v))
        });
        (Zone::Gear, caps(&word))
    }
}

fn unifi_role(kind: &str, model: &str) -> (Zone, String) {
    let (zone, word) = match kind {
        "usw" => (Zone::Switch, "SWITCH"),
        "uap" => (Zone::Wireless, "AP"),
        "udb" => (Zone::Wireless, "BRIDGE"),
        k if is_unifi_gateway(k) => (Zone::Gateway, "GATEWAY"),
        _ => (Zone::Gear, "UNIFI"),
    };
    (zone, caps(&format!("{word} {model}")))
}

fn is_unifi_gateway(kind: &str) -> bool {
    matches!(kind, "ugw" | "udm" | "uxg" | "ucg" | "udr")
}

fn port_names(open: &[u16]) -> String {
    open.iter()
        .map(|p| format!("{p} {}", crate::net::ports::service_name(*p)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A card for a host the sweep found, not yet hung anywhere. `this`: the
/// machine the sweep ran from.
fn sweep_card(s: &Seen, this: bool) -> Node {
    let vendor = s.mac.as_ref().and_then(mac::vendor);
    let private = s.mac.as_ref().is_some_and(mac::local_admin);
    let (zone, kind, client) = if this {
        (Zone::Client, "THIS MACHINE".into(), false)
    } else {
        classify(vendor, private, &s.open)
    };
    let label = match (&s.name, this) {
        (_, true) => "This machine".into(),
        (Some(n), _) => n.split('.').next().unwrap_or(n).to_string(),
        (None, _) => match vendor {
            Some(v) => format!(
                "{} device",
                short_vendor(v).to_lowercase().replace_first_upper()
            ),
            None => format!("Host {}", s.ip),
        },
    };
    let mut n = Node {
        id: format!("ip:{}", s.ip),
        zone,
        kind,
        label,
        mac: s.mac,
        client,
        rank: (
            known(s.name.as_deref(), vendor.is_some()),
            u32::from(s.ip) as u64,
        ),
        ..Default::default()
    };
    if let Some(m) = &s.mac {
        n.details.push(
            format!("MAC {} {}", mac::fmt(m), mac::vendor_label(m))
                .trim_end()
                .to_string(),
        );
    } else if !this {
        n.details
            .push("No MAC: past a router, or this machine".into());
    }
    if zone == Zone::Gear || zone == Zone::Server {
        n.chip("inferred");
        n.details
            .push("Role inferred from vendor and open ports".into());
    }
    n
}

/// A card for a UniFi client, not yet hung anywhere. Its name is what the
/// backup holds: a client-xxxxxxxx token when Config Export anonymized it.
fn unifi_client_card(c: &UClient) -> Node {
    let vendor = c.oui.as_deref();
    // Infrastructure hiding in the client table (an unadopted AP,
    // a NAS) is shown even with clients off.
    let (zone, kind, client) = match classify(vendor, false, &[]) {
        (Zone::Client, _, _) if vendor.is_none() => (Zone::Client, "CLIENT".into(), true),
        r => r,
    };
    let mut n = Node {
        id: format!("mac:{}", mac::fmt(&c.mac)),
        zone,
        kind,
        label: c.name.clone().unwrap_or_else(|| mac::fmt(&c.mac)),
        sub: c.ip.map(|i| i.to_string()).unwrap_or_default(),
        ip: c.ip,
        mac: Some(c.mac),
        client,
        seen_by: vec!["UniFi"],
        rank: (
            known(c.name.as_deref(), vendor.is_some()),
            u64::MAX - c.last_seen.max(0) as u64,
        ),
        ..Default::default()
    };
    n.details.push(format!("MAC {}", mac::fmt(&c.mac)));
    if let Some(v) = vendor {
        n.details.push(format!("Vendor {v} (UniFi)"));
    }
    n.details
        .push(if c.wired { "Wired" } else { "Wireless" }.into());
    if c.guest {
        n.chip("guest");
    }
    if !client {
        n.chip("in client table");
    }
    if c.blocked {
        n.flags.push("Blocked in the controller".into());
    }
    n
}

// ------------------------------------------------------------------- build

/// A gateway card: a firewall, or a controller's UniFi gateway.
struct Gw {
    i: usize,
    /// The networks behind it.
    lans: Vec<(Ipv4Addr, u8)>,
    /// The controller site a UniFi gateway is at.
    site: Option<String>,
}

#[derive(Default)]
struct B {
    nodes: Vec<Node>,
    by_mac: HashMap<Mac, usize>,
    by_ip: HashMap<Ipv4Addr, usize>,
    /// (severity, text, node ids): resolved to indices once the set is final.
    findings: Vec<(Sev, String, Vec<String>)>,
    /// Every gateway card from the data (not the stand-in for none).
    gws: Vec<Gw>,
    /// Each controller site's gateway addresses: its networks' and its
    /// UniFi gateways'.
    site_addrs: HashMap<String, Vec<Ipv4Addr>>,
    /// The gateway each controller site's devices plug straight into, when
    /// they all agree.
    site_gw: HashMap<String, Option<usize>>,
    /// Several backups or exports: findings say so.
    many: bool,
}

impl B {
    fn is_gw(&self, i: usize, gw: usize) -> bool {
        i == gw || self.gws.iter().any(|g| g.i == i)
    }

    /// Where something with no uplink of its own hangs: with one gateway,
    /// under it (`gw`). With several, under the one whose LAN holds its
    /// address, else the one its controller site's devices plug into or
    /// whose LAN holds the site's gateway address, else its site's UniFi
    /// gateway, else `gw` (the firewall opened last).
    fn home(&self, ip: Option<Ipv4Addr>, site: Option<&str>, gw: usize) -> usize {
        if self.gws.len() < 2 {
            return gw;
        }
        // Only one may claim it: two sites can both use 192.168.1.x.
        let serving = |ip: Ipv4Addr| {
            let mut on = self
                .gws
                .iter()
                .filter(|g| g.lans.iter().any(|&n| on_net(ip, n)));
            match (on.next(), on.next()) {
                (Some(g), None) => Some(g.i),
                _ => None,
            }
        };
        if let Some(g) = ip.and_then(serving) {
            return g;
        }
        let Some(site) = site else { return gw };
        if let Some(&Some(g)) = self.site_gw.get(site) {
            return g;
        }
        let addrs = self.site_addrs.get(site).map_or(&[][..], Vec::as_slice);
        if let Some(g) = addrs.iter().find_map(|&a| serving(a)) {
            return g;
        }
        self.gws
            .iter()
            .find(|g| g.site.as_deref() == Some(site))
            .map_or(gw, |g| g.i)
    }

    /// The gateway card at `ip` (this machine's default route), else `gw`.
    fn gw_at(&self, ip: Option<Ipv4Addr>, gw: usize) -> usize {
        ip.and_then(|ip| self.by_ip.get(&ip).copied())
            .filter(|&i| self.gws.iter().any(|g| g.i == i))
            .unwrap_or(gw)
    }

    fn add(&mut self, n: Node) -> usize {
        let i = self.nodes.len();
        if let Some(m) = n.mac {
            self.by_mac.entry(m).or_insert(i);
        }
        if let Some(ip) = n.ip {
            self.by_ip.entry(ip).or_insert(i);
        }
        self.nodes.push(n);
        i
    }

    fn find(&self, mac: Option<Mac>, ip: Option<Ipv4Addr>) -> Option<usize> {
        mac.and_then(|m| self.by_mac.get(&m))
            .or_else(|| ip.and_then(|i| self.by_ip.get(&i)))
            .copied()
    }

    fn finding(&mut self, sev: Sev, text: String, nodes: &[usize]) {
        let ids = nodes.iter().map(|&i| self.nodes[i].id.clone()).collect();
        self.findings.push((sev, text, ids));
    }

    /// Fold what the sweep saw into a card. Another address answering
    /// with the card's MAC (a second IP on one network card, an alias)
    /// lands here too: its lines say which address, and a line the card
    /// already has (the same DNS name) isn't added again.
    fn merge_seen(&mut self, i: usize, s: &Seen) {
        let n = &mut self.nodes[i];
        n.seen("sweep");
        if n.ip.is_none() {
            n.ip = Some(s.ip);
            self.by_ip.entry(s.ip).or_insert(i);
        }
        if n.mac.is_none() {
            n.mac = s.mac;
        }
        if n.sub.is_empty() {
            n.sub = s.ip.to_string();
        }
        let own = n.ip == Some(s.ip);
        let at = |line: String| {
            if own {
                line
            } else {
                format!("Also at {}: {line}", s.ip)
            }
        };
        n.detail(at(match s.rtt {
            Some(r) => format!("Answered ping in {}", crate::widgets::fmt_ms(r)),
            None => "Answered ARP, not ping (firewalled or ICMP off)".into(),
        }));
        // About the card's own address: another one answering only ARP
        // doesn't make the card ARP only.
        if s.rtt.is_none() && own {
            n.chip("ARP only");
        }
        if let Some(name) = &s.name {
            let line = format!("DNS name {name}");
            if !n.details.contains(&line) {
                n.detail(at(line));
            }
        }
        n.ports_checked |= s.checked;
        if !s.open.is_empty() {
            n.detail(at(format!("Open ports: {}", port_names(&s.open))));
            for p in &s.open {
                if !n.open.contains(p) {
                    n.open.push(*p);
                }
            }
        }
    }

    /// The firewall as a gateway card, from its own config, with a finding
    /// for any WAN interface that allows management. `first`: the first
    /// firewall opened, whose card is "gateway" as when it's the only one.
    fn sonic_gateway(&mut self, s: &Sonic, input: &Input, internet: usize, first: bool) -> usize {
        let lan = s
            .ifaces
            .iter()
            .find(|f| f.ip.is_some() && f.ip == input.gateway)
            .or_else(|| {
                s.ifaces
                    .iter()
                    .find(|f| f.ip.is_some() && f.zone.eq_ignore_ascii_case("lan"))
            });
        let id = match (first, lan.and_then(|f| f.mac)) {
            (true, _) => "gateway".to_string(),
            (false, Some(m)) => format!("gateway:{}", mac::fmt(&m)),
            (false, None) => format!("gateway:{}", self.gws.len() + 1),
        };
        let mut n = Node {
            id,
            zone: Zone::Gateway,
            kind: caps(&format!("SONICWALL {}", s.model.as_deref().unwrap_or(""))),
            label: s
                .name
                .clone()
                .or_else(|| s.model.clone())
                .unwrap_or_else(|| "SonicWall".into()),
            sub: lan
                .and_then(|f| f.ip)
                .map(|i| i.to_string())
                .unwrap_or_default(),
            ip: lan.and_then(|f| f.ip),
            mac: lan.and_then(|f| f.mac),
            seen_by: vec!["SonicWall"],
            up: Some((internet, Link::Backbone, None)),
            ..Default::default()
        };
        for (k, v) in [
            ("Model", &s.model),
            ("Firmware", &s.firmware),
            ("Serial", &s.serial),
        ] {
            if let Some(v) = v {
                n.details.push(format!("{k} {v}"));
            }
        }
        for f in &s.ifaces {
            let mut l = format!("{} {}", f.name, f.zone);
            if let Some(ip) = f.ip {
                l += &format!(" {ip}");
                if let Some(m) = &f.mask {
                    l += &format!(" / {m}");
                }
            }
            if !f.mgmt.is_empty() {
                l += &format!(" (mgmt: {})", f.mgmt.join(", "));
            }
            n.details.push(l);
        }
        let i = self.add(n);
        self.gws.push(Gw {
            i,
            lans: s.lans(),
            site: None,
        });
        for f in &s.ifaces {
            if let Some(m) = f.mac {
                self.by_mac.entry(m).or_insert(i);
            }
            if let Some(ip) = f.ip {
                self.by_ip.entry(ip).or_insert(i);
            }
        }
        for f in &s.ifaces {
            let open: Vec<&str> = f
                .mgmt
                .iter()
                .map(String::as_str)
                .filter(|m| matches!(*m, "http" | "https" | "ssh"))
                .collect();
            if f.zone.to_ascii_lowercase().contains("wan") && !open.is_empty() {
                self.finding(
                    Sev::High,
                    format!(
                        "{} ({}) allows {} management from the internet: limit it to LAN or VPN.",
                        f.name,
                        f.zone,
                        open.join("/").to_uppercase()
                    ),
                    &[i],
                );
                self.nodes[i].flags.push(format!(
                    "{} management on {}",
                    open.join("/").to_uppercase(),
                    f.name
                ));
            }
        }
        i
    }

    /// UniFi devices. With no firewall, each backup's first UniFi gateway
    /// takes the gateway role, and the newest backup's is `gw`.
    fn unifi_devices(&mut self, u: &Unifi, internet: usize, gw: &mut Option<usize>) -> Vec<usize> {
        let firewall = gw.is_some();
        let mut promoted: HashSet<usize> = HashSet::new();
        for d in u.devices.iter().filter(|d| is_unifi_gateway(&d.kind)) {
            if let (Some(site), Some(ip)) = (&d.site_id, d.ip) {
                self.site_addrs.entry(site.clone()).or_default().push(ip);
            }
        }
        for n in &u.networks {
            if let (Some(site), Some((ip, _))) = (&n.site_id, n.gw) {
                self.site_addrs.entry(site.clone()).or_default().push(ip);
            }
        }
        let mut device_idx: Vec<usize> = Vec::new();
        for d in &u.devices {
            let (zone, kind) = unifi_role(&d.kind, &d.model);
            let mut n = Node {
                id: format!("mac:{}", mac::fmt(&d.mac)),
                zone,
                kind,
                label: d.name.clone(),
                sub: d.ip.map(|i| i.to_string()).unwrap_or_default(),
                ip: d.ip,
                mac: Some(d.mac),
                seen_by: vec!["UniFi"],
                ..Default::default()
            };
            // A multi-site controller: each device at its own site.
            if u.site_ids.len() > 1 {
                n.site = d.site_id.as_ref().and_then(|id| {
                    u.site_ids
                        .iter()
                        .find(|(k, _)| k == id)
                        .map(|(_, name)| name.clone())
                });
                if let Some(site) = n.site.clone() {
                    n.chip(site.clone());
                    n.details.push(format!("Site: {site}"));
                }
            }
            n.details.push(format!("Model {}", d.model));
            n.details.push(format!("MAC {}", mac::fmt(&d.mac)));
            if let Some(v) = &d.version {
                n.details.push(format!("Firmware {v}"));
            }
            if let Some(s) = &d.serial {
                n.details.push(format!("Serial {s}"));
            }
            if d.ports > 0 {
                n.chip(format!("{} ports", d.ports));
            }
            if d.eol {
                n.flags.push("End-of-life model".into());
            }
            if d.disabled {
                n.flags.push("Disabled in the controller".into());
            }
            let gateway_type = is_unifi_gateway(&d.kind);
            let i = self.add(n);
            if gateway_type && !firewall && promoted.insert(d.file) {
                self.nodes[i].up = Some((internet, Link::Backbone, None));
                if gw.is_none() {
                    self.nodes[i].id = "gateway".into();
                }
                *gw = Some(i);
                let lans = u
                    .networks
                    .iter()
                    .filter(|n| d.site_id.is_some() && n.site_id == d.site_id)
                    .filter_map(|n| n.gw)
                    .collect();
                self.gws.push(Gw {
                    i,
                    lans,
                    site: d.site_id.clone(),
                });
            }
            device_idx.push(i);
        }
        device_idx
    }

    /// The UniFi networks, in the details of the gateway they're behind
    /// (with one gateway from the data, all of them there).
    fn unifi_networks(&mut self, u: &Unifi, gw: usize) {
        if u.networks.is_empty() || self.gws.is_empty() {
            return;
        }
        let mut at: Vec<(usize, Vec<&str>)> = Vec::new();
        for n in &u.networks {
            let g = self.home(n.gw.map(|g| g.0), n.site_id.as_deref(), gw);
            match at.iter_mut().find(|a| a.0 == g) {
                Some(a) => a.1.push(&n.line),
                None => at.push((g, vec![&n.line])),
            }
        }
        for (g, lines) in at {
            self.nodes[g].details.push("Networks (UniFi):".into());
            self.nodes[g]
                .details
                .extend(lines.iter().map(|n| format!("  {n}")));
        }
    }

    /// Still none: this machine's default route, or a placeholder holding
    /// the tree when there's data but no gateway in it.
    fn fallback_gateway(
        &mut self,
        input: &Input,
        has_data: bool,
        gw: Option<usize>,
        internet: usize,
    ) -> usize {
        let gw = match gw {
            Some(g) => g,
            None if input.gateway.is_some() || has_data => self.add(Node {
                id: "gateway".into(),
                zone: Zone::Gateway,
                kind: "GATEWAY".into(),
                label: if input.gateway.is_some() {
                    "Gateway".into()
                } else {
                    "Gateway (not in the data)".into()
                },
                sub: input.gateway.map(|g| g.to_string()).unwrap_or_default(),
                ip: input.gateway,
                details: input
                    .gateway
                    .map(|g| format!("Default gateway of this machine: {g}"))
                    .into_iter()
                    .collect(),
                up: Some((internet, Link::Backbone, None)),
                ..Default::default()
            }),
            None => internet,
        };
        if let Some(m) = input
            .gateway
            .and_then(|g| input.sweep.iter().find(|s| s.ip == g).and_then(|s| s.mac))
        {
            let at = self.gw_at(input.gateway, gw);
            self.by_mac.entry(m).or_insert(at);
            if self.nodes[at].mac.is_none() {
                self.nodes[at].mac = Some(m);
            }
        }
        gw
    }

    /// UniFi uplinks: each device under the one it reports, with mutual
    /// uplinks and unmanaged upstream devices called out.
    fn unifi_uplinks(
        &mut self,
        u: &Unifi,
        device_idx: &[usize],
        gw: usize,
        seen_mac: &HashMap<Mac, &Seen>,
    ) {
        let resolved: Vec<Option<usize>> = u
            .devices
            .iter()
            .map(|d| d.up.and_then(|(m, ..)| self.by_mac.get(&m).copied()))
            .collect();
        let children = |p: usize| resolved.iter().filter(|r| **r == Some(p)).count();
        let mut cut = HashSet::new();
        for a in 0..u.devices.len() {
            let (ia, Some(ib)) = (device_idx[a], resolved[a]) else {
                continue;
            };
            // Each other's uplink: one record is stale, or there's a loop.
            if let Some(bpos) = device_idx.iter().position(|&x| x == ib)
                && resolved[bpos] == Some(ia)
                && ia < ib
            {
                // The one with more below it goes on top and loses its uplink.
                let top = if children(ia) >= children(ib) { ia } else { ib };
                cut.insert(top);
                self.finding(
                    Sev::Med,
                    format!(
                        "UniFi lists {} and {} as each other's uplink: one record is stale or there's a loop. Drawn with {} on top; check the uplink port on both.",
                        self.nodes[ia].label, self.nodes[ib].label, self.nodes[top].label
                    ),
                    &[ia, ib],
                );
            }
        }
        // Which gateway each controller site plugs into, where its devices
        // that plug straight into one all agree.
        for (a, d) in u.devices.iter().enumerate() {
            if let (Some(site), Some(p)) = (&d.site_id, resolved[a])
                && p != device_idx[a]
                && self.gws.iter().any(|g| g.i == p)
            {
                let e = self.site_gw.entry(site.clone()).or_insert(Some(p));
                if *e != Some(p) {
                    *e = None;
                }
            }
        }
        let mut unresolved: BTreeMap<Mac, Vec<usize>> = BTreeMap::new();
        for (a, d) in u.devices.iter().enumerate() {
            let i = device_idx[a];
            if self.is_gw(i, gw) || cut.contains(&i) {
                continue;
            }
            let Some((um, port, wireless)) = d.up else {
                continue;
            };
            let link = if wireless {
                Link::Wireless
            } else {
                Link::Wired
            };
            let label = port.filter(|p| *p > 0).map(|p| format!("port {p}"));
            match resolved[a] {
                Some(p) => self.nodes[i].up = Some((p, link, label)),
                None => {
                    unresolved.entry(um).or_default().push(i);
                    self.nodes[i].up = Some((usize::MAX, link, label));
                }
            }
        }
        // Where a device with no uplink of its own goes.
        let homes: HashMap<usize, usize> = u
            .devices
            .iter()
            .zip(device_idx)
            .map(|(d, &i)| (i, self.home(d.ip, d.site_id.as_deref(), gw)))
            .collect();
        // An uplink MAC nothing here manages: a real device we can't name.
        for (um, kids) in unresolved {
            let s = seen_mac.get(&um);
            let vendor = mac::vendor(&um);
            let mut n = Node {
                id: format!("mac:{}", mac::fmt(&um)),
                zone: Zone::Unknown,
                kind: vendor.map_or("UNKNOWN DEVICE".into(), |v| {
                    caps(&format!("{} DEVICE", short_vendor(v)))
                }),
                label: s
                    .and_then(|s| s.name.clone())
                    .unwrap_or_else(|| "Unidentified upstream device".into()),
                sub: s.map_or_else(|| mac::fmt(&um), |s| s.ip.to_string()),
                ip: s.map(|s| s.ip),
                mac: Some(um),
                up: Some((homes[&kids[0]], Link::Assumed, None)),
                ..Default::default()
            };
            n.chip("not managed");
            n.details.push(format!("MAC {}", mac::fmt(&um)));
            n.details.push(match vendor {
                Some(v) => format!("Vendor {v} (MAC prefix; confirm on site)"),
                None => "Vendor unknown (MAC prefix not registered)".into(),
            });
            let names: Vec<_> = kids.iter().map(|&k| self.nodes[k].label.clone()).collect();
            n.details.push(format!("Uplink of {}", names.join(", ")));
            let d = self.add(n);
            for &k in &kids {
                if let Some(up) = &mut self.nodes[k].up {
                    up.0 = d;
                }
            }
            self.finding(
                Sev::Med,
                format!(
                    "{} uplink{} to {}{}, which {} describes: identify it on site.",
                    names.join(", "),
                    if kids.len() == 1 { "s" } else { "" },
                    mac::fmt(&um),
                    vendor.map(|v| format!(" ({v})")).unwrap_or_default(),
                    if self.many {
                        "none of the UniFi backups or firewall exports"
                    } else {
                        "neither the UniFi backup nor the firewall export"
                    }
                ),
                &[d],
            );
        }
        // No uplink recorded, or cut from a pair: under the gateway.
        for &i in device_idx {
            if self.nodes[i].up.is_none() {
                self.nodes[i].up = Some((homes[&i], Link::Assumed, None));
                if !cut.contains(&i) {
                    self.finding(
                        Sev::Low,
                        format!(
                            "No uplink recorded for {}: drawn under the gateway.",
                            self.nodes[i].label
                        ),
                        &[i],
                    );
                }
            }
        }
    }

    /// EOL hardware and mixed firmware.
    fn unifi_firmware(&mut self, u: &Unifi, device_idx: &[usize]) {
        let eol: Vec<usize> = u
            .devices
            .iter()
            .zip(device_idx)
            .filter(|(d, _)| d.eol)
            .map(|(_, &i)| i)
            .collect();
        if !eol.is_empty() {
            let list: Vec<_> = eol
                .iter()
                .map(|&i| format!("{} ({})", self.nodes[i].label, self.nodes[i].kind))
                .collect();
            self.finding(
                Sev::Med,
                format!(
                    "End-of-life hardware: {}. Plan replacements; no more firmware fixes.",
                    list.join(", ")
                ),
                &eol,
            );
        }
        let mut fw: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for d in &u.devices {
            if let Some(v) = &d.version {
                fw.entry(&d.model).or_default().insert(v);
            }
        }
        for (model, versions) in fw.iter().filter(|(_, v)| v.len() > 1) {
            let nodes: Vec<usize> = u
                .devices
                .iter()
                .zip(device_idx)
                .filter(|(d, _)| d.model == *model)
                .map(|(_, &i)| i)
                .collect();
            self.finding(
                Sev::Low,
                format!(
                    "{model} units run {} firmware versions ({}): bring them level.",
                    versions.len(),
                    versions.iter().copied().collect::<Vec<_>>().join(", ")
                ),
                &nodes,
            );
        }
    }

    /// What the firewall heard on its ports. LLDP never crosses a switch, so
    /// an LLDP neighbour is plugged into that port: a real link, which beats
    /// an assumed one. LLTD does cross switches: a Windows host is somewhere
    /// on that port's network, so its link stays assumed.
    fn firewall_heard(&mut self, s: &Sonic, gw: usize, internet: usize) {
        for h in &s.heard {
            let at = match &h.port {
                Some(p) => format!("{}, its {p}", h.iface),
                None => h.iface.clone(),
            };
            let note = if h.lltd {
                format!(
                    "The firewall hears it on {}'s network (LLTD; the switch path isn't known)",
                    h.iface
                )
            } else {
                format!("Plugged into the firewall's {at} (LLDP)")
            };
            let link = if h.lltd { Link::Assumed } else { Link::Wired };
            match self.find(h.mac, h.ip) {
                Some(i) if i == gw || i == internet => {}
                Some(i) => {
                    let n = &mut self.nodes[i];
                    n.seen("firewall");
                    n.details.push(note);
                    if !h.lltd && n.up.as_ref().is_none_or(|u| u.1 == Link::Assumed) {
                        n.up = Some((gw, Link::Wired, Some(at)));
                        // Its uplink is known now.
                        let id = n.id.clone();
                        self.findings.retain(|(_, text, ids)| {
                            !(text.starts_with("No uplink recorded")
                                && ids.len() == 1
                                && ids[0] == id)
                        });
                    }
                }
                None => {
                    let (zone, kind) = heard_role(h);
                    let mut n = Node {
                        id: match (h.mac, h.ip) {
                            (Some(m), _) => format!("mac:{}", mac::fmt(&m)),
                            (None, Some(ip)) => format!("ip:{ip}"),
                            _ => continue,
                        },
                        zone,
                        kind,
                        label: h
                            .name
                            .clone()
                            .or_else(|| h.ip.map(|ip| ip.to_string()))
                            .or_else(|| h.mac.map(|m| mac::fmt(&m)))
                            .unwrap_or_default(),
                        sub: h.ip.map(|ip| ip.to_string()).unwrap_or_default(),
                        ip: h.ip,
                        mac: h.mac,
                        client: zone == Zone::Client,
                        rank: (
                            known(
                                h.name.as_deref(),
                                h.mac.as_ref().and_then(mac::vendor).is_some(),
                            ),
                            h.ip.map_or(u64::MAX, |ip| u32::from(ip) as u64),
                        ),
                        up: Some((gw, link, (!h.lltd).then_some(at))),
                        ..Default::default()
                    };
                    n.seen("firewall");
                    n.details.push(note);
                    if let Some(d) = &h.description {
                        n.details.push(d.clone());
                    }
                    if let Some(m) = h.mac {
                        n.details.push(format!("MAC {}", mac::fmt(&m)));
                    }
                    self.add(n);
                }
            }
        }
    }

    /// With several gateways, each heads a site: one with no site named on
    /// it or below it takes the UniFi site below it (a one-site backup's),
    /// else its own name, with its file's name when two would share one.
    /// `fw`: each firewall's card.
    fn gateway_sites(&mut self, sonic: &[Firewall], fw: &[usize], unifi: Option<&Unifi>) {
        if self.gws.len() < 2 {
            return;
        }
        let n = self.nodes.len();
        let mut kids = vec![Vec::new(); n];
        for (i, node) in self.nodes.iter().enumerate() {
            if let Some(p) = node.parent().filter(|&p| p < n) {
                kids[p].push(i);
            }
        }
        let one_site = unifi.and_then(|u| match u.sites.as_slice() {
            [s] => Some(s.clone()),
            _ => None,
        });
        // Each card, and the UniFi site it takes (None: its own name).
        let mut unnamed: Vec<(usize, Option<String>)> = Vec::new();
        let mut labels: Vec<(String, String)> = Vec::new();
        for g in &self.gws {
            let mut seen = vec![false; n];
            let mut stack = vec![g.i];
            let (mut named, mut by_unifi) = (false, false);
            while let Some(k) = stack.pop() {
                if std::mem::replace(&mut seen[k], true) {
                    continue;
                }
                let c = &self.nodes[k];
                named |= c.site.is_some();
                by_unifi |= k != g.i && c.seen_by.contains(&"UniFi");
                stack.extend(&kids[k]);
            }
            if named {
                continue;
            }
            match (&one_site, by_unifi) {
                (Some(s), true) => unnamed.push((g.i, Some(s.clone()))),
                _ => {
                    let tag = match fw.iter().position(|&i| i == g.i) {
                        Some(k) => file_tag(sonic[k].file, k + 1),
                        None => self.nodes[g.i].sub.clone(),
                    };
                    unnamed.push((g.i, None));
                    labels.push((self.nodes[g.i].label.clone(), tag));
                }
            }
        }
        let mut shown = tell_apart(&labels).into_iter();
        for (i, site) in unnamed {
            self.nodes[i].site = site.or_else(|| shown.next());
        }
    }

    /// Sweep hosts. A client UniFi also knows is returned instead, to become
    /// its UniFi card under its AP or switch.
    fn sweep_hosts<'i>(
        &mut self,
        input: &'i Input,
        unifi: Option<&Unifi>,
        gw: usize,
        internet: usize,
    ) -> Vec<&'i Seen> {
        let unifi_client: HashSet<Mac> = unifi
            .map(|u| u.clients.iter().map(|c| c.mac).collect())
            .unwrap_or_default();
        let mut deferred = Vec::new();
        for s in &input.sweep {
            let is_gw = Some(s.ip) == input.gateway;
            let at = if is_gw {
                Some(self.gw_at(Some(s.ip), gw))
            } else {
                self.find(s.mac, Some(s.ip))
            };
            if let Some(i) = at.filter(|&i| i != internet) {
                self.merge_seen(i, s);
                continue;
            }
            let this = Some(s.ip) == input.this;
            let mut n = sweep_card(s, this);
            // A client UniFi knows becomes its card below, under its AP or switch.
            if n.client && s.mac.is_some_and(|m| unifi_client.contains(&m)) {
                deferred.push(s);
                continue;
            }
            n.up = Some((self.home(Some(s.ip), None, gw), Link::Assumed, None));
            let i = self.add(n);
            self.merge_seen(i, s);
        }
        if let Some(t) = input.this
            && !self.by_ip.contains_key(&t)
            && !input.is_empty()
        {
            let up = self.home(Some(t), None, gw);
            self.add(Node {
                id: format!("ip:{t}"),
                zone: Zone::Client,
                kind: "THIS MACHINE".into(),
                label: "This machine".into(),
                sub: t.to_string(),
                ip: Some(t),
                up: Some((up, Link::Assumed, None)),
                details: vec!["Where the sweep ran from".into()],
                ..Default::default()
            });
        }
        deferred
    }

    /// The card a UniFi client was last seen on (its AP or switch, when
    /// that's on the map), how it joins it, and the port.
    fn unifi_uplink(&self, c: &UClient) -> (Option<usize>, Link, Option<String>) {
        let parent = c.up.and_then(|(m, _)| self.by_mac.get(&m).copied());
        let link = match (parent, c.wired) {
            (None, _) => Link::Assumed,
            (Some(_), true) => Link::Wired,
            (Some(_), false) => Link::Wireless,
        };
        let port =
            c.up.and_then(|(_, p)| p)
                .filter(|p| *p > 0 && c.wired)
                .map(|p| format!("port {p}"));
        (parent, link, port)
    }

    /// UniFi clients: placed under the AP or switch they were last seen on.
    fn unifi_clients(&mut self, u: &Unifi, gw: usize, deferred: &[&Seen]) {
        for c in &u.clients {
            let (parent, link, port) = self.unifi_uplink(c);
            let up = Some((
                parent.unwrap_or_else(|| self.home(c.ip, c.site_id.as_deref(), gw)),
                link,
                port,
            ));
            if let Some(i) = self.find(Some(c.mac), None) {
                // Already a card (the sweep's): now we know where it plugs in.
                if !self.is_gw(i, gw) && self.nodes[i].link() == Some(Link::Assumed) {
                    self.nodes[i].up = up;
                }
                self.nodes[i].seen("UniFi");
                continue;
            }
            let mut n = unifi_client_card(c);
            n.up = up;
            let i = self.add(n);
            if let Some(s) = deferred.iter().find(|s| s.mac == Some(c.mac)) {
                self.merge_seen(i, s);
                // It answered just now.
                self.nodes[i].rank.1 = 0;
            }
        }
    }

    /// Findings the sweep's evidence supports.
    fn sweep_findings(&mut self, input: &Input, has_unifi: bool, device_idx: &[usize]) {
        let telnet: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| self.nodes[i].open.contains(&23))
            .collect();
        if !telnet.is_empty() {
            let list: Vec<_> = telnet.iter().map(|&i| describe(&self.nodes[i])).collect();
            self.finding(
            Sev::High,
            format!(
                "Telnet is open on {}: it sends logins in clear text. Turn it off; use SSH or HTTPS.",
                list.join(", ")
            ),
            &telnet,
        );
            for &i in &telnet {
                self.nodes[i].flags.push("Telnet open".into());
            }
        }
        let plain: Vec<usize> = (0..self.nodes.len())
            .filter(|&i| {
                let n = &self.nodes[i];
                matches!(
                    n.zone,
                    Zone::Gateway | Zone::Switch | Zone::Wireless | Zone::Gear
                ) && n.open.contains(&80)
                    && ![443, 8443, 4433].iter().any(|p| n.open.contains(p))
            })
            .collect();
        if !plain.is_empty() {
            let list: Vec<_> = plain.iter().map(|&i| describe(&self.nodes[i])).collect();
            self.finding(
                Sev::Low,
                format!(
                    "Management page only over plain HTTP on {}: enable HTTPS.",
                    list.join(", ")
                ),
                &plain,
            );
        }
        if input.ports_checked {
            let rdp: Vec<usize> = (0..self.nodes.len())
                .filter(|&i| self.nodes[i].open.contains(&3389))
                .collect();
            if rdp.len() > 1 {
                self.finding(
                Sev::Low,
                format!(
                    "Remote Desktop answers on {} hosts: make sure it isn't forwarded from the internet and needs NLA.",
                    rdp.len()
                ),
                &rdp,
            );
            }
        }
        if let (Some(range), true) = (&input.range, has_unifi) {
            let swept: HashSet<Ipv4Addr> = crate::net::targets::parse_range(range)
                .unwrap_or_default()
                .into_iter()
                .collect();
            let silent: Vec<usize> = device_idx
                .iter()
                .copied()
                .filter(|&i| {
                    let n = &self.nodes[i];
                    n.ip.is_some_and(|ip| swept.contains(&ip)) && !n.seen_by.contains(&"sweep")
                })
                .collect();
            if !silent.is_empty() {
                let list: Vec<_> = silent.iter().map(|&i| describe(&self.nodes[i])).collect();
                self.finding(
                Sev::Low,
                format!(
                    "In the swept range but no answer to ping or ARP: {}. Offline, or its address changed.",
                    list.join(", ")
                ),
                &silent,
            );
                for &i in &silent {
                    self.nodes[i].chip("no answer");
                }
            }
            let stray: Vec<usize> = (0..self.nodes.len())
                .filter(|&i| {
                    let n = &self.nodes[i];
                    n.seen_by == ["sweep"]
                        && n.mac
                            .as_ref()
                            .and_then(mac::vendor)
                            .is_some_and(|v| v.to_ascii_lowercase().contains("ubiquiti"))
                })
                .collect();
            if !stray.is_empty() {
                let list: Vec<_> = stray.iter().map(|&i| describe(&self.nodes[i])).collect();
                self.finding(
                    Sev::Low,
                    format!(
                        "Ubiquiti hardware {}: {}. Unadopted, or managed by another controller.",
                        if input.unifi.len() > 1 {
                            "in none of the UniFi backups"
                        } else {
                            "not in this backup"
                        },
                        list.join(", ")
                    ),
                    &stray,
                );
            }
        }
    }
}

pub fn build(input: &Input) -> Map {
    if let Some(doc) = input.mapnetwork {
        return import::build(doc, input);
    }
    let mut b = B {
        many: input.unifi.len() > 1 || input.sonicwall.len() > 1,
        ..Default::default()
    };
    let unifi = read_unifi(&input.unifi);
    let sonic = read_sonicwall(&input.sonicwall);
    // A backup is often read far from its site. What this machine sees only
    // joins the map when the sweep found hardware a backup or export knows
    // by MAC: the same network, not a look-alike 192.168.1.x.
    let local = (unifi.is_none() && sonic.is_empty()) || same_site(input, unifi.as_ref(), &sonic);
    let had_sweep = !input.sweep.is_empty();
    let away;
    let input = if local {
        input
    } else {
        away = Input {
            unifi: input.unifi.clone(),
            sonicwall: input.sonicwall.clone(),
            clients: input.clients,
            reveal: input.reveal,
            ..Default::default()
        };
        &away
    };
    let seen_mac: HashMap<Mac, &Seen> = input
        .sweep
        .iter()
        .filter_map(|s| Some((s.mac?, s)))
        .collect();

    let internet = b.add(Node {
        id: "internet".into(),
        zone: Zone::Internet,
        kind: "INTERNET".into(),
        label: "Internet".into(),
        sub: input.wan.map(|w| format!("WAN {w}")).unwrap_or_default(),
        details: input
            .wan
            .map(|w| format!("Public address {w}, as seen from this machine"))
            .into_iter()
            .collect(),
        ..Default::default()
    });

    // The gateway: the firewall's own config beats a UniFi gateway, which
    // beats the default route of this machine. Each firewall is a gateway
    // of its own; the one opened last takes what none other claims.
    let fw: Vec<usize> = sonic
        .iter()
        .enumerate()
        .map(|(k, f)| b.sonic_gateway(&f.s, input, internet, k == 0))
        .collect();
    let mut gw = sonic
        .iter()
        .zip(&fw)
        .max_by_key(|(f, _)| f.last)
        .map(|(_, &i)| i);
    let device_idx = match &unifi {
        Some(u) => b.unifi_devices(u, internet, &mut gw),
        None => Vec::new(),
    };
    let gw = b.fallback_gateway(input, unifi.is_some(), gw, internet);
    if let Some(u) = &unifi {
        b.unifi_uplinks(u, &device_idx, gw, &seen_mac);
        b.unifi_networks(u, gw);
        b.unifi_firmware(u, &device_idx);
    }
    for (f, &i) in sonic.iter().zip(&fw) {
        b.firewall_heard(&f.s, i, internet);
    }
    let deferred = b.sweep_hosts(input, unifi.as_ref(), gw, internet);
    if let Some(u) = &unifi {
        b.unifi_clients(u, gw, &deferred);
    }
    b.sweep_findings(input, unifi.is_some(), &device_idx);
    b.gateway_sites(&sonic, &fw, unifi.as_ref());

    let mut summary = summary(input, unifi.as_ref(), &sonic);
    if !local && had_sweep {
        summary += " · sweep not merged: it found no device the data knows (another network?)";
    } else if !local {
        summary += " · sweep here to add what this machine sees";
    }
    let mut map = limit(b, input);
    map.summary = summary;
    map.sweep_apart = !local && had_sweep;
    // One site unless a multi-site controller says otherwise: the
    // controller's site, the firewall's name, or this network.
    let site = unifi
        .as_ref()
        .and_then(|u| u.sites.first().cloned())
        .or_else(|| sonic.first().and_then(|f| f.s.name.clone()))
        .unwrap_or_else(|| "This network".into());
    map.fill_sites(&site);
    map.summary += &map.hidden_note();
    map
}

fn describe(n: &Node) -> String {
    match n.ip {
        Some(ip) if n.label != ip.to_string() => format!("{} ({ip})", n.label),
        _ => n.label.clone(),
    }
}

trait FirstUpper {
    fn replace_first_upper(&self) -> String;
}

impl FirstUpper for String {
    fn replace_first_upper(&self) -> String {
        let mut c = self.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default()
    }
}

/// Whether the sweep found hardware any backup or export knows by MAC.
fn same_site(input: &Input, unifi: Option<&Unifi>, sonic: &[Firewall]) -> bool {
    let mut macs: HashSet<Mac> = HashSet::new();
    if let Some(u) = unifi {
        macs.extend(u.devices.iter().map(|d| d.mac));
        macs.extend(u.devices.iter().filter_map(|d| d.up.map(|u| u.0)));
        macs.extend(u.clients.iter().map(|c| c.mac));
    }
    for s in sonic.iter().map(|f| &f.s) {
        macs.extend(s.ifaces.iter().filter_map(|f| f.mac));
        macs.extend(s.heard.iter().filter_map(|h| h.mac));
    }
    input
        .sweep
        .iter()
        .any(|h| h.mac.is_some_and(|m| macs.contains(&m)))
}

fn summary(input: &Input, unifi: Option<&Unifi>, sonic: &[Firewall]) -> String {
    let mut parts = Vec::new();
    match unifi {
        Some(u) if input.unifi.len() > 1 => {
            let sites = match u.sites.len() {
                n if n > 3 => format!("{n} sites"),
                _ => u.sites.join(", "),
            };
            parts.push(format!(
                "UniFi {sites} ({} devices from {} backups)",
                u.devices.len(),
                input.unifi.len()
            ));
        }
        Some(u) => {
            let site = u.sites.first().map(|s| format!(" {s}")).unwrap_or_default();
            parts.push(format!("UniFi{site} ({} devices)", u.devices.len()));
        }
        None => {}
    }
    match sonic {
        [] => {}
        [f] => parts.push(format!(
            "SonicWall {}",
            f.s.model.as_deref().unwrap_or("export")
        )),
        all => {
            let names: Vec<&str> = all
                .iter()
                .map(|f| {
                    f.s.name
                        .as_deref()
                        .or(f.s.model.as_deref())
                        .unwrap_or("export")
                })
                .collect();
            parts.push(format!("{} SonicWalls ({})", all.len(), names.join(", ")));
        }
    }
    match &input.range {
        Some(r) => parts.push(format!("sweep of {r} ({} hosts)", input.sweep.len())),
        None if !input.sweep.is_empty() => {
            parts.push(format!("sweep running ({} so far)", input.sweep.len()))
        }
        None => {}
    }
    parts.join(" · ")
}

/// Clients an uplink shows after a click on its +N: the next STEP, or all
/// of them (usize::MAX) when only a few would be left.
pub fn more_of(shown: usize, total: usize) -> usize {
    let next = shown.saturating_add(STEP);
    if next + STEP / 2 >= total {
        usize::MAX
    } else {
        next
    }
}

/// What − on an uplink leaves: the level's count again when it was
/// stepped past that (None: follow the level), else none of its clients.
pub fn fewer_of(stepped: Option<usize>, level: usize) -> Option<usize> {
    match stepped {
        Some(s) if s > level => None,
        _ => Some(0),
    }
}

/// Whether a card stands in for clients not drawn one by one: this app's
/// +N more card, or a /mapnetwork map's own "+7 more" that lists none of
/// them by address.
pub fn is_more(n: &Node) -> bool {
    n.id.starts_with("more:") || import::is_overflow(n)
}

/// The chip a card drawn by hand (on a plan or a training map) wears
/// before its others: "drawn", or "drawn link" when only its link up was.
/// A client's tile draws no chips: its hover and the panel say it.
pub fn drawn_chip(n: &Node) -> Option<&'static str> {
    if n.drawn {
        Some("drawn")
    } else if n.drawn_up {
        Some("drawn link")
    } else {
        None
    }
}

/// The chip on an uplink whose clients aren't drawn: "12 clients".
fn count_chip(n: usize) -> String {
    match n {
        1 => "1 client".into(),
        n => format!("{n} clients"),
    }
}

/// Whether a card's chip is its count of clients not drawn.
pub fn is_count_chip(c: &str) -> bool {
    c.strip_suffix(" clients")
        .or_else(|| c.strip_suffix(" client"))
        .is_some_and(|n| n.parse::<usize>().is_ok())
}

/// A client's name that's really an anonymized token (Config Export's
/// "UniFi clients: anonymized"): no more telling than no name.
fn is_token(name: &str) -> bool {
    name.strip_prefix("client-")
        .is_some_and(|h| h.len() == 8 && h.chars().all(|c| c.is_ascii_hexdigit()))
}

/// How much a client's source knows about it, for `Node::rank`: 0 a name,
/// 1 only a vendor, 2 nothing.
fn known(name: Option<&str>, vendor: bool) -> u8 {
    match name {
        Some(n) if !is_token(n) => 0,
        _ if vendor => 1,
        _ => 2,
    }
}

/// The order clients show in under an uplink: something to say first (a
/// warning, then a role such as a printer, camera or phone), then named,
/// then a vendor, anonymous last; the most recently seen first in each.
/// A card standing for more of them comes after them all.
fn reveal_key(n: &Node) -> (u8, u8, u64) {
    let role = n.zone != Zone::Client
        || ["PHONE", "WINDOWS HOST"]
            .iter()
            .any(|k| n.kind.starts_with(k));
    let tier = if is_more(n) {
        3
    } else if !n.flags.is_empty() {
        0
    } else if role {
        1
    } else {
        2
    };
    (tier, n.rank.0, n.rank.1)
}

/// Clients per uplink as the level says (or as the user stepped that
/// uplink), best first, tightening until the map fits the level's budget;
/// the rest go in a +N more card, or with none shown (Off) into a count on
/// the uplink.
fn limit(b: B, input: &Input) -> Map {
    let (level, budget) = input.clients.caps();
    let B {
        mut nodes,
        findings,
        ..
    } = b;
    let n = nodes.len();
    let stepped: Vec<Option<usize>> = nodes
        .iter()
        .map(|p| input.reveal.and_then(|r| r.get(&p.id).copied()))
        .collect();
    // A client with cards under it stays: they'd hang off nothing.
    let mut has_kids = vec![false; n];
    for node in &nodes {
        if let Some(p) = node.parent() {
            has_kids[p] = true;
        }
    }
    let mut by_parent: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, node) in nodes.iter().enumerate() {
        if node.client
            && !has_kids[i]
            && let Some(p) = node.parent()
        {
            by_parent.entry(p).or_default().push(i);
        }
    }
    for kids in by_parent.values_mut() {
        kids.sort_by_key(|&i| reveal_key(&nodes[i]));
    }
    let infra = n - by_parent.values().map(Vec::len).sum::<usize>();
    let cap_of = |p: usize, cap: usize| stepped[p].unwrap_or(cap);
    // Shows a +N more card for the rest: any uplink showing clients, and
    // every one once the level is past Off.
    let wants_more =
        |p: usize, cap: usize| cap_of(p, cap) > 0 || (level > 0 && stepped[p].is_none());
    let cost = |cap: usize| {
        infra
            + by_parent
                .iter()
                .map(|(&p, k)| {
                    let c = k.len().min(cap_of(p, cap));
                    c + usize::from(k.len() > c && wants_more(p, cap))
                })
                .sum::<usize>()
    };
    // Uplinks stepped by hand keep theirs, so the budget only tightens the rest.
    let mut cap = level;
    while cap > 0 && cost(cap) > budget {
        cap = match cap {
            c if c > PER_UPLINK => PER_UPLINK,
            c if c > 5 => 5,
            c if c > 3 => 3,
            c => c - 1,
        };
    }
    let fits = cost(cap) <= budget;
    let mut keep = vec![true; n];
    let mut folded = HashSet::new();
    let mut under = HashMap::new();
    let mut extra = Vec::new();
    let mut hidden = 0;
    for (&p, kids) in &by_parent {
        let shown = kids.len().min(cap_of(p, cap));
        under.insert(nodes[p].id.clone(), (shown, kids.len()));
        let rest = &kids[shown..];
        if rest.is_empty() {
            continue;
        }
        for &i in rest {
            keep[i] = false;
            folded.insert(nodes[i].id.clone());
        }
        hidden += rest.len();
        if wants_more(p, cap) && (fits || stepped[p].is_some()) {
            let mut more = Node {
                id: format!("more:{}", nodes[p].id),
                zone: Zone::Client,
                kind: "MORE CLIENTS".into(),
                label: format!("+{} more", rest.len()),
                sub: format!("via {}", nodes[p].label),
                client: true,
                up: Some((p, Link::Assumed, None)),
                // In its uplink's site box, with the clients it follows on from.
                site: nodes[p].site.clone(),
                ..Default::default()
            };
            more.chips = vec!["collapsed".into(), format!("{} devices", rest.len())];
            more.details = rest
                .iter()
                .take(60)
                .map(|&i| {
                    let c = &nodes[i];
                    [c.label.as_str(), c.sub.as_str()]
                        .iter()
                        .filter(|s| !s.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .collect();
            if rest.len() > 60 {
                more.details.push(format!("…and {} more", rest.len() - 60));
            }
            extra.push(more);
        } else {
            nodes[p].chip(count_chip(kids.len()));
        }
    }
    // Reindex.
    let mut map_to = vec![usize::MAX; n];
    let mut out = Vec::new();
    for (i, node) in nodes.into_iter().enumerate() {
        if keep[i] {
            map_to[i] = out.len();
            out.push(node);
        }
    }
    for node in &mut out {
        if let Some(up) = &mut node.up {
            up.0 = map_to[up.0];
        }
    }
    for mut more in extra {
        if let Some(up) = &mut more.up {
            up.0 = map_to[up.0];
        }
        out.push(more);
    }
    let index: HashMap<&str, usize> = out
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut findings: Vec<Finding> = findings
        .into_iter()
        .map(|(sev, text, ids)| Finding {
            sev,
            text,
            nodes: ids
                .iter()
                .filter_map(|id| index.get(id.as_str()).copied())
                .collect(),
        })
        .collect();
    findings.sort_by_key(|f| f.sev);
    Map {
        title: String::new(),
        nodes: out,
        findings,
        summary: String::new(),
        hidden,
        folded,
        closed: BTreeMap::new(),
        under,
        cap,
        sweep_apart: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn unifi(devices: &[Value], users: &[Value]) -> String {
        let mut v = vec![
            json!({"__cmd": "select", "collection": "site"}),
            json!({"desc": "HQ"}),
        ];
        v.push(json!({"__cmd": "select", "collection": "device"}));
        v.extend(devices.iter().cloned());
        v.push(json!({"__cmd": "select", "collection": "networkconf"}));
        v.push(json!({"name": "Voice", "vlan": 10}));
        if !users.is_empty() {
            v.push(json!({"__cmd": "select", "collection": "user"}));
            v.extend(users.iter().cloned());
        }
        Value::Array(v).to_string()
    }

    fn dev(name: &str, kind: &str, mac: &str, ip: &str, up: Option<(&str, i64)>) -> Value {
        let mut d = json!({"name": name, "type": kind, "model": "M", "mac": mac, "ip": ip,
            "version": "1.0"});
        if let Some((m, p)) = up {
            d["last_uplink"] = json!({"uplink_mac": m, "uplink_remote_port": p, "type": "wire"});
        }
        d
    }

    fn by_label<'a>(m: &'a Map, l: &str) -> &'a Node {
        m.nodes.iter().find(|n| n.label == l).expect(l)
    }

    fn parent<'a>(m: &'a Map, l: &str) -> &'a str {
        &m.nodes[by_label(m, l).parent().expect("has parent")].label
    }

    #[test]
    fn unifi_tree_with_reciprocal_pair_and_unknown_uplink() {
        let j = unifi(
            &[
                dev(
                    "Core",
                    "usw",
                    "00:00:00:00:00:01",
                    "10.0.0.2",
                    Some(("00:00:00:00:00:02", 18)),
                ),
                dev(
                    "Annex",
                    "usw",
                    "00:00:00:00:00:02",
                    "10.0.0.3",
                    Some(("00:00:00:00:00:01", 51)),
                ),
                dev(
                    "AP1",
                    "uap",
                    "00:00:00:00:00:03",
                    "10.0.0.4",
                    Some(("00:00:00:00:00:01", 3)),
                ),
                dev(
                    "AP2",
                    "uap",
                    "00:00:00:00:00:04",
                    "10.0.0.5",
                    Some(("00:00:00:00:00:01", 4)),
                ),
                dev(
                    "Shop",
                    "usw",
                    "00:00:00:00:00:05",
                    "10.0.0.6",
                    Some(("e4:38:83:00:00:09", 1)),
                ),
            ],
            &[],
        );
        let m = build(&Input {
            unifi: one(&j),
            gateway: Some("10.0.0.1".parse().unwrap()),
            ..Default::default()
        });
        assert_eq!(parent(&m, "Annex"), "Core", "more children wins the pair");
        assert_eq!(parent(&m, "Core"), "Gateway (not in the data)");
        assert_eq!(parent(&m, "AP1"), "Core");
        assert_eq!(
            by_label(&m, "AP1").up.as_ref().unwrap().2.as_deref(),
            Some("port 3")
        );
        assert_eq!(parent(&m, "Shop"), "Unidentified upstream device");
        assert_eq!(
            parent(&m, "Unidentified upstream device"),
            "Gateway (not in the data)"
        );
        assert!(
            m.findings
                .iter()
                .any(|f| f.text.contains("each other's uplink"))
        );
        assert!(
            m.findings
                .iter()
                .any(|f| f.text.contains("e4:38:83:00:00:09"))
        );
        // Every card reaches the internet.
        for i in 0..m.nodes.len() {
            assert_eq!(m.path(i)[0], 0, "{}", m.nodes[i].label);
        }
        assert!(m.summary.contains("UniFi HQ (5 devices)"));
    }

    /// One device answering on two addresses with one MAC (a second IP
    /// on its network card) is one card: each line once, the second
    /// address's own lines saying which address they're about.
    #[test]
    fn a_device_on_two_addresses_lists_each_detail_once() {
        let s = |ip: &str, rtt| Seen {
            ip: ip.parse().unwrap(),
            mac: mac::parse_mac("02:00:00:00:00:51"),
            name: Some("NAS".into()),
            rtt,
            open: vec![],
            checked: true,
        };
        let input = Input {
            sweep: vec![s("192.0.2.51", Some(0.36)), s("192.0.2.52", None)],
            gateway: Some("192.0.2.1".parse().unwrap()),
            clients: Clients::All,
            ..Default::default()
        };
        let m = build(&input);
        let nas: Vec<&Node> = m
            .nodes
            .iter()
            .filter(|n| n.mac == mac::parse_mac("02:00:00:00:00:51"))
            .collect();
        assert_eq!(nas.len(), 1, "one card");
        let d = &nas[0].details;
        let count = |line: &str| d.iter().filter(|l| l.as_str() == line).count();
        assert_eq!(count("DNS name NAS"), 1, "{d:?}");
        assert_eq!(count("Answered ping in 0.36 ms"), 1, "{d:?}");
        assert_eq!(
            count("Also at 192.0.2.52: Answered ARP, not ping (firewalled or ICMP off)"),
            1,
            "{d:?}"
        );
        assert!(
            !d.iter().any(|l| l.contains("Also at 192.0.2.52: DNS")),
            "{d:?}"
        );
        // The card answered ping at its own address.
        assert!(!nas[0].chips.iter().any(|c| c == "ARP only"));
    }

    #[test]
    fn sweep_only_merges_gateway_and_infers_roles() {
        let s = |ip: &str, mac: Option<&str>, open: &[u16]| Seen {
            ip: ip.parse().unwrap(),
            mac: mac.and_then(mac::parse_mac),
            name: None,
            rtt: Some(1.0),
            open: open.to_vec(),
            checked: true,
        };
        let input = Input {
            sweep: vec![
                s("192.168.1.1", Some("02:00:00:00:00:01"), &[80, 443]),
                s("192.168.1.20", None, &[9100]),
                s("192.168.1.30", None, &[23, 80]),
                s("192.168.1.40", None, &[]),
                s("192.168.1.50", None, &[]),
            ],
            range: Some("192.168.1.0/24".into()),
            ports_checked: true,
            gateway: Some("192.168.1.1".parse().unwrap()),
            this: Some("192.168.1.50".parse().unwrap()),
            ..Default::default()
        };
        let m = build(&input);
        let gw = by_label(&m, "Gateway");
        assert_eq!(gw.seen_by, ["sweep"]);
        // Clients off: the printer and plain host fold into a count; this
        // machine and the telnet box stay.
        assert!(m.nodes.iter().all(|n| !n.client));
        assert!(by_label(&m, "This machine").up.is_some());
        assert!(gw.chips.iter().any(|c| c == "2 clients"), "{:?}", gw.chips);
        assert_eq!(m.hidden, 2);
        let gear = m
            .nodes
            .iter()
            .find(|n| n.open.contains(&23))
            .expect("telnet host");
        assert_eq!((gear.zone, gear.client), (Zone::Gear, false));
        assert_eq!(m.findings[0].sev, Sev::High);
        assert!(m.findings[0].text.contains("Telnet"));

        let m = build(&Input {
            clients: Clients::Some,
            ..input
        });
        assert!(m.nodes.iter().any(|n| n.kind == "PRINTER"));
        assert_eq!(m.hidden, 0);
    }

    /// internet → gw (HQ) → core (HQ) → ap (Branch, hung off HQ's core)
    /// → client (no site); internet → edge (no site) → br-sw (Branch).
    fn two_sites() -> Map {
        let n = |label: &str, zone: Zone, up: Option<usize>, site: Option<&str>| Node {
            id: label.into(),
            label: label.into(),
            zone,
            up: up.map(|p| (p, Link::Wired, None)),
            site: site.map(str::to_string),
            ..Default::default()
        };
        Map {
            nodes: vec![
                n("internet", Zone::Internet, None, None),
                n("gw", Zone::Gateway, Some(0), Some("HQ")),
                n("core", Zone::Switch, Some(1), Some("HQ")),
                n("ap", Zone::Wireless, Some(2), Some("Branch")),
                n("client", Zone::Client, Some(3), None),
                n("edge", Zone::Unknown, Some(0), None),
                n("br-sw", Zone::Switch, Some(5), Some("Branch")),
            ],
            findings: vec![Finding {
                sev: Sev::Med,
                text: "x".into(),
                nodes: vec![2, 3, 4],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn every_card_but_the_internet_gets_a_site() {
        let mut m = two_sites();
        m.fill_sites("Elsewhere");
        let site = |l: &str| by_label(&m, l).site.clone();
        assert_eq!(site("internet"), None);
        assert_eq!(site("client").as_deref(), Some("Branch"), "its parent's");
        assert_eq!(
            site("edge").as_deref(),
            Some("Branch"),
            "the first below it"
        );
        let mut lone = Map {
            nodes: vec![m.nodes[0].clone(), m.nodes[5].clone()],
            ..Default::default()
        };
        lone.nodes[1].site = None;
        lone.fill_sites("Elsewhere");
        assert_eq!(lone.nodes[1].site.as_deref(), Some("Elsewhere"));

        // A sweep alone is one site: this network.
        let m = build(&Input {
            sweep: vec![Seen {
                ip: "10.0.0.9".parse().unwrap(),
                mac: None,
                name: None,
                rtt: Some(1.0),
                open: vec![23],
                checked: true,
            }],
            gateway: Some("10.0.0.1".parse().unwrap()),
            ..Default::default()
        });
        assert_eq!(m.sites(), ["This network"]);
    }

    #[test]
    fn a_closed_site_is_one_card_where_the_site_hangs() {
        let mut m = two_sites();
        m.fill_sites("Elsewhere");
        let sites = m.sites();
        m.close_sites(&HashSet::from(["Branch".to_string()]));
        // Where the site's first card was: the site keeps its place.
        assert_eq!(m.sites(), sites);
        let card = by_label(&m, "Branch");
        assert_eq!(
            (card.id.as_str(), card.kind.as_str()),
            ("site:Branch", "SITE")
        );
        // Its top card is edge, which took Branch from the switch below it.
        assert_eq!(parent(&m, "Branch"), "internet");
        assert_eq!(card.zone, Zone::Unknown, "the most significant role inside");
        assert_eq!(m.nodes.len(), 4);
        assert!(m.tucked("ap") && m.tucked("br-sw") && !m.tucked("core"));
        assert_eq!(m.closed["Branch"].len(), 4);
        // The finding now names core and the site's card, once.
        let site = m.nodes.iter().position(|n| n.id == "site:Branch").unwrap();
        let core = m.nodes.iter().position(|n| n.id == "core").unwrap();
        assert_eq!(m.findings[0].nodes, [core, site]);
        for i in 0..m.nodes.len() {
            assert_eq!(m.path(i)[0], 0, "{}", m.nodes[i].label);
        }

        // Closing HQ instead: Branch's AP hangs off HQ's card.
        let mut m = two_sites();
        m.fill_sites("Elsewhere");
        m.close_sites(&HashSet::from(["HQ".to_string()]));
        assert_eq!(parent(&m, "HQ"), "internet");
        assert_eq!(parent(&m, "ap"), "HQ");
        assert_eq!(by_label(&m, "HQ").zone, Zone::Gateway);
    }

    #[test]
    fn unifi_devices_go_to_their_controller_site() {
        let j = Value::Array(vec![
            json!({"__cmd": "select", "collection": "site"}),
            json!({"_id": "a", "desc": "Head Office"}),
            json!({"_id": "b", "desc": "Warehouse"}),
            json!({"__cmd": "select", "collection": "device"}),
            json!({"name": "HQ-SW", "type": "usw", "model": "M", "mac": "00:00:00:00:00:01",
                "site_id": "a"}),
            json!({"name": "WH-SW", "type": "usw", "model": "M", "mac": "00:00:00:00:00:02",
                "site_id": "b"}),
        ])
        .to_string();
        let m = build(&Input {
            unifi: one(&j),
            ..Default::default()
        });
        assert_eq!(by_label(&m, "HQ-SW").site.as_deref(), Some("Head Office"));
        assert_eq!(by_label(&m, "WH-SW").site.as_deref(), Some("Warehouse"));
        assert!(
            by_label(&m, "WH-SW")
                .chips
                .contains(&"Warehouse".to_string())
        );
        assert_eq!(m.sites().len(), 2);
    }

    #[test]
    fn clients_cap_and_budget() {
        let mut users = Vec::new();
        for i in 0..300 {
            users.push(
                json!({"mac": format!("00:11:22:33:{:02x}:{:02x}", i / 256, i % 256),
                "hostname": format!("pc{i}"), "last_uplink_mac": "00:00:00:00:00:01",
                "is_wired": true, "last_seen": i}),
            );
        }
        let j = unifi(
            &[dev("Core", "usw", "00:00:00:00:00:01", "10.0.0.2", None)],
            &users,
        );
        let m = build(&Input {
            unifi: one(&j),
            clients: Clients::Some,
            ..Default::default()
        });
        let kids = m
            .nodes
            .iter()
            .filter(|n| n.client && n.kind != "MORE CLIENTS")
            .count();
        assert_eq!(kids, PER_UPLINK);
        let more = m.nodes.iter().find(|n| n.kind == "MORE CLIENTS").unwrap();
        assert_eq!(more.label, "+290 more");
        assert_eq!(by_label(&m, "pc299").kind, "CLIENT", "most recent first");
        assert!(m.nodes.len() <= BUDGET);

        // All: a card for every one of the 300, past the budget.
        let count = |m: &Map| {
            m.nodes
                .iter()
                .filter(|n| n.client && n.kind != "MORE CLIENTS")
                .count()
        };
        let all = build(&Input {
            unifi: one(&j),
            clients: Clients::All,
            ..Default::default()
        });
        assert_eq!(count(&all), 300);
        assert!(all.nodes.iter().all(|n| n.kind != "MORE CLIENTS"));
        assert!(all.folded.is_empty());
        // Show all on the +N more card opens just that uplink.
        let core = m.nodes[more.parent().unwrap()].id.clone();
        let open: HashMap<String, usize> = [(core, usize::MAX)].into();
        let opened = build(&Input {
            unifi: one(&j),
            clients: Clients::Some,
            reveal: Some(&open),
            ..Default::default()
        });
        assert_eq!(count(&opened), 300);
        assert!(m.folded.contains(&by_label(&all, "pc0").id));

        let off = build(&Input {
            unifi: one(&j),
            ..Default::default()
        });
        assert!(
            by_label(&off, "Core")
                .chips
                .contains(&"300 clients".to_string())
        );
    }

    /// One switch with `users` as its UniFi clients.
    fn switch_with(users: &[Value]) -> String {
        unifi(
            &[dev("Core", "usw", "00:00:00:00:00:01", "10.0.0.2", None)],
            users,
        )
    }

    fn user(i: usize, name: Option<&str>, oui: Option<&str>, last_seen: i64) -> Value {
        let mut u = json!({"mac": format!("00:11:22:33:44:{i:02x}"),
            "last_uplink_mac": "00:00:00:00:00:01", "is_wired": true, "last_seen": last_seen});
        if let Some(n) = name {
            u["hostname"] = json!(n);
        }
        if let Some(o) = oui {
            u["oui"] = json!(o);
        }
        u
    }

    #[test]
    fn clients_reveal_best_first() {
        let j = switch_with(&[
            user(1, None, None, 900),
            user(2, Some("client-1a2b3c4d"), None, 800),
            user(3, None, Some("Apple, Inc."), 700),
            user(4, Some("old-laptop"), None, 100),
            user(5, Some("new-laptop"), None, 600),
            user(6, None, Some("Brother Industries"), 50),
            json!({"mac": "00:11:22:33:44:07", "last_uplink_mac": "00:00:00:00:00:01",
                "is_wired": true, "blocked": true, "last_seen": 1}),
        ]);
        let mut b = B::default();
        let u = parse_unifi(&j);
        let gw = b.add(Node::default());
        b.add(Node {
            id: "core".into(),
            mac: Some(u.devices[0].mac),
            zone: Zone::Switch,
            up: Some((gw, Link::Wired, None)),
            ..Default::default()
        });
        b.unifi_clients(&u, gw, &[]);
        let mut kids: Vec<&Node> = b.nodes.iter().filter(|n| n.client).collect();
        kids.sort_by_key(|n| reveal_key(n));
        let order: Vec<&str> = kids.iter().map(|n| n.label.as_str()).collect();
        // Blocked (a warning), the printer (a role), the named ones newest
        // first, a vendor, then nothing known: an anonymized token is no name.
        assert_eq!(
            order,
            [
                "00:11:22:33:44:07",
                "00:11:22:33:44:06",
                "new-laptop",
                "old-laptop",
                "00:11:22:33:44:03",
                "00:11:22:33:44:01",
                "client-1a2b3c4d",
            ]
        );
        // The token is kept as the name: anonymized stays anonymized.
        assert!(by_label_in(&b.nodes, "client-1a2b3c4d").is_some());
    }

    fn by_label_in<'a>(nodes: &'a [Node], l: &str) -> Option<&'a Node> {
        nodes.iter().find(|n| n.label == l)
    }

    #[test]
    fn steps_show_the_next_few_then_all() {
        assert_eq!(more_of(0, 40), STEP);
        assert_eq!(more_of(8, 40), 16);
        // Only a few would be left: all of them.
        assert_eq!(more_of(32, 40), usize::MAX);
        assert_eq!(more_of(0, 5), usize::MAX);
        assert_eq!(more_of(3, 300), 11);
        // − on an uplink stepped past the level: back to the level.
        assert_eq!(fewer_of(Some(16), 0), None);
        assert_eq!(fewer_of(Some(usize::MAX), 10), None);
        // On one the level opened (or stepped below it): none.
        assert_eq!(fewer_of(None, 10), Some(0));
        assert_eq!(fewer_of(Some(4), 10), Some(0));
        assert!(is_count_chip(&count_chip(1)) && is_count_chip(&count_chip(40)));
        assert!(!is_count_chip("guest") && !is_count_chip("many clients"));
    }

    #[test]
    fn one_uplink_steps_open_while_clients_are_off() {
        let users: Vec<Value> = (0..40)
            .map(|i| user(i, Some(&format!("pc{i}")), None, i as i64))
            .collect();
        let j = switch_with(&users);
        let build_with = |clients, reveal: &HashMap<String, usize>| {
            build(&Input {
                unifi: one(&j),
                clients,
                reveal: Some(reveal),
                ..Default::default()
            })
        };
        let none = HashMap::new();
        let off = build_with(Clients::Off, &none);
        let core = by_label(&off, "Core").id.clone();
        assert_eq!(off.under.get(&core), Some(&(0, 40)));
        assert!(
            off.summary.ends_with("40 clients not drawn"),
            "{}",
            off.summary
        );
        // +N on the card: the eight newest, and a +32 more card.
        let step: HashMap<String, usize> = [(core.clone(), more_of(0, 40))].into();
        let m = build_with(Clients::Off, &step);
        assert_eq!(m.under.get(&core), Some(&(8, 40)));
        let shown: Vec<&str> = m
            .nodes
            .iter()
            .filter(|n| n.client && n.kind != "MORE CLIENTS")
            .map(|n| n.label.as_str())
            .collect();
        assert_eq!(shown.len(), 8);
        assert!(shown.contains(&"pc39") && !shown.contains(&"pc31"));
        assert_eq!(by_label(&m, "+32 more").kind, "MORE CLIENTS");
        // The summary counts them as the chip does: 32 not drawn.
        assert_eq!(m.hidden, 32);
        assert!(m.summary.ends_with("32 clients not drawn"), "{}", m.summary);
        // Stepped to none under Some: a count again, no cards.
        let fold: HashMap<String, usize> = [(core.clone(), 0)].into();
        let m = build_with(Clients::Some, &fold);
        assert!(m.nodes.iter().all(|n| !n.client));
        assert!(
            by_label(&m, "Core")
                .chips
                .contains(&"40 clients".to_string())
        );
        // More shows up to 30, All every one.
        let more = build_with(Clients::More, &none);
        assert_eq!(more.under.get(&core), Some(&(30, 40)));
        let all = build_with(Clients::All, &none);
        assert_eq!(all.under.get(&core), Some(&(40, 40)));
    }

    #[test]
    fn a_big_sweep_steps_open_within_the_budget() {
        let sweep: Vec<Seen> = (0..3000u32)
            .map(|i| Seen {
                ip: Ipv4Addr::from(0x0a00_0000 + 2 + i),
                mac: None,
                name: (i % 3 == 0).then(|| format!("host{i}")),
                rtt: Some(1.0),
                open: Vec::new(),
                checked: true,
            })
            .collect();
        let make = |clients, reveal| {
            build(&Input {
                sweep: sweep.clone(),
                range: Some("10.0.0.0/20".into()),
                gateway: Some("10.0.0.1".parse().unwrap()),
                clients,
                reveal,
                ..Default::default()
            })
        };
        let some = make(Clients::Some, None);
        assert!(some.nodes.len() <= BUDGET, "{}", some.nodes.len());
        let gw = some.nodes[1].id.clone();
        let (shown, total) = some.under[&gw];
        assert_eq!((shown, total), (PER_UPLINK, 3000));
        // Named hosts come first.
        assert!(
            some.nodes
                .iter()
                .filter(|n| n.client && n.kind != "MORE CLIENTS")
                .all(|n| n.label.starts_with("host"))
        );
        let step: HashMap<String, usize> = [(gw.clone(), more_of(shown, total))].into();
        let more = make(Clients::Some, Some(&step));
        assert_eq!(more.under[&gw], (18, 3000));
        let all = make(Clients::All, None);
        assert_eq!(all.under[&gw], (3000, 3000));
        let l = layout::layout_in(&all, &|_| true, layout::Mode::Cluster);
        assert!(l.pos.iter().all(Option::is_some));
    }

    #[test]
    fn what_the_firewall_hears_goes_on_the_map() {
        let sw = json!({
            "system": {"model": "TZ 370"},
            "interfaces": [{"name": "X0", "zone": "LAN", "ip": "10.0.0.1", "mac": "18:b1:69:00:00:01"}],
            "neighbors": [
                {"interface": "X0", "protocol": "LLDP", "mac": "f0:9f:c2:00:00:09", "ip": "10.0.0.9",
                 "name": "Closet Switch", "description": "US-24-250W, 7.1.26", "port": "Port 24",
                 "capabilities": "2-Port-MAC-Relay"},
                {"interface": "X0", "protocol": "LLDP", "mac": "00:00:00:00:00:01", "ip": "10.0.0.2",
                 "name": "Core", "port": "Port 1", "capabilities": "Bridge"},
                {"interface": "X0", "protocol": "LLTD", "mac": "00:11:22:00:00:05", "ip": "10.0.0.165"}
            ]
        })
        .to_string();
        // Core is in the UniFi backup with no uplink recorded.
        let j = unifi(
            &[dev("Core", "usw", "00:00:00:00:00:01", "10.0.0.2", None)],
            &[],
        );
        let m = build(&Input {
            unifi: one(&j),
            sonicwall: one(&sw),
            clients: Clients::Some,
            ..Default::default()
        });
        let gw = m.nodes[1].label.clone();
        let closet = by_label(&m, "Closet Switch");
        assert_eq!(closet.zone, Zone::Switch);
        assert_eq!(closet.kind, "SWITCH US-24-250W");
        assert_eq!(parent(&m, "Closet Switch"), gw);
        assert_eq!(
            closet.up.as_ref().map(|u| (u.1, u.2.clone())),
            Some((Link::Wired, Some("X0, its Port 24".into())))
        );
        // The backup's switch: its assumed uplink becomes the firewall port.
        let core = by_label(&m, "Core");
        assert_eq!(core.link(), Some(Link::Wired));
        assert!(core.seen_by.contains(&"firewall"));
        assert!(
            !m.findings
                .iter()
                .any(|f| f.text.starts_with("No uplink recorded"))
        );
        let host = by_label(&m, "10.0.0.165");
        assert!(host.client);
        assert_eq!(host.kind, "WINDOWS HOST");
        // LLTD crosses switches: where it plugs in isn't known.
        assert_eq!(host.link(), Some(Link::Assumed));
    }

    #[test]
    fn sonicwall_is_the_gateway_and_wan_management_is_flagged() {
        let sw = json!({
            "system": {"model": "TZ 470", "firmwareVersion": "7.0.1"},
            "interfaces": [
                {"name": "X0", "zone": "LAN", "ip": "10.0.0.1", "mask": "255.255.255.0",
                 "mac": "18:b1:69:00:00:01", "management": ["https", "ping"]},
                {"name": "X1", "zone": "WAN", "ip": "203.0.113.5", "mac": "18:b1:69:00:00:02",
                 "management": ["https", "ssh"]}
            ]
        })
        .to_string();
        let j = unifi(
            &[dev(
                "Core",
                "usw",
                "00:00:00:00:00:01",
                "10.0.0.2",
                Some(("18:b1:69:00:00:01", 3)),
            )],
            &[],
        );
        let m = build(&Input {
            unifi: one(&j),
            sonicwall: one(&sw),
            gateway: Some("10.0.0.1".parse().unwrap()),
            ..Default::default()
        });
        assert!(
            m.summary.contains("sweep here to add"),
            "no sweep: nothing local merged, and the subtitle says how to add it"
        );
        let gw = &m.nodes[1];
        assert_eq!(gw.kind, "SONICWALL TZ 470");
        assert_eq!(gw.sub, "10.0.0.1");
        assert_eq!(parent(&m, "Core"), gw.label);
        assert_eq!(by_label(&m, "Core").link().unwrap(), Link::Wired);
        assert_eq!(m.findings[0].sev, Sev::High);
        assert!(m.findings[0].text.contains("X1 (WAN) allows HTTPS/SSH"));
    }

    #[test]
    fn local_scan_merges_only_on_a_shared_mac() {
        let j = unifi(
            &[dev("Core", "usw", "00:00:00:00:00:01", "10.0.0.2", None)],
            &[],
        );
        let host = |mac: &str| Seen {
            ip: "192.168.1.7".parse().unwrap(),
            mac: mac::parse_mac(mac),
            name: None,
            rtt: Some(1.0),
            open: vec![],
            checked: true,
        };
        let at_office = build(&Input {
            unifi: one(&j),
            sweep: vec![host("aa:00:00:00:00:07")],
            gateway: Some("192.168.1.1".parse().unwrap()),
            range: Some("192.168.1.0/24".into()),
            ..Default::default()
        });
        assert!(at_office.summary.contains("sweep not merged") && at_office.sweep_apart);
        let gw = |m: &Map| m.nodes.iter().find(|n| n.id == "gateway").unwrap().clone();
        assert_eq!(gw(&at_office).label, "Gateway (not in the data)");
        assert!(
            at_office
                .nodes
                .iter()
                .all(|n| n.ip != Some("192.168.1.7".parse().unwrap()))
        );
        let on_site = build(&Input {
            unifi: one(&j),
            sweep: vec![host("00:00:00:00:00:01")],
            gateway: Some("192.168.1.1".parse().unwrap()),
            ..Default::default()
        });
        assert_eq!(gw(&on_site).sub, "192.168.1.1");
        assert!(by_label(&on_site, "Core").seen_by.contains(&"sweep"));
        assert!(!on_site.sweep_apart);
    }

    /// A backup of one controller: its sites (`_id`, name), then devices
    /// (each with its site's `_id`) and clients.
    fn controller(sites: &[(&str, &str)], devices: &[Value], users: &[Value]) -> String {
        let mut v = vec![json!({"__cmd": "select", "collection": "site"})];
        v.extend(
            sites
                .iter()
                .map(|(id, name)| json!({"_id": id, "desc": name})),
        );
        v.push(json!({"__cmd": "select", "collection": "device"}));
        v.extend(devices.iter().cloned());
        v.push(json!({"__cmd": "select", "collection": "user"}));
        v.extend(users.iter().cloned());
        Value::Array(v).to_string()
    }

    fn at(mut d: Value, site: &str) -> Value {
        d["site_id"] = json!(site);
        d
    }

    fn file<'a>(name: &'a str, json: &'a str) -> Loaded<'a> {
        Loaded { name, json }
    }

    #[test]
    fn two_unifi_backups_are_two_controllers() {
        let a = controller(
            &[("5f0000000000000000000a01", "Default")],
            &[
                at(
                    dev("North Core", "usw", "00:00:00:00:0a:01", "192.0.2.2", None),
                    "5f0000000000000000000a01",
                ),
                at(
                    dev(
                        "North AP",
                        "uap",
                        "00:00:00:00:0a:02",
                        "192.0.2.3",
                        Some(("00:00:00:00:0a:01", 4)),
                    ),
                    "5f0000000000000000000a01",
                ),
                // Moved to the other controller since this backup.
                at(
                    dev("Loaner (old name)", "usw", "00:00:00:00:0c:01", "", None),
                    "5f0000000000000000000a01",
                ),
            ],
            &[json!({"mac": "00:11:22:33:44:01", "hostname": "laptop-old",
                "last_uplink_mac": "00:00:00:00:0a:01", "is_wired": true})],
        );
        let b = controller(
            &[("5f0000000000000000000b01", "Default")],
            &[
                at(
                    dev(
                        "South Core",
                        "usw",
                        "00:00:00:00:0b:01",
                        "198.51.100.2",
                        None,
                    ),
                    "5f0000000000000000000b01",
                ),
                at(
                    dev(
                        "Loaner",
                        "usw",
                        "00:00:00:00:0c:01",
                        "198.51.100.9",
                        Some(("00:00:00:00:0b:01", 9)),
                    ),
                    "5f0000000000000000000b01",
                ),
            ],
            &[json!({"mac": "00:11:22:33:44:01", "hostname": "laptop-new",
                "last_uplink_mac": "00:00:00:00:0b:01", "is_wired": true})],
        );
        let m = build(&Input {
            unifi: vec![file("north.unf", &a), file("south.unf", &b)],
            clients: Clients::All,
            ..Default::default()
        });
        // Both controllers' devices, each at its own site: two sites both
        // called Default are told apart by their backup.
        let site = |l: &str| by_label(&m, l).site.clone().unwrap_or_default();
        assert_eq!(site("North Core"), "Default (north)");
        assert_eq!(site("North AP"), "Default (north)");
        assert_eq!(site("South Core"), "Default (south)");
        assert_eq!(m.sites().len(), 2, "{:?}", m.sites());
        assert_eq!(parent(&m, "North AP"), "North Core");
        // In both: one card, as the backup opened later has it.
        let loaner: Vec<&Node> = m
            .nodes
            .iter()
            .filter(|n| n.id == "mac:00:00:00:00:0c:01")
            .collect();
        assert_eq!(loaner.len(), 1);
        assert_eq!(loaner[0].label, "Loaner");
        assert_eq!(parent(&m, "Loaner"), "South Core");
        assert_eq!(site("Loaner"), "Default (south)");
        assert!(m.nodes.iter().all(|n| n.label != "laptop-old"));
        assert_eq!(parent(&m, "laptop-new"), "South Core");
        assert!(
            m.summary
                .starts_with("UniFi Default (north), Default (south) (4 devices from 2 backups)"),
            "{}",
            m.summary
        );

        // One controller's backups from two days: one site, the newer
        // backup's names.
        let monday = controller(
            &[("5f0000000000000000000a01", "HQ")],
            &[at(
                dev("Core (old)", "usw", "00:00:00:00:0a:01", "192.0.2.2", None),
                "5f0000000000000000000a01",
            )],
            &[],
        );
        let tuesday = controller(
            &[("5f0000000000000000000a01", "HQ")],
            &[
                at(
                    dev("Core", "usw", "00:00:00:00:0a:01", "192.0.2.2", None),
                    "5f0000000000000000000a01",
                ),
                at(
                    dev("New AP", "uap", "00:00:00:00:0a:05", "192.0.2.5", None),
                    "5f0000000000000000000a01",
                ),
            ],
            &[],
        );
        let m = build(&Input {
            unifi: vec![file("hq-mon.unf", &monday), file("hq-tue.unf", &tuesday)],
            ..Default::default()
        });
        assert_eq!(m.sites(), ["HQ"]);
        assert!(m.nodes.iter().all(|n| n.label != "Core (old)"));
        assert_eq!(by_label(&m, "Core").site.as_deref(), Some("HQ"));
        assert!(by_label(&m, "New AP").chips.iter().all(|c| c != "HQ"));
    }

    /// A firewall's export: its name, LAN and its MAC there, and what it
    /// heard.
    fn firewall(name: &str, lan: &str, mac: &str, heard: &[Value]) -> String {
        json!({
            "system": {"model": "TZ 270", "firewallName": name},
            "interfaces": [
                {"name": "X0", "zone": "LAN", "ip": lan, "mask": "255.255.255.0", "mac": mac},
                {"name": "X1", "zone": "WAN", "ip": "203.0.113.10", "mask": "255.255.255.248"}
            ],
            "neighbors": heard,
        })
        .to_string()
    }

    #[test]
    fn two_sonicwalls_are_two_gateways() {
        let fw_a = firewall("fw-north", "192.0.2.1", "18:b1:69:00:0a:01", &[]);
        let fw_b = firewall(
            "fw-south",
            "198.51.100.1",
            "18:b1:69:00:0b:01",
            &[
                json!({"interface": "X0", "protocol": "LLDP", "mac": "f0:9f:c2:00:0b:09",
                "ip": "198.51.100.9", "name": "South Closet", "port": "Port 1",
                "capabilities": "Bridge"}),
            ],
        );
        let j = unifi(
            &[
                dev(
                    "North Core",
                    "usw",
                    "00:00:00:00:0a:01",
                    "192.0.2.2",
                    Some(("18:b1:69:00:0a:01", 1)),
                ),
                dev(
                    "South Core",
                    "usw",
                    "00:00:00:00:0b:01",
                    "198.51.100.2",
                    Some(("18:b1:69:00:0b:01", 2)),
                ),
                // No uplink: placed by the LAN its address is on.
                dev(
                    "South Spare",
                    "usw",
                    "00:00:00:00:0b:02",
                    "198.51.100.3",
                    None,
                ),
                // Nothing to go by: under the firewall opened last.
                dev("Stray", "usw", "00:00:00:00:0c:01", "", None),
            ],
            &[],
        );
        let input = |fws: Vec<Loaded>| {
            build(&Input {
                unifi: vec![file("hq.unf", &j)],
                sonicwall: fws,
                ..Default::default()
            })
        };
        let (a, b) = (fw_a.as_str(), fw_b.as_str());
        let m = input(vec![file("fw-north.wri", a), file("fw-south.wri", b)]);
        let gws: Vec<&Node> = m.nodes.iter().filter(|n| n.zone == Zone::Gateway).collect();
        assert_eq!(gws.len(), 2);
        for g in &gws {
            assert_eq!(g.parent(), Some(0), "{} hangs off the internet", g.label);
            assert_eq!(g.link(), Some(Link::Backbone));
        }
        assert_eq!(by_label(&m, "fw-north").id, "gateway", "the first opened");
        assert_eq!(by_label(&m, "fw-south").id, "gateway:18:b1:69:00:0b:01");
        assert_eq!(parent(&m, "North Core"), "fw-north");
        assert_eq!(
            parent(&m, "South Core"),
            "fw-south",
            "its uplink is B's MAC"
        );
        assert_eq!(parent(&m, "South Spare"), "fw-south", "on B's LAN");
        assert_eq!(parent(&m, "Stray"), "fw-south", "the one opened last");
        assert_eq!(parent(&m, "South Closet"), "fw-south", "what B heard");
        assert!(
            m.summary.contains("2 SonicWalls (fw-north, fw-south)"),
            "{}",
            m.summary
        );
        // Opened the other way round, the stray goes under the other one.
        let m = input(vec![file("fw-south.wri", b), file("fw-north.wri", a)]);
        assert_eq!(parent(&m, "Stray"), "fw-north");
        assert_eq!(parent(&m, "South Core"), "fw-south");
        assert_eq!(by_label(&m, "fw-south").id, "gateway");

        // A settings export and a report of one firewall: one card, with
        // what the report heard.
        let exp = firewall("fw-south", "198.51.100.1", "18:b1:69:00:0b:01", &[]);
        let m = input(vec![file("fw-south.wri", b), file("fw-south.exp", &exp)]);
        assert_eq!(
            m.nodes.iter().filter(|n| n.zone == Zone::Gateway).count(),
            1
        );
        assert_eq!(parent(&m, "South Closet"), "fw-south");
        assert!(m.summary.contains("SonicWall TZ 270"), "{}", m.summary);
    }

    /// Two sites of one client, each with its UniFi backup and its firewall's
    /// report, both on 192.0.2.0/24: each site's devices under its own
    /// firewall, in its own box.
    #[test]
    fn two_sites_each_with_a_backup_and_a_firewall() {
        let north = controller(
            &[("5f0000000000000000000a01", "Default")],
            &[
                at(
                    dev(
                        "North Core",
                        "usw",
                        "00:00:00:00:0a:01",
                        "192.0.2.2",
                        Some(("18:b1:69:00:0a:01", 1)),
                    ),
                    "5f0000000000000000000a01",
                ),
                at(
                    dev("North Spare", "usw", "00:00:00:00:0a:02", "192.0.2.3", None),
                    "5f0000000000000000000a01",
                ),
            ],
            &[],
        );
        let south = controller(
            &[("5f0000000000000000000b01", "Default")],
            &[
                at(
                    dev(
                        "South Core",
                        "usw",
                        "00:00:00:00:0b:01",
                        "192.0.2.2",
                        Some(("18:b1:69:00:0b:01", 1)),
                    ),
                    "5f0000000000000000000b01",
                ),
                at(
                    dev("South Spare", "usw", "00:00:00:00:0b:02", "192.0.2.3", None),
                    "5f0000000000000000000b01",
                ),
            ],
            &[],
        );
        let fw_n = firewall("fw-north", "192.0.2.1", "18:b1:69:00:0a:01", &[]);
        let fw_s = firewall("fw-south", "192.0.2.1", "18:b1:69:00:0b:01", &[]);
        let m = build(&Input {
            unifi: vec![file("north.unf", &north), file("south.unf", &south)],
            sonicwall: vec![file("fw-north.wri", &fw_n), file("fw-south.wri", &fw_s)],
            ..Default::default()
        });
        assert_eq!(parent(&m, "North Core"), "fw-north");
        assert_eq!(parent(&m, "South Core"), "fw-south");
        // The subnet can't say (both sites use it): their site's firewall.
        assert_eq!(parent(&m, "North Spare"), "fw-north");
        assert_eq!(parent(&m, "South Spare"), "fw-south");
        let site = |l: &str| by_label(&m, l).site.clone().unwrap_or_default();
        assert_eq!(site("fw-north"), "Default (north)");
        assert_eq!(site("fw-south"), "Default (south)");
        assert_eq!(m.sites().len(), 2);
        for i in 0..m.nodes.len() {
            assert_eq!(m.path(i)[0], 0, "{}", m.nodes[i].label);
        }

        // Two firewalls and nothing else: a site each, by name.
        let m = build(&Input {
            sonicwall: vec![file("fw-north.wri", &fw_n), file("fw-south.wri", &fw_s)],
            ..Default::default()
        });
        assert_eq!(m.sites(), ["fw-north", "fw-south"]);
    }

    #[test]
    fn two_sites_of_one_name_are_told_apart_by_their_files() {
        let names = |v: &[(&str, &str)]| {
            tell_apart(
                &v.iter()
                    .map(|(n, t)| (n.to_string(), t.to_string()))
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            names(&[
                ("Default", "northwind-north_backup_2026-11-10_v9.0.114"),
                ("Default", "northwind-south_backup_2026-11-10_v9.0.114"),
                ("Warehouse", "northwind-south_backup_2026-11-10_v9.0.114"),
            ]),
            ["Default (north)", "Default (south)", "Warehouse"]
        );
        // A part with no letters keeps the word before it.
        assert_eq!(
            names(&[
                ("Default", "network_backup_10.11.2026_14-05_v9.0.114"),
                ("Default", "network_backup_10.11.2026_14-20_v9.0.114"),
            ]),
            ["Default (14-05)", "Default (14-20)"]
        );
        // Files of one name: a number.
        assert_eq!(
            names(&[("HQ", "backup"), ("HQ", "backup")]),
            ["HQ (backup)", "HQ (backup) 2"]
        );
        assert_eq!(file_tag("/x/site.unf", 1), "site");
        assert_eq!(file_tag("", 2), "2");
    }

    #[test]
    fn kind_is_short_caps() {
        assert_eq!(caps("switch usl48pb with a long tail"), "SWITCH USL48PB");
        assert_eq!(caps("gateway "), "GATEWAY");
        assert_eq!(short_vendor("Ubiquiti Inc"), "UBIQUITI");
        assert_eq!(classify(Some("Ubiquiti Inc"), false, &[]).0, Zone::Gear);
        assert_eq!(classify(None, false, &[9100]).1, "PRINTER");
        assert_eq!(classify(None, true, &[]).1, "PRIVATE MAC");
    }
}
