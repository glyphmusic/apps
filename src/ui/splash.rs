//! Startup animation: points of light stream in from beyond the window edges
//! in a few curved currents and build the Glyph logo (dark plate and pink
//! note) cell by cell in the centre; then the splash lifts off. The logo is
//! never drawn as an image: the icon file only tells each particle where to
//! land and in which colour.
//!
//! Everything is painted with plain quads on one canvas (no text shaping),
//! so it stays cheap even with ~500 particles and their trails.

use gpui::{canvas, div, fill, point, prelude::*, px, size, AnyElement, Bounds, Hsla, Pixels};
use std::sync::Arc;
use std::time::Instant;

const ICON_PNG: &[u8] = include_bytes!("../../assets/icons/icon-256.png");
/// Sampling grid over the icon (cells per side).
const GRID: u32 = 40;
/// Number of currents the particles arrive in.
const STREAMS: u32 = 6;
/// Timeline (seconds).
/// When the plate cells have landed they close up into solid tiles.
const T_SOLID: f32 = 1.9;
const T_SOLID_DUR: f32 = 0.3;
const T_OUT_START: f32 = 2.5;
const T_OUT: f32 = 0.45;
pub const TOTAL: f32 = T_OUT_START + T_OUT;

#[derive(Clone, Copy)]
struct Particle {
    /// Target in icon space, -0.5..0.5.
    tx: f32,
    ty: f32,
    /// Direction it comes from (radians) and its sideways offset in the stream.
    angle: f32,
    lane: f32,
    /// Bend of the path (radians), shared by the stream.
    swirl: f32,
    delay: f32,
    dur: f32,
    color: Hsla,
    /// Part of the note (bright, arrives last) or the plate.
    note: bool,
}

pub struct Splash {
    started: Instant,
    particles: Arc<Vec<Particle>>,
    pub skipped: bool,
}

/// Small deterministic hash -> 0..1.
fn rnd(i: u32, salt: u32) -> f32 {
    let mut h = i.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Splash {
    pub fn new() -> Self {
        let img = image::load_from_memory(ICON_PNG).ok().map(|i| i.to_rgba8());
        let mut particles = Vec::new();
        if let Some(img) = &img {
            let small = image::imageops::resize(img, GRID, GRID, image::imageops::FilterType::Triangle);
            let mut i = 0u32;
            for y in 0..GRID {
                for x in 0..GRID {
                    let p = small.get_pixel(x, y).0;
                    let (r, g, b, a) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0);
                    let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    // Transparent corners stay empty, so the rounded plate emerges.
                    if a < 0.5 {
                        continue;
                    }
                    let note = luma > 0.28;
                    let stream = (rnd(i, 1) * STREAMS as f32) as u32 % STREAMS;
                    let base = stream as f32 / STREAMS as f32 * std::f32::consts::TAU + 0.4;
                    let swirl = if stream % 2 == 0 { 0.9 } else { -0.9 } * (0.75 + 0.5 * rnd(stream, 7));
                    // The plate is near black; lift it a touch so it reads on the black splash.
                    let lift = if note { 0.0 } else { 0.07 };
                    let color = Hsla::from(gpui::Rgba { r: r + lift, g: g + lift, b: b + lift * 1.3, a: 1.0 });
                    particles.push(Particle {
                        tx: (x as f32 + 0.5) / GRID as f32 - 0.5,
                        ty: (y as f32 + 0.5) / GRID as f32 - 0.5,
                        angle: base + (rnd(i, 2) - 0.5) * 0.35,
                        lane: rnd(i, 3) - 0.5,
                        swirl,
                        // Plate first, the note streams in on top of it.
                        delay: stream as f32 * 0.06 + rnd(i, 4) * 0.45 + if note { 0.35 } else { 0.0 },
                        dur: 0.8 + rnd(i, 5) * 0.35,
                        color,
                        note,
                    });
                    i += 1;
                }
            }
        }
        Self { started: Instant::now(), particles: Arc::new(particles), skipped: false }
    }

    pub fn active(&self) -> bool {
        !self.skipped && self.started.elapsed().as_secs_f32() < TOTAL
    }

    pub fn element(&self) -> AnyElement {
        let t = self.started.elapsed().as_secs_f32();
        let particles = self.particles.clone();
        let fade_out = 1.0 - smooth((t - T_OUT_START) / T_OUT);
        div()
            .absolute()
            .inset_0()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds: Bounds<Pixels>, _, window, _| {
                        paint(bounds, t, &particles, fade_out, window);
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

fn paint(bounds: Bounds<Pixels>, t: f32, particles: &[Particle], fade_out: f32, window: &mut gpui::Window) {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (cx, cy) = (ox + w / 2.0, oy + h / 2.0);
    window.paint_quad(fill(bounds, gpui::hsla(0.0, 0.0, 0.0, fade_out)));
    if fade_out <= 0.0 {
        return;
    }

    let icon_size = (w.min(h) * 0.26).clamp(130.0, 240.0);
    let cell = icon_size / GRID as f32;
    // Landed cells grow from dots into seamless tiles: the finished logo.
    let solid = smooth((t - T_SOLID) / T_SOLID_DUR);
    let dot_flying = (cell * 0.7).max(1.5);
    let dot_landed = cell * (0.7 + 0.32 * solid);
    let reach = w.max(h) * 0.78;
    // Slight settle of the whole logo once it is complete.
    let settle = 1.0 + 0.03 * (solid * std::f32::consts::PI).sin();
    let dots_alpha = fade_out;

    let at = |p: &Particle, e: f32| -> (f32, f32) {
        let a = p.angle;
        // Start beyond the edge, a little apart across the stream.
        let side = (a + std::f32::consts::FRAC_PI_2).sin_cos();
        let (sx, sy) = (cx + reach * a.cos() + side.1 * p.lane * 120.0, cy + reach * a.sin() + side.0 * p.lane * 120.0);
        // The control point is turned by the stream's swirl: curved currents.
        let ca = a + p.swirl;
        let (kx, ky) = (cx + reach * 0.55 * ca.cos(), cy + reach * 0.55 * ca.sin());
        let (tx, ty) = (cx + p.tx * icon_size * settle, cy + p.ty * icon_size * settle);
        let u = 1.0 - e;
        (u * u * sx + 2.0 * u * e * kx + e * e * tx, u * u * sy + 2.0 * u * e * ky + e * e * ty)
    };

    if dots_alpha > 0.0 {
        for p in particles {
            let raw = ((t - p.delay) / p.dur).clamp(0.0, 1.0);
            if raw <= 0.0 {
                continue;
            }
            let e = ease_in_out(raw);
            // Trail while moving: two fainter copies a bit behind.
            let trail: &[(f32, f32)] = if raw < 1.0 { &[(0.0, 1.0), (0.035, 0.45), (0.07, 0.2)] } else { &[(0.0, 1.0)] };
            for (back, alpha) in trail {
                let (x, y) = at(p, (e - back).max(0.0));
                let s = if raw >= 1.0 { dot_landed * settle } else if *back > 0.0 { dot_flying * 0.75 } else { dot_flying };
                let c = Hsla { a: p.color.a * alpha * dots_alpha, ..p.color };
                window.paint_quad(fill(Bounds { origin: point(px(x - s / 2.0), px(y - s / 2.0)), size: size(px(s), px(s)) }, c));
            }
        }
    }
}
