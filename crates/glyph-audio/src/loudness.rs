//! Loudness normalization for natively decoded songs, the way Spotify and
//! SoundCloud do it.
//!
//! * **One gain per song.** Each song gets one fixed gain for its whole
//!   length, from its integrated loudness (ITU BS.1770: K-weighted, gated).
//!   The gain never follows what is playing right now, so a loud chorus, a
//!   scream or a bass drop does not turn the rest of the song down.
//! * **Measured once.** [`analyze`] decodes the whole song in the background
//!   and remembers the result on disk ([`set_store`]). Until it is known a
//!   song starts at a careful guess and moves to its real gain once.
//! * **Level** (Loud / Normal / Quiet) sets the target (-11 / -14 / -19
//!   LUFS). Normal and Quiet never boost a song further than its loudest
//!   peak allows (-1 dBFS), like Spotify; Loud boosts and lets the limiter
//!   catch the peaks.
//! * **Equalizer after the gain.** The equalizer (bass boost included) runs
//!   after the gain and never changes it. It gets the room the song has
//!   left below the top ([`Leveller::room_db`]), so a boost is really heard:
//!   a loud song turned down by levelling keeps its bass boost instead of
//!   the equalizer turning everything down again to make room.
//! * **Peaks only.** What still goes over -1 dBFS (equalizer boosts, Loud)
//!   is caught by a look-ahead limiter in two parts: a bass peak turns only
//!   the bass down for a moment, any other peak turns everything down for a
//!   few milliseconds. Neither touches the rest of the song.
//!
//! Embedded players (YouTube, SoundCloud) never pass through here: their
//! audio is not ours to touch.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex};

/// Overall loudness target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    Loud,
    #[default]
    Normal,
    Quiet,
}

impl Level {
    pub fn target_lufs(self) -> f32 {
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
/// Bumped by [`set`]: playing songs move to the new gain.
static SETTINGS_GEN: AtomicU32 = AtomicU32::new(0);

/// Applies to everything playing (a short ramp, no jump).
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
    SETTINGS_GEN.fetch_add(1, Ordering::Relaxed);
}

fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        0 => Level::Loud,
        2 => Level::Quiet,
        _ => Level::Normal,
    }
}

/// Highest level a sample may reach (-1 dBFS).
pub const CEILING_DB: f32 = -1.0;
const CEILING: f32 = 0.891_250_9;
const MAX_BOOST_DB: f32 = 12.0;
const MAX_CUT_DB: f32 = -24.0;
/// Assumed loudness of a song not measured yet (a typical modern master):
/// it starts a little quiet rather than too loud.
const GUESS_LUFS: f32 = -9.0;
/// Without a measurement from [`analyze`], the song itself is measured this
/// long; then its gain is set once and stays.
const LIVE_SECONDS: f32 = 15.0;
/// How long a gain change takes (the one move to the measured gain, or a
/// changed setting).
const RAMP_SECONDS: f32 = 1.0;
/// Samples between two looks at the wanted gain.
const STEP: usize = 512;

/// Loudness of one whole song.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Analysis {
    /// Integrated loudness (LUFS).
    pub lufs: f32,
    /// Loudest sample (dBFS).
    pub peak_db: f32,
}

/// The gain a song gets (dB): Spotify's rules. Loud may push peaks into the
/// limiter; Normal and Quiet stop boosting where the loudest peak would
/// pass -1 dBFS.
pub fn gain_db_for(level: Level, a: Analysis) -> f32 {
    let mut g = level.target_lufs() - a.lufs;
    if level != Level::Loud && g > 0.0 {
        g = g.min((CEILING_DB - a.peak_db).max(0.0));
    }
    g.clamp(MAX_CUT_DB, MAX_BOOST_DB)
}

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn lin_to_db(v: f32) -> f32 {
    20.0 * v.max(1e-9).log10()
}

// ---- K-weighted loudness meter (ITU BS.1770-4) ----

#[derive(Clone, Copy, Default)]
struct Coef {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

#[derive(Clone, Copy, Default)]
struct Mem {
    z1: f64,
    z2: f64,
}

impl Coef {
    #[inline]
    fn run(&self, m: &mut Mem, x: f64) -> f64 {
        let y = self.b0 * x + m.z1;
        m.z1 = self.b1 * x - self.a1 * y + m.z2;
        m.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// The two K-weighting filters (a high shelf around 1.7 kHz, then a high
/// pass at 38 Hz) for any sample rate, as libebur128 designs them. The
/// high pass is why bass counts less than the ear's middle range.
fn k_weighting(rate: f64) -> [Coef; 2] {
    let shelf = {
        let f0 = 1681.974_450_955_533;
        let g = 3.999_843_853_973_347;
        let q = 0.707_175_236_955_419_6;
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.499_666_774_154_541_6);
        let a0 = 1.0 + k / q + k * k;
        Coef {
            b0: (vh + vb * k / q + k * k) / a0,
            b1: 2.0 * (k * k - vh) / a0,
            b2: (vh - vb * k / q + k * k) / a0,
            a1: 2.0 * (k * k - 1.0) / a0,
            a2: (1.0 - k / q + k * k) / a0,
        }
    };
    let high_pass = {
        let f0 = 38.135_470_876_024_44;
        let q = 0.500_327_037_323_877_3;
        let k = (std::f64::consts::PI * f0 / rate).tan();
        let a0 = 1.0 + k / q + k * k;
        Coef { b0: 1.0, b1: -2.0, b2: 1.0, a1: 2.0 * (k * k - 1.0) / a0, a2: (1.0 - k / q + k * k) / a0 }
    };
    [shelf, high_pass]
}

/// Weight of each channel. A mono song plays on both speakers, so it
/// counts twice (dual mono); in 5.1 the LFE does not count and the
/// surrounds count +1.5 dB.
fn channel_weights(channels: usize) -> Vec<f64> {
    match channels {
        1 => vec![2.0],
        6 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41],
        n => (0..n).map(|i| if i < 3 { 1.0 } else { 1.41 }).collect(),
    }
}

fn lufs_of(power: f64) -> f32 {
    (-0.691 + 10.0 * power.max(1e-20).log10()) as f32
}

/// Mean weighted power of a block at the absolute gate (-70 LUFS).
const ABS_GATE: f64 = 1.170_8e-7; // 10^((-70 + 0.691) / 10)

/// Measures K-weighted, gated loudness: 400 ms blocks every 100 ms.
pub struct Meter {
    channels: usize,
    weights: Vec<f64>,
    coefs: [Coef; 2],
    mem: Vec<[Mem; 2]>,
    ch: usize,
    frame: f64,
    sub_len: usize,
    sub_n: usize,
    sub_sum: f64,
    subs: [f64; 4],
    subs_seen: usize,
    /// Power of every 400 ms block above the absolute gate.
    blocks: Vec<f64>,
    peak: f32,
}

impl Meter {
    pub fn new(rate: u32, channels: u16) -> Self {
        let ch = channels.max(1) as usize;
        let rate = rate.max(1);
        Self {
            channels: ch,
            weights: channel_weights(ch),
            coefs: k_weighting(rate as f64),
            mem: vec![[Mem::default(); 2]; ch],
            ch: 0,
            frame: 0.0,
            sub_len: (rate as usize / 10).max(1),
            sub_n: 0,
            sub_sum: 0.0,
            subs: [0.0; 4],
            subs_seen: 0,
            blocks: Vec::with_capacity(LIVE_SECONDS as usize * 10 + 16),
            peak: 0.0,
        }
    }

