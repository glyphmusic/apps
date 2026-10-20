//! Channel rendering for natively decoded songs: mono, stereo, or spatial.
//!
//! * **Mono**: every channel mixed into one signal on both sides.
//! * **Stereo**: two channels as they are; multichannel files (5.1, 7.1, …)
//!   are mixed down by speaker position (ITU-R BS.775 style weights) instead
//!   of dropping every channel after the first two.
//! * **Spatial**: multichannel files keep their speaker positions. On a
//!   surround output with enough channels they pass straight through; on
//!   headphones / stereo outputs every channel becomes a virtual speaker at
//!   its standard angle and is rendered binaurally with a structural HRTF
//!   (spherical head after Brown & Duda 1998: interaural time difference by
//!   Woodworth's formula, head shadow as a first-order shelf per ear, a soft
//!   pinna cue for sources behind the listener). Stereo files play from two
//!   virtual speakers at ±30°, which moves the sound out of the head.
//!
//! The mode is global and applies live; each song's renderer picks a change
//! up within a few milliseconds.

use std::sync::atomic::{AtomicU16, AtomicU32, AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Mono,
    Stereo,
    Spatial,
}

static MODE: AtomicU8 = AtomicU8::new(1);
static GEN: AtomicU32 = AtomicU32::new(0);
/// Channels of the opened output device.
static DEVICE_CH: AtomicU16 = AtomicU16::new(2);

pub fn set_mode(m: Mode) {
    MODE.store(
        match m {
            Mode::Mono => 0,
            Mode::Stereo => 1,
            Mode::Spatial => 2,
        },
        Ordering::Relaxed,
    );
    GEN.fetch_add(1, Ordering::Relaxed);
}

pub fn mode() -> Mode {
    match MODE.load(Ordering::Relaxed) {
        0 => Mode::Mono,
        2 => Mode::Spatial,
        _ => Mode::Stereo,
    }
}

pub(crate) fn set_device_channels(n: u16) {
    DEVICE_CH.store(n.max(1), Ordering::Relaxed);
}

/// Where one input channel sits around the listener.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speaker {
    /// Degrees, 0 = front, positive = right, ±180 = behind.
    pub azimuth: f32,
    /// Low-frequency effects channel: no direction.
    pub lfe: bool,
}

const fn sp(azimuth: f32) -> Speaker {
    Speaker { azimuth, lfe: false }
}

/// Positions of symphonia's channel bits, in bit order (interleaved samples
/// follow the same order).
const BIT_POSITIONS: [Speaker; 26] = [
    sp(-30.0),  // FRONT_LEFT
    sp(30.0),   // FRONT_RIGHT
    sp(0.0),    // FRONT_CENTRE
    Speaker { azimuth: 0.0, lfe: true }, // LFE1
    sp(-135.0), // REAR_LEFT
    sp(135.0),  // REAR_RIGHT
    sp(-15.0),  // FRONT_LEFT_CENTRE
    sp(15.0),   // FRONT_RIGHT_CENTRE
    sp(180.0),  // REAR_CENTRE
    sp(-100.0), // SIDE_LEFT
    sp(100.0),  // SIDE_RIGHT
    sp(0.0),    // TOP_CENTRE
    sp(-30.0),  // TOP_FRONT_LEFT
    sp(0.0),    // TOP_FRONT_CENTRE
    sp(30.0),   // TOP_FRONT_RIGHT
    sp(-135.0), // TOP_REAR_LEFT
    sp(180.0),  // TOP_REAR_CENTRE
    sp(135.0),  // TOP_REAR_RIGHT
    sp(-150.0), // REAR_LEFT_CENTRE
    sp(150.0),  // REAR_RIGHT_CENTRE
    sp(-60.0),  // FRONT_LEFT_WIDE
    sp(60.0),   // FRONT_RIGHT_WIDE
    sp(-30.0),  // FRONT_LEFT_HIGH
    sp(0.0),    // FRONT_CENTRE_HIGH
    sp(30.0),   // FRONT_RIGHT_HIGH
    Speaker { azimuth: 0.0, lfe: true }, // LFE2
];

