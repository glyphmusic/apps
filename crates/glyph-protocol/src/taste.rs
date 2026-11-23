//! Shared wire types, canonical genre/mood/era normalization, sonic-similarity
//! tables, and intent signal structures for Glyph's server-first music taste
//! engine (server-powered by default, with strict local-only opt-out).

use crate::Track;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

fn default_true() -> bool {
    true
}

// ------------------------------------------------ tag enrichment wire types

/// Batch request for normalized genre/mood/era/similar tags (`POST /v1/taste/tags`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteTagsRequest {
    #[serde(default)]
    pub artists: Vec<String>,
}

/// Enriched metadata for one artist, cached globally on the server.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistTags {
    pub artist: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moods: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub eras: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub similar: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteTagsResponse {
    #[serde(default)]
    pub items: Vec<ArtistTags>,
}

// ------------------------------------------------ collaborative wire types

/// Anonymized artist affinity sent to `/v1/taste/collab` & `/v1/taste/recommend`.
/// Never contains user handles, raw timestamps, or raw playback history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TasteArtistSignal {
    pub artist: String,
    /// Normalized long-term preference strength in `0.0..=1.0`.
    pub weight: f32,
    /// Optional short-term / recent phase strength in `0.0..=1.0` (last ~7 days).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_weight: Option<f32>,
}

/// Signed genre, mood, or era affinity (`-1.0..=1.0`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TasteTagSignal {
    pub tag: String,
    pub score: f32,
}

/// Request for collaborative artist recommendations (`POST /v1/taste/collab`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteCollabRequest {
    /// User's top liked/listened artists with normalized weights.
    #[serde(default)]
    pub positive: Vec<TasteArtistSignal>,
    /// Artists to exclude from recommendations (hidden, disliked, high-skip).
    #[serde(default)]
    pub negative: Vec<String>,
    /// Genre/mood tags the user avoids (penalized/filtered).
    #[serde(default)]
    pub avoided_tags: Vec<String>,
    /// Optional signed genre affinities for genre-bridge scoring.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genre_scores: Vec<TasteTagSignal>,
    /// Optional signed mood affinities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mood_scores: Vec<TasteTagSignal>,
    /// Anonymized artist pairs completed in the same listening session
    /// (stronger vibe-transition signal than static library overlap).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_pairs: Vec<(String, String)>,
    /// Max recommended artists to return (default 15, max 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Whether the server may incorporate this anonymized profile into the
    /// community co-occurrence graph.
    #[serde(default = "default_true")]
    pub contribute: bool,
}

/// One artist recommended by collaborative filtering / community co-occurrence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollabArtistRec {
    pub artist: String,
    /// Recommendation strength (`> 0.0`).
    pub score: f32,
    /// Up to 3 seed artists from the user's profile that drove this match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub because_of: Vec<String>,
    /// Genres/tags associated with this recommended artist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    /// Whether this recommendation came from Glyph community co-occurrence
    /// (`true`) or metadata/related-artist cold-start fallback (`false`).
    #[serde(default)]
    pub collaborative: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteCollabResponse {
    #[serde(default)]
    pub recommendations: Vec<CollabArtistRec>,
    /// Approximate number of unique community contributors reflected in matched seeds.
    #[serde(default)]
    pub contributors: u32,
    /// True when collaborative co-occurrence data contributed directly.
    #[serde(default)]
    pub model_ready: bool,
}

// ------------------------------------------------ server-first engine wire types

/// Which surface requested recommendations from `POST /v1/taste/recommend`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TastePurpose {
    /// Home feed recommendations, Daily Mixes, collab recs, and tag enrichment.
    #[default]
    Home,
    /// Session-aware Radio / Autoplay continuation from seed tracks.
    Radio,
    /// Focused fresh discovery list.
    Discovery,
}

/// Real-time inferred user listening intention.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListeningMode {
    #[default]
    Balanced,
    /// Exploring new artists, searching broadly, playing radio/discovery.
    Discovery,
    /// Replaying favorites, rewinding, listening to liked songs / playlists.
    Comfort,
    /// Long uninterrupted completions with focus/ambient/instrumental/lo-fi cues.
    DeepFocus,
    /// Multiple recent plays concentrated on 1–2 artists.
    ArtistDive,
    /// Consecutive rapid skips: current vibe isn't landing, pivot deliberately.
    VibeReset,
    /// High-energy / upbeat / party / workout trajectory.
    HighEnergy,
    /// Late-night / calm / atmospheric wind-down trajectory.
    LateNightWindDown,
}

impl ListeningMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Discovery => "discovery",
            Self::Comfort => "comfort",
            Self::DeepFocus => "deep_focus",
            Self::ArtistDive => "artist_dive",
            Self::VibeReset => "vibe_reset",
            Self::HighEnergy => "high_energy",
            Self::LateNightWindDown => "late_night_wind_down",
        }
    }
}

/// Outcome of a single recent session event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventOutcome {
    #[default]
    Played,
    Completed,
    Replayed,
    Queued,
    Liked,
    Unliked,
    SkippedEarly,
    SkippedMid,
}

/// Where the user started a track from, used to interpret skip/play intention.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventContext {
    #[default]
    Unknown,
    /// User deliberately picked a specific track (artist page, album, home card).
    Explicit,
    /// Playing from user's own Liked songs or custom playlist.
    Library,
    /// Passive Radio / Autoplay / Daily Mix / Recommendations.
    Radio,
    /// Picked from Search results.
    Search,
}

/// One compact, anonymized event from the user's current listening session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionEventSignal {
    pub title: String,
    pub artist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default)]
    pub outcome: SessionEventOutcome,
    #[serde(default)]
    pub context: SessionEventContext,
    /// Minutes ago (`0..=720`), quantized so raw wall-clock timestamps are never sent.
    #[serde(default)]
    pub age_mins: u16,
}

/// Current listening session snapshot for real-time intent understanding.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteSessionContext {
    /// Most recent events first (up to 16, within the active session window).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_events: Vec<SessionEventSignal>,
    /// Canonical genres extracted from recent search queries (never raw query text).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query_genres: Vec<String>,
    /// Canonical moods extracted from recent search queries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query_moods: Vec<String>,
    /// Canonical eras extracted from recent search queries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query_eras: Vec<String>,
    /// Current consecutive manual skip count.
    #[serde(default)]
    pub recent_skip_streak: u8,
    /// Local hour (`0..=23`).
    #[serde(default)]
    pub hour: u8,
    /// Whether today is Saturday or Sunday locally.
    #[serde(default)]
    pub is_weekend: bool,
}

/// A seed track sent to the server engine for candidate retrieval & radio.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteSeedTrack {
    pub title: String,
    pub artist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default = "default_one")]
    pub weight: f32,
    /// True when this seed came from the active session or explicit radio trigger.
    #[serde(default)]
    pub is_session: bool,
}

