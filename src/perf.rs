//! The performance probe (#146). With `IS_ALIVE_PERF=<file.json>` set, the
//! app opens the Contoso example map, times its frames through a few
//! phases (see `PHASES`), reads its own memory and CPU time, writes what
//! it measured to the file and closes. `scripts/perf.ps1` runs it once
//! per renderer from a portable copy, so nothing of the user's is touched,
//! and turns the files into a table.
//!
//! Per frame it keeps the time since the previous frame (what the screen
//! shows: at the display's rate when nothing is slow) and eframe's CPU
//! time for the frame (layout, tessellation and handing it to the GPU,
//! without the wait for vsync). Per phase, the process's CPU use and
//! memory. GPU time and the graphics driver's own memory aren't seen.
//!
//! Off (and free) unless the variable is set. Window size, Animate and the
//! legend are set for the run and never saved.

use crate::{App, Tab};
use eframe::egui;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Instant;

/// Window size for every run: the field-laptop target (CLAUDE.md).
pub const SIZE: egui::Vec2 = egui::vec2(1280.0, 720.0);

/// Seconds a phase waits after its setup is in before it's measured: the
/// layout, the switch animation and the first textures settle.
const SETTLE: f64 = 2.0;

/// Seconds a phase is measured.
const MEASURE: f64 = 15.0;

/// Seconds the map may take to load before the run gives up.
const PATIENCE: f64 = 60.0;

/// The example map timed: the larger one (23 clinics, 81 devices).
const EXAMPLE: usize = 1;

struct Phase {
    name: &'static str,
    /// What it shows, for the report.
    what: &'static str,
    /// Run every frame until it returns true; then the phase settles and
    /// is measured.
    setup: fn(&mut App, &egui::Context) -> bool,
    /// Run every frame while settling and measuring: what keeps the phase
    /// what it is (packets flowing or not).
    hold: fn(&mut App, f64),
}

const PHASES: &[Phase] = &[
    Phase {
        name: "map-idle",
        what: "Flat map, nothing picked or moving: should hardly draw at all",
        setup: |a, _| {
            a.map.default_layout_for_shots();
            a.map.explode_for_shots(None, false)
        },
        hold: |a, now| a.map.flow_for_perf(false, false, now),
    },
    Phase {
        name: "map-flowing",
        what: "Flat map, a card picked, packets flowing on its links (Animate on): drawn at the display's rate",
        setup: |a, _| {
            a.map.select_for_shots();
            true
        },
        hold: |a, now| a.map.flow_for_perf(true, false, now),
    },
    Phase {
        name: "map-3d-spin",
        what: "Exploded 3D view spinning, a card picked, packets flowing",
        setup: |a, _| {
            let ready = a.map.explode_for_shots(Some(0.0), true);
            if ready {
                a.map.select_for_shots();
            }
            ready
        },
        hold: |a, now| a.map.flow_for_perf(true, true, now),
    },
];

/// One phase's frames: the gap since the previous frame and eframe's CPU
/// time for it, in milliseconds.
#[derive(Default)]
struct Frames {
    gap: Vec<f64>,
    cpu: Vec<f64>,
}

enum Stage {
    /// Opening the example map.
    Load {
        since: Option<f64>,
    },
    /// A phase's setup, then its settling.
    Setup {
        phase: usize,
        ready_at: Option<f64>,
    },
    Measure {
        phase: usize,
        from: f64,
        start: Mark,
    },
    Done,
}

/// The process's own counters at a moment.
#[derive(Clone, Copy)]
struct Mark {
    wall: Instant,
    cpu: Option<f64>,
}

impl Mark {
    fn now() -> Self {
        Self {
            wall: Instant::now(),
            cpu: process_cpu_secs(),
        }
    }
}

pub struct Perf {
    out: PathBuf,
    stage: Stage,
    frames: Frames,
    last: Option<Instant>,
    /// Frames during a measured phase with mouse or key input: the run
    /// was touched, and the report says so.
    touched: u32,
    /// Frames during a measured phase with the window not focused: the
    /// map's packets stop there, so the phase doesn't show what it says.
    unfocused: u32,
    phases: Vec<Value>,
    memory: Vec<Value>,
    started: Instant,
}

impl Perf {
    pub fn from_env() -> Option<Self> {
        let out = PathBuf::from(std::env::var_os("IS_ALIVE_PERF")?);
        Some(Self {
            out,
            stage: Stage::Load { since: None },
            frames: Frames::default(),
            last: None,
            touched: 0,
            unfocused: 0,
            phases: Vec::new(),
            memory: Vec::new(),
            started: Instant::now(),
        })
    }

    fn note_memory(&mut self, at: &str) {
        self.memory.push(json!({ "at": at, "memory": memory() }));
    }
}

