//! Saved maps: a map with the user's changes to it (cards moved, pins,
//! sites collapsed, the layout), kept as a file of its own so it opens
//! again without the sweep or the files it was drawn from. One JSON file
//! per map in the settings folder's `maps`, and the same file can be
//! shared: Open… on the bar, Config Export, or dropped on the window.
//!
//! Read back defensively: a size cap, limits on how many cards, findings
//! and strings, every link checked, and a file from a newer is-alive (a
//! higher format version) refused with a message rather than misread.

use super::history::Edits;
use super::*;
use crate::input;
use crate::map::Finding;
use serde_json::{Map as Obj, Value, json};
use std::path::{Path, PathBuf};

/// What the file says it is.
pub const FORMAT: &str = "is-alive-map";
/// The format version written; a file with a higher one is refused.
pub const VERSION: u64 = 1;

/// Largest file read: a big map with every client is a few MB.
const FILE_MAX: u64 = 32 << 20;
/// Most cards and findings on a map, as /mapnetwork maps are limited.
const NODES_MAX: usize = 10_000;
const FINDINGS_MAX: usize = 10_000;
/// Most items in one card's list (chips, details, flags, ports).
const LIST_MAX: usize = 1_000;
/// Most cards left out of the drawing that a map remembers.
const FOLDED_MAX: usize = 100_000;
/// Longest string kept, and longest card id.
const TEXT_MAX: usize = 4_000;
const ID_MAX: usize = 512;
/// Longest map name.
pub const NAME_MAX: usize = 80;
/// Most saved maps listed.
pub const FILES_MAX: usize = 200;
/// How far from the middle a dragged card may sit.
const FAR: f32 = 1.0e6;

/// One saved map: what it shows and how the user left it.
pub struct Snapshot {
    pub name: String,
    /// When it was first saved (RFC 3339, UTC): the bar's order.
    pub created: String,
    /// The map with every site open; collapsed ones are in `edits`.
    pub map: Map,
    pub edits: Edits,
    /// Pan and zoom, or None to fit the whole map.
    pub camera: Option<(Vec2, f32)>,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Snapshot({:?}, {} cards)",
            self.name,
            self.map.nodes.len()
        )
    }
}

/// Now, as a saved map's date.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A copy of a map (it isn't Clone: the view keeps one).
pub fn copy_map(m: &Map) -> Map {
    Map {
        nodes: m.nodes.clone(),
        findings: m.findings.clone(),
        summary: m.summary.clone(),
        hidden: m.hidden,
        folded: m.folded.clone(),
        closed: m.closed.clone(),
    }
}

/// True when `data` looks like a saved map (not some other JSON).
pub fn sniff(data: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&data[..data.len().min(4096)]);
    head.trim_start().starts_with('{') && head.contains(&format!("\"{FORMAT}\""))
}

fn zone_key(z: Zone) -> &'static str {
    match z {
        Zone::Internet => "internet",
        Zone::Gateway => "gateway",
        Zone::Switch => "switch",
        Zone::Wireless => "wireless",
        Zone::Gear => "gear",
        Zone::Server => "server",
        Zone::Device => "device",
        Zone::Client => "client",
        Zone::Unknown => "unknown",
    }
}

fn zone_of(k: &str) -> Zone {
    Zone::ALL
        .into_iter()
        .find(|&z| zone_key(z) == k)
        .unwrap_or(Zone::Unknown)
}

fn link_key(l: Link) -> &'static str {
    match l {
        Link::Backbone => "backbone",
        Link::Wired => "wired",
        Link::Wireless => "wireless",
        Link::Assumed => "assumed",
    }
}

fn link_of(k: &str) -> Link {
    match k {
        "backbone" => Link::Backbone,
        "wired" => Link::Wired,
        "wireless" => Link::Wireless,
        _ => Link::Assumed,
    }
}

fn sev_of(k: &str) -> Sev {
    match k {
        "high" => Sev::High,
        "med" => Sev::Med,
        _ => Sev::Low,
    }
}