fn default_one() -> f32 {
    1.0
}

/// Request to the server-side Taste Engine (`POST /v1/taste/recommend`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteRecommendRequest {
    #[serde(default)]
    pub purpose: TastePurpose,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positive_artists: Vec<TasteArtistSignal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub negative_artists: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seed_tracks: Vec<TasteSeedTrack>,
    /// Optional compact sample of familiar tracks so the server can blend
    /// 50/50 familiar + fresh Daily Mixes for `purpose = Home`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub familiar_tracks: Vec<Track>,
    /// Normalized `title|artist` song keys already known/recently played.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_track_keys: Vec<String>,
    /// Normalized `title|artist` song keys explicitly disliked or heavily skipped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disliked_track_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genre_scores: Vec<TasteTagSignal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mood_scores: Vec<TasteTagSignal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub era_scores: Vec<TasteTagSignal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub avoided_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub version_scores: BTreeMap<String, f32>,
    /// Anonymized artist pairs completed in the same listening session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_pairs: Vec<(String, String)>,
    #[serde(default)]
    pub session: TasteSessionContext,
    /// User's natural artist variety preference (`0.0..=1.0`).
    #[serde(default)]
    pub variety_preference: f32,
    /// Local date key (`YYYY-MM-DD`) for deterministic daily rotation.
    #[serde(default)]
    pub date_key: String,
    #[serde(default)]
    pub hide_explicit: bool,
    /// Optional ISO-2 country code (e.g. `"DE"`) for subtle regional lift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// Whether anonymized signals may refine the community co-occurrence graph.
    #[serde(default = "default_true")]
    pub contribute: bool,
}

/// One scored and explained track returned by the Taste Engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecommendedTrack {
    pub track: Track,
    pub score: f32,
    /// Machine-readable reason key (`"session_vibe"`, `"collab"`, `"because_of"`,
    /// `"genre_bridge"`, `"mood_match"`, `"vibe_reset"`, `"comfort"`, `"discovery"`).
    #[serde(default)]
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub because_of: Vec<String>,
}

/// Personalized mix card shared between server and local engines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyMix {
    pub id: String,
    /// `"intent"`, `"genre"`, `"time"`, or `"discovery"`.
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Deterministic hue `0..360` for subtle card tinting.
    pub accent_hue: u16,
    pub tracks: Vec<Track>,
}

/// Real-time inferred user intention snapshot surfaced in UI and used in scoring.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InferredIntent {
    #[serde(default)]
    pub mode: ListeningMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub focus_genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub focus_moods: Vec<String>,
    /// Target energy level in `0.0..=1.0` (0 = calm/ambient, 1 = intense/high-energy).
    #[serde(default = "default_half")]
    pub energy: f32,
    /// Exploration vs comfort balance in `0.0..=1.0` (0 = comfort, 1 = pure discovery).
    #[serde(default = "default_half")]
    pub novelty: f32,
    /// True when recent consecutive skips triggered an intentional vibe shift.
    #[serde(default)]
    pub vibe_pivot: bool,
}

fn default_half() -> f32 {
    0.5
}

/// Complete response from `POST /v1/taste/recommend`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteRecommendResponse {
    #[serde(default)]
    pub recommendations: Vec<RecommendedTrack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mixes: Vec<DailyMix>,
    #[serde(default)]
    pub collab: TasteCollabResponse,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artist_tags: Vec<ArtistTags>,
    #[serde(default)]
    pub intent: InferredIntent,
    /// `"server"` when computed by the server Taste Engine.
    #[serde(default)]
    pub engine: String,
}

// ------------------------------------------------ onboarding catalog

/// One starter artist offered during onboarding (`GET /v1/taste/onboarding`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OnboardingArtist {
    pub name: String,
    pub genre: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_track: Option<Track>,
}

/// Starter artists and songs offered in the onboarding taste picker.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OnboardingCatalogResponse {
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub artists: Vec<OnboardingArtist>,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

// ------------------------------------------------ canonical tag & era normalization

/// Noise tags frequently returned by community taggers (Last.fm, MusicBrainz)
/// that describe listener bookkeeping rather than musical sound.
const NOISE_TAGS: &[&str] = &[
    "seen live",
    "favorites",
    "favourites",
    "favorite",
    "favourite",
    "loved",
    "awesome",
    "good",
    "great",
    "best",
    "spotify",
    "albums i own",
    "my music",
    "under 2000 listeners",
    "all",
    "music",
    "male vocalists",
    "female vocalists",
    "singer",
    "band",
    "american",
    "british",
    "uk",
    "usa",
    "german",
    "french",
    "swedish",
    "canadian",
    "australian",
    "japanese",
    "korean",
    "russian",
    "polish",
    "spanish",
    "italian",
    "brazilian",
    "check out",
    "to listen",
    "fip",
    "radio",
    "2000s",
    "2010s",
    "2020s",
    "90s",
    "80s",
    "70s",
    "60s",
    "00s",
    "10s",
    "20s",
];

/// Canonical decade/era extraction rules (including German decade notation like `90er`).
const ERA_RULES: &[(&str, &str)] = &[
    ("1960s", "60s"),
    ("60s", "60s"),
    ("60er", "60s"),
    ("1970s", "70s"),
    ("70s", "70s"),
    ("70er", "70s"),
    ("1980s", "80s"),
    ("80s", "80s"),
    ("80er", "80s"),
    ("1990s", "90s"),
    ("90s", "90s"),
    ("90er", "90s"),
    ("2000s", "2000s"),
    ("00s", "2000s"),
    ("2000er", "2000s"),
    ("2010s", "2010s"),
    ("10s", "2010s"),
    ("2010er", "2010s"),
    ("2020s", "2020s"),
    ("20s", "2020s"),
    ("2020er", "2020s"),
];

