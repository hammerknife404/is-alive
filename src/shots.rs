//! Screenshot tours. With `IS_ALIVE_SHOTS=<dir>` set, the app visits every
//! tab, saves `<dir>/NN-<tab>.png` of each, then closes. With
//! `IS_ALIVE_SHOTS_SCENES=1` as well it takes the scene tour instead (see
//! `SCENES`): each tool after a run, the Map with cards picked, Settings and
//! the windows, each saved as `<dir>/<name>.png` once the run it shows has
//! finished, for checking a UI change by eye at a window size. A scene
//! whose run failed is tried once more, and one that still failed, or was
//! taken late, makes the app exit non-zero (see `failed`).
//! Off (and free) unless the variable is set.

use crate::{App, Tab, theme};
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Frames, and seconds, to let a tab settle (layout, fonts, the switch
/// animation) before its picture is taken.
const SETTLE: u32 = 30;
const SETTLE_SECS: f64 = 0.6;

/// Seconds a scene may wait for its run before it's taken anyway.
const PATIENCE: f64 = 120.0;

/// Seconds before a scene whose run failed is run once more (a
/// speed test refused with 429 often goes through a little later).
const RETRY: f64 = 45.0;

/// Set when a picture isn't what it should be: its run failed, it was
/// taken before the run finished, or it couldn't be saved.
static FAILED: AtomicBool = AtomicBool::new(false);

/// True once any picture of this tour went wrong: the app then exits with
/// status 1, so a script running the tour knows.
pub fn failed() -> bool {
    FAILED.load(Ordering::Relaxed)
}

/// What a failed scene does this frame.
enum Retry {
    /// Waits for its second try.
    Wait,
    /// Runs again now.
    Now,
    /// Already had its second try: taken as it is, and reported.
    GiveUp,
}

pub struct Shots {
    dir: PathBuf,
    next: usize,
    frames: u32,
    asked: bool,
    /// The scene tour: plain names, and each scene waits for its run.
    scenes: bool,
    /// The scene whose setup has run, and when it started.
    begun: Option<(usize, f64)>,
    /// When the current shot became ready.
    ready_at: Option<f64>,
    /// When the current scene's failed run is tried again.
    retry_at: Option<f64>,
    /// The scene that has had its second try.
    retried: Option<usize>,
    /// The scene already reported as failed (once, not every frame).
    reported: Option<usize>,
}

impl Shots {
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("IS_ALIVE_SHOTS")?);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self {
            dir,
            next: 0,
            frames: 0,
            asked: false,
            scenes: std::env::var_os("IS_ALIVE_SHOTS_SCENES").is_some_and(|v| v != "0"),
            begun: None,
            ready_at: None,
            retry_at: None,
            retried: None,
            reported: None,
        })
    }

    /// True for the scene tour (IS_ALIVE_SHOTS_SCENES).
    pub fn scenes(&self) -> bool {
        self.scenes
    }

    /// Called once a frame with the shot names; returns the index of the
    /// shot to show. Once `ready` (the classic tour's shots always are) a
    /// shot gets SETTLE frames and SETTLE_SECS before it's taken. Saves any
    /// screenshot that arrived and closes the window when done.
    pub fn step(&mut self, ctx: &egui::Context, names: &[&str], ready: bool) -> Option<usize> {
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            self.save(&img, names[self.next]);
            self.next += 1;
            self.frames = 0;
            self.asked = false;
            self.ready_at = None;
        }
        if self.next >= names.len() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return None;
        }
        let now = ctx.input(|i| i.time);
        let late = self
            .begun
            .is_some_and(|(i, t)| i == self.next && now - t > PATIENCE);
        if late && self.frames == 0 {
            self.fail(names[self.next], "didn't finish in time");
        }
        if ready || late {
            self.frames += 1;
            self.ready_at.get_or_insert(now);
        }
        let settled = self.ready_at.is_some_and(|t| now - t >= SETTLE_SECS);
        if self.frames >= SETTLE && settled && !self.asked {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.asked = true;
        }
        ctx.request_repaint();
        Some(self.next)
    }

    /// True once for each shot: the frame its setup should run.
    fn begin(&mut self, i: usize, now: f64) -> bool {
        if self.begun.is_some_and(|(b, _)| b == i) {
            return false;
        }
        self.begun = Some((i, now));
        true
    }

    /// A failed run: wait RETRY seconds and run it once more, then give up.
    fn retry(&mut self, i: usize, now: f64) -> Retry {
        if self.retried == Some(i) {
            return Retry::GiveUp;
        }
        let at = *self.retry_at.get_or_insert(now + RETRY);
        if now < at {
            return Retry::Wait;
        }
        self.retry_at = None;
        self.retried = Some(i);
        // PATIENCE counts from the second try.
        self.begun = Some((i, now));
        Retry::Now
    }

    /// Say what's wrong with the current picture (once) and remember it.
    fn fail(&mut self, name: &str, why: &str) {
        if self.reported != Some(self.next) {
            eprintln!("shots: {name}: {why}");
            self.reported = Some(self.next);
        }
        FAILED.store(true, Ordering::Relaxed);
    }

    fn save(&self, img: &egui::ColorImage, name: &str) {
        let [w, h] = img.size;
        let raw: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
        let name = name.to_lowercase().replace(' ', "-");
        let file = if self.scenes {
            format!("{name}.png")
        } else {
            format!("{:02}-{name}.png", self.next)
        };
        let path = self.dir.join(file);
        if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, raw)
            && let Err(e) = buf.save(&path)
        {
            eprintln!("shots: {}: {e}", path.display());
            FAILED.store(true, Ordering::Relaxed);
        }
    }
}

