//! Startup animation: ASCII characters stream in from beyond the window edges
//! in a few curved currents and assemble the Glyph note (the same character
//! raster as the app icon, without its plate) in the centre; then the splash
//! lifts off. Nothing is drawn as an image: every character is real text.

use gpui::{canvas, div, point, prelude::*, px, AnyElement, App, Bounds, Hsla, Pixels, SharedString, TextAlign, TextRun};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

// The icon's character raster (read off icon-1024.png: 31 columns x 29 rows).
const ART: [&str; 29] = [
    "                         ..+.. ",
    "                       ----#+ .",
    "                  ...-++-++++. ",
    "              ..-----+####+0.  ",
    "           .--.-++++##++++++.  ",
    "         +++++++++++---.. ++.  ",
    "        .+++++++.+....   .#+.  ",
    "        +0...+.++.        ++.  ",
    "        +++...            .+.  ",
    "        .+.               ++.  ",
    "        .+.               .0.  ",
    "        .+.               .+.  ",
    "        .+.               .+.  ",
    "        .+.               .+.  ",
    "        .+.               .+.  ",
    "        .+.               ++.  ",
    "        .+.           ... .+.  ",
    "        .+.         ..++++.+.  ",
    "        .+.        .+.++++0..  ",
    "        .+.       ...-+#+++0+  ",
    "  -++++ .+.       +.+####++++  ",
    " .+.+++#.+.       ..++.0++++.  ",
    ".+++###+0+.        +..+.+++.   ",
    ".+.+###++0+         ....+..    ",
    "..+###+++++           ...      ",
    "..+..0++++.                    ",
    " ....++++.                     ",
    "  ....+..                      ",
    "    ...                        ",
];
/// `p` = pink (highlight), `w` = white.
const TINT: [&str; 29] = [
    "                         wwwww ",
    "                       wwwwpp w",
    "                  wwwwwwwpwwww ",
    "              wwwwwwwpppppwww  ",
    "           wwwwwwwpppppwwwwww  ",
    "         wwwpwppppppwwwww www  ",
    "        wwwpppwwwwwwww   wwww  ",
    "        wwwwwwwwww        wpw  ",
    "        wwwwww            www  ",
    "        www               www  ",
    "        www               www  ",
    "        www               www  ",
    "        wpw               wpw  ",
    "        www               www  ",
    "        www               www  ",
    "        wpw               wpw  ",
    "        www           www www  ",
    "        www         wwwwwwwww  ",
    "        wpw        wwwpppwwww  ",
    "        wpw       wwwwpppwwww  ",
    "  wwwww www       wwwwpppppww  ",
    " wwwwppwwww       wwwwwwwppww  ",
    "wwwwpppwwww        wwwwwwpww   ",
    "wwwppppwwww         wwwwwww    ",
    "wwwwpppppww           www      ",
    "wwwwwwppwww                    ",
    " wwwwwwwww                     ",
    "  wwwwwww                      ",
    "    www                        ",
];
/// Character cell aspect of the icon raster (height / width).
const CELL_ASPECT: f32 = 25.2 / 19.85;
/// Number of currents the characters arrive in.
const STREAMS: u32 = 6;
/// Timeline (seconds).
const T_OUT_START: f32 = 2.45;
const T_OUT: f32 = 0.45;
pub const TOTAL: f32 = T_OUT_START + T_OUT;

