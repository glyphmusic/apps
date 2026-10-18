//! Playback for Glyph.
//!
//! CONTRACT (the UI depends on exactly these public items — extend, but do not
//! rename or change signatures without updating `docs/INTERFACES.md`):
//!
//! * [`PlayerController`] drives two backends behind one interface:
//!   native (rodio + symphonia, HTTP streaming with range requests) for
//!   `PlaybackSource::Stream` / `Preview`, and the visible embed mini-player
//!   for `PlaybackSource::Embed` (YouTube IFrame API, SoundCloud Widget API).
//! * All methods are non-blocking and callable from any thread (the UI thread).
//!   Work happens on the controller's own threads.
//! * Events are pulled by the UI from [`PlayerController::events`].
//! * [`PlayerController::spectrum`] returns 32 bands (0.0..=1.0) for the visuals;
//!   all zeros while an embed plays (visuals go procedural then).
//!
//! # Architecture
//!
//! One command channel feeds one player thread ([`engine`]). That thread owns
//! the rodio output, the decode threads' control blocks, the FFT analyser and
//! the embed backend, and publishes [`PlayerState`], the spectrum and
//! [`PlayerEvent`]s through shared cells.
//!
//! * **Native**: `native` (symphonia decode thread per track, HTTP via
//!   `http::HttpSource` with Range requests) -> bounded chunk channel ->
//!   `TrackSource` -> `rodio::Player`. The next track is preloaded
//!   ([`PlayerController::preload_next`]) and appended to the same rodio
//!   `Player`, which gives gapless transitions. After a gapless switch the
//!   engine emits `Ended`; the UI's following `load(next)` for the same track
//!   is recognised and ignored.
//! * **Embed**: see [`embed`] for the threading design and per-OS behaviour.
//! * **FFT**: the audio thread copies a mono tap, the player thread runs a
//!   2048-point FFT at ~60 Hz and maps it to 32 log-spaced, smoothed bands.
//!
//! # Platform requirements
//!
//! Linux: `libasound2-dev` (cpal/ALSA), `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
//! `libsoup-3.0-dev`, `libxdo-dev` (wry/tao), `pkg-config`. Windows needs the
//! WebView2 runtime (preinstalled on Windows 11). macOS needs nothing extra.

pub mod embed;
mod engine;
mod adaptive;
pub mod http;
pub mod eq;
pub mod loudness;
mod media_keys;
mod native;
pub mod spectrum;

pub use media_keys::MediaKeys;

use flume::{Receiver, Sender};
use glyph_protocol::{PlaybackSource, ProviderId, Track};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const SPECTRUM_BANDS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Idle,
    Loading,
    Playing,
    Paused,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Native,
    Embed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    pub status: Status,
    pub track: Option<Track>,
    pub provider: Option<ProviderId>,
    pub backend: Option<BackendKind>,
    pub is_preview: bool,
    pub position: Duration,
    pub duration: Option<Duration>,
    /// 0.0..=1.0
    pub volume: f32,
}

impl PlayerState {
    fn ui_changed(&self, old: &Self) -> bool {
        self.status != old.status || self.track != old.track || self.provider != old.provider
            || self.backend != old.backend || self.is_preview != old.is_preview
            || self.duration != old.duration || self.volume != old.volume
            || self.position.as_millis() / 100 != old.position.as_millis() / 100
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    StatusChanged(Status),
    /// Track finished on its own → UI advances the queue.
    Ended,
    /// Fired ~10 s before the end of a native track: UI should call
    /// `preload_next` for gapless playback.
    NearEnd,
    Error(String),
}

/// Placement for the embed mini-player window (320×180 by default).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MiniPlayerOptions {
    pub always_on_top: bool,
    /// Logical pixels, screen coordinates; `None` = bottom-right of the main window.
    pub position: Option<(f64, f64)>,
}

/// Messages to the player thread.
pub(crate) enum Cmd {
    Load(Track, PlaybackSource),
    Preload(Track, PlaybackSource),
    /// Forget the preloaded next song (the queue changed).
    DropNext,
    Play,
    Pause,
    Toggle,
    Stop,
    Seek(Duration),
    StreamQuality(String),
    Volume(f32),
    Crossfade(Duration),
    /// A-B loop of the current song (native playback): (start, end).
    Loop(Option<(Duration, Duration)>),
    /// Output device by name; None = follow the system default.
    OutputDevice(Option<String>),
    MiniOptions(MiniPlayerOptions),
    Opened { id: u64, opened: native::Opened },
    DecodeError { id: u64, message: String },
    Embed(u64, embed::EmbedEvent),
    HostAttached,
    Shutdown,
}