    /// One interleaved sample.
    #[inline]
    pub fn push(&mut self, s: f32) {
        self.peak = self.peak.max(s.abs());
        let c = self.ch;
        let [a, b] = &mut self.mem[c];
        let y = self.coefs[1].run(b, self.coefs[0].run(a, s as f64));
        self.frame += self.weights[c] * y * y;
        self.ch += 1;
        if self.ch < self.channels {
            return;
        }
        self.ch = 0;
        self.sub_sum += self.frame;
        self.frame = 0.0;
        self.sub_n += 1;
        if self.sub_n < self.sub_len {
            return;
        }
        self.subs[self.subs_seen % 4] = self.sub_sum / self.sub_len as f64;
        self.subs_seen += 1;
        self.sub_sum = 0.0;
        self.sub_n = 0;
        if self.subs_seen >= 4 {
            let p = self.subs.iter().sum::<f64>() / 4.0;
            if p > ABS_GATE {
                self.blocks.push(p);
            }
        }
    }

    /// Gated integrated loudness so far (absolute -70, relative -10 LU).
    pub fn integrated(&self) -> Option<f32> {
        if self.blocks.is_empty() {
            return None;
        }
        let mean = self.blocks.iter().sum::<f64>() / self.blocks.len() as f64;
        // -10 LU is a tenth of the power.
        let gate = mean * 0.1;
        let (sum, n) = self.blocks.iter().filter(|&&b| b >= gate).fold((0.0, 0usize), |(s, n), b| (s + b, n + 1));
        (n > 0).then(|| lufs_of(sum / n as f64))
    }

    /// Loudest sample so far (dBFS).
    pub fn peak_db(&self) -> f32 {
        lin_to_db(self.peak)
    }

    pub fn result(&self) -> Option<Analysis> {
        self.integrated().map(|lufs| Analysis { lufs, peak_db: self.peak_db() })
    }
}

// ---- per-song state shared between engine and audio thread ----

const UNKNOWN: i32 = i32::MIN;

/// Loudness of one song, handed to its audio thread once it is known.
pub struct TrackLoudness {
    /// Integrated loudness (LUFS × 100).
    lufs: AtomicI32,
    /// Loudest sample (dBFS × 100).
    peak: AtomicI32,
}

impl Default for TrackLoudness {
    fn default() -> Self {
        Self { lufs: AtomicI32::new(UNKNOWN), peak: AtomicI32::new(UNKNOWN) }
    }
}

impl TrackLoudness {
    pub fn set_known(&self, a: Analysis) {
        self.peak.store((a.peak_db * 100.0).round() as i32, Ordering::Relaxed);
        self.lufs.store((a.lufs * 100.0).round() as i32, Ordering::Release);
    }

    pub fn has_known(&self) -> bool {
        self.lufs.load(Ordering::Relaxed) != UNKNOWN
    }

