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

pub mod release;
pub mod trust;
pub use trust::{cover_cache_allowed, valid_video_id, VideoId};

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
    #[serde(default = "default_playlist_max_tracks")]
    pub playlist_max_tracks: usize,
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
    /// Features this server offers, by name (see [`capability`]). Clients
    /// test for a name instead of comparing versions; unknown names are
    /// ignored and a missing list means none of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

impl HealthResponse {
    pub fn supports(&self, name: &str) -> bool {
        self.capabilities.iter().any(|c| c == name)
    }
}

/// Names in [`HealthResponse::capabilities`].
pub mod capability {
    /// Accounts and sync (`/v1/account/*`, `/v1/sync/*`).
    pub const ACCOUNTS: &str = "accounts";
    /// The cover cache (`/v1/covers/*`).
    pub const COVERS: &str = "covers";
    /// Listening together (`/v1/jam/*`).
    pub const JAM: &str = "jam";
    /// AI assistants reach signed-in apps through `/mcp`.
    pub const MCP_RELAY: &str = "mcp_relay";
    /// Short playlist links (`/v1/share`).
    pub const SHARE: &str = "share";
    /// YouTube audio and downloads (`/v1/audio/youtube/*`).
    pub const YOUTUBE_AUDIO: &str = "youtube_audio";
    /// YouTube audio and downloads need a signed-in account on this server.
    pub const YOUTUBE_ACCOUNT_REQUIRED: &str = "youtube_account_required";
    /// Shared YouTube video ids (`/v1/youtube/hint`).
    pub const YOUTUBE_HINTS: &str = "youtube_hints";
    /// Pairing codes may carry a part only the two devices know (the server
    /// takes the first 8 characters) and the server relays
    /// [`crate::LoginStartRequest::pake`], so the approving device can hand
    /// over the account's data key without the server learning it.
    pub const PAIR_KEYS: &str = "pair_keys";
    /// [`crate::sync_kind::E2E_PLAYLIST`] items get the same revision check
    /// as plain playlists, so concurrent edits are merged, not lost.
    pub const E2E_SYNC: &str = "e2e_sync";
}

