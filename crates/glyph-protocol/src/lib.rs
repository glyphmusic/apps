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
    /// Marked explicit by a source that knows (Deezer, Spotify). Optional on
    /// the wire, so older servers and clients keep working.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub explicit: bool,
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
    /// Accounts and sync are available (`/v1/account/*`, `/v1/sync/*`).
    /// Missing on older servers, which means "no".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub accounts: bool,
    /// The cover cache (`/v1/covers/*`) is available.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub covers: bool,
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
    /// YouTube video id other Glyph users matched to this song with their own
    /// local yt-dlp (`/v1/youtube/hint`). Only the id is shared, never audio;
    /// clients with yt-dlp play it without searching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub youtube_hint: Option<String>,
}

// ---------------------------------------------------------- /v1/youtube/hint

/// `POST /v1/youtube/hint`: "this song is that YouTube video", sent by a
/// client after its local yt-dlp matched the song with high confidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YouTubeHintRequest {
    /// The wanted song (title, artist, duration are checked against the video).
    pub track: Track,
    /// 11-character YouTube video id.
    pub video_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YouTubeHintResponse {
    /// The report was counted.
    pub accepted: bool,
    /// The song's shared id is now set (verified or enough matching reports).
    pub confirmed: bool,
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

// ---------------------------------------------------------------- accounts & sync
//
// Optional. Servers without accounts answer these routes with 404 and leave
// `HealthResponse::accounts` false. See `docs/ACCOUNTS.md` in the app repo.

/// `POST /v1/account/login/start`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginStartRequest {
    /// Shown in the device list, e.g. "Desktop (Windows)".
    pub device_name: String,
    /// `windows`, `macos`, `linux`, `android`, `ios`.
    pub platform: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginStartResponse {
    pub login_id: String,
    /// Only the starting device knows it; required to collect the tokens.
    pub poll_secret: String,
    /// `K7PX-2QMD`: shown on the new device, typed on a signed-in one.
    pub user_code: String,
    /// Browser page for passkey sign-in / sign-up.
    pub url: String,
    pub expires_in_s: u64,
}

/// `POST /v1/account/login/poll`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginPollRequest {
    pub login_id: String,
    pub poll_secret: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LoginPollResponse {
    Pending,
    /// Signed in, but the account already has `device_limit` devices: remove one
    /// (login page or another device) and keep polling.
    DeviceLimit,
    Done { session: Session },
    Expired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_in_s: u64,
    pub device_id: String,
    pub account: Account,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    /// Without the `@`, lowercase.
    pub handle: String,
    /// 5 or 10.
    pub device_limit: u8,
    pub created_at: u64,
    /// A password is set (sign-in with handle + password works).
    #[serde(default)]
    pub has_password: bool,
}

/// `POST /v1/account/token/refresh`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// `PATCH /v1/account`. Fields left out stay unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_limit: Option<u8>,
}

/// `DELETE /v1/account`: the handle typed again as confirmation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteAccountRequest {
    pub confirm_handle: String,
}

/// `GET /v1/account/handle/{handle}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandleCheck {
    pub handle: String,
    pub available: bool,
    /// `invalid`, `reserved` or `taken` when not available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `POST /v1/account/login/approve`: a signed-in device approves a pairing code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApproveRequest {
    pub user_code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApproveResponse {
    pub device_name: String,
    pub platform: String,
    /// The account is at its device limit; the new device waits until one is removed.
    #[serde(default)]
    pub device_limit_reached: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub created_at: u64,
    pub last_seen: u64,
    /// The device making the request.
    #[serde(default)]
    pub current: bool,
}

/// `GET /v1/devices`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevicesResponse {
    pub devices: Vec<Device>,
    pub device_limit: u8,
}

/// `PATCH /v1/devices/{id}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevicePatch {
    pub name: String,
}

/// Item kinds the app syncs. Servers accept any short lowercase kind, so new
/// kinds need no server change; clients ignore kinds they do not know.
pub mod sync_kind {
    pub const LIKED: &str = "liked";
    pub const PLAYLIST: &str = "playlist";
    pub const FOLLOWED: &str = "followed";
    pub const HIDDEN: &str = "hidden";
    pub const ALBUM: &str = "album";
    pub const SEARCH: &str = "search";
    pub const DOWNLOAD: &str = "download";
    pub const SETTING: &str = "setting";
    /// End-to-end encrypted on the device; the server only stores the blob.
    pub const SECRET: &str = "secret";
}

/// One synced thing. Last writer (by `updated_at`, then device id) wins per
/// `(kind, key)`; deletions are tombstones (`deleted`, no value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncItem {
    pub kind: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// Client clock, Unix ms.
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    /// Assigned by the server; 0 in pushes.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rev: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// `POST /v1/sync/push`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncPushRequest {
    pub items: Vec<SyncItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncPushResponse {
    /// The account's newest revision after this push.
    pub rev: u64,
    pub accepted: u32,
    /// Pushed items that lost against a newer server copy, with that copy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stale: Vec<SyncItem>,
}