/// Speaker positions for `count` channels with symphonia channel mask `mask`
/// (0 = unknown: the usual WAVE order for that count).
pub fn layout(mask: u32, count: usize) -> Vec<Speaker> {
    let from_mask: Vec<Speaker> = (0..26).filter(|b| mask & (1 << b) != 0).map(|b| BIT_POSITIONS[b]).collect();
    if from_mask.len() == count {
        return from_mask;
    }
    let lfe = Speaker { azimuth: 0.0, lfe: true };
    match count {
        1 => vec![sp(0.0)],
        2 => vec![sp(-30.0), sp(30.0)],
        3 => vec![sp(-30.0), sp(30.0), sp(0.0)],
        4 => vec![sp(-30.0), sp(30.0), sp(-135.0), sp(135.0)],
        5 => vec![sp(-30.0), sp(30.0), sp(0.0), sp(-110.0), sp(110.0)],
        6 => vec![sp(-30.0), sp(30.0), sp(0.0), lfe, sp(-110.0), sp(110.0)],
        7 => vec![sp(-30.0), sp(30.0), sp(0.0), lfe, sp(180.0), sp(-100.0), sp(100.0)],
        8 => vec![sp(-30.0), sp(30.0), sp(0.0), lfe, sp(-135.0), sp(135.0), sp(-100.0), sp(100.0)],
        n => (0..n).map(|i| sp(-180.0 + 360.0 * (i as f32 + 0.5) / n as f32)).collect(),
    }
}

/// Head radius (m) and speed of sound (m/s) of the head model.
const HEAD_A: f32 = 0.0875;
const SOUND_C: f32 = 343.0;
/// Longest interaural delay we need, with room (samples at up to 192 kHz).
const DELAY_LEN: usize = 256;

/// One ear's view of one virtual speaker: delay + head-shadow shelf +
/// pinna low-pass for sources behind.
#[derive(Clone, Copy, Default)]
struct EarPath {
    delay: f32,
    b0: f32,
    b1: f32,
    a1: f32,
    x1: f32,
    y1: f32,
    /// One-pole low-pass amount (0 = none) and its state.
    rear: f32,
    lp: f32,
}

impl EarPath {
    /// `theta`: angle between the source and this ear's axis (rad, 0..π).
    fn new(theta: f32, rear: bool, rate: f32) -> Self {
        let a_c = HEAD_A / SOUND_C;
        // Woodworth / Brown-Duda: earlier at the near ear, later around the head.
        let t = if theta < std::f32::consts::FRAC_PI_2 { -a_c * theta.cos() } else { a_c * (theta - std::f32::consts::FRAC_PI_2) };
        let delay = (t + a_c) * rate;
        // Head shadow: (1 + α s/2ω0) / (1 + s/2ω0), bilinear transform.
        let theta_min = 150f32.to_radians();
        let alpha_min = 0.1;
        let alpha = (1.0 + alpha_min / 2.0) + (1.0 - alpha_min / 2.0) * (theta / theta_min * std::f32::consts::PI).cos();
        let w0 = SOUND_C / HEAD_A;
        let tau = 1.0 / (2.0 * w0);
        let k = 2.0 * rate;
        let d = 1.0 + tau * k;
        Self {
            delay,
            b0: (1.0 + alpha * tau * k) / d,
            b1: (1.0 - alpha * tau * k) / d,
            a1: (1.0 - tau * k) / d,
            rear: if rear { 0.35 } else { 0.0 },
            ..Default::default()
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 - self.a1 * self.y1;
        self.x1 = x;
        self.y1 = y;
        if self.rear > 0.0 {
            self.lp += (y - self.lp) * (1.0 - self.rear);
            self.lp
        } else {
            y
        }
    }
}

/// Binaural renderer: every input channel a virtual speaker.
struct Binaural {
    speakers: Vec<Speaker>,
    left: Vec<EarPath>,
    right: Vec<EarPath>,
    /// Per input channel: delay line of the filtered signal per ear.
    lines: Vec<[f32; DELAY_LEN]>,
    pos: usize,
    gain: f32,
}

impl Binaural {
    fn new(speakers: &[Speaker], rate: f32) -> Self {
        let mut left = Vec::new();
        let mut right = Vec::new();
        for s in speakers {
            let az = s.azimuth.to_radians();
            // Angle to each ear's axis (right ear at +90°, left at -90°).
            let to = |ear: f32| {
                let d = (az - ear).abs() % std::f32::consts::TAU;
                if d > std::f32::consts::PI { std::f32::consts::TAU - d } else { d }
            };
            let rear = s.azimuth.abs() > 100.0;
            left.push(EarPath::new(to(-std::f32::consts::FRAC_PI_2), rear, rate));
            right.push(EarPath::new(to(std::f32::consts::FRAC_PI_2), rear, rate));
        }
        let n = speakers.iter().filter(|s| !s.lfe).count().max(1) as f32;
        Self {
            lines: vec![[0.0; DELAY_LEN]; speakers.len() * 2],
            speakers: speakers.to_vec(),
            left,
            right,
            pos: 0,
            // Two speakers of correlated bass sum up; keep headroom.
            gain: (0.62 * (2.0 / n).sqrt()).min(0.62),
        }
    }

