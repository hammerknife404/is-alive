//! Miami Vice backdrop: a 16-bit synthwave scene drawn into a small pixel
//! buffer (one art pixel = 2×2 points) and scaled up with hard edges, or the
//! user's own image with a slow pan. Animated at 10 frames a second in a
//! 4-second loop, only while the window has focus.

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Points per art pixel.
const PX: f32 = 2.0;
/// Art-pixel scale for sizes first tuned at 4 points per pixel.
const K: i32 = 2;
const FPS: f64 = 10.0;
const LOOP_FRAMES: i64 = 40;
/// Behind the panels when the backdrop is off.
pub const SOLID: Color32 = Color32::from_rgb(22, 14, 44);

#[derive(Clone, PartialEq, Debug, Default)]
pub enum Source {
    #[default]
    Scene,
    Image(PathBuf),
    None,
}

#[derive(Default)]
pub struct Backdrop {
    scene: Option<Scene>,
    scene_tex: Option<TextureHandle>,
    frame: Option<i64>,
    image: Option<(PathBuf, TextureHandle, egui::Vec2)>,
    /// Why the chosen image couldn't be shown.
    pub err: Option<String>,
}

impl Backdrop {
    /// Paint under the panels; call before they are shown.
    pub fn paint(&mut self, ctx: &egui::Context, source: &Source, animate: bool) {
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());
        let focused = ctx.input(|i| i.viewport().focused != Some(false));
        let time = ctx.input(|i| i.time);
        let moving = animate && focused;
        match source {
            Source::None => {
                painter.rect_filled(rect, 0.0, SOLID);
            }
            Source::Scene => {
                let (w, h) = (
                    ((rect.width() / PX).ceil() as usize).max(16),
                    ((rect.height() / PX).ceil() as usize).max(16),
                );
                if self.scene.as_ref().is_none_or(|s| (s.w, s.h) != (w, h)) {
                    self.scene = Some(Scene::new(w, h));
                    self.frame = None;
                }
                let frame = if animate { (time * FPS) as i64 } else { 0 };
                if self.frame != Some(frame) || self.scene_tex.is_none() {
                    let scene = self.scene.as_mut().expect("built above");
                    let img = scene.render(frame.rem_euclid(LOOP_FRAMES));
                    match &mut self.scene_tex {
                        Some(t) => t.set(img, TextureOptions::NEAREST),
                        None => {
                            self.scene_tex =
                                Some(ctx.load_texture("backdrop", img, TextureOptions::NEAREST))
                        }
                    }
                    self.frame = Some(frame);
                }
                if let Some(t) = &self.scene_tex {
                    // The buffer is a whole number of art pixels; crop the overhang.
                    let uv = egui::Rect::from_min_max(
                        egui::pos2(0.0, 0.0),
                        egui::pos2(
                            rect.width() / (w as f32 * PX),
                            rect.height() / (h as f32 * PX),
                        ),
                    );
                    painter.image(t.id(), rect, uv, Color32::WHITE);
                }
            }
            Source::Image(path) => {
                if self.image.as_ref().is_none_or(|i| &i.0 != path) {
                    self.image = None;
                    match load_image(path) {
                        Ok(img) => {
                            let size = egui::vec2(img.size[0] as f32, img.size[1] as f32);
                            let t = ctx.load_texture("backdrop-image", img, TextureOptions::LINEAR);
                            self.image = Some((path.clone(), t, size));
                            self.err = None;
                        }
                        Err(e) => self.err = Some(e),
                    }
                }
                match &self.image {
                    Some((_, t, size)) => {
                        let s = if animate { time } else { 0.0 };
                        painter.image(t.id(), rect, pan_uv(*size, rect.size(), s), Color32::WHITE);
                    }
                    None => {
                        painter.rect_filled(rect, 0.0, SOLID);
                    }
                }
            }
        }
        if moving && *source != Source::None {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn load_image(path: &Path) -> Result<ColorImage, String> {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let img = image::open(path).map_err(|e| format!("{name}: {e}"))?;
    // Big photos are scaled down: the window never needs more.
    let img = img.thumbnail(2560, 1600).to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

/// Cover-fit crop of an image, zoomed in a little and drifting slowly across
/// it and back over a minute ("Ken Burns").
fn pan_uv(img: egui::Vec2, view: egui::Vec2, time: f64) -> egui::Rect {
    let fit = (view.x / img.x).max(view.y / img.y);
    let zoom = 1.08;
    let w = (view.x / (img.x * fit * zoom)).min(1.0);
    let h = (view.y / (img.y * fit * zoom)).min(1.0);
    let p = (0.5 - 0.5 * (time * std::f64::consts::TAU / 60.0).cos()) as f32;
    let x = (1.0 - w) * p;
    let y = (1.0 - h) * (0.3 + 0.4 * p);
    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
}

// Palette.
const SKY: [(u8, u8, u8); 9] = [
    (18, 8, 46),
    (34, 12, 72),
    (62, 16, 98),
    (98, 22, 120),
    (140, 30, 134),
    (186, 44, 140),
    (226, 70, 138),
    (250, 104, 120),
    (255, 140, 100),
];
const SUN: [(u8, u8, u8); 5] = [
    (255, 226, 92),
    (255, 196, 88),
    (255, 160, 96),
    (255, 120, 120),
    (255, 86, 150),
];
const BAYER: [[f32; 4]; 4] = [
    [0.0, 8.0, 2.0, 10.0],
    [12.0, 4.0, 14.0, 6.0],
    [3.0, 11.0, 1.0, 9.0],
    [15.0, 7.0, 13.0, 5.0],
];

fn rgb(c: (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

struct Scene {
    w: usize,
    h: usize,
    /// Dithered sky, precomputed.
    sky: Vec<Color32>,
    stars: Vec<(i32, i32, f32, i32)>,
    buildings: Vec<(i32, i32, i32)>,
    buf: Vec<Color32>,
}

/// Small deterministic generator so the skyline is the same every launch.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() % (hi - lo).max(1) as u64) as i32
    }
}

impl Scene {
    fn horizon(h: usize) -> i32 {
        (h as f32 * 0.60) as i32
    }

    fn new(w: usize, h: usize) -> Self {
        let hz = Self::horizon(h);
        let mut sky = vec![Color32::BLACK; w * h];
        for y in 0..hz.max(0) as usize {
            let f = (y as f32 / hz as f32).powf(1.2) * (SKY.len() - 1) as f32;
            let (i, frac) = (f as usize, f.fract());
            for x in 0..w {
                let t = (BAYER[y % 4][x % 4] + 0.5) / 16.0;
                let j = if frac > t {
                    (i + 1).min(SKY.len() - 1)
                } else {
                    i
                };
                sky[y * w + x] = rgb(SKY[j]);
            }
        }
        let mut r = Lcg(7);
        let stars = (0..56)
            .map(|_| {
                (
                    r.range(0, w as i32),
                    r.range(0, hz / 2),
                    r.range(0, 628) as f32 / 100.0,
                    r.range(1, 3),
                )
            })
            .collect();
        let (wi, hf) = (w as i32, h as f32);
        let mut buildings = Vec::new();
        let mut x = 0;
        while x < wi {
            let bw = r.range(8 * K, 18 * K);
            let near_sun = ((x + bw / 2) - wi / 2).abs() < (wi as f32 * 0.17) as i32;
            let bh = if near_sun {
                r.range((hf * 0.03) as i32, (hf * 0.08) as i32)
            } else {
                r.range((hf * 0.08) as i32, (hf * 0.30) as i32)
            };
            buildings.push((x, bw, bh));
            x += bw + r.range(1, 3) * K;
        }
        Self {
            w,
            h,
            sky,
            stars,
            buildings,
            buf: vec![Color32::BLACK; w * h],
        }
    }

    fn set(&mut self, x: i32, y: i32, c: Color32) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.buf[y as usize * self.w + x as usize] = c;
        }
    }

    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Color32) {
        for y in y0.max(0)..=y1.min(self.h as i32 - 1) {
            for x in x0.max(0)..=x1.min(self.w as i32 - 1) {
                self.buf[y as usize * self.w + x as usize] = c;
            }
        }
    }

