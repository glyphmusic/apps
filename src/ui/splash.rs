//! Startup animation: points of light stream in from beyond the window edges
//! in a few curved currents, settle into the Glyph note in the centre, then
//! the real icon (note on its dark plate) fades in and the splash lifts off.
//!
//! Everything is painted with plain quads on one canvas (no text shaping),
//! so it stays cheap even with ~500 particles and their trails.

use gpui::{canvas, div, fill, point, prelude::*, px, size, AnyElement, Bounds, Hsla, Pixels, RenderImage};
use std::sync::Arc;
use std::time::Instant;

const ICON_PNG: &[u8] = include_bytes!("../../assets/icons/icon-256.png");
/// Sampling grid over the icon (cells per side).
const GRID: u32 = 46;
/// Number of currents the particles arrive in.
const STREAMS: u32 = 6;
/// Timeline (seconds).
const T_FORMED: f32 = 1.85;
const T_ICON: f32 = 0.35;
const T_OUT_START: f32 = 2.45;
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
}

pub struct Splash {
    started: Instant,
    particles: Arc<Vec<Particle>>,
    icon: Option<Arc<RenderImage>>,
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
                    // Only the bright note forms from particles; the plate fades in later.
                    if a < 0.5 || luma < 0.28 {
                        continue;
                    }
                    let stream = (rnd(i, 1) * STREAMS as f32) as u32 % STREAMS;
                    let base = stream as f32 / STREAMS as f32 * std::f32::consts::TAU + 0.4;
                    let swirl = if stream % 2 == 0 { 0.9 } else { -0.9 } * (0.75 + 0.5 * rnd(stream, 7));
                    let color = Hsla::from(gpui::Rgba { r, g, b, a: 1.0 });
                    particles.push(Particle {
                        tx: (x as f32 + 0.5) / GRID as f32 - 0.5,
                        ty: (y as f32 + 0.5) / GRID as f32 - 0.5,
                        angle: base + (rnd(i, 2) - 0.5) * 0.35,
                        lane: rnd(i, 3) - 0.5,
                        swirl,
                        delay: stream as f32 * 0.07 + rnd(i, 4) * 0.55,
                        dur: 0.85 + rnd(i, 5) * 0.35,
                        color,
                    });
                    i += 1;
                }
            }
        }
        let icon = img.map(|mut i| {
            // RenderImage wants BGRA.
            for px_ in i.pixels_mut() {
                px_.0.swap(0, 2);
            }
            Arc::new(RenderImage::new(vec![image::Frame::new(i)]))
        });
        Self { started: Instant::now(), particles: Arc::new(particles), icon, skipped: false }
    }

    pub fn active(&self) -> bool {
        !self.skipped && self.started.elapsed().as_secs_f32() < TOTAL
    }

    pub fn element(&self) -> AnyElement {
        let t = self.started.elapsed().as_secs_f32();
        let particles = self.particles.clone();
        let icon = self.icon.clone();
        let fade_out = 1.0 - smooth((t - T_OUT_START) / T_OUT);
        div()
            .absolute()
            .inset_0()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds: Bounds<Pixels>, _, window, _| {
                        paint(bounds, t, &particles, icon.as_ref(), fade_out, window);
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

fn paint(bounds: Bounds<Pixels>, t: f32, particles: &[Particle], icon: Option<&Arc<RenderImage>>, fade_out: f32, window: &mut gpui::Window) {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (cx, cy) = (ox + w / 2.0, oy + h / 2.0);
    window.paint_quad(fill(bounds, gpui::hsla(0.0, 0.0, 0.0, fade_out)));
    if fade_out <= 0.0 {
        return;
    }

    let icon_size = (w.min(h) * 0.24).clamp(120.0, 220.0);
    let cell = icon_size / GRID as f32;
    let dot = (cell * 0.82).max(1.5);
    let reach = w.max(h) * 0.78;
    // Particles hand over to the real icon.
    let icon_in = smooth((t - T_FORMED) / T_ICON);
    let dots_alpha = (1.0 - icon_in) * fade_out;

    let at = |p: &Particle, e: f32| -> (f32, f32) {
        let a = p.angle;
        // Start beyond the edge, a little apart across the stream.
        let side = (a + std::f32::consts::FRAC_PI_2).sin_cos();
        let (sx, sy) = (cx + reach * a.cos() + side.1 * p.lane * 120.0, cy + reach * a.sin() + side.0 * p.lane * 120.0);
        // The control point is turned by the stream's swirl: curved currents.
        let ca = a + p.swirl;
        let (kx, ky) = (cx + reach * 0.55 * ca.cos(), cy + reach * 0.55 * ca.sin());
        let (tx, ty) = (cx + p.tx * icon_size, cy + p.ty * icon_size);
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
                let s = if *back > 0.0 { dot * 0.75 } else { dot };
                let c = Hsla { a: p.color.a * alpha * dots_alpha, ..p.color };
                window.paint_quad(fill(Bounds { origin: point(px(x - s / 2.0), px(y - s / 2.0)), size: size(px(s), px(s)) }, c));
            }
        }
    }

    if let Some(img) = icon {
        if icon_in > 0.0 {
            // Small settle: 94% -> 100% as it appears.
            let s = icon_size * (0.94 + 0.06 * icon_in);
            let rect = Bounds { origin: point(px(cx - s / 2.0), px(cy - s / 2.0)), size: size(px(s), px(s)) };
            let a = icon_in * fade_out;
            window.paint_layer(rect, |window| {
                // GPUI has no image alpha; draw it and veil it with black while it fades in.
                let _ = window.paint_image(rect, rect, gpui::Corners::all(px(s * 0.22)), img.clone(), 0, false);
                if a < 1.0 {
                    window.paint_quad(gpui::quad(
                        rect,
                        gpui::Corners::all(px(s * 0.22)),
                        gpui::hsla(0.0, 0.0, 0.0, 1.0 - a),
                        gpui::Edges::default(),
                        gpui::transparent_black(),
                        gpui::BorderStyle::default(),
                    ));
                }
            });
        }
    }
}
