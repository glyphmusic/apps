//! `GlowView`: the glow field + ASCII overlay as its own GPUI entity.
//!
//! Everything is rasterised on a worker thread; the UI thread only uploads
//! two finished images per frame and draws them:
//!
//! * the glow: a small BGRA image (one pixel per 8×8 logical px) stretched over
//!   the window — the GPU's linear sampler turns it into soft blobs;
//! * the ASCII layer + titlebar dot grid: one full-resolution BGRA image,
//!   drawn from a tiny pre-rasterised 5×7 bitmap font of the ramp characters
//!   (no text shaping, no per-glyph draw calls). Refreshed at 15 fps.
//!
//! The field runs at 30 fps, 10 fps while the window is inactive, and pauses
//! while the window is being moved or resized. The worker also reports the
//! displayed luminance per UI zone so the text colour can adapt.

use super::field::{self, Motion, Preset, Ramp};
use super::palette::{Palette, PaletteFade, Rgb};
use crate::guard::guard;
use crate::theme;
use glyph_audio::{PlayerController, SPECTRUM_BANDS};
use gpui::{
    canvas, div, fill, px, size, Bounds, Context, IntoElement, ParentElement, Pixels, Render,
    RenderImage, Size, Styled, Task, Window,
};
use noise::OpenSimplex;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Dark → bright.
pub const RAMP: &str = " .-=+xX0S8#@";
const PRESET_FADE: f32 = 1.5;
/// Glow sample pitch (logical px).
const GLOW_PITCH: f32 = 8.0;
/// ASCII cell (logical px). Sparse on purpose: the grid is texture, not content.
const CELL_W: f32 = 10.0;
const CELL_H: f32 = 16.0;
/// How far the ASCII layer follows the page when the main view scrolls
/// (0..1). 0: it stays put, which reads calmer than any half-step motion.
const PARALLAX: f32 = 0.0;

/// Text zones whose background luminance is reported.
#[derive(Debug, Clone, Copy)]
pub enum Zone {
    Titlebar = 0,
    Sidebar = 1,
    Main = 2,
}
pub const ZONES: usize = 3;

// 5×7 bitmap font for the ramp (rows top→bottom, 5 bits each, MSB = left).
pub(crate) const FONT: [[u8; 7]; 12] = [
    [0, 0, 0, 0, 0, 0, 0],                                         // ' '
    [0, 0, 0, 0, 0, 0b00100, 0],                                   // .
    [0, 0, 0, 0b01110, 0, 0, 0],                                   // -
    [0, 0, 0b11111, 0, 0b11111, 0, 0],                             // =
    [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],           // +
    [0, 0, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001],           // x
    [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001], // X
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110], // 0
    [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110], // S
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110], // 8
    [0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010], // #
    [0b01110, 0b10001, 0b10111, 0b10101, 0b10111, 0b10000, 0b01111], // @
];

/// Everything the worker needs for one frame (all `Copy`).
#[derive(Clone, Copy)]
struct Job {
    w: f32,
    h: f32,
    scale: f32,
    motion: Motion,
    preset: Preset,
    ramps: (Ramp, Ramp),
    prev: Option<(Preset, (Ramp, Ramp), f32)>,
    level: f32,
    density: f32,
    threshold: f32,
    ascii: bool,
    dot_level: f32,
    blend: f32,
    /// Main view scroll (logical px, >= 0 when scrolled down).
    scroll: f32,
}

struct FrameOut {
    glow: image::RgbaImage,
    ascii: Option<image::RgbaImage>,
    zones: [f32; ZONES],
    lum: Arc<theme::LumGrid>,
    ms: f32,
    /// Scroll the frame was drawn for (paint shifts by the difference).
    scroll: f32,
}

struct Worker {
    noise: OpenSimplex,
    samples: Vec<(f32, Rgb)>,
    ascii: Vec<u8>,
    lum: Vec<f32>,
}

