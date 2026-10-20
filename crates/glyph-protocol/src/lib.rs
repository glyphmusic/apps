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
    /// This server requires a signed-in account for API/media access.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub account_required: bool,
    /// Optional server access gate; older servers are public.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub server_auth_required: bool,
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
    /// Require YouTube, with an extended but bounded search and no preview fallback.
    #[serde(default)]
    pub youtube_only: bool,
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
    /// Approximate region/country observed by the server, never client-provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approximate_location: Option<String>,
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

// ---------------------------------------------------------------- profiles
// Picture, banner and a page other signed-in users open by handle. Private
// until the owner makes it public (server: `accounts/profile.rs`).

/// `GET|PATCH /v1/profile`: the owner's settings.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProfileSettings {
    pub public: bool,
    pub show_recent: bool,
    /// Ids of the playlists shown on the profile.
    #[serde(default)]
    pub playlists: Vec<String>,
    /// sha256 of the picture / banner (`/v1/profiles/{handle}/images/{sha}`).
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
}

/// `PATCH /v1/profile`
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProfilePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_recent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlists: Option<Vec<String>>,
}

/// `PUT /v1/profile/recent`: songs played last, newest first (at most 20 kept).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileRecent {
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfilePlaylist {
    pub id: String,
    pub name: String,
    pub songs: u32,
    #[serde(default)]
    pub cover: Option<String>,
    /// First songs, for the automatic cover mosaic when there is no cover.
    #[serde(default)]
    pub preview: Vec<Track>,
}

/// `GET /v1/profiles/{handle}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicProfile {
    pub handle: String,
    pub created_at: u64,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    /// Private: no playlists and no songs (except on the owner's own view).
    pub private: bool,
    /// The viewer's own profile.
    #[serde(default)]
    pub own: bool,
    #[serde(default)]
    pub playlists: Vec<ProfilePlaylist>,
    #[serde(default)]
    pub recent: Vec<Track>,
}

/// `GET /v1/profiles/{handle}/playlists/{id}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfilePlaylistSongs {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub cover: Option<String>,
    pub handle: String,
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
    /// A list downloads its new songs by itself (key: `liked` or a playlist id).
    pub const AUTO_DOWNLOAD: &str = "autodl";
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
        assert!(!h.accounts && !h.covers && !h.account_required);
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
#[cfg(not(target_arch = "wasm32"))]
pub mod network;
/// Finds the exact asset in a sha256sum manifest. A legacy sidecar may
/// contain only a single hash; named entries must never match another file.
pub fn checksum_for_asset(text: &str, filename: &str) -> Option<String> {
    let mut found = None;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let mut parts = line.splitn(2, char::is_whitespace);
        let hash = parts.next()?;
        let name = parts.next().map(|n| n.trim_start().trim_start_matches('*'));
        if name.is_some_and(|n| n != filename) { continue; }
        if name.is_none() && text.split_whitespace().count() != 1 { continue; }
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) || found.is_some() { return None; }
        found = Some(hash.to_ascii_lowercase());
    }
    found
}

#[cfg(test)]
mod checksum_tests {
    use super::checksum_for_asset;
    #[test]
    fn selects_exact_filename_and_rejects_missing_or_duplicate_entries() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let sums = format!("{b}  other.zip\r\n{a} *glyph.zip\r\n");
        assert_eq!(checksum_for_asset(&sums, "glyph.zip"), Some(a.clone()));
        assert_eq!(checksum_for_asset(&sums, "missing.zip"), None);
        assert_eq!(checksum_for_asset(&sums.repeat(2), "glyph.zip"), None);
        assert_eq!(checksum_for_asset(&a, "glyph.zip"), Some(a));
        assert_eq!(checksum_for_asset("bad  glyph.zip", "glyph.zip"), None);
    }
}

