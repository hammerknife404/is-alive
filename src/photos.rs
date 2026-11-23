//! Portable note photos: resized JPEGs embedded in Markdown, without EXIF
//! metadata (the encoder writes none). PNG ones from before still read.
use base64::{Engine, engine::general_purpose::STANDARD};
use eframe::egui;
use std::io::Cursor;
const PREFIX: &str = "![Photo](data:image/jpeg;base64,";
const PREFIXES: [&str; 2] = [PREFIX, "![Photo](data:image/png;base64,"];
/// Most photo data the notes hold, all photos together (base64 bytes):
/// about forty at full size. Kept apart from the text's own limit.
pub const PHOTOS_MAX: usize = 32 * 1024 * 1024;
/// A photo's JPEG quality: a rack or a label stays readable at a tenth of
/// a PNG's size.
const QUALITY: u8 = 85;
pub fn import(path: &std::path::Path) -> Result<String, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 20 * 1024 * 1024 {
        return Err("Choose a photo smaller than 20 MB".into());
    }
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12000);
    limits.max_image_height = Some(12000);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?;
    let image = if image.width() > 1600 || image.height() > 1200 {
        image.thumbnail(1600, 1200)
    } else {
        image
    };
    let mut jpeg = Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, QUALITY)
        .encode_image(&image.to_rgb8())
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "\n{PREFIX}{})\n",
        STANDARD.encode(jpeg.into_inner())
    ))
}
pub fn payload(line: &str) -> Option<&str> {
    PREFIXES
        .iter()
        .find_map(|p| line.strip_prefix(p))?
        .strip_suffix(')')
        .filter(|s| {
            s.len() <= 12 * 1024 * 1024
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
        })
}
pub fn render(ui: &mut egui::Ui, line: &str) -> bool {
    let Some(encoded) = payload(line) else {
        return false;
    };
    let id = egui::Id::new(line);
    let cached = ui.ctx().data_mut(|d| d.get_temp::<egui::TextureHandle>(id));
    let texture = cached.or_else(|| {
        let bytes = STANDARD.decode(encoded).ok()?;
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1600);
        limits.max_image_height = Some(1200);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode().ok()?.to_rgba8();
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        );
        let texture = ui
            .ctx()
            .load_texture(format!("photo-{id:?}"), color, Default::default());
        ui.ctx().data_mut(|d| d.insert_temp(id, texture.clone()));
        Some(texture)
    });
    if let Some(texture) = texture {
        ui.add(
            egui::Image::new(&texture)
                .max_width(ui.available_width().min(640.0))
                .corner_radius(crate::theme::R_CONTAINER),
        );
    } else {
        ui.colored_label(ui.visuals().error_fg_color, "Photo could not be decoded");
    }
    true
}

/// Keep portable image bytes outside the editable text. Markers can be moved or
/// deleted like ordinary Markdown; expansion happens only for preview/save/export.
#[derive(Default)]
pub struct Attachments(std::collections::BTreeMap<String, String>);
impl Attachments {
    pub fn compact(&mut self, text: &str) -> String {
        let mut out = String::new();
        for line in text.split_inclusive('\n') {
            if payload(line.trim()).is_some() {
                let mut n = self.0.len() + 1;
                let marker = loop {
                    let marker = format!("![Photo {n}](attachment:is-alive-photo-{n})");
                    if !self.0.contains_key(&marker) && !text.contains(&marker) {
                        break marker;
                    }
                    n += 1;
                };
                self.0.insert(marker.clone(), line.trim().to_string());
                out.push_str(&marker);
                if line.ends_with('\n') {
                    out.push('\n');
                }
            } else {
                out.push_str(line);
            }
        }
        out
    }
    /// How much photo data is held for `text`: the photos its markers name.
    pub fn size_in(&self, text: &str) -> usize {
        self.0
            .iter()
            .filter(|(marker, _)| text.contains(marker.as_str()))
            .map(|(_, image)| image.len())
            .sum()
    }

    pub fn expand(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (marker, image) in &self.0 {
            out = out.replace(marker, image);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A camera-sized picture comes in as a 1600 × 1200 JPEG, a fraction
    /// of what the PNG was, that reads back.
    #[test]
    fn a_photo_comes_in_small_enough() {
        let dir = std::env::temp_dir().join(format!("is-alive-photo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rack.png");
        let img = image::RgbImage::from_fn(2400, 1800, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x ^ y) % 256) as u8])
        });
        img.save(&path).unwrap();
        let md = import(&path).unwrap();
        let data = payload(md.trim()).expect("a photo line");
        assert!(md.trim().starts_with(PREFIX));
        let jpeg = STANDARD.decode(data).unwrap();
        let back = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((back.width(), back.height()), (1600, 1200));
        assert!(data.len() < 3 * 1024 * 1024, "{} bytes", data.len());
        // An older PNG one still reads.
        assert!(payload("![Photo](data:image/png;base64,AAAA)").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn attachment_edits_and_reopen_preserve_photo_bytes() {
        let raw = "before\n![Photo](data:image/png;base64,AAAA)\nafter";
        let mut attachments = Attachments::default();
        let compact = attachments.compact(raw);
        assert!(!compact.contains("base64"));
        assert_eq!(attachments.expand(&compact), raw);
        let edited = compact.replace("before", "edited");
        let saved = attachments.expand(&edited);
        let mut reopened = Attachments::default();
        let editor = reopened.compact(&saved);
        assert_eq!(reopened.expand(&editor), saved);
        assert_eq!(attachments.expand("deleted the photo"), "deleted the photo");
    }
    #[test]
    fn photo_round_trip() {
        let p = std::env::temp_dir().join(format!("is-alive-photo-{}.png", std::process::id()));
        image::RgbaImage::new(4, 3).save(&p).unwrap();
        let md = import(&p).unwrap();
        std::fs::remove_file(p).unwrap();
        let bytes = STANDARD.decode(payload(md.trim()).unwrap()).unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 4);
        assert_eq!(crate::redact::sanitize(&md).0, md);
    }
}
