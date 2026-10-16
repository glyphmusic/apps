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