/// Where a scene's run is.
enum Ready {
    Wait,
    Yes,
    /// Finished without what the picture is meant to show.
    Failed(String),
}

/// A run that has finished, well or not.
fn over(running: bool, err: Option<&str>) -> Ready {
    match (running, err) {
        (true, _) => Ready::Wait,
        (false, None) => Ready::Yes,
        (false, Some(e)) => Ready::Failed(e.to_string()),
    }
}

/// One picture of the scene tour: its file name, the tab it shows, what to
/// do on its first frame, and (each frame) whether its run is over.
struct Scene {
    name: &'static str,
    tab: Tab,
    start: fn(&mut App, &egui::Context),
    ready: fn(&mut App) -> Ready,
}

const fn scene(
    name: &'static str,
    tab: Tab,
    start: fn(&mut App, &egui::Context),
    ready: fn(&mut App) -> Ready,
) -> Scene {
    Scene {
        name,
        tab,
        start,
        ready,
    }
}

fn nothing(_: &mut App, _: &egui::Context) {}

fn at_once(_: &mut App) -> Ready {
    Ready::Yes
}

/// The scene tour, in order. Runs go to public test targets only
/// (1.1.1.1, example.com, Cloudflare's download file) and to this
/// machine's own LAN address; nothing is swept or traced, and the public
/// IP is never looked up.
const SCENES: &[Scene] = &[
    scene("window", Tab::Ping, nothing, at_once),
    scene(
        "ping",
        Tab::Ping,
        |a, ctx| a.ping.run_shown(ctx, "1.1.1.1", 10),
        |a| over(a.ping.is_running(), a.ping.error()),
    ),
    scene(
        "port-check",
        Tab::Ports,
        |a, ctx| {
            let me = a
                .info
                .lan
                .map_or_else(|| "127.0.0.1".into(), |(ip, _)| ip.to_string());
            a.ports.run(ctx, &me);
        },
        |a| over(a.ports.is_running(), a.ports.error()),
    ),
    scene("traceroute", Tab::Trace, nothing, at_once),
    scene(
        "subnet-sweep",
        Tab::Sweep,
        |a, _| a.sweep.show_for_shots(crate::net::targets::EXAMPLES),
        at_once,
    ),
    scene(
        "dns-lookup",
        Tab::Dns,
        |a, ctx| a.dns.run(ctx, "example.com"),
        |a| over(a.dns.is_running(), a.dns.error()),
    ),
    scene(
        "advanced",
        Tab::Advanced,
        |a, ctx| a.advanced.run(ctx, "example.com"),
        |a| over(a.advanced.is_running(), a.advanced.error()),
    ),
    scene("config-export-empty", Tab::Export, nothing, at_once),
    scene(
        "map",
        Tab::Map,
        |a, ctx| {
            // The map's pictures show its legend, whatever was saved.
            a.look.map_legend = true;
            a.export.open_example(ctx, 0)
        },
        |a| over(a.export.is_running(), None),
    ),
    scene("config-export", Tab::Export, nothing, at_once),
    scene("map-selected", Tab::Map, nothing, |a| {
        a.map.select_for_shots();
        Ready::Yes
    }),
    scene(
        "map-zones",
        Tab::Map,
        // The legend would cover the boxes beside the panel.
        |a, _| a.look.map_legend = false,
        |a| {
            if a.map.zones_for_shots() {
                Ready::Yes
            } else {
                Ready::Wait
            }
        },
    ),
    scene(
        "map-sites",
        Tab::Map,
        |a, ctx| {
            a.look.map_legend = true;
            a.map.default_layout_for_shots();
            a.export.open_example(ctx, 1)
        },
        |a| over(a.export.is_running(), None),
    ),
    scene("notes", Tab::Notes, |a, _| a.notes.show_split(), at_once),
    scene(
        "speed-test",
        Tab::Speed,
        |a, ctx| a.speed.run(ctx),
        |a| over(a.speed.is_running(), a.speed.error().as_deref()),
    ),
    scene("subnet-calc", Tab::Calc, nothing, at_once),
    scene(
        "domain-join",
        Tab::Join,
        |a, ctx| a.join.check(ctx, true),
        |a| over(a.join.is_running(), None),
    ),
    scene("results", Tab::Results, nothing, at_once),
    // Over the Map, not the Results log: what's behind the window then
    // doesn't depend on the network the tour ran on.
    scene(
        "site-bundle",
        Tab::Map,
        |a, _| a.bundle.open = true,
        at_once,
    ),
    scene(
        "site-report",
        Tab::Map,
        |a, _| {
            a.bundle.open = false;
            a.report.open = true;
        },
        at_once,
    ),
    scene(
        "map-light",
        Tab::Map,
        |a, ctx| {
            a.report.open = false;
            theme::apply(ctx, theme::Theme::Light);
        },
        at_once,
    ),
    scene(
        "settings",
        Tab::Ping,
        |a, ctx| {
            theme::apply(ctx, a.look.theme);
            a.shots_settings = true;
        },
        at_once,
    ),
];

