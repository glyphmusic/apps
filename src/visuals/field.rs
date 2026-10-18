//! The glow field: four preset shapes, evaluated analytically on the CPU at
//! cell resolution. Shapes stay fixed per preset; colours come either from the
//! preset's native ramp or from the song palette.

use super::palette::{Palette, Rgb};
use noise::{NoiseFn, OpenSimplex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Preset {
    /// Diagonal wedge from bottom-left to top-right.
    Strahl,
    /// Horizontal band, wide and white-hot in the middle.
    Band,
    /// Arc rising from below.
    Sonnenaufgang,
    /// Bright sky on top, a dark arc eating in from below.
    Horizont,
}

impl Preset {
    pub const ALL: [Preset; 4] = [
        Preset::Strahl,
        Preset::Band,
        Preset::Sonnenaufgang,
        Preset::Horizont,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Strahl => "strahl",
            Preset::Band => "band",
            Preset::Sonnenaufgang => "sonnenaufgang",
            Preset::Horizont => "horizont",
        }
    }

    pub fn from_seed(seed: &str) -> Preset {
        let h = seed.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
        Self::ALL[(h % 4) as usize]
    }

    /// Native colours: `(cool, warm)` ramps, each dark → core.
    fn native(self) -> (Ramp, Ramp) {
        match self {
            Preset::Strahl => (
                Ramp([
                    Rgb::hex(0x050f5c),
                    Rgb::hex(0x1d5cff),
                    Rgb::hex(0x46dcff),
                    Rgb::hex(0xffffff),
                ]),
                Ramp([
                    Rgb::hex(0x3a0e06),
                    Rgb::hex(0xff6516),
                    Rgb::hex(0xffcc3a),
                    Rgb::hex(0xfff8e0),
                ]),
            ),
            Preset::Band => {
                let r = Ramp([
                    Rgb::hex(0x16052f),
                    Rgb::hex(0x6526d6),
                    Rgb::hex(0xb88dff),
                    Rgb::hex(0xffffff),
                ]);
                (r, r)
            }
            Preset::Sonnenaufgang => {
                let r = Ramp([
                    Rgb::hex(0x360404),
                    Rgb::hex(0xb11d0b),
                    Rgb::hex(0xff781d),
                    Rgb::hex(0xfff1a6),
                ]);
                (r, r)
            }
            Preset::Horizont => (
                Ramp([
                    Rgb::hex(0x10042a),
                    Rgb::hex(0x6a29b6),
                    Rgb::hex(0xff6c90),
                    Rgb::hex(0xfff2f2),
                ]),
                Ramp([
                    Rgb::hex(0x10042a),
                    Rgb::hex(0x8a2fc0),
                    Rgb::hex(0xff8a7a),
                    Rgb::hex(0xffffff),
                ]),
            ),
        }
    }

    /// `theme`: the colour theme's glow colours, in place of the preset's own.
    pub fn ramps(self, theme: Option<&Palette>, palette: Option<&Palette>, blend: f32) -> (Ramp, Ramp) {
        let (c, w) = match theme {
            Some(t) => {
                let [a, b, d] = t.0;
                (Ramp([a.scale(0.35), a, b, b.lighten(0.85)]), Ramp([a.scale(0.35), b, d, d.lighten(0.85)]))
            }
            None => self.native(),
        };
        match palette {
            Some(p) if blend > 0.0 => {
                let [a, b, d] = p.0;
                let pc = Ramp([a.scale(0.35), a, b, b.lighten(0.85)]);
                let pw = Ramp([a.scale(0.35), b, d, d.lighten(0.85)]);
                (c.lerp(&pc, blend), w.lerp(&pw, blend))
            }
            _ => (c, w),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Ramp(pub [Rgb; 4]);

impl Ramp {
    pub fn lerp(&self, o: &Ramp, t: f32) -> Ramp {
        Ramp([
            self.0[0].lerp(o.0[0], t),
            self.0[1].lerp(o.0[1], t),
            self.0[2].lerp(o.0[2], t),
            self.0[3].lerp(o.0[3], t),
        ])
    }
    /// `v` in 0..=1 (dark → core).
    pub fn at(&self, v: f32) -> Rgb {
        let v = v.clamp(0.0, 1.0) * 3.0;
        let i = (v as usize).min(2);
        self.0[i].lerp(self.0[i + 1], v - i as f32)
    }
}

/// Time-varying parameters shared by every sample of one frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct Motion {
    pub t: f32,
    /// Slow drift (aspect units).
    pub dx: f32,
    pub dy: f32,
    /// Shape size multiplier (breathing + bass).
    pub size: f32,
    /// Overall gain (mids + playing).
    pub gain: f32,
    /// 0..1, how lively the organic noise is.
    pub energy: f32,
}

impl Motion {
    /// `playing`: 0..1 eased; bands: smoothed bass / mid / high.
    pub fn new(noise: &OpenSimplex, t: f32, playing: f32, bass: f32, mids: f32) -> Motion {
        use std::f32::consts::TAU;
        let n = |a: f64, b: f64| noise.get([a, b]) as f32;
        // 20–40 s cycles: breathing 28 s, drift ~34 s.
        let breath = (TAU * t / 28.0).sin();
        let drift_t = (t / 34.0) as f64;
        let amp = 0.035 + 0.035 * playing;
        Motion {
            t,
            dx: n(drift_t, 3.7) * amp * 2.0,
            dy: n(11.1, drift_t) * amp,
            size: 1.0 + breath * (0.04 + 0.05 * playing) + bass * 0.32,
            gain: 0.86 + 0.06 * breath * (1.0 - playing) + mids * 0.38 + playing * 0.08,
            energy: 0.4 + 0.6 * playing,
        }
    }
}

#[inline]
fn gauss(x: f32) -> f32 {
    (-x * x).exp()
}

#[inline]
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Brightness `b` (0..~1.2) and warm/cool mix `s` (0..1) at `(x, y)`;
/// `x` is in aspect units (0..aspect), `y` in 0..1 top → bottom.
pub fn sample(p: Preset, x: f32, y: f32, aspect: f32, m: &Motion) -> (f32, f32) {
    let x = x - m.dx;
    let y = y - m.dy;
    match p {
        Preset::Strahl => {
            // Beam from bottom-left (0, 1.05) to top-right (aspect, -0.05).
            let (ax, ay) = (-0.05 * aspect, 1.08);
            let (bx, by) = (aspect * 1.02, -0.08);
            let (dx, dy) = (bx - ax, by - ay);
            let len = (dx * dx + dy * dy).sqrt();
            let (ux, uy) = (dx / len, dy / len);
            let (px, py) = (x - ax, y - ay);
            let s = (px * ux + py * uy) / len;
            let d = (px * uy - py * ux).abs();
            let sc = s.clamp(0.0, 1.0);
            let w = (0.02 + 0.17 * sc.powf(1.15)) * m.size;
            let core = gauss(d / w);
            let halo = gauss(d / (w * 3.0)) * 0.26;
            let along = smoothstep(-0.05, 0.25, s) * (0.55 + 0.45 * sc);
            ((core + halo) * along, smoothstep(0.35, 1.0, s))
        }
        Preset::Band => {
            let u = (x / aspect) * 2.0 - 1.0;
            let c = 0.52;
            let profile = (1.0 - u * u).max(0.0).powf(0.85);
            let w = (0.025 + 0.20 * profile) * m.size;
            let d = y - c;
            let core = gauss(d / w) * (0.35 + 0.65 * profile);
            let halo = gauss(d / (w * 3.5 + 0.04)) * 0.32 * (0.3 + 0.7 * profile);
            (core + halo, 0.5)
        }
        Preset::Sonnenaufgang => {
            let (cx, cy) = (aspect * 0.5, 1.52);
            let r = 0.78 * m.size * aspect.clamp(1.0, 1.7) / 1.35;
            let dist = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            let e = dist - r;
            let rim = gauss(e / 0.06);
            let halo = gauss(e.max(0.0) / 0.38) * 0.6;
            let inner = if e < 0.0 { 0.75 * (e / 0.55).exp() } else { 0.0 };
            (rim.max(halo).max(inner), 1.0 - y.clamp(0.0, 1.0))
        }
        Preset::Horizont => {
            let sky = gauss(y / (0.62 * m.size)) * 0.98;
            let (cx, cy) = (aspect * 0.5, 1.95);
            let r = 1.32 / m.size.max(0.5) * aspect.clamp(1.0, 1.7) / 1.35 + 0.05;
            let dist = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            let mask = smoothstep(r - 0.02, r + 0.22, dist);
            let rim = gauss((dist - r) / 0.045) * 0.5;
            (sky * mask + rim, 1.0 - mask)
        }
    }
}

/// Organic shimmer on top of the analytic shape.
#[inline]
pub fn shimmer(noise: &OpenSimplex, x: f32, y: f32, m: &Motion) -> f32 {
    let v = noise.get([(x * 1.7) as f64, (y * 1.7) as f64, (m.t * 0.045) as f64]) as f32;
    v * (0.10 + 0.08 * m.energy)
}
