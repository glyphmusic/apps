//! Clean sample rate conversion for natively decoded songs.
//!
//! Most songs are 44.1 kHz, most outputs run at 48 kHz. Left to rodio, the
//! rate is changed by drawing straight lines between samples: that takes
//! about 4 dB off at 16 kHz and folds mirror images back into the top
//! octave, so songs sound dull and far away. Here each output sample is
//! built from 64 input samples with a windowed sinc (Kaiser), which keeps
//! everything up to about 20 kHz flat and cuts the mirror images, so rodio
//! sees the device rate and passes the samples straight through.

use std::sync::atomic::{AtomicU32, Ordering};

/// Rate of the opened output device (0 = not known yet).
static DEVICE_RATE: AtomicU32 = AtomicU32::new(0);

pub fn set_device_rate(rate: u32) {
    DEVICE_RATE.store(rate, Ordering::Relaxed);
}

pub fn device_rate() -> Option<u32> {
    match DEVICE_RATE.load(Ordering::Relaxed) {
        0 => None,
        r => Some(r),
    }
}

/// Input samples per output sample (per channel).
const TAPS: usize = 64;
const HALF: usize = TAPS / 2;
/// Fine steps between two input samples (filter rows).
const PHASES: usize = 512;
/// Kaiser window shape (~80 dB of stop band).
const BETA: f64 = 8.0;

/// Bessel function I0 (series), for the Kaiser window.
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..50 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < sum * 1e-12 {
            break;
        }
    }
    sum
}

pub struct Resampler {
    ch: usize,
    /// Input frames per output frame.
    step: f64,
    /// Where the next output sits between input frame `HALF - 1` and `HALF`
    /// of the window (0..1).
    t: f64,
    /// The last `TAPS` input frames, interleaved.
    buf: Vec<f32>,
    /// `PHASES + 1` rows of `TAPS` filter weights.
    table: Vec<f32>,
    primed: bool,
    /// Input frames still to feed after the song ended (silence), so its
    /// last samples come out too.
    tail: usize,
    ended: bool,
}

impl Resampler {
    /// `None` when there is nothing to convert.
    pub fn new(from: u32, to: u32, channels: usize) -> Option<Self> {
        if from == 0 || to == 0 || from == to || channels == 0 {
            return None;
        }
        // Cut just below the lower of the two Nyquist rates.
        let fc = (to as f64 / from as f64).min(1.0) * 0.95;
        let mut table = vec![0f32; (PHASES + 1) * TAPS];
        let norm = bessel_i0(BETA);
        for p in 0..=PHASES {
            let t = p as f64 / PHASES as f64;
            let row = &mut table[p * TAPS..(p + 1) * TAPS];
            let mut sum = 0.0;
            for (k, w) in row.iter_mut().enumerate() {
                // Distance of input sample k from the output point.
                let d = (HALF - 1) as f64 + t - k as f64;
                let x = std::f64::consts::PI * fc * d;
                let sinc = if x.abs() < 1e-9 { 1.0 } else { x.sin() / x };
                let r = d / HALF as f64;
                let win = if r.abs() >= 1.0 { 0.0 } else { bessel_i0(BETA * (1.0 - r * r).sqrt()) / norm };
                let v = fc * sinc * win;
                *w = v as f32;
                sum += v;
            }
            // Unity gain at every step (no ripple from rounding).
            if sum.abs() > 1e-9 {
                for w in row.iter_mut() {
                    *w = (*w as f64 / sum) as f32;
                }
            }
        }
        Some(Self {
            ch: channels,
            step: from as f64 / to as f64,
            t: 0.0,
            buf: vec![0.0; TAPS * channels],
            table,
            primed: false,
            tail: HALF,
            ended: false,
        })
    }

    /// Moves the window one input frame on. `pull` fills one frame and
    /// returns `false` at the end of the song.
    fn shift(&mut self, pull: &mut impl FnMut(&mut [f32]) -> bool) -> bool {
        let ch = self.ch;
        self.buf.copy_within(ch.., 0);
        let last = &mut self.buf[(TAPS - 1) * ch..];
        if !self.ended && pull(last) {
            return true;
        }
        self.ended = true;
        last.fill(0.0);
        if self.tail == 0 {
            return false;
        }
        self.tail -= 1;
        true
    }

    /// Writes the next output frame into `out` (`channels` samples).
    /// Returns `false` once the song and its tail are out.
    pub fn next_frame(&mut self, out: &mut [f32], mut pull: impl FnMut(&mut [f32]) -> bool) -> bool {
        let ch = self.ch;
        if !self.primed {
            self.primed = true;
            // Silence before the song, its first frame right at the output point.
            for _ in 0..=HALF {
                if !self.shift(&mut pull) {
                    return false;
                }
            }
        }
        while self.t >= 1.0 {
            self.t -= 1.0;
            if !self.shift(&mut pull) {
                return false;
            }
        }
        let pos = self.t * PHASES as f64;
        let p = (pos as usize).min(PHASES - 1);
        let f = (pos - p as f64) as f32;
        let (a, b) = (&self.table[p * TAPS..(p + 1) * TAPS], &self.table[(p + 1) * TAPS..(p + 2) * TAPS]);
        for (c, o) in out.iter_mut().enumerate().take(ch) {
            let mut acc = 0.0f32;
            for k in 0..TAPS {
                let w = a[k] + (b[k] - a[k]) * f;
                acc += w * self.buf[k * ch + c];
            }
            *o = acc;
        }
        self.t += self.step;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(from: u32, to: u32, freq: f32, secs: f32) -> Vec<f32> {
        let mut r = Resampler::new(from, to, 1).unwrap();
        let n = (from as f32 * secs) as usize;
        let mut i = 0usize;
        let mut out = Vec::new();
        let mut o = [0f32; 1];
        while r.next_frame(&mut o, |f| {
            if i >= n {
                return false;
            }
            f[0] = (2.0 * std::f32::consts::PI * freq * i as f32 / from as f32).sin() * 0.5;
            i += 1;
            true
        }) {
            out.push(o[0]);
        }
        out
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
    }

    #[test]
    fn keeps_length() {
        let out = run(44_100, 48_000, 1000.0, 1.0);
        assert!((out.len() as i64 - 48_000).abs() < 40, "{}", out.len());
    }

    #[test]
    fn highs_stay_flat() {
        // 16 kHz lost ~4 dB with straight lines; here it stays within 0.2 dB.
        for freq in [1000.0, 10_000.0, 16_000.0, 19_000.0] {
            let out = run(44_100, 48_000, freq, 0.5);
            let mid = &out[2000..out.len() - 2000];
            let db = 20.0 * (rms(mid) / (0.5 / 2f32.sqrt())).log10();
            assert!(db.abs() < 0.2, "{freq} Hz: {db} dB");
        }
    }

    #[test]
    fn down_cuts_what_would_fold() {
        // 30 kHz in a 96 kHz file cannot exist at 48 kHz: it must go, not fold to 18 kHz.
        let out = run(96_000, 48_000, 30_000.0, 0.5);
        let mid = &out[2000..out.len() - 2000];
        assert!(rms(mid) < 0.001, "{}", rms(mid));
    }

    #[test]
    fn same_rate_is_none() {
        assert!(Resampler::new(48_000, 48_000, 2).is_none());
    }
}
