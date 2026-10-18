//! Loudness levelling for natively decoded songs.
//!
//! * **Level** (Loud / Normal / Quiet) sets the target loudness, like the
//!   common streaming targets (-11 / -14 / -19 LUFS).
//! * **Normalize** measures each song while it plays (400 ms blocks, gated
//!   like ITU BS.1770, without K-weighting) and moves the gain slowly towards
//!   the target, so one quiet master next to a loud one no longer jumps. A
//!   song measured before starts at its known gain right away.
//! * A soft limiter above -1 dBFS catches peaks, so boosted songs never clip.
//!
//! Embedded players (YouTube, SoundCloud) never pass through here: their
//! audio is not ours to touch.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// Overall loudness target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    Loud,
    #[default]
    Normal,
    Quiet,
}

impl Level {
    fn target_lufs(self) -> f32 {
        match self {
            Level::Loud => -11.0,
            Level::Normal => -14.0,
            Level::Quiet => -19.0,
        }
    }

    /// Fixed offset when songs are not normalized.
    fn fixed_db(self) -> f32 {
        match self {
            Level::Loud => 3.0,
            Level::Normal => 0.0,
            Level::Quiet => -6.0,
        }
    }
}

static NORMALIZE: AtomicBool = AtomicBool::new(true);
static LEVEL: AtomicI32 = AtomicI32::new(1);

/// Applies immediately to everything playing.
pub fn set(level: Level, normalize: bool) {
    LEVEL.store(
        match level {
            Level::Loud => 0,
            Level::Normal => 1,
            Level::Quiet => 2,
        },
        Ordering::Relaxed,
    );
    NORMALIZE.store(normalize, Ordering::Relaxed);
}

fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        0 => Level::Loud,
        2 => Level::Quiet,
        _ => Level::Normal,
    }
}

const UNKNOWN: i32 = i32::MIN;
/// Assumed loudness of a song not measured yet (typical modern master):
/// starts a little quiet rather than too loud.
const DEFAULT_LUFS: f32 = -9.0;
/// Blocks needed before the measurement replaces the assumption.
const MIN_BLOCKS: usize = 8;
/// Gain change speed (dB per second): slow enough not to pump.
const SLEW_DB_S: f32 = 3.0;
const MAX_BOOST_DB: f32 = 9.0;
const MAX_CUT_DB: f32 = -18.0;
/// Soft limiter threshold (linear, about -1 dBFS).
const KNEE: f32 = 0.89;

/// Per-song loudness shared between the engine and the audio thread.
pub struct TrackLoudness {
    /// Integrated loudness measured so far (LUFS × 100).
    measured: AtomicI32,
    /// Known from an earlier play (LUFS × 100).
    known: AtomicI32,
}

impl Default for TrackLoudness {
    fn default() -> Self {
        Self { measured: AtomicI32::new(UNKNOWN), known: AtomicI32::new(UNKNOWN) }
    }
}

impl TrackLoudness {
    pub fn set_known(&self, lufs: f32) {
        self.known.store((lufs * 100.0).round() as i32, Ordering::Relaxed);
    }

    pub fn measured(&self) -> Option<f32> {
        match self.measured.load(Ordering::Relaxed) {
            UNKNOWN => None,
            v => Some(v as f32 / 100.0),
        }
    }

    fn known(&self) -> Option<f32> {
        match self.known.load(Ordering::Relaxed) {
            UNKNOWN => None,
            v => Some(v as f32 / 100.0),
        }
    }
}

/// Measuring + gain state on the audio thread (one per song).
pub struct Leveller {
    channels: usize,
    block_len: usize,
    block_sum: f64,
    block_n: usize,
    /// Mean square of every block above the absolute gate.
    blocks: Vec<f32>,
    integrated_lufs: Option<f32>,
    step_len: usize,
    step_n: usize,
    samples_per_s: f32,
    gain_db: f32,
    gain: f32,
    gain_target: f32,
    gain_delta: f32,
    gain_remaining: usize,
    started: bool,
    bypass: bool,
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn lufs_of(mean_sq: f64, channels: usize) -> f32 {
    (-0.691 + 10.0 * (mean_sq * channels as f64).max(1e-12).log10()) as f32
}

/// Gated integrated loudness of the blocks (absolute -70, relative -10 LU).
fn integrated(blocks: &[f32], channels: usize) -> Option<f32> {
    if blocks.is_empty() {
        return None;
    }
    let mean = |it: &mut dyn Iterator<Item = f32>| {
        let (s, n) = it.fold((0f64, 0usize), |(s, n), v| (s + v as f64, n + 1));
        (n > 0).then(|| s / n as f64)
    };
    // A -10 LU gate is exactly one tenth of the ungated linear energy.
    // Avoid evaluating log10 for every historical block in the audio callback.
    let gate = mean(&mut blocks.iter().copied())? * 0.1;
    let kept = mean(&mut blocks.iter().copied().filter(|&b| b as f64 >= gate))?;
    Some(lufs_of(kept, channels))
}

/// Soft knee limiter: linear below `KNEE`, smoothly saturating above.
pub fn limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        let room = 1.0 - KNEE;
        x.signum() * (KNEE + room * ((a - KNEE) / room).tanh())
    }
}

impl Leveller {
    pub fn new(rate: u32, channels: u16) -> Self {
        let ch = channels.max(1) as usize;
        let sps = rate.max(1) as f32 * ch as f32;
        Self {
            channels: ch,
            block_len: ((sps * 0.4) as usize).max(1),
            block_sum: 0.0,
            block_n: 0,
            // Reserve an hour of 400 ms blocks outside the audio callback.
            blocks: Vec::with_capacity(9000),
            integrated_lufs: None,
            step_len: 512,
            step_n: 0,
            samples_per_s: sps,
            gain_db: 0.0,
            gain: 1.0,
            gain_target: 1.0,
            gain_delta: 0.0,
            gain_remaining: 0,
            started: false,
            bypass: false,
        }
    }