impl App {
    /// Called first thing every frame while the probe runs.
    pub(crate) fn perf_step(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(mut p) = self.perf.take() else {
            return;
        };
        let now = ctx.input(|i| i.time);
        let gap = p.last.map(|t| t.elapsed().as_secs_f64() * 1000.0);
        p.last = Some(Instant::now());
        // The run's own setting: things move whatever Settings says, so
        // runs on two machines (or on battery) compare.
        self.look.animate = crate::theme::Animate::On;
        self.look.map_legend = true;
        match p.stage {
            Stage::Load { since } => {
                let first = since.is_none();
                let since = since.unwrap_or(now);
                if first {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(SIZE));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    p.note_memory("window open");
                    self.tab = Tab::Map;
                    self.export.open_example(ctx, EXAMPLE);
                }
                if !self.export.is_running() && self.map.cards() > 0 {
                    p.note_memory("map loaded");
                    p.stage = Stage::Setup {
                        phase: 0,
                        ready_at: None,
                    };
                } else if now - since > PATIENCE {
                    p.stage = Stage::Done;
                    write(&p, self, ctx, Some("the example map didn't load"));
                } else {
                    p.stage = Stage::Load { since: Some(since) };
                    ctx.request_repaint();
                }
            }
            Stage::Setup { phase, ready_at } => {
                let ph = &PHASES[phase];
                self.tab = Tab::Map;
                (ph.hold)(self, now);
                let ready_at = match ready_at {
                    Some(t) => t,
                    None if (ph.setup)(self, ctx) => now,
                    None => {
                        p.stage = Stage::Setup {
                            phase,
                            ready_at: None,
                        };
                        ctx.request_repaint();
                        self.perf = Some(p);
                        return;
                    }
                };
                p.stage = if now - ready_at >= SETTLE {
                    p.frames = Frames::default();
                    p.touched = 0;
                    p.unfocused = 0;
                    Stage::Measure {
                        phase,
                        from: now,
                        start: Mark::now(),
                    }
                } else {
                    Stage::Setup {
                        phase,
                        ready_at: Some(ready_at),
                    }
                };
                ctx.request_repaint();
            }
            Stage::Measure { phase, from, start } => {
                let ph = &PHASES[phase];
                (ph.hold)(self, now);
                if let Some(g) = gap {
                    p.frames.gap.push(g);
                }
                if let Some(c) = frame.info().cpu_usage {
                    p.frames.cpu.push(f64::from(c) * 1000.0);
                }
                if ctx.input(|i| i.events.iter().any(is_input)) {
                    p.touched += 1;
                }
                if ctx.input(|i| i.viewport().focused == Some(false)) {
                    p.unfocused += 1;
                }
                if now - from >= MEASURE {
                    let end = Mark::now();
                    let summary =
                        phase_summary(ph, &p.frames, start, end, (p.touched, p.unfocused));
                    p.phases.push(summary);
                    p.note_memory(ph.name);
                    if phase + 1 < PHASES.len() {
                        p.stage = Stage::Setup {
                            phase: phase + 1,
                            ready_at: None,
                        };
                        ctx.request_repaint();
                    } else {
                        p.stage = Stage::Done;
                        write(&p, self, ctx, None);
                    }
                } else {
                    // Idle draws only when something asks; this wakes the
                    // app at the phase's end and no sooner.
                    ctx.request_repaint_after_secs((from + MEASURE - now).max(0.0) as f32);
                }
            }
            Stage::Done => {}
        }
        self.perf = Some(p);
    }
}

fn is_input(e: &egui::Event) -> bool {
    matches!(
        e,
        egui::Event::PointerButton { .. }
            | egui::Event::PointerMoved(_)
            | egui::Event::MouseWheel { .. }
            | egui::Event::Key { .. }
            | egui::Event::Text(_)
    )
}

