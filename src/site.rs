//! Sites: one client site's notes, saved maps and card notes in a folder of
//! its own, and the configs it had open (by path, never copied: a backup
//! holds passwords). Switching saves the open site, clears what's in
//! memory (the configs and their secrets, the sweep, the live map, Results)
//! and opens the other, with no restart.
//!
//! Scratch is no site at all: the profile folder itself, where notes and
//! maps lived before sites existed, so nothing moves on upgrade. A site is
//! `sites/<folder>/` beside it, with `site.json` naming it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

const META: &str = "site.json";
const FORMAT: &str = "is-alive-site-1";
/// site.json is a few lines; anything bigger isn't one.
const META_MAX: u64 = 256 * 1024;
/// Longest site name.
pub const NAME_MAX: usize = 80;
/// Most configs a site remembers.
const CONFIGS_MAX: usize = 64;

/// The open site's folder name; None is Scratch.
static CURRENT: Mutex<Option<String>> = Mutex::new(None);

/// A site's folder name, or None for Scratch.
pub fn current() -> Option<String> {
    CURRENT.lock().map(|c| c.clone()).unwrap_or(None)
}

/// Open `folder` (None: Scratch). Unknown folders open Scratch.
pub fn set_current(folder: Option<String>) {
    let folder = folder
        .filter(|f| valid_folder(f) && sites_dir().is_some_and(|d| d.join(f).join(META).is_file()));
    if let Ok(mut c) = CURRENT.lock() {
        *c = folder;
    }
}

/// Where the open site keeps its files: the profile folder for Scratch.
pub fn dir() -> Option<PathBuf> {
    let base = crate::theme::data_dir()?;
    Some(match current() {
        Some(f) => base.join("sites").join(f),
        None => base,
    })
}

fn sites_dir() -> Option<PathBuf> {
    Some(crate::theme::data_dir()?.join("sites"))
}

/// Folder names are made here: lowercase letters, digits and dashes.
fn valid_folder(f: &str) -> bool {
    !f.is_empty()
        && f.len() <= 64
        && f.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !f.starts_with('-')
}

/// A folder name for `name`: "Harbor Dental — Front" is "harbor-dental-front".
fn slug(name: &str) -> String {
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
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "site".into() } else { s }
}

/// What site.json holds.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Meta {
    pub name: String,
    pub created: String,
    pub opened: String,
    /// Configs open when it was last left, by path.
    pub configs: Vec<PathBuf>,
}

impl Meta {
    fn to_json(&self) -> String {
        let configs: Vec<String> = self
            .configs
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let v = serde_json::json!({
            "format": FORMAT,
            "name": self.name,
            "created": self.created,
            "opened": self.opened,
            "configs": configs,
        });
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    /// Read back defensively: a size cap, short strings, at most
    /// CONFIGS_MAX paths. None if it isn't a site file.
    fn from_json(text: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        if v.get("format")?.as_str()? != FORMAT {
            return None;
        }
        let s = |k: &str, max: usize| {
            crate::input::line(v.get(k).and_then(|x| x.as_str()).unwrap_or(""), max)
        };
        let name = s("name", NAME_MAX);
        if name.trim().is_empty() {
            return None;
        }
        let configs = v
            .get("configs")
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|p| p.as_str())
                    .filter(|p| {
                        !p.is_empty() && p.len() <= 4096 && !p.chars().any(char::is_control)
                    })
                    .take(CONFIGS_MAX)
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default();
        Some(Meta {
            name,
            created: s("created", 40),
            opened: s("opened", 40),
            configs,
        })
    }
}

fn read_meta(folder: &Path) -> Option<Meta> {
    let text = crate::input::read_capped(&folder.join(META), META_MAX)?;
    Meta::from_json(&text)
}

fn write_meta(folder: &Path, m: &Meta) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let tmp = folder.join("site.json.tmp");
    crate::theme::write_private(&tmp, m.to_json())?;
    std::fs::rename(tmp, folder.join(META))
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The open site's details; None for Scratch.
pub fn meta() -> Option<Meta> {
    read_meta(&dir()?).filter(|_| current().is_some())
}

/// The open site's name, "Scratch" when there's none.
pub fn name() -> String {
    meta().map_or_else(|| "Scratch".into(), |m| m.name)
}

/// Remember the configs open in this site (only when they change).
pub fn remember_configs(paths: &[PathBuf]) {
    if current().is_none() {
        return;
    }
    let Some(d) = dir() else { return };
    let Some(mut m) = read_meta(&d) else { return };
    let paths: Vec<PathBuf> = paths.iter().take(CONFIGS_MAX).cloned().collect();
    if m.configs != paths {
        m.configs = paths;
        let _ = write_meta(&d, &m);
    }
}

/// Note when the open site was opened.
pub fn touch() {
    if current().is_none() {
        return;
    }
    let Some(d) = dir() else { return };
    if let Some(mut m) = read_meta(&d) {
        m.opened = now();
        let _ = write_meta(&d, &m);
    }
}

/// One site in the switcher.
#[derive(Clone, Debug)]
pub struct Info {
    /// None for Scratch.
    pub folder: Option<String>,
    pub name: String,
    pub opened: String,
    pub configs: usize,
    pub maps: usize,
    pub words: usize,
    pub path: PathBuf,
}

fn info(folder: Option<String>, path: PathBuf, meta: Option<Meta>) -> Info {
    let maps = std::fs::read_dir(path.join("maps"))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        })
        .unwrap_or(0);
    let words = crate::input::read_capped(&path.join("notes.md"), 4 * 1024 * 1024)
        .map_or(0, |t| t.split_whitespace().count());
    let m = meta.unwrap_or_default();
    Info {
        name: if folder.is_some() {
            m.name
        } else {
            "Scratch".into()
        },
        folder,
        opened: m.opened,
        configs: m.configs.len(),
        maps,
        words,
        path,
    }
}