pub(crate) struct Shared {
    pub host: Arc<embed::host::HostQueue>,
    pub state: Mutex<PlayerState>,
    pub spectrum: Mutex<[f32; SPECTRUM_BANDS]>,
    pub events_tx: Sender<PlayerEvent>,
    pub events_rx: Receiver<PlayerEvent>,
}

pub struct PlayerController {
    cmd: Sender<Cmd>,
    shared: Arc<Shared>,
}

impl PlayerController {
    pub fn new() -> anyhow::Result<Self> {
        let (events_tx, events_rx) = flume::unbounded();
        let shared = Arc::new(Shared {
            host: Arc::new(embed::host::HostQueue::default()),
            state: Mutex::new(PlayerState {
                status: Status::Idle,
                track: None,
                provider: None,
                backend: None,
                is_preview: false,
                position: Duration::ZERO,
                duration: None,
                volume: 1.0,
            }),
            spectrum: Mutex::new([0.0; SPECTRUM_BANDS]),
            events_tx,
            events_rx,
        });
        let (cmd, rx) = flume::unbounded();
        let engine = engine::Engine::new(shared.clone(), rx, cmd.clone());
        std::thread::Builder::new()
            .name("glyph-player".into())
            .spawn(move || engine.run())?;
        Ok(Self { cmd, shared })
    }

    fn send(&self, c: Cmd) {
        let _ = self.cmd.send(c);
    }

    pub fn load(&self, track: Track, source: PlaybackSource) {
        self.send(Cmd::Load(track, source));
    }
    /// Gapless: decode/buffer the next native track ahead of time.
    pub fn preload_next(&self, track: Track, source: PlaybackSource) {
        self.send(Cmd::Preload(track, source));
    }
    /// Drops a preloaded next song so it does not start after the current one.
    pub fn drop_next(&self) {
        self.send(Cmd::DropNext);
    }
    pub fn play(&self) {
        self.send(Cmd::Play);
    }
    pub fn pause(&self) {
        self.send(Cmd::Pause);
    }
    pub fn toggle(&self) {
        self.send(Cmd::Toggle);
    }
    pub fn stop(&self) {
        self.send(Cmd::Stop);
    }
    /// Changes the active server stream; manual choices remain fixed.
    pub fn set_stream_quality(&self, quality: &str) {
        self.send(Cmd::StreamQuality(quality.into()));
    }
    pub fn seek(&self, position: Duration) {
        self.send(Cmd::Seek(position));
    }
    /// 0.0..=1.0
    pub fn set_volume(&self, volume: f32) {
        // Reflect immediately so a UI slider reads back what it just set.
        if volume.is_finite() {
            self.shared.state.lock().unwrap().volume = volume.clamp(0.0, 1.0);
        }
        self.send(Cmd::Volume(volume));
    }
    /// Overlap between native tracks (0 = gapless, no fade).
    /// 10-band equalizer (native playback), gains in dB for `eq::BANDS`.
    pub fn set_equalizer(&self, enabled: bool, gains: [f32; 10]) {
        eq::set(enabled, gains);
    }

    /// Loudness target and per-song levelling (native playback).
    pub fn set_loudness(&self, level: loudness::Level, normalize: bool) {
        loudness::set(level, normalize);
    }

    /// Names of the audio outputs (for the settings list).
    pub fn output_devices() -> Vec<String> {
        use rodio::cpal::traits::{DeviceTrait, HostTrait};
        rodio::cpal::default_host()
            .output_devices()
            .map(|it| it.filter_map(|d| d.description().ok().map(|x| x.name().to_string())).collect())
            .unwrap_or_default()
    }

    /// Plays on the named output (None = system default, followed when it changes).
    pub fn set_output_device(&self, name: Option<String>) {
        self.send(Cmd::OutputDevice(name));
    }

    /// A-B loop: at the end point playback jumps back to the start, with a
    /// short fade out and in (the crossfade length, kept short). Native
    /// playback only; cleared when the song changes.
    pub fn set_loop(&self, ab: Option<(Duration, Duration)>) {
        self.send(Cmd::Loop(ab));
    }
    pub fn set_crossfade(&self, d: Duration) {
        self.send(Cmd::Crossfade(d.min(Duration::from_secs(20))));
    }
    pub fn state(&self) -> PlayerState {
        self.shared.state.lock().unwrap().clone()
    }
    /// Avoid cloning track metadata and repainting the complete UI for every
    /// decoder position update. Control/track changes still arrive immediately.
    pub fn state_if_changed(&self, previous: Option<&PlayerState>) -> Option<PlayerState> {
        let state = self.shared.state.lock().unwrap();
        let changed = previous.is_none_or(|old| state.ui_changed(old));
        changed.then(|| state.clone())
    }

