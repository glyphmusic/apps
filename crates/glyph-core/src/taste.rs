//! Listening taste & user-intent engine, personalized Daily Mixes, visible
//! Taste Profile, recommendation/radio scoring, and search ranking.
//!
//! Architecture (server-first by default, with strict local-only opt-out):
//! - Default (`config.taste.enabled && config.taste.server_collab`): sends a
//!   compact, anonymized taste + session-intent summary (no handles, no raw
//!   history) to `POST /v1/taste/recommend` so the Glyph server handles
//!   multi-channel candidate discovery, hub-demoted collaborative filtering,
//!   intent-aware scoring, MMR diversity re-ranking, Daily Mixes, and Radio.
//! - Opt-out (`config.taste.server_collab = false`): strictly 100 % on-device.
//!   Zero taste requests are sent to the server, server caches are cleared, and
//!   this local engine runs the full dual-horizon intent & MMR pipeline offline
//!   or directly on-device.
//! - Positive signals: likes, follows, playlists, onboarding picks, completions,
//!   replays/rewinds, deliberate clicks, manual queue adds, search picks &
//!   search mood/genre queries, time-of-day & weekday/weekend habits, "more".
//! - Negative signals: context-aware & burst-aware skip timing (differentiates
//!   rapid queue-surfing and library skips from Radio/Discovery rejection),
//!   Bayesian abandonment rate, repeated-skip streaks, explicit "less"/"block"/
//!   "hide", track-level dislikes, unlikes, explicit tag dislikes, and adaptive
//!   version/style avoidance.

use crate::config::TasteConfig;
use crate::dedupe::{normalize_artist, normalize_title, primary_artist, track_key};
use crate::library::Library;
pub use glyph_protocol::taste::{
    DailyMix, InferredIntent, ListeningMode, RecommendedTrack, SessionEventContext,
    SessionEventOutcome, SessionEventSignal, TastePurpose, TasteRecommendRequest,
    TasteRecommendResponse, TasteSeedTrack, TasteSessionContext, TasteTagSignal,
};
use glyph_protocol::taste::{
    builtin_artist_genres, classify_tags_full, extract_query_intent, genre_default_moods,
    genre_similarity, mood_energy_valence, mood_similarity, ArtistTags, CollabArtistRec,
    TasteArtistSignal, TasteCollabResponse,
};
use glyph_protocol::Track;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

const MAX_TRACK_SIGNALS: usize = 600;
const MAX_ARTIST_SIGNALS: usize = 800;
const MAX_TAG_CACHE: usize = 500;
const MAX_DISLIKED_TRACKS: usize = 300;
const MAX_SESSION_EVENTS: usize = 24;
const MAX_SESSION_PAIRS: usize = 40;
const TAG_CACHE_TTL_S: u64 = 14 * 86_400;
const LONG_DECAY_HALF_LIFE_S: f64 = 45.0 * 86_400.0;
const SHORT_DECAY_HALF_LIFE_S: f64 = 7.0 * 86_400.0;
const SESSION_WINDOW_S: u64 = 3 * 3600;

/// Canonical version/style identifiers tracked by the taste engine.
pub const VERSION_STYLES: &[&str] = &[
    "live",
    "acoustic",
    "remix",
    "slowed",
    "sped_up",
    "nightcore",
    "instrumental",
    "karaoke",
    "cover",
    "8d",
    "lofi",
    "demo",
];

/// Version styles that carry a mild negative prior until the listener's own
/// behavior or explicit feedback shows they enjoy them.
const DEFAULT_ODD_VERSIONS: &[&str] = &[
    "karaoke",
    "instrumental",
    "8d",
    "nightcore",
    "sped_up",
    "slowed",
    "cover",
];

/// Current local hour (`0..=23`).
pub fn local_hour() -> u8 {
    use chrono::Timelike;
    chrono::Local::now().hour() as u8
}

/// Whether today is Saturday or Sunday in local time.
pub fn local_is_weekend() -> bool {
    use chrono::Datelike;
    matches!(chrono::Local::now().weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun)
}

/// Current local calendar date key (`YYYY-MM-DD`) for deterministic Daily Mixes.
pub fn local_date_key() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Four coarse local time-of-day periods for listening habits & Time-of-Day mixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeBucket {
    Morning,
    Afternoon,
    Evening,
    Night,
}

impl TimeBucket {
    pub fn from_hour(hour: u8) -> Self {
        match hour % 24 {
            5..=10 => Self::Morning,
            11..=16 => Self::Afternoon,
            17..=21 => Self::Evening,
            _ => Self::Night,
        }
    }

    pub fn now() -> Self {
        Self::from_hour(local_hour())
    }

    pub fn index(self) -> usize {
        match self {
            Self::Morning => 0,
            Self::Afternoon => 1,
            Self::Evening => 2,
            Self::Night => 3,
        }
    }

    pub fn from_index(i: usize) -> Self {
        match i {
            0 => Self::Morning,
            1 => Self::Afternoon,
            2 => Self::Evening,
            _ => Self::Night,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Morning => "morning",
            Self::Afternoon => "afternoon",
            Self::Evening => "evening",
            Self::Night => "night",
        }
    }

    /// Mood affinities naturally aligned with each time of day.
    pub fn aligned_moods(self) -> &'static [&'static str] {
        match self {
            Self::Morning => &["Warm", "Upbeat", "Chill", "Focus"],
            Self::Afternoon => &["Energetic", "Upbeat", "Groovy", "Focus"],
            Self::Evening => &["Chill", "Warm", "Nostalgic", "Dreamy"],
            Self::Night => &["Late Night", "Atmospheric", "Dreamy", "Dark", "Chill"],
        }
    }
}

/// Explicit high-intent user actions beyond raw playback end/skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentAction {
    /// User clicked this specific track deliberately (search/artist/album/home).
    DeliberatePick,
    /// User rewound or repeated this track to hear it again.
    Replayed,
    /// User explicitly queued this track ("Play next" / "Add to queue").
    Queued,
    /// User curated this track into one of their playlists.
    PlaylistAdd,
    /// User liked this track.
    Liked,
    /// User removed a like from this track (gentle taste cooling).
    Unliked,
}

/// One persisted event in the rolling current-session ring buffer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default)]
    pub outcome: SessionEventOutcome,
    #[serde(default)]
    pub context: SessionEventContext,
    #[serde(default)]
    pub timestamp_s: u64,
}

/// Per-track implicit playback & intent signals (completions, skip timing,
/// replays, deliberate picks, queue/playlist adds, streaks).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackSignal {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub plays: u32,
    #[serde(default)]
    pub completions: u32,
    /// Explicit replays / rewinds / repeat-one loops.
    #[serde(default)]
    pub replays: u32,
    /// Deliberate user clicks (search, artist page, album, home).
    #[serde(default)]
    pub deliberate_plays: u32,
    /// Manual "Add to queue" / "Play next" actions.
    #[serde(default)]
    pub queue_adds: u32,
    /// Manual playlist additions.
    #[serde(default)]
    pub playlist_adds: u32,
    /// Skipped before 5 s or < 5 % of track length.
    #[serde(default)]
    pub instant_skips: u32,
    /// Skipped between 5 s and 20 s (or 5–20 %).
    #[serde(default)]
    pub early_skips: u32,
    /// Abandoned between 20 s and 40 % of track length.
    #[serde(default)]
    pub mid_abandons: u32,
    /// Consecutive manual skips without a completion or like.
    #[serde(default)]
    pub consecutive_skips: u16,
    /// Exponentially smoothed completion ratio (`0.0..=1.0`).
    #[serde(default)]
    pub avg_ratio: f32,
    #[serde(default)]
    pub last_event_s: u64,
    /// Timestamp of last positive play/completion (for recency fatigue).
    #[serde(default)]
    pub last_played_s: u64,
}

impl TrackSignal {
    pub fn weighted_skips(&self) -> f32 {
        self.instant_skips as f32 * 1.35
            + self.early_skips as f32 * 0.85
            + self.mid_abandons as f32 * 0.35
    }

    pub fn total_skips(&self) -> u32 {
        self.instant_skips + self.early_skips + self.mid_abandons
    }
}

/// Per-artist implicit playback signals, 7-day recent momentum, and time profile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistSignal {
    #[serde(default)]
    pub display: String,
    #[serde(default)]
    pub plays: u32,
    #[serde(default)]
    pub completions: u32,
    #[serde(default)]
    pub replays: u32,
    #[serde(default)]
    pub deliberate_plays: u32,
    #[serde(default)]
    pub instant_skips: u32,
    #[serde(default)]
    pub early_skips: u32,
    #[serde(default)]
    pub mid_abandons: u32,
    #[serde(default)]
    pub consecutive_skips: u16,
    /// Weighted listens per `[Morning, Afternoon, Evening, Night]`.
    #[serde(default)]
    pub time_buckets: [f32; 4],
    /// Short-term (~7 day) exponentially decayed momentum score.
    #[serde(default)]
    pub recent_score: f32,
    #[serde(default)]
    pub recent_updated_s: u64,
    #[serde(default)]
    pub weekend_plays: f32,
    #[serde(default)]
    pub weekday_plays: f32,
    #[serde(default)]
    pub last_event_s: u64,
}

impl ArtistSignal {
    pub fn weighted_skips(&self) -> f32 {
        self.instant_skips as f32 * 1.30
            + self.early_skips as f32 * 0.80
            + self.mid_abandons as f32 * 0.30
    }

    pub fn total_skips(&self) -> u32 {
        self.instant_skips + self.early_skips + self.mid_abandons
    }

    /// Bayesian-smoothed abandonment rate in `0.0..=1.0`.
    pub fn abandon_rate(&self) -> f32 {
        let sk = self.weighted_skips();
        let pos = self.plays as f32 + self.completions as f32 * 0.55 + self.replays as f32 * 1.1;
        if sk <= 0.0 {
            return 0.0;
        }
        (sk / (pos + sk + 2.5)).clamp(0.0, 1.0)
    }

    /// Decays `recent_score` to `now_s` (7-day half-life) and adds `delta`.
    fn bump_recent(&mut self, delta: f32, now_s: u64) {
        let prev = self.decayed_recent(now_s);
        self.recent_score = (prev + delta).clamp(-6.0, 30.0);
        self.recent_updated_s = now_s;
    }

    pub fn decayed_recent(&self, now_s: u64) -> f32 {
        if self.recent_updated_s == 0 || self.recent_score == 0.0 {
            return 0.0;
        }
        let age = now_s.saturating_sub(self.recent_updated_s) as f64;
        self.recent_score * ((-age / SHORT_DECAY_HALF_LIFE_S).exp2() as f32)
    }
}

/// Per-version/style implicit behavioral counts (`live`, `remix`, `sped_up`, ...).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VersionSignal {
    #[serde(default)]
    pub plays: u32,
    #[serde(default)]
    pub completions: u32,
    #[serde(default)]
    pub skips: u32,
    #[serde(default)]
    pub last_event_s: u64,
}

/// Locally cached genres, moods, eras, tags and similar artists for one normalized artist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CachedArtistTags {
    #[serde(default)]
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
    #[serde(default)]
    pub fetched_at_s: u64,
}

/// Cached server collaborative recommendations (cleared when `server_collab` is turned off).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CollabCache {
    #[serde(default)]
    pub recommendations: Vec<CollabArtistRec>,
    #[serde(default)]
    pub contributors: u32,
    #[serde(default)]
    pub model_ready: bool,
    #[serde(default)]
    pub fetched_at_s: u64,
}

/// High-frequency playback taste signals stored in `LibraryActivity`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteActivity {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub track_signals: BTreeMap<String, TrackSignal>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artist_signals: BTreeMap<String, ArtistSignal>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub version_signals: BTreeMap<String, VersionSignal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_events: Vec<SessionEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_pairs: Vec<(String, String, u64)>,
}

impl TasteActivity {
    pub fn is_empty(&self) -> bool {
        self.track_signals.is_empty()
            && self.artist_signals.is_empty()
            && self.version_signals.is_empty()
            && self.session_events.is_empty()
            && self.session_pairs.is_empty()
    }
}

/// Persistent local taste store embedded in [`Library`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TasteData {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub track_signals: BTreeMap<String, TrackSignal>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artist_signals: BTreeMap<String, ArtistSignal>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub version_signals: BTreeMap<String, VersionSignal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub onboarding_artists: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub onboarding_tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub onboarding_genres: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub onboarding_done: bool,
    /// Tracks explicitly marked "Not interested / Less like this" (keyed by `song_key`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub disliked_tracks: BTreeMap<String, Track>,
    /// Explicit genre/mood feedback (`+1` like, `-1` dislike), keyed by lowercased tag.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tag_feedback: BTreeMap<String, i8>,
    /// Explicit version/style feedback (`+1` prefer, `-1` avoid), keyed by canonical style.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub version_feedback: BTreeMap<String, i8>,
    /// Locally cached genre/mood/era/similar metadata per normalized artist.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artist_tags: BTreeMap<String, CachedArtistTags>,
    /// Last server collaborative filtering response (cleared when opted out to local-only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collab_cache: Option<CollabCache>,
    /// Rolling ring buffer of recent session events (newest first, max 24).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_events: Vec<SessionEvent>,
    /// Sequential artist transition pairs completed within the same session `(a, b, ts)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_pairs: Vec<(String, String, u64)>,
    /// Most recent server-inferred listening intention (cleared on local-only opt-out).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_server_intent: Option<InferredIntent>,
}

impl TasteData {
    pub fn is_empty(&self) -> bool {
        self.track_signals.is_empty()
            && self.artist_signals.is_empty()
            && self.version_signals.is_empty()
            && self.onboarding_artists.is_empty()
            && self.onboarding_tracks.is_empty()
            && self.onboarding_genres.is_empty()
            && !self.onboarding_done
            && self.disliked_tracks.is_empty()
            && self.tag_feedback.is_empty()
            && self.version_feedback.is_empty()
            && self.artist_tags.is_empty()
            && self.collab_cache.is_none()
            && self.session_events.is_empty()
            && self.session_pairs.is_empty()
    }

    pub fn activity_snapshot(&self) -> TasteActivity {
        TasteActivity {
            track_signals: self.track_signals.clone(),
            artist_signals: self.artist_signals.clone(),
            version_signals: self.version_signals.clone(),
            session_events: self.session_events.clone(),
            session_pairs: self.session_pairs.clone(),
        }
    }

    pub fn apply_activity(&mut self, act: TasteActivity) {
        if !act.track_signals.is_empty() {
            self.track_signals = act.track_signals;
        }
        if !act.artist_signals.is_empty() {
            self.artist_signals = act.artist_signals;
        }
        if !act.version_signals.is_empty() {
            self.version_signals = act.version_signals;
        }
        if !act.session_events.is_empty() {
            self.session_events = act.session_events;
        }
        if !act.session_pairs.is_empty() {
            self.session_pairs = act.session_pairs;
        }
    }

    /// Backwards-compatible playback outcome recorder (defaults to `Unknown` context).
    pub fn record_playback_outcome(
        &mut self,
        track: &Track,
        position_ms: u64,
        duration_ms: Option<u64>,
        manual_skip: bool,
        completed: bool,
        hour: u8,
        now_s: u64,
    ) {
        self.record_playback_outcome_ctx(
            track,
            position_ms,
            duration_ms,
            manual_skip,
            completed,
            SessionEventContext::Unknown,
            hour,
            false,
            now_s,
        );
    }

