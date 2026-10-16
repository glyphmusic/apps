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
