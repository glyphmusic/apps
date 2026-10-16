//! FFT tap: mono samples -> 32 log-spaced bands, normalised 0..1, smoothed.

use crate::SPECTRUM_BANDS;
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use std::sync::Arc;

/// FFT window length (samples).
pub const FFT_SIZE: usize = 2048;
const F_LO: f32 = 40.0;
const F_HI: f32 = 16_000.0;
/// dB range mapped onto 0..1.
const DB_FLOOR: f32 = -60.0;
const ATTACK: f32 = 0.65;
const RELEASE: f32 = 0.12;

/// Band edges in Hz: `SPECTRUM_BANDS + 1` log-spaced values from ~40 Hz up to
/// 16 kHz (clamped below Nyquist).
pub fn band_edges_hz(sample_rate: u32) -> [f32; SPECTRUM_BANDS + 1] {
    let hi = F_HI.min(sample_rate as f32 * 0.5 * 0.95).max(F_LO * 2.0);
    let ratio = hi / F_LO;
    let mut e = [0.0; SPECTRUM_BANDS + 1];
    for (i, v) in e.iter_mut().enumerate() {
        *v = F_LO * ratio.powf(i as f32 / SPECTRUM_BANDS as f32);
    }
    e
}

/// Start/end (exclusive) FFT bin for every band (always at least one bin,
/// never beyond `fft_size / 2`).
pub fn band_bin_ranges(sample_rate: u32, fft_size: usize) -> Vec<(usize, usize)> {
    let edges = band_edges_hz(sample_rate);
    let bin_hz = sample_rate as f32 / fft_size as f32;
    let max_bin = fft_size / 2;
    (0..SPECTRUM_BANDS)
        .map(|i| {
            let s = ((edges[i] / bin_hz).floor() as usize).clamp(1, max_bin - 1);
            let e = ((edges[i + 1] / bin_hz).ceil() as usize).clamp(s + 1, max_bin);
            (s, e)
        })
        .collect()
}

/// Map a linear amplitude (1.0 = full-scale sine) to 0..1 via dB.
pub fn amplitude_to_unit(amp: f32) -> f32 {
    let db = 20.0 * (amp.max(1e-9)).log10();
    ((db - DB_FLOOR) / -DB_FLOOR).clamp(0.0, 1.0)
}

/// Reduce amplitude spectrum bins to bands (peak per band, dB-normalised).
pub fn bands_from_amplitudes(amps: &[f32], ranges: &[(usize, usize)]) -> [f32; SPECTRUM_BANDS] {
    let mut out = [0.0; SPECTRUM_BANDS];
    for (o, &(s, e)) in out.iter_mut().zip(ranges) {
        let e = e.min(amps.len());
        let s = s.min(e);
        let peak = amps[s..e].iter().copied().fold(0.0f32, f32::max);
        *o = amplitude_to_unit(peak);
    }
    out
}

/// Attack/release smoothing in place.
pub fn smooth(prev: &mut [f32; SPECTRUM_BANDS], new: &[f32; SPECTRUM_BANDS]) {
    for (p, &n) in prev.iter_mut().zip(new) {
        let k = if n > *p { ATTACK } else { RELEASE };
        *p += (n - *p) * k;
        if *p < 0.002 {
            *p = 0.0;
        }
    }
}

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    buf: Vec<Complex32>,
    amps: Vec<f32>,
    ranges: Vec<(usize, usize)>,
    ranges_rate: u32,
    smoothed: [f32; SPECTRUM_BANDS],
}