/// Where a card's facts came from: the names the map uses (a card's
/// "Seen by" line). Anything else in a file is dropped.
const SEEN_BY: &[&str] = &[
    "sweep",
    "UniFi",
    "SonicWall",
    "firewall",
    "mapnetwork",
    "Meraki",
];

fn clients_key(c: Clients) -> &'static str {
    match c {
        Clients::Off => "off",
        Clients::Some => "some",
        Clients::All => "all",
    }
}

fn sorted_strs<'a>(it: impl Iterator<Item = &'a String>) -> Vec<&'a String> {
    let mut v: Vec<&String> = it.collect();
    v.sort();
    v
}

/// The file's text.
pub fn to_json(s: &Snapshot) -> String {
    let nodes: Vec<Value> = s.map.nodes.iter().map(node_json).collect();
    let findings: Vec<Value> = s
        .map
        .findings
        .iter()
        .map(|f| json!({"sev": f.sev.label(), "text": f.text, "nodes": f.nodes}))
        .collect();
    let mut placed: Vec<(&String, &Pos2)> = s.edits.placed.iter().collect();
    placed.sort_by(|a, b| a.0.cmp(b.0));
    let placed: Obj<String, Value> = placed
        .into_iter()
        .map(|(k, p)| (k.clone(), json!([p.x, p.y])))
        .collect();
    let mut faded: Vec<&str> = s.edits.hidden.iter().map(|&z| zone_key(z)).collect();
    faded.sort();
    let camera = s
        .camera
        .map(|(pan, zoom)| json!({"pan": [pan.x, pan.y], "zoom": zoom}));
    let doc = json!({
        "format": FORMAT,
        "version": VERSION,
        "name": s.name,
        "created": s.created,
        "app": env!("CARGO_PKG_VERSION"),
        "map": {
            "summary": s.map.summary,
            "hidden": s.map.hidden,
            "folded": sorted_strs(s.map.folded.iter()),
            "nodes": nodes,
            "findings": findings,
        },
        "view": {
            "layout": s.edits.mode.map(Mode::key),
            "clients": clients_key(s.edits.clients),
            "placed": placed,
            "pinned": s.edits.kept,
            "collapsed": sorted_strs(s.edits.closed.iter()),
            "faded": faded,
            "open": sorted_strs(s.edits.open.iter()),
            "camera": camera,
        },
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

fn node_json(n: &Node) -> Value {
    json!({
        "id": n.id,
        "zone": zone_key(n.zone),
        "kind": n.kind,
        "label": n.label,
        "sub": n.sub,
        "chips": n.chips,
        "details": n.details,
        "flags": n.flags,
        "ip": n.ip.map(|ip| ip.to_string()),
        "mac": n.mac.as_ref().map(crate::net::mac::fmt),
        "client": n.client,
        "seen_by": n.seen_by,
        "open": n.open,
        "up": n.up.as_ref().map(|(p, l, port)| json!({
            "parent": p,
            "link": link_key(*l),
            "port": port,
        })),
        "site": n.site,
    })
}

/// A string field, cleaned of hidden characters and cut to `max`.
fn text(v: &Value, k: &str, max: usize) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .map(|s| input::line(s, max))
        .unwrap_or_default()
}

/// A list of strings, each cleaned and cut, at most `most` of them.
fn texts(v: &Value, k: &str, most: usize, max: usize) -> Result<Vec<String>, String> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) if a.len() <= most => Ok(a
            .iter()
            .filter_map(Value::as_str)
            .map(|s| input::line(s, max))
            .collect()),
        Some(Value::Array(a)) => Err(format!(
            "{} entries in a \"{k}\" list (at most {most})",
            a.len()
        )),
        Some(_) => Err(format!("\"{k}\" isn't a list")),
    }
}

fn point(v: &Value) -> Option<Pos2> {
    let a = v.as_array()?;
    let (x, y) = (a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32);
    (x.is_finite() && y.is_finite() && x.abs() < FAR && y.abs() < FAR).then(|| pos2(x, y))
}

