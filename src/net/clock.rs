//! Where this computer's clock comes from, for stamping notes: its time
//! zone and the time source it syncs to. Looked up once, in the background,
//! since it means running the system's own tools.

use std::process::Command;
use std::sync::OnceLock;

struct Clock {
    zone: Option<String>,
    source: Option<String>,
}

static CLOCK: OnceLock<Clock> = OnceLock::new();

/// Start the lookup; call once at startup.
pub fn warm() {
    std::thread::spawn(|| {
        let _ = CLOCK.set(Clock {
            zone: zone(),
            source: source(),
        });
    });
}

/// "**2026-10-31 03:08** (UTC-05:00 America/Chicago, synced to time.windows.com) "
pub fn stamp() -> String {
    let now = chrono::Local::now();
    let mut about = vec![format!("UTC{}", now.format("%:z"))];
    let clock = CLOCK.get();
    if let Some(z) = clock.and_then(|c| c.zone.as_deref()) {
        about[0] += &format!(" {z}");
    }
    match clock.map(|c| c.source.as_deref()) {
        Some(Some(s)) => about.push(s.to_string()),
        Some(None) => about.push("clock source unknown".into()),
        None => {}
    }
    format!(
        "**{}** ({}) ",
        now.format("%Y-%m-%d %H:%M"),
        about.join(", ")
    )
}

/// Output of a system tool, trimmed; None if it fails or prints nothing.
fn run(path: std::path::PathBuf, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up.
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

#[cfg(windows)]
fn system32(exe: &str) -> std::path::PathBuf {
    let root = std::env::var_os("WINDIR").map_or("C:\\Windows".into(), std::path::PathBuf::from);
    root.join("System32").join(exe)
}

/// "Central Standard Time".
#[cfg(windows)]
fn zone() -> Option<String> {
    run(system32("tzutil.exe"), &["/g"]).map(|z| super::sanitize(&z))
}

/// "America/Chicago": TZ, or where /etc/localtime points.
#[cfg(not(windows))]
fn zone() -> Option<String> {
    std::env::var("TZ")
        .ok()
        .filter(|z| !z.is_empty())
        .or_else(|| {
            let target = std::fs::read_link("/etc/localtime").ok()?;
            let t = target.to_string_lossy();
            Some(t.split_once("zoneinfo/")?.1.to_string())
        })
        .map(|z| super::sanitize(z.trim_start_matches(':')))
}

/// "synced to time.windows.com", "synced to DC01.contoso.local", or that
/// it runs on its own clock.
#[cfg(windows)]
fn source() -> Option<String> {
    let s = run(system32("w32tm.exe"), &["/query", "/source"])?;
    Some(describe(&s))
}

#[cfg(not(windows))]
fn source() -> Option<String> {
    // chrony (Fedora, RHEL): the reference it's tracking.
    if let Some(t) = super::system_tool("chronyc").and_then(|p| run(p, &["-n", "-c", "tracking"])) {
        let name = t.split(',').nth(1).unwrap_or("").trim();
        if !name.is_empty() && name != "127.127.1.1" {
            return Some(describe(name));
        }
    }
    // systemd-timesyncd (Ubuntu, Debian).
    let p = super::system_tool("timedatectl")?;
    match run(p.clone(), &["show-timesync", "-p", "ServerName", "--value"]) {
        Some(s) => Some(describe(&s)),
        None => match run(p, &["show", "-p", "NTPSynchronized", "--value"])?.as_str() {
            "yes" => Some("synced (NTP)".into()),
            _ => Some("not synced".into()),
        },
    }
}

/// A source name as tools print it ("time.windows.com,0x9", "Local CMOS
/// Clock") in words.
fn describe(raw: &str) -> String {
    let name = super::sanitize(raw.split(',').next().unwrap_or(raw).trim());
    let l = name.to_ascii_lowercase();
    if l.contains("cmos")
        || l.contains("free-running")
        || l.contains("local") && l.contains("clock")
    {
        "own clock, not synced".into()
    } else {
        format!("synced to {name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_in_words() {
        assert_eq!(
            describe("time.windows.com,0x9"),
            "synced to time.windows.com"
        );
        assert_eq!(
            describe("DC01.contoso.local"),
            "synced to DC01.contoso.local"
        );
        assert_eq!(describe("Local CMOS Clock"), "own clock, not synced");
        assert_eq!(
            describe("Free-running System Clock"),
            "own clock, not synced"
        );
        let s = stamp();
        assert!(s.starts_with("**") && s.contains("(UTC"), "{s}");
    }
}
