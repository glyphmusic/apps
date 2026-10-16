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

use glyph_protocol::{PlaybackSource, ProviderId, Track};
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

pub struct PlayerController {
    // implementation detail
}

impl PlayerController {
    pub fn new() -> anyhow::Result<Self> {
        todo!()
    }
    pub fn load(&self, track: Track, source: PlaybackSource) {
        let _ = (track, source);
        todo!()
    }
    /// Gapless: decode/buffer the next native track ahead of time.
    pub fn preload_next(&self, track: Track, source: PlaybackSource) {
        let _ = (track, source);
        todo!()
    }
    pub fn play(&self) {
        todo!()
    }
    pub fn pause(&self) {
        todo!()
    }
    pub fn toggle(&self) {
        todo!()
    }
    pub fn stop(&self) {
        todo!()
    }
    pub fn seek(&self, position: Duration) {
        let _ = position;
        todo!()
    }
    /// 0.0..=1.0
    pub fn set_volume(&self, volume: f32) {
        let _ = volume;
        todo!()
    }
    pub fn state(&self) -> PlayerState {
        todo!()
    }
    pub fn events(&self) -> flume::Receiver<PlayerEvent> {
        todo!()
    }
    pub fn spectrum(&self) -> [f32; SPECTRUM_BANDS] {
        todo!()
    }
    pub fn set_mini_player_options(&self, options: MiniPlayerOptions) {
        let _ = options;
        todo!()
    }
}

/// OS media keys + now-playing info (souvlaki). Created by the app because
/// Windows needs the main window's HWND.
pub struct MediaKeys {
    // implementation detail
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

impl MediaKeys {
    /// `hwnd`: the main window handle on Windows, ignored elsewhere.
    pub fn new(hwnd: Option<*mut std::ffi::c_void>) -> anyhow::Result<Self> {
        let _ = hwnd;
        todo!()
    }
    pub fn events(&self) -> flume::Receiver<MediaKeyEvent> {
        todo!()
    }
    pub fn set_now_playing(&self, track: Option<&Track>, playing: bool, position: Duration) {
        let _ = (track, playing, position);
        todo!()
    }
}
