//! The map as a PNG: the picture the SVG holds, drawn with the CPU renderer
//! egui already ships (vello_cpu) in egui's own fonts, so it comes out the
//! same on every computer whatever fonts it has.

use super::scene::{Item, Rgb, Run, Scene, Stroke};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, MetadataProvider};
use std::borrow::Cow;
use vello_cpu::color::AlphaColor;
use vello_cpu::kurbo::{self, Affine, BezPath, Circle, RoundedRect, Shape};
use vello_cpu::{Level, Pixmap, RenderContext, RenderSettings, Resources};

/// Most pixels in one image (about 8000 × 8000): past this the scale comes down.
const MAX_PIXELS: f32 = 64.0e6;
/// Longest side, well inside what the renderer takes.
const MAX_SIDE: f32 = 16_000.0;

/// The scale `want` comes down to so the image stays a sensible size.
pub fn fit(size: eframe::egui::Vec2, want: f32) -> f32 {
    let area = (MAX_PIXELS / (size.x * size.y).max(1.0)).sqrt();
    let side = MAX_SIDE / size.x.max(size.y).max(1.0);
    want.min(area).min(side)
}

pub fn write(scene: &Scene, scale: f32) -> Result<Vec<u8>, String> {
    let scale = fit(scene.size, scale);
    let px = |v: f32| (v * scale).ceil().clamp(1.0, MAX_SIDE) as u16;
    let (w, h) = (px(scene.size.x), px(scene.size.y));
    let fonts = Fonts::new();
    let faces = fonts.faces();
    // The baseline SIMD level, not the fastest this CPU has: the same
    // pixels on every computer.
    let mut ctx = RenderContext::new_with(
        w,
        h,
        RenderSettings {
            level: Level::baseline(),
            num_threads: 0,
        },
    );
    ctx.set_transform(Affine::scale(f64::from(scale)));
    for item in &scene.items {
        draw(&mut ctx, &faces, item);
    }
    ctx.flush();
    let mut pix = Pixmap::new(w, h);
    ctx.render(&mut pix, &mut Resources::new());
    // The page is opaque: RGB is all a viewer needs.
    let rgb: Vec<u8> = pix
        .take_unpremultiplied()
        .into_iter()
        .flat_map(|p| [p.r, p.g, p.b])
        .collect();
    let mut out = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut out),
        &rgb,
        u32::from(w),
        u32::from(h),
        image::ExtendedColorType::Rgb8,
    )
    .map_err(|e| format!("Couldn't make the PNG: {e}"))?;
    Ok(out)
}

fn colour(rgb: Rgb, alpha: f32) -> AlphaColor<vello_cpu::color::Srgb> {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    AlphaColor::from_rgba8(rgb[0], rgb[1], rgb[2], a)
}

fn stroke_with(ctx: &mut RenderContext, st: &Stroke, path: &BezPath) {
    // SVG's defaults: square joins, flat ends.
    let mut k = kurbo::Stroke::new(f64::from(st.width))
        .with_join(kurbo::Join::Miter)
        .with_caps(kurbo::Cap::Butt);
    if let Some([a, b]) = st.dash {
        k = k.with_dashes(0.0, [f64::from(a), f64::from(b)]);
    }
    ctx.set_stroke(k);
    ctx.set_paint(colour(st.rgb, st.alpha));
    ctx.stroke_path(path);
}

fn draw(ctx: &mut RenderContext, faces: &Faces, item: &Item) {
    match item {
        Item::Rect {
            r,
            round,
            fill,
            stroke,
        } => {
            let path = RoundedRect::new(
                f64::from(r.min.x),
                f64::from(r.min.y),
                f64::from(r.max.x),
                f64::from(r.max.y),
                f64::from(*round),
            )
            .to_path(0.05);
            if let Some((c, a)) = fill {
                ctx.set_paint(colour(*c, *a));
                ctx.fill_path(&path);
            }
            if let Some(st) = stroke {
                stroke_with(ctx, st, &path);
            }
        }
        Item::Line { pts, stroke } => {
            let mut path = BezPath::new();
            for (k, p) in pts.iter().enumerate() {
                let p = (f64::from(p.x), f64::from(p.y));
                if k == 0 {
                    path.move_to(p);
                } else {
                    path.line_to(p);
                }
            }
            stroke_with(ctx, stroke, &path);
        }
        Item::Dot { c, r, rgb } => {
            ctx.set_paint(colour(*rgb, 1.0));
            ctx.fill_path(
                &Circle::new((f64::from(c.x), f64::from(c.y)), f64::from(*r)).to_path(0.05),
            );
        }
        Item::Text {
            at,
            size,
            mono,
            middle,
            runs,
        } => {
            let face = if *mono { &faces.mono } else { &faces.sans };
            let mut x = at.x;
            if *middle {
                let w: f32 = runs.iter().map(|r| faces.width(face, &r.text, *size)).sum();
                x -= w / 2.0;
            }
            for r in runs {
                x = text_run(ctx, faces, face, r, x, at.y, *size);
            }
        }
        Item::Card(items) => {
            for i in items {
                draw(ctx, faces, i);
            }
        }
    }
}