    pub fn known(&self) -> Option<Analysis> {
        match self.lufs.load(Ordering::Acquire) {
            UNKNOWN => None,
            l => Some(Analysis { lufs: l as f32 / 100.0, peak_db: self.peak.load(Ordering::Relaxed) as f32 / 100.0 }),
        }
    }
}

// ---- gain + limiter on the audio thread ----

/// One song's gain and peak limiter (audio thread).
///
/// [`Leveller::gain`] runs on the decoded sample (before the equalizer),
/// [`Leveller::limit`] on the equalized one.
pub struct Leveller {
    channels: usize,
    rate: f32,
    /// Measures the song itself while nothing better is known.
    meter: Option<Meter>,
    live: Option<Analysis>,
    seen: usize,
    live_after: usize,
    step_n: usize,
    gen: u32,
    started: bool,
    gain: f32,
    target_db: f32,
    ramp_step: f32,
    ramp_left: usize,
    limiter: Limiter,
    /// Room below -1 dBFS at the current gain (dB, see [`Leveller::room_db`]).
    room: f32,
    bypass: bool,
}

impl Leveller {
    pub fn new(rate: u32, channels: u16) -> Self {
        let ch = channels.max(1) as usize;
        let rate_f = rate.max(1) as f32;
        Self {
            channels: ch,
            rate: rate_f,
            meter: Some(Meter::new(rate, channels)),
            live: None,
            seen: 0,
            live_after: (LIVE_SECONDS * rate_f) as usize * ch,
            step_n: 0,
            gen: 0,
            started: false,
            gain: 1.0,
            target_db: 0.0,
            ramp_step: 0.0,
            ramp_left: 0,
            limiter: Limiter::new(rate, channels),
            room: 0.0,
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
        let a = shared.known().or(self.live).unwrap_or(Analysis { lufs: GUESS_LUFS, peak_db: 0.0 });
        gain_db_for(lvl, a)
    }

    /// The song's gain (the current one while it moves).
    pub fn current_gain(&self) -> f32 {
        self.gain
    }

    /// How far the song's loudest peak stays under -1 dBFS at its gain
    /// (dB, never below 0). The equalizer spends this room on boosts first.
    /// Without a measurement the peak counts as 0 dBFS.
    pub fn room_db(&self) -> f32 {
        self.room
    }

    fn update_room(&mut self, shared: &TrackLoudness) {
        let peak = shared.known().or(self.live).map_or(0.0, |a| a.peak_db.min(0.0));
        // While the gain moves, the larger of now and where it goes.
        let g = self.target_db.max(lin_to_db(self.gain));
        self.room = (CEILING_DB - (peak + g)).max(0.0);
    }

    /// One decoded sample in, the same sample at the song's gain out.
    #[inline]
    pub fn gain(&mut self, s: f32, shared: &TrackLoudness) -> f32 {
        if self.bypass {
            return s;
        }
        if !self.started {
            // Start right at the expected gain (no ramp from 0 dB).
            self.started = true;
            self.gen = SETTINGS_GEN.load(Ordering::Relaxed);
            self.target_db = self.desired_db(shared);
            self.gain = db_to_lin(self.target_db);
            self.update_room(shared);
        }
        if let Some(m) = self.meter.as_mut() {
            m.push(s);
            self.seen += 1;
        }
        self.step_n += 1;
        if self.step_n >= STEP {
            self.step_n = 0;
            self.check(shared);
        }
        if self.ramp_left > 0 {
            self.ramp_left -= 1;
            self.gain = if self.ramp_left == 0 { db_to_lin(self.target_db) } else { self.gain + self.ramp_step };
        }
        s * self.gain
    }

    /// Every [`STEP`] samples: has something new decided the song's gain?
    /// Only three things move it: the measured loudness arriving, the
    /// song's own measurement once [`LIVE_SECONDS`] have played without one,
    /// or a changed setting. Never the music itself.
    fn check(&mut self, shared: &TrackLoudness) {
        if self.meter.is_some() {
            if shared.has_known() {
                self.meter = None;
            } else if self.seen >= self.live_after {
                self.live = self.meter.take().and_then(|m| m.result());
            }
        }
        let gen = SETTINGS_GEN.load(Ordering::Relaxed);
        let want = self.desired_db(shared);
        self.update_room(shared);
        if (want - self.target_db).abs() < 0.05 && gen == self.gen {
            return;
        }
        self.gen = gen;
        self.target_db = want;
        let n = ((RAMP_SECONDS * self.rate) as usize * self.channels).max(1);
        self.ramp_step = (db_to_lin(want) - self.gain) / n as f32;
        self.ramp_left = n;
    }

    /// One equalized sample in, the same sample with its peaks caught out
    /// (a few milliseconds later).
    #[inline]
    pub fn limit(&mut self, s: f32) -> f32 {
        if self.bypass {
            return s;
        }
        self.limiter.push(s)
    }
}

/// Smallest wanted gain over a window, released slowly and smoothed.
///
/// Gains requested for each frame go through a sliding minimum over `len`
/// frames, a release (falls at once, rises slowly) and a moving average of
/// `len` frames. With the signal delayed by `len - 1` frames, the gain has
/// reached what a peak needs exactly when that peak comes out, and it moved
/// there smoothly.
struct Smoother {
    len: usize,
    window: VecDeque<(u64, f32)>,
    n: u64,
    held: f32,
    release: f32,
    avg: Vec<f32>,
    avg_i: usize,
    avg_sum: f64,
}

impl Smoother {
    fn new(len: usize, release_frames: f32) -> Self {
        let len = len.max(1);
        Self {
            len,
            window: VecDeque::with_capacity(len + 1),
            n: 0,
            held: 1.0,
            release: 1.0 - (-1.0 / release_frames.max(1.0)).exp(),
            avg: vec![1.0; len],
            avg_i: 0,
            avg_sum: len as f64,
        }
    }

