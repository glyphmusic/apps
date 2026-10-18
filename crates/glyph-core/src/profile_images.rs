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
    prepare_image(&image, banner)
}
fn prepare_image(image: &DynamicImage, banner: bool) -> Result<Vec<u8>> {
    let (w, h) = (image.width(), image.height());
    if w == 0 || h == 0 {
        bail!("empty image");
    }
    let image = if banner {
        let (cw, ch) = if w >= h.saturating_mul(3) {
            (h * 3, h)
        } else {
            (w, (w / 3).max(1))
        };
        image
            .crop_imm((w - cw) / 2, (h - ch) / 2, cw, ch)
            .thumbnail(1500, 500)
    } else {
        let side = w.min(h);
        image
            .crop_imm((w - side) / 2, (h - side) / 2, side, side)
            .thumbnail(512, 512)
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
            let bytes = prepare_image(&source, banner).unwrap();
            assert_eq!(&bytes[..4], b"RIFF");
            assert_eq!(&bytes[8..12], b"WEBP");
            assert!(bytes.len() < MAX_UPLOAD_BYTES);
            let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
            assert_eq!(image.dimensions(), size);
            assert_eq!(image.get_pixel(0, 0).0[3], 128);
        }
    }
}