/// Canonical genre rules: `(needle, canonical_label)`.
/// Checked longest/most-specific first.
const GENRE_RULES: &[(&str, &str)] = &[
    ("drum and bass", "Drum & Bass"),
    ("drum & bass", "Drum & Bass"),
    ("dnb", "Drum & Bass"),
    ("liquid funk", "Drum & Bass"),
    ("liquid dnb", "Drum & Bass"),
    ("breakcore", "Breakcore"),
    ("jungle", "Drum & Bass"),
    ("psychedelic rock", "Psychedelic"),
    ("psychedelic pop", "Psychedelic"),
    ("neo-psychedelia", "Psychedelic"),
    ("psychedelic", "Psychedelic"),
    ("krautrock", "Psychedelic"),
    ("dream pop", "Dream Pop"),
    ("dreampop", "Dream Pop"),
    ("shoegaze", "Shoegaze"),
    ("synthpop", "Synthpop"),
    ("synth-pop", "Synthpop"),
    ("synth pop", "Synthpop"),
    ("synthwave", "Synthwave"),
    ("retrowave", "Synthwave"),
    ("darkwave", "Darkwave"),
    ("coldwave", "Darkwave"),
    ("chillwave", "Chillwave"),
    ("vaporwave", "Vaporwave"),
    ("post-punk", "Post-Punk"),
    ("post punk", "Post-Punk"),
    ("post-rock", "Post-Rock"),
    ("post rock", "Post-Rock"),
    ("math rock", "Indie Rock"),
    ("midwest emo", "Emo"),
    ("indie rock", "Indie Rock"),
    ("indie pop", "Indie Pop"),
    ("indie folk", "Indie Folk"),
    ("bedroom pop", "Indie Pop"),
    ("alternative rock", "Alternative"),
    ("alt-rock", "Alternative"),
    ("alt rock", "Alternative"),
    ("alternative metal", "Metal"),
    ("alternative r&b", "R&B"),
    ("alt r&b", "R&B"),
    ("alternative", "Alternative"),
    ("indie", "Indie"),
    ("hip-hop", "Hip-Hop"),
    ("hip hop", "Hip-Hop"),
    ("hiphop", "Hip-Hop"),
    ("boom bap", "Hip-Hop"),
    ("conscious hip hop", "Hip-Hop"),
    ("deutschrap", "Rap"),
    ("german rap", "Rap"),
    ("german hip-hop", "Hip-Hop"),
    ("german hip hop", "Hip-Hop"),
    ("cloud rap", "Trap & Cloud Rap"),
    ("melodic rap", "Rap"),
    ("trap", "Trap"),
    ("uk drill", "Drill"),
    ("drill", "Drill"),
    ("grime", "Grime"),
    ("rap", "Rap"),
    ("r&b", "R&B"),
    ("rnb", "R&B"),
    ("rhythm and blues", "R&B"),
    ("contemporary r&b", "R&B"),
    ("neo-soul", "Soul"),
    ("neo soul", "Soul"),
    ("soul", "Soul"),
    ("funk", "Funk"),
    ("disco", "Disco"),
    ("nu-disco", "Disco"),
    ("afro house", "House"),
    ("melodic house", "House"),
    ("deep house", "House"),
    ("tech house", "House"),
    ("french house", "House"),
    ("progressive house", "House"),
    ("house", "House"),
    ("hard techno", "Techno"),
    ("melodic techno", "Techno"),
    ("acid techno", "Techno"),
    ("minimal techno", "Techno"),
    ("hypertechno", "Techno"),
    ("techno", "Techno"),
    ("hardstyle", "Hardstyle"),
    ("trance", "Trance"),
    ("psytrance", "Trance"),
    ("future bass", "Electronic"),
    ("dubstep", "Dubstep"),
    ("uk garage", "UK Garage"),
    ("2-step", "UK Garage"),
    ("garage", "UK Garage"),
    ("idm", "IDM"),
    ("downtempo", "Downtempo"),
    ("trip-hop", "Trip-Hop"),
    ("trip hop", "Trip-Hop"),
    ("ambient", "Ambient"),
    ("dark ambient", "Ambient"),
    ("lo-fi", "Lo-Fi"),
    ("lofi", "Lo-Fi"),
    ("chillhop", "Lo-Fi"),
    ("edm", "Electronic"),
    ("electropop", "Electropop"),
    ("dark pop", "Electropop"),
    ("electro", "Electronic"),
    ("electronic", "Electronic"),
    ("electronica", "Electronic"),
    ("hyperpop", "Hyperpop"),
    ("k-pop", "K-Pop"),
    ("kpop", "K-Pop"),
    ("k-r&b", "R&B"),
    ("j-pop", "J-Pop"),
    ("jpop", "J-Pop"),
    ("j-rock", "Rock"),
    ("anime", "J-Pop"),
    ("city pop", "City Pop"),
    ("deutschpop", "Pop"),
    ("german pop", "Pop"),
    ("pop rock", "Pop"),
    ("art pop", "Art Pop"),
    ("chamber pop", "Indie Pop"),
    ("pop", "Pop"),
    ("heavy metal", "Metal"),
    ("death metal", "Metal"),
    ("black metal", "Metal"),
    ("thrash metal", "Metal"),
    ("nu metal", "Metal"),
    ("nu-metal", "Metal"),
    ("metalcore", "Metalcore"),
    ("deathcore", "Metalcore"),
    ("progressive metal", "Metal"),
    ("doom metal", "Metal"),
    ("metal", "Metal"),
    ("hardcore punk", "Punk"),
    ("pop punk", "Punk"),
    ("pop-punk", "Punk"),
    ("punk rock", "Punk"),
    ("skate punk", "Punk"),
    ("punk", "Punk"),
    ("emo", "Emo"),
    ("grunge", "Grunge"),
    ("stoner rock", "Rock"),
    ("deutschrock", "Rock"),
    ("hard rock", "Rock"),
    ("classic rock", "Rock"),
    ("progressive rock", "Rock"),
    ("prog rock", "Rock"),
    ("blues rock", "Rock"),
    ("garage rock", "Rock"),
    ("folk rock", "Folk & Acoustic"),
    ("rock", "Rock"),
    ("jazz fusion", "Jazz"),
    ("nu jazz", "Jazz"),
    ("smooth jazz", "Jazz"),
    ("jazz rap", "Hip-Hop"),
    ("jazz", "Jazz"),
    ("blues", "Blues"),
    ("classical", "Classical"),
    ("neoclassical", "Classical"),
    ("orchestral", "Classical"),
    ("soundtrack", "Soundtrack"),
    ("ost", "Soundtrack"),
    ("film score", "Soundtrack"),
    ("video game music", "Soundtrack"),
    ("piano", "Instrumental"),
    ("instrumental", "Instrumental"),
    ("singer-songwriter", "Folk & Acoustic"),
    ("singer songwriter", "Folk & Acoustic"),
    ("acoustic", "Folk & Acoustic"),
    ("folk", "Folk & Acoustic"),
    ("country", "Country"),
    ("americana", "Country"),
    ("reggae", "Reggae"),
    ("dub", "Reggae"),
    ("reggaeton", "Latin"),
    ("latin", "Latin"),
    ("bossa nova", "Bossa Nova"),
    ("afrobeats", "Afrobeats"),
    ("afrobeat", "Afrobeats"),
    ("afropop", "Afrobeats"),
    ("amapiano", "Afrobeats"),
    ("dancehall", "Dancehall"),
    ("dance", "Dance"),
    ("phonk", "Phonk"),
    ("drift phonk", "Phonk"),
    ("gospel", "Soul"),
    ("new wave", "New Wave"),
    ("britpop", "Indie Rock"),
];