/// `GET /v1/sync/pull?since={rev}&limit={n}`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncPullResponse {
    pub items: Vec<SyncItem>,
    /// Pass as `since` next time.
    pub rev: u64,
    /// More items are waiting; pull again right away.
    #[serde(default)]
    pub more: bool,
    /// `since` is older than the tombstone horizon: pull from 0 and push local
    /// items again.
    #[serde(default)]
    pub reset: bool,
}

/// Password sign-in. The password never leaves the device: clients derive
/// `key = Argon2id(password, salt)` with these parameters and send the key;
/// the server stores only a SHA-256 of it. Every client must use exactly these.
pub mod password_kdf {
    /// Memory in KiB (19 MiB, OWASP minimum for Argon2id).
    pub const M_KIB: u32 = 19_456;
    pub const T_COST: u32 = 2;
    pub const P_COST: u32 = 1;
    /// Key length in bytes (base64url on the wire).
    pub const KEY_LEN: usize = 32;
    /// Random salt length in bytes (base64url on the wire).
    pub const SALT_LEN: usize = 16;
    pub const MIN_PASSWORD_CHARS: usize = 10;
}

/// `GET /v1/account/password/salt/{handle}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PasswordSalt {
    pub salt: String,
}

/// `POST /v1/account/password/register`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PasswordRegisterRequest {
    pub handle: String,
    pub salt: String,
    pub key: String,
    pub device_name: String,
    pub platform: String,
}

/// `POST /v1/account/password/login`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PasswordLoginRequest {
    pub handle: String,
    pub key: String,
    pub device_name: String,
    pub platform: String,
}

/// `PUT /v1/account/password` (signed in): set or change the password.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PasswordSetRequest {
    pub salt: String,
    pub key: String,
}

/// `POST /v1/account/password/check` (signed in): is this the password key?
/// 200, or 403 `wrong_password`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PasswordCheckRequest {
    pub key: String,
}

/// Answer of a password sign-up: the session plus the one-time recovery code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignupResponse {
    pub session: Session,
    pub recovery_code: String,
}

/// `POST /v1/account/passkey/link` (signed in): browser page that adds a passkey.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkStartResponse {
    pub url: String,
    pub expires_in_s: u64,
}

/// Hosts whose images the server's cover cache will fetch. Clients route only
/// these through `/v1/covers/`; everything else is loaded directly.
pub fn cover_cache_allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase();
    if host.contains(':') || host.contains('@') || host.is_empty() {
        return false;
    }
    const SUFFIXES: &[&str] = &[
        "dzcdn.net", "deezer.com", "audius.co", "jamendo.com", "archive.org", "sndcdn.com", "ytimg.com",
        "ggpht.com", "scdn.co", "spotifycdn.com", "coverartarchive.org",
    ];
    host == "lastfm.freetls.fastly.net" || SUFFIXES.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

#[cfg(test)]
mod account_tests {
    use super::*;

    #[test]
    fn old_health_without_account_fields_parses() {
        let h: HealthResponse = serde_json::from_str(r#"{"server_version":"1","protocol_version":1,"providers":[]}"#).unwrap();
        assert!(!h.accounts && !h.covers);
        let s = serde_json::to_string(&h).unwrap();
        assert!(!s.contains("accounts"));
    }

    #[test]
    fn sync_item_roundtrip_skips_defaults() {
        let i = SyncItem { kind: "liked".into(), key: "audius:1".into(), value: None, updated_at: 5, deleted: true, rev: 0 };
        let s = serde_json::to_string(&i).unwrap();
        assert_eq!(s, r#"{"kind":"liked","key":"audius:1","updated_at":5,"deleted":true}"#);
        assert_eq!(serde_json::from_str::<SyncItem>(&s).unwrap(), i);
        let p: LoginPollResponse = serde_json::from_str(r#"{"status":"pending"}"#).unwrap();
        assert_eq!(p, LoginPollResponse::Pending);
    }

    #[test]
    fn cover_hosts() {
        assert!(cover_cache_allowed("https://e-cdns-images.dzcdn.net/images/cover/x/500x500.jpg"));
        assert!(cover_cache_allowed("https://i.ytimg.com/vi/abc/hqdefault.jpg"));
        assert!(cover_cache_allowed("https://lastfm.freetls.fastly.net/i/u/300x300/x.png"));
        assert!(!cover_cache_allowed("https://evil.fastly.net/x.png"));
        assert!(!cover_cache_allowed("http://i.ytimg.com/x.jpg"));
        assert!(!cover_cache_allowed("https://ytimg.com.evil.example/x.jpg"));
        assert!(!cover_cache_allowed("https://user@i.ytimg.com/x.jpg"));
        assert!(!cover_cache_allowed("https://i.ytimg.com:444/x.jpg"));
    }
}
