//! 10-band graphic equalizer for natively decoded songs (biquads: low shelf,
//! eight peaking bands, high shelf), plus a bass shelf and a headroom cut
//! before it. Settings are global and apply live; each song's processor
//! picks up changes within a few milliseconds.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

/// Band centre frequencies (Hz).
pub const BANDS: [f32; 10] = [32.0, 64.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
/// Gain range per band (dB).
pub const MAX_DB: f32 = 12.0;

static ENABLED: AtomicBool = AtomicBool::new(false);
static GEN: AtomicU32 = AtomicU32::new(0);
static GAINS: Mutex<[f32; 10]> = Mutex::new([0.0; 10]);
/// Bass shelf gain and extra headroom (dB).
static EXTRAS: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));
/// Corner of the bass shelf (Hz).
pub const BASS_HZ: f32 = 90.0;

/// Applies immediately to everything playing.
pub fn set(enabled: bool, gains: [f32; 10]) {
    *GAINS.lock().unwrap() = gains.map(|g| if g.is_finite() { g.clamp(-MAX_DB, MAX_DB) } else { 0.0 });
    ENABLED.store(enabled, Ordering::Relaxed);
    GEN.fetch_add(1, Ordering::Relaxed);
}

/// Bass shelf (-12 ..= 12 dB) and headroom taken off before the equalizer
/// (0 ..= 12 dB). Both only act while the equalizer is on.
pub fn set_extras(bass_db: f32, headroom_db: f32) {
    let clean = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { 0.0 };
    *EXTRAS.lock().unwrap() = (clean(bass_db, -MAX_DB, MAX_DB), clean(headroom_db, 0.0, MAX_DB));
    GEN.fetch_add(1, Ordering::Relaxed);
}

#[derive(Clone, Copy, Default)]
struct Coef {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Clone, Copy, Default)]
struct St {
    z1: f32,
    z2: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    LowShelf,
    Peak,
    HighShelf,
}

fn design(kind: Kind, f: f32, gain_db: f32, rate: f32) -> Coef {
    let a = 10f32.powf(gain_db / 40.0);
    let w0 = 2.0 * std::f32::consts::PI * f / rate;
    let (s, c) = w0.sin_cos();
    let (b0, b1, b2, a0, a1, a2) = match kind {
        Kind::Peak => {
            let alpha = s / (2.0 * 1.1);
            (1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a)
        }
        Kind::LowShelf => {
            let alpha = s / 2.0 * std::f32::consts::SQRT_2;
            let k = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) - (a - 1.0) * c + k),
                2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                a * ((a + 1.0) - (a - 1.0) * c - k),
                (a + 1.0) + (a - 1.0) * c + k,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - k,
            )
        }
        Kind::HighShelf => {
            let alpha = s / 2.0 * std::f32::consts::SQRT_2;
            let k = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) + (a - 1.0) * c + k),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - k),
                (a + 1.0) - (a - 1.0) * c + k,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - k,
            )
        }
    };
    Coef { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
}

/// Per-song filter state (one set of biquads per channel).
pub struct Equalizer {
    rate: f32,
    channels: usize,
    ch: usize,
    gen: u32,
    on: bool,
    pre: f32,
    coefs: Vec<Coef>,
    state: Vec<St>,
    check: u32,
    bypass: bool,
}

impl Equalizer {
    /// Never filters (tests of the decode path).
    pub fn bypass() -> Self {
        Self { bypass: true, ..Self::new(48_000, 2) }
    }

    pub fn new(rate: u32, channels: u16) -> Self {
        let mut e = Self {
            rate: rate.max(1) as f32,
            channels: channels.max(1) as usize,
            ch: 0,
            gen: u32::MAX,
            on: false,
            pre: 1.0,
            coefs: Vec::new(),
            state: Vec::new(),
            check: 0,
            bypass: false,
        };
        e.refresh();
        e
    }