impl Worker {
    fn run(&mut self, j: &Job) -> FrameOut {
        let t0 = Instant::now();
        let aspect = j.w / j.h;
        let iw = ((j.w / GLOW_PITCH).ceil() as usize).max(2);
        let ih = ((j.h / GLOW_PITCH).ceil() as usize).max(2);
        let m = &j.motion;
        let (cool, warm) = j.ramps;
        // Only the ASCII layer follows the page (parallax); the glow stays put
        // so the titlebar and window edges never shift.
        let shift = j.scroll * PARALLAX;
        let noise = &self.noise;
        // Field value + colour at normalised (x, y).
        let field_at = |x: f32, y: f32| -> (f32, Rgb) {
            let sh = field::shimmer(noise, x, y, m);
            let eval = |p: Preset, cool: &Ramp, warm: &Ramp| {
                let (b, s) = field::sample(p, x, y, aspect, m);
                let b = ((b + sh * b.min(1.0) * 1.4) * m.gain).max(0.0);
                (b, cool.at(b).lerp(warm.at(b), s))
            };
            let (mut b, mut c) = eval(j.preset, &cool, &warm);
            if let Some((pp, (pc, pw), pt)) = &j.prev {
                let (b2, c2) = eval(*pp, pc, pw);
                b = b2 + (b - b2) * pt;
                c = c2.lerp(c, *pt);
            }
            (b, c)
        };

        // --- glow (low-res)
        let mut glow = vec![0u8; iw * ih * 4];
        self.samples.clear();
        self.lum.clear();
        for py in 0..ih {
            let y = (py as f32 + 0.5) / ih as f32;
            for px_ in 0..iw {
                let x = (px_ as f32 + 0.5) / iw as f32 * aspect;
                let (b, c) = field_at(x, y);
                // Softer and darker than the raw field: it is a background.
                let shade = c.scale(smoothstep(0.02, 1.1, b) * j.level * 0.5);
                let o = (py * iw + px_) * 4;
                glow[o] = u8c(shade.2);
                glow[o + 1] = u8c(shade.1);
                glow[o + 2] = u8c(shade.0);
                glow[o + 3] = 255;
                self.samples.push((b, c));
                self.lum.push(shade.luma());
            }
        }
        let glow = image::RgbaImage::from_raw(iw as u32, ih as u32, glow).expect("glow size");

        // --- per-zone luminance (90th percentile of displayed glow + ASCII share)
        let zone_rect = |z: Zone| -> (f32, f32, f32, f32) {
            let top = theme::TITLEBAR_H;
            match z {
                Zone::Titlebar => (0.0, 0.0, j.w, top),
                Zone::Sidebar => (0.0, top, theme::SIDEBAR_W + theme::GAP * 2.0, j.h - theme::PLAYER_H - 20.0),
                Zone::Main => (theme::SIDEBAR_W + theme::GAP, top, j.w * 0.62, j.h - theme::PLAYER_H - 20.0),
            }
        };
        let mut zones = [0f32; ZONES];
        for (zi, z) in [Zone::Titlebar, Zone::Sidebar, Zone::Main].into_iter().enumerate() {
            let (x0, y0, x1, y1) = zone_rect(z);
            let (cx0, cy0) = ((x0 / GLOW_PITCH) as usize, (y0 / GLOW_PITCH) as usize);
            let (cx1, cy1) = (((x1 / GLOW_PITCH) as usize).min(iw), ((y1 / GLOW_PITCH) as usize).min(ih));
            let mut v: Vec<f32> = Vec::new();
            for y in cy0..cy1 {
                for x in cx0..cx1 {
                    v.push(self.lum[y * iw + x] * 1.12);
                }
            }
            if !v.is_empty() {
                let k = ((v.len() as f32 * 0.9) as usize).min(v.len() - 1);
                v.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
                zones[zi] = v[k];
            }
        }

        // --- ASCII layer + titlebar dots (full resolution, straight-alpha BGRA)
        let ascii = j.ascii.then(|| {
            let sw = (j.w * j.scale).round().max(1.0) as usize;
            let sh_ = (j.h * j.scale).round().max(1.0) as usize;
            self.ascii.clear();
            self.ascii.resize(sw * sh_ * 4, 0);
            let buf = &mut self.ascii;
            let p = j.scale.round().max(1.0) as usize; // font pixel size
            let cols = (j.w / CELL_W) as usize;
            let rows = (j.h / CELL_H) as usize + 1;
            // Characters move with the page too (sub-cell offset), so the art
            // scrolls instead of flickering in place.
            let sub = shift.rem_euclid(CELL_H);
            let sample = |lx: f32, ly: f32| -> (f32, Rgb) {
                let gx = ((lx / GLOW_PITCH) as usize).min(iw - 1);
                let gy = ((ly / GLOW_PITCH) as usize).min(ih - 1);
                self.samples[gy * iw + gx]
            };
            for row in 0..rows {
                let ly = row as f32 * CELL_H - sub;
                if ly < theme::TITLEBAR_H || ly + CELL_H > j.h {
                    continue;
                }
                for col in 0..cols {
                    let lx = col as f32 * CELL_W;
                    // The rows slide with the page, but each character takes its
                    // brightness from the glow right behind it, so the art never
                    // drifts away from the light (or out of the window).
                    let (b, c) = sample(lx + CELL_W * 0.5, ly + CELL_H * 0.5);
                    let dv = b * j.density;
                    if dv < j.threshold {
                        continue;
                    }
                    let idx = ((dv.min(1.0) * (RAMP.len() - 1) as f32).round() as usize).clamp(1, RAMP.len() - 1);
                    let alpha = (0.16 + 0.16 * dv.min(1.0)) * j.level;
                    let col_ = c.lighten(0.3);
                    let (r, g, bl) = (u8c(col_.0), u8c(col_.1), u8c(col_.2));
                    let a = u8c(alpha);
                    let ox = ((lx + (CELL_W - 5.0) * 0.5) * j.scale) as usize;
                    let oy = ((ly + (CELL_H - 7.0) * 0.5) * j.scale) as usize;
                    for (ry, bits) in FONT[idx].iter().enumerate() {
                        if *bits == 0 {
                            continue;
                        }
                        for rx in 0..5 {
                            if bits & (0b10000 >> rx) == 0 {
                                continue;
                            }
                            for dy in 0..p {
                                let y = oy + ry * p + dy;
                                if y >= sh_ {
                                    continue;
                                }
                                let base = (y * sw + ox + rx * p) * 4;
                                for dx in 0..p {
                                    let o = base + dx * 4;
                                    if o + 3 < buf.len() {
                                        buf[o] = bl;
                                        buf[o + 1] = g;
                                        buf[o + 2] = r;
                                        buf[o + 3] = a;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Titlebar dot grid: 2 px dots, 4 px pitch, glow colour along the top.
            let floor = Rgb::hex(0x0c1a6b);
            let pitch = 4.0;
            let dots_x = (j.w / pitch) as usize;
            let dots_y = (theme::TITLEBAR_H / pitch) as usize;
            let d = (2.0 * j.scale).round().max(1.0) as usize;
            for i in 0..dots_x {
                let lx = i as f32 * pitch + 1.0;
                let (b, c) = sample(lx, 6.0);
                let tint = floor.lerp(c, smoothstep(0.0, 0.6, b).max(j.blend * 0.55));
                let (r, g, bl) = (u8c(tint.0), u8c(tint.1), u8c(tint.2));
                for k in 0..dots_y {
                    let fade = 1.0 - (k as f32 / dots_y as f32) * 0.35;
                    let a = u8c(j.dot_level * fade);
                    let ox = (lx * j.scale) as usize;
                    let oy = ((k as f32 * pitch + 1.0) * j.scale) as usize;
                    for dy in 0..d {
                        for dx in 0..d {
                            let o = ((oy + dy) * sw + ox + dx) * 4;
                            if o + 3 < buf.len() {
                                buf[o] = bl;
                                buf[o + 1] = g;
                                buf[o + 2] = r;
                                buf[o + 3] = a;
                            }
                        }
                    }
                }
            }
            image::RgbaImage::from_raw(sw as u32, sh_ as u32, buf.clone()).expect("ascii size")
        });

        // Displayed luminance (glow + the ASCII layer's share) for per-element ink.
        let lum = Arc::new(theme::LumGrid {
            w: iw,
            h: ih,
            pitch: GLOW_PITCH,
            data: self.lum.iter().map(|l| l * 1.12).collect(),
        });
        FrameOut { glow, ascii, zones, lum, ms: t0.elapsed().as_secs_f32() * 1000.0, scroll: j.scroll }
    }
}

pub struct GlowView {
    noise: OpenSimplex,
    last_tick: Instant,
    preset: Preset,
    prev_preset: Preset,
    /// Colour theme glow colours, the previous ones and since when (fade).
    theme: Option<Palette>,
    prev_theme: Option<Palette>,
    theme_since: Instant,
    preset_since: Instant,
    fade: PaletteFade,
    player: Option<Arc<PlayerController>>,
    spectrum_ok: bool,
    playing_target: bool,
    playing: f32,
    bands: [f32; 3],
    pulse: f32,
    intensity: f32,
    music_reactivity: f32,
    /// Field clock (s). Stands still while the intensity is 0.
    anim_t: f32,
    /// How much the field moves (1 = normal, 0 = still at intensity 0).
    motion_k: f32,
    /// Size + scroll of the last frame drawn while still: nothing new to
    /// compute until one of them changes.
    still_drawn: Option<(f32, f32, f32)>,
    active: bool,
    /// Last size change (resize in progress → hold the frame).
    moved_at: Instant,
    window_size: Size<Pixels>,
    window_origin: gpui::Point<Pixels>,
    view_size: Size<Pixels>,
    scale: f32,
    frame_no: u64,
    jobs: mpsc::SyncSender<Job>,
    results: mpsc::Receiver<FrameOut>,
    busy: bool,
    glow_img: Option<Arc<RenderImage>>,
    ascii_img: Option<Arc<RenderImage>>,
    stale: Vec<Arc<RenderImage>>,
    zones: [f32; ZONES],
    lum_grid: Option<Arc<theme::LumGrid>>,
    scroll: f32,
    ascii_scroll: f32,
    /// Scroll the displayed ASCII image was drawn for.
    ascii_drawn_at: f32,
    stats: (f32, f32, u32, Instant),
    _task: Task<()>,
}

impl GlowView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (job_tx, job_rx) = mpsc::sync_channel::<Job>(1);
        let (out_tx, out_rx) = mpsc::sync_channel::<FrameOut>(1);
        std::thread::Builder::new()
            .name("glyph-glow".into())
            .spawn(move || {
                let mut w = Worker {
                    noise: OpenSimplex::new(0x61_79_70_68),
                    samples: Vec::new(),
                    ascii: Vec::new(),
                    lum: Vec::new(),
                };
                while let Ok(job) = job_rx.recv() {
                    if out_tx.send(w.run(&job)).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn glow worker");

        let task = cx.spawn(async move |this, cx| loop {
            let interval = match this.update(cx, |v, cx| {
                if v.tick() {
                    cx.notify();
                }
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
        // Hold the last frame only while the window is being *resized* (the
        // image would have to be recomputed for every size). A plain move
        // changes nothing in the picture, so the field keeps running; pausing
        // it there made the first frames after the drag hitch.
        cx.observe_window_bounds(window, |v, window, _| {
            let b = window.bounds();
            if b.size != v.window_size || b.origin != v.window_origin {
                v.window_size = b.size;
                v.window_origin = b.origin;
                v.moved_at = Instant::now();
            }
        })
        .detach();
        let now = Instant::now();
        Self {
            noise: OpenSimplex::new(0x61_79_70_68),
            last_tick: now,
            preset: Preset::Strahl,
            prev_preset: Preset::Strahl,
            theme: None,
            prev_theme: None,
            theme_since: Instant::now() - Duration::from_secs(10),
            preset_since: now - Duration::from_secs(10),
            fade: PaletteFade::default(),
            player: None,
            spectrum_ok: true,
            playing_target: false,
            playing: 0.0,
            bands: [0.0; 3],
            pulse: 0.0,
            intensity: 0.8,
            music_reactivity: 1.0,
            anim_t: 0.0,
            motion_k: 1.0,
            still_drawn: None,
            active: true,
            moved_at: now - Duration::from_secs(10),
            window_size: window.bounds().size,
            window_origin: window.bounds().origin,
            view_size: size(px(0.0), px(0.0)),
            scale: window.scale_factor(),
            frame_no: 0,
            jobs: job_tx,
            results: out_rx,
            busy: false,
            glow_img: None,
            ascii_img: None,
            stale: Vec::new(),
            zones: [0.0; ZONES],
            lum_grid: None,
            scroll: 0.0,
            ascii_scroll: 0.0,
            ascii_drawn_at: 0.0,
            stats: (0.0, 0.0, 0, now),
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
    pub fn set_music_reactivity(&mut self, v: f32) {
        self.music_reactivity = v.clamp(0.0, 1.0);
        self.still_drawn = None;
    }

    pub fn set_intensity(&mut self, v: f32) {
        self.intensity = v.clamp(0.0, 1.0);
    }
    /// Colour theme glow colours (`None`: the presets' own).
    pub fn set_theme(&mut self, theme: Option<Palette>) {
        if theme != self.theme {
            self.prev_theme = self.theme;
            self.theme = theme;
            self.theme_since = Instant::now();
            self.still_drawn = None;
        }
    }
    pub fn set_palette(&mut self, palette: Option<Palette>) {
        self.fade.set(palette);
    }
    pub fn set_preset(&mut self, preset: Preset) {
        if preset != self.preset {
            self.prev_preset = self.preset;
            self.preset = preset;
            self.preset_since = Instant::now();
        }
    }
    /// Ramps of `preset`, fading from the previous theme to the current one.
    fn themed_ramps(&self, preset: Preset, pal: Option<&Palette>, blend: f32, theme_t: f32) -> (super::field::Ramp, super::field::Ramp) {
        let now = preset.ramps(self.theme.as_ref(), pal, blend);
        if theme_t >= 1.0 {
            return now;
        }
        let before = preset.ramps(self.prev_theme.as_ref(), pal, blend);
        let t = theme_t * theme_t * (3.0 - 2.0 * theme_t);
        (before.0.lerp(&now.0, t), before.1.lerp(&now.1, t))
    }
    pub fn preset(&self) -> Preset {
        self.preset
    }
    /// Scroll position of the main view (px scrolled down). Returns true when
    /// it changed (the view then repaints right away, shifting the last frame).
    pub fn set_scroll(&mut self, y: f32) -> bool {
        let y = y.max(0.0);
        let changed = (y - self.scroll).abs() > 0.01;
        self.scroll = y;
        changed
    }
    /// Short glow pulse, e.g. on every new lyric line.
    pub fn pulse(&mut self) {
        self.pulse = 1.0;
    }
    /// Displayed background luminance (0..1) per text zone.
    pub fn zones(&self) -> [f32; ZONES] {
        self.zones
    }
    /// Displayed background luminance grid of the last frame (window coords).
    pub fn lum_grid(&self) -> Option<Arc<theme::LumGrid>> {
        self.lum_grid.clone()
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

    /// Collects a finished frame and queues the next job. Returns true when
    /// there is something new to draw.
    fn tick(&mut self) -> bool {
        // Only paint can release GPU images via Window::drop_image. When a
        // window is minimized/occluded, generating more frames would keep
        // appending retired images forever. Apply backpressure until paint
        // drains them; keep requesting paint so restoration resumes naturally.
        if self.stale.len() >= 4 {
            return true;
        }
        let mut fresh = false;
        if let Ok(out) = self.results.try_recv() {
            self.busy = false;
            let t0 = Instant::now();
            let glow = Arc::new(RenderImage::new(vec![image::Frame::new(out.glow)]));
            if let Some(old) = self.glow_img.replace(glow) {
                self.stale.push(old);
            }
            if let Some(a) = out.ascii {
                self.ascii_drawn_at = out.scroll;
                let a = Arc::new(RenderImage::new(vec![image::Frame::new(a)]));
                if let Some(old) = self.ascii_img.replace(a) {
                    self.stale.push(old);
                }
            }
            self.zones = out.zones;
            self.lum_grid = Some(out.lum);
            let ui_ms = t0.elapsed().as_secs_f32() * 1000.0;
            self.stats.0 = self.stats.0.max(out.ms);
            self.stats.1 = self.stats.1.max(ui_ms);
            self.stats.2 += 1;
            if self.stats.3.elapsed() > Duration::from_secs(5) {
                tracing::debug!(
                    "glow: worker max {:.2} ms, ui upload max {:.2} ms, {} frames / 5 s",
                    self.stats.0,
                    self.stats.1,
                    self.stats.2
                );
                self.stats = (0.0, 0.0, 0, Instant::now());
            }
            fresh = true;
        }

        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f32().min(0.25);
        self.last_tick = now;
        // Window move / resize in progress: hold the last frame.
        if self.moved_at.elapsed() < Duration::from_millis(250) || self.busy {
            return fresh;
        }
        // Intensity 0: the background stands still (eases to a stop).
        let k_target = if self.intensity > 0.0 { 1.0 } else { 0.0 };
        self.motion_k += (k_target - self.motion_k) * (1.0 - (-dt * 3.0).exp());
        if (k_target - self.motion_k).abs() < 0.002 {
            self.motion_k = k_target;
        }
        self.anim_t += dt * self.motion_k;
        let t = self.anim_t;
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
            return fresh;
        }
        let mk = self.motion_k;
        let [bass, mids, highs] = self.bands.map(|b| b * mk * self.music_reactivity);
        let pulse = self.pulse * mk * self.music_reactivity;
        let playing = self.playing * self.music_reactivity;
        let preset_t = (self.preset_since.elapsed().as_secs_f32() / PRESET_FADE).min(1.0);
        let theme_t = (self.theme_since.elapsed().as_secs_f32() / PRESET_FADE).min(1.0);
        let mut still_key = None;
        if mk == 0.0 {
            // Still: draw again only when the picture can differ.
            let key = (w, h, self.scroll);
            let settled = self.fade.settled() && preset_t >= 1.0 && theme_t >= 1.0 && (target - self.playing).abs() < 0.001;
            if settled && self.glow_img.is_some() && self.ascii_img.is_some() && self.still_drawn == Some(key) {
                return fresh;
            }
            still_key = settled.then_some(key);
        }
        let mut motion = Motion::new(&self.noise, t, playing, bass, mids);
        motion.gain += pulse * 0.25;
        let (pal, blend) = self.fade.current();
        let job = Job {
            w,
            h,
            scale: self.scale,
            motion,
            preset: self.preset,
            ramps: self.themed_ramps(self.preset, pal.as_ref(), blend, theme_t),
            prev: (preset_t < 1.0)
                .then(|| (self.prev_preset, self.themed_ramps(self.prev_preset, pal.as_ref(), blend, theme_t), preset_t)),
            level: 0.5 + 0.5 * self.intensity,
            density: 0.75 + 0.45 * highs + 0.08 * playing,
            threshold: 0.42 - 0.06 * highs,
            // 15 fps normally, every frame while the page scrolls or stands still
            // (a still frame is the last one for a while).
            ascii: self.frame_no % 2 == 0 || mk == 0.0 || self.ascii_img.is_none() || (self.scroll - self.ascii_scroll).abs() > 0.5,
            dot_level: 0.18 + 0.14 * self.intensity + 0.28 * bass + 0.10 * pulse,
            blend,
            scroll: self.scroll,
        };
        if job.ascii {
            self.ascii_scroll = self.scroll;
        }
        self.frame_no += 1;
        if self.jobs.try_send(job).is_ok() {
            self.busy = true;
            self.still_drawn = still_key;
        }
        fresh
    }

    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window) {
        if bounds.size != self.view_size || window.scale_factor() != self.scale {
            self.view_size = bounds.size;
            self.scale = window.scale_factor();
            self.moved_at = Instant::now() - Duration::from_millis(240);
        }
        for old in self.stale.drain(..) {
            let _ = window.drop_image(old);
        }
        window.paint_quad(fill(bounds, theme::bg()));
        // Frames arrive a few ms after a scroll; shift the last ones by the
        // difference so the background follows the page without lag.
        let shifted = |b: Bounds<Pixels>, drawn_at: f32, scroll: f32| {
            let dy = -(scroll - drawn_at) * PARALLAX;
            Bounds { origin: gpui::point(b.origin.x, b.origin.y + px(dy)), size: b.size }
        };
        if let Some(img) = self.glow_img.clone() {
            let _ = window.paint_image(bounds, bounds, Default::default(), img, 0, false);
        }
        if let Some(img) = self.ascii_img.clone() {
            // The titlebar dot grid stays put; the field below moves.
            let bar = px(theme::TITLEBAR_H);
            let top = Bounds { origin: bounds.origin, size: size(bounds.size.width, bar) };
            let rest = Bounds {
                origin: gpui::point(bounds.origin.x, bounds.origin.y + bar),
                size: size(bounds.size.width, bounds.size.height - bar),
            };
            let _ = window.paint_image(top, bounds, Default::default(), img.clone(), 0, false);
            let _ = window.paint_image(rest, shifted(bounds, self.ascii_drawn_at, self.scroll), Default::default(), img, 0, false);
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

#[inline]
fn u8c(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0) as u8
}