    #[inline]
    fn push(&mut self, want: f32) -> f32 {
        while self.window.back().is_some_and(|&(_, v)| v >= want) {
            self.window.pop_back();
        }
        self.window.push_back((self.n, want));
        while self.window.front().is_some_and(|&(i, _)| i + self.len as u64 <= self.n) {
            self.window.pop_front();
        }
        self.n += 1;
        let min = self.window.front().map_or(1.0, |&(_, v)| v);
        self.held = if min < self.held { min } else { self.held + (min - self.held) * self.release };
        let old = std::mem::replace(&mut self.avg[self.avg_i], self.held);
        self.avg_i = (self.avg_i + 1) % self.len;
        self.avg_sum += (self.held - old) as f64;
        (self.avg_sum / self.len as f64).min(1.0) as f32
    }
}

/// Where the bass ends for the limiter (Hz).
const CROSSOVER_HZ: f32 = 150.0;
/// The bass is never turned down more than this (-15 dB).
const BASS_FLOOR: f32 = 0.178;
const BASS_LOOKAHEAD_S: f32 = 0.010;
const BASS_RELEASE_S: f32 = 0.250;
const FULL_LOOKAHEAD_S: f32 = 0.005;
const FULL_RELEASE_S: f32 = 0.120;

#[derive(Clone, Copy, Default)]
struct Bq {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Clone, Copy, Default)]
struct BqMem {
    z1: f32,
    z2: f32,
}

impl Bq {
    /// Second order Butterworth (RBJ, Q = 1/√2); two in a row make one half
    /// of a Linkwitz-Riley crossover, whose halves add up flat.
    fn butterworth(high: bool, f: f32, rate: f32) -> Self {
        let w = 2.0 * std::f32::consts::PI * (f / rate).min(0.45);
        let (s, c) = w.sin_cos();
        let alpha = s / std::f32::consts::SQRT_2;
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = if high { ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0) } else { ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0) };
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: -2.0 * c / a0, a2: (1.0 - alpha) / a0 }
    }

    #[inline]
    fn run(&self, m: &mut BqMem, x: f32) -> f32 {
        let y = self.b0 * x + m.z1;
        m.z1 = self.b1 * x - self.a1 * y + m.z2;
        m.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Look-ahead peak limiter in two parts, both linked over all channels:
///
/// 1. **Bass.** The signal is split at [`CROSSOVER_HZ`]. Where a peak over
///    the ceiling is mostly bass, only the bass is turned down, just enough
///    and only for that moment; voice and everything above stay as they are.
/// 2. **Everything.** Whatever is still over the ceiling turns the whole
///    signal down for a few milliseconds.
///
/// Below the ceiling the signal passes unchanged (the crossover adds up flat).
struct Limiter {
    channels: usize,
    ch: usize,
    lp: Bq,
    hp: Bq,
    mem: Vec<[BqMem; 4]>,
    lo: Vec<f32>,
    hi: Vec<f32>,
    bass: Smoother,
    bass_len: usize,
    bass_delay: Vec<f32>,
    bass_pos: usize,
    full: Smoother,
    full_len: usize,
    full_delay: Vec<f32>,
    full_pos: usize,
    mid: Vec<f32>,
    out: Vec<f32>,
}

impl Limiter {
    fn new(rate: u32, channels: u16) -> Self {
        let ch = channels.max(1) as usize;
        let r = rate.max(1) as f32;
        let bass_len = ((BASS_LOOKAHEAD_S * r) as usize).max(2);
        let full_len = ((FULL_LOOKAHEAD_S * r) as usize).max(2);
        Self {
            channels: ch,
            ch: 0,
            lp: Bq::butterworth(false, CROSSOVER_HZ, r),
            hp: Bq::butterworth(true, CROSSOVER_HZ, r),
            mem: vec![[BqMem::default(); 4]; ch],
            lo: vec![0.0; ch],
            hi: vec![0.0; ch],
            bass: Smoother::new(bass_len, BASS_RELEASE_S * r),
            bass_len,
            bass_delay: vec![0.0; bass_len * ch * 2],
            bass_pos: 0,
            full: Smoother::new(full_len, FULL_RELEASE_S * r),
            full_len,
            full_delay: vec![0.0; full_len * ch],
            full_pos: 0,
            mid: vec![0.0; ch],
            out: vec![0.0; ch],
        }
    }

    #[inline]
    fn push(&mut self, s: f32) -> f32 {
        let c = self.ch;
        let [l1, l2, h1, h2] = &mut self.mem[c];
        let lo = self.lp.run(l1, s);
        self.lo[c] = self.lp.run(l2, lo);
        let hi = self.hp.run(h1, s);
        self.hi[c] = self.hp.run(h2, hi);
        let y = self.out[c];
        self.ch += 1;
        if self.ch == self.channels {
            self.ch = 0;
            self.frame();
        }
        y
    }

    fn frame(&mut self) {
        let ch = self.channels;
        // 1. Bass: how far must the bass go down for this frame?
        let mut want = 1.0f32;
        for c in 0..ch {
            let (lo, hi) = (self.lo[c], self.hi[c]);
            let y = lo + hi;
            if y.abs() > CEILING && lo.abs() > hi.abs() {
                want = want.min(((CEILING.copysign(y) - hi) / lo).clamp(BASS_FLOOR, 1.0));
            }
        }
        let g = self.bass.push(want);
        let w = self.bass_pos * ch * 2;
        for c in 0..ch {
            self.bass_delay[w + 2 * c] = self.lo[c];
            self.bass_delay[w + 2 * c + 1] = self.hi[c];
        }
        self.bass_pos = (self.bass_pos + 1) % self.bass_len;
        let r = self.bass_pos * ch * 2;
        // 2. Everything: whatever is still over the ceiling.
        let mut want = 1.0f32;
        for c in 0..ch {
            let v = g * self.bass_delay[r + 2 * c] + self.bass_delay[r + 2 * c + 1];
            self.mid[c] = v;
            if v.abs() > CEILING {
                want = want.min(CEILING / v.abs());
            }
        }
        let g = self.full.push(want);
        let w = self.full_pos * ch;
        self.full_delay[w..w + ch].copy_from_slice(&self.mid);
        self.full_pos = (self.full_pos + 1) % self.full_len;
        let r = self.full_pos * ch;
        for c in 0..ch {
            self.out[c] = (g * self.full_delay[r + c]).clamp(-1.0, 1.0);
        }
    }
}

// ---- measuring whole songs in the background, remembered on disk ----

struct Store {
    path: Option<PathBuf>,
    known: HashMap<String, Analysis>,
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store { path: None, known: HashMap::new() }));
static RUNNING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
static WANTED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Longest stretch of a song that is measured (a three-hour mix is known
/// well enough after its first quarter of an hour).
const ANALYZE_MAX_S: f64 = 15.0 * 60.0;

/// Where measured songs are remembered (one line each: LUFS, peak, key).
/// Loads what is there.
pub fn set_store(path: PathBuf) {
    let mut known = HashMap::new();
    let mut lines = 0usize;
    if let Ok(text) = std::fs::read_to_string(&path) {
        for line in text.lines() {
            let mut it = line.splitn(3, '\t');
            let (Some(l), Some(p), Some(k)) = (it.next(), it.next(), it.next()) else { continue };
            if let (Ok(lufs), Ok(peak_db)) = (l.parse::<f32>(), p.parse::<f32>()) {
                if lufs.is_finite() && peak_db.is_finite() && !k.is_empty() {
                    known.insert(k.to_string(), Analysis { lufs, peak_db });
                    lines += 1;
                }
            }
        }
    }
    // Lines are only added: rewrite the file when it holds many old ones.
    if lines > known.len() * 2 + 64 {
        let body: String = known.iter().map(|(k, a)| format!("{:.2}\t{:.2}\t{k}\n", a.lufs, a.peak_db)).collect();
        let _ = std::fs::write(&path, body);
    }
    let mut s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    s.known.extend(known);
    s.path = Some(path);
}

/// A song measured before.
pub fn lookup(key: &str) -> Option<Analysis> {
    STORE.lock().unwrap_or_else(|e| e.into_inner()).known.get(key).copied()
}

pub fn remember(key: &str, a: Analysis) {
    if key.contains(['\n', '\t']) || !a.lufs.is_finite() || !a.peak_db.is_finite() {
        return;
    }
    let mut s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    if s.known.insert(key.to_string(), a) == Some(a) {
        return;
    }
    if let Some(path) = &s.path {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{:.2}\t{:.2}\t{key}", a.lufs, a.peak_db);
        }
    }
}

