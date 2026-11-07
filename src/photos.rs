//! Portable note photos: resized PNGs embedded in Markdown, without EXIF metadata.
use base64::{Engine, engine::general_purpose::STANDARD};
use eframe::egui;
use std::io::Cursor;
const PREFIX: &str = "![Photo](data:image/png;base64,";
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
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "\n{PREFIX}{})\n",
        STANDARD.encode(png.into_inner())
    ))
}
pub fn payload(line: &str) -> Option<&str> {
    line.strip_prefix(PREFIX)?.strip_suffix(')').filter(|s| {
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
                .corner_radius(8),
        );
    } else {
        ui.colored_label(ui.visuals().error_fg_color, "Photo could not be decoded");
    }
    true
}
#[cfg(test)]
mod tests {
    use super::*;
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