fn default_playlist_max_tracks() -> usize { 5_000 }

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
    /// The wanted song. The server looks the song up in its own catalog and
    /// checks the video against that, not against these fields.
    pub track: Track,
    /// 11-character YouTube video id; anything else fails to parse.
    pub video_id: VideoId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YouTubeHintResponse {
    /// The report was counted.
    pub accepted: bool,
    /// The song's shared id is now this video (verified by the server).
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
/// Hex SHA-256 of the device's machine id and a Glyph-only salt: survives
/// reinstalls, so an operator can ban a device; it reveals nothing else.
pub const HARDWARE_ID_HEADER: &str = "x-glyph-hardware-id";

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
    /// The new device's first pairing message (SPAKE2, base64url), keyed with
    /// a part of the pairing code the server never sees. The approving
    /// device gets it back in [`ApproveResponse::pake`] and hands over the
    /// account's data key with it (see [`capability::PAIR_KEYS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pake: Option<String>,
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
    /// [`LoginStartRequest::pake`] of the approved device, if it sent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pake: Option<String>,
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
    /// Any other item, end-to-end encrypted with the account's data key: the
    /// key is a keyed hash of the real `(kind, key)`, the value
    /// `{v, kid, n, ct}`. The server cannot read either. (Kinds are letters
    /// and `_` only; servers refuse anything else.)
    pub const E2E: &str = "sealed";
    /// An encrypted playlist; servers check its `rev` like [`PLAYLIST`].
    pub const E2E_PLAYLIST: &str = "sealed_playlist";
    /// Wrapped copies of the data key and device public keys (never the
    /// data key in the clear, except `escrow` on accounts without a password).
    pub const KEYRING: &str = "keyring";
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

/// Key that encrypts data with the account password, derived on the device
/// only. It uses Argon2id with its own parameters and secret, so no salt a
/// server hands out for [`password_kdf`] can make the sign-in key equal it.
pub mod vault_kdf {
    pub const M_KIB: u32 = 65_536;
    pub const T_COST: u32 = 3;
    pub const P_COST: u32 = 1;
    pub const KEY_LEN: usize = 32;
    /// Argon2 secret input: domain separation from the sign-in key.
    pub const SECRET: &[u8] = b"glyph-vault-v2";
    /// Salt = first 16 bytes of `SHA-256(SALT_PREFIX + account id)`.
    pub const SALT_PREFIX: &str = "glyph-vault-v2:";
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
    /// Name of the call that asks the app for the tools its user allows
    /// (`tools/list` through the server relay). The app answers
    /// `{"tools": [...]}` with [`Permissions::tools`].
    pub const LIST_TOOLS: &str = "tools/list";
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

    /// How much assistants may do in one [`Area`] (Settings › Integrations ›
    /// AI assistants). `Write` includes reading.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum Access {
        None,
        Read,
        Write,
    }

    /// Groups of tools, each with its own [`Access`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Area {
        /// What plays, the queue, lyrics and search; writing controls playback.
        Playback,
        /// Playlists and liked songs; writing changes them.
        Library,
        /// Listening statistics (read only).
        Stats,
    }

    impl Area {
        pub const ALL: [Area; 3] = [Area::Playback, Area::Library, Area::Stats];

        /// `false` for areas without tools that change anything.
        pub fn has_write(self) -> bool {
            !matches!(self, Area::Stats)
        }
    }

    /// The area of a tool and whether it changes something; `None` for a
    /// name that is not a tool.
    pub fn tool_area(name: &str) -> Option<(Area, bool)> {
        Some(match name {
            "now_playing" | "get_queue" | "get_lyrics" | "search" => (Area::Playback, false),
            "play" | "pause" | "toggle_play" | "next" | "previous" | "seek" | "set_volume" | "set_shuffle" | "set_repeat"
            | "play_song" | "play_artist" | "queue_add" | "play_playlist" => (Area::Playback, true),
            "list_playlists" | "get_playlist" => (Area::Library, false),
            "create_playlist" | "rename_playlist" | "delete_playlist" | "add_to_playlist" | "remove_from_playlist" | "like" | "unlike" => {
                (Area::Library, true)
            }
            "listening_stats" => (Area::Stats, false),
            _ => return None,
        })
    }

    /// What assistants may do. By default they control playback and search,
    /// read playlists but do not change them, and see no statistics.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(default)]
    pub struct Permissions {
        pub playback: Access,
        pub library: Access,
        pub stats: Access,
    }

    impl Default for Permissions {
        fn default() -> Self {
            Self { playback: Access::Write, library: Access::Read, stats: Access::None }
        }
    }

    impl Permissions {
        pub fn get(&self, area: Area) -> Access {
            match area {
                Area::Playback => self.playback,
                Area::Library => self.library,
                Area::Stats => self.stats,
            }
        }

        /// Sets `area`; `Write` on a read-only area counts as `Read`.
        pub fn set(&mut self, area: Area, access: Access) {
            let access = if access == Access::Write && !area.has_write() { Access::Read } else { access };
            match area {
                Area::Playback => self.playback = access,
                Area::Library => self.library = access,
                Area::Stats => self.stats = access,
            }
        }

        pub fn allows(&self, tool: &str) -> bool {
            match tool_area(tool) {
                Some((area, writes)) => match self.get(area) {
                    Access::None => false,
                    Access::Read => !writes,
                    Access::Write => true,
                },
                None => false,
            }
        }

        /// `tools/list` entries of the allowed tools.
        pub fn tools(&self) -> Vec<serde_json::Value> {
            tools().into_iter().filter(|t| t["name"].as_str().is_some_and(|n| self.allows(n))).collect()
        }
    }

    /// The tool definitions named in an app's [`LIST_TOOLS`] answer, taken
    /// from [`tools`] (an answer can only narrow the list, never add to it).
    /// `None` when the answer is not a tool list.
    pub fn listed_tools(answer: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
        let names: Vec<&str> = answer.get("tools")?.as_array()?.iter().filter_map(|t| t["name"].as_str()).collect();
        Some(tools().into_iter().filter(|t| t["name"].as_str().is_some_and(|n| names.contains(&n))).collect())
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

        #[test]
        fn every_tool_has_an_area() {
            for t in super::tools() {
                let name = t["name"].as_str().unwrap();
                assert!(super::tool_area(name).is_some(), "{name} needs an area in tool_area");
            }
            assert_eq!(super::tool_area("rm_rf"), None);
        }

        #[test]
        fn permissions_filter_tools() {
            use super::{Access, Area, Permissions};
            let p = Permissions::default();
            assert!(p.allows("play") && p.allows("search") && p.allows("list_playlists"));
            assert!(!p.allows("delete_playlist") && !p.allows("like") && !p.allows("listening_stats"));
            assert!(!p.allows("rm_rf"));
            let mut none = p;
            for area in Area::ALL {
                none.set(area, Access::None);
            }
            assert!(none.tools().is_empty());
            let mut all = p;
            for area in Area::ALL {
                all.set(area, Access::Write);
            }
            assert_eq!(all.stats, Access::Read, "statistics have no write tools");
            assert_eq!(all.tools().len(), super::tools().len());
            let mut read = p;
            read.set(Area::Playback, Access::Read);
            assert!(read.allows("now_playing") && !read.allows("pause"));
            // Old settings files without the section keep the defaults.
            let parsed: Permissions = serde_json::from_str(r#"{"library":"write"}"#).unwrap();
            assert_eq!((parsed.playback, parsed.library, parsed.stats), (Access::Write, Access::Write, Access::None));
        }

        #[test]
        fn listed_tools_only_narrow() {
            let answer = serde_json::json!({"tools":[{"name":"play"},{"name":"evil","description":"do harm"}]});
            let listed = super::listed_tools(&answer).unwrap();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0]["name"], "play");
            assert!(listed[0]["inputSchema"].is_object());
            assert!(super::listed_tools(&super::text_result(&"error".into(), true)).is_none());
        }
    }
}

