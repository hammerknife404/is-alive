//! Embeds the icon, version info and application manifest in the Windows
//! .exe (Explorer, taskbar, Alt-Tab, DPI, no admin prompt). Other targets take the icon from the window or the AppImage.
//! Also lists every file under skills/ for `src/skills.rs` to embed.

use std::fmt::Write;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/is-alive.ico");
    println!("cargo:rerun-if-changed=packaging/windows/is-alive.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/is-alive.ico")
            // What Explorer shows under Properties → Details, and in the
            // taskbar and Task Manager (FileDescription).
            .set("ProductName", "is-alive")
            .set("FileDescription", "is-alive network tester")
            .set("InternalName", "is-alive")
            .set("OriginalFilename", "is-alive.exe")
            .set("CompanyName", "hammerknife404")
            .set("LegalCopyright", "Copyright 2026 hammerknife404")
            // Windows 10/11, per-monitor DPI, long paths, current-style
            // dialogs, and no administrator prompt.
            .set_manifest_file("packaging/windows/is-alive.manifest");
        res.compile().expect("failed to embed Windows icon and manifest");
    }
    skills();
}

/// `skills.rs` in OUT_DIR: `FILES`, each file under skills/ as
/// (path relative to skills/, bytes), in a fixed order.
fn skills() {
    println!("cargo:rerun-if-changed=skills");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    let mut out = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for f in &files {
        let rel = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        writeln!(
            out,
            "    ({rel:?}, include_bytes!({:?})),",
            f.display().to_string()
        )
        .unwrap();
    }
    out += "];\n";
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("skills.rs");
    std::fs::write(dest, out).expect("write skills.rs");
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        if name == "__pycache__" || name.to_string_lossy().starts_with('.') {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}