/// Mood rules mapping raw tags or search query words (EN + DE) to user-facing moods.
const MOOD_TAG_RULES: &[(&str, &str)] = &[
    ("chill", "Chill"),
    ("chillout", "Chill"),
    ("relax", "Chill"),
    ("entspannt", "Chill"),
    ("ruhig", "Chill"),
    ("mellow", "Chill"),
    ("calm", "Chill"),
    ("laid-back", "Chill"),
    ("melanchol", "Melancholic"),
    ("sad", "Melancholic"),
    ("traurig", "Melancholic"),
    ("bittersweet", "Melancholic"),
    ("depressive", "Melancholic"),
    ("heartbreak", "Melancholic"),
    ("dreamy", "Dreamy"),
    ("verträumt", "Dreamy"),
    ("ethereal", "Dreamy"),
    ("hazy", "Dreamy"),
    ("energetic", "Energetic"),
    ("energie", "Energetic"),
    ("upbeat", "Upbeat"),
    ("happy", "Upbeat"),
    ("fröhlich", "Upbeat"),
    ("feel good", "Upbeat"),
    ("gute laune", "Upbeat"),
    ("summer", "Upbeat"),
    ("sommer", "Upbeat"),
    ("party", "Energetic"),
    ("workout", "Energetic"),
    ("training", "Energetic"),
    ("gym", "Energetic"),
    ("hype", "Energetic"),
    ("aggressive", "Intense"),
    ("heavy", "Intense"),
    ("hart", "Intense"),
    ("angry", "Intense"),
    ("dark", "Dark"),
    ("dunkel", "Dark"),
    ("düster", "Dark"),
    ("gothic", "Dark"),
    ("moody", "Dark"),
    ("noir", "Dark"),
    ("atmospheric", "Atmospheric"),
    ("cinematic", "Atmospheric"),
    ("space", "Atmospheric"),
    ("ambient", "Atmospheric"),
    ("focus", "Focus"),
    ("fokus", "Focus"),
    ("study", "Focus"),
    ("lernen", "Focus"),
    ("coding", "Focus"),
    ("konzentration", "Focus"),
    ("work", "Focus"),
    ("night", "Late Night"),
    ("nacht", "Late Night"),
    ("nocturnal", "Late Night"),
    ("midnight", "Late Night"),
    ("late night", "Late Night"),
    ("groovy", "Groovy"),
    ("funky", "Groovy"),
    ("danceable", "Groovy"),
    ("nostalgic", "Nostalgic"),
    ("nostalg", "Nostalgic"),
    ("retro", "Nostalgic"),
    ("vintage", "Nostalgic"),
    ("warm", "Warm"),
    ("cozy", "Warm"),
    ("gemütlich", "Warm"),
    ("intimate", "Warm"),
    ("romantic", "Warm"),
];

/// Default mood associations inferred from canonical genres when explicit mood
/// tags are absent.
const GENRE_MOODS: &[(&str, &[&str])] = &[
    ("Dream Pop", &["Dreamy", "Atmospheric"]),
    ("Shoegaze", &["Dreamy", "Atmospheric"]),
    ("Psychedelic", &["Dreamy", "Groovy"]),
    ("Synthwave", &["Late Night", "Nostalgic"]),
    ("Darkwave", &["Dark", "Late Night"]),
    ("Chillwave", &["Chill", "Dreamy"]),
    ("Vaporwave", &["Nostalgic", "Dreamy"]),
    ("Lo-Fi", &["Chill", "Focus"]),
    ("Ambient", &["Atmospheric", "Focus"]),
    ("Downtempo", &["Chill", "Late Night"]),
    ("Trip-Hop", &["Dark", "Late Night"]),
    ("House", &["Groovy", "Energetic"]),
    ("Techno", &["Energetic", "Dark"]),
    ("Trance", &["Energetic", "Atmospheric"]),
    ("Hardstyle", &["Intense", "Energetic"]),
    ("Drum & Bass", &["Energetic", "Intense"]),
    ("Breakcore", &["Intense", "Energetic"]),
    ("Dubstep", &["Intense", "Dark"]),
    ("UK Garage", &["Groovy", "Late Night"]),
    ("IDM", &["Atmospheric", "Focus"]),
    ("Electronic", &["Energetic", "Atmospheric"]),
    ("Synthpop", &["Upbeat", "Nostalgic"]),
    ("Electropop", &["Upbeat", "Energetic"]),
    ("Hyperpop", &["Energetic", "Intense"]),
    ("Pop", &["Upbeat"]),
    ("Art Pop", &["Dreamy", "Atmospheric"]),
    ("K-Pop", &["Upbeat", "Energetic"]),
    ("J-Pop", &["Upbeat", "Nostalgic"]),
    ("City Pop", &["Nostalgic", "Groovy"]),
    ("Hip-Hop", &["Groovy", "Energetic"]),
    ("Rap", &["Energetic", "Intense"]),
    ("Grime", &["Intense", "Dark"]),
    ("Trap", &["Dark", "Energetic"]),
    ("Trap & Cloud Rap", &["Late Night", "Atmospheric"]),
    ("Drill", &["Dark", "Intense"]),
    ("Phonk", &["Dark", "Energetic"]),
    ("R&B", &["Warm", "Late Night"]),
    ("Soul", &["Warm", "Groovy"]),
    ("Funk", &["Groovy", "Upbeat"]),
    ("Disco", &["Groovy", "Upbeat"]),
    ("Indie", &["Chill", "Nostalgic"]),
    ("Indie Rock", &["Nostalgic", "Energetic"]),
    ("Indie Pop", &["Dreamy", "Upbeat"]),
    ("Indie Folk", &["Warm", "Melancholic"]),
    ("Folk & Acoustic", &["Warm", "Chill"]),
    ("Country", &["Warm", "Nostalgic"]),
    ("Alternative", &["Melancholic", "Energetic"]),
    ("Post-Punk", &["Dark", "Melancholic"]),
    ("Post-Rock", &["Atmospheric", "Melancholic"]),
    ("Rock", &["Energetic"]),
    ("Punk", &["Intense", "Energetic"]),
    ("Emo", &["Melancholic", "Intense"]),
    ("Grunge", &["Dark", "Intense"]),
    ("Metal", &["Intense", "Dark"]),
    ("Metalcore", &["Intense"]),
    ("Jazz", &["Chill", "Warm"]),
    ("Blues", &["Warm", "Melancholic"]),
    ("Bossa Nova", &["Chill", "Warm"]),
    ("Classical", &["Focus", "Atmospheric"]),
    ("Soundtrack", &["Atmospheric", "Focus"]),
    ("Instrumental", &["Focus", "Chill"]),
    ("Reggae", &["Chill", "Warm"]),
    ("Dancehall", &["Groovy", "Upbeat"]),
    ("Latin", &["Upbeat", "Groovy"]),
    ("Afrobeats", &["Groovy", "Upbeat"]),
    ("Dance", &["Energetic", "Upbeat"]),
    ("New Wave", &["Nostalgic", "Upbeat"]),
];

