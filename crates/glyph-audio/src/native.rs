//! Native backend pieces: HTTP -> symphonia decode thread -> bounded chunk
//! channel -> [`TrackSource`] (a rodio `Source`) played by a rodio `Player`.
//!
//! One decode thread per track. It opens the stream, probes the format,
//! decodes the first buffer (to learn the real rate/channel count), reports
//! [`Opened`] to the engine and then keeps decoding until the bounded channel
//! (backpressure = read-ahead of a few seconds) or the track ends. Seeking is
//! done by the decode thread (`FormatReader::seek`), stale chunks are dropped
//! via a generation counter.

use crate::http::AudioSource;
use crate::Cmd;
use flume::{Receiver, Sender, TrySendError};
use rodio::{ChannelCount, SampleRate, Source};
use std::io::{Read, Seek, SeekFrom};
use std::num::{NonZeroU16, NonZeroU32};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Chunks (one per codec packet) buffered between decoder and audio thread.
// ~12 seconds of AAC packets, insulating playback from brief CPU/network stalls.
const CHUNK_CAP: usize = 512;
#[cfg(test)]
pub(crate) static TEST_UNDERRUNS: AtomicU64 = AtomicU64::new(0);
/// Mono samples collected per tap flush.
const TAP_FLUSH: usize = 512;
/// Mono samples kept for the FFT.
pub const TAP_KEEP: usize = 4096;

pub struct Chunk {
    pub gen: u64,
    /// Frame index (in the track) of the first frame of `samples`.
    pub start_frame: u64,
    pub samples: Vec<f32>,
}

fn sample_reaper() -> &'static Sender<Vec<f32>> {
    static REAPER: std::sync::OnceLock<Sender<Vec<f32>>> = std::sync::OnceLock::new();
    REAPER.get_or_init(|| {
        let (tx, rx) = flume::unbounded::<Vec<f32>>();
        std::thread::Builder::new()
            .name("glyph-audio-reaper".into())
            .spawn(move || while let Ok(b) = rx.recv() { drop(b); })
            .expect("start audio buffer reaper");
        tx
    })
}

/// Control block shared between engine and decode thread.
pub struct DecodeCtl {
    pub stop: AtomicBool,
    pub gen: AtomicU64,
    pub seek: Mutex<Option<f64>>,
    /// Decode thread has exited (EOF or error); seeks need a reload.
    pub done: AtomicBool,
    /// Recycled sample buffers returned from the real-time audio callback so
    /// heap deallocation and per-packet allocation are avoided during playback.
    recycle_tx: Sender<Vec<f32>>,
    recycle_rx: Receiver<Vec<f32>>,
}

impl Default for DecodeCtl {
    fn default() -> Self {
        // Initialize on the engine thread, never lazily inside the audio callback.
        sample_reaper();
        let (recycle_tx, recycle_rx) = flume::bounded(CHUNK_CAP * 2);
        Self {
            stop: AtomicBool::new(false),
            gen: AtomicU64::new(0),
            seek: Mutex::new(None),
            done: AtomicBool::new(false),
            recycle_tx,
            recycle_rx,
        }
    }
}

impl DecodeCtl {
    pub fn request_seek(&self, secs: f64) {
        // Bump generation first so chunks decoded before the seek are dropped.
        self.gen.fetch_add(1, Ordering::SeqCst);
        *self.seek.lock().unwrap() = Some(secs);
    }

    /// Returns a buffer from the real-time audio thread without freeing it there.
    pub fn recycle(&self, mut buf: Vec<f32>) {
        if buf.capacity() == 0 {
            return;
        }
        buf.clear();
        if let Err(TrySendError::Full(buf) | TrySendError::Disconnected(buf)) = self.recycle_tx.try_send(buf) {
            let _ = sample_reaper().try_send(buf);
        }
    }

    /// Takes a recycled buffer for the next decoded packet, if available.
    pub fn take_recycled(&self) -> Vec<f32> {
        self.recycle_rx.try_recv().unwrap_or_default()
    }
}