    /// Context-aware & burst-aware playback outcome recorder:
    /// - Distinguishes rapid queue-surfing (`>= 2` instant skips within 12 s)
    ///   from genuine artist rejection.
    /// - Calibrates skip penalties by listening context (`Radio` rejection is
    ///   stronger than skipping a track in your own `Library`).
    /// - Captures sequential same-session artist transition pairs for session CF.
    #[allow(clippy::too_many_arguments)]
    pub fn record_playback_outcome_ctx(
        &mut self,
        track: &Track,
        position_ms: u64,
        duration_ms: Option<u64>,
        manual_skip: bool,
        completed: bool,
        context: SessionEventContext,
        hour: u8,
        is_weekend: bool,
        now_s: u64,
    ) {
        let dur = duration_ms.or(track.duration_ms).filter(|d| *d >= 5_000);
        let ratio: f32 = if completed {
            1.0
        } else if let Some(d) = dur {
            (position_ms as f32 / d as f32).clamp(0.0, 1.0)
        } else {
            (position_ms as f32 / 180_000.0).clamp(0.0, 1.0)
        };

        let is_complete = completed || ratio >= 0.80;
        let is_instant_skip = manual_skip && !is_complete && (position_ms < 5_000 || ratio < 0.05);
        let is_early_skip =
            manual_skip && !is_complete && !is_instant_skip && (position_ms < 20_000 || ratio < 0.20);
        let is_mid_abandon =
            manual_skip && !is_complete && !is_instant_skip && !is_early_skip && ratio < 0.40;
        let is_play = is_complete
            || (!is_instant_skip
                && !is_early_skip
                && !is_mid_abandon
                && (position_ms >= 20_000 || ratio >= 0.40));

        if !is_play && !is_instant_skip && !is_early_skip && !is_mid_abandon {
            return;
        }

        // Detect rapid queue-surfing (browsing through tracks < 4 s each within 12 s).
        // Don't poison long-term artist/track hatred while the user is merely skipping
        // forward in a playlist/album to reach a specific song.
        let rapid_burst = is_instant_skip
            && position_ms < 4_000
            && self
                .session_events
                .iter()
                .take(2)
                .filter(|e| {
                    matches!(e.outcome, SessionEventOutcome::SkippedEarly)
                        && now_s.saturating_sub(e.timestamp_s) <= 12
                })
                .count()
                >= 2;

        // Context-aware skip handling:
        // - Library skip ("not in the mood right now"): only record a mild mid-abandon
        //   on the artist instead of full instant hate.
        // - Rapid queue-surfing burst: record session event for VibeReset, but skip
        //   long-term permanent artist/track hate accumulation.
        let is_library_skip = context == SessionEventContext::Library && !is_play;
        let bucket = TimeBucket::from_hour(hour).index();
        let sk = song_key(track);

        let ts = self.track_signals.entry(sk).or_insert_with(|| TrackSignal {
            title: track.title.clone(),
            artist: primary_artist(&track.artist),
            ..Default::default()
        });
        if ts.title.is_empty() {
            ts.title = track.title.clone();
        }
        if ts.artist.is_empty() {
            ts.artist = primary_artist(&track.artist);
        }
        ts.avg_ratio = if ts.last_event_s == 0 {
            ratio
        } else {
            ts.avg_ratio * 0.65 + ratio * 0.35
        };
        ts.last_event_s = now_s;

        if is_complete {
            ts.plays = ts.plays.saturating_add(1);
            ts.completions = ts.completions.saturating_add(1);
            ts.consecutive_skips = 0;
            ts.last_played_s = now_s;
        } else if is_play {
            ts.plays = ts.plays.saturating_add(1);
            ts.consecutive_skips = 0;
            ts.last_played_s = now_s;
        } else if !rapid_burst {
            if is_instant_skip && !is_library_skip {
                ts.instant_skips = ts.instant_skips.saturating_add(1);
                ts.consecutive_skips = ts.consecutive_skips.saturating_add(1);
            } else if is_early_skip || (is_instant_skip && is_library_skip) {
                ts.early_skips = ts.early_skips.saturating_add(1);
                ts.consecutive_skips = ts.consecutive_skips.saturating_add(1);
            } else if is_mid_abandon {
                ts.mid_abandons = ts.mid_abandons.saturating_add(1);
                ts.consecutive_skips = ts.consecutive_skips.saturating_add(1);
            }
        }

        // Update per-artist signals for every credited artist.
        let primary_disp = primary_artist(&track.artist);
        let primary_norm = normalize_artist(&primary_disp);
        for (norm, display) in credited_named(&track.artist).into_iter().take(3) {
            let entry = self.artist_signals.entry(norm).or_insert_with(|| ArtistSignal {
                display: display.clone(),
                ..Default::default()
            });
            if entry.display.is_empty() {
                entry.display = display;
            }
            entry.last_event_s = now_s;
            if is_complete {
                entry.plays = entry.plays.saturating_add(1);
                entry.completions = entry.completions.saturating_add(1);
                entry.consecutive_skips = 0;
                entry.time_buckets[bucket] += 1.0;
                if is_weekend { entry.weekend_plays += 1.0; } else { entry.weekday_plays += 1.0; }
                entry.bump_recent(1.5, now_s);
            } else if is_play {
                entry.plays = entry.plays.saturating_add(1);
                entry.consecutive_skips = 0;
                entry.time_buckets[bucket] += 0.65;
                if is_weekend { entry.weekend_plays += 0.65; } else { entry.weekday_plays += 0.65; }
                entry.bump_recent(0.85, now_s);
            } else if !rapid_burst {
                if is_library_skip {
                    // Skipping a song from your own library is "not right now", not artist hatred.
                    entry.mid_abandons = entry.mid_abandons.saturating_add(1);
                    entry.bump_recent(-0.35, now_s);
                } else {
                    let ctx_mult = if context == SessionEventContext::Radio { 1.25 } else { 1.0 };
                    if is_instant_skip {
                        entry.instant_skips = entry.instant_skips.saturating_add(1);
                        entry.consecutive_skips = entry.consecutive_skips.saturating_add(1);
                        entry.bump_recent(-1.4 * ctx_mult, now_s);
                    } else if is_early_skip {
                        entry.early_skips = entry.early_skips.saturating_add(1);
                        entry.consecutive_skips = entry.consecutive_skips.saturating_add(1);
                        entry.bump_recent(-0.85 * ctx_mult, now_s);
                    } else if is_mid_abandon {
                        entry.mid_abandons = entry.mid_abandons.saturating_add(1);
                        entry.consecutive_skips = entry.consecutive_skips.saturating_add(1);
                        entry.bump_recent(-0.35 * ctx_mult, now_s);
                    }
                }
            }
        }

        // Update version/style signals if the track carries version cues.
        for style in detect_versions(&track.title, track.album.as_deref()) {
            let v = self.version_signals.entry(style.to_string()).or_default();
            v.last_event_s = now_s;
            if is_complete {
                v.plays = v.plays.saturating_add(1);
                v.completions = v.completions.saturating_add(1);
            } else if is_play {
                v.plays = v.plays.saturating_add(1);
            } else if !rapid_burst {
                v.skips = v.skips.saturating_add(1);
            }
        }

        // Capture sequential same-session artist transition pair on positive plays!
        if (is_complete || is_play) && !primary_norm.is_empty() {
            if let Some(prev) = self.session_events.iter().find(|e| {
                matches!(
                    e.outcome,
                    SessionEventOutcome::Completed
                        | SessionEventOutcome::Played
                        | SessionEventOutcome::Replayed
                        | SessionEventOutcome::Liked
                ) && now_s.saturating_sub(e.timestamp_s) <= 1800
            }) {
                let prev_disp = primary_artist(&prev.artist);
                let prev_norm = normalize_artist(&prev_disp);
                if !prev_norm.is_empty() && prev_norm != primary_norm {
                    self.push_session_pair(prev_disp, primary_disp.clone(), now_s);
                }
            }
        }

        // Append to rolling session event ring buffer (newest first).
        let outcome = if is_complete {
            SessionEventOutcome::Completed
        } else if is_play {
            SessionEventOutcome::Played
        } else if is_instant_skip || is_early_skip {
            SessionEventOutcome::SkippedEarly
        } else {
            SessionEventOutcome::SkippedMid
        };
        self.push_session_event(SessionEvent {
            title: track.title.clone(),
            artist: primary_disp,
            album: track.album.clone(),
            outcome,
            context,
            timestamp_s: now_s,
        });

        self.trim();
    }

    /// Records explicit high-intent actions (replays, deliberate clicks, manual
    /// queueing, playlist curation, likes, unlikes) to deeply understand intent.
    pub fn record_intent_action(
        &mut self,
        track: &Track,
        action: IntentAction,
        context: SessionEventContext,
        now_s: u64,
    ) {
        let sk = song_key(track);
        let primary_disp = primary_artist(&track.artist);
        let ts = self.track_signals.entry(sk).or_insert_with(|| TrackSignal {
            title: track.title.clone(),
            artist: primary_disp.clone(),
            ..Default::default()
        });
        if ts.title.is_empty() {
            ts.title = track.title.clone();
        }
        if ts.artist.is_empty() {
            ts.artist = primary_disp.clone();
        }
        ts.last_event_s = now_s;

        let (recent_delta, outcome_opt) = match action {
            IntentAction::Replayed => {
                ts.replays = ts.replays.saturating_add(1);
                ts.consecutive_skips = 0;
                ts.last_played_s = now_s;
                (2.2, Some(SessionEventOutcome::Replayed))
            }
            IntentAction::DeliberatePick => {
                ts.deliberate_plays = ts.deliberate_plays.saturating_add(1);
                (0.75, None)
            }
            IntentAction::Queued => {
                ts.queue_adds = ts.queue_adds.saturating_add(1);
                ts.consecutive_skips = 0;
                (1.4, Some(SessionEventOutcome::Queued))
            }
            IntentAction::PlaylistAdd => {
                ts.playlist_adds = ts.playlist_adds.saturating_add(1);
                ts.consecutive_skips = 0;
                (1.5, None)
            }
            IntentAction::Liked => {
                ts.consecutive_skips = 0;
                (2.0, Some(SessionEventOutcome::Liked))
            }
            IntentAction::Unliked => (-1.1, Some(SessionEventOutcome::Unliked)),
        };

        for (norm, display) in credited_named(&track.artist).into_iter().take(3) {
            let entry = self.artist_signals.entry(norm).or_insert_with(|| ArtistSignal {
                display: display.clone(),
                ..Default::default()
            });
            if entry.display.is_empty() {
                entry.display = display;
            }
            entry.last_event_s = now_s;
            match action {
                IntentAction::Replayed => {
                    entry.replays = entry.replays.saturating_add(1);
                    entry.consecutive_skips = 0;
                }
                IntentAction::DeliberatePick => {
                    entry.deliberate_plays = entry.deliberate_plays.saturating_add(1);
                }
                IntentAction::Queued | IntentAction::PlaylistAdd | IntentAction::Liked => {
                    entry.consecutive_skips = 0;
                }
                IntentAction::Unliked => {}
            }
            entry.bump_recent(recent_delta, now_s);
        }

        if let Some(outcome) = outcome_opt {
            self.push_session_event(SessionEvent {
                title: track.title.clone(),
                artist: primary_disp,
                album: track.album.clone(),
                outcome,
                context,
                timestamp_s: now_s,
            });
        }
        self.trim();
    }

    fn push_session_event(&mut self, ev: SessionEvent) {
        self.session_events.insert(0, ev);
        self.session_events.truncate(MAX_SESSION_EVENTS);
    }

    fn push_session_pair(&mut self, a: String, b: String, now_s: u64) {
        let ka = normalize_artist(&a);
        let kb = normalize_artist(&b);
        self.session_pairs.retain(|(x, y, _)| {
            let kx = normalize_artist(x);
            let ky = normalize_artist(y);
            !((kx == ka && ky == kb) || (kx == kb && ky == ka))
        });
        self.session_pairs.insert(0, (a, b, now_s));
        self.session_pairs.truncate(MAX_SESSION_PAIRS);
    }

    /// Saves the user's onboarding favorites (artists, songs, genres).
    pub fn set_onboarding(&mut self, artists: Vec<String>, tracks: Vec<Track>, genres: Vec<String>) {
        let mut seen_a = HashSet::new();
        self.onboarding_artists = artists
            .into_iter()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty() && seen_a.insert(normalize_artist(a)))
            .take(30)
            .collect();
        let mut seen_t = HashSet::new();
        self.onboarding_tracks = tracks
            .into_iter()
            .filter(|t| !t.title.trim().is_empty() && seen_t.insert(song_key(t)))
            .take(30)
            .collect();
        let mut seen_g = HashSet::new();
        self.onboarding_genres = genres
            .into_iter()
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty() && seen_g.insert(g.to_lowercase()))
            .take(15)
            .collect();
        self.onboarding_done = true;
    }

    /// Marks or unmarks a specific song as "Not interested / Disliked".
    pub fn set_disliked_track(&mut self, track: &Track, disliked: bool) {
        let key = song_key(track);
        if disliked {
            self.disliked_tracks.insert(key, track.clone());
            if self.disliked_tracks.len() > MAX_DISLIKED_TRACKS {
                if let Some(first) = self.disliked_tracks.keys().next().cloned() {
                    self.disliked_tracks.remove(&first);
                }
            }
        } else {
            self.disliked_tracks.remove(&key);
        }
    }

    pub fn is_disliked_track(&self, track: &Track) -> bool {
        self.disliked_tracks.contains_key(&song_key(track))
    }

    /// Sets explicit genre/mood feedback (`Some(1)` like, `Some(-1)` dislike, `None` neutral).
    pub fn set_tag_feedback(&mut self, tag: &str, pref: Option<i8>) {
        let key = tag.trim().to_lowercase();
        if key.is_empty() {
            return;
        }
        match pref.filter(|p| *p != 0) {
            Some(p) => {
                self.tag_feedback.insert(key, p.clamp(-1, 1));
            }
            None => {
                self.tag_feedback.remove(&key);
            }
        }
    }

    /// Sets explicit style/version feedback (`Some(1)` prefer, `Some(-1)` avoid, `None` neutral).
    pub fn set_version_feedback(&mut self, style: &str, pref: Option<i8>) {
        let key = style.trim().to_lowercase();
        if !VERSION_STYLES.contains(&key.as_str()) {
            return;
        }
        match pref.filter(|p| *p != 0) {
            Some(p) => {
                self.version_feedback.insert(key, p.clamp(-1, 1));
            }
            None => {
                self.version_feedback.remove(&key);
            }
        }
    }

    /// Merges server/provider artist tag metadata into the local cache.
    pub fn merge_artist_tags(&mut self, items: &[ArtistTags], now_s: u64) {
        for item in items {
            let key = normalize_artist(&item.artist);
            if key.is_empty() {
                continue;
            }
            let item_empty = item.genres.is_empty()
                && item.moods.is_empty()
                && item.eras.is_empty()
                && item.tags.is_empty()
                && item.similar.is_empty();
            if item_empty {
                if let Some(prev) = self.artist_tags.get_mut(&key) {
                    if !prev.genres.is_empty() || !prev.similar.is_empty() {
                        prev.fetched_at_s = now_s;
                        continue;
                    }
                }
            }
            self.artist_tags.insert(
                key,
                CachedArtistTags {
                    artist: item.artist.clone(),
                    genres: item.genres.clone(),
                    moods: item.moods.clone(),
                    eras: item.eras.clone(),
                    tags: item.tags.clone(),
                    similar: item.similar.clone(),
                    fetched_at_s: now_s,
                },
            );
        }
        if self.artist_tags.len() > MAX_TAG_CACHE {
            let mut entries: Vec<(String, CachedArtistTags)> =
                std::mem::take(&mut self.artist_tags).into_iter().collect();
            entries.sort_by(|a, b| b.1.fetched_at_s.cmp(&a.1.fetched_at_s));
            entries.truncate(MAX_TAG_CACHE);
            self.artist_tags = entries.into_iter().collect();
        }
    }

    pub fn set_collab_cache(&mut self, resp: TasteCollabResponse, now_s: u64) {
        self.collab_cache = Some(CollabCache {
            recommendations: resp.recommendations,
            contributors: resp.contributors,
            model_ready: resp.model_ready,
            fetched_at_s: now_s,
        });
    }

    pub fn clear_collab_cache(&mut self) {
        self.collab_cache = None;
        self.last_server_intent = None;
    }

    /// Clears learned taste signals and preferences while preserving the rest of the library.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn trim(&mut self) {
        if self.track_signals.len() > MAX_TRACK_SIGNALS {
            let mut entries: Vec<(String, TrackSignal)> =
                std::mem::take(&mut self.track_signals).into_iter().collect();
            entries.sort_by(|a, b| {
                b.1.last_event_s
                    .cmp(&a.1.last_event_s)
                    .then_with(|| (b.1.plays + b.1.total_skips()).cmp(&(a.1.plays + a.1.total_skips())))
            });
            entries.truncate(MAX_TRACK_SIGNALS);
            self.track_signals = entries.into_iter().collect();
        }
        if self.artist_signals.len() > MAX_ARTIST_SIGNALS {
            let mut entries: Vec<(String, ArtistSignal)> =
                std::mem::take(&mut self.artist_signals).into_iter().collect();
            entries.sort_by(|a, b| {
                b.1.last_event_s
                    .cmp(&a.1.last_event_s)
                    .then_with(|| (b.1.plays + b.1.total_skips()).cmp(&(a.1.plays + a.1.total_skips())))
            });
            entries.truncate(MAX_ARTIST_SIGNALS);
            self.artist_signals = entries.into_iter().collect();
        }
    }
}

/// Time decay factor in `0.1..=1.0` with a 45-day long-term half-life.
fn recency_decay(last_s: u64, now_s: u64) -> f32 {
    if last_s == 0 || now_s <= last_s {
        return 1.0;
    }
    let age = (now_s - last_s) as f64;
    ((-age / LONG_DECAY_HALF_LIFE_S).exp2() as f32).clamp(0.10, 1.0)
}

/// Sublinear log-power compression so a single heavily-liked artist never
/// squashes the rest of a listener's multi-genre library to near-zero.
fn sublinear_pos(v: f32) -> f32 {
    if v <= 0.0 {
        0.0
    } else {
        (1.0 + v).ln().powf(1.15)
    }
}

/// Detects canonical version/style tokens from a track title and optional album
/// using strict word boundaries so regular words like `"Discover"` or `"Alive"`
/// are never misclassified.
pub fn detect_versions(title: &str, album: Option<&str>) -> Vec<&'static str> {
    let norm = |s: &str| -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push(' ');
        for c in s.to_lowercase().chars() {
            if c.is_alphanumeric() {
                out.push(c);
            } else {
                out.push(' ');
            }
        }
        out.push(' ');
        out
    };
    let t = norm(title);
    let a = album.map(norm).unwrap_or_default();
    let has_word = |hay: &str, phrase: &str| hay.contains(&format!(" {phrase} "));

    let mut out: Vec<&'static str> = Vec::new();
    let mut push = |style: &'static str| {
        if !out.contains(&style) {
            out.push(style);
        }
    };

    if has_word(&t, "karaoke") || has_word(&t, "backing track") { push("karaoke"); }
    if has_word(&t, "instrumental") { push("instrumental"); }
    if has_word(&t, "8d") || has_word(&t, "8d audio") { push("8d"); }
    if has_word(&t, "nightcore") { push("nightcore"); }
    if has_word(&t, "sped up") || has_word(&t, "speed up") || has_word(&t, "spedups") { push("sped_up"); }
    if has_word(&t, "slowed") || has_word(&t, "slowed reverb") || has_word(&t, "slowed down") { push("slowed"); }
    if has_word(&t, "cover") || has_word(&t, "tribute") || has_word(&t, "remake") { push("cover"); }
    if has_word(&t, "remix") || has_word(&t, "club mix") || has_word(&t, "extended mix") || has_word(&t, "vip mix") {
        push("remix");
    }
    if has_word(&t, "acoustic") || has_word(&t, "unplugged") { push("acoustic"); }
    if has_word(&t, "live")
        || has_word(&t, "live at")
        || has_word(&t, "live from")
        || has_word(&a, "live at")
        || has_word(&a, "live from")
        || has_word(&a, "unplugged")
    {
        push("live");
    }
    if has_word(&t, "lofi") || has_word(&t, "lo fi") { push("lofi"); }
    if has_word(&t, "demo") || has_word(&t, "demo version") { push("demo"); }
    out
}

/// Multi-dimensional listening taste & real-time session intent computed from a [`Library`].
#[derive(Debug, Default, Clone)]
pub struct Taste {
    /// Positive normalized artist weights (`0.0..=1.0`), sublinearly compressed.
    artists: HashMap<String, f32>,
    /// Short-term (~7 day) positive artist momentum (`0.0..=1.0`).
    artist_recent: HashMap<String, f32>,
    /// Signed artist affinities (`-1.0..=1.0`), capturing both love and dislike.
    artist_affinities: HashMap<String, f32>,
    /// Display spelling for known normalized artists.
    artist_display: HashMap<String, String>,
    /// Human-readable reason keys per artist (`"liked"`, `"followed"`, `"replayed"`, ...).
    artist_reasons: HashMap<String, Vec<String>>,
    /// Per-artist time-of-day normalized weights `[Morning, Afternoon, Evening, Night]`.
    artist_time: HashMap<String, [f32; 4]>,
    /// Cached normalized artist -> `(genres, moods, eras, similar)`.
    artist_meta: HashMap<String, (Vec<String>, Vec<String>, Vec<String>, Vec<String>)>,
    /// Signed genre affinities (`-1.0..=1.0`), keyed by canonical genre name.
    genre_scores: HashMap<String, f32>,
    /// Signed mood affinities (`-1.0..=1.0`), keyed by canonical mood name.
    mood_scores: HashMap<String, f32>,
    /// Signed decade/era affinities (`-1.0..=1.0`), keyed by canonical era (`"90s"`, ...).
    era_scores: HashMap<String, f32>,
    /// Learned + explicit version/style preference (`-1.0..=1.0`).
    version_scores: HashMap<String, f32>,
    /// Track-level skip suppression (`0.0..=1.0`), keyed by `song_key`.
    track_penalties: HashMap<String, f32>,
    /// Track-level replay/curation affinity (`0.0..=1.0`), keyed by `song_key`.
    track_love: HashMap<String, f32>,
    /// Track-level recency fatigue (`0.0..=1.0`, high if just played within a few hours).
    track_fatigue: HashMap<String, f32>,
    /// Tracks explicitly disliked by the user (`song_key`).
    disliked_tracks: HashSet<String>,
    /// Hidden / blocked artists (normalized).
    hidden: HashSet<String>,
    /// Liked tracks (`song_key`).
    liked: HashSet<String>,
    /// Active session genre vector (derived from recent session events + search queries).
    session_genres: HashMap<String, f32>,
    /// Active session mood vector.
    session_moods: HashMap<String, f32>,
    /// Recently skipped artists/genres/moods in current session (for VibeReset pivot).
    repelled_artists: HashSet<String>,
    repelled_genres: HashSet<String>,
    repelled_moods: HashSet<String>,
    /// Real-time inferred local listening intent.
    intent: InferredIntent,
    /// Variety score (`0.0..=1.0`) for exploration calibration.
    variety_score: f32,
    /// The user's country ("DE"), for a small lift of songs released there.
    region: Option<String>,
    /// Glyph's own artist graph: normalized artist -> linked artists with a
    /// strength in `0.0..=1.0`, strongest first. Built from cached similar
    /// artists, co-listened session pairs, playlist neighbours and the
    /// collaborative cache (see [`build_artist_links`]).
    artist_links: HashMap<String, Vec<(String, f32)>>,
    /// How close an artist is to what the listener loves, through the graph
    /// (`0.0..=1.0`). Artists the listener rejected are never in it.
    artist_graph: HashMap<String, f32>,
    /// Songs the listener picked from search before (`song_key`).
    search_picks: HashSet<String>,
}