/// Returns default moods associated with a canonical genre.
pub fn genre_default_moods(genre: &str) -> &'static [&'static str] {
    GENRE_MOODS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(genre))
        .map(|(_, moods)| *moods)
        .unwrap_or(&[])
}

/// Approximate `(energy, valence)` coordinates in `0.0..=1.0` for canonical moods,
/// used to gauge sonic/emotional continuity and vibe pivots.
pub fn mood_energy_valence(mood: &str) -> (f32, f32) {
    match mood {
        "Intense" => (0.95, 0.28),
        "Energetic" => (0.86, 0.72),
        "Upbeat" => (0.76, 0.90),
        "Groovy" => (0.70, 0.78),
        "Dark" => (0.60, 0.18),
        "Nostalgic" => (0.45, 0.55),
        "Warm" => (0.42, 0.74),
        "Melancholic" => (0.35, 0.22),
        "Late Night" => (0.38, 0.40),
        "Dreamy" => (0.34, 0.56),
        "Atmospheric" => (0.32, 0.46),
        "Chill" => (0.26, 0.65),
        "Focus" => (0.22, 0.52),
        _ => (0.50, 0.50),
    }
}

/// Pairwise genre-family similarity table `(a, b, similarity)` for closely related
/// genres so recommendations and mixes smoothly bridge adjacent scenes.
const GENRE_NEIGHBORS: &[(&str, &str, f32)] = &[
    // Indie / Dream / Shoegaze / Alt
    ("Dream Pop", "Shoegaze", 0.85),
    ("Dream Pop", "Indie Pop", 0.78),
    ("Dream Pop", "Chillwave", 0.75),
    ("Dream Pop", "Indie", 0.70),
    ("Dream Pop", "Psychedelic", 0.68),
    ("Dream Pop", "Art Pop", 0.65),
    ("Shoegaze", "Post-Punk", 0.65),
    ("Shoegaze", "Post-Rock", 0.68),
    ("Shoegaze", "Alternative", 0.62),
    ("Indie", "Indie Rock", 0.84),
    ("Indie", "Indie Pop", 0.84),
    ("Indie", "Indie Folk", 0.76),
    ("Indie", "Alternative", 0.74),
    ("Indie Rock", "Alternative", 0.80),
    ("Indie Rock", "Rock", 0.72),
    ("Indie Rock", "Post-Punk", 0.70),
    ("Indie Pop", "Electropop", 0.68),
    ("Indie Pop", "Synthpop", 0.66),
    ("Indie Pop", "Pop", 0.65),
    ("Indie Folk", "Folk & Acoustic", 0.88),
    ("Post-Punk", "Darkwave", 0.80),
    ("Post-Punk", "New Wave", 0.76),
    ("Post-Punk", "Alternative", 0.65),
    ("Post-Rock", "Ambient", 0.65),
    ("Psychedelic", "Rock", 0.62),
    ("Psychedelic", "Funk", 0.58),
    // Electronic / Club / Synth
    ("Electronic", "House", 0.78),
    ("Electronic", "Techno", 0.72),
    ("Electronic", "Electropop", 0.75),
    ("Electronic", "Downtempo", 0.72),
    ("Electronic", "IDM", 0.76),
    ("Electronic", "Drum & Bass", 0.68),
    ("Electronic", "UK Garage", 0.72),
    ("Electronic", "Dance", 0.78),
    ("Electronic", "Synthpop", 0.70),
    ("Electronic", "Synthwave", 0.70),
    ("Electronic", "Trance", 0.70),
    ("House", "Techno", 0.74),
    ("House", "Disco", 0.76),
    ("House", "UK Garage", 0.76),
    ("House", "Dance", 0.82),
    ("Techno", "Trance", 0.74),
    ("Techno", "Hardstyle", 0.72),
    ("Techno", "Darkwave", 0.60),
    ("Drum & Bass", "UK Garage", 0.72),
    ("Drum & Bass", "Dubstep", 0.74),
    ("Drum & Bass", "Breakcore", 0.76),
    ("Synthpop", "Synthwave", 0.82),
    ("Synthpop", "New Wave", 0.82),
    ("Synthpop", "Electropop", 0.80),
    ("Synthpop", "City Pop", 0.68),
    ("Synthwave", "Darkwave", 0.76),
    ("Synthwave", "Chillwave", 0.72),
    ("Chillwave", "Vaporwave", 0.74),
    ("Chillwave", "Lo-Fi", 0.70),
    ("Downtempo", "Trip-Hop", 0.82),
    ("Downtempo", "Ambient", 0.74),
    ("Downtempo", "Lo-Fi", 0.74),
    ("Ambient", "Instrumental", 0.72),
    ("Ambient", "Classical", 0.62),
    ("Ambient", "Soundtrack", 0.68),
    ("Hyperpop", "Electropop", 0.70),
    // Hip-Hop / Rap / R&B / Soul
    ("Hip-Hop", "Rap", 0.88),
    ("Hip-Hop", "Trap", 0.78),
    ("Hip-Hop", "Trap & Cloud Rap", 0.76),
    ("Hip-Hop", "Lo-Fi", 0.62),
    ("Hip-Hop", "R&B", 0.66),
    ("Hip-Hop", "Soul", 0.62),
    ("Rap", "Trap", 0.82),
    ("Rap", "Drill", 0.80),
    ("Rap", "Grime", 0.78),
    ("Rap", "Trap & Cloud Rap", 0.80),
    ("Trap", "Drill", 0.78),
    ("Trap", "Trap & Cloud Rap", 0.84),
    ("Trap", "Phonk", 0.72),
    ("R&B", "Soul", 0.85),
    ("R&B", "Pop", 0.64),
    ("R&B", "Funk", 0.68),
    ("Soul", "Funk", 0.80),
    ("Soul", "Jazz", 0.64),
    ("Soul", "Blues", 0.68),
    ("Funk", "Disco", 0.82),
    ("Funk", "City Pop", 0.72),
    // Rock / Punk / Metal
    ("Rock", "Alternative", 0.78),
    ("Rock", "Grunge", 0.78),
    ("Rock", "Punk", 0.72),
    ("Rock", "Metal", 0.68),
    ("Rock", "Blues", 0.64),
    ("Alternative", "Grunge", 0.82),
    ("Alternative", "Emo", 0.72),
    ("Punk", "Emo", 0.78),
    ("Punk", "Metalcore", 0.65),
    ("Metal", "Metalcore", 0.84),
    ("Metal", "Grunge", 0.65),
    // Pop / Global
    ("Pop", "Electropop", 0.82),
    ("Pop", "Dance", 0.78),
    ("Pop", "Disco", 0.70),
    ("Pop", "Art Pop", 0.72),
    ("Pop", "K-Pop", 0.72),
    ("Pop", "J-Pop", 0.68),
    ("J-Pop", "City Pop", 0.74),
    ("J-Pop", "K-Pop", 0.66),
    ("Latin", "Dancehall", 0.72),
    ("Latin", "Afrobeats", 0.68),
    ("Afrobeats", "Dancehall", 0.78),
    ("Afrobeats", "Reggae", 0.65),
    ("Reggae", "Dancehall", 0.80),
    // Jazz / Classical / Acoustic
    ("Jazz", "Bossa Nova", 0.80),
    ("Jazz", "Blues", 0.74),
    ("Jazz", "Lo-Fi", 0.68),
    ("Jazz", "Instrumental", 0.66),
    ("Classical", "Soundtrack", 0.82),
    ("Classical", "Instrumental", 0.80),
    ("Soundtrack", "Instrumental", 0.78),
    ("Folk & Acoustic", "Country", 0.72),
];