/// Playback progress, written by the audio thread, read by the engine.
#[derive(Default)]
pub struct TrackShared {
    pub pos_frames: AtomicU64,
    /// Generation of the last audible position; older positions cannot undo a seek.
    pub playback_gen: AtomicU64,
    pub started: AtomicBool,
    /// The callback is supplying silence because decoded audio ran out.
    pub buffering: AtomicBool,
    pub finished: AtomicBool,
    /// Decoder starvation events; consumed and logged by the engine, never the callback.
    pub underruns: AtomicU64,
    pub loudness: crate::loudness::TrackLoudness,
}

/// Rolling mono tap for the FFT.
pub struct Tap {
    pub samples: Mutex<Vec<f32>>,
}

impl Default for Tap {
    fn default() -> Self {
        // The real-time callback must not allocate when publishing FFT samples.
        Self { samples: Mutex::new(Vec::with_capacity(TAP_KEEP * 2)) }
    }
}

pub struct Opened {
    pub rx: Receiver<Chunk>,
    pub rate: SampleRate,
    pub channels: ChannelCount,
    /// symphonia channel bits (speaker positions); 0 = unknown.
    pub layout: u32,
    pub duration: Option<Duration>,
    pub ctl: Arc<DecodeCtl>,
}

/// Byte length of a leading ID3v2 header (magic + version + flags + size).
const ID3V2_HEADER: u64 = 10;

/// Offset of a *degenerate* ID3v2 header, or `None`.
///
/// Deezer's official 30 s previews (and a few other encoders) open with an
/// ID3v2.4 header whose declared tag size is 0. symphonia's ID3v2 reader
/// treats that empty tag as broken and aborts the whole probe with
/// "out of bounds" before it ever reaches the MPEG frames, so nothing plays.
/// The announced tag carries no frames, so it is safe to step over it.
///
/// A real ID3v2 tag (size > 0) is returned as `None` and read normally.
///
/// Leaves the stream where the probe has to start: after the empty tag, or
/// back at 0. MPEG survives a wrong start (frame sync), MP4 does not: its
/// `ftyp` box sits at byte 4 and the probe ends with "end of stream".
fn degenerate_id3v2_offset(src: &mut AudioSource) -> Option<u64> {
    if !src.is_seekable() {
        return None;
    }
    let off = read_degenerate_id3v2(src);
    src.seek(SeekFrom::Start(off.unwrap_or(0))).ok()?;
    off
}

fn read_degenerate_id3v2(src: &mut AudioSource) -> Option<u64> {
    src.seek(SeekFrom::Start(0)).ok()?;
    let mut h = [0u8; ID3V2_HEADER as usize];
    src.read_exact(&mut h).ok()?;
    if &h[0..3] != b"ID3" {
        return None;
    }
    // Size is synchsafe: 7 bits per byte, high bit always zero.
    let size = u32::from(h[6] & 0x7f) << 21
        | u32::from(h[7] & 0x7f) << 14
        | u32::from(h[8] & 0x7f) << 7
        | u32::from(h[9] & 0x7f);
    (size == 0).then_some(ID3V2_HEADER)
}

/// Decide a symphonia probe hint from mime/url.
pub fn hint_for(url: &str, mime: Option<&str>) -> Hint {
    let mut hint = Hint::new();
    if let Some(m) = mime {
        hint.mime_type(m);
    }
    let path = url.split(['?', '#']).next().unwrap_or(url);
    if let Some(ext) = path.rsplit('.').next() {
        if ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            hint.with_extension(&ext.to_ascii_lowercase());
        }
    }
    hint
}

/// Spawn the open+decode thread. Result is delivered as `Cmd::Opened` /
/// `Cmd::OpenFailed` to the engine.
pub fn spawn_decoder(id: u64, url: String, mime: Option<String>, engine: Sender<Cmd>) {
    let ctl2 = Arc::new(DecodeCtl::default());
    let spawned = std::thread::Builder::new().name(format!("glyph-decode-{id}")).spawn(move || {
        let res = decode_thread(id, &url, mime.as_deref(), &engine, ctl2.clone());
        ctl2.done.store(true, Ordering::SeqCst);
        if let Err(msg) = res {
            if !ctl2.stop.load(Ordering::SeqCst) {
                let _ = engine.send(Cmd::DecodeError { id, message: msg });
            }
        }
    });
    if let Err(e) = spawned {
        tracing::error!("cannot spawn decode thread: {e}");
    }
}

