//! Shared wire types for the Glyph app and `glyph-server`.
//!
//! Both repos depend on this crate (the server via a pinned git tag), so any
//! change to a request or response shape is a compile error on the other side.
//! Bump [`PROTOCOL_VERSION`] on every breaking change.
//!
//! Must stay dependency-light and compile for `wasm32-unknown-unknown`
//! (Cloudflare Workers) as well as every desktop target.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Wire protocol version. The app refuses servers with a different major value.
pub const PROTOCOL_VERSION: u32 = 1;

/// Every source Glyph knows about. Serialized as lowercase strings
/// (`"audius"`, `"youtube"`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Audius,
    Jamendo,
    Archive,
    SoundCloud,
    YouTube,
    Spotify,
    Deezer,
    MusicBrainz,
    LastFm,
    LrcLib,
}

impl ProviderId {
    pub const ALL: [ProviderId; 10] = [
        Self::Audius,
        Self::Jamendo,
        Self::Archive,
        Self::SoundCloud,
        Self::YouTube,
        Self::Spotify,
        Self::Deezer,
        Self::MusicBrainz,
        Self::LastFm,
        Self::LrcLib,
    ];

    /// Two-letter ASCII badge shown next to tracks, e.g. `[AU]`.
    pub fn badge(self) -> &'static str {
        match self {
            Self::Audius => "AU",
            Self::Jamendo => "JA",
            Self::Archive => "IA",
            Self::SoundCloud => "SC",
            Self::YouTube => "YT",
            Self::Spotify => "SP",
            Self::Deezer => "DZ",
            Self::MusicBrainz => "MB",
            Self::LastFm => "LF",
            Self::LrcLib => "LR",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Audius => "Audius",
            Self::Jamendo => "Jamendo",
            Self::Archive => "Internet Archive",
            Self::SoundCloud => "SoundCloud",
            Self::YouTube => "YouTube",
            Self::Spotify => "Spotify",
            Self::Deezer => "Deezer",
            Self::MusicBrainz => "MusicBrainz",
            Self::LastFm => "Last.fm",
            Self::LrcLib => "LRCLIB",
        }
    }
}

/// What a provider can do with a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capabilities {
    MetadataOnly,
    Preview,
    FullPlayback,
}

/// A song as Glyph sees it, independent of where it came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// Provider-local id at `source`.
    pub id: String,
    pub source: ProviderId,
    pub title: String,
    pub artist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isrc: Option<String>,
    /// Link to the original on the source platform (attribution, "share").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permalink: Option<String>,
    /// Ids of the same song at other providers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub external_ids: BTreeMap<ProviderId, String>,
}

impl Track {
    /// Every provider this track is known at (source first), for `[AU][SC][YT]` badges.
    pub fn sources(&self) -> Vec<ProviderId> {
        let mut v = vec![self.source];
        v.extend(self.external_ids.keys().copied().filter(|p| *p != self.source));
        v
    }
}

/// How to actually play a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlaybackSource {
    /// Native decode (rodio/symphonia) from an official stream URL.
    Stream {
        provider: ProviderId,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
    },
    /// Official embed widget in the visible mini-player (YouTube, SoundCloud).
    Embed { provider: ProviderId, embed_id: String },
    /// Official 30 s preview, shown as `[PREVIEW 0:30]`.
    Preview { provider: ProviderId, url: String },
}

impl PlaybackSource {
    pub fn provider(&self) -> ProviderId {
        match self {
            Self::Stream { provider, .. }
            | Self::Embed { provider, .. }
            | Self::Preview { provider, .. } => *provider,
        }
    }
}

// ---------------------------------------------------------------- /v1/health

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub server_version: String,
    pub protocol_version: u32,
    pub providers: Vec<ProviderId>,
}

// ---------------------------------------------------------------- /v1/search

/// One NDJSON line of the `/v1/search?q=` stream. The stream ends with `Done`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SearchEvent {
    /// A batch of (already merged) results from one provider.
    Results { provider: ProviderId, tracks: Vec<Track> },
    /// A provider failed or timed out (4 s); the search goes on.
    ProviderError { provider: ProviderId, message: String },
    Done,
}

// ---------------------------------------------------------------- /v1/resolve

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveRequest {
    pub track: Track,
    /// Preferred order of full sources; empty = server default
    /// (Audius → Jamendo → SoundCloud → YouTube).
    #[serde(default)]
    pub priority: Vec<ProviderId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveResponse {
    /// The matched track at the chosen provider (None = nothing found).
    pub matched: Option<Track>,
    pub source: Option<PlaybackSource>,
    /// 0.0..=1.0
    pub confidence: f32,
}

// ---------------------------------------------------------------- /v1/lyrics

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricsResponse {
    pub provider: ProviderId,
    /// Line-synced lyrics; empty when only plain text exists.
    pub synced: Vec<LyricLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plain: Option<String>,
    pub instrumental: bool,
}

// ---------------------------------------------------------------- /v1/similar

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarResponse {
    pub tracks: Vec<Track>,
}

// ---------------------------------------------------------------- /v1/import/spotify

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportRequest {
    /// Public playlist link (Spotify, Deezer or YouTube).
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportResponse {
    pub name: String,
    pub source: ProviderId,
    /// Metadata only; resolving happens per track via `/v1/resolve`.
    pub tracks: Vec<Track>,
}

// ---------------------------------------------------------------- errors

/// JSON body of every non-2xx server response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_s: Option<u64>,
}

/// Header carrying the anonymous install id used for rate limiting.
pub const INSTALL_ID_HEADER: &str = "x-glyph-install-id";
