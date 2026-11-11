//! Whether the machine is running on its battery, for Settings › Animate's
//! "When plugged in". Read when a frame asks, at most every 30 seconds:
//! nothing wakes the app to check.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a reading holds.
const RECHECK: Duration = Duration::from_secs(30);

/// The machine runs on its battery (as of the last 30 seconds). A desktop,
/// or a system that won't say, never does.
pub fn on_battery() -> bool {
    static LAST: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, on)) = *last
        && at.elapsed() < RECHECK
    {
        return on;
    }
    let on = read();
    *last = Some((Instant::now(), on));
    on
}

/// A power supply as Linux lists it under /sys/class/power_supply.
#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
struct Supply {
    /// `type`: Battery, Mains, USB…
    kind: String,
    /// `scope`: Device for a wireless mouse's or headset's battery; empty
    /// or System for the machine's own.
    scope: String,
    /// `status`: Charging, Discharging, Not charging, Full, Unknown.
    status: String,
}

/// On battery: one of the machine's own batteries is discharging. Plugged
/// in, they're charging, full or "not charging"; a desktop has none. A
/// mouse's battery discharges all the time and doesn't count.
#[cfg(any(target_os = "linux", test))]
fn discharging(supplies: &[Supply]) -> bool {
    supplies
        .iter()
        .any(|s| s.kind == "Battery" && s.scope != "Device" && s.status == "Discharging")
}

#[cfg(target_os = "linux")]
fn read() -> bool {
    let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else {
        return false;
    };
    let field = |p: &std::path::Path, f: &str| {
        std::fs::read_to_string(p.join(f))
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let supplies: Vec<Supply> = dir
        .flatten()
        .map(|e| {
            let p = e.path();
            Supply {
                kind: field(&p, "type"),
                scope: field(&p, "scope"),
                status: field(&p, "status"),
            }
        })
        .collect();
    discharging(&supplies)
}

#[cfg(windows)]
fn read() -> bool {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    // SAFETY: a plain struct of integers, which the call fills in.
    let mut s: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // ACLineStatus: 0 on battery, 1 plugged in, 255 unknown.
    (unsafe { GetSystemPowerStatus(&mut s) }) != 0 && s.ACLineStatus == 0
}

#[cfg(not(any(target_os = "linux", windows)))]
fn read() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supply(kind: &str, scope: &str, status: &str) -> Supply {
        Supply {
            kind: kind.into(),
            scope: scope.into(),
            status: status.into(),
        }
    }

    #[test]
    fn only_the_machines_own_battery_discharging_counts() {
        // A desktop: no battery at all.
        assert!(!discharging(&[]));
        assert!(!discharging(&[supply("Mains", "", "")]));
        // A laptop on its charger, charging, full or held at a limit.
        for status in ["Charging", "Full", "Not charging", "Unknown"] {
            let l = [supply("Mains", "", ""), supply("Battery", "", status)];
            assert!(!discharging(&l), "{status}");
        }
        // Unplugged.
        assert!(discharging(&[
            supply("Mains", "", ""),
            supply("Battery", "", "Discharging")
        ]));
        // Two batteries: one drains while the other waits its turn.
        assert!(discharging(&[
            supply("Battery", "System", "Unknown"),
            supply("Battery", "System", "Discharging")
        ]));
        // A wireless mouse's battery always discharges: a desktop with one
        // isn't on battery.
        assert!(!discharging(&[
            supply("Mains", "", ""),
            supply("Battery", "Device", "Discharging")
        ]));
    }

    #[test]
    fn a_reading_holds_for_half_a_minute() {
        // Whatever this machine says, it says it twice in a row.
        assert_eq!(on_battery(), on_battery());
    }
}
