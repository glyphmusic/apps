//! Listening taste engine, personalized Daily Mixes, visible Taste Profile,
//! recommendation/radio scoring, and search ranking.
//!
//! Architecture (hybrid privacy-first):
//! - All behavioral signals and the full profile live locally in [`Library::taste`]
//!   (never uploaded as raw history).
//! - Positive signals: likes, follows, playlists, onboarding picks, completions,
//!   replays, time-of-day listening patterns, search picks, explicit "more".
//! - Negative signals: instant/early/mid skips with Bayesian abandonment rate,
//!   repeated-skip streaks, explicit "less"/"block"/"hide", track-level dislikes,
//!   explicit tag dislikes, and adaptive version/style avoidance (live, acoustic,
//!   remix, sped up, slowed, nightcore, karaoke, cover, instrumental, 8d, ...).
//! - Optional server enrichment (`config.taste.server_collab` / `metadata_enrichment`)
//!   adds shared genre/mood tags and anonymized item-item collaborative filtering.
//!
//! Search queries may quote phrases (`"blue in green"`): quoted parts must
//! appear exactly (case-insensitive) in the title, artist or album.
//!
//! Search ranking never filters by region: the user's country (from the system
//! locale) only lifts songs released there a little, next to their taste,
//! the artists they searched before and how complete a result is.

use crate::config::TasteConfig;
use crate::dedupe::{normalize_artist, normalize_title, primary_artist, track_key};
use crate::library::Library;
use glyph_protocol::taste::{
    ArtistTags, CollabArtistRec, TasteArtistSignal, TasteCollabResponse,
};
use glyph_protocol::Track;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

const MAX_TRACK_SIGNALS: usize = 600;
const MAX_ARTIST_SIGNALS: usize = 800;
const MAX_TAG_CACHE: usize = 500;
const MAX_DISLIKED_TRACKS: usize = 300;
const TAG_CACHE_TTL_S: u64 = 14 * 86_400;
const DECAY_HALF_LIFE_S: f64 = 30.0 * 86_400.0;

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

/// Per-track implicit playback signals (completions, skip timing, streaks).
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
}

impl TrackSignal {
    pub fn weighted_skips(&self) -> f32 {
        self.instant_skips as f32 * 1.4
            + self.early_skips as f32 * 0.85
            + self.mid_abandons as f32 * 0.35
    }

    pub fn total_skips(&self) -> u32 {
        self.instant_skips + self.early_skips + self.mid_abandons
    }
}

/// Per-artist implicit playback signals & time-of-day profile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtistSignal {
    #[serde(default)]
    pub display: String,
    #[serde(default)]
    pub plays: u32,
    #[serde(default)]
    pub completions: u32,
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
    #[serde(default)]
    pub last_event_s: u64,
}

impl ArtistSignal {
    pub fn weighted_skips(&self) -> f32 {
        self.instant_skips as f32 * 1.35
            + self.early_skips as f32 * 0.80
            + self.mid_abandons as f32 * 0.30
    }

    pub fn total_skips(&self) -> u32 {
        self.instant_skips + self.early_skips + self.mid_abandons
    }

    /// Bayesian-smoothed abandonment rate in `0.0..=1.0`.
    pub fn abandon_rate(&self) -> f32 {
        let sk = self.weighted_skips();
        let pos = self.plays as f32 + self.completions as f32 * 0.5;
        if sk <= 0.0 {
            return 0.0;
        }
        (sk / (pos + sk + 2.5)).clamp(0.0, 1.0)
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

/// Locally cached genres, moods, tags and similar artists for one normalized artist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CachedArtistTags {
    #[serde(default)]
    pub artist: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moods: Vec<String>,
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
}

impl TasteActivity {
    pub fn is_empty(&self) -> bool {
        self.track_signals.is_empty()
            && self.artist_signals.is_empty()
            && self.version_signals.is_empty()
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
    /// Locally cached genre/mood/similar metadata per normalized artist.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artist_tags: BTreeMap<String, CachedArtistTags>,
    /// Last server collaborative filtering response (only when `server_collab` is on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collab_cache: Option<CollabCache>,
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
    }

    pub fn activity_snapshot(&self) -> TasteActivity {
        TasteActivity {
            track_signals: self.track_signals.clone(),
            artist_signals: self.artist_signals.clone(),
            version_signals: self.version_signals.clone(),
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
    }

    /// Records the behavioral outcome when a track finishes or is skipped/abandoned.
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
        let dur = duration_ms.or(track.duration_ms).filter(|d| *d >= 5_000);
        let ratio: f32 = if completed {
            1.0
        } else if let Some(d) = dur {
            (position_ms as f32 / d as f32).clamp(0.0, 1.0)
        } else {
            // Unknown duration: treat 180 s as nominal song length.
            (position_ms as f32 / 180_000.0).clamp(0.0, 1.0)
        };

        let is_complete = completed || ratio >= 0.80;
        let is_instant_skip = manual_skip && !is_complete && (position_ms < 5_000 || ratio < 0.05);
        let is_early_skip =
            manual_skip && !is_complete && !is_instant_skip && (position_ms < 20_000 || ratio < 0.20);
        let is_mid_abandon =
            manual_skip && !is_complete && !is_instant_skip && !is_early_skip && ratio < 0.40;
        let is_play = is_complete || (!is_instant_skip && !is_early_skip && !is_mid_abandon && (position_ms >= 20_000 || ratio >= 0.40));

        if !is_play && !is_instant_skip && !is_early_skip && !is_mid_abandon {
            return;
        }

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
        } else if is_play {
            ts.plays = ts.plays.saturating_add(1);
            ts.consecutive_skips = 0;
        } else if is_instant_skip {
            ts.instant_skips = ts.instant_skips.saturating_add(1);
            ts.consecutive_skips = ts.consecutive_skips.saturating_add(1);
        } else if is_early_skip {
            ts.early_skips = ts.early_skips.saturating_add(1);
            ts.consecutive_skips = ts.consecutive_skips.saturating_add(1);
        } else if is_mid_abandon {
            ts.mid_abandons = ts.mid_abandons.saturating_add(1);
            ts.consecutive_skips = ts.consecutive_skips.saturating_add(1);
        }