/// Model Context Protocol (MCP): AI assistants control the Glyph app.
///
/// The app serves MCP itself on `http://127.0.0.1:<port>/mcp` ("local"), or
/// a signed-in app is reached through the Glyph server ("server"): the
/// assistant talks to `<server>/mcp` with the user's MCP token, the server
/// queues the tool call, the app picks it up (`GET /v1/mcp/pull`), runs it
/// and posts the result (`POST /v1/mcp/result`). Both offer the same tools,
/// defined here.
pub mod mcp {
    use serde::{Deserialize, Serialize};

    /// MCP revision both ends speak (Streamable HTTP, JSON responses).
    pub const PROTOCOL_VERSION: &str = "2025-06-18";
    /// Longest wait for the app's answer to one tool call (ms).
    pub const CALL_TIMEOUT_MS: u64 = 25_000;
    /// How long one `pull` waits for a call before answering empty (ms).
    pub const PULL_WAIT_MS: u64 = 20_000;

    /// One queued tool call (server -> app).
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Call {
        pub id: String,
        pub name: String,
        #[serde(default)]
        pub arguments: serde_json::Value,
    }

    #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
    pub struct PullResponse {
        #[serde(default)]
        pub calls: Vec<Call>,
        /// Assistants asking for access (OAuth sign-in on the server's page):
        /// the app asks the user and answers with [`Decision`].
        #[serde(default)]
        pub approvals: Vec<Approval>,
    }

