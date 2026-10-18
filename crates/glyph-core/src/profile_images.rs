//! Bounded profile image preparation, run only on background/blocking workers.
use anyhow::{bail, Result};
use image::DynamicImage;
use std::path::Path;
pub const MAX_SOURCE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_UPLOAD_BYTES: usize = 512 * 1024;

pub fn encode_webp(image: &DynamicImage, quality: f32) -> Result<Vec<u8>> {
    let rgba = image.to_rgba8();
    Ok(webp::Encoder::from_rgba(&rgba, rgba.width(), rgba.height())
        .encode_simple(false, quality)
        .map_err(|e| anyhow::anyhow!("WebP encoding failed: {e:?}"))?
        .to_vec())
}
pub fn prepare(path: &Path, banner: bool) -> Result<Vec<u8>> {
    prepare_crop(path, banner, Crop::default())
}

/// User framing from the crop dialog: `zoom` >= 1 over the largest centered
/// square / 3:1 region, `cx`/`cy` its center as fractions of the image.
#[derive(Clone, Copy, Debug)]
pub struct Crop {
    pub zoom: f32,
    pub cx: f32,
    pub cy: f32,
}

impl Default for Crop {
    fn default() -> Self {
        Self { zoom: 1.0, cx: 0.5, cy: 0.5 }
    }
}

/// Crop region `(x, y, w, h)` in pixels for an image of `w` x `h`.
pub fn crop_rect(w: u32, h: u32, banner: bool, c: Crop) -> (u32, u32, u32, u32) {
    let aspect = if banner { 3.0 } else { 1.0 };
    let (fw, fh) = (w as f32, h as f32);
    let (bw, bh) = if fw >= fh * aspect { (fh * aspect, fh) } else { (fw, fw / aspect) };
    let z = c.zoom.max(1.0);
    let (cw, ch) = ((bw / z).max(1.0), (bh / z).max(1.0));
    let x = (c.cx * fw - cw / 2.0).clamp(0.0, fw - cw);
    let y = (c.cy * fh - ch / 2.0).clamp(0.0, fh - ch);
    (x as u32, y as u32, (cw as u32).max(1).min(w), (ch as u32).max(1).min(h))
}

/// Pixel size of an image file without decoding it.
pub fn dimensions(path: &Path) -> Result<(u32, u32)> {
    Ok(image::ImageReader::open(path)?.with_guessed_format()?.into_dimensions()?)
}

pub fn prepare_crop(path: &Path, banner: bool, crop: Crop) -> Result<Vec<u8>> {
    if std::fs::metadata(path)?.len() > MAX_SOURCE_BYTES {
        bail!("profile image source exceeds 20 MiB");
    }
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    reader.limits(limits);
    let image = reader.decode()?;
    prepare_image(&image, banner, crop)
}
fn prepare_image(image: &DynamicImage, banner: bool, crop: Crop) -> Result<Vec<u8>> {
    let (w, h) = (image.width(), image.height());
    if w == 0 || h == 0 {
        bail!("empty image");
    }
    let (x, y, cw, ch) = crop_rect(w, h, banner, crop);
    let image = if banner {
        image.crop_imm(x, y, cw, ch).thumbnail(1500, 500)
    } else {
        image.crop_imm(x, y, cw, ch).thumbnail(512, 512)
    };
    for quality in [82.0, 70.0, 58.0] {
        let encoded = encode_webp(&image, quality)?;
        if encoded.len() <= MAX_UPLOAD_BYTES {
            return Ok(encoded);
        }
    }
    bail!("profile image exceeds upload limit after compression")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_webp_is_bounded_preserves_alpha_and_has_expected_dimensions() {
        let source = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1800,
            900,
            image::Rgba([90, 130, 180, 128]),
        ));
        for (banner, size) in [(false, (512, 512)), (true, (1500, 500))] {
            let bytes = prepare_image(&source, banner, Crop::default()).unwrap();
            assert_eq!(&bytes[..4], b"RIFF");
            assert_eq!(&bytes[8..12], b"WEBP");
            assert!(bytes.len() < MAX_UPLOAD_BYTES);
            let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
            assert_eq!(image.dimensions(), size);
            assert_eq!(image.get_pixel(0, 0).0[3], 128);
        }
    }
}