    fn tap(line: &[f32; DELAY_LEN], pos: usize, delay: f32) -> f32 {
        let d = delay.clamp(0.0, (DELAY_LEN - 2) as f32);
        let i = d.floor() as usize;
        let f = d - i as f32;
        let a = line[(pos + DELAY_LEN - i) % DELAY_LEN];
        let b = line[(pos + DELAY_LEN - i - 1) % DELAY_LEN];
        a + (b - a) * f
    }

    fn process(&mut self, input: &[f32], out: &mut [f32; 2]) {
        let (mut l, mut r) = (0.0, 0.0);
        self.pos = (self.pos + 1) % DELAY_LEN;
        for (c, &x) in input.iter().enumerate().take(self.speakers.len()) {
            if self.speakers[c].lfe {
                l += x * 0.5;
                r += x * 0.5;
                continue;
            }
            let li = c * 2;
            self.lines[li][self.pos] = self.left[c].process(x);
            self.lines[li + 1][self.pos] = self.right[c].process(x);
            l += Self::tap(&self.lines[li], self.pos, self.left[c].delay);
            r += Self::tap(&self.lines[li + 1], self.pos, self.right[c].delay);
        }
        out[0] = l * self.gain;
        out[1] = r * self.gain;
    }
}

/// Mixes `input` (laid out as `speakers`) to a stereo pair by position.
fn downmix(speakers: &[Speaker], input: &[f32], weights: &[(f32, f32)], out: &mut [f32; 2]) {
    let (mut l, mut r) = (0.0, 0.0);
    for (c, &x) in input.iter().enumerate().take(speakers.len()) {
        let (wl, wr) = weights[c];
        l += x * wl;
        r += x * wr;
    }
    out[0] = l;
    out[1] = r;
}

/// Equal-power pan weights for a stereo downmix, scaled so a full-scale
/// signal on every front channel does not clip.
fn downmix_weights(speakers: &[Speaker]) -> Vec<(f32, f32)> {
    let raw: Vec<(f32, f32)> = speakers
        .iter()
        .map(|s| {
            if s.lfe {
                return (0.0, 0.0);
            }
            // Front/back folded onto the left-right axis.
            let az = s.azimuth.clamp(-180.0, 180.0);
            let lateral = if az.abs() > 90.0 { az.signum() * (180.0 - az.abs()) } else { az };
            let p = ((lateral / 90.0).clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            // Surround channels a little quieter (BS.775: -3 dB).
            let k = if az.abs() > 90.0 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 };
            (p.cos() * k, p.sin() * k)
        })
        .collect();
    let sum_l: f32 = raw.iter().map(|w| w.0).sum();
    let sum_r: f32 = raw.iter().map(|w| w.1).sum();
    let norm = sum_l.max(sum_r).max(1.0);
    raw.into_iter().map(|(l, r)| (l / norm, r / norm)).collect()
}

/// Per-song channel renderer: input frames in, output frames out.
pub struct Renderer {
    rate: f32,
    speakers: Vec<Speaker>,
    out_channels: u16,
    gen: u32,
    mode: Mode,
    weights: Vec<(f32, f32)>,
    binaural: Option<Binaural>,
    bypass: bool,
}

impl Renderer {
    /// Passes stereo through untouched (tests of the decode path).
    pub fn bypass(channels: u16) -> Self {
        Self { bypass: true, out_channels: channels, ..Self::new(48_000, channels, 0) }
    }

    /// `mask`: symphonia channel bits of the song (0 = unknown).
    pub fn new(rate: u32, channels: u16, mask: u32) -> Self {
        let speakers = layout(mask, channels.max(1) as usize);
        let device = DEVICE_CH.load(Ordering::Relaxed);
        // Real surround outputs get multichannel files as they are (Spatial
        // only); everything else renders to two channels.
        let out_channels = if channels > 2 && device >= channels { channels } else { 2 };
        let mut r = Self {
            rate: rate.max(1) as f32,
            weights: downmix_weights(&speakers),
            speakers,
            out_channels,
            gen: u32::MAX,
            mode: Mode::Stereo,
            binaural: None,
            bypass: false,
        };
        r.refresh();
        r
    }

    /// Channels this renderer puts out (fixed for the song).
    pub fn out_channels(&self) -> u16 {
        self.out_channels
    }

    fn refresh(&mut self) {
        let g = GEN.load(Ordering::Relaxed);
        if g == self.gen {
            return;
        }
        self.gen = g;
        self.mode = mode();
        self.binaural = (self.mode == Mode::Spatial && self.out_channels == 2).then(|| {
            // Mono files have nothing to place; the head model would only colour them.
            if self.speakers.len() < 2 {
                return None;
            }
            Some(Binaural::new(&self.speakers, self.rate))
        }).flatten();
    }

