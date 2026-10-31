//! Fonts for Notes and the text viewer: egui's own Sans and Mono, plus the
//! log-friendly monospace fonts this computer already has (Consolas,
//! Cascadia Mono, DejaVu Sans Mono…). Nothing is bundled; a font is read
//! from disk only when it's picked.

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const SANS: &str = "Sans";
pub const MONO: &str = "Mono";

/// (name shown, file names it ships as), in menu order.
const KNOWN: &[(&str, &[&str])] = &[
    ("Consolas", &["consola.ttf"]),
    (
        "Cascadia Mono",
        &[
            "CascadiaMono.ttf",
            "CascadiaMono-Regular.ttf",
            "CascadiaMono-Regular.otf",
        ],
    ),
    (
        "JetBrains Mono",
        &["JetBrainsMono-Regular.ttf", "JetBrainsMono-Regular.otf"],
    ),
    ("DejaVu Sans Mono", &["DejaVuSansMono.ttf"]),
    ("Liberation Mono", &["LiberationMono-Regular.ttf"]),
    ("Noto Sans Mono", &["NotoSansMono-Regular.ttf"]),
    (
        "Fira Mono",
        &["FiraMono-Regular.ttf", "FiraMono-Regular.otf"],
    ),
    (
        "Source Code Pro",
        &["SourceCodePro-Regular.ttf", "SourceCodePro-Regular.otf"],
    ),
    (
        "Ubuntu Mono",
        &["UbuntuMono-R.ttf", "UbuntuMono-Regular.ttf"],
    ),
    ("Lucida Console", &["lucon.ttf"]),
    ("Courier New", &["cour.ttf"]),
];

/// Folders fonts are installed in.
fn dirs() -> Vec<PathBuf> {
    let mut d = Vec::new();
    if cfg!(windows) {
        let root = std::env::var_os("WINDIR").map_or(PathBuf::from(r"C:\Windows"), PathBuf::from);
        d.push(root.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            d.push(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
        }
    } else {
        d.push("/usr/share/fonts".into());
        d.push("/usr/local/share/fonts".into());
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            d.push(home.join(".local/share/fonts"));
            d.push(home.join(".fonts"));
        }
    }
    d
}

/// Every known font file under `dir`, a few folders deep.
fn scan(dir: &Path, depth: u8, found: &mut Vec<(&'static str, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                scan(&p, depth - 1, found);
            }
            continue;
        }
        let Some(file) = p.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        for (name, files) in KNOWN {
            if files.iter().any(|f| f.eq_ignore_ascii_case(file))
                && !found.iter().any(|(n, _)| n == name)
            {
                found.push((name, p.clone()));
            }
        }
    }
}

/// The installed fonts from `KNOWN`, in menu order; looked up once.
fn installed() -> &'static [(&'static str, PathBuf)] {
    static FOUND: OnceLock<Vec<(&'static str, PathBuf)>> = OnceLock::new();
    FOUND.get_or_init(|| {
        let mut found = Vec::new();
        for d in dirs() {
            scan(&d, 4, &mut found);
        }
        found.sort_by_key(|(n, _)| KNOWN.iter().position(|(k, _)| k == n));
        found
    })
}

/// Every font the menu offers: Sans, Mono, then what's installed.
pub fn names() -> Vec<&'static str> {
    [SANS, MONO]
        .into_iter()
        .chain(installed().iter().map(|(n, _)| *n))
        .collect()
}

/// The font that's been loaded into egui, if any besides its own.
static LOADED: Mutex<Option<String>> = Mutex::new(None);

/// The family to draw `name` in, loading it from disk the first time.
/// Anything that can't be loaded falls back to Mono.
pub fn family(ctx: &egui::Context, name: &str) -> FontFamily {
    match name {
        SANS => return FontFamily::Proportional,
        MONO => return FontFamily::Monospace,
        _ => {}
    }
    let Ok(mut loaded) = LOADED.lock() else {
        return FontFamily::Monospace;
    };
    let wanted = FontFamily::Name(name.into());
    if loaded.as_deref() == Some(name) {
        // set_fonts takes effect from the next frame: until egui has the
        // family, draw in Mono rather than ask for a font it can't find.
        return if ctx.fonts(|f| f.definitions().families.contains_key(&wanted)) {
            wanted
        } else {
            ctx.request_repaint();
            FontFamily::Monospace
        };
    }
    let Some((_, path)) = installed().iter().find(|(n, _)| *n == name) else {
        return FontFamily::Monospace;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return FontFamily::Monospace;
    };
    let mut defs = FontDefinitions::default();
    defs.font_data
        .insert(name.into(), FontData::from_owned(bytes).into());
    // Its own glyphs first, then egui's for anything it lacks (emoji too).
    let mut chain = vec![name.to_string()];
    chain.extend(defs.families[&FontFamily::Monospace].iter().cloned());
    defs.families.insert(FontFamily::Name(name.into()), chain);
    ctx.set_fonts(defs);
    *loaded = Some(name.to_string());
    ctx.request_repaint();
    FontFamily::Monospace
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_known_files_by_name_only() {
        let dir = std::env::temp_dir().join(format!("is-alive-fonts-{}", std::process::id()));
        let sub = dir.join("truetype/dejavu");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("DejaVuSansMono.ttf"), b"x").unwrap();
        std::fs::write(dir.join("CONSOLA.TTF"), b"x").unwrap();
        std::fs::write(dir.join("Arial.ttf"), b"x").unwrap();
        let mut found = Vec::new();
        scan(&dir, 4, &mut found);
        let names: Vec<_> = found.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"DejaVu Sans Mono") && names.contains(&"Consolas"));
        assert_eq!(names.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_or_builtin_names_fall_back() {
        let ctx = egui::Context::default();
        assert_eq!(family(&ctx, SANS), FontFamily::Proportional);
        assert_eq!(family(&ctx, MONO), FontFamily::Monospace);
        assert_eq!(family(&ctx, "No Such Font"), FontFamily::Monospace);
        assert_eq!(&names()[..2], [SANS, MONO]);
    }
}