#[derive(Clone, Copy)]
struct Particle {
    ch: char,
    pink: bool,
    /// Target cell (column, row).
    col: f32,
    row: f32,
    /// Direction it comes from (radians) and its sideways offset in the stream.
    angle: f32,
    lane: f32,
    /// Bend of the path (radians), shared by the stream.
    swirl: f32,
    delay: f32,
    dur: f32,
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
        let mut particles = Vec::new();
        let mut i = 0u32;
        for (row, (line, tint)) in ART.iter().zip(TINT.iter()).enumerate() {
            for (col, (ch, t)) in line.chars().zip(tint.chars()).enumerate() {
                if ch == ' ' {
                    continue;
                }
                let stream = (rnd(i, 1) * STREAMS as f32) as u32 % STREAMS;
                let base = stream as f32 / STREAMS as f32 * std::f32::consts::TAU + 0.4;
                let swirl = if stream % 2 == 0 { 0.9 } else { -0.9 } * (0.75 + 0.5 * rnd(stream, 7));
                particles.push(Particle {
                    ch,
                    pink: t == 'p',
                    col: col as f32,
                    row: row as f32,
                    angle: base + (rnd(i, 2) - 0.5) * 0.35,
                    lane: rnd(i, 3) - 0.5,
                    swirl,
                    delay: stream as f32 * 0.07 + rnd(i, 4) * 0.6,
                    dur: 0.85 + rnd(i, 5) * 0.35,
                });
                i += 1;
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
                    move |bounds: Bounds<Pixels>, _, window, cx| {
                        paint(bounds, t, &particles, fade_out, window, cx);
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

fn paint(bounds: Bounds<Pixels>, t: f32, particles: &[Particle], fade_out: f32, window: &mut gpui::Window, cx: &mut App) {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let (cx_, cy_) = (ox + w / 2.0, oy + h / 2.0);
    window.paint_quad(gpui::fill(bounds, gpui::hsla(0.0, 0.0, 0.0, fade_out)));
    if fade_out <= 0.0 {
        return;
    }

    let cols = ART[0].len() as f32;
    let rows = ART.len() as f32;
    // The note is about a third of the smaller window side tall.
    let row_h = ((h.min(w) * 0.26) / rows).clamp(5.0, 11.0);
    // Same proportions as the icon raster; the logo is kept small.
    let cell_w = row_h / CELL_ASPECT;
    let cell_h = row_h;
    let font_size = px(row_h * 0.95);
    let (left, top) = (cx_ - cols * cell_w / 2.0, cy_ - rows * cell_h / 2.0);
    let reach = w.max(h) * 0.78;
    let font = gpui::Font { weight: gpui::FontWeight::BOLD, ..gpui::font(crate::theme::MONO) };
    let white = gpui::hsla(0.0, 0.0, 0.96, 1.0);
    let pink = crate::theme::pink();

    // One shaped glyph per (character, colour, alpha step), reused for every copy.
    let mut shaped: HashMap<(char, bool, u8), gpui::ShapedLine> = HashMap::new();
    let mut glyph = |ch: char, is_pink: bool, alpha: f32, window: &mut gpui::Window| -> gpui::ShapedLine {
        let step = (alpha.clamp(0.0, 1.0) * 12.0).round() as u8;
        shaped
            .entry((ch, is_pink, step))
            .or_insert_with(|| {
                let base: Hsla = if is_pink { pink } else { white };
                let s: SharedString = ch.to_string().into();
                window.text_system().shape_line(
                    s,
                    font_size,
                    &[TextRun {
                        len: ch.len_utf8(),
                        font: font.clone(),
                        color: Hsla { a: step as f32 / 12.0, ..base },
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
            })
            .clone()
    };

    let at = |p: &Particle, e: f32| -> (f32, f32) {
        let a = p.angle;
        let side = (a + std::f32::consts::FRAC_PI_2).sin_cos();
        let (sx, sy) = (cx_ + reach * a.cos() + side.1 * p.lane * 120.0, cy_ + reach * a.sin() + side.0 * p.lane * 120.0);
        // The control point is turned by the stream's swirl: curved currents.
        let ca = a + p.swirl;
        let (kx, ky) = (cx_ + reach * 0.55 * ca.cos(), cy_ + reach * 0.55 * ca.sin());
        let (tx, ty) = (left + p.col * cell_w, top + p.row * cell_h);
        let u = 1.0 - e;
        (u * u * sx + 2.0 * u * e * kx + e * e * tx, u * u * sy + 2.0 * u * e * ky + e * e * ty)
    };

    for p in particles {
        let raw = ((t - p.delay) / p.dur).clamp(0.0, 1.0);
        if raw <= 0.0 {
            continue;
        }
        let e = ease_in_out(raw);
        // Trail while moving: two fainter copies a little behind.
        let trail: &[(f32, f32)] = if raw < 1.0 { &[(0.07, 0.18), (0.035, 0.4), (0.0, 1.0)] } else { &[(0.0, 1.0)] };
        for (back, alpha) in trail {
            let (x, y) = at(p, (e - back).max(0.0));
            let line = glyph(p.ch, p.pink, alpha * fade_out, window);
            let _ = line.paint(point(px(x), px(y)), px(cell_h), TextAlign::Left, None, window, cx);
        }
    }
}

// ---- the note as a live logo (Settings -> About) ---------------------------
//
// The same characters assemble into the note when the page opens (like the
// splash, inside a small box), then react to the mouse like the landing page:
// characters near the cursor are pushed out of a bubble, a spring pulls them
// home and they push each other apart; now and then one flickers in the accent.

struct Logo {
    born: Instant,
    seen: Instant,
    /// Per character: offset from its home and velocity.
    off: Vec<[f32; 4]>,
    /// Per character: flicker until (seconds since born) and the stand-in.
    glitch: Vec<(f32, char)>,
    seed: u32,
}

thread_local! {
    static LOGO: std::cell::RefCell<Option<Logo>> = const { std::cell::RefCell::new(None) };
    static LOGO_PARTS: Arc<Vec<Particle>> = Splash::new().particles;
}

/// The note logo, `row_h` px per character row.
pub fn note_logo(row_h: f32) -> impl IntoElement {
    let cols = ART[0].len() as f32;
    let rows = ART.len() as f32;
    let cell_w = row_h / CELL_ASPECT;
    let pad = row_h * 2.0;
    canvas(|_, _, _| (), move |bounds: Bounds<Pixels>, _, window, cx| paint_logo(bounds, row_h, window, cx))
        .w(px(cols * cell_w + pad * 2.0))
        .h(px(rows * row_h + pad * 2.0))
}

fn paint_logo(bounds: Bounds<Pixels>, row_h: f32, window: &mut gpui::Window, cx: &mut App) {
    let parts = LOGO_PARTS.with(|p| p.clone());
    let settled = parts.iter().map(|p| p.delay + p.dur).fold(0.0f32, f32::max);
    let still = cx.reduce_motion();
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (cx_, cy_) = (f32::from(bounds.origin.x) + w / 2.0, f32::from(bounds.origin.y) + h / 2.0);
    let cols = ART[0].len() as f32;
    let rows = ART.len() as f32;
    let cell_h = row_h;
    let cell_w = row_h / CELL_ASPECT;
    let (left, top) = (cx_ - cols * cell_w / 2.0, cy_ - rows * cell_h / 2.0);
    let reach = w.max(h) * 0.9;
    let at = |p: &Particle, e: f32| -> (f32, f32) {
        let a = p.angle;
        let side = (a + std::f32::consts::FRAC_PI_2).sin_cos();
        let (sx, sy) = (cx_ + reach * a.cos() + side.1 * p.lane * 40.0, cy_ + reach * a.sin() + side.0 * p.lane * 40.0);
        let ca = a + p.swirl;
        let (kx, ky) = (cx_ + reach * 0.55 * ca.cos(), cy_ + reach * 0.55 * ca.sin());
        let (tx, ty) = (left + p.col * cell_w, top + p.row * cell_h);
        let u = 1.0 - e;
        (u * u * sx + 2.0 * u * e * kx + e * e * tx, u * u * sy + 2.0 * u * e * ky + e * e * ty)
    };
    let mouse = window.mouse_position();
    let mouse = bounds.contains(&mouse).then(|| (f32::from(mouse.x), f32::from(mouse.y)));

    LOGO.with(|l| {
        let mut l = l.borrow_mut();
        // Shown already assembled (no fly-in); it only reacts to the mouse.
        if l.as_ref().is_none_or(|s| s.seen.elapsed().as_secs_f32() > 1.0 || s.off.len() != parts.len()) {
            let now = Instant::now();
            *l = Some(Logo {
                born: now.checked_sub(std::time::Duration::from_secs_f32(settled)).unwrap_or(now),
                seen: Instant::now(),
                off: vec![[0.0; 4]; parts.len()],
                glitch: vec![(0.0, ' '); parts.len()],
                seed: 1,
            });
        }
        let Some(s) = l.as_mut() else { return };
        s.seen = Instant::now();
        let t = if still { settled } else { s.born.elapsed().as_secs_f32() };

        if !still {
            // Flicker: now and then one character turns into another for a moment.
            s.seed = s.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if t > settled && (s.seed >> 8) % 100 < 18 {
                const FLICKER: [char; 5] = ['.', '-', '+', '#', '0'];
                let i = (s.seed >> 3) as usize % parts.len();
                s.glitch[i] = (t + 0.12 + rnd(s.seed, 9) * 0.2, FLICKER[(s.seed >> 13) as usize % FLICKER.len()]);
            }
            // Spring home, cursor bubble, characters pushing each other apart.
            let bubble = row_h * 4.5;
            let gap = cell_w * 0.95;
            let pos: Vec<(f32, f32)> = parts
                .iter()
                .zip(&s.off)
                .map(|(p, o)| {
                    let e = ease_in_out(((t - p.delay) / p.dur).clamp(0.0, 1.0));
                    let (x, y) = at(p, e);
                    (x + o[0] + cell_w / 2.0, y + o[1] + cell_h / 2.0)
                })
                .collect();
            for (i, o) in s.off.iter_mut().enumerate() {
                o[2] -= o[0] * 0.06;
                o[3] -= o[1] * 0.06;
                let (x, y) = pos[i];
                if let Some((mx, my)) = mouse {
                    let (dx, dy) = (x - mx, y - my);
                    let d = (dx * dx + dy * dy).sqrt().max(0.01);
                    if d < bubble {
                        let f = (bubble - d) / bubble * 2.2;
                        o[2] += dx / d * f;
                        o[3] += dy / d * f;
                    }
                }
                if t >= settled && (o[0].abs() > 0.05 || o[1].abs() > 0.05 || mouse.is_some()) {
                    for (j, &(qx, qy)) in pos.iter().enumerate() {
                        let (dx, dy) = (x - qx, y - qy);
                        if j == i || dx.abs() >= gap || dy.abs() >= gap {
                            continue;
                        }
                        let d = (dx * dx + dy * dy).sqrt();
                        if d < gap && d > 0.001 {
                            let f = (gap - d) / gap * 0.5;
                            o[2] += dx / d * f;
                            o[3] += dy / d * f;
                        }
                    }
                }
            }
            for o in s.off.iter_mut() {
                o[2] *= 0.78;
                o[3] *= 0.78;
                o[0] += o[2];
                o[1] += o[3];
            }
            window.request_animation_frame();
        }

        // Draw: shaped glyphs per (character, colour, alpha step), reused.
        let font_size = px(row_h * 0.95);
        let font = gpui::Font { weight: gpui::FontWeight::BOLD, ..gpui::font(crate::theme::MONO) };
        let white = gpui::hsla(0.0, 0.0, 0.96, 1.0);
        let pink = crate::theme::pink();
        let mut shaped: HashMap<(char, bool, u8), gpui::ShapedLine> = HashMap::new();
        for (i, p) in parts.iter().enumerate() {
            let raw = ((t - p.delay) / p.dur).clamp(0.0, 1.0);
            if raw <= 0.0 {
                continue;
            }
            let e = ease_in_out(raw);
            let glitching = s.glitch[i].0 > t;
            let ch = if glitching { s.glitch[i].1 } else { p.ch };
            let is_pink = p.pink || glitching;
            let trail: &[(f32, f32)] = if raw < 1.0 { &[(0.07, 0.18), (0.035, 0.4), (0.0, 1.0)] } else { &[(0.0, 1.0)] };
            for (back, alpha) in trail {
                let (x, y) = at(p, (e - back).max(0.0));
                let step = (alpha.clamp(0.0, 1.0) * 12.0).round() as u8;
                let line = shaped
                    .entry((ch, is_pink, step))
                    .or_insert_with(|| {
                        let base: Hsla = if is_pink { pink } else { white };
                        window.text_system().shape_line(
                            SharedString::from(ch.to_string()),
                            font_size,
                            &[TextRun {
                                len: ch.len_utf8(),
                                font: font.clone(),
                                color: Hsla { a: step as f32 / 12.0, ..base },
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }],
                            None,
                        )
                    })
                    .clone();
                let _ = line.paint(point(px(x + s.off[i][0]), px(y + s.off[i][1])), px(cell_h), TextAlign::Left, None, window, cx);
            }
        }
    });
}