    /// Renders one input frame into `out` (`out_channels` samples).
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        if self.bypass {
            let n = input.len().min(out.len());
            out[..n].copy_from_slice(&input[..n]);
            return;
        }
        // Picking up a mode change costs one atomic load per frame.
        self.refresh();
        let n = input.len();
        if self.out_channels as usize == n && n > 2 {
            // Surround output: positions as they are, or folded for mono / stereo.
            match self.mode {
                Mode::Spatial => out[..n].copy_from_slice(input),
                Mode::Stereo => {
                    let mut st = [0.0; 2];
                    downmix(&self.speakers, input, &self.weights, &mut st);
                    out.fill(0.0);
                    out[0] = st[0];
                    out[1] = st[1];
                }
                Mode::Mono => {
                    let mut st = [0.0; 2];
                    downmix(&self.speakers, input, &self.weights, &mut st);
                    out.fill(0.0);
                    let m = (st[0] + st[1]) * 0.5;
                    out[0] = m;
                    out[1] = m;
                }
            }
            return;
        }
        let mut st = [0.0f32; 2];
        match n {
            0 => {}
            1 => st = [input[0], input[0]],
            2 if self.binaural.is_none() => st = [input[0], input[1]],
            _ => match self.binaural.as_mut() {
                Some(b) => b.process(input, &mut st),
                None => downmix(&self.speakers, input, &self.weights, &mut st),
            },
        }
        if self.mode == Mode::Mono {
            let m = (st[0] + st[1]) * 0.5;
            st = [m, m];
        }
        out[0] = st[0];
        out[1] = st[1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mode is global: tests that set it take turns.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn render(mode: Mode, ch: u16, frames: &[Vec<f32>]) -> Vec<[f32; 2]> {
        set_mode(mode);
        let mut r = Renderer::new(48_000, ch, 0);
        frames
            .iter()
            .map(|f| {
                let mut o = [0.0; 2];
                r.process(f, &mut o);
                o
            })
            .collect()
    }

    #[test]
    fn layouts_follow_mask_or_count() {
        let l = layout(0b1111_11, 6);
        assert_eq!(l.len(), 6);
        assert!(l[3].lfe);
        assert_eq!(layout(0, 2), vec![sp(-30.0), sp(30.0)]);
        assert_eq!(layout(0, 9).len(), 9);
    }

    #[test]
    fn mono_mixes_both_sides() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let out = render(Mode::Mono, 2, &[vec![1.0, 0.0]]);
        assert_eq!(out[0], [0.5, 0.5]);
        set_mode(Mode::Stereo);
    }

    #[test]
    fn stereo_keeps_stereo_and_folds_surround() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let out = render(Mode::Stereo, 2, &[vec![0.3, -0.2]]);
        assert_eq!(out[0], [0.3, -0.2]);
        // 5.1: centre to both sides, left surround mostly left, LFE dropped.
        let out = render(Mode::Stereo, 6, &[vec![0.0, 0.0, 1.0, 0.0, 0.0, 0.0], vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0], vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0]]);
        assert!((out[0][0] - out[0][1]).abs() < 1e-6 && out[0][0] > 0.1);
        assert!(out[1][0] > out[1][1] * 3.0);
        assert_eq!(out[2], [0.0, 0.0]);
        // Never louder than full scale on all channels.
        let out = render(Mode::Stereo, 6, &[vec![1.0; 6]]);
        assert!(out[0][0] <= 1.0001 && out[0][1] <= 1.0001);
    }

    #[test]
    fn spatial_places_a_right_source_right_and_later_left() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_mode(Mode::Spatial);
        let mut r = Renderer::new(48_000, 6, 0);
        // Impulse on the right surround channel.
        let mut left = Vec::new();
        let mut right = Vec::new();
        for i in 0..200 {
            let mut f = vec![0.0; 6];
            if i == 0 {
                f[5] = 1.0;
            }
            let mut o = [0.0; 2];
            r.process(&f, &mut o);
            left.push(o[0]);
            right.push(o[1]);
        }
        let peak = |v: &[f32]| v.iter().enumerate().max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap()).map(|(i, x)| (i, x.abs())).unwrap();
        let (il, pl) = peak(&left);
        let (ir, pr) = peak(&right);
        assert!(pr > pl, "right ear louder: {pr} vs {pl}");
        assert!(ir < il, "right ear first: {ir} vs {il}");
        set_mode(Mode::Stereo);
    }
}