/// Every site, most recently opened first, then Scratch.
pub fn list() -> Vec<Info> {
    let mut out = Vec::new();
    if let Some(sites) = sites_dir()
        && let Ok(rd) = std::fs::read_dir(&sites)
    {
        for e in rd.flatten().take(500) {
            let f = e.file_name().to_string_lossy().into_owned();
            if !valid_folder(&f) {
                continue;
            }
            let path = e.path();
            if let Some(m) = read_meta(&path) {
                out.push(info(Some(f), path, Some(m)));
            }
        }
    }
    out.sort_by(|a, b| b.opened.cmp(&a.opened).then(a.name.cmp(&b.name)));
    if let Some(base) = crate::theme::data_dir() {
        out.push(info(None, base, None));
    }
    out
}

/// Make a site called `name` and return its folder name.
pub fn create(name: &str) -> Result<String, String> {
    let name = crate::input::line(name, NAME_MAX);
    if name.trim().is_empty() {
        return Err("Give the site a name".into());
    }
    if list()
        .iter()
        .any(|s| s.folder.is_some() && s.name.eq_ignore_ascii_case(name.trim()))
    {
        return Err(format!("There's already a site called {}", name.trim()));
    }
    let sites = sites_dir().ok_or("No profile folder")?;
    let base = slug(&name);
    let folder = (1..1000)
        .map(|k| {
            if k == 1 {
                base.clone()
            } else {
                format!("{base}-{k}")
            }
        })
        .find(|f| !sites.join(f).exists())
        .ok_or("Too many sites with that name")?;
    let at = now();
    write_meta(
        &sites.join(&folder),
        &Meta {
            name: name.trim().to_string(),
            created: at.clone(),
            opened: at,
            configs: Vec::new(),
        },
    )
    .map_err(|e| format!("Couldn't make the site's folder: {e}"))?;
    Ok(folder)
}

/// Rename the site in `folder` (its folder keeps its name).
pub fn rename(folder: &str, name: &str) -> Result<(), String> {
    let name = crate::input::line(name, NAME_MAX);
    if name.trim().is_empty() {
        return Err("Give the site a name".into());
    }
    let d = sites_dir().ok_or("No profile folder")?.join(folder);
    let mut m = read_meta(&d).ok_or("That site is gone")?;
    m.name = name.trim().to_string();
    write_meta(&d, &m).map_err(|e| e.to_string())
}

/// Delete the site in `folder` and everything in it (not the open one).
pub fn delete(folder: &str) -> Result<(), String> {
    if !valid_folder(folder) {
        return Err("Not a site".into());
    }
    if current().as_deref() == Some(folder) {
        return Err("Switch to another site first".into());
    }
    let d = sites_dir().ok_or("No profile folder")?.join(folder);
    // Only a folder that is a site.
    read_meta(&d).ok_or("Not a site")?;
    std::fs::remove_dir_all(&d).map_err(|e| e.to_string())
}

/// "Today", "Yesterday" or the date, for the switcher.
pub fn when(rfc3339: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(rfc3339) else {
        return String::new();
    };
    let day = t.with_timezone(&chrono::Local).date_naive();
    let today = chrono::Local::now().date_naive();
    match (today - day).num_days() {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => day.format("%-d %b %Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_plain_folder_names() {
        assert_eq!(
            slug("Harbor Dental — Front office"),
            "harbor-dental-front-office"
        );
        assert_eq!(slug("  ../../etc  "), "etc");
        assert_eq!(slug("Ünïcode Clinic #7"), "n-code-clinic-7");
        assert_eq!(slug("!!!"), "site");
        assert!(valid_folder(&slug("Contoso Health / Clinic 7")));
        assert!(!valid_folder("../x"));
        assert!(!valid_folder("A"));
        assert!(!valid_folder(""));
    }

    #[test]
    fn meta_round_trips_and_rejects_others() {
        let m = Meta {
            name: "Harbor Dental".into(),
            created: "2026-11-09T05:00:00Z".into(),
            opened: "2026-11-09T06:00:00Z".into(),
            configs: vec![PathBuf::from("/home/tech/backups/harbor.unf")],
        };
        assert_eq!(Meta::from_json(&m.to_json()), Some(m));
        assert_eq!(
            Meta::from_json("{\"format\":\"other\",\"name\":\"x\"}"),
            None
        );
        assert_eq!(Meta::from_json("not json"), None);
        assert_eq!(
            Meta::from_json(&format!("{{\"format\":\"{FORMAT}\",\"name\":\"  \"}}")),
            None
        );
        // Control characters in a path, and too many paths, are dropped.
        let many: Vec<String> = (0..100).map(|i| format!("/c/{i}")).collect();
        let doc = serde_json::json!({"format": FORMAT, "name": "x", "configs": many, "extra": 1});
        assert_eq!(
            Meta::from_json(&doc.to_string()).unwrap().configs.len(),
            CONFIGS_MAX
        );
        let doc = serde_json::json!({"format": FORMAT, "name": "x", "configs": ["/a\nb", "/ok"]});
        assert_eq!(
            Meta::from_json(&doc.to_string()).unwrap().configs,
            vec![PathBuf::from("/ok")]
        );
    }

    #[test]
    fn when_is_relative_for_recent_days() {
        assert_eq!(when(&now()), "Today");
        assert_eq!(when("garbage"), "");
        assert!(when("2020-01-02T03:04:05Z").contains("2020"));
    }
}