/// Read a saved map's text. Anything that isn't one, is from a newer
/// is-alive, or is past the limits is refused with a plain reason.
pub fn from_json(text_in: &str) -> Result<Snapshot, String> {
    let doc: Value =
        serde_json::from_str(text_in).map_err(|_| "not an is-alive map file".to_string())?;
    if doc.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err("not an is-alive map file".into());
    }
    let version = doc.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version == 0 {
        return Err("an is-alive map file with no format version".into());
    }
    if version > VERSION {
        return Err(format!(
            "saved by a newer is-alive (map format {version}; this one reads {VERSION}): \
             update is-alive to open it"
        ));
    }
    let m = doc.get("map").ok_or("no map in the file")?;
    let nodes = m
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or("no cards in the map")?;
    if nodes.is_empty() {
        return Err("no cards in the map".into());
    }
    if nodes.len() > NODES_MAX {
        return Err(format!(
            "{} cards: a saved map opens with up to {NODES_MAX}",
            nodes.len()
        ));
    }
    let nodes: Vec<Node> = nodes.iter().map(node_of).collect::<Result<_, String>>()?;
    check_tree(&nodes)?;
    let findings = match m.get("findings") {
        Some(Value::Array(a)) if a.len() > FINDINGS_MAX => {
            return Err(format!("{} findings (at most {FINDINGS_MAX})", a.len()));
        }
        Some(Value::Array(a)) => a
            .iter()
            .map(|f| Finding {
                sev: sev_of(&text(f, "sev", 8)),
                text: text(f, "text", TEXT_MAX),
                nodes: f
                    .get("nodes")
                    .and_then(Value::as_array)
                    .map(|v| {
                        v.iter()
                            .filter_map(Value::as_u64)
                            .map(|k| k as usize)
                            .filter(|&k| k < nodes.len())
                            .take(LIST_MAX)
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect(),
        _ => Vec::new(),
    };
    let map = Map {
        summary: text(m, "summary", TEXT_MAX),
        hidden: m.get("hidden").and_then(Value::as_u64).unwrap_or(0) as usize,
        folded: texts(m, "folded", FOLDED_MAX, ID_MAX)?
            .into_iter()
            .collect(),
        nodes,
        findings,
        closed: Default::default(),
    };
    let view = doc.get("view").cloned().unwrap_or(json!({}));
    let ids: HashSet<&str> = map.nodes.iter().map(|n| n.id.as_str()).collect();
    let placed = match view.get("placed") {
        Some(Value::Object(o)) => o
            .iter()
            .filter(|(k, _)| ids.contains(k.as_str()))
            .filter_map(|(k, p)| Some((k.clone(), point(p)?)))
            .collect(),
        _ => HashMap::new(),
    };
    let edits = Edits {
        placed,
        mode: view
            .get("layout")
            .and_then(Value::as_str)
            .and_then(Mode::from_key),
        clients: match view.get("clients").and_then(Value::as_str) {
            Some("some") => Clients::Some,
            Some("all") => Clients::All,
            _ => Clients::Off,
        },
        open: texts(&view, "open", NODES_MAX, ID_MAX)?
            .into_iter()
            .collect(),
        closed: texts(&view, "collapsed", NODES_MAX, TEXT_MAX)?
            .into_iter()
            .collect(),
        hidden: texts(&view, "faded", Zone::ALL.len(), 16)?
            .iter()
            .map(|k| zone_of(k))
            .collect(),
        kept: texts(&view, "pinned", NODES_MAX, ID_MAX)?
            .into_iter()
            .filter(|k| ids.contains(k.as_str()))
            .collect(),
    };
    let camera = view.get("camera").and_then(|c| {
        let pan = point(c.get("pan")?)?;
        let zoom = c.get("zoom")?.as_f64()? as f32;
        (MIN_ZOOM..=MAX_ZOOM)
            .contains(&zoom)
            .then_some((pan.to_vec2(), zoom))
    });
    let name = text(&doc, "name", NAME_MAX);
    Ok(Snapshot {
        name: if name.is_empty() {
            "Untitled map".into()
        } else {
            name
        },
        created: text(&doc, "created", 40),
        map,
        edits,
        camera,
    })
}

fn node_of(v: &Value) -> Result<Node, String> {
    if !v.is_object() {
        return Err("a card that isn't an object".into());
    }
    let id = text(v, "id", ID_MAX);
    if id.is_empty() {
        return Err("a card with no id".into());
    }
    let up = match v.get("up") {
        Some(u) if u.is_object() => Some((
            u.get("parent")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("card {id}: a link with no parent"))? as usize,
            link_of(&text(u, "link", 16)),
            u.get("port")
                .and_then(Value::as_str)
                .map(|s| input::line(s, TEXT_MAX)),
        )),
        _ => None,
    };
    let seen: Vec<String> = texts(v, "seen_by", LIST_MAX, 32)?;
    Ok(Node {
        zone: zone_of(&text(v, "zone", 16)),
        kind: text(v, "kind", TEXT_MAX),
        label: text(v, "label", TEXT_MAX),
        sub: text(v, "sub", TEXT_MAX),
        chips: texts(v, "chips", LIST_MAX, TEXT_MAX)?,
        details: texts(v, "details", LIST_MAX, TEXT_MAX)?,
        flags: texts(v, "flags", LIST_MAX, TEXT_MAX)?,
        ip: v
            .get("ip")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok()),
        mac: v
            .get("mac")
            .and_then(Value::as_str)
            .and_then(crate::net::mac::parse_mac),
        client: v.get("client").and_then(Value::as_bool).unwrap_or(false),
        seen_by: SEEN_BY
            .iter()
            .copied()
            .filter(|s| seen.iter().any(|k| k == s))
            .collect(),
        open: v
            .get("open")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_u64)
                    .filter_map(|p| u16::try_from(p).ok())
                    .take(LIST_MAX)
                    .collect()
            })
            .unwrap_or_default(),
        up,
        site: v
            .get("site")
            .and_then(Value::as_str)
            .map(|s| input::line(s, TEXT_MAX))
            .filter(|s| !s.is_empty()),
        // Not read: a gone card is one a rebuild found missing, and a
        // saved map is never rebuilt from data (a card marked gone in a
        // file would be taken out as if let go, its children cut loose).
        id,
        ..Default::default()
    })
}