// ---------------------------------------------------------------- /v1/jam

/// Jam: listening together. A signed-in host starts a jam on the server and
/// shares its code; friends (signed in too) join with it. The host's app is
/// the only one that plays the queue: it publishes what plays, where it is
/// and what comes next (`POST /v1/jam/{code}/state`) and picks up what the
/// others ask for (`GET /v1/jam/{code}/ops`). Everyone else follows the
/// published state (`GET /v1/jam/{code}?rev=`) and plays the same song at
/// the same moment on their own device (or listens at the host's).
pub mod jam {
    use crate::Track;
    use serde::{Deserialize, Serialize};

    /// A jam ends by itself after this long (ms).
    pub const MAX_AGE_MS: u64 = 12 * 60 * 60 * 1000;
    /// People in one jam, host included.
    pub const MAX_MEMBERS: usize = 20;
    /// Songs in a jam's queue.
    pub const MAX_QUEUE: usize = 200;
    /// Songs of the host's own list published after the queue.
    pub const MAX_AFTER: usize = 50;
    /// Entries of the activity feed.
    pub const MAX_FEED: usize = 30;
    /// How long `GET /v1/jam/{code}` waits for a newer state (ms).
    pub const WAIT_MS: u64 = 15_000;
    /// How long the host's `GET /v1/jam/{code}/ops` waits for requests (ms).
    pub const OPS_WAIT_MS: u64 = 15_000;
    /// The host counts as offline after this long without a sign of life (ms).
    pub const HOST_TIMEOUT_MS: u64 = 45_000;
    /// A guest may take over a jam whose host is offline this long (ms).
    pub const HANDOVER_MS: u64 = 120_000;
    /// Letters of a code (no 0/O, 1/I).
    pub const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    pub const CODE_LEN: usize = 6;
    /// Prefix of a shared jam code (`glyph:jam:K7P2QX`).
    pub const CODE_PREFIX: &str = "glyph:jam:";
    /// Link that opens the app on a jam (`glyph://jam/K7P2QX`).
    pub const APP_LINK_PREFIX: &str = "glyph://jam/";