/// Draw one run with its baseline at `y`; where the next run starts.
fn text_run(
    ctx: &mut RenderContext,
    faces: &Faces,
    face: &FontRef,
    r: &Run,
    mut x: f32,
    y: f32,
    size: f32,
) -> f32 {
    let mut path = BezPath::new();
    for c in r.text.chars() {
        let Some((font, gid)) = faces.glyph(face, c) else {
            x += size * 0.5;
            continue;
        };
        if let Some(g) = font.outline_glyphs().get(gid) {
            let mut pen = Pen {
                path: &mut path,
                x,
                y,
            };
            let _ = g.draw(
                DrawSettings::unhinted(Size::new(size), LocationRef::default()),
                &mut pen,
            );
        }
        x += advance(font, gid, size);
    }
    ctx.set_paint(colour(r.rgb, 1.0));
    ctx.fill_path(&path);
    // egui's sans has one light weight: bold is the outline thickened.
    if r.weight >= 600 {
        ctx.set_stroke(kurbo::Stroke::new(f64::from(size) * 0.05));
        ctx.stroke_path(&path);
    }
    x
}

fn advance(font: &FontRef, gid: skrifa::GlyphId, size: f32) -> f32 {
    font.glyph_metrics(Size::new(size), LocationRef::default())
        .advance_width(gid)
        .unwrap_or(size * 0.5)
}

/// Glyph outlines into a path, flipped to y-down at (x, y).
struct Pen<'a> {
    path: &'a mut BezPath,
    x: f32,
    y: f32,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> (f64, f64) {
        (f64::from(self.x + x), f64::from(self.y - y))
    }
}

impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.path.move_to(p);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.path.line_to(p);
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (c, p) = (self.at(cx0, cy0), self.at(x, y));
        self.path.quad_to(c, p);
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (c0, c1, p) = (self.at(cx0, cy0), self.at(cx1, cy1), self.at(x, y));
        self.path.curve_to(c0, c1, p);
    }

    fn close(&mut self) {
        self.path.close_path();
    }
}

/// egui's built-in fonts: Ubuntu Light, Hack, and its two emoji fonts for
/// what those lack.
struct Fonts(Vec<Cow<'static, [u8]>>);

struct Faces<'a> {
    sans: FontRef<'a>,
    mono: FontRef<'a>,
    rest: Vec<FontRef<'a>>,
}

impl Fonts {
    fn new() -> Self {
        let defs = eframe::egui::FontDefinitions::default();
        let names = [
            "Ubuntu-Light",
            "Hack",
            "NotoEmoji-Regular",
            "emoji-icon-font",
        ];
        Fonts(
            names
                .iter()
                .filter_map(|n| defs.font_data.get(*n))
                .map(|d| d.font.clone())
                .collect(),
        )
    }

    fn faces(&self) -> Faces<'_> {
        let mut all: Vec<FontRef> = self.0.iter().filter_map(|d| FontRef::new(d).ok()).collect();
        assert!(all.len() >= 2, "egui ships its default fonts");
        let rest = all.split_off(2);
        let mono = all.pop().expect("two fonts");
        let sans = all.pop().expect("two fonts");
        Faces { sans, mono, rest }
    }
}

impl<'a> Faces<'a> {
    /// The font that has `c`: `face` first, then the emoji fonts, then
    /// the other of sans and mono.
    fn glyph<'s>(
        &'s self,
        face: &'s FontRef<'a>,
        c: char,
    ) -> Option<(&'s FontRef<'a>, skrifa::GlyphId)> {
        std::iter::once(face)
            .chain(&self.rest)
            .chain([&self.sans, &self.mono])
            .find_map(|f| Some((f, f.charmap().map(c)?)))
    }

    fn width(&self, face: &FontRef<'a>, s: &str, size: f32) -> f32 {
        s.chars()
            .map(|c| {
                self.glyph(face, c)
                    .map_or(size * 0.5, |(f, g)| advance(f, g, size))
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Look, scene};
    use super::*;
    use crate::map::layout::Mode;

    fn decode(png: &[u8]) -> image::RgbImage {
        image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .expect("decodes")
            .to_rgb8()
    }

    #[test]
    fn acme_png_decodes_at_the_size_asked() {
        super::super::tests::with_acme(Mode::Sites, |d| {
            for (scale, white) in [(1.0, false), (2.0, true)] {
                let s = scene::draw(d, Look { dark: true, white });
                let img = decode(&write(&s, scale).expect("png"));
                assert_eq!(img.width(), (s.size.x * scale).ceil() as u32);
                assert_eq!(img.height(), (s.size.y * scale).ceil() as u32);
                // The page's colour in the corner; something drawn on it.
                let corner = img.get_pixel(1, 1).0;
                assert_eq!(corner, if white { [255; 3] } else { [0x16, 0x18, 0x1c] });
                assert!(img.pixels().any(|p| p.0 != corner));
            }
        });
    }

    #[test]
    fn png_is_the_same_every_time() {
        super::super::tests::with_acme(Mode::Cluster, |d| {
            let s = scene::draw(
                d,
                Look {
                    dark: false,
                    white: false,
                },
            );
            assert_eq!(write(&s, 1.0).unwrap(), write(&s, 1.0).unwrap());
        });
    }

    #[test]
    fn a_huge_map_is_scaled_down_not_refused() {
        let size = eframe::egui::vec2(20_000.0, 9_000.0);
        let s = fit(size, 2.0);
        assert!(s < 1.0 && size.x * s <= MAX_SIDE && size.x * size.y * s * s <= MAX_PIXELS * 1.001);
        assert_eq!(fit(eframe::egui::vec2(800.0, 600.0), 2.0), 2.0);
    }

    #[test]
    fn text_is_drawn_in_egui_fonts() {
        let fonts = Fonts::new();
        let f = fonts.faces();
        assert!(f.width(&f.sans, "Gateway", 13.0) > 30.0);
        assert!(f.glyph(&f.sans, '…').is_some());
    }
}