/// Soft similarity between two canonical genres in `0.0..=1.0`.
pub fn genre_similarity(a: &str, b: &str) -> f32 {
    if a.eq_ignore_ascii_case(b) {
        return 1.0;
    }
    for &(x, y, sim) in GENRE_NEIGHBORS {
        if (x.eq_ignore_ascii_case(a) && y.eq_ignore_ascii_case(b))
            || (x.eq_ignore_ascii_case(b) && y.eq_ignore_ascii_case(a))
        {
            return sim;
        }
    }
    0.0
}

/// Soft similarity between two canonical moods in `0.0..=1.0`, based on
/// explicit mood adjacency and continuous energy/valence proximity.
pub fn mood_similarity(a: &str, b: &str) -> f32 {
    if a.eq_ignore_ascii_case(b) {
        return 1.0;
    }
    let (ea, va) = mood_energy_valence(a);
    let (eb, vb) = mood_energy_valence(b);
    let dist = ((ea - eb).powi(2) + (va - vb).powi(2)).sqrt();
    // Smooth cosine-like decay: distance 0 -> 1.0, distance >= 0.65 -> 0.0.
    (1.0 - dist / 0.65).clamp(0.0, 0.85)
}

/// Built-in genre lookup for ~80 well-known artists across global & European
/// scenes so cold-start, offline, and newly started servers immediately understand
/// artist genres & moods without waiting on upstream APIs.
pub fn builtin_artist_genres(norm_artist: &str) -> Option<&'static [&'static str]> {
    Some(match norm_artist {
        "tame impala" => &["Psychedelic", "Indie", "Dream Pop"],
        "arctic monkeys" => &["Indie Rock", "Alternative", "Rock"],
        "men i trust" => &["Dream Pop", "Indie Pop", "Chillwave"],
        "daft punk" => &["Electronic", "House", "Disco"],
        "fred again" => &["Electronic", "House", "UK Garage"],
        "kaytranada" => &["House", "Electronic", "R&B"],
        "kendrick lamar" => &["Hip-Hop", "Rap"],
        "tyler" | "tyler the creator" => &["Hip-Hop", "Alternative", "Soul"],
        "travis scott" => &["Trap", "Hip-Hop"],
        "frank ocean" => &["R&B", "Soul", "Alternative"],
        "sza" => &["R&B", "Soul", "Pop"],
        "weeknd" | "the weeknd" => &["Pop", "R&B", "Synthpop"],
        "billie eilish" => &["Pop", "Alternative", "Electropop"],
        "dua lipa" => &["Pop", "Disco", "Dance"],
        "radiohead" => &["Alternative", "Indie Rock", "Art Pop"],
        "linkin park" => &["Rock", "Alternative", "Metal"],
        "metallica" => &["Metal", "Rock"],
        "deftones" => &["Metal", "Shoegaze", "Alternative"],
        "miles davis" => &["Jazz"],
        "nujabes" => &["Lo-Fi", "Hip-Hop", "Jazz"],
        "lana del rey" => &["Dream Pop", "Indie Pop", "Pop"],
        "bon iver" => &["Indie Folk", "Folk & Acoustic", "Indie"],
        "bad bunny" => &["Latin", "Trap"],
        "burna boy" => &["Afrobeats", "Pop"],
        "hans zimmer" => &["Soundtrack", "Classical"],
        "ludovico einaudi" => &["Classical", "Instrumental"],
        "gorillaz" => &["Alternative", "Electronic", "Hip-Hop"],
        "flume" => &["Electronic", "Downtempo"],
        "khruangbin" => &["Psychedelic", "Funk", "Soul"],
        "crumb" => &["Psychedelic", "Indie", "Jazz"],
        "m83" => &["Synthpop", "Dream Pop", "Electronic"],
        "beach house" => &["Dream Pop", "Shoegaze", "Indie"],
        "mac demarco" => &["Indie", "Indie Rock", "Dream Pop"],
        "clairo" => &["Indie Pop", "Dream Pop"],
        "kanye west" | "ye" => &["Hip-Hop", "Rap"],
        "drake" => &["Hip-Hop", "R&B", "Pop"],
        "pink floyd" => &["Rock", "Psychedelic"],
        "nirvana" => &["Grunge", "Rock", "Alternative"],
        "taylor swift" => &["Pop", "Indie Folk"],
        "justice" => &["Electronic", "House", "Disco"],
        "disclosure" => &["House", "UK Garage", "Electronic"],
        "bicep" => &["Electronic", "House", "Techno"],
        "four tet" => &["Electronic", "IDM", "House"],
        "aphex twin" => &["IDM", "Electronic", "Ambient"],
        "boards of canada" => &["IDM", "Ambient", "Downtempo"],
        "bonobo" => &["Downtempo", "Electronic", "Trip-Hop"],
        "massive attack" => &["Trip-Hop", "Downtempo", "Electronic"],
        "portishead" => &["Trip-Hop", "Downtempo", "Alternative"],
        "burial" => &["UK Garage", "Dubstep", "Ambient"],
        "charlotte de witte" => &["Techno", "Electronic"],
        "amelie lens" => &["Techno", "Electronic"],
        "peggy gou" => &["House", "Techno", "Electronic"],
        "paul kalkbrenner" => &["Techno", "Electronic"],
        "charli xcx" => &["Hyperpop", "Electropop", "Pop"],
        "lorde" => &["Electropop", "Indie Pop", "Art Pop"],
        "fka twigs" => &["Art Pop", "R&B", "Electronic"],
        "bjork" => &["Art Pop", "Electronic", "Alternative"],
        "slowdive" => &["Shoegaze", "Dream Pop"],
        "my bloody valentine" => &["Shoegaze", "Alternative"],
        "cigarettes after sex" => &["Dream Pop", "Ambient", "Indie"],
        "the smiths" | "smiths" => &["Indie Rock", "Post-Punk", "New Wave"],
        "the cure" | "cure" => &["Post-Punk", "New Wave", "Darkwave"],
        "joy division" => &["Post-Punk", "Darkwave"],
        "depeche mode" => &["Synthpop", "Darkwave", "New Wave"],
        "the strokes" | "strokes" => &["Indie Rock", "Rock", "Alternative"],
        "tame" | "pond" => &["Psychedelic", "Indie Rock"],
        "mgmt" => &["Psychedelic", "Synthpop", "Indie"],
        "phoenix" => &["Indie Pop", "Synthpop", "Indie Rock"],
        "vampire weekend" => &["Indie Rock", "Indie Pop"],
        "phoebe bridgers" => &["Indie Folk", "Indie Rock"],
        "mitski" => &["Indie Rock", "Art Pop", "Alternative"],
        "j cole" => &["Hip-Hop", "Rap"],
        "playboi carti" => &["Trap", "Hip-Hop"],
        "mf doom" => &["Hip-Hop", "Rap"],
        "outkast" => &["Hip-Hop", "Funk", "Soul"],
        "mac miller" => &["Hip-Hop", "Jazz", "Soul"],
        "anderson paak" => &["R&B", "Funk", "Soul"],
        "daniel caesar" => &["R&B", "Soul"],
        "steve lacy" => &["R&B", "Funk", "Indie"],
        "bring me the horizon" => &["Metalcore", "Rock", "Alternative"],
        "rammstein" => &["Metal", "Rock"],
        "kraftklub" => &["Indie Rock", "Rock"],
        "annenmaykantereit" => &["Indie", "Pop", "Rock"],
        "provinz" => &["Indie Pop", "Indie"],
        "apache 207" => &["Rap", "Pop"],
        "pashanim" => &["Rap", "Hip-Hop"],
        "cro" => &["Rap", "Pop", "Hip-Hop"],
        "peter fox" => &["Hip-Hop", "Dancehall", "Pop"],
        "seeed" => &["Dancehall", "Reggae", "Hip-Hop"],
        "nina chuba" => &["Pop", "Rap"],
        "kiz" | "k i z" => &["Rap", "Hip-Hop"],
        "alligatoah" => &["Rap", "Hip-Hop", "Pop"],
        "casper" => &["Rap", "Indie Rock", "Alternative"],
        "bill evans" => &["Jazz", "Instrumental"],
        "john coltrane" => &["Jazz"],
        "robert glasper" => &["Jazz", "R&B", "Hip-Hop"],
        "badbadnotgood" => &["Jazz", "Hip-Hop", "Soul"],
        "j dilla" => &["Hip-Hop", "Lo-Fi", "Jazz"],
        "tatsuro yamashita" => &["City Pop", "Funk", "Pop"],
        "yoasobi" => &["J-Pop", "Pop"],
        "newjeans" => &["K-Pop", "R&B", "UK Garage"],
        "bts" => &["K-Pop", "Pop"],
        "blackpink" => &["K-Pop", "Pop", "Electronic"],
        "rosalia" => &["Latin", "Pop", "Art Pop"],
        "stromae" => &["Electropop", "Hip-Hop", "Dance"],
        "kavinsky" => &["Synthwave", "Electronic"],
        "perturbator" => &["Synthwave", "Darkwave"],
        "the midnight" | "midnight" => &["Synthwave", "Synthpop"],
        "system of a down" => &["Metal", "Alternative", "Rock"],
        "tool" => &["Metal", "Rock"],
        "foo fighters" => &["Rock", "Alternative"],
        "red hot chili peppers" => &["Rock", "Funk", "Alternative"],
        "queen" => &["Rock", "Pop"],
        "fleetwood mac" => &["Rock", "Pop", "Folk & Acoustic"],
        "david bowie" => &["Rock", "Art Pop", "New Wave"],
        "bob marley" => &["Reggae"],
        _ => return None,
    })
}