impl App {
    /// The scene tour: each scene's setup once, then its picture once its
    /// run has finished. A run that failed gets a second try; if that fails
    /// too, the picture is taken and the tour reported as failed.
    pub(crate) fn scene_shots_step(&mut self, ctx: &egui::Context) {
        let names: Vec<&str> = SCENES.iter().map(|s| s.name).collect();
        let now = ctx.input(|i| i.time);
        let Some(s) = &mut self.shots else { return };
        let i = s.next;
        let ready = match SCENES.get(i) {
            Some(scene) => {
                self.tab = scene.tab;
                if s.begin(i, now) {
                    (scene.start)(self, ctx);
                }
                let state = (scene.ready)(self);
                let Some(s) = &mut self.shots else { return };
                match state {
                    Ready::Wait => false,
                    Ready::Yes => true,
                    Ready::Failed(why) => match s.retry(i, now) {
                        Retry::Wait => false,
                        Retry::Now => {
                            eprintln!("shots: {}: {why}; trying once more", scene.name);
                            (scene.start)(self, ctx);
                            false
                        }
                        Retry::GiveUp => {
                            s.fail(scene.name, &why);
                            true
                        }
                    },
                }
            }
            None => true,
        };
        if let Some(s) = &mut self.shots {
            s.step(ctx, &names, ready);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SCENES;

    /// Each scene's picture is saved by its name: each once, file-safe.
    #[test]
    fn scene_names_are_unique_and_plain() {
        let mut seen = std::collections::HashSet::new();
        for s in SCENES {
            assert!(seen.insert(s.name), "{} twice", s.name);
            assert!(
                s.name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{}",
                s.name
            );
        }
    }
}