        // Update per-artist signals for every credited artist.
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
            } else if is_play {
                entry.plays = entry.plays.saturating_add(1);
                entry.consecutive_skips = 0;
                entry.time_buckets[bucket] += 0.6;
            } else if is_instant_skip {
                entry.instant_skips = entry.instant_skips.saturating_add(1);
                entry.consecutive_skips = entry.consecutive_skips.saturating_add(1);
            } else if is_early_skip {
                entry.early_skips = entry.early_skips.saturating_add(1);
                entry.consecutive_skips = entry.consecutive_skips.saturating_add(1);
            } else if is_mid_abandon {
                entry.mid_abandons = entry.mid_abandons.saturating_add(1);
                entry.consecutive_skips = entry.consecutive_skips.saturating_add(1);
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
            } else {
                v.skips = v.skips.saturating_add(1);
            }
        }

        self.trim();
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

/// Time decay factor in `0.1..=1.0` with a 30-day half-life.
fn recency_decay(last_s: u64, now_s: u64) -> f32 {
    if last_s == 0 || now_s <= last_s {
        return 1.0;
    }
    let age = (now_s - last_s) as f64;
    ((-age / DECAY_HALF_LIFE_S).exp2() as f32).clamp(0.10, 1.0)
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

    if has_word(&t, "karaoke") || has_word(&t, "backing track") {
        push("karaoke");
    }
    if has_word(&t, "instrumental") {
        push("instrumental");
    }
    if has_word(&t, "8d") || has_word(&t, "8d audio") {
        push("8d");
    }
    if has_word(&t, "nightcore") {
        push("nightcore");
    }
    if has_word(&t, "sped up") || has_word(&t, "speed up") || has_word(&t, "spedups") {
        push("sped_up");
    }
    if has_word(&t, "slowed") || has_word(&t, "slowed reverb") || has_word(&t, "slowed down") {
        push("slowed");
    }
    if has_word(&t, "cover") || has_word(&t, "tribute") || has_word(&t, "remake") {
        push("cover");
    }
    if has_word(&t, "remix")
        || has_word(&t, "club mix")
        || has_word(&t, "extended mix")
        || has_word(&t, "vip mix")
    {
        push("remix");
    }
    if has_word(&t, "acoustic") || has_word(&t, "unplugged") {
        push("acoustic");
    }
    if has_word(&t, "live")
        || has_word(&t, "live at")
        || has_word(&t, "live from")
        || has_word(&a, "live at")
        || has_word(&a, "live from")
        || has_word(&a, "unplugged")
    {
        push("live");
    }
    if has_word(&t, "lofi") || has_word(&t, "lo fi") {
        push("lofi");
    }
    if has_word(&t, "demo") || has_word(&t, "demo version") {
        push("demo");
    }
    out
}

/// Built-in local fallback genres for well-known starter artists so the Taste
/// Profile & Daily Mixes show rich genres immediately even offline or before
/// server tag enrichment completes.
fn builtin_artist_genres(norm_artist: &str) -> Option<&'static [&'static str]> {
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
        "kanye west" => &["Hip-Hop", "Rap"],
        "drake" => &["Hip-Hop", "R&B", "Pop"],
        "pink floyd" => &["Rock", "Psychedelic"],
        "nirvana" => &["Grunge", "Rock", "Alternative"],
        "taylor swift" => &["Pop", "Indie Folk"],
        _ => return None,
    })
}

/// Multi-dimensional listening taste computed from a [`Library`].
#[derive(Debug, Default, Clone)]
pub struct Taste {
    /// Positive normalized artist weights (`0.0..=1.0`), backwards-compatible.
    artists: HashMap<String, f32>,
    /// Signed artist affinities (`-1.0..=1.0`), capturing both love and dislike.
    artist_affinities: HashMap<String, f32>,
    /// Display spelling for known normalized artists.
    artist_display: HashMap<String, String>,
    /// Human-readable reason keys per artist (`"liked"`, `"followed"`, `"onboarding"`, ...).
    artist_reasons: HashMap<String, Vec<String>>,
    /// Per-artist time-of-day normalized weights `[Morning, Afternoon, Evening, Night]`.
    artist_time: HashMap<String, [f32; 4]>,
    /// Cached normalized artist -> `(genres, moods, similar)`.
    artist_meta: HashMap<String, (Vec<String>, Vec<String>, Vec<String>)>,
    /// Signed genre affinities (`-1.0..=1.0`), keyed by canonical genre name.
    genre_scores: HashMap<String, f32>,
    /// Signed mood affinities (`-1.0..=1.0`), keyed by canonical mood name.
    mood_scores: HashMap<String, f32>,
    /// Learned + explicit version/style preference (`-1.0..=1.0`).
    version_scores: HashMap<String, f32>,
    /// Track-level skip suppression (`0.0..=1.0`), keyed by `song_key`.
    track_penalties: HashMap<String, f32>,
    /// Tracks explicitly disliked by the user (`song_key`).
    disliked_tracks: HashSet<String>,
    /// Hidden / blocked artists (normalized).
    hidden: HashSet<String>,
    /// Liked tracks (`song_key`).
    liked: HashSet<String>,
    /// The user's country ("DE"), for a small lift of songs released there.
    region: Option<String>,
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

/// Separators between the artists of one credit, with the spaces they are
/// written with ("A feat. B", "A & B", "A, B", "A x B", ...).
const CREDIT_SEPS: [&str; 15] = [
    " feat. ", " feat ", " ft. ", " ft ", " featuring ", " & ", " x ", " × ", " with ", " vs. ", " vs ", " и ", ";", "/", ",",
];

/// A credit split into the artists as written: "A feat. B" -> ["A", "B"].
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

/// Every credited artist of "A, B feat. C" as (normalised key, name as
/// written), so statistics can show each artist under their own spelling.
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