    pub fn events(&self) -> Receiver<PlayerEvent> {
        self.shared.events_rx.clone()
    }
    pub fn spectrum(&self) -> [f32; SPECTRUM_BANDS] {
        *self.shared.spectrum.lock().unwrap()
    }
    pub fn set_mini_player_options(&self, options: MiniPlayerOptions) {
        self.send(Cmd::MiniOptions(options));
    }
}

impl PlayerController {
    /// Host the embed webview as a child of the app window. Must be called on the UI/main thread.
    ///
    /// The handle must stay valid for the life of the window. Returns an error
    /// where child webviews are impossible (Wayland, missing WebView2, ...);
    /// the popup/browser fallback then stays in effect. On Windows the child is
    /// created on a background thread: this returns `Ok` immediately (never block
    /// the UI thread here) and an eventual failure is logged and falls back.
    pub fn attach_embed_host(&self, parent: raw_window_handle::RawWindowHandle) -> anyhow::Result<()> {
        embed::host::attach(&self.shared.host, parent)?;
        self.send(Cmd::HostAttached);
        Ok(())
    }

    /// Logical-pixel rect inside the parent window (top-left origin) where the embed is shown.
    /// Sizes below 200x200 (YouTube's minimum) are raised and a warning is logged.
    pub fn set_embed_bounds(&self, x: f32, y: f32, width: f32, height: f32) {
        let (b, changed) = embed::host::clamp_bounds(x, y, width, height);
        if changed {
            tracing::warn!("embed bounds {width}x{height} below the 200x200 minimum; using {}x{}", b.w, b.h);
        }
        self.shared.host.push(embed::host::HostCmd::Bounds(b));
    }

    /// Show/hide the embedded player (it is also hidden while a native track plays).
    pub fn set_embed_visible(&self, visible: bool) {
        self.shared.host.push(embed::host::HostCmd::UiVisible(visible));
    }

    /// Apply queued embed commands to the child webview. Call on the UI/main
    /// thread (the one that called `attach_embed_host`), once per frame or at
    /// least every ~16 ms while an embed is shown. Cheap when the queue is empty.
    pub fn pump_embed(&self) {
        embed::host::pump(&self.shared.host);
    }
}

impl Drop for PlayerController {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Shutdown);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKeyEvent {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    SeekForward,
    SeekBackward,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn track(id: &str, source: ProviderId) -> Track {
        Track {
            id: id.into(),
            source,
            title: "t".into(),
            artist: "a".into(),
            album: None,
            duration_ms: Some(1000),
            cover_url: None,
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false,
        }
    }