/// Every card's parent is another card, no id is used twice, and
/// following parents always ends (no loops).
fn check_tree(nodes: &[Node]) -> Result<(), String> {
    let n = nodes.len();
    let mut ids = HashSet::new();
    for (i, node) in nodes.iter().enumerate() {
        if !ids.insert(node.id.as_str()) {
            return Err(format!("two cards share the id {}", node.id));
        }
        if let Some(p) = node.parent()
            && (p >= n || p == i)
        {
            return Err(format!(
                "card {} hangs off a card that isn't there",
                node.id
            ));
        }
    }
    // Each card's way up, marked as it's found to end: linear overall.
    let mut ends = vec![false; n];
    for i in 0..n {
        let mut path = Vec::new();
        let mut k = i;
        while !ends[k] {
            if path.len() > n {
                return Err(format!("card {} hangs off itself further up", nodes[i].id));
            }
            path.push(k);
            match nodes[k].parent() {
                Some(p) => k = p,
                None => break,
            }
        }
        for k in path {
            ends[k] = true;
        }
    }
    Ok(())
}

/// Read a saved map file: a regular file no bigger than `FILE_MAX`.
pub fn read(path: &Path) -> Result<Snapshot, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    if meta.len() > FILE_MAX {
        return Err(format!(
            "{} MB is far bigger than any saved map (at most {} MB)",
            meta.len() >> 20,
            FILE_MAX >> 20
        ));
    }
    let text = input::read_capped(path, FILE_MAX).ok_or("couldn't read it")?;
    from_json(&text)
}