/// Keeps HTTP Range seeks available after incremental MP4 initialization.
struct ProbeSource {
    inner: AudioSource,
    incremental_probe: Arc<AtomicBool>,
}

impl Read for ProbeSource {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> { self.inner.read(bytes) }
}
impl Seek for ProbeSource {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> { self.inner.seek(to) }
}
impl MediaSource for ProbeSource {
    fn is_seekable(&self) -> bool {
        !self.incremental_probe.load(Ordering::Relaxed) && self.inner.is_seekable()
    }
    fn byte_len(&self) -> Option<u64> { self.inner.byte_len() }
}

/// A moov before media data makes this MP4 streamable. Inspect only the
/// already-buffered prefix; non-fast-start files retain the normal probe.
fn streamable_http_mp4(src: &mut AudioSource) -> bool {
    if !matches!(src, AudioSource::Http(_)) || !src.is_seekable() { return false; }
    let Ok(original) = src.stream_position() else { return false; };
    let inspect = |src: &mut AudioSource| -> std::io::Result<bool> {
        let mut offset = 0u64;
        let mut ftyp = false;
        while offset + 8 <= 64 * 1024 {
            src.seek(SeekFrom::Start(offset))?;
            let mut header = [0u8; 8];
            src.read_exact(&mut header)?;
            let size = u32::from_be_bytes(header[..4].try_into().unwrap()) as u64;
            if size < 8 { return Ok(false); }
            match &header[4..] {
                b"ftyp" => ftyp = true,
                b"moov" => return Ok(ftyp),
                b"mdat" | b"moof" => return Ok(false),
                _ => {},
            }
            offset += size;
        }
        Ok(false)
    };
    let result = inspect(src).unwrap_or(false);
    let _ = src.seek(SeekFrom::Start(original));
    result
}

/// Opens `url` and probes its container.
pub(crate) fn open_format(url: &str, mime: Option<&str>) -> Result<Box<dyn FormatReader>, String> {
    let mut src = AudioSource::open(url).map_err(|e| format!("cannot open stream: {e} [url={url}]"))?;
    // Step over an empty ID3v2 tag (Deezer previews) that symphonia cannot parse.
    if let Some(off) = degenerate_id3v2_offset(&mut src) {
        tracing::debug!("skipping degenerate ID3v2 header ({off} bytes) in {url}");
    }
    // Symphonia scans every atom in seekable MP4s during try_new, even when
    // moov and the first segment index are at the front. Use its streaming
    // initialization for these network files, then restore real Range seeks.
    let incremental_probe = Arc::new(AtomicBool::new(streamable_http_mp4(&mut src)));
    let mss = MediaSourceStream::new(Box::new(ProbeSource { inner: src, incremental_probe: incremental_probe.clone() }), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &hint_for(url, mime),
            mss,
            &FormatOptions { enable_gapless: true, ..Default::default() },
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("unsupported audio format: {e}"))?;
    incremental_probe.store(false, Ordering::Relaxed);
    Ok(probed.format)
}

