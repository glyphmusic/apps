//! `GlowView`: the glow field + ASCII overlay as its own GPUI entity.
//!
//! GPUI has no custom-shader hook, so (per the plan's fallback) the field is
//! computed on the CPU at cell resolution:
//!
//! * the glow itself is written into a small BGRA image (one pixel per half
//!   character cell) and drawn stretched over the window; the GPU's linear
//!   sampler turns it into the soft gaussian blobs;
//! * the ASCII layer paints one cached glyph per cell (`window.paint_glyph`
//!   with glyph ids resolved once), coloured with the lightened glow colour.
//!
//! The view re-computes at 30 fps (10 fps while the window is inactive) and
//! only notifies itself, so the rest of the UI is not re-rendered by it.

use super::field::{self, Motion, Preset};
use super::palette::{Palette, PaletteFade, Rgb};
use crate::guard::guard;
use crate::theme;
use glyph_audio::{PlayerController, SPECTRUM_BANDS};
use gpui::{
    canvas, div, fill, font, point, px, size, Bounds, Context, FontId, GlyphId, Hsla,
    IntoElement, ParentElement, Pixels, Render, RenderImage, Size, Styled, Task, TextRun, Window,
};
use noise::OpenSimplex;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Dark → bright.
pub const RAMP: &str = " .-=+xX0S8#@";
const FONT_SIZE: f32 = 10.0;
const CELL_H: f32 = 12.0;
const PRESET_FADE: f32 = 1.5;

struct GlyphCache {
    font_id: FontId,
    ids: Vec<Option<GlyphId>>,
    cell_w: f32,
    baseline: f32,
}

#[derive(Clone, Copy)]
struct Cell {
    ch: u8,
    color: Hsla,
}

pub struct GlowView {
    noise: OpenSimplex,
    t0: Instant,
    last_tick: Instant,
    preset: Preset,
    prev_preset: Preset,
    preset_since: Instant,
    fade: PaletteFade,
    player: Option<Arc<PlayerController>>,
    spectrum_ok: bool,
    playing_target: bool,
    playing: f32,
    /// Smoothed bass / mids / highs.
    bands: [f32; 3],
    /// Short pulse (lyric line change), decays.
    pulse: f32,
    intensity: f32,
    active: bool,
    view_size: Size<Pixels>,
    glyphs: Option<GlyphCache>,
    // Output of the last tick.
    cols: usize,
    rows: usize,
    cell_w: f32,
    image: Option<Arc<RenderImage>>,
    stale: Vec<Arc<RenderImage>>,
    cells: Vec<Cell>,
    dot_colors: Vec<Hsla>,
    dot_level: f32,
    _task: Task<()>,
}