    /// Passes samples through untouched (tests of the decode path).
    pub fn bypass() -> Self {
        Self { bypass: true, ..Self::new(48_000, 2) }
    }

    fn desired_db(&self, shared: &TrackLoudness) -> f32 {
        let lvl = level();
        if !NORMALIZE.load(Ordering::Relaxed) {
            return lvl.fixed_db();
        }
        // Only a newly completed measurement block changes this estimate.
        // Re-scanning the entire song every 512 samples creates callback spikes.
        let est = self.integrated_lufs.unwrap_or_else(|| shared.known().unwrap_or(DEFAULT_LUFS));
        (lvl.target_lufs() - est).clamp(MAX_CUT_DB, MAX_BOOST_DB)
    }

    /// One sample in, levelled and limited sample out.
    #[inline]
    pub fn process(&mut self, s: f32, shared: &TrackLoudness) -> f32 {
        if self.bypass {
            return s;
        }
        if !self.started {
            // Start right at the expected gain (no ramp from 0 dB).
            self.gain_db = self.desired_db(shared);
            self.gain = db_to_lin(self.gain_db);
            self.gain_target = self.gain;
            self.started = true;
        }
        self.block_sum += (s * s) as f64;
        self.block_n += 1;
        if self.block_n >= self.block_len {
            let ms = self.block_sum / self.block_n as f64;
            if lufs_of(ms, self.channels) > -70.0 {
                self.blocks.push(ms as f32);
                if self.blocks.len() >= MIN_BLOCKS {
                    if let Some(l) = integrated(&self.blocks, self.channels) {
                        self.integrated_lufs = Some(l);
                        shared.measured.store((l * 100.0).round() as i32, Ordering::Relaxed);
                    }
                }
            }
            self.block_sum = 0.0;
            self.block_n = 0;
        }
        self.step_n += 1;
        if self.step_n >= self.step_len {
            self.step_n = 0;
            let want = self.desired_db(shared);
            let max_step = SLEW_DB_S * self.step_len as f32 / self.samples_per_s;
            let d = (want - self.gain_db).clamp(-max_step, max_step);
            if d != 0.0 {
                self.gain_db += d;
                self.gain_target = db_to_lin(self.gain_db);
                self.gain_delta = (self.gain_target - self.gain) / self.step_len as f32;
                self.gain_remaining = self.step_len;
            }
        }
        // Interpolate gain instead of a small discontinuity every 512 samples.
        if self.gain_remaining > 0 {
            self.gain += self.gain_delta;
            self.gain_remaining -= 1;
            if self.gain_remaining == 0 { self.gain = self.gain_target; }
        }
        limit(s * self.gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_is_transparent_below_knee_and_bounded_above() {
        assert_eq!(limit(0.5), 0.5);
        assert_eq!(limit(-0.8), -0.8);
        assert!(limit(3.0) <= 1.0 && limit(3.0) > KNEE);
        assert!(limit(-3.0) >= -1.0);
    }

    #[test]
    fn loud_and_quiet_songs_end_up_close() {
        // A loud and a quiet sine; after a while both reach about the same level.
        let run = |amp: f32| -> f32 {
            let shared = TrackLoudness::default();
            let mut l = Leveller::new(8000, 1);
            let mut out_sq = 0f64;
            let n = 8000 * 30;
            for i in 0..n {
                let s = amp * (i as f32 * 0.05).sin();
                let y = l.process(s, &shared);
                if i > n - 8000 {
                    out_sq += (y * y) as f64;
                }
            }
            lufs_of(out_sq / 8000.0, 1)
        };
        set(Level::Normal, true);
        let (loud, quiet) = (run(0.9), run(0.1));
        assert!((loud - quiet).abs() < 1.5, "loud {loud} vs quiet {quiet}");
        assert!((loud - -14.0).abs() < 2.0, "target missed: {loud}");
    }
}

#[cfg(test)]
mod callback_regression_tests {
    use super::*;
    #[test]
    fn linear_gate_matches_original_decibel_gate() {
        let blocks = [1e-7, 1e-5, 0.001, 0.04, 0.5];
        let mean = blocks.iter().map(|&b| b as f64).sum::<f64>() / blocks.len() as f64;
        for channels in [1, 2, 6] {
            let gate = lufs_of(mean, channels) - 10.0;
            let kept: Vec<_> = blocks.iter().copied().filter(|&b| lufs_of(b as f64, channels) >= gate).collect();
            let expected = lufs_of(kept.iter().map(|&b| b as f64).sum::<f64>() / kept.len() as f64, channels);
            assert!((integrated(&blocks, channels).unwrap() - expected).abs() < 1e-5);
        }
    }
    #[test]
    fn normalization_gain_changes_have_no_sample_step() {
        let mut processor = Leveller::new(48_000, 2);
        processor.started = true;
        processor.gain = 0.5;
        processor.gain_target = 0.6;
        processor.gain_delta = 0.001;
        processor.gain_remaining = 100;
        let shared = TrackLoudness::default();
        let mut previous = 0.05;
        for _ in 0..100 {
            let sample = processor.process(0.1, &shared);
            assert!((sample - previous).abs() <= 0.00011);
            previous = sample;
        }
        assert!((previous - 0.06).abs() < 1e-6);
    }
}
