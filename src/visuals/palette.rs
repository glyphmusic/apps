//! Song palette: 2–3 dominant colours from the cover (median cut), and a
//! 1.5 s cross-fade between palettes.

use gpui::{Hsla, Rgba};
use std::path::Path;
use std::time::{Duration, Instant};

pub const FADE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb(pub f32, pub f32, pub f32);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0.0, 0.0, 0.0);
    pub const WHITE: Rgb = Rgb(1.0, 1.0, 1.0);

    pub const fn hex(h: u32) -> Rgb {
        Rgb(
            ((h >> 16) & 0xff) as f32 / 255.0,
            ((h >> 8) & 0xff) as f32 / 255.0,
            (h & 0xff) as f32 / 255.0,
        )
    }
    pub fn lerp(self, o: Rgb, t: f32) -> Rgb {
        Rgb(
            self.0 + (o.0 - self.0) * t,
            self.1 + (o.1 - self.1) * t,
            self.2 + (o.2 - self.2) * t,
        )
    }
    pub fn scale(self, k: f32) -> Rgb {
        Rgb(self.0 * k, self.1 * k, self.2 * k)
    }
    /// Mix towards white.
    pub fn lighten(self, t: f32) -> Rgb {
        self.lerp(Rgb::WHITE, t)
    }
    pub fn luma(self) -> f32 {
        0.2126 * self.0 + 0.7152 * self.1 + 0.0722 * self.2
    }
    pub fn saturation(self) -> f32 {
        let mx = self.0.max(self.1).max(self.2);
        let mn = self.0.min(self.1).min(self.2);
        if mx <= 0.0 {
            0.0
        } else {
            (mx - mn) / mx
        }
    }
    pub fn hsla(self, a: f32) -> Hsla {
        Rgba {
            r: self.0.clamp(0.0, 1.0),
            g: self.1.clamp(0.0, 1.0),
            b: self.2.clamp(0.0, 1.0),
            a,
        }
        .into()
    }
}

/// Three dominant colours of a song, ordered dark → bright.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette(pub [Rgb; 3]);

impl Palette {
    pub fn lerp(&self, o: &Palette, t: f32) -> Palette {
        Palette([
            self.0[0].lerp(o.0[0], t),
            self.0[1].lerp(o.0[1], t),
            self.0[2].lerp(o.0[2], t),
        ])
    }
    /// Accent for UI tints (chips, active lyric line).
    pub fn accent(&self) -> Rgb {
        self.0[1]
    }

    /// Deterministic palette from a string (track without cover): keeps every
    /// song visually distinct until the cover arrives.
    pub fn from_seed(seed: &str) -> Palette {
        let mut h: u32 = 2166136261;
        for b in seed.bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(16777619);
        }
        let hue = (h % 360) as f32 / 360.0;
        let c = |dh: f32, s: f32, l: f32| -> Rgb {
            let hs: Rgba = gpui::hsla((hue + dh).fract(), s, l, 1.0).into();
            Rgb(hs.r, hs.g, hs.b)
        };
        Palette([c(0.0, 0.85, 0.30), c(0.07, 0.9, 0.55), c(0.14, 0.95, 0.72)])
    }

    /// Median cut over a downsampled cover. Near-black / grey pixels are
    /// dropped so the glow picks the colourful part of the artwork.
    pub fn from_image(path: &Path) -> Option<Palette> {
        let img = image::open(path).ok()?.thumbnail(64, 64).to_rgb8();
        let mut px: Vec<[u8; 3]> = img
            .pixels()
            .map(|p| p.0)
            .filter(|p| {
                let r = Rgb(p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
                r.luma() > 0.06 && (r.saturation() > 0.18 || r.luma() > 0.85)
            })
            .collect();
        if px.len() < 16 {
            px = img.pixels().map(|p| p.0).collect();
        }
        if px.is_empty() {
            return None;
        }
        let mut boxes = vec![px];
        while boxes.len() < 6 {
            let (idx, ch, range) = boxes
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let (c, r) = widest_channel(b);
                    (i, c, r)
                })
                .max_by_key(|(_, _, r)| *r)?;
            if range < 8 || boxes[idx].len() < 2 {
                break;
            }
            let mut b = boxes.swap_remove(idx);
            b.sort_unstable_by_key(|p| p[ch]);
            let hi = b.split_off(b.len() / 2);
            boxes.push(b);
            boxes.push(hi);
        }
        let mut cols: Vec<(Rgb, f32)> = boxes
            .iter()
            .filter(|b| !b.is_empty())
            .map(|b| {
                let n = b.len() as f32;
                let s = b.iter().fold([0f32; 3], |a, p| {
                    [a[0] + p[0] as f32, a[1] + p[1] as f32, a[2] + p[2] as f32]
                });
                let c = Rgb(s[0] / n / 255.0, s[1] / n / 255.0, s[2] / n / 255.0);
                (c, n * (0.35 + c.saturation()))
            })
            .collect();
        cols.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut picked: Vec<Rgb> = Vec::new();
        for (c, _) in &cols {
            if picked.iter().all(|p| dist(*p, *c) > 0.18) {
                picked.push(*c);
            }
            if picked.len() == 3 {
                break;
            }
        }
        if picked.is_empty() {
            return None;
        }
        while picked.len() < 3 {
            let last = *picked.last().unwrap();
            picked.push(last.lighten(0.35));
        }
        picked.sort_by(|a, b| a.luma().total_cmp(&b.luma()));
        // Glows need saturated, bright-ish colours: normalise the max channel.
        let boost = |c: Rgb| {
            let mx = c.0.max(c.1).max(c.2).max(0.05);
            c.scale((0.95 / mx).min(2.5))
        };
        Some(Palette([
            boost(picked[0]).scale(0.75),
            boost(picked[1]),
            boost(picked[2]),
        ]))
    }
}

fn widest_channel(b: &[[u8; 3]]) -> (usize, u8) {
    let mut lo = [255u8; 3];
    let mut hi = [0u8; 3];
    for p in b {
        for c in 0..3 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
        }
    }
    (0..3)
        .map(|c| (c, hi[c].saturating_sub(lo[c])))
        .max_by_key(|(_, r)| *r)
        .unwrap_or((0, 0))
}

fn dist(a: Rgb, b: Rgb) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt()
}

/// Cross-fades from the previous to the target palette over [`FADE`].
#[derive(Debug, Clone)]
pub struct PaletteFade {
    from: Option<Palette>,
    to: Option<Palette>,
    start: Instant,
}

impl Default for PaletteFade {
    fn default() -> Self {
        Self { from: None, to: None, start: Instant::now() - FADE }
    }
}

impl PaletteFade {
    pub fn set(&mut self, target: Option<Palette>) {
        if self.to == target {
            return;
        }
        let (cur, blend) = self.current();
        self.from = if blend > 0.5 { cur } else { None };
        self.to = target;
        self.start = Instant::now();
    }

    fn progress(&self) -> f32 {
        let t = (self.start.elapsed().as_secs_f32() / FADE.as_secs_f32()).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// `(palette, blend)`: `blend` is how much the palette replaces the
    /// preset's native colours (fades in/out when there is no song).
    pub fn current(&self) -> (Option<Palette>, f32) {
        let t = self.progress();
        match (self.from, self.to) {
            (Some(a), Some(b)) => (Some(a.lerp(&b, t)), 1.0),
            (None, Some(b)) => (Some(b), t),
            (Some(a), None) => (Some(a), 1.0 - t),
            (None, None) => (None, 0.0),
        }
    }

    pub fn target(&self) -> Option<Palette> {
        self.to
    }
}