    /// The jam code in `s`: a bare code ("k7p2qx", "K7P-2QX"), a shared
    /// `glyph:jam:` code or a `…/jam/<code>` link.
    pub fn normalize_code(s: &str) -> Option<String> {
        let s = s.trim();
        let raw = if let Some(rest) = s.strip_prefix(CODE_PREFIX) {
            rest
        } else if let Some(i) = s.find("/jam/") {
            s[i + 5..].split(['?', '#', '/']).next().unwrap_or("")
        } else {
            s
        };
        let code: String = raw.chars().filter(|c| !matches!(c, '-' | ' ')).map(|c| c.to_ascii_uppercase()).collect();
        (code.len() == CODE_LEN && code.bytes().all(|b| CODE_ALPHABET.contains(&b))).then_some(code)
    }

    /// Text to share for a jam.
    pub fn share_text(code: &str) -> String {
        format!("{CODE_PREFIX}{code}")
    }

    /// `s` is meant as a jam (code, app link or `…/jam/<code>` web link),
    /// not as a search.
    pub fn is_jam_text(s: &str) -> bool {
        let s = s.trim();
        let marked = s.starts_with(CODE_PREFIX) || s.starts_with(APP_LINK_PREFIX) || ((s.starts_with("https://") || s.starts_with("http://")) && s.contains("/jam/"));
        marked && normalize_code(s).is_some()
    }