fn decode_thread(
    id: u64,
    url: &str,
    mime: Option<&str>,
    engine: &Sender<Cmd>,
    ctl: Arc<DecodeCtl>,
) -> Result<(), String> {
    let mut format = open_format(url, mime)?;
    let started = std::time::Instant::now();
    tracing::info!(track_id = id, local = !url.starts_with("http"), "playback: decoder format opened");
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("no audio track")?
        .clone();
    let track_id = track.id;
    let time_base = track.codec_params.time_base;
    let duration = match (track.codec_params.n_frames, time_base) {
        // Fragmented MP4 starts with an empty sample table (duration zero).
        // Treat that as unknown so the engine retains the track metadata.
        (Some(n), Some(tb)) if n > 0 => {
            let t = tb.calc_time(n);
            Some(Duration::from_secs_f64(t.seconds as f64 + t.frac))
        }
        _ => None,
    };
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("unsupported codec: {e}"))?;

    let (tx, rx) = flume::bounded::<Chunk>(CHUNK_CAP);
    let mut rx = Some(rx);
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut announced = false;
    let mut frames_sent: u64 = 0;
    let mut gen = ctl.gen.load(Ordering::SeqCst);
    let mut rate_hz: u32 = track.codec_params.sample_rate.unwrap_or(44_100);

    loop {
        if ctl.stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        // Seek request?
        let pending = ctl.seek.lock().unwrap().take();
        if let Some(secs) = pending {
            gen = ctl.gen.load(Ordering::SeqCst);
            match format.seek(
                SeekMode::Accurate,
                SeekTo::Time { time: Time::from(secs.max(0.0)), track_id: Some(track_id) },
            ) {
                Ok(seeked) => {
                    decoder.reset();
                    frames_sent = match time_base {
                        Some(tb) => {
                            let t = tb.calc_time(seeked.actual_ts);
                            ((t.seconds as f64 + t.frac) * rate_hz as f64) as u64
                        }
                        None => (secs * rate_hz as f64) as u64,
                    };
                }
                Err(e) => tracing::warn!("seek failed: {e}"),
            }
        }

        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(SymError::ResetRequired) => return Ok(()),
            Err(e) => {
                return if announced { Err(format!("stream error: {e}")) } else { Err(format!("read error: {e}")) };
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(e)) => {
                tracing::debug!("decode error (skipped): {e}");
                continue;
            }
            Err(SymError::IoError(_)) => continue,
            Err(e) => return Err(format!("decode error: {e}")),
        };
        let spec = *decoded.spec();
        if decoded.frames() == 0 {
            continue;
        }
        let sb = sample_buf
            .get_or_insert_with(|| SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        if sb.capacity() < decoded.frames() * spec.channels.count() {
            *sb = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        }
        sb.copy_interleaved_ref(decoded);
        rate_hz = spec.rate;
        let ch = spec.channels.count().max(1);
        let mut samples = ctl.take_recycled();
        samples.extend_from_slice(sb.samples());
        let frames = (samples.len() / ch) as u64;
        let chunk = Chunk { gen, start_frame: frames_sent, samples };
        frames_sent += frames;

        if !announced {
            tracing::info!(track_id = id, elapsed_ms = started.elapsed().as_millis() as u64, "playback: first audio packet decoded");
            announced = true;
            let _ = tx.send(chunk);
            let opened = Opened {
                rx: rx.take().expect("announced once"),
                rate: NonZeroU32::new(spec.rate.max(1)).unwrap(),
                channels: NonZeroU16::new(ch as u16).unwrap(),
                layout: spec.channels.bits(),
                duration,
                ctl: ctl.clone(),
            };
            if engine.send(Cmd::Opened { id, opened }).is_err() {
                return Ok(());
            }
            continue;
        }
        // Backpressure: wait for room, but stay responsive to stop/seek.
        let mut chunk = Some(chunk);
        while let Some(c) = chunk.take() {
            if ctl.stop.load(Ordering::SeqCst) {
                return Ok(());
            }
            if ctl.seek.lock().unwrap().is_some() {
                break; // drop it, seek handled at the top of the loop
            }
            match tx.try_send(c) {
                Ok(()) => {}
                Err(TrySendError::Full(c)) => {
                    chunk = Some(c);
                    std::thread::sleep(Duration::from_millis(15));
                }
                Err(TrySendError::Disconnected(_)) => return Ok(()),
            }
        }
    }
}