impl Analyzer {
    pub fn new() -> Self {
        let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let window = (0..FFT_SIZE)
            .map(|i| {
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE as f32 - 1.0)).cos()
            })
            .collect();
        Self {
            fft,
            window,
            buf: vec![Complex32::new(0.0, 0.0); FFT_SIZE],
            amps: vec![0.0; FFT_SIZE / 2],
            ranges: Vec::new(),
            ranges_rate: 0,
            smoothed: [0.0; SPECTRUM_BANDS],
        }
    }

    /// Feed the most recent mono samples (any length; older ones are ignored,
    /// short input is zero-padded at the front) and get the smoothed bands.
    pub fn process(&mut self, recent: &[f32], sample_rate: u32) -> [f32; SPECTRUM_BANDS] {
        if self.ranges_rate != sample_rate {
            self.ranges = band_bin_ranges(sample_rate.max(8000), FFT_SIZE);
            self.ranges_rate = sample_rate;
        }
        let take = recent.len().min(FFT_SIZE);
        let src = &recent[recent.len() - take..];
        let pad = FFT_SIZE - take;
        for i in 0..FFT_SIZE {
            let s = if i < pad { 0.0 } else { src[i - pad] };
            self.buf[i] = Complex32::new(s * self.window[i], 0.0);
        }
        self.fft.process(&mut self.buf);
        // A full-scale sine under a Hann window peaks at N/4.
        let norm = FFT_SIZE as f32 / 4.0;
        for (a, c) in self.amps.iter_mut().zip(&self.buf) {
            *a = c.norm() / norm;
        }
        let bands = bands_from_amplitudes(&self.amps, &self.ranges);
        smooth(&mut self.smoothed, &bands);
        self.smoothed
    }

    /// Decay towards silence (paused / embed / no track).
    pub fn decay(&mut self) -> [f32; SPECTRUM_BANDS] {
        smooth(&mut self.smoothed, &[0.0; SPECTRUM_BANDS]);
        self.smoothed
    }
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_are_log_spaced_and_monotonic() {
        let e = band_edges_hz(44_100);
        assert!((e[0] - 40.0).abs() < 1e-3);
        assert!((e[SPECTRUM_BANDS] - 16_000.0).abs() < 1.0);
        for w in e.windows(2) {
            assert!(w[1] > w[0]);
        }
        let r0 = e[1] / e[0];
        let r1 = e[SPECTRUM_BANDS] / e[SPECTRUM_BANDS - 1];
        assert!((r0 - r1).abs() < 1e-3);
    }

    #[test]
    fn edges_clamp_to_nyquist_for_low_rates() {
        let e = band_edges_hz(16_000);
        assert!(e[SPECTRUM_BANDS] < 8_000.0);
    }

    #[test]
    fn ranges_cover_at_least_one_bin_and_stay_in_bounds() {
        for sr in [16_000u32, 44_100, 48_000, 96_000] {
            let r = band_bin_ranges(sr, FFT_SIZE);
            assert_eq!(r.len(), SPECTRUM_BANDS);
            for (s, e) in r {
                assert!(s >= 1 && e > s && e <= FFT_SIZE / 2);
            }
        }
    }

    #[test]
    fn db_mapping() {
        assert_eq!(amplitude_to_unit(0.0), 0.0);
        assert!((amplitude_to_unit(1.0) - 1.0).abs() < 1e-6);
        let half = amplitude_to_unit(0.0316); // about -30 dB
        assert!((half - 0.5).abs() < 0.01);
    }

    #[test]
    fn smoothing_attacks_fast_releases_slow() {
        let mut p = [0.0; SPECTRUM_BANDS];
        let mut hi = [0.0; SPECTRUM_BANDS];
        hi[3] = 1.0;
        smooth(&mut p, &hi);
        assert!(p[3] > 0.6);
        let before = p[3];
        smooth(&mut p, &[0.0; SPECTRUM_BANDS]);
        assert!(p[3] < before && p[3] > before * 0.8);
    }

    #[test]
    fn sine_lands_in_the_right_band() {
        let sr = 44_100u32;
        let f = 1000.0f32;
        let samples: Vec<f32> = (0..FFT_SIZE)
            .map(|i| (2.0 * std::f32::consts::PI * f * i as f32 / sr as f32).sin() * 0.8)
            .collect();
        let mut a = Analyzer::new();
        let mut bands = [0.0; SPECTRUM_BANDS];
        for _ in 0..10 {
            bands = a.process(&samples, sr);
        }
        let edges = band_edges_hz(sr);
        let expect = (0..SPECTRUM_BANDS)
            .find(|&i| f >= edges[i] && f < edges[i + 1])
            .unwrap();
        let argmax = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert!((argmax as i32 - expect as i32).abs() <= 1, "{argmax} vs {expect}");
        assert!(bands[argmax] > 0.8);
        for _ in 0..200 {
            bands = a.decay();
        }
        assert!(bands.iter().all(|&b| b == 0.0));
    }
}