    /// What guests may do besides listening. The host changes it any time.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Perms {
        /// Add songs to the queue (and vote).
        #[serde(default = "yes")]
        pub add: bool,
        /// Play a picked song right away instead of queueing it.
        #[serde(default)]
        pub play_now: bool,
        /// Skip to the next / previous song.
        #[serde(default)]
        pub skip: bool,
        /// Jump within the song.
        #[serde(default)]
        pub seek: bool,
        /// Play and pause.
        #[serde(default)]
        pub pause: bool,
    }

    fn yes() -> bool {
        true
    }

    impl Default for Perms {
        fn default() -> Self {
            Self { add: true, play_now: false, skip: false, seek: false, pause: false }
        }
    }

    impl Perms {
        /// Guests may send this playback request.
        pub fn allows(&self, c: &Control) -> bool {
            match c {
                Control::Play | Control::Pause => self.pause,
                Control::Next | Control::Previous => self.skip,
                Control::Seek { .. } => self.seek,
            }
        }

        /// Guests control playback in any way.
        pub fn any_control(&self) -> bool {
            self.pause || self.skip || self.seek || self.play_now
        }

        /// The host runs this request of a guest. `Join`, `Leave` and
        /// `Knock` come from the server; removing (own songs) and reactions
        /// are always fine.
        pub fn allows_op(&self, op: &Op) -> bool {
            match op {
                Op::Join | Op::Leave | Op::Knock | Op::Remove { .. } | Op::React { .. } => true,
                Op::Add { .. } | Op::Vote { .. } => self.add,
                Op::PlayNow { .. } => self.play_now,
                Op::Control { control } => self.allows(control),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Member {
        /// @handle without the `@`.
        pub handle: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub host: bool,
        /// sha256 of their profile picture (`/v1/profiles/{handle}/images/{sha}`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub avatar: Option<String>,
    }

    /// One song in the jam's queue.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Item {
        /// Stable while the song is queued (the host's song key).
        pub id: String,
        pub track: Track,
        /// Handle of who added it.
        pub by: String,
        /// Handles that voted for it.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub votes: Vec<String>,
    }

    /// What plays at the host.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Now {
        pub track: Track,
        pub position_ms: u64,
        pub playing: bool,
        /// Server time (ms) `position_ms` was true at: the host's estimate
        /// of the server clock; the server fills it in when it is missing
        /// or far off.
        #[serde(default)]
        pub at_ms: u64,
        /// The song as the host's app found it (the source it plays from).
        /// Guests try this one first, so everyone hears the same version.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub resolved: Option<Track>,
    }

    /// A reaction to the song playing.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Reaction {
        Heart,
        Fire,
        Sparkles,
        ThumbsUp,
    }

    impl Reaction {
        pub const ALL: [Reaction; 4] = [Reaction::Heart, Reaction::Fire, Reaction::Sparkles, Reaction::ThumbsUp];

        /// Lucide icon name.
        pub fn icon(self) -> &'static str {
            match self {
                Reaction::Heart => "heart",
                Reaction::Fire => "flame",
                Reaction::Sparkles => "sparkles",
                Reaction::ThumbsUp => "thumbs-up",
            }
        }
    }

    /// Something that happened in the jam (activity feed).
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Event {
        /// Server time (ms).
        pub at_ms: u64,
        /// Handle it is about.
        pub who: String,
        pub what: What,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum What {
        Joined,
        Left,
        Added { title: String },
        Played { title: String },
        React { reaction: Reaction },
        /// Took over as host.
        Host,
    }

    /// Adds `ev` to a feed, keeping the newest [`MAX_FEED`].
    pub fn push_feed(feed: &mut Vec<Event>, ev: Event) {
        feed.push(ev);
        if feed.len() > MAX_FEED {
            let cut = feed.len() - MAX_FEED;
            feed.drain(..cut);
        }
    }

    /// The whole jam as the host published it.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct State {
        pub code: String,
        /// Raised by the server on every publish.
        #[serde(default)]
        pub rev: u64,
        /// Handle of the host.
        pub host: String,
        #[serde(default)]
        pub members: Vec<Member>,
        /// What guests may do.
        #[serde(default)]
        pub perms: Perms,
        /// New members wait until the host lets them in.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub approve_joins: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub now: Option<Now>,
        #[serde(default)]
        pub queue: Vec<Item>,
        /// What the host plays after the queue (its own list), so every
        /// member's queue shows the same and can load ahead.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub after: Vec<Track>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub feed: Vec<Event>,
        /// The host's app gave a sign of life within [`HOST_TIMEOUT_MS`];
        /// the server sets it.
        #[serde(default = "yes")]
        pub host_online: bool,
        /// Server clock (ms) when this answer left; the server sets it.
        #[serde(default)]
        pub server_ms: u64,
        /// Web link to invite people (`https://glyph.my/jam/<code>`); the
        /// server sets it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub link: Option<String>,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                code: String::new(),
                rev: 0,
                host: String::new(),
                members: Vec::new(),
                perms: Perms::default(),
                approve_joins: false,
                now: None,
                queue: Vec::new(),
                after: Vec::new(),
                feed: Vec::new(),
                host_online: true,
                server_ms: 0,
                link: None,
            }
        }
    }

    impl State {
        /// Position of `now` at server time `server_now` (ms).
        pub fn position_at(&self, server_now: u64) -> Option<u64> {
            let n = self.now.as_ref()?;
            let moved = if n.playing { server_now.saturating_sub(n.at_ms) } else { 0 };
            let pos = n.position_ms + moved;
            Some(n.track.duration_ms.map_or(pos, |d| pos.min(d)))
        }
    }

    /// `POST /v1/jam/{code}/join`: in, or waiting for the host to let you in.
    #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
    pub struct JoinResponse {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub state: Option<State>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub waiting: bool,
    }

    /// Playback requests of guests (only what [`Perms`] allows).
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "action", rename_all = "snake_case")]
    pub enum Control {
        Play,
        Pause,
        Next,
        Previous,
        Seek { ms: u64 },
    }

    /// What a member asks the host for (`POST /v1/jam/{code}/op`). `Join`,
    /// `Leave` and `Knock` are queued by the server itself.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case")]
    pub enum Op {
        Join,
        Leave,
        /// Someone wants in (the host lets them in with `/admit`).
        Knock,
        Add { track: Track },
        /// Play this song right now ([`Perms::play_now`]).
        PlayNow { track: Track },
        /// Vote for (or with `up: false` take back the vote on) a queued song.
        Vote { item: String, up: bool },
        /// Take a song out again (own songs; the host decides).
        Remove { item: String },
        Control { control: Control },
        React { reaction: Reaction },
    }

    /// One request as the host gets it.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct OpFrom {
        /// Handle of the member.
        pub from: String,
        pub op: Op,
        /// Their profile picture (set by the server on `Join` and `Knock`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub avatar: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
    pub struct OpsResponse {
        #[serde(default)]
        pub ops: Vec<OpFrom>,
    }

    // Everything in a jam comes from other people's apps: the receiving app
    // treats its tracks as claims (see `crate::trust`).

    impl Now {
        pub fn sanitize(&mut self) {
            self.track.sanitize();
            if let Some(t) = self.resolved.as_mut() {
                t.sanitize();
            }
        }
    }

    impl State {
        pub fn sanitize(&mut self) {
            for item in &mut self.queue {
                item.track.sanitize();
            }
            for t in &mut self.after {
                t.sanitize();
            }
            if let Some(now) = self.now.as_mut() {
                now.sanitize();
            }
        }
    }

    impl OpsResponse {
        pub fn sanitize(&mut self) {
            for op in &mut self.ops {
                if let Op::Add { track } | Op::PlayNow { track } = &mut op.op {
                    track.sanitize();
                }
            }
        }
    }

    /// `POST /v1/jam/{code}/kick`: the host removes someone.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct KickRequest {
        pub handle: String,
    }

    /// `POST /v1/jam/{code}/admit`: the host lets someone in (or not).
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct AdmitRequest {
        pub handle: String,
        pub allow: bool,
    }

    /// `GET /v1/jam/{code}/time`: the server clock, for clock sync.
    #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
    pub struct TimeResponse {
        pub server_ms: u64,
    }

    /// `GET /v1/jam/{code}/live` (WebSocket, optional): the server pushes
    /// changes instead of members asking for them. Without it (older or
    /// self-hosted servers, or the free push budget used up) the apps long
    /// poll `GET /v1/jam/{code}` and `/ops` as before.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "t", rename_all = "snake_case")]
    pub enum Live {
        /// The host published (members).
        State { state: Box<State> },
        /// Only the position moved (`POST /v1/jam/{code}/now`): the rest
        /// of the state stays.
        Now { rev: u64, now: Now, server_ms: u64 },
        /// Requests wait for the host: `GET /v1/jam/{code}/ops?wait=0`.
        Ops,
        /// The host's app connected or went away.
        Host { online: bool },
        /// No more pushes for now: long poll instead.
        Fallback,
    }

    /// Keepalive the apps send; the server answers [`LIVE_PONG`].
    pub const LIVE_PING: &str = "ping";
    pub const LIVE_PONG: &str = "pong";
    /// How often the apps send [`LIVE_PING`] (ms).
    pub const LIVE_PING_MS: u64 = 50_000;
    /// A host on push still asks for requests this often (ms): its sign of
    /// life, and requests whose push got lost.
    pub const LIVE_HOST_EVERY_MS: u64 = 30_000;
    /// Close code: connect again (someone took over as host).
    pub const LIVE_CLOSE_AGAIN: u16 = 4000;
    /// Close code: the jam ended.
    pub const LIVE_CLOSE_ENDED: u16 = 4001;
    /// Close code: removed from the jam.
    pub const LIVE_CLOSE_REMOVED: u16 = 4003;

    #[cfg(test)]
    mod tests {
        use super::*;

        fn track() -> Track {
            Track {
                id: "1".into(),
                source: crate::ProviderId::Audius,
                title: "t".into(),
                artist: "a".into(),
                album: None,
                duration_ms: Some(60_000),
                cover_url: None,
                isrc: None,
                permalink: None,
                external_ids: Default::default(),
                explicit: false,
            }
        }

        #[test]
        fn codes_are_found_in_links_and_shared_text() {
            assert_eq!(normalize_code("k7p2qx").as_deref(), Some("K7P2QX"));
            assert_eq!(normalize_code(" K7P-2QX ").as_deref(), Some("K7P2QX"));
            assert_eq!(normalize_code("glyph:jam:K7P2QX").as_deref(), Some("K7P2QX"));
            assert_eq!(normalize_code("https://glyph.my/jam/K7P2QX?x=1").as_deref(), Some("K7P2QX"));
            assert_eq!(normalize_code("K7P2Q0"), None);
            assert_eq!(normalize_code("K7P2Q"), None);
            assert_eq!(normalize_code("hello world"), None);
            assert_eq!(normalize_code("glyph://jam/K7P2QX").as_deref(), Some("K7P2QX"));
            assert!(is_jam_text("glyph:jam:K7P2QX") && is_jam_text("glyph://jam/k7p2qx") && is_jam_text("https://glyph.my/jam/K7P2QX"));
            // A bare code is no search query.
            assert!(!is_jam_text("K7P2QX") && !is_jam_text("glyph:jam:nope"));
        }

        #[test]
        fn perms_default_to_adding_only() {
            let p: Perms = serde_json::from_str("{}").unwrap();
            assert_eq!(p, Perms::default());
            assert!(p.add && !p.any_control() && !p.allows(&Control::Next));
            assert!(p.allows_op(&Op::Add { track: track() }) && !p.allows_op(&Op::PlayNow { track: track() }));
            assert!(p.allows_op(&Op::React { reaction: Reaction::Fire }));
            let p = Perms { seek: true, add: false, ..p };
            assert!(p.allows(&Control::Seek { ms: 1 }) && !p.allows(&Control::Pause));
            assert!(!p.allows_op(&Op::Vote { item: "x".into(), up: true }));
        }

        #[test]
        fn position_moves_only_while_playing() {
            let mut s = State { now: Some(Now { track: track(), position_ms: 10_000, playing: true, at_ms: 1_000, resolved: None }), ..Default::default() };
            assert_eq!(s.position_at(3_000), Some(12_000));
            assert_eq!(s.position_at(500_000), Some(60_000));
            s.now.as_mut().unwrap().playing = false;
            assert_eq!(s.position_at(3_000), Some(10_000));
        }

        #[test]
        fn old_states_read_with_defaults() {
            let s: State = serde_json::from_str(r#"{"code":"K7P2QX","host":"a"}"#).unwrap();
            assert!(s.host_online && s.perms.add && s.feed.is_empty());
        }

        #[test]
        fn feed_keeps_the_newest() {
            let mut f = Vec::new();
            for i in 0..(MAX_FEED as u64 + 5) {
                push_feed(&mut f, Event { at_ms: i, who: "a".into(), what: What::Joined });
            }
            assert_eq!(f.len(), MAX_FEED);
            assert_eq!(f[0].at_ms, 5);
        }

        #[test]
        fn ops_round_trip() {
            let o = OpFrom { from: "ana".into(), op: Op::Control { control: Control::Seek { ms: 5 } }, avatar: None };
            let j = serde_json::to_string(&o).unwrap();
            assert_eq!(serde_json::from_str::<OpFrom>(&j).unwrap(), o);
            let v: Op = serde_json::from_str(r#"{"op":"vote","item":"x","up":true}"#).unwrap();
            assert_eq!(v, Op::Vote { item: "x".into(), up: true });
            let r: Op = serde_json::from_str(r#"{"op":"react","reaction":"thumbs_up"}"#).unwrap();
            assert_eq!(r, Op::React { reaction: Reaction::ThumbsUp });
            let e = Event { at_ms: 1, who: "a".into(), what: What::Added { title: "x".into() } };
            assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&e).unwrap()).unwrap(), e);
        }

        #[test]
        fn live_messages_are_tagged() {
            assert_eq!(serde_json::to_string(&Live::Ops).unwrap(), r#"{"t":"ops"}"#);
            assert_eq!(serde_json::from_str::<Live>(r#"{"t":"host","online":false}"#).unwrap(), Live::Host { online: false });
            let s = Live::State { state: Box::new(State { code: "K7P2QX".into(), rev: 3, ..Default::default() }) };
            assert_eq!(serde_json::from_str::<Live>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
            let n = Live::Now { rev: 4, now: Now { track: track(), position_ms: 9, playing: true, at_ms: 5, resolved: None }, server_ms: 6 };
            assert_eq!(serde_json::from_str::<Live>(&serde_json::to_string(&n).unwrap()).unwrap(), n);
        }
    }
}
pub mod playback;