    /// An assistant asks to control Glyph; the same code shows on its sign-in page.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Approval {
        pub id: String,
        /// Name the assistant registered with (e.g. "Claude").
        pub client: String,
        pub code: String,
    }

    /// `POST /v1/mcp/approve`.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Decision {
        pub id: String,
        pub allow: bool,
    }

    /// The app's answer to one call (app -> server). `result` is an MCP
    /// `CallToolResult` (`content`, `structuredContent`, `isError`).
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct ResultRequest {
        pub id: String,
        pub result: serde_json::Value,
    }

    /// A fresh MCP token for the signed-in user (shown once; the server only
    /// keeps its hash) and the endpoint to give the assistant.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct TokenResponse {
        pub token: String,
        pub url: String,
    }

    fn obj(props: serde_json::Value, required: &[&str]) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": props, "required": required })
    }

    fn tool(name: &str, description: &str, schema: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "name": name, "description": description, "inputSchema": schema })
    }

    /// Every tool the app offers, as MCP `tools/list` entries.
    pub fn tools() -> Vec<serde_json::Value> {
        use serde_json::json;
        let none = || obj(json!({}), &[]);
        let song = json!({ "type": "string", "description": "Song reference from search / queue / playlist results (`ref`)." });
        let query = json!({ "type": "string", "description": "Free text, e.g. \"Daft Punk - One More Time\". Used when no ref is given." });
        vec![
            tool("now_playing", "What Glyph plays right now: song, position, volume, shuffle, repeat, queue length.", none()),
            tool("play", "Resume playback.", none()),
            tool("pause", "Pause playback.", none()),
            tool("toggle_play", "Play / pause.", none()),
            tool("next", "Skip to the next song.", none()),
            tool("previous", "Back to the previous song.", none()),
            tool("seek", "Jump to a position in the current song.", obj(json!({ "seconds": { "type": "number", "minimum": 0 } }), &["seconds"])),
            tool("set_volume", "Set the volume (0-100).", obj(json!({ "volume": { "type": "number", "minimum": 0, "maximum": 100 } }), &["volume"])),
            tool("set_shuffle", "Shuffle on / off; optionally smart (spreads artists) or random.", obj(json!({ "on": { "type": "boolean" }, "style": { "type": "string", "enum": ["smart", "random"] } }), &["on"])),
            tool("set_repeat", "Repeat mode.", obj(json!({ "mode": { "type": "string", "enum": ["off", "all", "one"] } }), &["mode"])),
            tool("search", "Search songs. Results carry a `ref` for the other tools.", obj(json!({ "query": { "type": "string" }, "limit": { "type": "integer", "minimum": 1, "maximum": 50 } }), &["query"])),
            tool("play_song", "Play a song now (by ref, or the best search hit for query).", obj(json!({ "ref": song, "query": query }), &[])),
            tool("play_artist", "Play an artist's songs.", obj(json!({ "name": { "type": "string" }, "shuffle": { "type": "boolean" } }), &["name"])),
            tool("queue_add", "Add a song to the queue (\"Up next\"); `next` puts it first.", obj(json!({ "ref": song, "query": query, "next": { "type": "boolean" } }), &[])),
            tool("get_queue", "The current queue: songs queued by hand and what plays next from the list.", none()),
            tool("list_playlists", "The user's playlists (id, name, song count).", none()),
            tool("get_playlist", "Songs of a playlist (`liked` for Liked Songs).", obj(json!({ "id": { "type": "string" } }), &["id"])),
            tool("play_playlist", "Play a playlist (`liked` for Liked Songs).", obj(json!({ "id": { "type": "string" }, "shuffle": { "type": "boolean" } }), &["id"])),
            tool("create_playlist", "Create a playlist, optionally with songs.", obj(json!({ "name": { "type": "string", "maxLength": 100 }, "refs": { "type": "array", "items": song } }), &["name"])),
            tool("rename_playlist", "Rename a playlist.", obj(json!({ "id": { "type": "string" }, "name": { "type": "string", "maxLength": 100 } }), &["id", "name"])),
            tool("delete_playlist", "Delete a playlist.", obj(json!({ "id": { "type": "string" } }), &["id"])),
            tool("add_to_playlist", "Add songs to a playlist.", obj(json!({ "id": { "type": "string" }, "refs": { "type": "array", "items": song } }), &["id", "refs"])),
            tool("remove_from_playlist", "Remove the song at `index` (0-based) from a playlist.", obj(json!({ "id": { "type": "string" }, "index": { "type": "integer", "minimum": 0 } }), &["id", "index"])),
            tool("like", "Like a song (the current one without ref).", obj(json!({ "ref": song }), &[])),
            tool("unlike", "Remove a song from Liked Songs (the current one without ref).", obj(json!({ "ref": song }), &[])),
            tool("get_lyrics", "Lyrics of the current song (plain text).", none()),
            tool("listening_stats", "Listening statistics, if the user turned them on.", obj(json!({ "range": { "type": "string", "enum": ["month", "last_month", "year", "all"] } }), &[])),
        ]
    }

    /// A text `CallToolResult` (with the data as structured content too).
    pub fn text_result(value: &serde_json::Value, is_error: bool) -> serde_json::Value {
        let text = match value {
            serde_json::Value::String(s) => s.clone(),
            v => serde_json::to_string_pretty(v).unwrap_or_default(),
        };
        let mut r = serde_json::json!({ "content": [{ "type": "text", "text": text }], "isError": is_error });
        if value.is_object() {
            r["structuredContent"] = value.clone();
        }
        r
    }

    /// Answers the protocol-level JSON-RPC methods; `None` for `tools/call`
    /// (the caller runs the tool) and notifications (no answer).
    pub fn handle_meta(method: &str, server_name: &str, version: &str) -> Option<Result<serde_json::Value, (i64, String)>> {
        match method {
            "initialize" => Some(Ok(serde_json::json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": server_name, "title": "Glyph", "version": version },
                "instructions": "Controls the Glyph music app: playback, queue, search, playlists, likes, lyrics. Songs are passed between tools by their `ref`."
            }))),
            "ping" => Some(Ok(serde_json::json!({}))),
            "tools/list" => Some(Ok(serde_json::json!({ "tools": tools() }))),
            "tools/call" => None,
            m if m.starts_with("notifications/") => None,
            _ => Some(Err((-32601, format!("method not found: {method}")))),
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn tools_are_named_uniquely_with_object_schemas() {
            let tools = super::tools();
            let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
            assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
            names.sort();
            let n = names.len();
            names.dedup();
            assert_eq!(names.len(), n);
            assert!(super::handle_meta("tools/list", "glyph", "1").unwrap().is_ok());
            assert!(super::handle_meta("tools/call", "glyph", "1").is_none());
            assert!(super::handle_meta("nope", "glyph", "1").unwrap().is_err());
        }
    }
}