    fn wait_for(rx: &Receiver<PlayerEvent>, secs: u64, f: impl Fn(&PlayerEvent) -> bool) -> Option<PlayerEvent> {
        let end = Instant::now() + Duration::from_secs(secs);
        while let Ok(e) = rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
            if f(&e) {
                return Some(e);
            }
        }
        None
    }

    #[test]
    fn embed_for_unsupported_provider_errors() {
        let p = PlayerController::new().unwrap();
        let rx = p.events();
        p.load(
            track("x", ProviderId::Audius),
            PlaybackSource::Embed { provider: ProviderId::Audius, embed_id: "x".into() },
        );
        // Either the mini-player cannot start in this environment or the provider has no embed.
        let e = wait_for(&rx, 20, |e| matches!(e, PlayerEvent::Error(_)));
        assert!(e.is_some());
        assert_eq!(p.state().status, Status::Idle);
    }

    #[test]
    #[ignore = "requires GLYPH_TEST_AUDIO and a real output device"]
    fn starts_downloaded_audio_promptly() {
        let path = std::env::var("GLYPH_TEST_AUDIO").expect("GLYPH_TEST_AUDIO");
        let startup_timeout = if path.starts_with("http://") || path.starts_with("https://") { 20 } else { 5 };
        let p = PlayerController::new().unwrap();
        p.set_volume(0.0);
        p.set_output_device(std::env::var("GLYPH_TEST_OUTPUT_DEVICE").ok());
        let rx = p.events();
        let start = Instant::now();
        p.load(track("local-probe", ProviderId::YouTube), PlaybackSource::Stream {
            provider: ProviderId::YouTube, url: path, mime: Some("audio/mp4".into()),
        });
        p.play();
        let event = wait_for(&rx, startup_timeout, |e| matches!(e, PlayerEvent::StatusChanged(Status::Playing) | PlayerEvent::Error(_)));
        println!("audio startup after {:?}: {event:?}", start.elapsed());
        assert!(matches!(event, Some(PlayerEvent::StatusChanged(Status::Playing))), "{event:?}");
        if let Ok(seconds) = std::env::var("GLYPH_TEST_CONTINUITY_SECONDS") {
            let seconds: u64 = seconds.parse().unwrap();
            let underruns = native::TEST_UNDERRUNS.load(std::sync::atomic::Ordering::Relaxed);
            let mut previous = p.state().position;
            for _ in 0..seconds {
                std::thread::sleep(Duration::from_secs(1));
                let current = p.state().position;
                let progress = current.saturating_sub(previous);
                assert!(progress >= Duration::from_millis(750), "audio stalled: {progress:?} progress in one second at {current:?}");
                previous = current;
            }
            assert_eq!(native::TEST_UNDERRUNS.load(std::sync::atomic::Ordering::Relaxed), underruns, "decoder ran out of audio during playback");
            println!("continuous audio position advanced for {seconds} seconds with no decoder underruns");
        }
        if let Ok(seek) = std::env::var("GLYPH_TEST_SEEK_SECONDS") {
            let seek: f64 = seek.parse().unwrap();
            p.seek(Duration::from_secs_f64(seek));
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline && p.state().position.as_secs_f64() < seek + 0.25 {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(p.state().position.as_secs_f64() >= seek + 0.25, "seek did not resume audio");
            println!("seek resumed at {:?}", p.state().position);
        }
        p.stop();
    }

    #[test]
    fn volume_is_clamped_and_readable() {
        let p = PlayerController::new().unwrap();
        p.set_volume(3.0);
        assert_eq!(p.state().volume, 1.0);
        p.set_volume(0.25);
        assert_eq!(p.state().volume, 0.25);
        assert_eq!(p.spectrum(), [0.0; SPECTRUM_BANDS]);
    }

    /// Plays a generated WAV through the real pipeline. Needs an output device;
    /// without one the engine must report an error instead of hanging.
    #[test]
    fn plays_local_wav_end_to_end() {
        let dir = std::env::temp_dir().join(format!("glyph-audio-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        let rate = 44_100u32;
        let n = rate / 2;
        let mut data = Vec::new();
        for i in 0..n {
            let s = ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.3 * i16::MAX as f32) as i16;
            data.extend_from_slice(&s.to_le_bytes());
        }
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        std::fs::write(&path, wav).unwrap();

        let p = PlayerController::new().unwrap();
        p.set_volume(0.0);
        let rx = p.events();
        p.load(
            track("tone", ProviderId::Archive),
            PlaybackSource::Stream {
                provider: ProviderId::Archive,
                url: path.to_string_lossy().to_string(),
                mime: Some("audio/wav".into()),
            },
        );
        let e = wait_for(&rx, 20, |e| matches!(e, PlayerEvent::Ended | PlayerEvent::Error(_)));
        match e {
            Some(PlayerEvent::Ended) => {
                let st = p.state();
                assert_eq!(st.status, Status::Ended);
                assert!(st.duration.unwrap() >= Duration::from_millis(400));
            }
            Some(PlayerEvent::Error(m)) => eprintln!("no audio device in this environment: {m}"),
            other => panic!("unexpected: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod ui_snapshot_tests {
    use super::*;
    #[test]
    fn position_updates_are_coalesced_but_controls_and_seeks_are_immediate() {
        let old = PlayerState { status: Status::Playing, track: None, provider: None,
            backend: Some(BackendKind::Native), is_preview: false,
            position: Duration::from_millis(1000), duration: None, volume: 0.5 };
        let mut new = old.clone();
        new.position = Duration::from_millis(1099); assert!(!new.ui_changed(&old));
        new.position = Duration::from_millis(1100); assert!(new.ui_changed(&old));
        new.position = Duration::from_millis(200); assert!(new.ui_changed(&old));
        new = old.clone(); new.status = Status::Paused; assert!(new.ui_changed(&old));
        new = old.clone(); new.volume = 0.25; assert!(new.ui_changed(&old));
        new = old.clone(); new.duration = Some(Duration::from_secs(200)); assert!(new.ui_changed(&old));
    }
}