    /// Bresenham line, stamped `width` art pixels square.
    fn line(&mut self, a: (f32, f32), b: (f32, f32), c: Color32, width: i32) {
        let (mut x0, mut y0) = (a.0.round() as i32, a.1.round() as i32);
        let (x1, y1) = (b.0.round() as i32, b.1.round() as i32);
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let mut err = dx + dy;
        let half = width / 2;
        loop {
            self.rect(
                x0 - half,
                y0 - half,
                x0 - half + width - 1,
                y0 - half + width - 1,
                c,
            );
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn polyline(&mut self, pts: &[(f32, f32)], c: Color32, width: i32) {
        for p in pts.windows(2) {
            self.line(p[0], p[1], c, width);
        }
    }

    fn render(&mut self, frame: i64) -> ColorImage {
        use std::f32::consts::TAU;
        let ph = frame as f32 / LOOP_FRAMES as f32;
        let (w, h) = (self.w as i32, self.h as i32);
        let (wf, hf) = (w as f32, h as f32);
        let hz = Self::horizon(self.h);
        self.buf.copy_from_slice(&self.sky);

        // Stars blink on and off, each on its own beat.
        for i in 0..self.stars.len() {
            let (x, y, p, sz) = self.stars[i];
            if (p + ph * TAU * 2.0).sin() > -0.3 {
                self.rect(
                    x,
                    y,
                    x + sz - 1,
                    y + sz - 1,
                    Color32::from_rgb(255, 236, 255),
                );
            }
        }

        // Sun: a glow ring that flips between two pinks, banded disc, and
        // stripes stepping down through the lower half.
        let (cx, cy, r) = (w / 2, hz - (hf * 0.05) as i32, (hf * 0.2) as i32);
        let glow = if (ph * TAU).sin() > 0.0 {
            Color32::from_rgb(255, 70, 150)
        } else {
            Color32::from_rgb(220, 56, 140)
        };
        let (ri, ro) = (r + K, r + 2 * K);
        for y in (cy - ro)..=(cy + ro).min(hz - 1) {
            for x in (cx - ro)..=(cx + ro) {
                let d2 = (x - cx).pow(2) + (y - cy).pow(2);
                if d2 >= ri * ri && d2 <= ro * ro {
                    self.set(x, y, glow);
                }
            }
        }
        let step = (ph * 8.0 * K as f32) as i32;
        for y in (cy - r)..(cy + r).min(hz) {
            let half = ((r * r - (y - cy).pow(2)).max(0) as f32).sqrt() as i32;
            let band = SUN[(((y - (cy - r)) as f32 / (2 * r) as f32) * SUN.len() as f32) as usize
                % SUN.len()];
            let off = (y - cy - step).rem_euclid(8 * K);
            if y > cy - r / 6 && off < K + (y - cy).div_euclid(6) {
                continue;
            }
            self.rect(cx - half, y, cx + half, y, rgb(band));
        }

        // Skyline with teal rooftops and a scatter of lit windows.
        for i in 0..self.buildings.len() {
            let (bx, bw, bh) = self.buildings[i];
            self.rect(
                bx,
                hz - bh,
                bx + bw - 1,
                hz - 1,
                Color32::from_rgb(48, 26, 100),
            );
            self.rect(
                bx,
                hz - bh,
                bx + bw - 1,
                hz - bh,
                Color32::from_rgb(40, 210, 210),
            );
            let mut wy = hz - bh + 3 * K;
            while wy < hz - 2 * K {
                let mut wx = bx + 2 * K;
                while wx < bx + bw - 2 * K {
                    if (wx / K * 7 + wy / K * 13) % 5 == 0 {
                        self.rect(
                            wx,
                            wy,
                            wx + K - 1,
                            wy + K - 1,
                            Color32::from_rgb(255, 200, 120),
                        );
                    }
                    wx += 3 * K;
                }
                wy += 4 * K;
            }
        }

        // Ground and a grid drifting toward the viewer.
        self.rect(0, hz, w - 1, h - 1, Color32::from_rgb(20, 6, 42));
        let n = 12;
        for k in 0..=n {
            let t = (k as f32 + ph) / n as f32;
            let y = hz + ((hf - hz as f32) * t * t) as i32;
            let c = if t > 0.25 {
                Color32::from_rgb(255, 60, 170)
            } else {
                Color32::from_rgb(150, 40, 120)
            };
            self.rect(0, y, w - 1, y, c);
        }
        for k in -7..=7 {
            self.line(
                (cx as f32, hz as f32),
                (cx as f32 + k as f32 * wf / 5.0, hf),
                Color32::from_rgb(30, 170, 180),
                1,
            );
        }
        self.rect(0, hz, w - 1, hz, Color32::from_rgb(255, 150, 200));

        // Palm silhouettes, barely swaying.
        let palm = |s: &mut Self, px: f32, base: f32, height: f32, lean: f32, phase: f32| {
            let sway = 0.012 * (ph * TAU + phase).sin();
            let steps = 14 * K;
            let trunk: Vec<(f32, f32)> = (0..=steps)
                .map(|i| {
                    let t = i as f32 / steps as f32;
                    (
                        px + (lean + sway) * t.powf(1.6) * height * 0.4,
                        base - t * height,
                    )
                })
                .collect();
            let ink = Color32::from_rgb(12, 4, 28);
            s.polyline(&trunk, ink, 3 * K);
            let (tx, ty) = *trunk.last().expect("non-empty");
            let len = height * 0.42;
            for f in 0..8 {
                let a = -170.0 + 160.0 * f as f32 / 7.0;
                let ang = (a + 1.5 * (ph * TAU + phase).sin()).to_radians();
                let fs = 9 * K;
                let frond: Vec<(f32, f32)> = (0..=fs)
                    .map(|i| {
                        let s = i as f32 / fs as f32;
                        (
                            tx + ang.cos() * len * s,
                            ty + ang.sin() * len * s + len * 0.35 * s * s,
                        )
                    })
                    .collect();
                s.polyline(&frond, ink, 2 * K);
            }
        };
        let hz = hz as f32;
        palm(self, wf * 0.07, hz + hf * 0.06, hf * 0.55, 1.0, 0.0);
        palm(self, wf * 0.16, hz + hf * 0.03, hf * 0.40, 0.6, 1.3);
        palm(self, wf * 0.93, hz + hf * 0.06, hf * 0.55, -1.0, 2.1);
        palm(self, wf * 0.84, hz + hf * 0.03, hf * 0.42, -0.6, 0.7);

        ColorImage::new([self.w, self.h], self.buf.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_renders_every_frame_at_odd_sizes() {
        for (w, h) in [(16, 16), (491, 331), (960, 540)] {
            let mut s = Scene::new(w, h);
            for f in [0, 13, LOOP_FRAMES - 1] {
                let img = s.render(f);
                assert_eq!(img.size, [w, h]);
            }
        }
    }

    #[test]
    fn loop_is_seamless_and_moves() {
        let mut s = Scene::new(320, 200);
        let a = s.render(0).pixels;
        let b = s.render(LOOP_FRAMES / 2).pixels;
        assert_ne!(a, b, "animation changes the picture");
        // Frame 40 would be frame 0 again: callers wrap with rem_euclid.
        assert_eq!(s.render(0).pixels, a);
    }

    #[test]
    fn pan_stays_inside_the_image() {
        for t in [0.0, 15.0, 30.0, 45.0] {
            let uv = pan_uv(egui::vec2(1920.0, 1080.0), egui::vec2(980.0, 660.0), t);
            assert!(uv.min.x >= 0.0 && uv.min.y >= 0.0 && uv.max.x <= 1.0001 && uv.max.y <= 1.0001);
        }
    }
}