/// rodio source fed from the decode thread. Never blocks the audio thread:
/// on underrun it emits silence; it ends once the decoder is gone and drained.
pub struct TrackSource {
    rx: Receiver<Chunk>,
    cur: Vec<f32>,
    idx: usize,
    base_frame: u64,
    gen: u64,
    channels: ChannelCount,
    rate: SampleRate,
    duration: Option<Duration>,
    ctl: Arc<DecodeCtl>,
    shared: Arc<TrackShared>,
    tap: Arc<Tap>,
    tap_local: Vec<f32>,
    tap_acc: f32,
    tap_n: usize,
    counter: u32,
    silence_remaining: usize,
    starving: bool,
    level: crate::loudness::Leveller,
    eq: crate::eq::Equalizer,
    /// Mono / stereo / spatial rendering of each decoded frame.
    render: crate::spatial::Renderer,
    out_channels: ChannelCount,
    /// One decoded frame and the rendered frame being handed out.
    frame_in: Vec<f32>,
    frame_out: Vec<f32>,
    out_idx: usize,
}

impl TrackSource {
    pub fn new(opened: Opened, shared: Arc<TrackShared>, tap: Arc<Tap>) -> Self {
        let level = crate::loudness::Leveller::new(opened.rate.get(), opened.channels.get());
        let eq = crate::eq::Equalizer::new(opened.rate.get(), opened.channels.get());
        let render = crate::spatial::Renderer::new(opened.rate.get(), opened.channels.get(), opened.layout);
        let out_channels = NonZeroU16::new(render.out_channels()).unwrap_or(opened.channels);
        Self {
            render,
            out_channels,
            frame_in: vec![0.0; opened.channels.get() as usize],
            frame_out: vec![0.0; out_channels.get() as usize],
            out_idx: usize::MAX,
            rx: opened.rx,
            cur: Vec::new(),
            idx: 0,
            base_frame: 0,
            gen: 0,
            channels: opened.channels,
            rate: opened.rate,
            duration: opened.duration,
            ctl: opened.ctl,
            shared,
            tap,
            tap_local: Vec::with_capacity(TAP_KEEP),
            tap_acc: 0.0,
            tap_n: 0,
            counter: 0,
            silence_remaining: 0,
            starving: false,
            level,
            eq,
        }
    }

    fn publish_pos(&self) {
        let ch = self.channels.get() as usize;
        let frame = self.base_frame + (self.idx / ch) as u64;
        self.shared.pos_frames.store(frame, Ordering::Relaxed);
        self.shared.playback_gen.store(self.gen, Ordering::Release);
    }

    fn flush_tap(&mut self) {
        if let Ok(mut g) = self.tap.samples.try_lock() {
            g.extend_from_slice(&self.tap_local);
            let len = g.len();
            if len > TAP_KEEP {
                g.drain(..len - TAP_KEEP);
            }
            self.tap_local.clear();
        } else if self.tap_local.len() >= TAP_KEEP {
            self.tap_local.clear();
        }
    }
}

impl Drop for TrackSource {
    fn drop(&mut self) {
        self.ctl.recycle(std::mem::take(&mut self.cur));
        self.ctl.recycle(std::mem::take(&mut self.tap_local));
    }
}

impl TrackSource {
    /// Renders with `r` instead (tests: plain pass-through).
    #[cfg(test)]
    fn with_renderer(mut self, r: crate::spatial::Renderer) -> Self {
        self.out_channels = NonZeroU16::new(r.out_channels()).unwrap();
        self.frame_out = vec![0.0; r.out_channels() as usize];
        self.render = r;
        self
    }
}

impl Iterator for TrackSource {
    type Item = f32;

    /// Hands out the rendered frame sample by sample; decodes, filters and
    /// renders the next frame when it is used up.
    fn next(&mut self) -> Option<f32> {
        if self.out_idx < self.frame_out.len() {
            let s = self.frame_out[self.out_idx];
            self.out_idx += 1;
            return Some(s);
        }
        for c in 0..self.frame_in.len() {
            self.frame_in[c] = self.next_input()?;
        }
        self.render.process(&self.frame_in, &mut self.frame_out);
        self.out_idx = 1;
        Some(self.frame_out[0])
    }
}