/// Country part of a locale: "de-DE", "en_US.UTF-8", "zh-Hans-CN" -> DE, US, CN.
fn region_of(locale: &str) -> Option<String> {
    let base = locale.split(['.', '@']).next()?;
    base.split(['-', '_'])
        .skip(1)
        .find(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()))
        .map(|p| p.to_ascii_uppercase())
}

/// The user's country from the system locale (read once).
pub fn user_region() -> Option<String> {
    static REGION: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    REGION.get_or_init(|| sys_locale::get_locale().as_deref().and_then(region_of)).clone()
}

/// Registrant country of an ISRC ("DEUM71500123" -> DE).
fn isrc_country(isrc: &str) -> Option<String> {
    let c: String = isrc.trim().chars().take(2).collect();
    (c.len() == 2 && c.chars().all(|c| c.is_ascii_alphabetic())).then(|| c.to_ascii_uppercase())
}

/// Every credited artist of "A, B feat. C" (normalised).
pub fn credited(credit: &str) -> Vec<String> {
    credited_named(credit).into_iter().map(|(k, _)| k).collect()
}

const CREDIT_SEPS: [&str; 15] = [
    " feat. ", " feat ", " ft. ", " ft ", " featuring ", " & ", " x ", " × ", " with ", " vs. ", " vs ", " и ", ";", "/", ",",
];