impl GlowView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let task = cx.spawn(async move |this, cx| loop {
            let interval = match this.update(cx, |v, cx| {
                v.tick();
                cx.notify();
                if v.active { 33 } else { 100 }
            }) {
                Ok(ms) => ms,
                Err(_) => break,
            };
            cx.background_executor()
                .timer(Duration::from_millis(interval))
                .await;
        });
        cx.observe_window_activation(window, |v, window, _| {
            v.active = window.is_window_active();
        })
        .detach();
        let now = Instant::now();
        Self {
            noise: OpenSimplex::new(0x61_79_70_68),
            t0: now,
            last_tick: now,
            preset: Preset::Strahl,
            prev_preset: Preset::Strahl,
            preset_since: now - Duration::from_secs(10),
            fade: PaletteFade::default(),
            player: None,
            spectrum_ok: true,
            playing_target: false,
            playing: 0.0,
            bands: [0.0; 3],
            pulse: 0.0,
            intensity: 0.8,
            active: true,
            view_size: size(px(0.0), px(0.0)),
            glyphs: None,
            cols: 0,
            rows: 0,
            cell_w: 6.0,
            image: None,
            stale: Vec::new(),
            cells: Vec::new(),
            dot_colors: Vec::new(),
            dot_level: 0.3,
            _task: task,
        }
    }

    pub fn set_player(&mut self, player: Option<Arc<PlayerController>>) {
        self.player = player;
        self.spectrum_ok = true;
    }
    pub fn set_playing(&mut self, playing: bool) {
        self.playing_target = playing;
    }
    pub fn set_intensity(&mut self, v: f32) {
        self.intensity = v.clamp(0.0, 1.0);
    }
    pub fn set_palette(&mut self, palette: Option<Palette>) {
        self.fade.set(palette);
    }
    pub fn palette(&self) -> Option<Palette> {
        self.fade.target()
    }
    pub fn set_preset(&mut self, preset: Preset) {
        if preset != self.preset {
            self.prev_preset = self.preset;
            self.preset = preset;
            self.preset_since = Instant::now();
        }
    }
    pub fn preset(&self) -> Preset {
        self.preset
    }
    /// Short glow pulse, e.g. on every new lyric line.
    pub fn pulse(&mut self) {
        self.pulse = 1.0;
    }

    fn read_spectrum(&mut self) -> [f32; 3] {
        let Some(player) = self.player.as_ref().filter(|_| self.spectrum_ok) else {
            return [0.0; 3];
        };
        let Some(s) = guard(|| player.spectrum()) else {
            self.spectrum_ok = false;
            return [0.0; 3];
        };
        let avg = |r: std::ops::Range<usize>| {
            let n = r.len() as f32;
            r.map(|i| s[i.min(SPECTRUM_BANDS - 1)]).sum::<f32>() / n
        };
        [avg(0..4), avg(4..16), avg(16..32)]
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f32().min(0.25);
        self.last_tick = now;
        let t = (now - self.t0).as_secs_f32();

        let target = if self.playing_target { 1.0 } else { 0.0 };
        self.playing += (target - self.playing) * (1.0 - (-dt * 1.2).exp());
        let raw = self.read_spectrum();
        for (b, r) in self.bands.iter_mut().zip(raw) {
            let k = if r > *b { 0.55 } else { 0.12 };
            *b += (r - *b) * k;
        }
        self.pulse = (self.pulse - dt * 1.8).max(0.0);

        let w = f32::from(self.view_size.width);
        let h = f32::from(self.view_size.height);
        if w < 8.0 || h < 8.0 {
            return;
        }
        self.cell_w = self.glyphs.as_ref().map(|g| g.cell_w).unwrap_or(6.0);
        let cols = (w / self.cell_w).ceil() as usize;
        let rows = (h / CELL_H).ceil() as usize;
        let (iw, ih) = (cols, rows * 2);
        let aspect = w / h;

        let [bass, mids, highs] = self.bands;
        let mut m = Motion::new(&self.noise, t, self.playing, bass, mids);
        m.gain += self.pulse * 0.25;
        let (pal, blend) = self.fade.current();
        let (cool, warm) = self.preset.ramps(pal.as_ref(), blend);
        let preset_t = (self.preset_since.elapsed().as_secs_f32() / PRESET_FADE).min(1.0);
        let prev = (preset_t < 1.0).then(|| {
            (self.prev_preset, self.prev_preset.ramps(pal.as_ref(), blend))
        });
        let level = 0.55 + 0.45 * self.intensity;

        // --- glow image + per-pixel colour/brightness
        let mut bytes = vec![0u8; iw * ih * 4];
        let mut samples: Vec<(f32, Rgb)> = Vec::with_capacity(iw * ih);
        for py in 0..ih {
            let y = (py as f32 + 0.5) / ih as f32;
            for px_ in 0..iw {
                let x = (px_ as f32 + 0.5) / iw as f32 * aspect;
                let sh = field::shimmer(&self.noise, x, y, &m);
                let eval = |p: Preset, cool: &field::Ramp, warm: &field::Ramp| {
                    let (b, s) = field::sample(p, x, y, aspect, &m);
                    let b = ((b + sh * b.min(1.0) * 1.4) * m.gain).max(0.0);
                    (b, cool.at(b).lerp(warm.at(b), s))
                };
                let (mut b, mut c) = eval(self.preset, &cool, &warm);
                if let Some((pp, (pc, pw))) = &prev {
                    let (b2, c2) = eval(*pp, pc, pw);
                    b = b2 + (b - b2) * preset_t;
                    c = c2.lerp(c, preset_t);
                }
                let shade = c.scale(smoothstep(0.015, 0.75, b) * level * 0.92);
                let o = (py * iw + px_) * 4;
                let [r, g, bl] = to_u8(shade);
                bytes[o] = bl;
                bytes[o + 1] = g;
                bytes[o + 2] = r;
                bytes[o + 3] = 255;
                samples.push((b, c));
            }
        }
        if let Some(buf) = image::RgbaImage::from_raw(iw as u32, ih as u32, bytes) {
            let img = Arc::new(RenderImage::new(vec![image::Frame::new(buf)]));
            if let Some(old) = self.image.replace(img) {
                self.stale.push(old);
            }
        }

        // --- ASCII cells (two image pixels per cell)
        self.cells.clear();
        self.cells.reserve(cols * rows);
        let density = 0.82 + 0.5 * highs + 0.08 * self.playing;
        let threshold = 0.11 - 0.05 * highs;
        for row in 0..rows {
            for col in 0..cols {
                let a = samples[(row * 2) * iw + col];
                let b = samples[(row * 2 + 1) * iw + col];
                let v = (a.0 + b.0) * 0.5;
                let c = a.1.lerp(b.1, 0.5);
                let dv = v * density;
                if dv < threshold {
                    self.cells.push(Cell { ch: 0, color: Hsla::default() });
                    continue;
                }
                let idx = ((dv.min(1.0)) * (RAMP.len() - 1) as f32).round() as usize;
                let idx = idx.clamp(1, RAMP.len() - 1) as u8;
                let alpha = (0.70 + 0.20 * dv.min(1.0)) * (0.6 + 0.4 * level);
                self.cells.push(Cell { ch: idx, color: c.lighten(0.38).hsla(alpha) });
            }
        }
        self.cols = cols;
        self.rows = rows;

        // --- titlebar dot colours: glow along the top edge, deep-blue floor.
        let dots = (w / 4.0).ceil() as usize;
        self.dot_colors.clear();
        let floor = Rgb::hex(0x0c1a6b);
        for i in 0..dots {
            let x = (i as f32 + 0.5) / dots as f32 * aspect;
            let (b, s) = field::sample(self.preset, x, 0.12, aspect, &m);
            let c = cool.at(0.45 + b * 0.5).lerp(warm.at(0.45 + b * 0.5), s);
            let tint = floor.lerp(c, smoothstep(0.0, 0.6, b).max(blend * 0.55));
            self.dot_colors.push(tint.hsla(1.0));
        }
        self.dot_level = 0.20 + 0.16 * self.intensity + 0.30 * bass + 0.10 * self.pulse;
    }

    fn ensure_glyphs(&mut self, window: &mut Window) {
        if self.glyphs.is_some() {
            return;
        }
        let f = font(theme::FONT);
        let ts = window.text_system().clone();
        let font_id = ts.resolve_font(&f);
        let run = TextRun {
            len: RAMP.len(),
            font: f,
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = ts.shape_line(RAMP.into(), px(FONT_SIZE), &[run], None);
        let mut ids = vec![None; RAMP.len()];
        for r in &line.runs {
            for g in &r.glyphs {
                if g.index < ids.len() && g.index > 0 {
                    ids[g.index] = Some(g.id);
                }
            }
        }
        let cell_w = (f32::from(line.width) / RAMP.len() as f32).max(4.0);
        let asc = f32::from(line.ascent);
        let desc = f32::from(line.descent).abs();
        let baseline = (CELL_H - asc - desc) / 2.0 + asc;
        self.glyphs = Some(GlyphCache { font_id, ids, cell_w, baseline });
    }

    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window) {
        self.ensure_glyphs(window);
        if bounds.size != self.view_size {
            self.view_size = bounds.size;
            self.tick();
        }
        for old in self.stale.drain(..) {
            let _ = window.drop_image(old);
        }
        window.paint_quad(fill(bounds, theme::bg()));
        if let Some(img) = self.image.clone() {
            let _ = window.paint_image(bounds, bounds, Default::default(), img, 0, false);
        }

        // Titlebar dot grid: 2 px dots, 4 px pitch.
        let rows = (theme::TITLEBAR_H / 4.0) as usize;
        for (i, c) in self.dot_colors.iter().enumerate() {
            let x = bounds.origin.x + px(i as f32 * 4.0 + 1.0);
            for r in 0..rows {
                let fadey = 1.0 - (r as f32 / rows as f32) * 0.35;
                let y = bounds.origin.y + px(r as f32 * 4.0 + 1.0);
                window.paint_quad(
                    fill(Bounds::new(point(x, y), size(px(2.0), px(2.0))), c.opacity(self.dot_level * fadey))
                        .corner_radii(px(1.0)),
                );
            }
        }

        let Some(g) = self.glyphs.as_ref() else { return };
        let fs = px(FONT_SIZE);
        let top = theme::TITLEBAR_H;
        for row in 0..self.rows {
            let y = row as f32 * CELL_H;
            if y + CELL_H < top {
                continue; // titlebar shows the dot grid instead
            }
            for col in 0..self.cols {
                let cell = self.cells[row * self.cols + col];
                if cell.ch == 0 {
                    continue;
                }
                let Some(id) = g.ids[cell.ch as usize] else { continue };
                let origin = point(
                    bounds.origin.x + px(col as f32 * self.cell_w),
                    bounds.origin.y + px(y + g.baseline),
                );
                let _ = window.paint_glyph(origin, g.font_id, id, fs, cell.color);
            }
        }
    }
}

impl Render for GlowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        div().size_full().child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    this.update(cx, |v, _| v.paint(bounds, window));
                },
            )
            .size_full(),
        )
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn to_u8(c: Rgb) -> [u8; 3] {
    [
        (c.0.clamp(0.0, 1.0) * 255.0) as u8,
        (c.1.clamp(0.0, 1.0) * 255.0) as u8,
        (c.2.clamp(0.0, 1.0) * 255.0) as u8,
    ]
}