impl TrackSource {
    /// Next decoded sample of the song's own channel layout (after the
    /// equalizer and levelling).
    fn next_input(&mut self) -> Option<f32> {
        let requested = self.ctl.gen.load(Ordering::SeqCst);
        if requested != self.gen {
            let old = std::mem::take(&mut self.cur);
            self.ctl.recycle(old);
            self.idx = 0;
            self.silence_remaining = 0;
            self.gen = requested;
        }
        // Retry only at complete interleaved-frame boundaries. Otherwise a
        // decoder waking after an odd number of zero samples swaps L/R.
        if self.silence_remaining > 0 {
            self.silence_remaining -= 1;
            return Some(0.0);
        }
        if self.idx >= self.cur.len() {
            // Dropped (e.g. a preloaded next song that is no longer next):
            // end now instead of playing what is still buffered.
            if self.ctl.stop.load(Ordering::SeqCst) {
                self.shared.finished.store(true, Ordering::SeqCst);
                return None;
            }
            let gen = self.ctl.gen.load(Ordering::SeqCst);
            loop {
                match self.rx.try_recv() {
                    Ok(c) if c.gen < gen => {
                        self.ctl.recycle(c.samples);
                        continue;
                    }
                    Ok(c) => {
                        if c.samples.is_empty() {
                            self.ctl.recycle(c.samples);
                            continue;
                        }
                        self.starving = false;
                        self.shared.buffering.store(false, Ordering::Relaxed);
                        let old = std::mem::replace(&mut self.cur, c.samples);
                        self.ctl.recycle(old);
                        self.idx = 0;
                        self.base_frame = c.start_frame;
                        self.gen = c.gen;
                        self.shared.started.store(true, Ordering::Relaxed);
                        self.publish_pos();
                        break;
                    }
                    Err(flume::TryRecvError::Empty) => {
                        if !self.starving {
                            self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                            #[cfg(test)]
                            TEST_UNDERRUNS.fetch_add(1, Ordering::Relaxed);
                            self.starving = true;
                            self.shared.buffering.store(true, Ordering::Relaxed);
                        }
                        self.silence_remaining = self.channels.get() as usize - 1;
                        return Some(0.0);
                    },
                    Err(flume::TryRecvError::Disconnected) => {
                        self.shared.finished.store(true, Ordering::SeqCst);
                        return None;
                    }
                }
            }
        }
        let ch = self.channels.get() as usize;
        let s = self.cur[self.idx];
        self.idx += 1;
        // mono tap
        self.tap_acc += s;
        self.tap_n += 1;
        if self.tap_n == ch {
            self.tap_local.push(self.tap_acc / ch as f32);
            self.tap_acc = 0.0;
            self.tap_n = 0;
            if self.tap_local.len() >= TAP_FLUSH {
                self.flush_tap();
            }
        }
        self.counter = self.counter.wrapping_add(1);
        if self.counter % 1024 == 0 {
            self.publish_pos();
        }
        let s = self.eq.process(s);
        Some(self.level.process(s, &self.shared.loudness))
    }
}