/// The songs worth measuring now (the current and the next one); a
/// measurement of any other song stops.
pub fn set_wanted(keys: Vec<String>) {
    *WANTED.lock().unwrap_or_else(|e| e.into_inner()) = keys;
}

fn wanted(key: &str) -> bool {
    WANTED.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|k| k == key)
}

/// Measures the song at `url` in the background (once; nothing happens when
/// it is known or already being measured). The result lands in the store,
/// where the engine picks it up.
pub fn analyze(key: String, url: String, mime: Option<String>) {
    if lookup(&key).is_some() || !RUNNING.lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone()) {
        return;
    }
    let spawned = std::thread::Builder::new().name("glyph-loudness".into()).spawn({
        let key = key.clone();
        move || {
            let res = crate::native::measure(&url, mime.as_deref(), ANALYZE_MAX_S, &|| wanted(&key));
            match res {
                Ok(Some(a)) => {
                    tracing::debug!(lufs = a.lufs, peak = a.peak_db, "song loudness measured");
                    remember(&key, a);
                }
                Ok(None) => {}
                Err(e) => tracing::info!("song loudness not measured: {e}"),
            }
            RUNNING.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
        }
    });
    if spawned.is_err() {
        RUNNING.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::f32::consts::TAU;

    /// Settings are global: tests that play songs run one at a time.
    pub(crate) static SERIAL: Mutex<()> = Mutex::new(());

    fn sine(rate: u32, f: f32, amp: f32, secs: f32) -> impl Iterator<Item = f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n).map(move |i| amp * (TAU * f * i as f32 / rate as f32).sin())
    }

    fn measure_stereo(rate: u32, left: &[f32], right: &[f32]) -> f32 {
        let mut m = Meter::new(rate, 2);
        for (l, r) in left.iter().zip(right) {
            m.push(*l);
            m.push(*r);
        }
        m.integrated().unwrap()
    }

    /// Loudness of interleaved stereo (no gating needed for steady tones).
    fn lufs_stereo(rate: u32, interleaved: &[f32]) -> f32 {
        let mut m = Meter::new(rate, 2);
        for s in interleaved {
            m.push(*s);
        }
        m.integrated().unwrap()
    }

    #[test]
    fn full_scale_sine_in_one_channel_is_minus_3_lufs() {
        // BS.1770's reference: a 0 dBFS 1 kHz sine in one front channel.
        let rate = 48_000;
        let l: Vec<f32> = sine(rate, 997.0, 1.0, 5.0).collect();
        let r = vec![0.0; l.len()];
        let lufs = measure_stereo(rate, &l, &r);
        assert!((lufs - -3.01).abs() < 0.1, "{lufs}");
        // Also at 44.1 kHz.
        let l: Vec<f32> = sine(44_100, 997.0, 1.0, 5.0).collect();
        let lufs = measure_stereo(44_100, &l, &vec![0.0; l.len()]);
        assert!((lufs - -3.01).abs() < 0.1, "{lufs}");
    }

    #[test]
    fn bass_counts_less_than_the_middle() {
        // The same level at 30 Hz reads clearly quieter than at 1 kHz (the
        // old meter counted bass fully, so bassy songs were turned down).
        let rate = 48_000;
        let mid: Vec<f32> = sine(rate, 1000.0, 0.5, 4.0).collect();
        let bass: Vec<f32> = sine(rate, 30.0, 0.5, 4.0).collect();
        let (m, b) = (measure_stereo(rate, &mid, &mid), measure_stereo(rate, &bass, &bass));
        assert!(b < m - 4.0, "bass {b} vs mid {m}");
    }

    #[test]
    fn silence_and_quiet_parts_do_not_count() {
        // Gating: a long silence or a very quiet intro does not pull a
        // song's loudness down.
        let rate = 48_000;
        let loud: Vec<f32> = sine(rate, 1000.0, 0.5, 6.0).collect();
        let mut with_gaps = vec![0.0f32; rate as usize * 10];
        with_gaps.extend(sine(rate, 1000.0, 0.001, 10.0));
        with_gaps.extend(&loud);
        let a = measure_stereo(rate, &loud, &loud);
        let b = measure_stereo(rate, &with_gaps, &with_gaps);
        assert!((a - b).abs() < 0.2, "{a} vs {b}");
    }

    #[test]
    fn mono_counts_like_the_same_song_on_both_speakers() {
        let rate = 48_000;
        let s: Vec<f32> = sine(rate, 1000.0, 0.3, 4.0).collect();
        let mut m = Meter::new(rate, 1);
        s.iter().for_each(|v| m.push(*v));
        let stereo = measure_stereo(rate, &s, &s);
        assert!((m.integrated().unwrap() - stereo).abs() < 0.05);
    }

    #[test]
    fn spotify_rules_for_the_gain() {
        let loud_master = Analysis { lufs: -6.0, peak_db: -0.1 };
        let quiet_master = Analysis { lufs: -20.0, peak_db: -9.0 };
        let quiet_peaky = Analysis { lufs: -20.0, peak_db: -2.0 };
        // Loud songs are turned down to the target.
        assert_eq!(gain_db_for(Level::Normal, loud_master), -8.0);
        assert_eq!(gain_db_for(Level::Quiet, loud_master), -13.0);
        assert_eq!(gain_db_for(Level::Loud, loud_master), -5.0);
        // Quiet songs are boosted, but in Normal and Quiet only until the
        // loudest peak touches -1 dBFS.
        assert_eq!(gain_db_for(Level::Normal, quiet_master), 6.0);
        assert_eq!(gain_db_for(Level::Normal, quiet_peaky), 1.0);
        assert_eq!(gain_db_for(Level::Quiet, quiet_peaky), 1.0);
        // Loud boosts all the way; the limiter catches the peaks.
        assert_eq!(gain_db_for(Level::Loud, quiet_peaky), 9.0);
        // Never more than the limits.
        assert_eq!(gain_db_for(Level::Loud, Analysis { lufs: -40.0, peak_db: -30.0 }), MAX_BOOST_DB);
        assert_eq!(gain_db_for(Level::Quiet, Analysis { lufs: 6.0, peak_db: 0.0 }), MAX_CUT_DB);
    }

    /// Plays interleaved stereo through gain (and limiter); returns the
    /// gain after every second.
    fn gains_per_second(rate: u32, input: &[f32], shared: &TrackLoudness, known_after: Option<(usize, Analysis)>) -> Vec<f32> {
        let mut l = Leveller::new(rate, 2);
        let mut out = vec![];
        for (i, s) in input.iter().enumerate() {
            if let Some((at, a)) = known_after {
                if i == at {
                    shared.set_known(a);
                }
            }
            let y = l.gain(*s, shared);
            let _ = l.limit(y);
            if i % (rate as usize * 2) == 0 {
                out.push(l.current_gain());
            }
        }
        out
    }

    /// A song with a quiet verse, a loud part in the middle (a scream, a
    /// drop) and a quiet end.
    fn verse_drop_verse(rate: u32) -> Vec<f32> {
        let mut v = vec![];
        let part = |f: f32, amp: f32, secs: f32| -> Vec<f32> { sine(rate, f, amp, secs).flat_map(|s| [s, s]).collect() };
        v.extend(part(440.0, 0.1, 20.0));
        v.extend(part(60.0, 0.9, 10.0));
        v.extend(part(440.0, 0.1, 20.0));
        v
    }

    #[test]
    fn a_loud_part_does_not_turn_the_song_down() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set(Level::Normal, true);
        let rate = 8_000;
        let song = verse_drop_verse(rate);
        // Known before it plays: one gain from start to end.
        let shared = TrackLoudness::default();
        shared.set_known(Analysis { lufs: -12.0, peak_db: -0.9 });
        let g = gains_per_second(rate, &song, &shared, None);
        let first = g[0];
        assert!(g.iter().all(|x| (x - first).abs() < 1e-6), "gain moved: {g:?}");
        assert!((lin_to_db(first) - -2.0).abs() < 0.01);
    }

    #[test]
    fn the_measurement_arriving_moves_the_gain_once() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set(Level::Normal, true);
        let rate = 8_000;
        let song = verse_drop_verse(rate);
        let shared = TrackLoudness::default();
        // Arrives two seconds in (stereo: 2 samples per frame).
        let g = gains_per_second(rate, &song, &shared, Some((rate as usize * 4, Analysis { lufs: -20.0, peak_db: -8.0 })));
        // Starts at the guess (-9 LUFS: -5 dB) ...
        assert!((lin_to_db(g[0]) - -5.0).abs() < 0.01, "{g:?}");
        // ... and from second 4 on stays at the real gain (+6 dB), through
        // the loud part and to the end.
        let want = db_to_lin(6.0);
        assert!(g[4..].iter().all(|x| (x - want).abs() < 1e-4), "{g:?}");
    }

    #[test]
    fn without_a_measurement_the_song_sets_its_gain_once() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set(Level::Normal, true);
        let rate = 8_000;
        let song = verse_drop_verse(rate);
        let shared = TrackLoudness::default();
        let g = gains_per_second(rate, &song, &shared, None);
        // The guess until LIVE_SECONDS, then one value that the drop at
        // 20..30 s does not change.
        assert!(g[..15].iter().all(|x| (lin_to_db(*x) - -5.0).abs() < 0.01), "{g:?}");
        let settled = g[17];
        assert!(g[17..].iter().all(|x| (x - settled).abs() < 1e-6), "{g:?}");
    }

    #[test]
    fn loud_and_quiet_songs_end_up_equally_loud() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set(Level::Normal, true);
        let rate = 48_000;
        let play = |amp: f32| -> f32 {
            let input: Vec<f32> = sine(rate, 1000.0, amp, 6.0).flat_map(|s| [s, s]).collect();
            let mut m = Meter::new(rate, 2);
            input.iter().for_each(|s| m.push(*s));
            let shared = TrackLoudness::default();
            shared.set_known(m.result().unwrap());
            let mut l = Leveller::new(rate, 2);
            let out: Vec<f32> = input.iter().map(|s| { let y = l.gain(*s, &shared); l.limit(y) }).collect();
            lufs_stereo(rate, &out[rate as usize..])
        };
        let (loud, quiet) = (play(0.9), play(0.08));
        assert!((loud - -14.0).abs() < 0.3, "loud song at {loud}");
        assert!((quiet - -14.0).abs() < 0.3, "quiet song at {quiet}");
    }

    #[test]
    fn normalize_off_and_level_changes_apply_smoothly() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set(Level::Normal, true);
        let shared = TrackLoudness::default();
        shared.set_known(Analysis { lufs: -10.0, peak_db: -0.5 });
        let rate = 8_000;
        let mut l = Leveller::new(rate, 2);
        let mut last = 0.0f32;
        let mut biggest_step = 0.0f32;
        for i in 0..rate as usize * 2 * 4 {
            if i == rate as usize * 2 {
                set(Level::Normal, false);
            }
            let y = l.gain(0.5, &shared);
            if i > 0 {
                biggest_step = biggest_step.max((y - last).abs());
            }
            last = y;
        }
        set(Level::Normal, true);
        assert!((l.current_gain() - 1.0).abs() < 1e-4, "{}", l.current_gain());
        assert!(biggest_step < 0.001, "jump of {biggest_step}");
    }

    #[test]
    fn limiter_passes_quiet_audio_and_never_lets_peaks_through() {
        let rate = 48_000;
        let mut lim = Limiter::new(rate, 2);
        // Quiet music: the same loudness out (the crossover adds up flat).
        let quiet: Vec<f32> = (0..rate as usize * 2).flat_map(|i| {
            let t = i as f32 / rate as f32;
            let s = 0.3 * (TAU * 80.0 * t).sin() + 0.2 * (TAU * 1200.0 * t).sin();
            [s, s]
        }).collect();
        let out: Vec<f32> = quiet.iter().map(|s| lim.push(*s)).collect();
        let (a, b) = (lufs_stereo(rate, &quiet), lufs_stereo(rate, &out));
        assert!((a - b).abs() < 0.05, "{a} vs {b}");
        // Very loud noise: nothing above the ceiling ever comes out.
        let mut seed = 1u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        let mut lim = Limiter::new(rate, 2);
        let mut peak = 0.0f32;
        for _ in 0..rate as usize * 2 * 2 {
            peak = peak.max(lim.push(noise() * 4.0).abs());
        }
        assert!(peak <= CEILING + 1e-4, "{peak}");
    }

    /// Amplitude of `f` in `x` (Goertzel).
    fn tone(x: &[f32], rate: u32, f: f32) -> f32 {
        let w = TAU * f / rate as f32;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            re += (*v as f64) * (w as f64 * i as f64).cos();
            im += (*v as f64) * (w as f64 * i as f64).sin();
        }
        (2.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32
    }

    #[test]
    fn a_bass_peak_only_turns_the_bass_down() {
        // Bass boosted far over the ceiling with a voice on top: the voice
        // keeps its level, the bass comes down, nothing clips.
        let rate = 48_000;
        let mut lim = Limiter::new(rate, 2);
        let n = rate as usize * 3;
        let input: Vec<f32> = (0..n).map(|i| {
            let t = i as f32 / rate as f32;
            1.4 * (TAU * 50.0 * t).sin() + 0.15 * (TAU * 1000.0 * t).sin()
        }).collect();
        let mut out = Vec::with_capacity(n);
        let mut peak = 0.0f32;
        for s in &input {
            let l = lim.push(*s);
            let _ = lim.push(*s);
            peak = peak.max(l.abs());
            out.push(l);
        }
        let tail = &out[rate as usize..];
        let voice = tone(tail, rate, 1000.0);
        let bass = tone(tail, rate, 50.0);
        assert!(peak <= CEILING + 1e-4, "{peak}");
        assert!((voice - 0.15).abs() < 0.015, "voice turned down to {voice}");
        assert!(bass < 1.0, "bass not reduced: {bass}");
    }

    #[test]
    fn a_short_click_lets_go_right_after() {
        // The old leveller turned the song down for seconds after a loud
        // moment; the limiter is back to full level a moment later.
        let rate = 48_000;
        let mut lim = Limiter::new(rate, 1);
        let mut out = vec![];
        for i in 0..rate as usize {
            let t = i as f32 / rate as f32;
            let click = if (12_000..12_048).contains(&i) { 1.5 } else { 0.0 };
            out.push(lim.push(0.2 * (TAU * 1000.0 * t).sin() + click));
        }
        // 0.5 s after the click the tone is back at full level.
        let after = &out[rate as usize / 2 + 12_000..];
        assert!((tone(after, rate, 1000.0) - 0.2).abs() < 0.01);
    }

    #[test]
    fn measured_songs_are_remembered_on_disk() {
        let dir = std::env::temp_dir().join(format!("glyph-loudness-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("loudness.tsv");
        set_store(path.clone());
        remember("Spotify:abc", Analysis { lufs: -8.25, peak_db: -0.5 });
        remember("bad\nkey", Analysis { lufs: -8.0, peak_db: 0.0 });
        STORE.lock().unwrap().known.clear();
        set_store(path.clone());
        assert_eq!(lookup("Spotify:abc"), Some(Analysis { lufs: -8.25, peak_db: -0.5 }));
        assert_eq!(lookup("bad\nkey"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod eq_tests {
    //! The equalizer behind the levelling: a bass boost must be heard.
    use super::*;
    use std::f32::consts::TAU;

    /// A loud modern master: kick, bass line and dense mids, pushed into a
    /// limiter (peaks at the top, about -8 LUFS).
    fn loud_master(rate: u32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        let mut seed = 1u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        let mut out = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f32 / rate as f32;
            let beat = (t * 2.0).fract();
            let kick = (-beat * 12.0).exp() * (TAU * 55.0 * t).sin() * 0.7;
            let bass = 0.25 * (TAU * 82.0 * t).sin();
            let mids = 0.3 * (TAU * 440.0 * t).sin() + 0.2 * (TAU * 1320.0 * t).sin() + 0.15 * noise();
            let s = ((kick + bass + mids) * 1.2).clamp(-0.98, 0.98);
            out.push(s);
            out.push(s);
        }
        out
    }

    /// Level (dB) below and above 150 Hz.
    fn bands(rate: u32, x: &[f32]) -> (f32, f32) {
        let lp = Bq::butterworth(false, 150.0, rate as f32);
        let hp = Bq::butterworth(true, 150.0, rate as f32);
        let (mut a, mut b, mut c, mut d) = (BqMem::default(), BqMem::default(), BqMem::default(), BqMem::default());
        let (mut lo, mut hi) = (0f64, 0f64);
        for s in x.iter().step_by(2) {
            let l = lp.run(&mut b, lp.run(&mut a, *s));
            let h = hp.run(&mut d, hp.run(&mut c, *s));
            lo += (l * l) as f64;
            hi += (h * h) as f64;
        }
        let n = (x.len() / 2).max(1) as f64;
        (10.0 * (lo / n).max(1e-12).log10() as f32, 10.0 * (hi / n).max(1e-12).log10() as f32)
    }

    /// Plays `input` the way native.rs does (gain, equalizer, limiter).
    fn play(rate: u32, input: &[f32], normalize: bool, eq_on: bool, bass_db: f32) -> (f32, f32) {
        set(Level::Normal, normalize);
        crate::eq::set(eq_on, [0.0; 10]);
        crate::eq::set_extras(bass_db, 0.0);
        let mut m = Meter::new(rate, 2);
        input.iter().for_each(|s| m.push(*s));
        let shared = TrackLoudness::default();
        shared.set_known(m.result().unwrap());
        let mut l = Leveller::new(rate, 2);
        let mut e = crate::eq::Equalizer::new(rate, 2);
        let out: Vec<f32> = input
            .iter()
            .map(|s| {
                let y = l.gain(*s, &shared);
                e.set_room(l.room_db());
                let y = e.process(y);
                l.limit(y)
            })
            .collect();
        let skip = (rate as usize * 2).min(out.len());
        bands(rate, &out[skip..])
    }

    #[test]
    fn bass_boost_is_heard_with_even_volume() {
        let _g = super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let rate = 48_000;
        let song = loud_master(rate, 6.0);
        let (b0, m0) = play(rate, &song, true, false, 0.0);
        let (b1, m1) = play(rate, &song, true, true, 6.0);
        crate::eq::set_extras(0.0, 0.0);
        crate::eq::set(false, [0.0; 10]);
        set(Level::Normal, true);
        // The bass really gets louder; the rest stays where levelling put it.
        assert!(b1 - b0 > 3.5, "bass only {:+.1} dB", b1 - b0);
        assert!((m1 - m0).abs() < 0.5, "mids moved {:+.1} dB", m1 - m0);
    }

    #[test]
    fn without_room_the_equalizer_still_makes_room() {
        // Levelling off and a song at the top: boosts still get room, so
        // the limiter is not left to flatten them.
        let _g = super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let rate = 48_000;
        let song = loud_master(rate, 4.0);
        let (_, m0) = play(rate, &song, false, false, 0.0);
        let (_, m1) = play(rate, &song, false, true, 6.0);
        crate::eq::set_extras(0.0, 0.0);
        crate::eq::set(false, [0.0; 10]);
        set(Level::Normal, true);
        assert!(m1 < m0 - 2.5, "no room made: mids {m0:.1} -> {m1:.1}");
    }

    /// Real songs: `GLYPH_TEST_SONGS="a.m4a;b.mp3" cargo test -p glyph-audio
    /// real_songs -- --ignored --nocapture` prints bass and mids with and
    /// without a +6 dB bass boost.
    #[test]
    #[ignore]
    fn real_songs() {
        let _g = super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let rate = 48_000;
        let list = std::env::var("GLYPH_TEST_SONGS").unwrap_or_default();
        for path in list.split(';').filter(|p| !p.is_empty()) {
            let mut pcm = crate::pcm::Pcm::open(path, None, rate).expect("open");
            let _ = pcm.seek(30.0);
            let mut buf = Vec::new();
            pcm.read(&mut buf, rate as usize * 40);
            let song: Vec<f32> = buf.iter().map(|v| *v as f32 / 32768.0).collect();
            for norm in [true, false] {
                let (b0, m0) = play(rate, &song, norm, false, 0.0);
                let (b1, m1) = play(rate, &song, norm, true, 6.0);
                println!("{path} even={norm}: bass {:+.1} dB, mids {:+.1} dB", b1 - b0, m1 - m0);
            }
        }
        crate::eq::set_extras(0.0, 0.0);
        crate::eq::set(false, [0.0; 10]);
        set(Level::Normal, true);
    }
}
