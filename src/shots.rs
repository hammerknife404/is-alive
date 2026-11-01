//! Screenshot tour for UI work: with `IS_ALIVE_SHOTS=<dir>` set, the app
//! visits every tab, saves `<dir>/NN-<tab>.png` of each, then closes.
//! Off (and free) unless the variable is set.

use eframe::egui;
use std::path::PathBuf;

/// Frames to let a tab settle (layout, fonts, the switch animation).
const SETTLE: u32 = 30;

pub struct Shots {
    dir: PathBuf,
    next: usize,
    frames: u32,
    asked: bool,
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
        })
    }

    /// Called once a frame with the tab count; returns the tab index to show.
    /// Saves any screenshot that arrived and closes the window when done.
    pub fn step(&mut self, ctx: &egui::Context, tabs: &[&str]) -> Option<usize> {
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let [w, h] = img.size;
            let raw: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            let name = tabs[self.next].to_lowercase().replace(' ', "-");
            let path = self.dir.join(format!("{:02}-{name}.png", self.next));
            if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, raw)
                && let Err(e) = buf.save(&path)
            {
                eprintln!("shots: {}: {e}", path.display());
            }
            self.next += 1;
            self.frames = 0;
            self.asked = false;
        }
        if self.next >= tabs.len() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return None;
        }
        self.frames += 1;
        if self.frames >= SETTLE && !self.asked {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.asked = true;
        }
        ctx.request_repaint();
        Some(self.next)
    }
}
