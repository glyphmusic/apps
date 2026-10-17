//! ASCII art for covers: real covers rendered through the ramp, or a
//! generated pattern when there is no cover (end of the fallback chain).

use super::glow::RAMP;
use super::palette::{Palette, Rgb};
use noise::{NoiseFn, OpenSimplex};
use std::path::Path;

/// One row of text plus an optional per-row tint.
pub type AsciiRow = (String, Option<Palette>);

fn ramp_char(v: f32) -> char {
    let r = RAMP.as_bytes();
    let i = (v.clamp(0.0, 1.0) * (r.len() - 1) as f32).round() as usize;
    r[i] as char
}

/// Cover → `cols × rows` characters (cells are ~1:2, so rows ≈ cols / 2).
pub fn from_image(path: &Path, cols: u32, rows: u32) -> Option<Vec<AsciiRow>> {
    let img = image::open(path).ok()?;
    let small = img
        .resize_exact(cols, rows, image::imageops::FilterType::Triangle)
        .to_rgb8();
    let mut out = Vec::with_capacity(rows as usize);
    for y in 0..rows {
        let mut line = String::with_capacity(cols as usize);
        let mut acc = Rgb(0.0, 0.0, 0.0);
        for x in 0..cols {
            let p = small.get_pixel(x, y).0;
            let c = Rgb(p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
            acc = Rgb(acc.0 + c.0, acc.1 + c.1, acc.2 + c.2);
            line.push(ramp_char(c.luma().powf(0.8)));
        }
        let n = cols as f32;
        let avg = Rgb(acc.0 / n, acc.1 / n, acc.2 / n);
        let mx = avg.0.max(avg.1).max(avg.2).max(0.05);
        let tint = avg.scale(0.95 / mx);
        out.push((line, Some(Palette([tint, tint, tint]))));
    }
    Some(out)
}

/// One row of cover ASCII: the text plus colour runs `(byte start, len, colour)`.
pub type ColoredRow = (String, Vec<(usize, usize, Rgb)>);

/// The real cover as ASCII: each character's glyph follows the pixel's
/// brightness and its colour is the pixel's own colour (lifted a bit, since a
/// thin glyph shows less colour than a solid pixel). Neighbouring characters
/// with nearly the same colour share one run.
pub fn from_image_colored(path: &Path, cols: u32, rows: u32) -> Option<Vec<ColoredRow>> {
    let img = image::open(path).ok()?;
    let small = img
        .resize_exact(cols, rows, image::imageops::FilterType::Triangle)
        .to_rgb8();
    let quant = |c: Rgb| -> (u8, u8, u8) {
        let q = |v: f32| ((v.clamp(0.0, 1.0) * 15.0).round() as u8);
        (q(c.0), q(c.1), q(c.2))
    };
    // Stretch the cover's own luma range over the ramp, so flat covers still
    // get structure instead of one dense character everywhere.
    let lumas: Vec<f32> = small
        .pixels()
        .map(|p| Rgb(p.0[0] as f32 / 255.0, p.0[1] as f32 / 255.0, p.0[2] as f32 / 255.0).luma())
        .collect();
    let lo = lumas.iter().cloned().fold(1.0f32, f32::min);
    let hi = lumas.iter().cloned().fold(0.0f32, f32::max);
    let span = (hi - lo).max(0.08);
    let mut out = Vec::with_capacity(rows as usize);
    for y in 0..rows {
        let mut line = String::with_capacity(cols as usize);
        let mut runs: Vec<(usize, usize, Rgb)> = Vec::new();
        let mut last: Option<(u8, u8, u8)> = None;
        for x in 0..cols {
            let p = small.get_pixel(x, y).0;
            let c = Rgb(p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
            let l = ((c.luma() - lo) / span).clamp(0.0, 1.0);
            line.push(ramp_char(l.powf(0.9)));
            // Lift dark-but-coloured pixels so the hue stays visible.
            let mx = c.0.max(c.1).max(c.2).max(0.001);
            let target = (0.35 + 0.65 * mx).min(1.0);
            let shown = c.scale(target / mx);
            let q = quant(shown);
            if Some(q) == last {
                if let Some(r) = runs.last_mut() {
                    r.1 += 1;
                }
            } else {
                runs.push((x as usize, 1, shown));
                last = Some(q);
            }
        }
        out.push((line, runs));
    }
    Some(out)
}

/// Deterministic generated pattern (concentric noise rings) for a seed.
pub fn pattern(seed: &str, cols: u32, rows: u32) -> Vec<AsciiRow> {
    let h = seed
        .bytes()
        .fold(2166136261u32, |a, b| (a ^ b as u32).wrapping_mul(16777619));
    let noise = OpenSimplex::new(h);
    let mut out = Vec::with_capacity(rows as usize);
    for y in 0..rows {
        let mut line = String::with_capacity(cols as usize);
        for x in 0..cols {
            let u = x as f64 / cols as f64 - 0.5;
            let v = (y as f64 / rows as f64 - 0.5) * 1.0;
            let r = (u * u + v * v).sqrt();
            let n = noise.get([u * 3.0, v * 3.0]);
            let ring = ((r * 9.0 + n * 1.6).sin() * 0.5 + 0.5) as f32;
            let fall = (1.0 - (r as f32 * 1.6)).clamp(0.0, 1.0);
            line.push(ramp_char(ring * fall));
        }
        out.push((line, None));
    }
    out
}

/// A photo rasterised as coloured ASCII (the background's 5×7 bitmap font),
/// `w × h` pixels, cropped to fill like `object-fit: cover`. `focus_y` picks
/// the vertical crop centre (0 top .. 1 bottom; portraits look best ~0.3).
/// Returns straight-alpha BGRA for `RenderImage`.
pub fn raster(path: &Path, w: u32, h: u32, focus_y: f32) -> Option<image::RgbaImage> {
    use super::glow::FONT;
    const CW: u32 = 7; // cell incl. spacing; the glyph is 5×7 px
    const CH: u32 = 10;
    const P: u32 = 1;
    let img = image::open(path).ok()?.to_rgb8();
    let cols = w / CW;
    let rows = h / CH;
    // Crop the source to the grid's aspect, then sample one pixel per cell.
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let want = (cols * CW) as f32 / (rows * CH) as f32;
    let (cw, ch) = if iw / ih > want { (ih * want, ih) } else { (iw, iw / want) };
    let x0 = (iw - cw) / 2.0;
    let y0 = ((ih - ch) * focus_y.clamp(0.0, 1.0)).max(0.0);
    let cropped = image::imageops::crop_imm(&img, x0 as u32, y0 as u32, cw as u32, ch as u32).to_image();
    let small = image::imageops::resize(&cropped, cols, rows, image::imageops::FilterType::Triangle);
    let lumas: Vec<f32> = small
        .pixels()
        .map(|p| Rgb(p.0[0] as f32 / 255.0, p.0[1] as f32 / 255.0, p.0[2] as f32 / 255.0).luma())
        .collect();
    let lo = lumas.iter().cloned().fold(1.0f32, f32::min);
    let hi = lumas.iter().cloned().fold(0.0f32, f32::max);
    let span = (hi - lo).max(0.08);
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let n = RAMP.len();
    for r in 0..rows {
        for c in 0..cols {
            let p = small.get_pixel(c, r).0;
            let col = Rgb(p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
            let l = ((lumas[(r * cols + c) as usize] - lo) / span).clamp(0.0, 1.0);
            let idx = (l * (n - 1) as f32).round() as usize;
            if idx == 0 {
                continue;
            }
            let mx = col.0.max(col.1).max(col.2).max(0.001);
            let shown = col.scale((0.45 + 0.55 * mx).min(1.0) / mx);
            let (rr, gg, bb) = ((shown.0 * 255.0) as u8, (shown.1 * 255.0) as u8, (shown.2 * 255.0) as u8);
            // Fade out over the lower 55 % so the hero melts into the page.
            let fy = (r as f32 + 0.5) / rows as f32;
            let fade = 1.0 - ((fy - 0.45) / 0.55).clamp(0.0, 1.0);
            let fade = fade * fade * (3.0 - 2.0 * fade);
            let a = ((90.0 + 165.0 * l) * fade) as u8;
            if a < 4 {
                continue;
            }
            let ox = c * CW + 1;
            let oy = r * CH + 1;
            for (ry, bits) in FONT[idx].iter().enumerate() {
                for rx in 0..5u32 {
                    if bits & (0b10000 >> rx) == 0 {
                        continue;
                    }
                    for dy in 0..P {
                        for dx in 0..P {
                            let x = ox + rx * P + dx;
                            let y = oy + ry as u32 * P + dy;
                            if x < w && y < h {
                                let o = ((y * w + x) * 4) as usize;
                                buf[o] = bb;
                                buf[o + 1] = gg;
                                buf[o + 2] = rr;
                                buf[o + 3] = a;
                            }
                        }
                    }
                }
            }
        }
    }
    image::RgbaImage::from_raw(w, h, buf)
}

/// The photo itself (no ASCII), cropped like `object-fit: cover`, slightly
/// dimmed and faded out over the lower part. Straight-alpha BGRA.
pub fn photo_fade(path: &Path, w: u32, h: u32, focus_y: f32) -> Option<image::RgbaImage> {
    let img = image::open(path).ok()?.to_rgb8();
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let want = w as f32 / h as f32;
    let (cw, ch) = if iw / ih > want { (ih * want, ih) } else { (iw, iw / want) };
    let x0 = (iw - cw) / 2.0;
    let y0 = ((ih - ch) * focus_y.clamp(0.0, 1.0)).max(0.0);
    let cropped = image::imageops::crop_imm(&img, x0 as u32, y0 as u32, cw as u32, ch as u32).to_image();
    let small = image::imageops::resize(&cropped, w, h, image::imageops::FilterType::Triangle);
    let mut buf = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let fy = (y as f32 + 0.5) / h as f32;
        let f = 1.0 - ((fy - 0.4) / 0.6).clamp(0.0, 1.0);
        let a = (f * f * (3.0 - 2.0 * f) * 255.0) as u8;
        for x in 0..w {
            let px_ = small.get_pixel(x, y).0;
            let o = ((y * w + x) * 4) as usize;
            buf[o] = (px_[2] as f32 * 0.8) as u8;
            buf[o + 1] = (px_[1] as f32 * 0.8) as u8;
            buf[o + 2] = (px_[0] as f32 * 0.8) as u8;
            buf[o + 3] = a;
        }
    }
    image::RgbaImage::from_raw(w, h, buf)
}