fn clean_tag(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .replace(['_', '\t', '\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn match_era(tag: &str) -> Option<&'static str> {
    for &(needle, era) in ERA_RULES {
        if tag == needle {
            return Some(era);
        }
    }
    let padded = format!(" {} ", tag.replace(['-', '/'], " "));
    for &(needle, era) in ERA_RULES {
        if padded.contains(&format!(" {needle} ")) {
            return Some(era);
        }
    }
    None
}

fn is_noise(tag: &str, artist_lower: &str) -> bool {
    if tag.len() < 2 || tag.len() > 40 {
        return true;
    }
    if tag.chars().all(|c| c.is_ascii_digit() || c == 's' || c == '-') {
        return true;
    }
    if !artist_lower.is_empty() {
        let t_bare = tag.strip_prefix("the ").unwrap_or(tag);
        let a_bare = artist_lower.strip_prefix("the ").unwrap_or(artist_lower);
        if tag == artist_lower || (!t_bare.is_empty() && t_bare == a_bare) {
            return true;
        }
    }
    NOISE_TAGS.contains(&tag)
}

fn match_genre(tag: &str) -> Option<&'static str> {
    for &(needle, canonical) in GENRE_RULES {
        if tag == needle {
            return Some(canonical);
        }
    }
    // Word-level containment fallback for compound tags like "french indie pop" or "uk rap".
    let padded = format!(" {tag} ");
    let dehyphen = format!(" {} ", tag.replace(['-', '/'], " "));
    for &(needle, canonical) in GENRE_RULES {
        let token = format!(" {needle} ");
        if padded.contains(&token) || dehyphen.contains(&token) {
            return Some(canonical);
        }
    }
    None
}

fn match_mood(tag: &str) -> Option<&'static str> {
    let padded = format!(" {} ", tag.replace(['-', '/'], " "));
    let words: Vec<&str> = padded.split_whitespace().collect();
    for &(needle, mood) in MOOD_TAG_RULES {
        if tag == needle || padded.contains(&format!(" {needle} ")) {
            return Some(mood);
        }
        if needle.len() >= 5 && words.iter().any(|w| w.starts_with(needle)) {
            return Some(mood);
        }
    }
    None
}

/// Extracts canonical decade/era labels (`"80s"`, `"90s"`, `"2000s"`, ...) from raw tags.
pub fn extract_eras(raw_tags: &[String]) -> Vec<String> {
    let mut eras = Vec::new();
    let mut seen = HashSet::new();
    for raw in raw_tags {
        let tag = clean_tag(raw);
        if let Some(era) = match_era(&tag) {
            if eras.len() < 3 && seen.insert(era) {
                eras.push(era.to_string());
            }
        }
    }
    eras
}

/// Normalizes raw provider tags into `(genres, moods, eras, clean_tags)`.
pub fn classify_tags_full(
    raw_tags: &[String],
    artist_name: &str,
) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let artist_lower = clean_tag(artist_name);
    let mut genres: Vec<String> = Vec::new();
    let mut moods: Vec<String> = Vec::new();
    let mut eras: Vec<String> = Vec::new();
    let mut clean: Vec<String> = Vec::new();
    let mut seen_g: HashSet<String> = HashSet::new();
    let mut seen_m: HashSet<String> = HashSet::new();
    let mut seen_e: HashSet<String> = HashSet::new();
    let mut seen_c: HashSet<String> = HashSet::new();

    for raw in raw_tags {
        let tag = clean_tag(raw);
        if let Some(era) = match_era(&tag) {
            if eras.len() < 3 && seen_e.insert(era.to_string()) {
                eras.push(era.to_string());
            }
        }
        if is_noise(&tag, &artist_lower) {
            continue;
        }
        if let Some(g) = match_genre(&tag) {
            if genres.len() < 6 && seen_g.insert(g.to_string()) {
                genres.push(g.to_string());
            }
        }
        if let Some(m) = match_mood(&tag) {
            if moods.len() < 5 && seen_m.insert(m.to_string()) {
                moods.push(m.to_string());
            }
        }
        if clean.len() < 10 && seen_c.insert(tag.clone()) {
            clean.push(tag);
        }
    }

    // Fill moods from canonical genres if provider returned genres without mood words.
    if moods.len() < 3 {
        for g in &genres {
            for &m in genre_default_moods(g) {
                if moods.len() >= 4 {
                    break;
                }
                if seen_m.insert(m.to_string()) {
                    moods.push(m.to_string());
                }
            }
        }
    }

    (genres, moods, eras, clean)
}