        // --- Explicit positive library signals ---
        for t in &lib.liked {
            add(&mut raw_w, &mut display_map, &mut reasons, &t.artist, 3.0, Some("liked"));
        }
        for f in &lib.followed {
            add(&mut raw_w, &mut display_map, &mut reasons, &f.name, 6.0, Some("followed"));
        }
        // User playlists: cap contribution per artist per playlist so one mega-playlist doesn't skew.
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
        // Legacy binary skips (still respected for older data or when rich signals weren't recorded).
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

        // --- Rich implicit behavioral signals (completion, skip timing, abandon rate, streaks) ---
        let mut artist_time: HashMap<String, [f32; 4]> = HashMap::new();
        for (norm, sig) in &lib.taste.artist_signals {
            if !sig.display.is_empty() {
                display_map.entry(norm.clone()).or_insert_with(|| sig.display.clone());
            }
            let decay = recency_decay(sig.last_event_s, now_s);
            let pos = (1.0 + sig.plays as f32).ln() * 1.1
                + (1.0 + sig.completions as f32).ln() * 1.4;
            let neg = sig.instant_skips as f32 * 1.6
                + sig.early_skips as f32 * 0.9
                + sig.mid_abandons as f32 * 0.35
                + (sig.consecutive_skips.min(6) as f32) * 0.75;
            let abandon = sig.abandon_rate();
            let abandon_penalty = if abandon > 0.45 && sig.total_skips() >= 2 {
                abandon * 4.0
            } else {
                0.0
            };
            let delta = (pos - neg - abandon_penalty) * decay;
            *raw_w.entry(norm.clone()).or_default() += delta;
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

        // --- Recent search picks ---
        for (i, item) in lib.search_items.iter().take(40).enumerate() {
            let fade = 0.95f32.powi(i as i32);
            match item {
                crate::library::SearchItem::Artist { name, .. } => add(&mut raw_w, &mut display_map, &mut reasons, name, 2.5 * fade, None),
                crate::library::SearchItem::Song { track } => add(&mut raw_w, &mut display_map, &mut reasons, &track.artist, 1.5 * fade, None),
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
        }

        let max_pos = raw_w.values().cloned().fold(0.0f32, f32::max).max(1.0);
        let max_neg = raw_w
            .values()
            .filter(|v| **v < 0.0)
            .map(|v| v.abs())
            .fold(0.0f32, f32::max)
            .max(4.0);

        let mut artists: HashMap<String, f32> = HashMap::new();
        let mut artist_affinities: HashMap<String, f32> = HashMap::new();
        for (k, v) in raw_w {
            if v > 0.0 {
                let norm_v = (v / max_pos).min(1.0);
                artists.insert(k.clone(), norm_v);
                artist_affinities.insert(k, norm_v);
            } else if v < 0.0 {
                let neg_v = -(v.abs() / max_neg).min(1.0);
                artist_affinities.insert(k, neg_v);
            }
        }
        for h in &hidden {
            artist_affinities.insert(h.clone(), -1.0);
        }

        // --- Track-level skip penalties & explicit dislikes ---
        let mut track_penalties: HashMap<String, f32> = HashMap::new();
        for (k, sig) in &lib.taste.track_signals {
            let decay = recency_decay(sig.last_event_s, now_s);
            let sk = sig.weighted_skips();
            if sk > 0.0 {
                let pos = sig.plays as f32 + sig.completions as f32 * 0.8;
                let base = sk / (pos + sk + 1.5);
                let streak_boost = (sig.consecutive_skips.min(4) as f32) * 0.14;
                let pen = ((base + streak_boost) * decay).clamp(0.0, 1.0);
                if pen > 0.12 {
                    track_penalties.insert(k.clone(), pen);
                }
            }
        }
        let disliked_tracks: HashSet<String> = lib.taste.disliked_tracks.keys().cloned().collect();
        for dk in &disliked_tracks {
            track_penalties.insert(dk.clone(), 1.0);
        }

        // --- Artist genre/mood metadata map (cached + built-in fallback) ---
        let mut artist_meta: HashMap<String, (Vec<String>, Vec<String>, Vec<String>)> = HashMap::new();
        for (k, c) in &lib.taste.artist_tags {
            if !c.artist.is_empty() {
                display_map.entry(k.clone()).or_insert_with(|| c.artist.clone());
            }
            let (mut genres, mut moods) = (c.genres.clone(), c.moods.clone());
            if genres.is_empty() {
                if let Some(gs) = builtin_artist_genres(k) {
                    let raw: Vec<String> = gs.iter().map(|s| (*s).to_string()).collect();
                    let (fg, fm, _) = glyph_protocol::taste::classify_tags(&raw, k);
                    genres = fg;
                    if moods.is_empty() {
                        moods = fm;
                    }
                }
            }
            artist_meta.insert(k.clone(), (genres, moods, c.similar.clone()));
        }
        for k in artist_affinities.keys() {
            if !artist_meta.contains_key(k) {
                if let Some(gs) = builtin_artist_genres(k) {
                    let raw: Vec<String> = gs.iter().map(|s| (*s).to_string()).collect();
                    let (genres, moods, _) = glyph_protocol::taste::classify_tags(&raw, k);
                    artist_meta.insert(k.clone(), (genres, moods, Vec::new()));
                }
            }
        }

        // --- Aggregate genre & mood scores from artist affinities + explicit feedback ---
        let mut genre_raw: HashMap<String, f32> = HashMap::new();
        let mut mood_raw: HashMap<String, f32> = HashMap::new();

        for g in &lib.taste.onboarding_genres {
            *genre_raw.entry(g.clone()).or_default() += 1.2;
        }
        for (artist_k, aff) in &artist_affinities {
            if let Some((genres, moods, _)) = artist_meta.get(artist_k) {
                // Dampen single-artist negative affinity so hiding one artist
                // inside a loved genre doesn't wipe out the whole genre.
                let eff_aff = if *aff >= 0.0 { *aff } else { *aff * 0.35 };
                for (i, g) in genres.iter().enumerate() {
                    let rank_w = 1.0 / (1.0 + i as f32 * 0.35);
                    *genre_raw.entry(g.clone()).or_default() += eff_aff * rank_w;
                }
                for (i, m) in moods.iter().enumerate() {
                    let rank_w = 1.0 / (1.0 + i as f32 * 0.35);
                    *mood_raw.entry(m.clone()).or_default() += eff_aff * rank_w;
                }
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
                // Capitalize nicely for display.
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

        // --- Adaptive version/style preferences ---
        let mut version_scores: HashMap<String, f32> = HashMap::new();
        for &style in VERSION_STYLES {
            let explicit = lib.taste.version_feedback.get(style).copied();
            let sig = lib.taste.version_signals.get(style);
            // Count liked songs carrying this version style too!
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
                    // Blend learned preference with mild default prior.
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

        let liked = lib.liked.iter().map(song_key).collect();

        Self {
            artists,
            artist_affinities,
            artist_display: display_map,
            artist_reasons: reasons,
            artist_time,
            artist_meta,
            genre_scores,
            mood_scores,
            version_scores,
            track_penalties,
            disliked_tracks,
            hidden,
            liked,
            region: user_region(),
        }
    }

    /// Same weights for another country (tests, server-side ranking).
    pub fn with_region(mut self, region: Option<&str>) -> Self {
        self.region = region.map(str::to_ascii_uppercase);
        self
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

    pub fn track_skip_penalty(&self, t: &Track) -> f32 {
        self.track_penalties.get(&song_key(t)).copied().unwrap_or(0.0)
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

    /// Signed genre/mood affinity in `-1.0..=1.0` for `t`'s credited artists.
    pub fn track_tag_affinity(&self, t: &Track) -> f32 {
        let mut sum = 0.0f32;
        let mut cnt = 0.0f32;
        for a in credited(&t.artist) {
            if let Some((genres, moods, _)) = self.artist_meta.get(&a) {
                for g in genres {
                    if let Some(&s) = self.genre_scores.get(g) {
                        sum += s;
                        cnt += 1.0;
                    }
                }
                for m in moods {
                    if let Some(&s) = self.mood_scores.get(m) {
                        sum += s * 0.6;
                        cnt += 0.6;
                    }
                }
            } else if let Some(gs) = builtin_artist_genres(&a) {
                for &g in gs {
                    if let Some(&s) = self.genre_scores.get(g) {
                        sum += s;
                        cnt += 1.0;
                    }
                }
            }
        }
        if cnt > 0.0 {
            (sum / cnt).clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }

    /// Time-of-day alignment in `0.0..=1.0` for `t` in `bucket`.
    pub fn time_affinity(&self, t: &Track, bucket: TimeBucket) -> f32 {
        let idx = bucket.index();
        let mut best = 0.0f32;
        for a in credited(&t.artist) {
            if let Some(arr) = self.artist_time.get(&a) {
                best = best.max(arr[idx]);
            }
            if let Some((_, moods, _)) = self.artist_meta.get(&a) {
                let aligned = bucket.aligned_moods();
                let hits = moods.iter().filter(|m| aligned.contains(&m.as_str())).count();
                if hits > 0 {
                    best = best.max((hits as f32 * 0.35).min(0.75));
                }
            } else if let Some(gs) = builtin_artist_genres(&a) {
                let raw: Vec<String> = gs.iter().map(|s| (*s).to_string()).collect();
                let (_, moods, _) = glyph_protocol::taste::classify_tags(&raw, &a);
                let aligned = bucket.aligned_moods();
                let hits = moods.iter().filter(|m| aligned.contains(&m.as_str())).count();
                if hits > 0 {
                    best = best.max((hits as f32 * 0.35).min(0.75));
                }
            }
        }
        best.clamp(0.0, 1.0)
    }

    /// Unified score for Home recommendations, Radio / Autoplay, Daily Mixes,
    /// and playlist suggestions.
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
        let time_aff = self.time_affinity(t, bucket);
        let ver_aff = self.version_affinity(t);
        let skip_pen = self.track_skip_penalty(t);

        let mut sc = base_sim;
        // Positive artists boost, negative artists suppress sharply.
        sc += if artist_aff >= 0.0 {
            artist_aff * 0.95
        } else {
            artist_aff * 2.6
        };
        sc += tag_aff * 0.85;
        sc += time_aff * 0.35;
        sc += ver_aff * 1.1;
        sc -= skip_pen * 2.8;
        // Gentle discovery lift for fresh compatible tracks so feeds don't stagnate.
        if !is_familiar && tag_aff > 0.15 && artist_aff >= 0.0 {
            sc += 0.25;
        }
        if t.cover_url.as_deref().is_none_or(|c| c.trim().is_empty()) {
            sc -= 0.2;
        }
        sc
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

    /// Anonymized top positive artist signals for optional server collaborative filtering.
    pub fn top_positive_signals(&self, limit: usize) -> Vec<TasteArtistSignal> {
        let mut v: Vec<(&String, &f32)> = self.artists.iter().collect();
        v.sort_by(|a, b| b.1.total_cmp(a.1).then_with(|| a.0.cmp(b.0)));
        v.into_iter()
            .take(limit)
            .map(|(k, w)| TasteArtistSignal {
                artist: self.artist_display.get(k).cloned().unwrap_or_else(|| k.clone()),
                weight: *w,
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
    /// `"hybrid"`, `"local"`, or `"off"`.
    pub mode: String,
    /// `0.0..=1.0` profile maturity.
    pub confidence: f32,
    pub signals_count: u32,
    pub onboarding_done: bool,
    pub top_genres: Vec<WeightedTag>,
    pub avoided_genres: Vec<WeightedTag>,
    pub moods: Vec<WeightedTag>,
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
    let taste = Taste::from_library_at(lib, now_s);
    let mode = if !cfg.enabled {
        "off"
    } else if cfg.server_collab && server_available {
        "hybrid"
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

    // Listening habits (time-of-day, completion rate, skip rate, variety, versions).
    let mut buckets = [0.0f32; 4];
    let mut total_plays: u32 = 0;
    let mut total_completions: u32 = 0;
    let mut total_early_skips: u32 = 0;
    let mut total_skips: u32 = 0;
    for sig in lib.taste.artist_signals.values() {
        for i in 0..4 {
            buckets[i] += sig.time_buckets[i];
        }
        total_plays = total_plays.saturating_add(sig.plays);
        total_completions = total_completions.saturating_add(sig.completions);
        total_early_skips = total_early_skips.saturating_add(sig.instant_skips + sig.early_skips);
        total_skips = total_skips.saturating_add(sig.total_skips());
    }
    // Fold stats hourly distribution as a fallback if artist_signals time buckets are empty.
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

    // Normalized Shannon entropy across top positive artists -> variety score 0..1.
    let variety_score = {
        let vals: Vec<f32> = taste.artists.values().copied().filter(|v| *v > 0.02).collect();
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
                .map(|(g, _, _)| g.iter().take(2).cloned().collect())
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
        + total_skips;
    let confidence = (1.0 - (-(signals_count as f32) / 18.0).exp()).clamp(0.0, 1.0);

    let (collab_recs, collab_contributors, collab_model_ready) = if cfg.enabled && cfg.server_collab {
        match &lib.taste.collab_cache {
            Some(c) => (c.recommendations.clone(), c.contributors, c.model_ready),
            None => (Vec::new(), 0, false),
        }
    } else {
        (Vec::new(), 0, false)
    };

    TasteProfile {
        mode,
        confidence,
        signals_count,
        onboarding_done: lib.taste.onboarding_done,
        top_genres,
        avoided_genres,
        moods,
        habits: ListeningHabits {
            time_of_day,
            peak_period,
            completion_rate,
            early_skip_rate,
            variety_score,
            total_plays,
            total_skips,
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

// ------------------------------------------------ Daily Mixes generator

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyMix {
    pub id: String,
    /// `"genre"`, `"time"`, or `"discovery"`.
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Deterministic hue `0..360` for subtle card tinting.
    pub accent_hue: u16,
    pub tracks: Vec<Track>,
}

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
    // Group tracks by primary artist while preserving each artist's score order.
    let mut by_artist: Vec<(String, std::collections::VecDeque<Track>)> = Vec::new();
    for t in tracks {
        let a = normalize_artist(&primary_artist(&t.artist));
        if let Some((_, q)) = by_artist.iter_mut().find(|(k, _)| *k == a) {
            if q.len() < max_per_artist {
                q.push_back(t);
            }
        } else {
            let mut q = std::collections::VecDeque::new();
            q.push_back(t);
            by_artist.push((a, q));
        }
    }
    let mut out: Vec<Track> = Vec::with_capacity(limit);
    let mut last: Option<String> = None;
    while out.len() < limit {
        // Pick the non-consecutive artist with the most remaining tracks
        // (ties keep the higher-ranked artist that appeared earlier).
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

/// Generates personalized Daily Mixes (genre/cluster mixes, time-of-day mix,
/// and discovery mix) from the local library and fetched candidate tracks.
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

    // Familiar pool: liked songs + onboarding songs + user playlist tracks + recent tracks.
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
    for t in &lib.liked {
        add_fam(t);
    }
    for t in &lib.taste.onboarding_tracks {
        add_fam(t);
    }
    for pl in &lib.playlists {
        if pl.shared_by.is_none() {
            for t in &pl.tracks {
                add_fam(t);
            }
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

    // 1. Genre / Cluster Daily Mixes (up to 3 distinct top genres).
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
        let matches_genre = |t: &Track| -> bool {
            credited(&t.artist).iter().any(|a| {
                if let Some((gs, _, _)) = taste.artist_meta.get(a) {
                    gs.iter().any(|g| g.eq_ignore_ascii_case(&genre))
                } else if let Some(gs) = builtin_artist_genres(a) {
                    gs.iter().any(|g| g.eq_ignore_ascii_case(&genre))
                } else {
                    false
                }
            })
        };

        let mut fam_scored: Vec<(Track, f32)> = familiar_all
            .iter()
            .filter(|t| matches_genre(t) && !used_globally.contains(&track_key(t)))
            .map(|t| {
                let s = taste.score_recommendation(t, 1.0, bucket, true)
                    + daily_jitter(date_key, &mix_id, &track_key(t));
                (t.clone(), s)
            })
            .filter(|(_, s)| s.is_finite() && *s > -0.5)
            .collect();
        fam_scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));

        let mut fresh_scored: Vec<(Track, f32)> = fresh_all
            .iter()
            .filter(|t| matches_genre(t) && !used_globally.contains(&track_key(t)))
            .map(|t| {
                let s = taste.score_recommendation(t, 1.0, bucket, false)
                    + daily_jitter(date_key, &mix_id, &track_key(t));
                (t.clone(), s)
            })
            .filter(|(_, s)| s.is_finite() && *s > -0.5)
            .collect();
        fresh_scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));

        // Interleave ~50% familiar + ~50% fresh.
        let mut combined: Vec<Track> = Vec::new();
        let mut fi = fam_scored.into_iter().map(|(t, _)| t);
        let mut ni = fresh_scored.into_iter().map(|(t, _)| t);
        for _ in 0..24 {
            if let Some(t) = fi.next() {
                combined.push(t);
            }
            if let Some(t) = ni.next() {
                combined.push(t);
            }
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

    // Fallback Daily Mix 1 if no genres are tagged yet (brand-new offline library).
    if mixes.is_empty() {
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

    // 2. Time-of-Day Mix (Morning / Afternoon / Evening / Late Night).
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

    // 3. Discovery Mix (fresh songs not liked and not played recently).
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

// ---------------------------------------------------------------- queries

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

/// Folded words without punctuation ("Beyoncé - Halo" -> beyonce halo).
fn words(s: &str) -> Vec<String> {
    crate::dedupe::squash(s)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// Joiners people type between title and artist ("Song by Artist").
const JOINERS: [&str; 8] = ["by", "von", "feat", "ft", "featuring", "x", "and", "und"];

/// Song and artist typed together, in any order or with joiners.
fn title_artist_match(t: &Track, q: &str) -> f32 {
    let qw = words(q);
    if qw.len() < 2 {
        return 0.0;
    }
    let all_in_query = |ws: &[String]| !ws.is_empty() && ws.iter().all(|w| qw.contains(w));
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
        .filter(|w| !JOINERS.contains(&w.as_str()) && !full_title.contains(w) && !credits.contains(w) && !album.contains(w))
        .count();
    s - (missing as f32 * 0.6).min(2.4)
}

/// Rank of `t` for `query` (higher first): text match, adaptive version/style
/// preference, artist/genre taste, negative skip/dislike signals, region, and
/// metadata completeness.
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
    }

    // Adaptive version/style scoring: boost a style the user explicitly typed,
    // otherwise apply the learned (or default) style preference.
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
            // Default -0.55 maps to roughly -1.5, matching historical odd-version demotion.
            s += (pref * 2.7).max(-2.2);
        } else if pref > 0.15 {
            s += (pref * 0.7).min(0.6);
        }
    }

    // Signed artist taste + genre alignment + negative song signals.
    let aff = taste.artist_affinity(&t.artist);
    if aff >= 0.0 {
        s += aff * 2.0;
    } else {
        s += aff * 1.6;
    }
    s += taste.track_tag_affinity(t) * 0.45;
    if taste.is_liked(t) {
        s += 1.0;
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
            explicit: false,
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
            explicit: false,
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
    fn searched_artists_count_as_interest() {
        let mut lib = Library::default();
        lib.search_items.push(crate::library::SearchItem::Artist { name: "Searched".into(), track: None });
        let taste = Taste::from_library(&lib);
        assert!(taste.artist_weight("Searched") > 0.0);
        assert!(score(&bare("Song", "Searched"), "song", &taste) > score(&bare("Song", "Other"), "song", &taste));
    }

    #[test]
    fn skips_and_hidden_lower_weight() {
        let mut lib = Library::default();
        lib.push_recent(bare("A", "Skipped"));
        lib.push_recent(bare("B", "Kept"));
        lib.artist_skips.insert("skipped".into(), 8);
        lib.hidden_artists.push("Gone".into());
        lib.liked.push(bare("C", "Gone"));
        let taste = Taste::from_library(&lib);
        assert_eq!(taste.artist_weight("Skipped"), 0.0);
        assert!(taste.artist_weight("Kept") > 0.0);
        assert_eq!(taste.artist_weight("Gone"), 0.0);
        assert!(taste.is_hidden("Gone"));
    }

    #[test]
    fn detect_versions_uses_word_boundaries() {
        assert!(detect_versions("Discover", None).is_empty());
        assert!(detect_versions("Alive", None).is_empty());
        assert!(detect_versions("Deliver Me", None).is_empty());
        assert!(detect_versions("Undercover Martyn", None).is_empty());

        assert_eq!(detect_versions("Song (Live at Red Rocks)", None), vec!["live"]);
        assert_eq!(detect_versions("Song - Sped Up", None), vec!["sped_up"]);
        assert!(detect_versions("Song (Slowed + Reverb)", None).contains(&"slowed"));
        assert!(detect_versions("Song - Acoustic Remix", None).contains(&"acoustic"));
        assert!(detect_versions("Song - Acoustic Remix", None).contains(&"remix"));
    }

    #[test]
    fn implicit_skip_timing_and_adaptive_versions_shape_recommendations() {
        let mut lib = Library::default();
        let now = 1_750_000_000;
        let loved = tr("Let It Happen", "Tame Impala");
        let hated = tr("Annoying Track", "Skip Artist");
        let slowed_a = tr("After Dark (Slowed)", "Mr Kitty");
        let slowed_b = tr("Another (Slowed)", "Synth Band");

        // Complete loved track multiple times at night (23:00).
        for _ in 0..4 {
            lib.taste.record_playback_outcome(&loved, 210_000, Some(210_000), false, true, 23, now);
        }
        // Instant skip hated track 3 times (< 3 s).
        for _ in 0..3 {
            lib.taste.record_playback_outcome(&hated, 2_000, Some(200_000), true, false, 14, now);
        }
        // Complete slowed tracks repeatedly -> slowed version flips positive!
        for _ in 0..5 {
            lib.taste.record_playback_outcome(&slowed_a, 200_000, Some(200_000), false, true, 23, now);
        }

        let taste = Taste::from_library_at(&lib, now);
        assert!(taste.artist_affinity("Tame Impala") > 0.5);
        assert!(taste.artist_affinity("Skip Artist") < -0.3);
        assert!(taste.track_skip_penalty(&hated) > 0.5);
        assert!(taste.version_affinity(&slowed_b) > 0.1, "slowed should learn positive preference");
        assert!(taste.version_affinity(&tr("X (Karaoke)", "Y")) < -0.3);

        // Recommendation scoring should clearly rank loved over hated.
        let s_loved = taste.score_recommendation(&loved, 1.0, TimeBucket::Night, false);
        let s_hated = taste.score_recommendation(&hated, 1.0, TimeBucket::Night, false);
        assert!(s_loved > s_hated + 2.0);
    }

    #[test]
    fn explicit_track_dislike_blocks_single_song_without_banning_artist() {
        let mut lib = Library::default();
        let good = tr("Good Song", "Favorite Band");
        let bad = tr("Bad Song", "Favorite Band");
        lib.liked.push(good.clone());
        lib.taste.set_disliked_track(&bad, true);

        let taste = Taste::from_library(&lib);
        assert!(taste.artist_weight("Favorite Band") > 0.5);
        assert!(taste.is_disliked_track(&bad));
        assert!(!taste.is_disliked_track(&good));
        assert_eq!(taste.score_recommendation(&bad, 2.0, TimeBucket::Evening, false), f32::NEG_INFINITY);
        assert!(taste.score_recommendation(&good, 1.0, TimeBucket::Evening, true) > 0.5);
    }

    #[test]
    fn daily_mixes_cluster_by_genre_time_and_discovery_deterministically() {
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

        let taste = Taste::from_library_at(&lib, now);
        let mixes_a = build_daily_mixes(&lib, &taste, &candidates, "2026-11-22", 23, false);
        let mixes_b = build_daily_mixes(&lib, &taste, &candidates, "2026-11-22", 23, false);
        assert_eq!(mixes_a, mixes_b, "mixes must be deterministic for the same day");
        assert!(mixes_a.iter().any(|m| m.kind == "genre"));
        assert!(mixes_a.iter().any(|m| m.kind == "time" && m.title == "Late Night Mix"));
        assert!(mixes_a.iter().any(|m| m.kind == "discovery"));
        // Verify no artist appears back-to-back in any mix.
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

    #[test]
    fn build_profile_respects_server_collab_opt_out() {
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

        let hybrid = build_profile(&lib, &TasteConfig { enabled: true, server_collab: true, metadata_enrichment: true }, true, 1000, 20);
        assert_eq!(hybrid.mode, "hybrid");
        assert_eq!(hybrid.collab_recs.len(), 1);
        assert!(!hybrid.top_genres.is_empty());

        let local = build_profile(&lib, &TasteConfig { enabled: true, server_collab: false, metadata_enrichment: true }, true, 1000, 20);
        assert_eq!(local.mode, "local");
        assert!(local.collab_recs.is_empty(), "server collab recs must not surface when opted out");

        let off = build_profile(&lib, &TasteConfig { enabled: false, server_collab: true, metadata_enrichment: true }, true, 1000, 20);
        assert_eq!(off.mode, "off");
        assert!(off.collab_recs.is_empty());
    }

    #[test]
    fn empty_library_and_legacy_deserialization_are_safe() {
        // Legacy library JSON without `taste` field must load cleanly.
        let legacy: Library = serde_json::from_str(r#"{"liked":[],"recent":[],"playlists":[]}"#).unwrap();
        assert!(legacy.taste.is_empty());
        let taste = Taste::from_library(&legacy);
        assert!(taste.is_empty());
        assert!(build_daily_mixes(&legacy, &taste, &[], "2026-11-22", 12, false).is_empty());
        let prof = build_profile(&legacy, &TasteConfig::default(), false, 1000, 12);
        assert_eq!(prof.mode, "local");
        assert_eq!(prof.signals_count, 0);

        // Legacy config TOML without `[taste]` section defaults to enabled + hybrid.
        let cfg: crate::Config = toml::from_str("").unwrap();
        assert!(cfg.taste.enabled);
        assert!(cfg.taste.server_collab);
        assert!(cfg.taste.metadata_enrichment);
    }

    #[test]
    fn activity_snapshot_roundtrips_taste_behavioral_signals() {
        let mut lib = Library::default();
        let song = tr("Show Me How", "Men I Trust");
        lib.taste.record_playback_outcome(&song, 3_000, Some(215_000), true, false, 22, 10_000);
        let snap = lib.activity_snapshot();
        assert!(!snap.taste.is_empty());
        let mut restored = Library::default();
        restored.apply_activity(snap);
        assert_eq!(restored.taste.artist_signals.get("men i trust").map(|s| s.instant_skips), Some(1));
    }

    #[test]
    fn search_score_respects_typed_version_and_learned_version_preference() {
        let mut lib = Library::default();
        let slowed = tr("After Dark (Slowed)", "Mr Kitty");
        for _ in 0..6 {
            lib.taste.record_playback_outcome(&slowed, 200_000, Some(200_000), false, true, 23, 10_000);
        }
        let taste = Taste::from_library_at(&lib, 10_000);
        let neutral = Taste::default();
        let normal = tr("Hello", "Artist");
        let slowed_other = tr("Hello (Slowed)", "Artist");
        // Neutral user penalizes slowed when not typed.
        assert!(score(&normal, "hello", &neutral) > score(&slowed_other, "hello", &neutral) + 1.0);
        // User who listens to slowed does NOT suffer the odd-version penalty.
        assert!(score(&slowed_other, "hello", &taste) > score(&slowed_other, "hello", &neutral) + 1.0);
        // Explicitly typing "karaoke" boosts karaoke version above studio & plain query.
        let kar = tr("Hello (Karaoke)", "Artist");
        assert!(score(&kar, "hello karaoke", &neutral) > score(&normal, "hello karaoke", &neutral));
        assert!(score(&kar, "hello karaoke", &neutral) > score(&kar, "hello", &neutral));
        // Word boundary: "Discover" is NOT penalized as a cover!
        let disc = tr("Discover", "Artist");
        let plain = tr("Discolor", "Artist");
        assert!((score(&disc, "discover", &neutral) - score(&plain, "discolor", &neutral)).abs() < 0.01);
    }

    #[test]
    fn skipped_track_start_does_not_get_phantom_positive_affinity_from_recent_or_plays() {
        let mut lib = Library::default();
        let now = 1_750_000_000;
        let skipped = tr("Unwanted Autoplay", "Random Autoplay Artist");

        // Simulate real Core::start_track_with followed by Core::next(false) after 6s:
        lib.push_recent(skipped.clone());
        lib.record_play(&skipped);
        lib.record_skip(&skipped);
        lib.taste.record_playback_outcome(&skipped, 6_000, Some(200_000), true, false, 15, now);

        let taste = Taste::from_library_at(&lib, now);
        assert!(
            taste.artist_affinity("Random Autoplay Artist") < -0.3,
            "expected negative affinity for early-skipped artist, got {}",
            taste.artist_affinity("Random Autoplay Artist")
        );
        assert_eq!(taste.artist_weight("Random Autoplay Artist"), 0.0);

        let prof = build_profile(&lib, &TasteConfig::default(), false, now, 15);
        assert!(!prof.liked_artists.iter().any(|a| a.name == "Random Autoplay Artist"));
    }

    #[test]
    fn the_weeknd_and_tyler_the_creator_resolve_builtin_genres_and_preserve_casing() {
        assert_eq!(
            credited_named("Tyler, The Creator"),
            vec![("tyler".to_string(), "Tyler, The Creator".to_string())]
        );
        let mut lib = Library::default();
        let now = 1_750_000_000;
        lib.taste.set_onboarding(
            vec!["The Weeknd".into(), "Tyler, The Creator".into()],
            vec![],
            vec![],
        );
        // Also simulate lowercase artist_plays entry alongside mixed-case artist_signals:
        let mit = tr("Show Me How", "Men I Trust");
        lib.record_play(&mit);
        lib.taste.record_playback_outcome(&mit, 200_000, Some(200_000), false, true, 21, now);
        // Simulate empty server tag response (e.g. keyless server / upstream miss):
        lib.taste.merge_artist_tags(
            &[ArtistTags {
                artist: "The Weeknd".into(),
                genres: vec![],
                moods: vec![],
                tags: vec![],
                similar: vec!["Bruno Mars".into()],
            }],
            now,
        );

        let prof = build_profile(&lib, &TasteConfig::default(), false, now, 21);
        assert!(prof.liked_artists.iter().any(|a| a.name == "Tyler, The Creator" && a.genres.contains(&"Hip-Hop".to_string())));
        assert!(prof.liked_artists.iter().any(|a| a.name == "The Weeknd" && a.genres.contains(&"Pop".to_string())));
        assert!(prof.top_genres.iter().any(|g| g.name == "Synthpop"));
        assert!(prof.liked_artists.iter().any(|a| a.name == "Men I Trust"));
        assert!(!prof.liked_artists.iter().any(|a| a.name == "The Creator"));
    }
}