/// Read a saved map handed over whole (Config Export, a drop).
pub fn parse(data: &[u8]) -> Result<Snapshot, String> {
    if data.len() as u64 > FILE_MAX {
        return Err(format!(
            "{} MB is far bigger than any saved map (at most {} MB)",
            data.len() >> 20,
            FILE_MAX >> 20
        ));
    }
    from_json(&String::from_utf8_lossy(data))
}

/// Where saved maps live: `maps` in the settings folder.
pub fn folder() -> Option<PathBuf> {
    Some(crate::theme::data_dir()?.join("maps"))
}

/// Every saved map in `dir`, oldest first, and a line for each file that
/// couldn't be read.
pub fn list(dir: &Path) -> (Vec<(PathBuf, Snapshot)>, Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (Vec::new(), Vec::new());
    };
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    let mut errs = Vec::new();
    if paths.len() > FILES_MAX {
        errs.push(format!(
            "{} saved maps: only the first {FILES_MAX} are listed",
            paths.len()
        ));
        paths.truncate(FILES_MAX);
    }
    let mut out = Vec::new();
    for p in paths {
        match read(&p) {
            Ok(s) => out.push((p, s)),
            Err(e) => errs.push(format!(
                "{}: {e}",
                p.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }
    out.sort_by(|a, b| a.1.created.cmp(&b.1.created).then(a.0.cmp(&b.0)));
    (out, errs)
}

/// A file name from a map's name: lower-case letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
        if s.len() >= 48 {
            break;
        }
    }
    let s = s.trim_end_matches('-');
    if s.is_empty() { "map".into() } else { s.into() }
}

/// A new file in `dir` for a map called `name`, not taken by another.
pub fn fresh_path(dir: &Path, name: &str) -> PathBuf {
    let base = slug(name);
    (1..)
        .map(|k| match k {
            1 => dir.join(format!("{base}.json")),
            k => dir.join(format!("{base}-{k}.json")),
        })
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join("map.json"))
}

/// Write a saved map, whole or not at all: a temporary file renamed over
/// the old one.
pub fn write(path: &Path, s: &Snapshot) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, to_json(s)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

impl MapView {
    /// A saved map's view: drawn from its own copy of the map, as it was
    /// left.
    pub(super) fn from_saved(s: Snapshot) -> Self {
        let mut v = Self {
            side: true,
            follow: s.camera.is_none(),
            frozen: Some(Box::new(s.map)),
            ..Default::default()
        };
        v.apply_edits(&s.edits);
        v.history.settle(s.edits);
        if let Some((pan, zoom)) = s.camera {
            (v.pan, v.zoom) = (pan, zoom);
        }
        v
    }

    /// This map as a saved map called `name`. `full` is the map with every
    /// site open (a live map is built again for it).
    pub(super) fn snapshot(&self, name: &str, created: String, full: Map) -> Snapshot {
        Snapshot {
            name: name.to_string(),
            created,
            map: full,
            edits: self.edits(),
            camera: (!self.follow && self.zoom > 0.0).then_some((self.pan, self.zoom)),
        }
    }

    /// A saved map: rebuilt from its own copy, only when a site was
    /// collapsed or expanded (or an undo asked for it).
    pub(super) fn refresh_saved(&mut self, ctx: &egui::Context) {
        if !self.stale && !self.map.nodes.is_empty() {
            return;
        }
        let Some(full) = &self.frozen else { return };
        self.stale = false;
        let mut map = copy_map(full);
        map.close_sites(&self.closed);
        self.map = map;
        self.relayout(ctx);
    }
}