/// Normalizes a list of raw provider tags (e.g. from Last.fm, Deezer, or
/// MusicBrainz) into deduplicated `(genres, moods, clean_tags)`.
pub fn classify_tags(
    raw_tags: &[String],
    artist_name: &str,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let (genres, moods, _, clean) = classify_tags_full(raw_tags, artist_name);
    (genres, moods, clean)
}

/// Extracts musical intent `(genres, moods, eras)` from a free-text search query
/// such as `"chill late night"`, `"90s hip hop"`, `"traurig indie"`, or `"techno workout"`.
pub fn extract_query_intent(query: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let q = clean_tag(query);
    if q.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let mut genres: Vec<String> = Vec::new();
    let mut moods: Vec<String> = Vec::new();
    let mut eras: Vec<String> = Vec::new();
    let mut seen_g: HashSet<&'static str> = HashSet::new();
    let mut seen_m: HashSet<&'static str> = HashSet::new();
    let mut seen_e: HashSet<&'static str> = HashSet::new();

    let padded = format!(" {} ", q.replace(['-', '/', ',', '&'], " "));
    let orig_padded = format!(" {q} ");
    let words: Vec<&str> = padded.split_whitespace().collect();

    for &(needle, canonical) in GENRE_RULES {
        let token = format!(" {needle} ");
        if (orig_padded.contains(&token) || padded.contains(&token)) && seen_g.insert(canonical) {
            genres.push(canonical.to_string());
            if genres.len() >= 4 {
                break;
            }
        }
    }
    for &(needle, mood) in MOOD_TAG_RULES {
        let token = format!(" {needle} ");
        let matched = orig_padded.contains(&token)
            || padded.contains(&token)
            || (needle.len() >= 5 && words.iter().any(|w| w.starts_with(needle)));
        if matched && seen_m.insert(mood) {
            moods.push(mood.to_string());
            if moods.len() >= 4 {
                break;
            }
        }
    }
    for &(needle, era) in ERA_RULES {
        let token = format!(" {needle} ");
        if (orig_padded.contains(&token) || padded.contains(&token)) && seen_e.insert(era) {
            eras.push(era.to_string());
            if eras.len() >= 2 {
                break;
            }
        }
    }

    (genres, moods, eras)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_tags_extracts_genres_moods_eras_and_drops_noise() {
        let raw: Vec<String> = [
            "seen live",
            "Tame Impala",
            "psychedelic rock",
            "dream pop",
            "2010s",
            "australian",
            "chill",
            "indie",
            "favorites",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let (genres, moods, eras, clean) = classify_tags_full(&raw, "Tame Impala");
        assert!(genres.contains(&"Psychedelic".to_string()));
        assert!(genres.contains(&"Dream Pop".to_string()));
        assert!(genres.contains(&"Indie".to_string()));
        assert!(moods.contains(&"Chill".to_string()));
        assert!(moods.contains(&"Dreamy".to_string()));
        assert_eq!(eras, vec!["2010s"]);
        assert!(!clean
            .iter()
            .any(|t| t == "seen live" || t == "favorites" || t == "tame impala" || t == "2010s"));
    }

    #[test]
    fn classify_tags_infers_moods_from_genres() {
        let raw = vec!["synthwave".to_string(), "darkwave".to_string()];
        let (genres, moods, _) = classify_tags(&raw, "Perturbator");
        assert_eq!(genres, vec!["Synthwave", "Darkwave"]);
        assert!(moods.contains(&"Late Night".to_string()));
        assert!(moods.contains(&"Dark".to_string()));
    }

    #[test]
    fn classify_tags_handles_empty_and_compound() {
        let (g, m, c) = classify_tags(&[], "Nobody");
        assert!(g.is_empty() && m.is_empty() && c.is_empty());

        let raw = vec!["french indie pop".to_string(), "drum and bass".to_string()];
        let (g, _, _) = classify_tags(&raw, "Artist");
        assert!(g.contains(&"Indie Pop".to_string()));
        assert!(g.contains(&"Drum & Bass".to_string()));
    }

    #[test]
    fn genre_and_mood_similarity_captures_sonic_proximity() {
        assert_eq!(genre_similarity("Dream Pop", "Dream Pop"), 1.0);
        assert!(genre_similarity("Dream Pop", "Shoegaze") > 0.8);
        assert!(genre_similarity("Shoegaze", "Dream Pop") > 0.8);
        assert_eq!(genre_similarity("Classical", "Metalcore"), 0.0);

        assert_eq!(mood_similarity("Chill", "Chill"), 1.0);
        assert!(mood_similarity("Dreamy", "Atmospheric") > 0.6);
        assert!(mood_similarity("Chill", "Intense") < 0.1);
    }

    #[test]
    fn extract_query_intent_parses_english_and_german_moods_genres_and_eras() {
        let (g, m, e) = extract_query_intent("90er entspannt hip hop");
        assert!(g.contains(&"Hip-Hop".to_string()));
        assert!(m.contains(&"Chill".to_string()));
        assert!(e.contains(&"90s".to_string()));

        let (g2, m2, _) = extract_query_intent("late night techno workout");
        assert!(g2.contains(&"Techno".to_string()));
        assert!(m2.contains(&"Late Night".to_string()));
        assert!(m2.contains(&"Energetic".to_string()));
    }
}