    fn refresh(&mut self) {
        let gen = GEN.load(Ordering::Relaxed);
        if gen == self.gen {
            return;
        }
        self.gen = gen;
        self.on = ENABLED.load(Ordering::Relaxed);
        let gains = *GAINS.lock().unwrap();
        let (bass, headroom) = *EXTRAS.lock().unwrap();
        self.coefs.clear();
        self.pre = 1.0;
        if self.on {
            for (i, (&f, &g)) in BANDS.iter().zip(gains.iter()).enumerate() {
                if g.abs() < 0.05 || f >= self.rate * 0.45 {
                    continue;
                }
                let kind = match i {
                    0 => Kind::LowShelf,
                    9 => Kind::HighShelf,
                    _ => Kind::Peak,
                };
                self.coefs.push(design(kind, f, g, self.rate));
            }
            if bass.abs() >= 0.05 {
                self.coefs.push(design(Kind::LowShelf, BASS_HZ, bass, self.rate));
            }
            // Headroom for boosts (the limiter after this catches the rest),
            // plus what the user takes off on top.
            let boost = gains.iter().cloned().fold(0.0f32, f32::max).max(bass.max(0.0) + gains[0].max(0.0) * 0.5);
            self.pre = 10f32.powf(-(boost * 0.6 + headroom) / 20.0);
        }
        if self.coefs.is_empty() && self.pre >= 0.9999 {
            self.on = false;
        }
        let n = self.coefs.len() * self.channels;
        if self.state.len() != n {
            self.state = vec![St::default(); n];
        }
    }

    /// One interleaved sample in, filtered sample out.
    #[inline]
    pub fn process(&mut self, s: f32) -> f32 {
        if self.bypass {
            return s;
        }
        self.check = self.check.wrapping_add(1);
        if self.check % 512 == 0 {
            self.refresh();
        }
        let ch = self.ch;
        self.ch = (self.ch + 1) % self.channels;
        if !self.on {
            return s;
        }
        let mut x = s * self.pre;
        let nb = self.coefs.len();
        for b in 0..nb {
            let c = self.coefs[b];
            let st = &mut self.state[ch * nb + b];
            // Transposed direct form II.
            let y = c.b0 * x + st.z1;
            st.z1 = c.b1 * x - c.a1 * y + st.z2;
            st.z2 = c.b2 * x - c.a2 * y;
            x = y;
        }
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms_through(f: f32, gains: [f32; 10]) -> f32 {
        set(true, gains);
        let rate = 48_000.0;
        let mut e = Equalizer::new(48_000, 1);
        let mut acc = 0.0f64;
        let n = 48_000;
        for i in 0..n {
            let s = (2.0 * std::f32::consts::PI * f * i as f32 / rate).sin() * 0.25;
            let y = e.process(s);
            if i > n / 2 {
                acc += (y * y) as f64;
            }
        }
        ((acc / (n / 2) as f64).sqrt() / (0.25 / std::f64::consts::SQRT_2)) as f32
    }

    #[test]
    fn bands_boost_and_cut_where_they_should() {
        // One test: the settings are global.
        let flat = rms_through(1000.0, [0.0; 10]);
        assert!((flat - 1.0).abs() < 0.02, "flat changed the level: {flat}");
        let mut g = [0.0; 10];
        g[5] = 9.0; // 1 kHz up
        let boosted = rms_through(1000.0, g);
        let other = rms_through(125.0, g);
        assert!(boosted > other * 1.8, "1k {boosted} vs 125 Hz {other}");
        let mut cut = [0.0; 10];
        cut[2] = -12.0;
        assert!(rms_through(125.0, cut) < 0.4);
        // Bass shelf lifts the lows, not the highs; headroom lowers all.
        set_extras(9.0, 0.0);
        let low = rms_through(50.0, [0.0; 10]);
        let high = rms_through(4000.0, [0.0; 10]);
        assert!(low > high * 1.5, "bass {low} vs 4k {high}");
        set_extras(0.0, 6.0);
        let quiet = rms_through(1000.0, [0.0; 10]);
        assert!((quiet - 0.5).abs() < 0.03, "6 dB headroom gave {quiet}");
        set_extras(0.0, 0.0);
        set(false, [0.0; 10]);
    }
}
