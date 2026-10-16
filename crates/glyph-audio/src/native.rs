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
use std::num::{NonZeroU16, NonZeroU32};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Chunks (one per codec packet) buffered between decoder and audio thread.
const CHUNK_CAP: usize = 128;
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

/// Control block shared between engine and decode thread.
#[derive(Default)]
pub struct DecodeCtl {
    pub stop: AtomicBool,
    pub gen: AtomicU64,
    pub seek: Mutex<Option<f64>>,
    /// Decode thread has exited (EOF or error); seeks need a reload.
    pub done: AtomicBool,
}

impl DecodeCtl {
    pub fn request_seek(&self, secs: f64) {
        // Bump generation first so chunks decoded before the seek are dropped.
        self.gen.fetch_add(1, Ordering::SeqCst);
        *self.seek.lock().unwrap() = Some(secs);
    }
}

/// Playback progress, written by the audio thread, read by the engine.
#[derive(Default)]
pub struct TrackShared {
    pub pos_frames: AtomicU64,
    pub started: AtomicBool,
    pub finished: AtomicBool,
}

/// Rolling mono tap for the FFT.
#[derive(Default)]
pub struct Tap {
    pub samples: Mutex<Vec<f32>>,
}

pub struct Opened {
    pub rx: Receiver<Chunk>,
    pub rate: SampleRate,
    pub channels: ChannelCount,
    pub duration: Option<Duration>,
    pub ctl: Arc<DecodeCtl>,
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

fn decode_thread(
    id: u64,
    url: &str,
    mime: Option<&str>,
    engine: &Sender<Cmd>,
    ctl: Arc<DecodeCtl>,
) -> Result<(), String> {
    let src = AudioSource::open(url).map_err(|e| format!("cannot open stream: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(src), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &hint_for(url, mime),
            mss,
            &FormatOptions { enable_gapless: true, ..Default::default() },
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("unsupported audio format: {e}"))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("no audio track")?
        .clone();
    let track_id = track.id;
    let time_base = track.codec_params.time_base;
    let duration = match (track.codec_params.n_frames, time_base) {
        (Some(n), Some(tb)) => {
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
        let samples = sb.samples().to_vec();
        let frames = (samples.len() / ch) as u64;
        let chunk = Chunk { gen, start_frame: frames_sent, samples };
        frames_sent += frames;

        if !announced {
            announced = true;
            let _ = tx.send(chunk);
            let opened = Opened {
                rx: rx.take().expect("announced once"),
                rate: NonZeroU32::new(spec.rate.max(1)).unwrap(),
                channels: NonZeroU16::new(ch as u16).unwrap(),
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
}

impl TrackSource {
    pub fn new(opened: Opened, shared: Arc<TrackShared>, tap: Arc<Tap>) -> Self {
        Self {
            rx: opened.rx,
            cur: Vec::new(),
            idx: 0,
            base_frame: 0,
            channels: opened.channels,
            rate: opened.rate,
            duration: opened.duration,
            ctl: opened.ctl,
            shared,
            tap,
            tap_local: Vec::with_capacity(TAP_FLUSH),
            tap_acc: 0.0,
            tap_n: 0,
            counter: 0,
        }
    }

    fn publish_pos(&self) {
        let ch = self.channels.get() as usize;
        let frame = self.base_frame + (self.idx / ch) as u64;
        self.shared.pos_frames.store(frame, Ordering::Relaxed);
    }

    fn flush_tap(&mut self) {
        if let Ok(mut g) = self.tap.samples.try_lock() {
            g.extend_from_slice(&self.tap_local);
            let len = g.len();
            if len > TAP_KEEP {
                g.drain(..len - TAP_KEEP);
            }
            self.tap_local.clear();
        } else if self.tap_local.len() > TAP_KEEP {
            self.tap_local.clear();
        }
    }
}

impl Iterator for TrackSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.idx >= self.cur.len() {
            let gen = self.ctl.gen.load(Ordering::SeqCst);
            loop {
                match self.rx.try_recv() {
                    Ok(c) if c.gen < gen => continue,
                    Ok(c) => {
                        self.cur = c.samples;
                        self.idx = 0;
                        self.base_frame = c.start_frame;
                        self.shared.started.store(true, Ordering::Relaxed);
                        self.publish_pos();
                        break;
                    }
                    Err(flume::TryRecvError::Empty) => return Some(0.0),
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
        Some(s)
    }
}

impl Source for TrackSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.channels
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

    #[test]
    fn hint_uses_extension_ignoring_query() {
        let h = hint_for("https://x.test/a/b/song.MP3?sig=abc.def", None);
        let _ = h; // Hint has no getters; just make sure it does not panic.
        let _ = hint_for("https://x.test/stream", Some("audio/mpeg"));
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
            duration: None,
            ctl,
        };
        let shared = Arc::new(TrackShared::default());
        let mut src = TrackSource::new(opened, shared.clone(), Arc::new(Tap::default()));
        let got: Vec<f32> = src.by_ref().collect();
        assert_eq!(got, vec![0.5, 0.25, 0.125, 0.0625]);
        assert!(shared.finished.load(Ordering::SeqCst));
        assert!(shared.started.load(Ordering::SeqCst));
        assert_eq!(shared.pos_frames.load(Ordering::SeqCst), 100);
    }

    #[test]
    fn underrun_yields_silence_not_end() {
        let (tx, rx) = flume::bounded::<Chunk>(2);
        let opened = Opened {
            rx,
            rate: NonZeroU32::new(44_100).unwrap(),
            channels: NonZeroU16::new(1).unwrap(),
            duration: None,
            ctl: Arc::new(DecodeCtl::default()),
        };
        let mut src = TrackSource::new(opened, Arc::new(TrackShared::default()), Arc::new(Tap::default()));
        assert_eq!(src.next(), Some(0.0));
        drop(tx);
        assert_eq!(src.next(), None);
    }
}
