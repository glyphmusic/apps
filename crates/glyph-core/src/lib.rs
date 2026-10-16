//! GPUI-free core of Glyph.
//!
//! CONTRACT (the UI depends on exactly these public items — extend, but do not
//! rename or change signatures without updating `docs/INTERFACES.md`):
//!
//! * [`Backend`] is the single entry point the UI talks to.
//! * Every `async fn` on `Backend` must be awaitable from a NON-tokio executor
//!   (GPUI's). `Backend` owns a tokio runtime and internally spawns work on it,
//!   handing results back through channels.
//! * [`Queue`] and [`Library`] are plain synchronous data structures.

pub mod config;
pub mod library;
pub mod lyrics;
pub mod queue;
pub mod updater;

pub use config::Config;
pub use library::{Library, Playlist};
pub use lyrics::Lyrics;
pub use queue::{Queue, RepeatMode};
pub use updater::{UpdateInfo, UpdateStatus};

use futures::stream::BoxStream;
use glyph_protocol::{PlaybackSource, ProviderId, SearchEvent, Track};
use std::path::PathBuf;

/// Where metadata requests go.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendMode {
    /// Talking to a glyph-server instance.
    Server { url: String, server_version: String },
    /// No server configured: providers queried directly with local keys.
    Direct,
    /// Server answered with another protocol version; fell back to Direct.
    /// UI shows `[ SERVER v? · INKOMPATIBEL ]`.
    Incompatible { url: String, protocol_version: u32 },
}

/// Result of the resolver for one track.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub track: Track,
    pub source: PlaybackSource,
    pub confidence: f32,
}

/// One line of import progress, e.g. `IMPORT spotify/playlist ██████░░ 98/142`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportProgress {
    pub label: String,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportInput {
    /// Playlist URL pasted into the search bar.
    Url(String),
    /// CSV / Spotify JSON export / M3U / M3U8 / XSPF dropped on the window.
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportMatch {
    pub original: Track,
    pub resolved: Option<Resolved>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportResult {
    pub name: String,
    /// confidence >= 0.85
    pub sure: Vec<ImportMatch>,
    /// 0.5 ..< 0.85
    pub unsure: Vec<ImportMatch>,
    pub not_found: Vec<ImportMatch>,
}

pub struct Backend {
    // implementation detail
}

impl Backend {
    /// Loads config side effects (cache dirs), checks `/v1/health` if a server
    /// is configured and decides the [`BackendMode`].
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        let _ = config;
        todo!()
    }

    pub fn mode(&self) -> BackendMode {
        todo!()
    }

    pub fn config(&self) -> &Config {
        todo!()
    }

    /// Unified search over all enabled providers, results arrive as they come
    /// in (already de-duplicated across providers). Ends with `SearchEvent::Done`.
    pub fn search(&self, query: &str) -> BoxStream<'static, SearchEvent> {
        let _ = query;
        todo!()
    }

    /// Best legal full source (or a preview) for a track.
    pub async fn resolve(&self, track: &Track) -> anyhow::Result<Option<Resolved>> {
        let _ = track;
        todo!()
    }

    pub async fn lyrics(&self, track: &Track) -> anyhow::Result<Option<Lyrics>> {
        let _ = track;
        todo!()
    }

    /// Radio: similar tracks (Last.fm track.getSimilar), not yet resolved.
    pub async fn similar(&self, track: &Track) -> anyhow::Result<Vec<Track>> {
        let _ = track;
        todo!()
    }

    /// Local path of the (cached, size-normalised) cover, following the
    /// fallback chain provider → Spotify/Deezer → Cover Art Archive.
    /// `None` = UI draws a generated ASCII pattern.
    pub async fn cover(&self, track: &Track) -> anyhow::Result<Option<PathBuf>> {
        let _ = track;
        todo!()
    }

    pub async fn import(
        &self,
        input: ImportInput,
        progress: flume_like::Sender<ImportProgress>,
    ) -> anyhow::Result<ImportResult> {
        let _ = (input, progress);
        todo!()
    }

    /// Provider is currently skipped (error / exhausted quota, 10 min).
    pub fn provider_cooling_down(&self, provider: ProviderId) -> bool {
        let _ = provider;
        todo!()
    }

    /// Checks GitHub releases (see [`updater`]).
    pub async fn check_for_update(&self) -> anyhow::Result<UpdateStatus> {
        todo!()
    }

    /// Downloads, verifies (SHA-256) and installs the update. Restart is up to the UI.
    pub async fn install_update(
        &self,
        info: UpdateInfo,
        progress: flume_like::Sender<(u64, u64)>,
    ) -> anyhow::Result<()> {
        let _ = (info, progress);
        todo!()
    }
}

/// Progress channel type used across the API (re-export so the UI does not
/// need to pick a channel crate). Implemented with `tokio::sync::mpsc::UnboundedSender`.
pub mod flume_like {
    pub type Sender<T> = tokio::sync::mpsc::UnboundedSender<T>;
    pub type Receiver<T> = tokio::sync::mpsc::UnboundedReceiver<T>;
    pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
        tokio::sync::mpsc::unbounded_channel()
    }
}