/// Writes the results and closes the window. A run that couldn't finish
/// writes what it has, with why.
fn write(p: &Perf, app: &App, ctx: &egui::Context, error: Option<&str>) {
    let (ppp, inner, monitor) = ctx.input(|i| {
        let v = i.viewport();
        (
            i.pixels_per_point,
            v.inner_rect.map(|r| [r.width(), r.height()]),
            v.monitor_size.map(|s| [s.x, s.y]),
        )
    });
    let doc = json!({
        "probe": 1,
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "renderer": crate::startup::renderer_line(),
        "on_battery": crate::power::on_battery(),
        "window_points": inner,
        "monitor_points": monitor,
        "pixels_per_point": ppp,
        "map": crate::skills::EXAMPLES[EXAMPLE].0,
        "map_cards": app.map.cards(),
        "settle_secs": SETTLE,
        "measure_secs": MEASURE,
        "run_secs": round(p.started.elapsed().as_secs_f64()),
        "phases": p.phases,
        "memory": p.memory,
        "error": error,
    });
    let text = serde_json::to_string_pretty(&doc).unwrap_or_default();
    if let Err(e) = std::fs::write(&p.out, text) {
        eprintln!("perf: {}: {e}", p.out.display());
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
}

fn phase_summary(
    ph: &Phase,
    f: &Frames,
    start: Mark,
    end: Mark,
    (touched, unfocused): (u32, u32),
) -> Value {
    let wall = end.wall.duration_since(start.wall).as_secs_f64();
    let cpu = start
        .cpu
        .zip(end.cpu)
        .map(|(a, b)| round((b - a) / wall * 100.0));
    json!({
        "name": ph.name,
        "what": ph.what,
        "secs": round(wall),
        "frames": f.gap.len(),
        "fps": round(f.gap.len() as f64 / wall),
        "gap_ms": stats(&f.gap),
        "cpu_ms": stats(&f.cpu),
        // Frames that took more than two of the phase's typical gap: a
        // stutter the eye can see.
        "hitches": hitches(&f.gap),
        "process_cpu_percent": cpu,
        "touched_frames": touched,
        "unfocused_frames": unfocused,
    })
}

fn round(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Percentile of sorted values, nearest rank.
fn pct(sorted: &[f64], p: f64) -> f64 {
    let i = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[i.clamp(1, sorted.len()) - 1]
}

fn stats(v: &[f64]) -> Value {
    if v.is_empty() {
        return Value::Null;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    json!({
        "p50": round(pct(&s, 50.0)),
        "p95": round(pct(&s, 95.0)),
        "p99": round(pct(&s, 99.0)),
        "max": round(s[s.len() - 1]),
        "mean": round(s.iter().sum::<f64>() / s.len() as f64),
    })
}

fn hitches(gaps: &[f64]) -> usize {
    if gaps.len() < 10 {
        return 0;
    }
    let mut s = gaps.to_vec();
    s.sort_by(f64::total_cmp);
    let typical = pct(&s, 50.0);
    gaps.iter().filter(|&&g| g > typical * 2.0).count()
}

/// CPU seconds this process has used, all threads, user and kernel.
#[cfg(target_os = "linux")]
fn process_cpu_secs() -> Option<f64> {
    // SAFETY: getrusage only writes the struct it's given.
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) } != 0 {
        return None;
    }
    let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
    Some(t(u.ru_utime) + t(u.ru_stime))
}

#[cfg(windows)]
fn process_cpu_secs() -> Option<f64> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let z = || FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut c, mut e, mut k, mut u) = (z(), z(), z(), z());
    // SAFETY: the pseudo-handle needs no closing; the call only writes the
    // four structs it's given.
    if unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) } == 0 {
        return None;
    }
    // 100-nanosecond ticks.
    let t =
        |f: FILETIME| (u64::from(f.dwHighDateTime) << 32 | u64::from(f.dwLowDateTime)) as f64 / 1e7;
    Some(t(k) + t(u))
}

#[cfg(not(any(target_os = "linux", windows)))]
fn process_cpu_secs() -> Option<f64> {
    None
}

/// Resident memory now and at its peak, and memory only this process
/// holds, in MB. Graphics memory on a dedicated GPU isn't in it.
#[cfg(target_os = "linux")]
fn memory() -> Value {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let kb = |key: &str| {
        status
            .lines()
            .find_map(|l| {
                l.strip_prefix(key)?
                    .trim()
                    .strip_suffix("kB")?
                    .trim()
                    .parse::<f64>()
                    .ok()
            })
            .map(|k| round(k / 1024.0))
    };
    json!({
        "resident_mb": kb("VmRSS:"),
        "peak_resident_mb": kb("VmHWM:"),
        "private_mb": kb("RssAnon:"),
    })
}

#[cfg(windows)]
fn memory() -> Value {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: all-zero is a valid PROCESS_MEMORY_COUNTERS_EX; the call
    // fills at most `cb` bytes of it.
    let mut m: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&raw mut m).cast::<PROCESS_MEMORY_COUNTERS>(),
            m.cb,
        )
    } != 0;
    if !ok {
        return Value::Null;
    }
    let mb = |b: usize| round(b as f64 / (1024.0 * 1024.0));
    json!({
        "resident_mb": mb(m.WorkingSetSize),
        "peak_resident_mb": mb(m.PeakWorkingSetSize),
        "private_mb": mb(m.PrivateUsage),
    })
}

#[cfg(not(any(target_os = "linux", windows)))]
fn memory() -> Value {
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_nearest_rank() {
        let s: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(pct(&s, 50.0), 50.0);
        assert_eq!(pct(&s, 95.0), 95.0);
        assert_eq!(pct(&s, 99.0), 99.0);
        assert_eq!(pct(&s, 100.0), 100.0);
        assert_eq!(pct(&[7.0], 99.0), 7.0);
        assert_eq!(stats(&[]), Value::Null);
    }

    #[test]
    fn a_hitch_is_twice_the_typical_gap() {
        let mut g = vec![16.7; 60];
        g.extend([40.0, 34.0, 30.0]);
        assert_eq!(hitches(&g), 2);
        // Too few frames to say what's typical (an idle phase).
        assert_eq!(hitches(&[1.0, 500.0]), 0);
    }

    #[test]
    fn phase_names_are_unique_and_plain() {
        let mut seen = std::collections::HashSet::new();
        for p in PHASES {
            assert!(seen.insert(p.name), "{} twice", p.name);
            assert!(
                p.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            );
        }
    }

    #[test]
    fn this_process_reads_its_own_counters() {
        if cfg!(any(target_os = "linux", windows)) {
            assert!(process_cpu_secs().is_some_and(|s| s >= 0.0));
            let m = memory();
            assert!(m["resident_mb"].as_f64().is_some_and(|v| v > 0.0), "{m}");
        }
    }
}