fn credit_parts(credit: &str) -> Vec<&str> {
    let lower = credit.to_lowercase();
    if lower.len() != credit.len() {
        return credit.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    }
    let mut out: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < lower.len() {
        let hit = CREDIT_SEPS
            .iter()
            .filter(|sep| {
                lower[i..].starts_with(**sep)
                    && !(**sep == "," && lower[i..].starts_with(", the creator"))
            })
            .map(|sep| sep.len())
            .max();
        if let Some(len) = hit {
            let part = credit[start..i].trim();
            if !part.is_empty() {
                out.push(part);
            }
            i += len;
            start = i;
        } else {
            i += lower[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        }
    }
    let tail = credit[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Every credited artist of "A, B feat. C" as (normalised key, name as written).
pub fn credited_named(credit: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for part in credit_parts(credit) {
        let key = normalize_artist(part);
        if key.is_empty() || out.iter().any(|(k, _)| *k == key) {
            continue;
        }
        out.push((key, part.to_string()));
    }
    out
}

pub fn song_key(t: &Track) -> String {
    format!("{}|{}", normalize_title(&t.title), normalize_artist(&t.artist))
}

impl Taste {
    pub fn from_library(lib: &Library) -> Self {
        Self::from_library_at(lib, crate::paths::now_unix())
    }

    pub fn from_library_at(lib: &Library, now_s: u64) -> Self {
        Self::from_library_context(lib, now_s, local_hour(), local_is_weekend())
    }

    pub fn from_library_context(lib: &Library, now_s: u64, hour: u8, is_weekend: bool) -> Self {
        let mut raw_w: HashMap<String, f32> = HashMap::new();
        let mut display_map: HashMap<String, String> = HashMap::new();
        let mut reasons: HashMap<String, Vec<String>> = HashMap::new();

        let add_reason = |reasons: &mut HashMap<String, Vec<String>>, k: &str, r: &str| {
            let list = reasons.entry(k.to_string()).or_default();
            if !list.iter().any(|x| x == r) {
                list.push(r.to_string());
            }
        };

        let add = |raw_w: &mut HashMap<String, f32>,
                   display_map: &mut HashMap<String, String>,
                   reasons: &mut HashMap<String, Vec<String>>,
                   credit: &str,
                   v: f32,
                   reason: Option<&str>| {
            for (norm, disp) in credited_named(credit) {
                *raw_w.entry(norm.clone()).or_default() += v;
                display_map.entry(norm.clone()).or_insert(disp);
                if let Some(r) = reason {
                    add_reason(reasons, &norm, r);
                }
            }
        };

        // --- Explicit positive library signals (sublinear cap per artist on liked) ---
        let mut liked_per_artist: HashMap<String, u32> = HashMap::new();
        for t in &lib.liked {
            for (norm, disp) in credited_named(&t.artist) {
                let c = liked_per_artist.entry(norm.clone()).or_default();
                *c += 1;
                // Diminishing returns after 5 liked songs so one artist can't drown out others.
                let step = 3.0 / (1.0 + (*c - 1) as f32 * 0.22);
                *raw_w.entry(norm.clone()).or_default() += step;
                display_map.entry(norm.clone()).or_insert(disp);
                add_reason(&mut reasons, &norm, "liked");
            }
        }
        for f in &lib.followed {
            add(&mut raw_w, &mut display_map, &mut reasons, &f.name, 6.0, Some("followed"));
        }
        for pl in &lib.playlists {
            if pl.shared_by.is_some() {
                continue;
            }
            let mut per_pl: HashMap<String, u32> = HashMap::new();
            for t in &pl.tracks {
                for (norm, disp) in credited_named(&t.artist) {
                    let c = per_pl.entry(norm.clone()).or_default();
                    *c += 1;
                    if *c <= 3 {
                        *raw_w.entry(norm.clone()).or_default() += 1.0;
                        display_map.entry(norm.clone()).or_insert(disp);
                        add_reason(&mut reasons, &norm, "playlist");
                    }
                }
            }
        }

        // --- Onboarding picks (cold-start bootstrap) ---
        for a in &lib.taste.onboarding_artists {
            add(&mut raw_w, &mut display_map, &mut reasons, a, 6.5, Some("onboarding"));
        }
        for t in &lib.taste.onboarding_tracks {
            add(&mut raw_w, &mut display_map, &mut reasons, &t.artist, 4.0, Some("onboarding"));
        }

        // --- Implicit listening & recency ---
        for (i, t) in lib.recent_tracks().enumerate() {
            if lib.taste.is_disliked_track(t) {
                continue;
            }
            if lib
                .taste
                .track_signals
                .get(&song_key(t))
                .is_some_and(|s| s.plays == 0 && s.total_skips() > 0)
            {
                continue;
            }
            add(&mut raw_w, &mut display_map, &mut reasons, &t.artist, 1.5 * 0.96f32.powi(i as i32), Some("recent"));
        }
        for (a, n) in &lib.artist_plays {
            for norm in credited(a) {
                let tracked = lib
                    .taste
                    .artist_signals
                    .get(&norm)
                    .map_or(0, |s| s.plays.saturating_add(s.total_skips()));
                let legacy_plays = n.saturating_sub(tracked);
                if legacy_plays > 0 {
                    *raw_w.entry(norm.clone()).or_default() += (1.0 + legacy_plays as f32).ln() * 1.2;
                    if legacy_plays >= 3 {
                        add_reason(&mut reasons, &norm, "frequent");
                    }
                }
            }
        }
        for (a, n) in &lib.artist_skips {
            for norm in credited(a) {
                let tracked = lib
                    .taste
                    .artist_signals
                    .get(&norm)
                    .map_or(0, |s| s.total_skips());
                let legacy_skips = n.saturating_sub(tracked);
                if legacy_skips > 0 {
                    *raw_w.entry(norm.clone()).or_default() -= (legacy_skips as f32).min(10.0) * 0.6;
                    if legacy_skips >= 3 {
                        add_reason(&mut reasons, &norm, "skips");
                    }
                }
            }
        }

        // --- Rich implicit behavioral & intent signals (dual-horizon!) ---
        let mut artist_time: HashMap<String, [f32; 4]> = HashMap::new();
        let mut recent_raw: HashMap<String, f32> = HashMap::new();
        for (norm, sig) in &lib.taste.artist_signals {
            if !sig.display.is_empty() {
                display_map.entry(norm.clone()).or_insert_with(|| sig.display.clone());
            }
            let decay = recency_decay(sig.last_event_s, now_s);
            let pos = (1.0 + sig.plays as f32).ln() * 1.1
                + (1.0 + sig.completions as f32).ln() * 1.45
                + (1.0 + sig.replays as f32).ln() * 2.15
                + (1.0 + sig.deliberate_plays as f32).ln() * 0.85;
            let neg = sig.instant_skips as f32 * 1.55
                + sig.early_skips as f32 * 0.85
                + sig.mid_abandons as f32 * 0.30
                + (sig.consecutive_skips.min(6) as f32) * 0.70;
            let abandon = sig.abandon_rate();
            let abandon_penalty = if abandon > 0.45 && sig.total_skips() >= 2 {
                abandon * 4.0
            } else {
                0.0
            };
            let short_momentum = sig.decayed_recent(now_s);
            if short_momentum > 0.1 {
                recent_raw.insert(norm.clone(), short_momentum);
            }
            // Dual-horizon blend: long-term decayed signal + short-term 7-day phase momentum.
            let delta = (pos - neg - abandon_penalty) * decay + short_momentum * 0.45;
            *raw_w.entry(norm.clone()).or_default() += delta;
            if sig.replays >= 2 {
                add_reason(&mut reasons, norm, "replayed");
            }
            if sig.completions >= 3 && abandon < 0.3 {
                add_reason(&mut reasons, norm, "frequent");
            }
            if abandon >= 0.45 && sig.total_skips() >= 2 {
                add_reason(&mut reasons, norm, "high_skips");
            }
            let sum_t: f32 = sig.time_buckets.iter().sum();
            if sum_t > 0.0 {
                artist_time.insert(
                    norm.clone(),
                    [
                        sig.time_buckets[0] / sum_t,
                        sig.time_buckets[1] / sum_t,
                        sig.time_buckets[2] / sum_t,
                        sig.time_buckets[3] / sum_t,
                    ],
                );
            }
        }

        // --- Explicit recommendation feedback ---
        for (a, preference) in &lib.recommendation_feedback {
            let reason = if *preference > 0 { "more" } else { "less" };
            let delta = if *preference > 0 { 8.0 } else { -12.0 };
            for norm in credited(a) {
                *raw_w.entry(norm.clone()).or_default() += delta;
                add_reason(&mut reasons, &norm, reason);
            }
        }

        // --- Recent search picks AND search query genre/mood/era intent ---
        let mut query_genres: Vec<String> = Vec::new();
        let mut query_moods: Vec<String> = Vec::new();
        let mut query_eras: Vec<String> = Vec::new();
        for (i, item) in lib.search_items.iter().take(40).enumerate() {
            let fade = 0.95f32.powi(i as i32);
            match item {
                crate::library::SearchItem::Artist { name, .. } => {
                    add(&mut raw_w, &mut display_map, &mut reasons, name, 2.5 * fade, None);
                }
                crate::library::SearchItem::Song { track } => {
                    add(&mut raw_w, &mut display_map, &mut reasons, &track.artist, 1.5 * fade, None);
                }
                crate::library::SearchItem::Query { text } if i < 10 => {
                    let (gs, ms, es) = extract_query_intent(text);
                    for g in gs {
                        if !query_genres.contains(&g) { query_genres.push(g); }
                    }
                    for m in ms {
                        if !query_moods.contains(&m) { query_moods.push(m); }
                    }
                    for e in es {
                        if !query_eras.contains(&e) { query_eras.push(e); }
                    }
                }
                crate::library::SearchItem::Query { .. } => {}
            }
        }

        // --- Hidden / blocked artists ---
        let hidden: HashSet<String> = lib
            .hidden_artists
            .iter()
            .chain(
                lib.recommendation_feedback
                    .iter()
                    .filter(|(_, v)| **v <= -2)
                    .map(|(a, _)| a),
            )
            .flat_map(|h| credited(h))
            .collect();
        for h in &hidden {
            raw_w.remove(h);
            recent_raw.remove(h);
        }

        // Sublinear log-power normalization for positive artist weights!
        let max_pos_comp = raw_w
            .values()
            .filter(|v| **v > 0.0)
            .map(|v| sublinear_pos(*v))
            .fold(0.0f32, f32::max)
            .max(1.0);
        let max_neg = raw_w
            .values()
            .filter(|v| **v < 0.0)
            .map(|v| v.abs())
            .fold(0.0f32, f32::max)
            .max(4.0);
        let max_recent = recent_raw.values().cloned().fold(0.0f32, f32::max).max(1.0);

        let mut artists: HashMap<String, f32> = HashMap::new();
        let mut artist_affinities: HashMap<String, f32> = HashMap::new();
        let mut artist_recent: HashMap<String, f32> = HashMap::new();
        for (k, v) in raw_w {
            if v > 0.0 {
                let norm_v = (sublinear_pos(v) / max_pos_comp).clamp(0.0, 1.0);
                artists.insert(k.clone(), norm_v);
                artist_affinities.insert(k.clone(), norm_v);
            } else if v < 0.0 {
                let neg_v = -(v.abs() / max_neg).min(1.0);
                artist_affinities.insert(k, neg_v);
            }
        }
        for (k, v) in recent_raw {
            artist_recent.insert(k, (v / max_recent).clamp(0.0, 1.0));
        }
        for h in &hidden {
            artist_affinities.insert(h.clone(), -1.0);
        }

        // --- Track-level skip penalties, replay/curation love, recency fatigue & explicit dislikes ---
        let mut track_penalties: HashMap<String, f32> = HashMap::new();
        let mut track_love: HashMap<String, f32> = HashMap::new();
        let mut track_fatigue: HashMap<String, f32> = HashMap::new();
        for (k, sig) in &lib.taste.track_signals {
            let decay = recency_decay(sig.last_event_s, now_s);
            let sk = sig.weighted_skips();
            if sk > 0.0 {
                let pos = sig.plays as f32 + sig.completions as f32 * 0.8 + sig.replays as f32 * 1.4;
                let base = sk / (pos + sk + 1.5);
                let streak_boost = (sig.consecutive_skips.min(4) as f32) * 0.14;
                let pen = ((base + streak_boost) * decay).clamp(0.0, 1.0);
                if pen > 0.12 {
                    track_penalties.insert(k.clone(), pen);
                }
            }
            let love_raw = sig.replays as f32 * 0.45
                + sig.queue_adds as f32 * 0.25
                + sig.playlist_adds as f32 * 0.30
                + sig.deliberate_plays as f32 * 0.15;
            if love_raw > 0.0 {
                track_love.insert(k.clone(), (love_raw / (love_raw + 1.2)).clamp(0.0, 1.0));
            }
            // Recency fatigue: tracks played in the last 6 hours gently cool down in mixes.
            if sig.last_played_s > 0 && now_s >= sig.last_played_s {
                let age_s = now_s - sig.last_played_s;
                if age_s < 6 * 3600 {
                    let fat = (1.0 - (age_s as f32 / (6.0 * 3600.0))).clamp(0.0, 1.0);
                    track_fatigue.insert(k.clone(), fat);
                }
            }
        }
        let disliked_tracks: HashSet<String> = lib.taste.disliked_tracks.keys().cloned().collect();
        for dk in &disliked_tracks {
            track_penalties.insert(dk.clone(), 1.0);
        }

        // --- Artist genre/mood/era metadata map (cached + built-in fallback) ---
        let mut artist_meta: HashMap<String, (Vec<String>, Vec<String>, Vec<String>, Vec<String>)> =
            HashMap::new();
        for (k, c) in &lib.taste.artist_tags {
            if !c.artist.is_empty() {
                display_map.entry(k.clone()).or_insert_with(|| c.artist.clone());
            }
            let (mut genres, mut moods, eras) = (c.genres.clone(), c.moods.clone(), c.eras.clone());
            if genres.is_empty() {
                if let Some(gs) = builtin_artist_genres(k) {
                    let raw: Vec<String> = gs.iter().map(|s| (*s).to_string()).collect();
                    let (fg, fm, _, _) = classify_tags_full(&raw, k);
                    genres = fg;
                    if moods.is_empty() {
                        moods = fm;
                    }
                }
            }
            artist_meta.insert(k.clone(), (genres, moods, eras, c.similar.clone()));
        }
        for k in artist_affinities
            .keys()
            .chain(lib.taste.session_events.iter().map(|e| &e.artist))
        {
            let norm = normalize_artist(k);
            let key = if norm.is_empty() { k.clone() } else { norm };
            if !artist_meta.contains_key(&key) {
                if let Some(gs) = builtin_artist_genres(&key) {
                    let raw: Vec<String> = gs.iter().map(|s| (*s).to_string()).collect();
                    let (genres, moods, eras, _) = classify_tags_full(&raw, &key);
                    artist_meta.insert(key, (genres, moods, eras, Vec::new()));
                }
            }
        }

        // --- Aggregate genre, mood & era scores with breadth bonus + explicit feedback ---
        let mut genre_raw: HashMap<String, f32> = HashMap::new();
        let mut genre_artists: HashMap<String, u32> = HashMap::new();
        let mut mood_raw: HashMap<String, f32> = HashMap::new();
        let mut era_raw: HashMap<String, f32> = HashMap::new();

        for g in &lib.taste.onboarding_genres {
            *genre_raw.entry(g.clone()).or_default() += 1.25;
        }
        for g in &query_genres {
            *genre_raw.entry(g.clone()).or_default() += 0.55;
        }
        for m in &query_moods {
            *mood_raw.entry(m.clone()).or_default() += 0.55;
        }
        for e in &query_eras {
            *era_raw.entry(e.clone()).or_default() += 0.60;
        }

        for (artist_k, aff) in &artist_affinities {
            if let Some((genres, moods, eras, _)) = artist_meta.get(artist_k) {
                let eff_aff = if *aff >= 0.0 { *aff } else { *aff * 0.35 };
                for (i, g) in genres.iter().enumerate() {
                    let rank_w = 1.0 / (1.0 + i as f32 * 0.35);
                    *genre_raw.entry(g.clone()).or_default() += eff_aff * rank_w;
                    if *aff > 0.15 {
                        *genre_artists.entry(g.clone()).or_default() += 1;
                    }
                }
                for (i, m) in moods.iter().enumerate() {
                    let rank_w = 1.0 / (1.0 + i as f32 * 0.35);
                    *mood_raw.entry(m.clone()).or_default() += eff_aff * rank_w;
                }
                for e in eras {
                    *era_raw.entry(e.clone()).or_default() += eff_aff * 0.7;
                }
            }
        }
        // Genre breadth lift: genres supported by multiple distinct loved artists gain confidence.
        for (g, v) in &mut genre_raw {
            if *v > 0.0 {
                let distinct = genre_artists.get(g).copied().unwrap_or(1);
                *v *= 1.0 + 0.18 * (distinct as f32).ln();
            }
        }

        // Apply explicit tag feedback (`+1` / `-1`).
        for (tag_l, pref) in &lib.taste.tag_feedback {
            let delta = if *pref > 0 { 2.5 } else { -3.5 };
            let mut matched = false;
            for (g, v) in &mut genre_raw {
                if g.to_lowercase() == *tag_l {
                    *v += delta;
                    matched = true;
                }
            }
            for (m, v) in &mut mood_raw {
                if m.to_lowercase() == *tag_l {
                    *v += delta;
                    matched = true;
                }
            }
            if !matched {
                let label = title_case(tag_l);
                *genre_raw.entry(label).or_default() += delta;
            }
        }

        let normalize_signed = |raw: HashMap<String, f32>| -> HashMap<String, f32> {
            let max_p = raw.values().cloned().fold(0.0f32, f32::max).max(1.0);
            let max_n = raw
                .values()
                .filter(|v| **v < 0.0)
                .map(|v| v.abs())
                .fold(0.0f32, f32::max)
                .max(1.5);
            raw.into_iter()
                .map(|(k, v)| {
                    let s = if v >= 0.0 {
                        (v / max_p).min(1.0)
                    } else {
                        -(v.abs() / max_n).min(1.0)
                    };
                    (k, s)
                })
                .collect()
        };
        let genre_scores = normalize_signed(genre_raw);
        let mood_scores = normalize_signed(mood_raw);
        let era_scores = normalize_signed(era_raw);

        // --- Adaptive version/style preferences ---
        let mut version_scores: HashMap<String, f32> = HashMap::new();
        for &style in VERSION_STYLES {
            let explicit = lib.taste.version_feedback.get(style).copied();
            let sig = lib.taste.version_signals.get(style);
            let liked_cnt = lib
                .liked
                .iter()
                .filter(|t| detect_versions(&t.title, t.album.as_deref()).contains(&style))
                .count() as f32;
            let score = if let Some(p) = explicit.filter(|p| *p != 0) {
                if p > 0 { 0.85 } else { -0.95 }
            } else {
                let plays = sig.map_or(0.0, |s| s.plays as f32 + s.completions as f32 * 0.7) + liked_cnt * 2.0;
                let skips = sig.map_or(0.0, |s| s.skips as f32);
                let total = plays + skips;
                if total >= 1.0 {
                    let prior = if DEFAULT_ODD_VERSIONS.contains(&style) { -0.55 } else { 0.0 };
                    let learned = (plays - skips * 1.25) / (total + 2.0);
                    let conf = (total / (total + 2.5)).clamp(0.0, 1.0);
                    (prior * (1.0 - conf) + learned * conf).clamp(-1.0, 1.0)
                } else if DEFAULT_ODD_VERSIONS.contains(&style) {
                    -0.55
                } else {
                    0.0
                }
            };
            version_scores.insert(style.to_string(), score);
        }

        // Variety score (normalized Shannon entropy across positive artists).
        let variety_score = {
            let vals: Vec<f32> = artists.values().copied().filter(|v| *v > 0.02).collect();
            if vals.len() <= 1 {
                0.0
            } else {
                let sum: f32 = vals.iter().sum();
                let h: f32 = vals
                    .iter()
                    .map(|v| {
                        let p = *v / sum;
                        -p * p.ln()
                    })
                    .sum();
                (h / (vals.len() as f32).ln()).clamp(0.0, 1.0)
            }
        };

        // --- Local real-time session intention inference ---
        let (intent, session_genres, session_moods, repelled_artists, repelled_genres, repelled_moods) =
            infer_local_intent(
                &lib.taste.session_events,
                &query_genres,
                &query_moods,
                &artist_meta,
                &genre_scores,
                &mood_scores,
                variety_score,
                hour,
                is_weekend,
                now_s,
            );

        let liked = lib.liked.iter().map(song_key).collect();
        let artist_links = build_artist_links(lib, &artist_meta, now_s);
        let artist_graph = build_artist_graph(lib, &artists, &artist_affinities, &hidden, &artist_links);
        let search_picks = lib
            .search_items
            .iter()
            .filter_map(|i| match i {
                crate::library::SearchItem::Song { track } => Some(song_key(track)),
                _ => None,
            })
            .collect();

        Self {
            artists,
            artist_recent,
            artist_affinities,
            artist_display: display_map,
            artist_reasons: reasons,
            artist_time,
            artist_meta,
            genre_scores,
            mood_scores,
            era_scores,
            version_scores,
            track_penalties,
            track_love,
            track_fatigue,
            disliked_tracks,
            hidden,
            liked,
            session_genres,
            session_moods,
            repelled_artists,
            repelled_genres,
            repelled_moods,
            intent,
            variety_score,
            region: user_region(),
            artist_links,
            artist_graph,
            search_picks,
        }
    }

    /// Same weights for another country (tests, server-side ranking).
    pub fn with_region(mut self, region: Option<&str>) -> Self {
        self.region = region.map(str::to_ascii_uppercase);
        self
    }

    pub fn inferred_intent(&self) -> &InferredIntent {
        &self.intent
    }

    pub fn variety_score(&self) -> f32 {
        self.variety_score
    }

    /// 0..1, the best positive weight of the credited artists.
    pub fn artist_weight(&self, credit: &str) -> f32 {
        credited(credit)
            .iter()
            .filter_map(|a| self.artists.get(a))
            .cloned()
            .fold(0.0, f32::max)
    }

    /// Signed artist affinity in `-1.0..=1.0`.
    pub fn artist_affinity(&self, credit: &str) -> f32 {
        let mut best: Option<f32> = None;
        for a in credited(credit) {
            if self.hidden.contains(&a) {
                return -1.0;
            }
            if let Some(&v) = self.artist_affinities.get(&a) {
                best = Some(match best {
                    Some(cur) => cur.max(v),
                    None => v,
                });
            }
        }
        best.unwrap_or(0.0)
    }

    pub fn is_hidden(&self, credit: &str) -> bool {
        credited(credit).iter().any(|a| self.hidden.contains(a))
    }

    pub fn is_liked(&self, t: &Track) -> bool {
        self.liked.contains(&song_key(t))
    }

    pub fn is_disliked_track(&self, t: &Track) -> bool {
        self.disliked_tracks.contains(&song_key(t))
    }

    /// How close `credit` is to the listener's favourites through Glyph's
    /// artist graph (`0.0..=1.0`); `0.0` for unknown or rejected artists.
    pub fn graph_affinity(&self, credit: &str) -> f32 {
        credited(credit)
            .iter()
            .filter_map(|a| self.artist_graph.get(a))
            .copied()
            .fold(0.0, f32::max)
    }

    /// Linked artists of one normalized artist, strongest first.
    pub fn artist_links(&self, artist_key: &str) -> &[(String, f32)] {
        self.artist_links.get(artist_key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Display spelling of a normalized artist (the key itself when unknown).
    pub fn display_artist(&self, artist_key: &str) -> String {
        self.artist_display.get(artist_key).cloned().unwrap_or_else(|| title_case(artist_key))
    }

    /// The listener picked this song from search before.
    pub fn is_search_pick(&self, t: &Track) -> bool {
        self.search_picks.contains(&song_key(t))
    }

    /// Canonical genres, moods and eras of `t` (from its credited artists).
    pub fn track_tags(&self, t: &Track) -> (Vec<String>, Vec<String>, Vec<String>) {
        self.track_genres_moods_eras(t)
    }

    pub fn track_skip_penalty(&self, t: &Track) -> f32 {
        self.track_penalties.get(&song_key(t)).copied().unwrap_or(0.0)
    }

    pub fn track_love_score(&self, t: &Track) -> f32 {
        self.track_love.get(&song_key(t)).copied().unwrap_or(0.0)
    }

    pub fn track_fatigue(&self, t: &Track) -> f32 {
        self.track_fatigue.get(&song_key(t)).copied().unwrap_or(0.0)
    }

    /// Signed version/style affinity in `-1.0..=1.0` for `t`.
    pub fn version_affinity(&self, t: &Track) -> f32 {
        let styles = detect_versions(&t.title, t.album.as_deref());
        if styles.is_empty() {
            return 0.0;
        }
        let mut sum = 0.0f32;
        for s in &styles {
            sum += self.version_scores.get(*s).copied().unwrap_or(0.0);
        }
        (sum / styles.len() as f32).clamp(-1.0, 1.0)
    }

    fn track_genres_moods_eras(&self, t: &Track) -> (Vec<String>, Vec<String>, Vec<String>) {
        let mut gs: Vec<String> = Vec::new();
        let mut ms: Vec<String> = Vec::new();
        let mut es: Vec<String> = Vec::new();
        for a in credited(&t.artist) {
            if let Some((genres, moods, eras, _)) = self.artist_meta.get(&a) {
                for g in genres {
                    if !gs.contains(g) { gs.push(g.clone()); }
                }
                for m in moods {
                    if !ms.contains(m) { ms.push(m.clone()); }
                }
                for e in eras {
                    if !es.contains(e) { es.push(e.clone()); }
                }
            } else if let Some(builtin) = builtin_artist_genres(&a) {
                for &g in builtin {
                    if !gs.iter().any(|x| x == g) { gs.push(g.to_string()); }
                    for &m in genre_default_moods(g) {
                        if !ms.iter().any(|x| x == m) { ms.push(m.to_string()); }
                    }
                }
            }
        }
        (gs, ms, es)
    }

    /// Signed soft genre/mood/era affinity in `-1.0..=1.0` using canonical
    /// genre-family & mood similarity normalized over all candidate tags so
    /// unrelated tracks cannot get inflated scores from a single loose mood overlap.
    pub fn track_tag_affinity(&self, t: &Track) -> f32 {
        let (gs, ms, es) = self.track_genres_moods_eras(t);
        let mut sum = 0.0f32;
        let mut total_w = 0.0f32;
        if !self.genre_scores.is_empty() {
            for g in &gs {
                total_w += 1.0;
                let mut best_sim = 0.0f32;
                let mut best_sc = 0.0f32;
                for (ug, &us) in &self.genre_scores {
                    let sim = genre_similarity(g, ug);
                    if sim > best_sim {
                        best_sim = sim;
                        best_sc = us;
                    }
                }
                if best_sim >= 0.55 {
                    sum += best_sc * best_sim;
                }
            }
        }
        if !self.mood_scores.is_empty() {
            for m in &ms {
                total_w += 0.45;
                let mut best_sim = 0.0f32;
                let mut best_sc = 0.0f32;
                for (um, &us) in &self.mood_scores {
                    let sim = mood_similarity(m, um);
                    if sim > best_sim {
                        best_sim = sim;
                        best_sc = us;
                    }
                }
                if best_sim >= 0.55 {
                    sum += best_sc * best_sim * 0.45;
                }
            }
        }
        if !self.era_scores.is_empty() {
            for e in &es {
                total_w += 0.25;
                if let Some(&us) = self.era_scores.get(e) {
                    sum += us * 0.25;
                }
            }
        }
        if total_w > 0.0 {
            (sum / total_w).clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }

    /// Short-term session intent alignment in `-1.0..=1.0` (includes VibeReset repulsion).
    pub fn session_intent_affinity(&self, t: &Track) -> f32 {
        let (gs, ms, _) = self.track_genres_moods_eras(t);
        let mut sum = 0.0f32;
        let mut total_w = 0.0f32;
        if !self.session_genres.is_empty() {
            for g in &gs {
                total_w += 1.0;
                let mut best_sim = 0.0f32;
                let mut best_ss = 0.0f32;
                for (sg, &ss) in &self.session_genres {
                    let sim = genre_similarity(g, sg);
                    if sim > best_sim {
                        best_sim = sim;
                        best_ss = ss;
                    }
                }
                if best_sim >= 0.60 {
                    sum += best_ss.clamp(-1.5, 1.5) * best_sim;
                }
            }
        }
        if !self.session_moods.is_empty() {
            for m in &ms {
                total_w += 0.50;
                let mut best_sim = 0.0f32;
                let mut best_ss = 0.0f32;
                for (sm, &ss) in &self.session_moods {
                    let sim = mood_similarity(m, sm);
                    if sim > best_sim {
                        best_sim = sim;
                        best_ss = ss;
                    }
                }
                if best_sim >= 0.60 {
                    sum += best_ss.clamp(-1.5, 1.5) * best_sim * 0.50;
                }
            }
        }
        let mut base = if total_w > 0.0 { (sum / total_w).clamp(-1.0, 1.0) } else { 0.0 };
        if self.intent.vibe_pivot {
            if gs.iter().any(|g| self.repelled_genres.contains(g)) {
                base -= 0.75;
            }
            if ms.iter().any(|m| self.repelled_moods.contains(m)) {
                base -= 0.45;
            }
        }
        base.clamp(-1.0, 1.0)
    }

    /// Time-of-day alignment in `0.0..=1.0` for `t` in `bucket`.
    pub fn time_affinity(&self, t: &Track, bucket: TimeBucket) -> f32 {
        let idx = bucket.index();
        let mut best = 0.0f32;
        let aligned = bucket.aligned_moods();
        for a in credited(&t.artist) {
            if let Some(arr) = self.artist_time.get(&a) {
                best = best.max(arr[idx]);
            }
            if let Some((_, moods, _, _)) = self.artist_meta.get(&a) {
                let hits = moods.iter().filter(|m| aligned.contains(&m.as_str())).count();
                if hits > 0 {
                    best = best.max((hits as f32 * 0.35).min(0.75));
                }
            } else if let Some(gs) = builtin_artist_genres(&a) {
                let mut hits = 0usize;
                for &g in gs {
                    hits += genre_default_moods(g)
                        .iter()
                        .filter(|m| aligned.contains(m))
                        .count();
                }
                if hits > 0 {
                    best = best.max((hits as f32 * 0.30).min(0.75));
                }
            }
        }
        best.clamp(0.0, 1.0)
    }

    /// Unified intent-aware score for Home recommendations, Radio / Autoplay,
    /// Daily Mixes, and playlist suggestions.
    pub fn score_recommendation(
        &self,
        t: &Track,
        base_sim: f32,
        bucket: TimeBucket,
        is_familiar: bool,
    ) -> f32 {
        if self.is_hidden(&t.artist) || self.is_disliked_track(t) {
            return f32::NEG_INFINITY;
        }
        let artist_aff = self.artist_affinity(&t.artist);
        let tag_aff = self.track_tag_affinity(t);
        let sess_aff = self.session_intent_affinity(t);
        let time_aff = self.time_affinity(t, bucket);
        let ver_aff = self.version_affinity(t);
        let skip_pen = self.track_skip_penalty(t);
        let love = self.track_love_score(t);
        let fatigue = self.track_fatigue(t);

        let artist_scale = match self.intent.mode {
            ListeningMode::Comfort => 1.15,
            ListeningMode::ArtistDive => 1.10,
            ListeningMode::Discovery => 0.70,
            _ => 0.95,
        };

        let mut sc = base_sim;
        sc += if artist_aff >= 0.0 {
            artist_aff * artist_scale
        } else {
            artist_aff * 2.6
        };
        sc += tag_aff * 0.90;
        sc += sess_aff * 0.75;
        sc += time_aff * 0.35;
        sc += ver_aff * 1.1;
        sc += love * 0.45;
        sc -= skip_pen * 2.8;
        if !is_familiar {
            sc -= fatigue * 0.85;
        } else {
            sc -= fatigue * 0.35;
        }

        // Session repulsion for artists just skipped in this session.
        if credited(&t.artist).iter().any(|a| self.repelled_artists.contains(a)) {
            sc -= 2.2;
        }

        // Glyph artist graph: a new artist next to several favourites is a
        // far better bet than a random one with matching tags. Known artists
        // already carry their own affinity, so the lift fades for them.
        if artist_aff >= 0.0 {
            let graph = self.graph_affinity(&t.artist);
            if graph > 0.0 {
                let known = self.artist_weight(&t.artist);
                let novelty = if is_familiar { 0.5 } else { 0.7 + 0.3 * self.intent.novelty.clamp(0.0, 1.0) };
                sc += graph * 0.85 * (1.0 - known).max(0.25) * novelty;
            }
        }

        // Discovery lift calibrated to user's inferred novelty preference.
        if !is_familiar && tag_aff > 0.15 && artist_aff >= 0.0 {
            sc += 0.40 * self.intent.novelty.clamp(0.25, 1.0);
        }
        if t.cover_url.as_deref().is_none_or(|c| c.trim().is_empty()) {
            sc -= 0.2;
        }
        sc
    }

    /// Maximal Marginal Relevance (MMR) local re-ranker for Home / Radio lists:
    /// prevents consecutive same-artist tracks and clusters of identical genres.
    pub fn mmr_rank_tracks(
        &self,
        scored: &[(Track, f32)],
        limit: usize,
        max_per_artist: usize,
    ) -> Vec<Track> {
        if scored.is_empty() || limit == 0 {
            return Vec::new();
        }
        let pool_len = scored.len().min(limit * 4).max(limit.min(scored.len()));
        let slice = &scored[..pool_len];
        let max_s = slice.first().map(|x| x.1).unwrap_or(1.0);
        let min_s = slice.last().map(|x| x.1).unwrap_or(0.0);
        let span = (max_s - min_s).max(0.001);
        let lambda = match self.intent.mode {
            ListeningMode::ArtistDive => 0.84,
            ListeningMode::Discovery | ListeningMode::VibeReset => 0.66,
            _ => 0.74,
        };

        let track_genres: Vec<Vec<String>> = slice
            .iter()
            .map(|(t, _)| self.track_genres_moods_eras(t).0)
            .collect();
        let mut used = vec![false; pool_len];
        let mut per_artist: HashMap<String, usize> = HashMap::new();
        let mut picked_artists: Vec<String> = Vec::with_capacity(limit);
        let mut picked_genres: Vec<&[String]> = Vec::with_capacity(limit);
        let mut out: Vec<Track> = Vec::with_capacity(limit);

        while out.len() < limit {
            let last_artist = picked_artists.last().map(String::as_str);
            let mut best_idx: Option<usize> = None;
            let mut best_mmr = f32::NEG_INFINITY;

            for (i, (track, raw_sc)) in slice.iter().enumerate() {
                if used[i] {
                    continue;
                }
                let ak = normalize_artist(&primary_artist(&track.artist));
                let cnt = per_artist.get(&ak).copied().unwrap_or(0);
                if cnt >= max_per_artist {
                    continue;
                }
                let back_to_back = last_artist == Some(ak.as_str());
                let rel = ((*raw_sc - min_s) / span).clamp(0.0, 1.0);
                let mut max_red: f32 = if back_to_back { 1.0 } else { 0.0 };
                if cnt > 0 {
                    max_red = max_red.max(0.72);
                }
                for prev_g in picked_genres.iter().rev().take(3) {
                    for g1 in &track_genres[i] {
                        for g2 in *prev_g {
                            let sim = genre_similarity(g1, g2);
                            if sim > 0.6 {
                                max_red = max_red.max(sim * 0.42);
                            }
                        }
                    }
                }
                let mmr = lambda * rel - (1.0 - lambda) * max_red - if back_to_back { 2.0 } else { 0.0 };
                if mmr > best_mmr {
                    best_mmr = mmr;
                    best_idx = Some(i);
                }
            }

            let Some(idx) = best_idx else { break };
            used[idx] = true;
            let track = slice[idx].0.clone();
            let ak = normalize_artist(&primary_artist(&track.artist));
            *per_artist.entry(ak.clone()).or_default() += 1;
            picked_artists.push(ak);
            picked_genres.push(&track_genres[idx]);
            out.push(track);
        }
        out
    }

    /// Builds a compact, anonymized `TasteRecommendRequest` for the server-first
    /// Taste Engine (`POST /v1/taste/recommend`), keeping payload well under 20 KB.
    #[allow(clippy::too_many_arguments)]
    pub fn build_recommend_request(
        &self,
        lib: &Library,
        purpose: TastePurpose,
        seed_tracks: Vec<TasteSeedTrack>,
        extra_exclude_keys: &[String],
        date_key: String,
        hour: u8,
        is_weekend: bool,
        hide_explicit: bool,
        now_s: u64,
        limit: Option<usize>,
    ) -> TasteRecommendRequest {
        let positive_artists = self.top_positive_signals(32);
        let negative_artists = self.negative_artists(28);
        let avoided_tags = self.avoided_tags(16);

        let mut genre_scores: Vec<TasteTagSignal> = self
            .genre_scores
            .iter()
            .filter(|(_, s)| s.abs() > 0.08)
            .map(|(k, s)| TasteTagSignal { tag: k.clone(), score: *s })
            .collect();
        genre_scores.sort_by(|a, b| b.score.abs().total_cmp(&a.score.abs()));
        genre_scores.truncate(18);

        let mut mood_scores: Vec<TasteTagSignal> = self
            .mood_scores
            .iter()
            .filter(|(_, s)| s.abs() > 0.08)
            .map(|(k, s)| TasteTagSignal { tag: k.clone(), score: *s })
            .collect();
        mood_scores.sort_by(|a, b| b.score.abs().total_cmp(&a.score.abs()));
        mood_scores.truncate(14);

        let mut era_scores: Vec<TasteTagSignal> = self
            .era_scores
            .iter()
            .filter(|(_, s)| s.abs() > 0.08)
            .map(|(k, s)| TasteTagSignal { tag: k.clone(), score: *s })
            .collect();
        era_scores.sort_by(|a, b| b.score.abs().total_cmp(&a.score.abs()));
        era_scores.truncate(6);

        let version_scores: BTreeMap<String, f32> = self
            .version_scores
            .iter()
            .filter(|(_, v)| v.abs() > 0.05)
            .map(|(k, v)| (k.clone(), *v))
            .collect();

        // Compact session events (quantized to age_mins, never raw timestamps).
        let recent_events: Vec<SessionEventSignal> = lib
            .taste
            .session_events
            .iter()
            .filter(|e| now_s.saturating_sub(e.timestamp_s) <= SESSION_WINDOW_S)
            .take(14)
            .map(|e| SessionEventSignal {
                title: e.title.clone(),
                artist: e.artist.clone(),
                album: e.album.clone(),
                outcome: e.outcome,
                context: e.context,
                age_mins: (now_s.saturating_sub(e.timestamp_s) / 60).min(720) as u16,
            })
            .collect();

        let recent_skip_streak = recent_events
            .iter()
            .take_while(|e| matches!(e.outcome, SessionEventOutcome::SkippedEarly | SessionEventOutcome::SkippedMid))
            .count()
            .min(10) as u8;

        let mut query_genres = Vec::new();
        let mut query_moods = Vec::new();
        let mut query_eras = Vec::new();
        for item in lib.search_items.iter().take(6) {
            if let crate::library::SearchItem::Query { text } = item {
                let (gs, ms, es) = extract_query_intent(text);
                for g in gs { if !query_genres.contains(&g) && query_genres.len() < 4 { query_genres.push(g); } }
                for m in ms { if !query_moods.contains(&m) && query_moods.len() < 4 { query_moods.push(m); } }
                for e in es { if !query_eras.contains(&e) && query_eras.len() < 2 { query_eras.push(e); } }
            }
        }

        let session_pairs: Vec<(String, String)> = lib
            .taste
            .session_pairs
            .iter()
            .take(12)
            .map(|(a, b, _)| (a.clone(), b.clone()))
            .collect();

        // Compact sample of familiar tracks (up to 32, stripped of heavy external_ids) for Home Daily Mixes.
        let mut familiar_tracks: Vec<Track> = Vec::new();
        if purpose == TastePurpose::Home {
            let mut seen_fam: HashSet<String> = HashSet::new();
            for t in lib
                .liked
                .iter()
                .chain(lib.taste.onboarding_tracks.iter())
                .chain(lib.recent_tracks().take(20))
            {
                if (hide_explicit && t.maybe_explicit())
                    || self.is_hidden(&t.artist)
                    || self.is_disliked_track(t)
                {
                    continue;
                }
                let k = song_key(t);
                if seen_fam.insert(k) {
                    let mut slim = t.clone();
                    slim.external_ids.clear();
                    familiar_tracks.push(slim);
                    if familiar_tracks.len() >= 32 {
                        break;
                    }
                }
            }
        }

        let mut known_track_keys: Vec<String> = Vec::new();
        let mut seen_known: HashSet<String> = HashSet::new();
        for k in extra_exclude_keys
            .iter()
            .cloned()
            .chain(lib.recent_tracks().take(35).map(song_key))
            .chain(lib.liked.iter().take(60).map(song_key))
        {
            if seen_known.insert(k.clone()) {
                known_track_keys.push(k);
                if known_track_keys.len() >= 110 {
                    break;
                }
            }
        }

        let disliked_track_keys: Vec<String> = self
            .disliked_tracks
            .iter()
            .chain(
                self.track_penalties
                    .iter()
                    .filter(|(_, p)| **p >= 0.65)
                    .map(|(k, _)| k),
            )
            .take(80)
            .cloned()
            .collect();

        TasteRecommendRequest {
            purpose,
            positive_artists,
            negative_artists,
            seed_tracks,
            familiar_tracks,
            known_track_keys,
            disliked_track_keys,
            genre_scores,
            mood_scores,
            era_scores,
            avoided_tags,
            version_scores,
            session_pairs,
            session: TasteSessionContext {
                recent_events,
                query_genres,
                query_moods,
                query_eras,
                recent_skip_streak,
                hour,
                is_weekend,
            },
            variety_preference: self.variety_score,
            date_key,
            hide_explicit,
            region: self.region.clone(),
            limit,
            contribute: true,
        }
    }

    /// The `n` artists with the most positive weight (normalised names).
    pub fn top_artists(&self, n: usize) -> Vec<String> {
        let mut v: Vec<(&String, &f32)> = self.artists.iter().collect();
        v.sort_by(|a, b| {
            b.1.partial_cmp(a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(b.0))
        });
        v.into_iter().take(n).map(|(a, _)| a.clone()).collect()
    }

    /// Anonymized top positive artist signals (with short-term momentum) for server engine.
    pub fn top_positive_signals(&self, limit: usize) -> Vec<TasteArtistSignal> {
        let mut v: Vec<(&String, &f32)> = self.artists.iter().collect();
        v.sort_by(|a, b| b.1.total_cmp(a.1).then_with(|| a.0.cmp(b.0)));
        v.into_iter()
            .take(limit)
            .map(|(k, w)| TasteArtistSignal {
                artist: self.artist_display.get(k).cloned().unwrap_or_else(|| k.clone()),
                weight: *w,
                recent_weight: self.artist_recent.get(k).copied().filter(|r| *r > 0.05),
            })
            .collect()
    }

    /// Normalized display names of artists to suppress in collaborative recommendations.
    pub fn negative_artists(&self, limit: usize) -> Vec<String> {
        let mut v: Vec<(&String, &f32)> = self
            .artist_affinities
            .iter()
            .filter(|(_, w)| **w <= -0.25)
            .collect();
        v.sort_by(|a, b| a.1.total_cmp(b.1).then_with(|| a.0.cmp(b.0)));
        v.into_iter()
            .take(limit)
            .map(|(k, _)| self.artist_display.get(k).cloned().unwrap_or_else(|| k.clone()))
            .collect()
    }

    /// Genre/mood tags the user avoids, for collaborative suppression.
    pub fn avoided_tags(&self, limit: usize) -> Vec<String> {
        let mut v: Vec<(&String, &f32)> = self
            .genre_scores
            .iter()
            .chain(self.mood_scores.iter())
            .filter(|(_, s)| **s <= -0.25)
            .collect();
        v.sort_by(|a, b| a.1.total_cmp(b.1).then_with(|| a.0.cmp(b.0)));
        v.into_iter().take(limit).map(|(k, _)| k.clone()).collect()
    }

    /// Returns top user artists whose cached tags are missing or stale (> 14 days).
    pub fn missing_tag_artists(&self, lib: &Library, now_s: u64, limit: usize) -> Vec<String> {
        let mut candidates: Vec<(&String, f32)> = self
            .artist_affinities
            .iter()
            .map(|(k, v)| (k, v.abs()))
            .collect();
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let mut out = Vec::new();
        for (norm, _) in candidates {
            let fresh = lib
                .taste
                .artist_tags
                .get(norm)
                .is_some_and(|c| {
                    let ttl = if c.genres.is_empty() && c.similar.is_empty() {
                        3600
                    } else {
                        TAG_CACHE_TTL_S
                    };
                    now_s.saturating_sub(c.fetched_at_s) < ttl
                });
            if !fresh {
                let disp = self
                    .artist_display
                    .get(norm)
                    .cloned()
                    .unwrap_or_else(|| norm.clone());
                out.push(disp);
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.artists.is_empty()
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
// ---------------------------------------------------------------- Glyph artist graph

/// Most links kept per artist.
const MAX_LINKS: usize = 24;
/// Largest playlist (distinct artists) read for playlist neighbours.
const MAX_PLAYLIST_ARTISTS: usize = 60;

/// Combines two link strengths like independent evidence (noisy-or), so
/// several weak hints add up but never pass `1.0`.
fn noisy_or(a: f32, b: f32) -> f32 {
    1.0 - (1.0 - a.clamp(0.0, 1.0)) * (1.0 - b.clamp(0.0, 1.0))
}

fn link(links: &mut HashMap<String, HashMap<String, f32>>, a: &str, b: &str, w: f32) {
    if a.is_empty() || b.is_empty() || a == b || w <= 0.0 {
        return;
    }
    let e = links.entry(a.to_string()).or_default().entry(b.to_string()).or_insert(0.0);
    *e = noisy_or(*e, w);
}

/// Glyph's own artist graph, built only from what Glyph already knows (no
/// extra request): cached similar artists, songs the listener played one
/// after another to the end, artists sharing the listener's playlists, and
/// the collaborative cache ("because of" seeds).
pub fn build_artist_links(
    lib: &Library,
    artist_meta: &HashMap<String, (Vec<String>, Vec<String>, Vec<String>, Vec<String>)>,
    now_s: u64,
) -> HashMap<String, Vec<(String, f32)>> {
    let mut links: HashMap<String, HashMap<String, f32>> = HashMap::new();
    // 1. Cached similar artists (ranked lists): strong but one-way, mirrored softly.
    for (a, (_, _, _, similar)) in artist_meta {
        for (i, s) in similar.iter().take(MAX_LINKS).enumerate() {
            let b = normalize_artist(s);
            let w = (0.9 - i as f32 * 0.035).max(0.25);
            link(&mut links, a, &b, w);
            link(&mut links, &b, a, w * 0.6);
        }
    }
    // 2. Co-listened pairs: both songs ran to the end one after another.
    for (x, y, ts) in &lib.taste.session_pairs {
        let decay = 0.5f64.powf(now_s.saturating_sub(*ts) as f64 / SHORT_DECAY_HALF_LIFE_S) as f32;
        let w = 0.35 + 0.5 * decay.clamp(0.0, 1.0);
        let (kx, ky) = (normalize_artist(x), normalize_artist(y));
        link(&mut links, &kx, &ky, w);
        link(&mut links, &ky, &kx, w);
    }
    // 3. Playlist neighbours: artists the listener put side by side.
    for p in &lib.playlists {
        let mut set: Vec<String> = Vec::new();
        for t in &p.tracks {
            let k = normalize_artist(&primary_artist(&t.artist));
            if !k.is_empty() && !set.contains(&k) {
                set.push(k);
                if set.len() >= MAX_PLAYLIST_ARTISTS {
                    break;
                }
            }
        }
        if set.len() < 2 {
            continue;
        }
        // Small, focused playlists say more than huge mixed ones.
        let w = (0.9 / (set.len() as f32).sqrt()).clamp(0.08, 0.55);
        for (i, a) in set.iter().enumerate() {
            for b in set.iter().skip(i + 1) {
                link(&mut links, a, b, w);
                link(&mut links, b, a, w);
            }
        }
    }
    // 4. Collaborative cache: "people who like X also like Y".
    if let Some(c) = &lib.taste.collab_cache {
        for rec in &c.recommendations {
            let b = normalize_artist(&rec.artist);
            for src in &rec.because_of {
                let a = normalize_artist(src);
                let w = (0.45 + rec.score.clamp(0.0, 1.0) * 0.4).min(0.85);
                link(&mut links, &a, &b, w);
                link(&mut links, &b, &a, w * 0.5);
            }
        }
    }
    links
        .into_iter()
        .map(|(a, m)| {
            let mut v: Vec<(String, f32)> = m.into_iter().collect();
            v.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
            v.truncate(MAX_LINKS);
            (a, v)
        })
        .collect()
}

/// How close every artist is to what the listener loves, one hop through
/// [`build_artist_links`]. Hidden or disliked artists stay out.
fn build_artist_graph(
    lib: &Library,
    artists: &HashMap<String, f32>,
    affinities: &HashMap<String, f32>,
    hidden: &HashSet<String>,
    links: &HashMap<String, Vec<(String, f32)>>,
) -> HashMap<String, f32> {
    let mut g: HashMap<String, f32> = HashMap::new();
    for (a, &w) in artists {
        if w < 0.05 {
            continue;
        }
        let Some(ls) = links.get(a) else { continue };
        for (b, s) in ls {
            let e = g.entry(b.clone()).or_insert(0.0);
            *e = noisy_or(*e, w * s);
        }
    }
    if let Some(c) = &lib.taste.collab_cache {
        for rec in &c.recommendations {
            let b = normalize_artist(&rec.artist);
            let e = g.entry(b).or_insert(0.0);
            *e = noisy_or(*e, rec.score.clamp(0.0, 1.0) * 0.7);
        }
    }
    g.retain(|k, v| !hidden.contains(k) && affinities.get(k).is_none_or(|a| *a > -0.15) && *v > 0.02);
    g
}

/// Levenshtein distance, giving up (returns `max + 1`) once it passes `max`.
fn edit_distance(a: &str, b: &str, max: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            row_min = row_min.min(cur[j]);
        }
        if row_min > max {
            return max + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The same word, or a small typo of it ("beleiver" ~ "believer"). Short
/// words must match exactly so "the" never turns into "she".
fn word_close(typed: &str, real: &str) -> bool {
    if typed == real {
        return true;
    }
    let n = typed.chars().count().min(real.chars().count());
    let max = match n {
        0..=3 => return false,
        4..=7 => 1,
        _ => 2,
    };
    edit_distance(typed, real, max) <= max
}

fn infer_local_intent(
    events: &[SessionEvent],
    query_genres: &[String],
    query_moods: &[String],
    artist_meta: &HashMap<String, (Vec<String>, Vec<String>, Vec<String>, Vec<String>)>,
    genre_scores: &HashMap<String, f32>,
    mood_scores: &HashMap<String, f32>,
    variety_score: f32,
    hour: u8,
    is_weekend: bool,
    now_s: u64,
) -> (
    InferredIntent,
    HashMap<String, f32>,
    HashMap<String, f32>,
    HashSet<String>,
    HashSet<String>,
    HashSet<String>,
) {
    let mut sess_genres: HashMap<String, f32> = HashMap::new();
    let mut sess_moods: HashMap<String, f32> = HashMap::new();
    let mut repelled_artists: HashSet<String> = HashSet::new();
    let mut repelled_genres: HashSet<String> = HashSet::new();
    let mut repelled_moods: HashSet<String> = HashSet::new();
    let mut recent_pos_events: Vec<(String, String)> = Vec::new();
    let mut early_skips_recent = 0u32;
    let mut completions_recent = 0u32;
    let mut replays_or_likes = 0u32;
    let mut library_events = 0u32;
    let mut discovery_events = 0u32;
    let mut total_recent = 0u32;

    let active_events: Vec<&SessionEvent> = events
        .iter()
        .filter(|e| now_s.saturating_sub(e.timestamp_s) <= SESSION_WINDOW_S)
        .take(16)
        .collect();

    let recent_skip_streak = active_events
        .iter()
        .take_while(|e| matches!(e.outcome, SessionEventOutcome::SkippedEarly | SessionEventOutcome::SkippedMid))
        .count();

    for (idx, ev) in active_events.iter().enumerate() {
        let age_mins = (now_s.saturating_sub(ev.timestamp_s) / 60) as f32;
        let decay = 0.5f32.powf(age_mins / 45.0).clamp(0.15, 1.0);
        let ak = normalize_artist(&primary_artist(&ev.artist));
        let tk = normalize_title(&ev.title);
        let is_lib = ev.context == SessionEventContext::Library;
        let is_disc = matches!(ev.context, SessionEventContext::Radio | SessionEventContext::Search);
        if age_mins <= 90.0 {
            total_recent += 1;
            if is_lib { library_events += 1; }
            if is_disc { discovery_events += 1; }
        }
        let delta = match ev.outcome {
            SessionEventOutcome::Replayed => {
                replays_or_likes += 1;
                completions_recent += 1;
                if !ak.is_empty() { recent_pos_events.push((ak.clone(), tk)); }
                2.4 * decay
            }
            SessionEventOutcome::Liked => {
                replays_or_likes += 1;
                if !ak.is_empty() { recent_pos_events.push((ak.clone(), tk)); }
                2.2 * decay
            }
            SessionEventOutcome::Queued => {
                if !ak.is_empty() { recent_pos_events.push((ak.clone(), tk)); }
                1.7 * decay
            }
            SessionEventOutcome::Completed => {
                completions_recent += 1;
                if !ak.is_empty() { recent_pos_events.push((ak.clone(), tk)); }
                1.4 * decay
            }
            SessionEventOutcome::Played => {
                if !ak.is_empty() { recent_pos_events.push((ak.clone(), tk)); }
                0.85 * decay
            }
            SessionEventOutcome::SkippedMid => {
                if age_mins <= 30.0 && !ak.is_empty() { repelled_artists.insert(ak.clone()); }
                if is_lib { -0.25 * decay } else { -0.55 * decay }
            }
            SessionEventOutcome::SkippedEarly => {
                if idx < 4 && age_mins <= 20.0 { early_skips_recent += 1; }
                if age_mins <= 35.0 && !ak.is_empty() { repelled_artists.insert(ak.clone()); }
                if is_lib { -0.55 * decay } else { -1.45 * decay }
            }
            SessionEventOutcome::Unliked => -1.1 * decay,
        };

        if let Some((genres, moods, _, _)) = artist_meta.get(&ak) {
            for (i, g) in genres.iter().enumerate() {
                let w = 1.0 / (1.0 + i as f32 * 0.35);
                *sess_genres.entry(g.clone()).or_default() += delta * w;
                if matches!(ev.outcome, SessionEventOutcome::SkippedEarly) && age_mins <= 25.0 && i < 2 {
                    repelled_genres.insert(g.clone());
                }
            }
            for (i, m) in moods.iter().enumerate() {
                let w = 1.0 / (1.0 + i as f32 * 0.35);
                *sess_moods.entry(m.clone()).or_default() += delta * w;
                if matches!(ev.outcome, SessionEventOutcome::SkippedEarly) && age_mins <= 25.0 && i < 2 {
                    repelled_moods.insert(m.clone());
                }
            }
        }
    }

    for g in query_genres {
        *sess_genres.entry(g.clone()).or_default() += 1.6;
        repelled_genres.remove(g);
    }
    for m in query_moods {
        *sess_moods.entry(m.clone()).or_default() += 1.6;
        repelled_moods.remove(m);
    }
    repelled_genres.retain(|g| sess_genres.get(g).copied().unwrap_or(0.0) <= 0.1);
    repelled_moods.retain(|m| sess_moods.get(m).copied().unwrap_or(0.0) <= 0.1);

    // ArtistDive requires >= 3 positive plays of an artist spanning >= 2 distinct tracks
    // (looping a single favorite song twice is Comfort, not a discography dive).
    let last5: Vec<&(String, String)> = recent_pos_events.iter().take(5).collect();
    let mut artist_counts: HashMap<&String, usize> = HashMap::new();
    let mut artist_titles: HashMap<&String, HashSet<&String>> = HashMap::new();
    for (a, tk) in &last5 {
        *artist_counts.entry(a).or_default() += 1;
        artist_titles.entry(a).or_default().insert(tk);
    }
    let is_artist_dive = last5.len() >= 3
        && artist_counts
            .iter()
            .any(|(a, &c)| c >= 3 && artist_titles.get(a).is_some_and(|s| s.len() >= 2));
    let vibe_pivot = recent_skip_streak >= 2 || early_skips_recent >= 2;

    let bucket = TimeBucket::from_hour(hour);
    let prior_moods = bucket.aligned_moods();
    let mut e_sum = 0.0f32;
    let mut e_w = 0.0f32;
    for (m, &v) in &sess_moods {
        if v > 0.1 {
            e_sum += mood_energy_valence(m).0 * v;
            e_w += v;
        }
    }
    let prior_e = prior_moods.iter().map(|m| mood_energy_valence(m).0).sum::<f32>()
        / (prior_moods.len().max(1) as f32)
        + if is_weekend && (hour >= 18 || hour <= 2) { 0.08 } else { 0.0 };
    let mut energy = if e_w > 0.0 {
        (e_sum / e_w) * 0.78 + prior_e * 0.22
    } else {
        prior_e
    };
    if vibe_pivot && !repelled_moods.is_empty() {
        let rep_e = repelled_moods.iter().map(|m| mood_energy_valence(m).0).sum::<f32>()
            / repelled_moods.len() as f32;
        energy = if rep_e >= 0.55 {
            (energy * 0.4 + 0.30).clamp(0.18, 0.55)
        } else {
            (energy * 0.4 + 0.68).clamp(0.50, 0.85)
        };
    }

    let focus_cues = ["Focus"];
    let focus_genres_set = ["Lo-Fi", "Ambient", "Classical", "Instrumental", "Soundtrack"];
    let has_focus = sess_moods.iter().any(|(m, &v)| v > 0.6 && focus_cues.contains(&m.as_str()))
        || sess_genres.iter().any(|(g, &v)| v > 0.6 && focus_genres_set.contains(&g.as_str()));
    let distinct_recent = recent_pos_events.iter().take(6).map(|(a, _)| a).collect::<HashSet<_>>().len();

    let mode = if vibe_pivot {
        ListeningMode::VibeReset
    } else if is_artist_dive && early_skips_recent == 0 {
        ListeningMode::ArtistDive
    } else if replays_or_likes >= 2 {
        ListeningMode::Comfort
    } else if has_focus && completions_recent >= 2 && early_skips_recent == 0 {
        ListeningMode::DeepFocus
    } else if total_recent >= 3 && library_events * 10 >= total_recent * 7 && early_skips_recent == 0 {
        ListeningMode::Comfort
    } else if (total_recent >= 2 && discovery_events * 2 >= total_recent)
        || (distinct_recent >= 4 && variety_score >= 0.55)
    {
        ListeningMode::Discovery
    } else if energy >= 0.72 && e_w >= 1.0 {
        ListeningMode::HighEnergy
    } else if (hour >= 22 || hour <= 4) && energy <= 0.46 {
        ListeningMode::LateNightWindDown
    } else {
        ListeningMode::Balanced
    };

    let base_nov = variety_score.clamp(0.20, 0.85);
    let novelty = match mode {
        ListeningMode::Discovery => (base_nov + 0.25).clamp(0.55, 0.95),
        ListeningMode::Comfort => (base_nov - 0.28).clamp(0.10, 0.45),
        ListeningMode::DeepFocus => (base_nov - 0.15).clamp(0.20, 0.55),
        ListeningMode::ArtistDive => (base_nov - 0.10).clamp(0.25, 0.60),
        ListeningMode::VibeReset => 0.62,
        _ => base_nov,
    };

    let mut focus_g: Vec<(String, f32)> = sess_genres
        .iter()
        .filter(|(g, v)| **v > 0.2 && !repelled_genres.contains(*g))
        .map(|(g, v)| (g.clone(), *v))
        .collect();
    if focus_g.is_empty() || vibe_pivot {
        for (g, &s) in genre_scores {
            if s > 0.25 && !repelled_genres.contains(g) && !focus_g.iter().any(|(x, _)| x == g) {
                focus_g.push((g.clone(), s * 0.8));
            }
        }
    }
    focus_g.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut focus_m: Vec<(String, f32)> = sess_moods
        .iter()
        .filter(|(m, v)| **v > 0.2 && !repelled_moods.contains(*m))
        .map(|(m, v)| (m.clone(), *v))
        .collect();
    if focus_m.is_empty() || vibe_pivot {
        for (m, &s) in mood_scores {
            if s > 0.25 && !repelled_moods.contains(m) && !focus_m.iter().any(|(x, _)| x == m) {
                focus_m.push((m.clone(), s * 0.8));
            }
        }
    }
    if focus_m.is_empty() {
        for &m in prior_moods.iter().take(2) {
            focus_m.push((m.to_string(), 0.5));
        }
    }
    focus_m.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let intent = InferredIntent {
        mode,
        focus_genres: focus_g.into_iter().take(3).map(|(g, _)| g).collect(),
        focus_moods: focus_m.into_iter().take(3).map(|(m, _)| m).collect(),
        energy: energy.clamp(0.0, 1.0),
        novelty,
        vibe_pivot,
    };

    (intent, sess_genres, sess_moods, repelled_artists, repelled_genres, repelled_moods)
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), ch.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ------------------------------------------------ visible Taste Profile DTOs

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightedTag {
    pub name: String,
    /// `0.0..=1.0` magnitude.
    pub score: f32,
    /// Share percentage `0..=100`.
    pub share_pct: u8,
    /// Explicit user feedback: `-1`, `0`, or `+1`.
    pub feedback: i8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionInsight {
    pub style: String,
    pub score: f32,
    pub plays: u32,
    pub skips: u32,
    pub feedback: i8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListeningHabits {
    /// Shares `0..=100` for `[Morning, Afternoon, Evening, Night]`.
    pub time_of_day: [u8; 4],
    pub peak_period: TimeBucket,
    /// Completion ratio `0.0..=1.0`.
    pub completion_rate: f32,
    /// Early skip rate `0.0..=1.0`.
    pub early_skip_rate: f32,
    /// Artist diversity / Shannon entropy `0.0..=1.0`.
    pub variety_score: f32,
    pub total_plays: u32,
    pub total_skips: u32,
    #[serde(default)]
    pub total_replays: u32,
    pub versions: Vec<VersionInsight>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistAffinityInsight {
    pub name: String,
    pub score: f32,
    pub reasons: Vec<String>,
    pub genres: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistDislikeInsight {
    pub name: String,
    /// `"hidden"`, `"blocked"`, `"less"`, or `"high_skips"`.
    pub kind: String,
    pub skips: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TasteProfile {
    /// `"server"` (default server-powered Taste Engine), `"local"` (opted out / offline), or `"off"`.
    pub mode: String,
    /// `0.0..=1.0` profile maturity.
    pub confidence: f32,
    pub signals_count: u32,
    pub onboarding_done: bool,
    pub top_genres: Vec<WeightedTag>,
    pub avoided_genres: Vec<WeightedTag>,
    pub moods: Vec<WeightedTag>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_eras: Vec<WeightedTag>,
    #[serde(default)]
    pub intent: InferredIntent,
    pub habits: ListeningHabits,
    pub liked_artists: Vec<ArtistAffinityInsight>,
    pub disliked_artists: Vec<ArtistDislikeInsight>,
    pub disliked_tracks: Vec<Track>,
    pub onboarding_artists: Vec<String>,
    pub onboarding_tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub onboarding_genres: Vec<String>,
    pub collab_recs: Vec<CollabArtistRec>,
    pub collab_contributors: u32,
    pub collab_model_ready: bool,
}

/// Builds the complete visible Taste Profile snapshot for the UI.
pub fn build_profile(
    lib: &Library,
    cfg: &TasteConfig,
    server_available: bool,
    now_s: u64,
    current_hour: u8,
) -> TasteProfile {
    let taste = Taste::from_library_context(lib, now_s, current_hour, local_is_weekend());
    let mode = if !cfg.enabled {
        "off"
    } else if cfg.server_collab && server_available {
        "server"
    } else {
        "local"
    }
    .to_string();

    // Genres (positive & avoided).
    let mut pos_g: Vec<(String, f32)> = taste
        .genre_scores
        .iter()
        .filter(|(_, s)| **s > 0.05)
        .map(|(k, s)| (k.clone(), *s))
        .collect();
    pos_g.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let sum_g: f32 = pos_g.iter().take(10).map(|(_, s)| *s).sum::<f32>().max(0.001);
    let top_genres: Vec<WeightedTag> = pos_g
        .into_iter()
        .take(10)
        .map(|(name, score)| {
            let fb = lib.taste.tag_feedback.get(&name.to_lowercase()).copied().unwrap_or(0);
            WeightedTag {
                name,
                score,
                share_pct: ((score / sum_g) * 100.0).round().clamp(1.0, 100.0) as u8,
                feedback: fb,
            }
        })
        .collect();

    let mut neg_g: Vec<(String, f32)> = taste
        .genre_scores
        .iter()
        .filter(|(_, s)| **s <= -0.15)
        .map(|(k, s)| (k.clone(), s.abs()))
        .collect();
    neg_g.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let avoided_genres: Vec<WeightedTag> = neg_g
        .into_iter()
        .take(8)
        .map(|(name, score)| {
            let fb = lib.taste.tag_feedback.get(&name.to_lowercase()).copied().unwrap_or(0);
            WeightedTag {
                name,
                score,
                share_pct: (score * 100.0).round().clamp(1.0, 100.0) as u8,
                feedback: fb,
            }
        })
        .collect();

    // Moods.
    let mut pos_m: Vec<(String, f32)> = taste
        .mood_scores
        .iter()
        .filter(|(_, s)| **s > 0.05)
        .map(|(k, s)| (k.clone(), *s))
        .collect();
    pos_m.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let sum_m: f32 = pos_m.iter().take(8).map(|(_, s)| *s).sum::<f32>().max(0.001);
    let moods: Vec<WeightedTag> = pos_m
        .into_iter()
        .take(8)
        .map(|(name, score)| {
            let fb = lib.taste.tag_feedback.get(&name.to_lowercase()).copied().unwrap_or(0);
            WeightedTag {
                name,
                score,
                share_pct: ((score / sum_m) * 100.0).round().clamp(1.0, 100.0) as u8,
                feedback: fb,
            }
        })
        .collect();

    // Eras.
    let mut pos_e: Vec<(String, f32)> = taste
        .era_scores
        .iter()
        .filter(|(_, s)| **s > 0.08)
        .map(|(k, s)| (k.clone(), *s))
        .collect();
    pos_e.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let sum_e: f32 = pos_e.iter().take(5).map(|(_, s)| *s).sum::<f32>().max(0.001);
    let top_eras: Vec<WeightedTag> = pos_e
        .into_iter()
        .take(5)
        .map(|(name, score)| WeightedTag {
            name,
            score,
            share_pct: ((score / sum_e) * 100.0).round().clamp(1.0, 100.0) as u8,
            feedback: 0,
        })
        .collect();

    // Listening habits.
    let mut buckets = [0.0f32; 4];
    let mut total_plays: u32 = 0;
    let mut total_completions: u32 = 0;
    let mut total_replays: u32 = 0;
    let mut total_early_skips: u32 = 0;
    let mut total_skips: u32 = 0;
    for sig in lib.taste.artist_signals.values() {
        for i in 0..4 {
            buckets[i] += sig.time_buckets[i];
        }
        total_plays = total_plays.saturating_add(sig.plays);
        total_completions = total_completions.saturating_add(sig.completions);
        total_replays = total_replays.saturating_add(sig.replays);
        total_early_skips = total_early_skips.saturating_add(sig.instant_skips + sig.early_skips);
        total_skips = total_skips.saturating_add(sig.total_skips());
    }
    if buckets.iter().sum::<f32>() <= 0.0 && !lib.stats.is_empty() {
        let summary = lib.stats.summary(crate::stats::Range::All, crate::stats::When::now());
        for (h, ms) in summary.hours.iter().enumerate() {
            buckets[TimeBucket::from_hour(h as u8).index()] += *ms as f32 / 60_000.0;
        }
    }
    if total_plays == 0 {
        total_plays = lib.artist_plays.values().copied().sum();
    }
    if total_skips == 0 {
        total_skips = lib.artist_skips.values().copied().sum();
        total_early_skips = total_skips;
    }

    let bucket_sum: f32 = buckets.iter().sum();
    let time_of_day = if bucket_sum > 0.0 {
        [
            ((buckets[0] / bucket_sum) * 100.0).round() as u8,
            ((buckets[1] / bucket_sum) * 100.0).round() as u8,
            ((buckets[2] / bucket_sum) * 100.0).round() as u8,
            ((buckets[3] / bucket_sum) * 100.0).round() as u8,
        ]
    } else {
        [0, 0, 0, 0]
    };
    let peak_period = if bucket_sum > 0.0 {
        let best_i = buckets
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap_or(0);
        TimeBucket::from_index(best_i)
    } else {
        TimeBucket::from_hour(current_hour)
    };

    let outcomes = (total_plays + total_skips) as f32;
    let completion_rate = if outcomes > 0.0 {
        let completed_est = if total_completions > 0 {
            total_completions as f32
        } else {
            total_plays as f32 * 0.85
        };
        (completed_est / outcomes).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let early_skip_rate = if outcomes > 0.0 {
        (total_early_skips as f32 / outcomes).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let versions: Vec<VersionInsight> = VERSION_STYLES
        .iter()
        .map(|&style| {
            let sig = lib.taste.version_signals.get(style);
            VersionInsight {
                style: style.to_string(),
                score: taste.version_scores.get(style).copied().unwrap_or(0.0),
                plays: sig.map_or(0, |s| s.plays),
                skips: sig.map_or(0, |s| s.skips),
                feedback: lib.taste.version_feedback.get(style).copied().unwrap_or(0),
            }
        })
        .collect();

    // Likes ("Mag ich").
    let mut liked_a: Vec<(&String, &f32)> = taste.artists.iter().collect();
    liked_a.sort_by(|a, b| b.1.total_cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let liked_artists: Vec<ArtistAffinityInsight> = liked_a
        .into_iter()
        .take(12)
        .map(|(norm, score)| ArtistAffinityInsight {
            name: taste
                .artist_display
                .get(norm)
                .cloned()
                .unwrap_or_else(|| title_case(norm)),
            score: *score,
            reasons: taste.artist_reasons.get(norm).cloned().unwrap_or_default(),
            genres: taste
                .artist_meta
                .get(norm)
                .map(|(g, _, _, _)| g.iter().take(2).cloned().collect())
                .unwrap_or_default(),
        })
        .collect();

    // Dislikes ("Mag ich nicht").
    let mut disliked_artists: Vec<ArtistDislikeInsight> = Vec::new();
    let mut seen_dis = HashSet::new();
    for h in &lib.hidden_artists {
        let k = normalize_artist(h);
        if !k.is_empty() && seen_dis.insert(k.clone()) {
            let skips = lib.taste.artist_signals.get(&k).map_or(0, |s| s.total_skips());
            disliked_artists.push(ArtistDislikeInsight {
                name: h.clone(),
                kind: "hidden".into(),
                skips,
            });
        }
    }
    for (a, pref) in &lib.recommendation_feedback {
        if *pref < 0 {
            let k = normalize_artist(a);
            if !k.is_empty() && seen_dis.insert(k.clone()) {
                let skips = lib.taste.artist_signals.get(&k).map_or(0, |s| s.total_skips());
                disliked_artists.push(ArtistDislikeInsight {
                    name: taste
                        .artist_display
                        .get(&k)
                        .cloned()
                        .unwrap_or_else(|| title_case(a)),
                    kind: if *pref <= -2 { "blocked" } else { "less" }.into(),
                    skips,
                });
            }
        }
    }
    let mut skipped_a: Vec<(&String, &ArtistSignal)> = lib
        .taste
        .artist_signals
        .iter()
        .filter(|(k, s)| {
            !seen_dis.contains(*k) && s.total_skips() >= 2 && s.abandon_rate() >= 0.45
        })
        .collect();
    skipped_a.sort_by(|a, b| b.1.total_skips().cmp(&a.1.total_skips()));
    for (k, sig) in skipped_a.into_iter().take(8) {
        seen_dis.insert(k.clone());
        disliked_artists.push(ArtistDislikeInsight {
            name: if sig.display.is_empty() {
                title_case(k)
            } else {
                sig.display.clone()
            },
            kind: "high_skips".into(),
            skips: sig.total_skips(),
        });
    }

    let signals_count = (lib.liked.len()
        + lib.followed.len()
        + lib.taste.onboarding_artists.len()
        + lib.taste.onboarding_tracks.len()
        + lib.recommendation_feedback.len()
        + lib.taste.disliked_tracks.len()
        + lib.taste.tag_feedback.len()
        + lib.taste.version_feedback.len()) as u32
        + total_plays
        + total_skips
        + total_replays;
    let confidence = (1.0 - (-(signals_count as f32) / 18.0).exp()).clamp(0.0, 1.0);

    let (collab_recs, collab_contributors, collab_model_ready) = if cfg.enabled && cfg.server_collab {
        match &lib.taste.collab_cache {
            Some(c) => (c.recommendations.clone(), c.contributors, c.model_ready),
            None => (Vec::new(), 0, false),
        }
    } else {
        (Vec::new(), 0, false)
    };

    // Prefer fresh local session intent when session events exist, else use server intent if in server mode.
    let intent = if !lib.taste.session_events.is_empty() || !cfg.server_collab {
        taste.intent.clone()
    } else {
        lib.taste
            .last_server_intent
            .clone()
            .unwrap_or_else(|| taste.intent.clone())
    };

    TasteProfile {
        mode,
        confidence,
        signals_count,
        onboarding_done: lib.taste.onboarding_done,
        top_genres,
        avoided_genres,
        moods,
        top_eras,
        intent,
        habits: ListeningHabits {
            time_of_day,
            peak_period,
            completion_rate,
            early_skip_rate,
            variety_score: taste.variety_score,
            total_plays,
            total_skips,
            total_replays,
            versions,
        },
        liked_artists,
        disliked_artists,
        disliked_tracks: lib.taste.disliked_tracks.values().cloned().collect(),
        onboarding_artists: lib.taste.onboarding_artists.clone(),
        onboarding_tracks: lib.taste.onboarding_tracks.clone(),
        onboarding_genres: lib.taste.onboarding_genres.clone(),
        collab_recs,
        collab_contributors,
        collab_model_ready,
    }
}

// ------------------------------------------------ Daily Mixes generator (local fallback & offline)

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Deterministic daily jitter in `0.0..0.35` so Daily Mixes rotate cleanly every day.
fn daily_jitter(date_key: &str, mix_id: &str, track_k: &str) -> f32 {
    let h = fnv1a(&format!("{date_key}|{mix_id}|{track_k}"));
    ((h % 1000) as f32 / 1000.0) * 0.35
}

fn spread_mix_artists(tracks: Vec<Track>, max_per_artist: usize, limit: usize) -> Vec<Track> {
    let mut by_artist: Vec<(String, VecDeque<Track>)> = Vec::new();
    for t in tracks {
        let a = normalize_artist(&primary_artist(&t.artist));
        if let Some((_, q)) = by_artist.iter_mut().find(|(k, _)| *k == a) {
            if q.len() < max_per_artist {
                q.push_back(t);
            }
        } else {
            let mut q = VecDeque::new();
            q.push_back(t);
            by_artist.push((a, q));
        }
    }
    let mut out: Vec<Track> = Vec::with_capacity(limit);
    let mut last: Option<String> = None;
    while out.len() < limit {
        let pick = by_artist
            .iter()
            .enumerate()
            .filter(|(_, (a, q))| !q.is_empty() && last.as_deref() != Some(a.as_str()))
            .max_by(|(ia, (_, qa)), (ib, (_, qb))| qa.len().cmp(&qb.len()).then_with(|| ib.cmp(ia)))
            .map(|(i, _)| i);
        let Some(i) = pick else { break };
        if let Some(t) = by_artist[i].1.pop_front() {
            last = Some(by_artist[i].0.clone());
            out.push(t);
        }
    }
    out
}

/// Generates personalized Daily Mixes locally (Intent/Session Mix, soft
/// genre-family mixes, Time-of-Day Mix, and Discovery Mix) — used in Local-only
/// opt-out mode or when offline / server unavailable.
pub fn build_daily_mixes(
    lib: &Library,
    taste: &Taste,
    candidates: &[Track],
    date_key: &str,
    current_hour: u8,
    hide_explicit: bool,
) -> Vec<DailyMix> {
    let bucket = TimeBucket::from_hour(current_hour);
    let recent_keys: HashSet<String> = lib.recent_tracks().take(25).map(track_key).collect();

    let mut familiar_all: Vec<Track> = Vec::new();
    let mut familiar_seen: HashSet<String> = HashSet::new();
    let mut add_fam = |t: &Track| {
        if (hide_explicit && t.maybe_explicit())
            || taste.is_hidden(&t.artist)
            || taste.is_disliked_track(t)
        {
            return;
        }
        if familiar_seen.insert(track_key(t)) {
            familiar_all.push(t.clone());
        }
    };
    for t in &lib.liked { add_fam(t); }
    for t in &lib.taste.onboarding_tracks { add_fam(t); }
    for pl in &lib.playlists {
        if pl.shared_by.is_none() {
            for t in &pl.tracks { add_fam(t); }
        }
    }
    for t in lib.recent_tracks() {
        if lib
            .taste
            .track_signals
            .get(&song_key(t))
            .is_some_and(|s| s.plays == 0 && s.total_skips() > 0)
        {
            continue;
        }
        add_fam(t);
    }

    let mut fresh_all: Vec<Track> = Vec::new();
    let mut fresh_seen: HashSet<String> = HashSet::new();
    for t in candidates {
        if (hide_explicit && t.maybe_explicit())
            || taste.is_hidden(&t.artist)
            || taste.is_disliked_track(t)
        {
            continue;
        }
        let k = track_key(t);
        if !familiar_seen.contains(&k) && fresh_seen.insert(k) {
            fresh_all.push(t.clone());
        }
    }
    if familiar_all.is_empty() && fresh_all.is_empty() {
        return Vec::new();
    }

    let mut mixes: Vec<DailyMix> = Vec::new();

    // 0. Session / Intent Mix (when active listening mode is distinctive or pivoting).
    let show_intent_mix = taste.intent.vibe_pivot
        || !matches!(taste.intent.mode, ListeningMode::Balanced)
        || !taste.session_genres.is_empty();
    if show_intent_mix {
        let mix_id = format!("daily-intent-{}", taste.intent.mode.as_str());
        let mut scored: Vec<(Track, f32)> = familiar_all
            .iter()
            .map(|t| (t, true))
            .chain(fresh_all.iter().map(|t| (t, false)))
            .filter(|(t, _)| !credited(&t.artist).iter().any(|a| taste.repelled_artists.contains(a)))
            .map(|(t, fam)| {
                let s = taste.score_recommendation(t, 0.9 + taste.session_intent_affinity(t) * 0.9, bucket, fam)
                    + daily_jitter(date_key, &mix_id, &track_key(t));
                (t.clone(), s)
            })
            .filter(|(_, s)| s.is_finite() && *s > -0.3)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
        let tracks = spread_mix_artists(scored.into_iter().map(|(t, _)| t).collect(), 2, 18);
        if tracks.len() >= 3 {
            let lead_genre = taste.intent.focus_genres.first().cloned();
            let (title, hue) = match taste.intent.mode {
                ListeningMode::VibeReset => ("Fresh Pivot Mix".to_string(), 168u16),
                ListeningMode::DeepFocus => ("Deep Focus Mix".to_string(), 210u16),
                ListeningMode::ArtistDive => ("In the Zone".to_string(), 292u16),
                ListeningMode::Comfort => ("Comfort Mix".to_string(), 28u16),
                ListeningMode::HighEnergy => ("Energy Boost".to_string(), 12u16),
                ListeningMode::LateNightWindDown => ("Wind Down Mix".to_string(), 248u16),
                ListeningMode::Discovery | ListeningMode::Balanced => (
                    lead_genre
                        .as_deref()
                        .map(|g| format!("{g} Session"))
                        .unwrap_or_else(|| "Right Now Mix".to_string()),
                    328u16,
                ),
            };
            let tags: Vec<String> = taste.intent.focus_moods.iter().take(2).cloned().collect();
            let subtitle = mix_artists_subtitle(&tracks, lead_genre.as_deref());
            mixes.push(DailyMix {
                id: mix_id,
                kind: "intent".into(),
                title,
                subtitle,
                tags,
                accent_hue: hue,
                tracks,
            });
        }
    }

    // 1. Genre / Cluster Daily Mixes (up to 3 distinct top genres with soft genre-family matching).
    let mut top_genres: Vec<(String, f32)> = taste
        .genre_scores
        .iter()
        .filter(|(_, s)| **s > 0.12)
        .map(|(g, s)| (g.clone(), *s))
        .collect();
    top_genres.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut used_globally: HashSet<String> = HashSet::new();
    let mut cluster_idx = 0usize;

    for (genre, _) in top_genres.into_iter().take(6) {
        if cluster_idx >= 3 {
            break;
        }
        let mix_id = format!("daily-genre-{}", cluster_idx + 1);
        let genre_match = |t: &Track| -> f32 {
            let (gs, _, _) = taste.track_genres_moods_eras(t);
            gs.iter()
                .map(|g| genre_similarity(g, &genre))
                .fold(0.0f32, f32::max)
        };

        let mut fam_scored: Vec<(Track, f32)> = familiar_all
            .iter()
            .filter_map(|t| {
                if used_globally.contains(&track_key(t)) {
                    return None;
                }
                let gm = genre_match(t);
                if gm < 0.65 {
                    return None;
                }
                let s = taste.score_recommendation(t, 0.7 + gm * 0.6, bucket, true)
                    + daily_jitter(date_key, &mix_id, &track_key(t));
                (s.is_finite() && s > -0.5).then(|| (t.clone(), s))
            })
            .collect();
        fam_scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));

        let mut fresh_scored: Vec<(Track, f32)> = fresh_all
            .iter()
            .filter_map(|t| {
                if used_globally.contains(&track_key(t)) {
                    return None;
                }
                let gm = genre_match(t);
                if gm < 0.65 {
                    return None;
                }
                let s = taste.score_recommendation(t, 0.7 + gm * 0.6, bucket, false)
                    + daily_jitter(date_key, &mix_id, &track_key(t));
                (s.is_finite() && s > -0.5).then(|| (t.clone(), s))
            })
            .collect();
        fresh_scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));

        let mut combined: Vec<Track> = Vec::new();
        let mut fi = fam_scored.into_iter().map(|(t, _)| t);
        let mut ni = fresh_scored.into_iter().map(|(t, _)| t);
        for _ in 0..24 {
            if let Some(t) = fi.next() { combined.push(t); }
            if let Some(t) = ni.next() { combined.push(t); }
        }
        let tracks = spread_mix_artists(combined, 3, 20);
        if tracks.len() < 3 {
            continue;
        }
        for t in &tracks {
            used_globally.insert(track_key(t));
        }
        let subtitle = mix_artists_subtitle(&tracks, Some(&genre));
        let hue = (fnv1a(&genre) % 360) as u16;
        cluster_idx += 1;
        mixes.push(DailyMix {
            id: mix_id,
            kind: "genre".into(),
            title: format!("Daily Mix {cluster_idx}"),
            subtitle,
            tags: vec![genre],
            accent_hue: hue,
            tracks,
        });
    }

    if !mixes.iter().any(|m| m.kind == "genre") {
        let mix_id = "daily-genre-1".to_string();
        let mut all_scored: Vec<(Track, f32)> = familiar_all
            .iter()
            .map(|t| (t.clone(), taste.score_recommendation(t, 1.1, bucket, true) + daily_jitter(date_key, &mix_id, &track_key(t))))
            .chain(fresh_all.iter().map(|t| (t.clone(), taste.score_recommendation(t, 0.9, bucket, false) + daily_jitter(date_key, &mix_id, &track_key(t)))))
            .filter(|(_, s)| s.is_finite() && *s > -0.5)
            .collect();
        all_scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
        let tracks = spread_mix_artists(all_scored.into_iter().map(|(t, _)| t).collect(), 3, 20);
        if tracks.len() >= 2 {
            let subtitle = mix_artists_subtitle(&tracks, None);
            mixes.push(DailyMix {
                id: mix_id,
                kind: "genre".into(),
                title: "Daily Mix 1".into(),
                subtitle,
                tags: vec![],
                accent_hue: 328,
                tracks,
            });
        }
    }

    // 2. Time-of-Day Mix.
    {
        let mix_id = format!("daily-time-{}", bucket.as_str());
        let mut scored: Vec<(Track, f32)> = familiar_all
            .iter()
            .map(|t| {
                let ta = taste.time_affinity(t, bucket);
                let sc = taste.score_recommendation(t, 0.8 + ta * 1.5, bucket, true)
                    + daily_jitter(date_key, &mix_id, &track_key(t));
                (t.clone(), sc)
            })
            .chain(fresh_all.iter().map(|t| {
                let ta = taste.time_affinity(t, bucket);
                let sc = taste.score_recommendation(t, 0.7 + ta * 1.5, bucket, false)
                    + daily_jitter(date_key, &mix_id, &track_key(t));
                (t.clone(), sc)
            }))
            .filter(|(_, s)| s.is_finite() && *s > -0.3)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
        let tracks = spread_mix_artists(scored.into_iter().map(|(t, _)| t).collect(), 2, 18);
        if tracks.len() >= 3 {
            let (title, hue) = match bucket {
                TimeBucket::Morning => ("Morning Mix", 36u16),
                TimeBucket::Afternoon => ("Afternoon Mix", 198u16),
                TimeBucket::Evening => ("Evening Mix", 278u16),
                TimeBucket::Night => ("Late Night Mix", 242u16),
            };
            let tags: Vec<String> = bucket.aligned_moods().iter().take(2).map(|s| (*s).to_string()).collect();
            let subtitle = mix_artists_subtitle(&tracks, None);
            mixes.push(DailyMix {
                id: mix_id,
                kind: "time".into(),
                title: title.into(),
                subtitle,
                tags,
                accent_hue: hue,
                tracks,
            });
        }
    }

    // 3. Discovery Mix.
    {
        let mix_id = "daily-discovery".to_string();
        let collab_boost: HashMap<String, f32> = lib
            .taste
            .collab_cache
            .as_ref()
            .map(|c| {
                c.recommendations
                    .iter()
                    .map(|r| (normalize_artist(&r.artist), r.score.clamp(0.0, 2.0)))
                    .collect()
            })
            .unwrap_or_default();

        let mut scored: Vec<(Track, f32)> = fresh_all
            .iter()
            .filter(|t| !taste.is_liked(t) && !recent_keys.contains(&track_key(t)))
            .map(|t| {
                let cb = credited(&t.artist)
                    .iter()
                    .filter_map(|a| collab_boost.get(a))
                    .cloned()
                    .fold(0.0f32, f32::max);
                let sc = taste.score_recommendation(t, 0.9 + cb * 0.8, bucket, false)
                    + daily_jitter(date_key, &mix_id, &track_key(t));
                (t.clone(), sc)
            })
            .filter(|(_, s)| s.is_finite() && *s > -0.3)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
        let tracks = spread_mix_artists(scored.into_iter().map(|(t, _)| t).collect(), 2, 18);
        if tracks.len() >= 3 {
            let subtitle = mix_artists_subtitle(&tracks, None);
            mixes.push(DailyMix {
                id: mix_id,
                kind: "discovery".into(),
                title: "Discovery Mix".into(),
                subtitle,
                tags: vec!["Fresh".into()],
                accent_hue: 325,
                tracks,
            });
        }
    }

    mixes
}

fn mix_artists_subtitle(tracks: &[Track], lead_tag: Option<&str>) -> String {
    let mut names: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for t in tracks {
        let a = primary_artist(&t.artist);
        let k = normalize_artist(&a);
        if !k.is_empty() && seen.insert(k) {
            names.push(a);
            if names.len() >= 3 {
                break;
            }
        }
    }
    let artists = names.join(", ");
    match (lead_tag, artists.is_empty()) {
        (Some(tag), false) => format!("{tag} · {artists}"),
        (Some(tag), true) => tag.to_string(),
        (None, false) => artists,
        (None, true) => String::new(),
    }
}

// ---------------------------------------------------------------- queries & intent-aware search ranking

/// Splits `query` into the plain words (quotes removed, sent to the sources)
/// and the quoted phrases that must match exactly.
pub fn parse_query(query: &str) -> (String, Vec<String>) {
    let mut plain = String::new();
    let mut phrases = Vec::new();
    let mut in_quote = false;
    let mut cur = String::new();
    for c in query.chars() {
        match c {
            '"' | '\u{201c}' | '\u{201d}' | '\u{201e}' => {
                if in_quote {
                    let p = cur.trim().to_lowercase();
                    if !p.is_empty() {
                        phrases.push(p);
                    }
                    plain.push(' ');
                    plain.push_str(cur.trim());
                    plain.push(' ');
                    cur.clear();
                }
                in_quote = !in_quote;
            }
            _ if in_quote => cur.push(c),
            _ => plain.push(c),
        }
    }
    if in_quote && !cur.trim().is_empty() {
        phrases.push(cur.trim().to_lowercase());
        plain.push(' ');
        plain.push_str(cur.trim());
    }
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    (plain, phrases)
}

fn squash(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every quoted phrase appears in the title, artist or album.
pub fn matches_phrases(t: &Track, phrases: &[String]) -> bool {
    if phrases.is_empty() {
        return true;
    }
    let hay = [
        squash(&t.title),
        squash(&t.artist),
        t.album.as_deref().map(squash).unwrap_or_default(),
    ];
    phrases.iter().all(|p| {
        let p = squash(p);
        hay.iter().any(|h| h.contains(&p))
    })
}

fn words(s: &str) -> Vec<String> {
    crate::dedupe::squash(s)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

const JOINERS: [&str; 8] = ["by", "von", "feat", "ft", "featuring", "x", "and", "und"];

fn title_artist_match(t: &Track, q: &str) -> f32 {
    let qw = words(q);
    if qw.len() < 2 {
        return 0.0;
    }
    // Typo tolerant: "imagine dragns beleiver" still names the song.
    let in_query = |w: &String| qw.iter().any(|q| word_close(q, w));
    let all_in_query = |ws: &[String]| !ws.is_empty() && ws.iter().all(in_query);
    let core_title = words(&normalize_title(&t.title));
    let full_title = words(&t.title);
    let credits: Vec<String> = credited(&t.artist).iter().flat_map(|a| words(a)).collect();
    let halves: Vec<&str> = t.title.split(" - ").collect();
    let artist_typed = credited(&t.artist).iter().any(|a| all_in_query(&words(a)))
        || (halves.len() == 2 && halves.iter().all(|h| all_in_query(&words(&normalize_title(h)))));
    let mut s = 0.0;
    if all_in_query(&core_title) && artist_typed {
        s += 4.0;
    }
    let album = t.album.as_deref().map(words).unwrap_or_default();
    let missing = qw
        .iter()
        .filter(|w| {
            let found = |ws: &[String]| ws.iter().any(|x| word_close(w, x));
            !JOINERS.contains(&w.as_str()) && !found(&full_title) && !found(&credits) && !found(&album)
        })
        .count();
    s - (missing as f32 * 0.6).min(2.4)
}

/// Rank of `t` for `query` (higher first): text match, search genre/mood/era
/// intent understanding, adaptive version/style preference, artist/genre taste,
/// replay love, negative skip/dislike signals, region, and metadata completeness.
pub fn score(t: &Track, query: &str, taste: &Taste) -> f32 {
    let q = squash(query);
    let title = squash(&t.title);
    let artist = squash(&t.artist);
    let title = match title
        .strip_prefix(&format!("{artist} - "))
        .or_else(|| title.strip_prefix(&format!("{artist} – ")))
    {
        Some(rest) if !artist.is_empty() && !rest.is_empty() => rest.to_string(),
        _ => title,
    };
    let mut s = 0.0;
    if !q.is_empty() {
        if title == q {
            s += 3.0;
        } else if title.starts_with(&q) {
            s += 2.0;
        } else if title.contains(&q) {
            s += 1.2;
        }
        if artist == q {
            s += 2.5;
        } else if !artist.is_empty() && (artist.contains(&q) || q.contains(&artist)) {
            s += 1.0;
        }
        s += title_artist_match(t, &q);

        // One mistyped word ("beleiver", "metalica"): nothing matched as
        // written, but a close word in the title or the artist did.
        let exact_hit = title.contains(&q) || (!artist.is_empty() && (artist.contains(&q) || q.contains(&artist)));
        let qw = words(&q);
        if !exact_hit && qw.len() == 1 {
            let w = &qw[0];
            if words(&title).iter().any(|x| word_close(w, x)) {
                s += 1.5;
            } else if words(&t.artist).iter().any(|x| word_close(w, x)) {
                s += 1.3;
            }
        }

        // Search query musical intent (e.g. "chill late night", "90s hip hop", "traurig indie"):
        // boost tracks whose artist genres/moods/eras align with the queried vibe!
        let (qg, qm, qe) = extract_query_intent(&q);
        if !qg.is_empty() || !qm.is_empty() || !qe.is_empty() {
            let (tg, tm, te) = taste.track_genres_moods_eras(t);
            let mut intent_hit = 0.0f32;
            for want in &qg {
                let best = tg.iter().map(|g| genre_similarity(g, want)).fold(0.0f32, f32::max);
                if best >= 0.70 {
                    intent_hit += best * 1.1;
                }
            }
            for want in &qm {
                let best = tm.iter().map(|m| mood_similarity(m, want)).fold(0.0f32, f32::max);
                if best >= 0.70 {
                    intent_hit += best * 0.75;
                }
            }
            for want in &qe {
                if te.iter().any(|e| e == want) {
                    intent_hit += 0.65;
                }
            }
            s += intent_hit.min(2.2);
        }
    }

    // Adaptive version/style scoring.
    let q_versions = detect_versions(query, None);
    for style in detect_versions(&t.title, t.album.as_deref()) {
        if q_versions.contains(&style) {
            s += 0.8;
            continue;
        }
        let pref = taste
            .version_scores
            .get(style)
            .copied()
            .unwrap_or_else(|| if DEFAULT_ODD_VERSIONS.contains(&style) { -0.55 } else { 0.0 });
        if pref < 0.0 {
            s += (pref * 2.7).max(-2.2);
        } else if pref > 0.15 {
            s += (pref * 0.7).min(0.6);
        }
    }

    // Signed artist taste + soft genre alignment + replay/like love + negative song signals.
    let aff = taste.artist_affinity(&t.artist);
    if aff >= 0.0 {
        s += aff * 2.0;
    } else {
        s += aff * 1.6;
    }
    s += taste.track_tag_affinity(t) * 0.50;
    s += taste.track_love_score(t) * 0.65;
    if taste.is_liked(t) {
        s += 1.0;
    }
    // Picked from search before: people search the same songs again.
    if taste.is_search_pick(t) {
        s += 0.8;
    }
    // New artists close to the listener's favourites rank above strangers.
    if aff >= 0.0 {
        s += taste.graph_affinity(&t.artist) * 0.6;
    }
    if taste.is_disliked_track(t) {
        s -= 1.8;
    } else {
        s -= taste.track_skip_penalty(t) * 0.9;
    }

    // Proper release (ISRC) + user region lift; never a filter.
    if let Some(isrc) = t.isrc.as_deref().filter(|i| !i.trim().is_empty()) {
        s += 0.3;
        if taste.region.is_some() && isrc_country(isrc) == taste.region {
            s += 0.5;
        }
    }
    // Completeness check.
    if t.cover_url.as_deref().is_none_or(|c| c.trim().is_empty()) {
        s -= 0.6;
    }
    if t.duration_ms.is_none_or(|d| d == 0) {
        s -= 0.5;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use glyph_protocol::ProviderId;

    fn tr(title: &str, artist: &str) -> Track {
        Track {
            id: format!("{artist}:{title}"),
            source: ProviderId::Deezer,
            title: title.into(),
            artist: artist.into(),
            album: None,
            duration_ms: Some(210_000),
            cover_url: Some("https://img.example/c.jpg".into()),
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false, explicit_rating: None,
        }
    }

    fn bare(title: &str, artist: &str) -> Track {
        Track {
            id: title.into(),
            source: ProviderId::Deezer,
            title: title.into(),
            artist: artist.into(),
            album: None,
            duration_ms: None,
            cover_url: None,
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false, explicit_rating: None,
        }
    }

    #[test]
    fn credited_names_keep_their_spelling() {
        assert_eq!(
            credited_named("Petit Biscuit feat. XYLO"),
            vec![
                ("petit biscuit".to_string(), "Petit Biscuit".to_string()),
                ("xylo".to_string(), "XYLO".to_string())
            ]
        );
        assert_eq!(
            credited("Petit Biscuit feat. XYLO"),
            credited_named("Petit Biscuit feat. XYLO")
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            credited_named("A, B & C"),
            vec![("a".into(), "A".into()), ("b".into(), "B".into()), ("c".into(), "C".into())]
        );
        assert_eq!(credited_named("  "), Vec::<(String, String)>::new());
    }

    #[test]
    fn quotes_are_exact_phrases() {
        let (plain, ph) = parse_query("miles \"Blue in Green\" davis");
        assert_eq!(plain, "miles Blue in Green davis");
        assert_eq!(ph, vec!["blue in green".to_string()]);
        assert!(matches_phrases(&bare("Blue in Green", "Miles Davis"), &ph));
        assert!(!matches_phrases(&bare("Green in Blue", "Miles Davis"), &ph));
        let (_, open) = parse_query("\"so what");
        assert_eq!(open, vec!["so what".to_string()]);
    }

    #[test]
    fn taste_and_text_rank() {
        let mut lib = Library::default();
        lib.liked.push(bare("Song", "Liked Band"));
        lib.followed.push(crate::library::FollowedArtist { name: "Liked Band".into(), track: None });
        let taste = Taste::from_library(&lib);
        assert!(taste.artist_weight("Liked Band feat. X") > 0.9);
        let a = score(&bare("Hello", "Liked Band"), "hello", &taste);
        let b = score(&bare("Hello", "Other"), "hello", &taste);
        let c = score(&bare("Hello (Karaoke)", "Other"), "hello", &taste);
        assert!(a > b && b > c);
        assert_eq!(taste.top_artists(1), vec!["liked band".to_string()]);
    }

    #[test]
    fn title_plus_artist_picks_the_right_song() {
        let taste = Taste::default();
        let right = bare("Believer", "Imagine Dragons");
        let cover = bare("Believer", "Some Cover Band");
        let other = bare("Thunder", "Imagine Dragons");
        for q in [
            "believer imagine dragons",
            "imagine dragons believer",
            "Believer - Imagine Dragons",
            "believer von imagine dragons",
            "believer by imagine dragons",
        ] {
            let r = score(&right, q, &taste);
            assert!(r > score(&cover, q, &taste), "{q}");
            assert!(r > score(&other, q, &taste), "{q}");
        }
        assert!(score(&bare("Halo", "Beyoncé"), "halo beyonce", &taste) > score(&bare("Halo", "Other"), "halo beyonce", &taste));
        let upload = bare("Imagine Dragons - Believer (Official Music Video)", "ImagineDragonsVEVO");
        assert!(score(&upload, "imagine dragons believer", &taste) > score(&cover, "imagine dragons believer", &taste));
        let feat = bare("Stay", "Justin Bieber feat. The Kid LAROI");
        assert!(score(&feat, "stay justin bieber", &taste) > score(&bare("Stay", "Rihanna"), "stay justin bieber", &taste));
        assert!(score(&right, "believer", &taste) > score(&other, "believer", &taste));
    }

    #[test]
    fn small_typos_still_find_the_song() {
        let taste = Taste::default();
        let right = bare("Believer", "Imagine Dragons");
        let other = bare("Thunder", "Imagine Dragons");
        let stranger = bare("Something", "Other Band");
        assert!(score(&right, "beleiver", &taste) > score(&other, "beleiver", &taste) + 1.0);
        assert!(score(&right, "imagine dragns beleiver", &taste) > score(&other, "imagine dragns beleiver", &taste));
        assert!(score(&bare("Enter Sandman", "Metallica"), "metalica", &taste) > score(&stranger, "metalica", &taste) + 1.0);
        // Short words never count as typos of each other.
        assert!(!word_close("she", "the"));
        assert!(word_close("beleiver", "believer"));
        assert!(!word_close("believer", "achiever"));
    }

    #[test]
    fn search_picks_and_graph_neighbours_rank_higher() {
        let mut lib = Library::default();
        lib.liked.push(bare("Show Me How", "Men I Trust"));
        lib.create_playlist("Night").tracks = vec![bare("Show Me How", "Men I Trust"), bare("Locket", "Crumb")];
        lib.search_items.push(crate::library::SearchItem::Song { track: bare("Hello", "Picked") });
        let taste = Taste::from_library(&lib);
        assert!(score(&bare("Hello", "Picked"), "hello", &taste) > score(&bare("Hello", "Other"), "hello", &taste));
        assert!(score(&bare("Bones", "Crumb"), "bones", &taste) > score(&bare("Bones", "Stranger"), "bones", &taste));
        let base = TimeBucket::Night;
        assert!(
            taste.score_recommendation(&bare("Bones", "Crumb"), 0.5, base, false)
                > taste.score_recommendation(&bare("Bones", "Stranger"), 0.5, base, false)
        );
    }

    #[test]
    fn region_and_completeness_rank_but_never_filter() {
        let taste = Taste::default().with_region(Some("de"));
        let mut german = bare("Hello", "A");
        german.isrc = Some("DEUM71500123".into());
        german.cover_url = Some("https://x/c.jpg".into());
        german.duration_ms = Some(200_000);
        let mut us = german.clone();
        us.isrc = Some("USUM71500123".into());
        let plain = bare("Hello", "A");
        let (g, u, b) = (score(&german, "hello", &taste), score(&us, "hello", &taste), score(&plain, "hello", &taste));
        assert!(g > u && u > b, "{g} {u} {b}");
        let mut exact = bare("Hello World", "B");
        exact.isrc = Some("GBUM71500123".into());
        exact.cover_url = german.cover_url.clone();
        exact.duration_ms = german.duration_ms;
        assert!(score(&exact, "hello world", &taste) > score(&german, "hello world", &taste));
        assert_eq!(region_of("de-DE").as_deref(), Some("DE"));
        assert_eq!(region_of("en_US.UTF-8").as_deref(), Some("US"));
        assert_eq!(region_of("zh-Hans-CN").as_deref(), Some("CN"));
        assert_eq!(region_of("de"), None);
    }

    #[test]
    fn artist_search_puts_releases_over_titled_uploads() {
        let taste = Taste::default();
        let mut release = bare("Instant Crush", "Daft Punk");
        release.isrc = Some("USQX91300101".into());
        release.cover_url = Some("https://x/c.jpg".into());
        release.duration_ms = Some(337_000);
        let mut upload = bare("Daft Punk - Get Lucky", "Daft Punk");
        upload.cover_url = Some("https://x/w.jpg".into());
        assert!(score(&release, "daft punk", &taste) > score(&upload, "daft punk", &taste));
        assert!(score(&upload, "get lucky", &taste) > score(&release, "get lucky", &taste));
    }

    #[test]
    fn searched_artists_and_mood_queries_shape_intent() {
        let mut lib = Library::default();
        lib.search_items.push(crate::library::SearchItem::Artist { name: "Searched".into(), track: None });
        lib.search_items.push(crate::library::SearchItem::Query { text: "chill dream pop".into() });
        let taste = Taste::from_library(&lib);
        assert!(taste.artist_weight("Searched") > 0.0);
        assert!(score(&bare("Song", "Searched"), "song", &taste) > score(&bare("Song", "Other"), "song", &taste));
        // Mood/genre search query lifts matching artist above mismatched genre artist!
        let dreamy = tr("Space Song", "Beach House");
        let heavy = tr("Enter Sandman", "Metallica");
        assert!(score(&dreamy, "dream pop", &taste) > score(&heavy, "dream pop", &taste) + 0.8);
    }

    #[test]
    fn sublinear_compression_prevents_one_artist_from_crushing_taste_diversity() {
        let mut lib = Library::default();
        for i in 0..35 {
            lib.liked.push(tr(&format!("Track {i}"), "Obsession Band"));
        }
        lib.followed.push(crate::library::FollowedArtist { name: "Second Favorite".into(), track: None });
        lib.liked.push(tr("Great Song", "Second Favorite"));
        let taste = Taste::from_library(&lib);
        assert!(
            taste.artist_weight("Second Favorite") > 0.50,
            "expected sublinear compression to preserve secondary favorite weight, got {}",
            taste.artist_weight("Second Favorite")
        );
    }

    #[test]
    fn context_aware_and_burst_aware_skips_understand_user_intent() {
        let mut lib = Library::default();
        let now = 1_750_000_000;
        let fav = tr("Let It Happen", "Tame Impala");
        lib.liked.push(fav.clone());

        // 1. Rapid queue-surfing burst (< 3s each within 4s): 3rd track should NOT be poisoned!
        let a = tr("Track A", "Surf Artist A");
        let b = tr("Track B", "Surf Artist B");
        let c = tr("Track C", "Surf Artist C");
        lib.taste.record_playback_outcome_ctx(&a, 2_000, Some(200_000), true, false, SessionEventContext::Library, 18, false, now);
        lib.taste.record_playback_outcome_ctx(&b, 1_800, Some(200_000), true, false, SessionEventContext::Library, 18, false, now + 3);
        lib.taste.record_playback_outcome_ctx(&c, 1_500, Some(200_000), true, false, SessionEventContext::Library, 18, false, now + 6);
        assert_eq!(
            lib.taste.artist_signals.get("surf artist c").map(|s| s.instant_skips).unwrap_or(0),
            0,
            "rapid queue-surfing burst must not permanently mark skipped artist as instant-hated"
        );

        // 2. Skipping a liked artist in Library context does not flip them negative,
        //    whereas skipping a Radio track immediately suppresses it!
        lib.taste.record_playback_outcome_ctx(&fav, 4_000, Some(200_000), true, false, SessionEventContext::Library, 18, false, now + 60);
        let radio_skip = tr("Unwanted Radio", "Radio Reject");
        lib.taste.record_playback_outcome_ctx(&radio_skip, 4_000, Some(200_000), true, false, SessionEventContext::Radio, 18, false, now + 120);

        let taste = Taste::from_library_context(&lib, now + 130, 18, false);
        assert!(taste.artist_affinity("Tame Impala") > 0.3, "library skip must not poison loved artist");
        assert!(taste.artist_affinity("Radio Reject") < -0.3, "radio skip should suppress unwanted artist");
        assert_eq!(taste.inferred_intent().mode, ListeningMode::VibeReset);
    }

    #[test]
    fn replays_and_session_transitions_strengthen_intent_and_pairs() {
        let mut lib = Library::default();
        let now = 1_750_000_000;
        let t1 = tr("Show Me How", "Men I Trust");
        let t2 = tr("Space Song", "Beach House");
        lib.taste.record_playback_outcome_ctx(&t1, 210_000, Some(210_000), false, true, SessionEventContext::Explicit, 23, false, now);
        lib.taste.record_intent_action(&t1, IntentAction::Replayed, SessionEventContext::Explicit, now + 100);
        lib.taste.record_intent_action(&t1, IntentAction::Replayed, SessionEventContext::Explicit, now + 300);
        lib.taste.record_playback_outcome_ctx(&t2, 220_000, Some(220_000), false, true, SessionEventContext::Explicit, 23, false, now + 500);

        assert!(!lib.taste.session_pairs.is_empty(), "sequential completions should capture session transition pair");
        let taste = Taste::from_library_context(&lib, now + 520, 23, false);
        assert!(taste.track_love_score(&t1) > 0.35);
        assert_eq!(taste.inferred_intent().mode, ListeningMode::Comfort);
    }

    #[test]
    fn build_profile_defaults_to_server_mode_and_respects_local_only_opt_out() {
        let mut lib = Library::default();
        lib.liked.push(tr("Let It Happen", "Tame Impala"));
        lib.taste.set_collab_cache(
            TasteCollabResponse {
                recommendations: vec![CollabArtistRec {
                    artist: "Pond".into(),
                    score: 0.9,
                    because_of: vec!["Tame Impala".into()],
                    genres: vec!["Psychedelic".into()],
                    collaborative: true,
                }],
                contributors: 12,
                model_ready: true,
            },
            1000,
        );

        let server_mode = build_profile(&lib, &TasteConfig::default(), true, 1000, 20);
        assert_eq!(server_mode.mode, "server");
        assert_eq!(server_mode.collab_recs.len(), 1);
        assert!(!server_mode.top_genres.is_empty());

        let local_opt_out = build_profile(
            &lib,
            &TasteConfig { enabled: true, server_collab: false, metadata_enrichment: true, drift_radio: false },
            true,
            1000,
            20,
        );
        assert_eq!(local_opt_out.mode, "local");
        assert!(local_opt_out.collab_recs.is_empty(), "server recs must not surface when opted out to local-only");

        let off = build_profile(
            &lib,
            &TasteConfig { enabled: false, server_collab: true, metadata_enrichment: true, drift_radio: false },
            true,
            1000,
            20,
        );
        assert_eq!(off.mode, "off");
        assert!(off.collab_recs.is_empty());
    }

    #[test]
    fn empty_library_and_legacy_deserialization_are_safe() {
        let legacy: Library = serde_json::from_str(r#"{"liked":[],"recent":[],"playlists":[]}"#).unwrap();
        assert!(legacy.taste.is_empty());
        let taste = Taste::from_library(&legacy);
        assert!(taste.is_empty());
        assert!(build_daily_mixes(&legacy, &taste, &[], "2026-11-22", 12, false).is_empty());
        let prof = build_profile(&legacy, &TasteConfig::default(), false, 1000, 12);
        assert_eq!(prof.mode, "local");
        assert_eq!(prof.signals_count, 0);

        let cfg: crate::Config = toml::from_str("").unwrap();
        assert!(cfg.taste.enabled);
        assert!(cfg.taste.server_collab);
        assert!(cfg.taste.is_server_mode());
        assert!(!cfg.taste.is_local_only());
        assert!(cfg.taste.metadata_enrichment);
    }

    #[test]
    fn skipped_track_start_does_not_get_phantom_positive_affinity_from_recent_or_plays() {
        let mut lib = Library::default();
        let now = 1_750_000_000;
        let skipped = tr("Unwanted Autoplay", "Random Autoplay Artist");

        lib.push_recent(skipped.clone());
        lib.record_play(&skipped);
        lib.record_skip(&skipped);
        lib.taste.record_playback_outcome(&skipped, 6_000, Some(200_000), true, false, 15, now);

        let taste = Taste::from_library_at(&lib, now);
        assert!(taste.artist_affinity("Random Autoplay Artist") < -0.3);
        assert_eq!(taste.artist_weight("Random Autoplay Artist"), 0.0);

        let prof = build_profile(&lib, &TasteConfig::default(), false, now, 15);
        assert!(!prof.liked_artists.iter().any(|a| a.name == "Random Autoplay Artist"));
    }

    #[test]
    fn soft_genre_family_matching_bridges_related_scenes() {
        let mut lib = Library::default();
        lib.liked.push(tr("Show Me How", "Men I Trust")); // Dream Pop / Indie Pop
        let taste = Taste::from_library(&lib);
        // Slowdive is Shoegaze / Dream Pop -> high soft tag affinity!
        let slowdive = tr("When the Sun Hits", "Slowdive");
        let metallica = tr("Enter Sandman", "Metallica");
        assert!(taste.track_tag_affinity(&slowdive) > taste.track_tag_affinity(&metallica) + 0.5);
    }

    #[test]
    fn daily_mixes_cluster_deterministically_with_no_back_to_back_artists() {
        let mut lib = Library::default();
        let now = 1_750_000_000;
        lib.taste.set_onboarding(
            vec!["Tame Impala".into(), "Daft Punk".into(), "Kendrick Lamar".into()],
            vec![
                tr("Let It Happen", "Tame Impala"),
                tr("Instant Crush", "Daft Punk"),
                tr("HUMBLE.", "Kendrick Lamar"),
            ],
            vec!["Indie".into(), "Electronic".into()],
        );
        lib.liked.push(tr("Borderline", "Tame Impala"));
        lib.liked.push(tr("Show Me How", "Men I Trust"));
        lib.liked.push(tr("One More Time", "Daft Punk"));
        lib.liked.push(tr("Delilah", "Fred again.."));
        lib.taste.record_playback_outcome(&tr("Show Me How", "Men I Trust"), 200_000, Some(200_000), false, true, 23, now);

        let candidates = vec![
            tr("New Person Same Old Mistakes", "Tame Impala"),
            tr("Lauren", "Men I Trust"),
            tr("Locket", "Crumb"),
            tr("Space Song", "Beach House"),
            tr("Around the World", "Daft Punk"),
            tr("10%", "Kaytranada"),
            tr("Never Be Like You", "Flume"),
            tr("DNA.", "Kendrick Lamar"),
            tr("EARFQUAKE", "Tyler, The Creator"),
        ];

        let taste = Taste::from_library_context(&lib, now, 23, false);
        let mixes_a = build_daily_mixes(&lib, &taste, &candidates, "2026-11-22", 23, false);
        let mixes_b = build_daily_mixes(&lib, &taste, &candidates, "2026-11-22", 23, false);
        assert_eq!(mixes_a, mixes_b);
        assert!(mixes_a.iter().any(|m| m.kind == "genre"));
        assert!(mixes_a.iter().any(|m| m.kind == "time" && m.title == "Late Night Mix"));
        assert!(mixes_a.iter().any(|m| m.kind == "discovery"));
        for mix in &mixes_a {
            for w in mix.tracks.windows(2) {
                assert_ne!(
                    normalize_artist(&primary_artist(&w[0].artist)),
                    normalize_artist(&primary_artist(&w[1].artist)),
                    "mix {} repeated artist back-to-back",
                    mix.id
                );
            }
        }
    }
}