/// internet - gateway - {switch - ap, server}, at two sites.
#[cfg(test)]
pub(super) fn sample() -> Snapshot {
    let node = |id: &str, zone, up: Option<usize>, site: &str| Node {
        id: id.into(),
        zone,
        kind: "KIND".into(),
        label: format!("{id} label"),
        sub: "10.0.0.1".into(),
        chips: vec!["chip".into()],
        details: vec!["detail".into()],
        flags: vec![],
        ip: Some("10.0.0.1".parse().unwrap()),
        mac: Some([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]),
        seen_by: vec!["sweep", "UniFi"],
        open: vec![22, 443],
        up: up.map(|p| (p, Link::Wired, Some("port 3".into()))),
        site: (!site.is_empty()).then(|| site.to_string()),
        ..Default::default()
    };
    let map = Map {
        nodes: vec![
            node("internet", Zone::Internet, None, ""),
            node("gw", Zone::Gateway, Some(0), "HQ"),
            node("sw", Zone::Switch, Some(1), "HQ"),
            node("ap", Zone::Wireless, Some(2), "Branch"),
            node("nas", Zone::Server, Some(1), "Branch"),
        ],
        findings: vec![Finding {
            sev: Sev::High,
            text: "Default password".into(),
            nodes: vec![1, 4],
        }],
        summary: "Built from a sweep".into(),
        hidden: 3,
        folded: ["c1".to_string()].into_iter().collect(),
        closed: Default::default(),
    };
    let mut edits = Edits {
        mode: Some(Mode::Tree),
        clients: Clients::Some,
        kept: vec!["sw".into()],
        ..Default::default()
    };
    edits.placed.insert("ap".into(), pos2(120.5, -40.0));
    edits.closed.insert("Branch".into());
    edits.hidden.insert(Zone::Server);
    edits.open.insert("sw".into());
    Snapshot {
        name: "Head office".into(),
        created: "2026-11-01T10:00:00Z".into(),
        map,
        edits,
        camera: Some((vec2(10.0, 20.0), 0.75)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_map_reads_back_as_it_was_written() {
        let s = sample();
        let text = to_json(&s);
        assert!(sniff(text.as_bytes()));
        let r = from_json(&text).unwrap();
        assert_eq!(r.name, "Head office");
        assert_eq!(r.created, s.created);
        assert_eq!(r.edits, s.edits);
        assert_eq!(r.camera, s.camera);
        assert_eq!(r.map.summary, s.map.summary);
        assert_eq!(r.map.hidden, 3);
        assert_eq!(r.map.folded, s.map.folded);
        assert_eq!(r.map.nodes.len(), 5);
        for (a, b) in r.map.nodes.iter().zip(&s.map.nodes) {
            assert_eq!(
                (&a.id, a.zone, &a.label, &a.chips, a.ip, a.mac, &a.seen_by),
                (&b.id, b.zone, &b.label, &b.chips, b.ip, b.mac, &b.seen_by)
            );
            assert_eq!((&a.up, &a.site, &a.open), (&b.up, &b.site, &b.open));
        }
        assert_eq!(r.map.findings[0].nodes, [1, 4]);
        assert_eq!(r.map.findings[0].sev, Sev::High);
        // Written again, the same text.
        assert_eq!(to_json(&r), text);
    }

    #[test]
    fn a_saved_map_opens_drawn_as_it_was_left() {
        let mut v = MapView::from_saved(sample());
        v.refresh_saved(&egui::Context::default());
        // Branch collapsed into one card; the pin and the move kept.
        assert!(v.map.nodes.iter().any(|n| n.id == "site:Branch"));
        assert!(v.map.tucked("ap"));
        assert_eq!(v.kept, ["sw"]);
        assert_eq!(v.mode(), Mode::Tree);
        assert!(!v.follow);
        assert_eq!(v.zoom, 0.75);
        // Nothing to undo yet; expanding the site is a step.
        assert_eq!(v.history.undo_label(), None);
        v.flip("Branch");
        v.track_edits();
        assert_eq!(v.history.undo_label(), Some("expand Branch"));
        v.refresh_saved(&egui::Context::default());
        assert!(v.map.nodes.iter().any(|n| n.id == "ap"));
        // Saved again, its full map and the change.
        let full = copy_map(v.frozen.as_ref().unwrap());
        let s = v.snapshot("Again", now(), full);
        assert!(s.edits.closed.is_empty());
        assert_eq!(s.map.nodes.len(), 5);
    }

    #[test]
    fn garbage_newer_and_oversized_files_are_refused() {
        for bad in ["", "not json", "[1,2]", r#"{"format":"other","version":1}"#] {
            let e = from_json(bad).err().unwrap();
            assert_eq!(e, "not an is-alive map file", "{bad}");
        }
        let newer = to_json(&sample()).replace("\"version\": 1", "\"version\": 2");
        let e = from_json(&newer).err().unwrap();
        assert!(e.contains("newer is-alive"), "{e}");
        let big = vec![b' '; FILE_MAX as usize + 1];
        assert!(parse(&big).unwrap_err().contains("far bigger"));
        // Too many cards.
        let mut s = sample();
        s.map.nodes = (0..NODES_MAX + 1)
            .map(|i| Node {
                id: format!("n{i}"),
                ..Default::default()
            })
            .collect();
        s.map.findings.clear();
        assert!(from_json(&to_json(&s)).unwrap_err().contains("up to"));
    }

    #[test]
    fn a_card_marked_gone_in_a_file_stays_with_its_children() {
        // gw has sw and nas below it: taken out as gone, they'd hang off
        // the wrong cards (or none).
        let mut doc: Value = serde_json::from_str(&to_json(&sample())).unwrap();
        doc["map"]["nodes"][1]["gone"] = json!(true);
        let s = from_json(&doc.to_string()).unwrap();
        assert!(s.map.nodes.iter().all(|n| !n.gone));
        let mut v = MapView::from_saved(s);
        v.closed.clear();
        let ctx = egui::Context::default();
        v.refresh_saved(&ctx);
        v.drop_released(&ctx);
        assert_eq!(v.map.nodes.len(), 5);
        let again = to_json(&v.snapshot("x", now(), copy_map(&v.map)));
        assert!(!again.contains("\"gone\""));
    }

    #[test]
    fn broken_links_and_loops_are_refused() {
        let mut s = sample();
        s.map.nodes[2].up = Some((9, Link::Wired, None));
        assert!(from_json(&to_json(&s)).unwrap_err().contains("isn't there"));
        let mut s = sample();
        // gw → sw → gw: a loop that never reaches the internet.
        s.map.nodes[1].up = Some((2, Link::Wired, None));
        assert!(from_json(&to_json(&s)).unwrap_err().contains("itself"));
        let mut s = sample();
        s.map.nodes[3].id = "sw".into();
        assert!(from_json(&to_json(&s)).unwrap_err().contains("share"));
    }

    #[test]
    fn strings_are_cleaned_and_cut() {
        let mut s = sample();
        s.name = format!("Evil\u{202E}name{}", "x".repeat(500));
        s.map.nodes[1].label = "a\u{202E}b\u{0007}c".into();
        let r = from_json(&to_json(&s)).unwrap();
        assert!(r.name.starts_with("Evilname"));
        assert_eq!(r.name.chars().count(), NAME_MAX);
        assert_eq!(r.map.nodes[1].label, "abc");
        // A pin or a move for a card that isn't on the map is dropped.
        let text = to_json(&sample()).replace(
            "\"pinned\": [\n      \"sw\"",
            "\"pinned\": [\n      \"nope\"",
        );
        assert!(from_json(&text).unwrap().edits.kept.is_empty());
    }

    #[test]
    fn slugs_and_fresh_paths() {
        assert_eq!(slug("Head Office / 2nd floor"), "head-office-2nd-floor");
        assert_eq!(slug("  ***  "), "map");
        assert_eq!(slug(&"a".repeat(100)).len(), 48);
        let dir = std::env::temp_dir().join(format!("is-alive-maps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = fresh_path(&dir, "Site A");
        assert_eq!(p, dir.join("site-a.json"));
        write(&p, &sample()).unwrap();
        assert_eq!(fresh_path(&dir, "Site A"), dir.join("site-a-2.json"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