impl Source for TrackSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.out_channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes a whole local file the way `decode_thread` opens it:
    /// `GLYPH_TEST_AUDIO=<file> cargo test -p glyph-audio decodes_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn decodes_file_to_the_end() {
        let path = std::env::var("GLYPH_TEST_AUDIO").expect("GLYPH_TEST_AUDIO");
        let mut format = open_format(&path, Some("audio/mp4")).unwrap();
        let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).unwrap().clone();
        let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).unwrap();
        let rate = track.codec_params.sample_rate.unwrap() as f64;
        let mut frames = 0u64;
        while let Ok(packet) = format.next_packet() {
            if packet.track_id() == track.id {
                frames += decoder.decode(&packet).unwrap().frames() as u64;
            }
        }
        println!("decoded {:.1} s at {rate} Hz", frames as f64 / rate);
        assert!(frames as f64 / rate > 60.0);
    }

    #[test]
    fn id3_check_leaves_stream_at_probe_start() {
        let dir = std::env::temp_dir().join(format!("glyph-id3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let open = |name: &str, bytes: &[u8]| {
            let p = dir.join(name);
            std::fs::write(&p, bytes).unwrap();
            AudioSource::open(p.to_str().unwrap()).unwrap()
        };
        // MP4 (YouTube m4a): no tag, must be back at 0 for the `ftyp` box.
        let mut mp4 = open("a.m4a", b"\0\0\0\x18ftypdash\0\0\0\0iso6mp41");
        assert_eq!(degenerate_id3v2_offset(&mut mp4), None);
        assert_eq!(mp4.stream_position().unwrap(), 0);
        // Deezer preview: empty ID3v2 tag is skipped.
        let mut mp3 = open("b.mp3", b"ID3\x04\0\0\0\0\0\0\xff\xfb\x90\x64");
        assert_eq!(degenerate_id3v2_offset(&mut mp3), Some(10));
        assert_eq!(mp3.stream_position().unwrap(), 10);
        // Real tag: read by symphonia from the start.
        let mut tagged = open("c.mp3", b"ID3\x04\0\0\0\0\0\x05TIT2\0\xff\xfb");
        assert_eq!(degenerate_id3v2_offset(&mut tagged), None);
        assert_eq!(tagged.stream_position().unwrap(), 0);
        drop((mp4, mp3, tagged));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hint_uses_extension_ignoring_query() {
        let h = hint_for("https://x.test/a/b/song.MP3?sig=abc.def", None);
        let _ = h; // Hint has no getters; just make sure it does not panic.
        let _ = hint_for("https://x.test/stream", Some("audio/mpeg"));
    }

    fn file_source(bytes: Vec<u8>) -> AudioSource {
        let p = std::env::temp_dir().join(format!("glyph-id3-{}.bin", std::process::id()));
        std::fs::write(&p, &bytes).unwrap();
        AudioSource::File(std::fs::File::open(&p).unwrap())
    }

    /// Deezer's 30 s previews open with an ID3v2.4 header declaring tag size 0.
    /// symphonia aborts the probe on it, so we must step over it.
    #[test]
    fn degenerate_id3v2_header_is_skipped() {
        // ID3, v2.4, no flags, synchsafe size 0, then an MPEG frame sync.
        let mut deezer = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
        deezer.extend_from_slice(&[0xFF, 0xFB, 0x92, 0x64]);
        assert_eq!(degenerate_id3v2_offset(&mut file_source(deezer.clone())), Some(ID3V2_HEADER));

        // A real tag with frames must be left alone.
        let mut real = b"ID3\x04\x00\x00\x00\x00\x00\x20".to_vec();
        real.extend_from_slice(&[0u8; 32]);
        real.extend_from_slice(&[0xFF, 0xFB, 0x92, 0x64]);
        assert_eq!(degenerate_id3v2_offset(&mut file_source(real)), None);

        // Raw MPEG frames, no tag at all.
        let raw = vec![0xFF, 0xFB, 0x92, 0x64, 0x00, 0x00];
        assert_eq!(degenerate_id3v2_offset(&mut file_source(raw)), None);
    }

    #[test]
    fn seek_holds_position_until_new_generation_arrives() {
        let (tx, rx) = flume::bounded(8);
        let ctl = Arc::new(DecodeCtl::default());
        let shared = Arc::new(TrackShared::default());
        let opened = Opened { rx, rate: NonZeroU32::new(48_000).unwrap(),
            channels: NonZeroU16::new(2).unwrap(), layout: 0, duration: None, ctl: ctl.clone() };
        let mut src = TrackSource::new(opened, shared.clone(), Arc::new(Tap::default()));
        src.level = crate::loudness::Leveller::bypass();
        src.eq = crate::eq::Equalizer::bypass();
        src = src.with_renderer(crate::spatial::Renderer::bypass(2));
        tx.send(Chunk { gen: 0, start_frame: 100, samples: vec![0.5; 4096] }).unwrap();
        assert_eq!(src.next(), Some(0.5));
        assert_eq!(src.next(), Some(0.5));
        ctl.request_seek(60.0);
        assert_eq!(src.next(), Some(0.0));
        assert_ne!(shared.playback_gen.load(Ordering::Acquire), ctl.gen.load(Ordering::SeqCst));
        assert_eq!(shared.pos_frames.load(Ordering::Relaxed), 100);
        tx.send(Chunk { gen: 1, start_frame: 2_880_000, samples: vec![0.25; 4] }).unwrap();
        assert_eq!(src.next(), Some(0.0)); // finish the silent stereo frame
        assert_eq!(src.next(), Some(0.25));
        assert_eq!(shared.playback_gen.load(Ordering::Acquire), ctl.gen.load(Ordering::SeqCst));
        assert_eq!(shared.pos_frames.load(Ordering::Relaxed), 2_880_000);
    }

    #[test]
    fn track_source_discards_stale_generations_and_ends() {
        let (tx, rx) = flume::bounded(8);
        let ctl = Arc::new(DecodeCtl::default());
        ctl.request_seek(1.0); // gen -> 1
        tx.send(Chunk { gen: 0, start_frame: 0, samples: vec![9.0, 9.0] }).unwrap();
        tx.send(Chunk { gen: 1, start_frame: 100, samples: vec![0.5, 0.25, 0.125, 0.0625] }).unwrap();
        drop(tx);
        let opened = Opened {
            rx,
            rate: NonZeroU32::new(48_000).unwrap(),
            channels: NonZeroU16::new(2).unwrap(),
            layout: 0,
            duration: None,
            ctl,
        };
        let shared = Arc::new(TrackShared::default());
        let mut src = TrackSource::new(opened, shared.clone(), Arc::new(Tap::default()));
        src.level = crate::loudness::Leveller::bypass();
        src.eq = crate::eq::Equalizer::bypass();
        src = src.with_renderer(crate::spatial::Renderer::bypass(2));
        let got: Vec<f32> = src.by_ref().collect();
        assert_eq!(got, vec![0.5, 0.25, 0.125, 0.0625]);
        assert!(shared.finished.load(Ordering::SeqCst));
        assert!(shared.started.load(Ordering::SeqCst));
        assert_eq!(shared.pos_frames.load(Ordering::SeqCst), 100);
    }

    #[test]
    fn stereo_starvation_preserves_frames_when_decoder_wakes_between_channels() {
        let (tx, rx) = flume::bounded::<Chunk>(4);
        let shared = Arc::new(TrackShared::default());
        let opened = Opened { rx, rate: NonZeroU32::new(48_000).unwrap(),
            channels: NonZeroU16::new(2).unwrap(), layout: 0, duration: None,
            ctl: Arc::new(DecodeCtl::default()) };
        let mut src = TrackSource::new(opened, shared.clone(), Arc::new(Tap::default()));
        src.level = crate::loudness::Leveller::bypass();
        src.eq = crate::eq::Equalizer::bypass();
        src = src.with_renderer(crate::spatial::Renderer::bypass(2));
        assert_eq!(src.next(), Some(0.0)); // missing left channel
        assert!(shared.buffering.load(Ordering::Relaxed));
        tx.send(Chunk { gen: 0, start_frame: 0, samples: vec![0.75,-0.25] }).unwrap();
        assert_eq!(src.next(), Some(0.0)); // finish the silent stereo frame
        assert_eq!(src.next(), Some(0.75));
        assert!(!shared.buffering.load(Ordering::Relaxed));
        assert_eq!(src.next(), Some(-0.25));
        assert_eq!(shared.underruns.load(Ordering::Relaxed),1);
    }

    #[test]
    fn underrun_yields_silence_not_end() {
        let (tx, rx) = flume::bounded::<Chunk>(2);
        let opened = Opened {
            rx,
            rate: NonZeroU32::new(44_100).unwrap(),
            channels: NonZeroU16::new(1).unwrap(),
            layout: 0,
            duration: None,
            ctl: Arc::new(DecodeCtl::default()),
        };
        let mut src = TrackSource::new(opened, Arc::new(TrackShared::default()), Arc::new(Tap::default()))
            .with_renderer(crate::spatial::Renderer::bypass(1));
        assert_eq!(src.next(), Some(0.0));
        drop(tx);
        assert_eq!(src.next(), None);
    }
}
