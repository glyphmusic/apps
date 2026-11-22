//! App-layer orchestration for Glyph's hybrid taste engine:
//! - Visible Taste Profile snapshots
//! - Onboarding starter catalog & favorite artist/song calibration
//! - Background tag enrichment & optional server collaborative filtering
//! - Explicit negative/positive feedback for tracks, genres/moods, and versions

use crate::controller::Core;
use glyph_core::config::TasteConfig;
use glyph_core::taste::{self, Taste};
use glyph_protocol::taste::TasteCollabRequest;
use glyph_protocol::Track;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

type Res<T = ()> = Result<T, String>;
type C<'a> = &'a Arc<Core>;

const COLLAB_TTL_S: u64 = 2 * 3600;

fn now_s() -> u64 {
    glyph_core::paths::now_unix()
}

fn current_hour() -> u8 {
    taste::local_hour()
}

pub fn today_key() -> String {
    taste::local_date_key()
}

fn server_taste_available(core: &Core) -> bool {
    core.backend().is_some_and(|b| b.server_taste_available())
}

/// Returns the complete visible Taste Profile snapshot for the UI.
pub fn taste_profile(core: C) -> Value {
    let server_ok = server_taste_available(core);
    let st = core.st();
    let prof = taste::build_profile(
        &st.library,
        &st.config.taste,
        server_ok,
        now_s(),
        current_hour(),
    );
    serde_json::to_value(prof).unwrap_or_else(|_| json!({}))
}

/// Returns the current Daily Mixes (building locally on demand if none are cached yet).
pub fn taste_mixes(core: C) -> Value {
    let mut st = core.st();
    if !st.config.taste.enabled {
        return json!([]);
    }
    if st.feeds.mixes.is_empty() {
        let taste = Taste::from_library(&st.library);
        let hide = st.config.audio.hide_explicit;
        let pool: Vec<Track> = st
            .feeds
            .recs
            .iter()
            .chain(st.feeds.trending.iter())
            .cloned()
            .collect();
        st.feeds.mixes = taste::build_daily_mixes(
            &st.library,
            &taste,
            &pool,
            &today_key(),
            current_hour(),
            hide,
        );
    }
    serde_json::to_value(&st.feeds.mixes).unwrap_or_else(|_| json!([]))
}

/// Runs bounded metadata tag enrichment and (when opted in) anonymized
/// server collaborative filtering. Respects all user privacy toggles.
pub async fn enrich_and_sync(core: &Arc<Core>, force: bool) {
    if crate::storage::is_offline() {
        return;
    }
    let Some(b) = core.backend() else { return };
    let (cfg, missing_tags, collab_req, need_collab) = {
        let st = core.st();
        let cfg = st.config.taste.clone();
        if !cfg.enabled {
            return;
        }
        let now = now_s();
        let taste = Taste::from_library_at(&st.library, now);
        let missing = if cfg.metadata_enrichment {
            taste.missing_tag_artists(&st.library, now, 18)
        } else {
            Vec::new()
        };
        let collab_stale = force
            || st
                .library
                .taste
                .collab_cache
                .as_ref()
                .is_none_or(|c| now.saturating_sub(c.fetched_at_s) >= COLLAB_TTL_S);
        let need_collab = cfg.server_collab && collab_stale && !taste.is_empty();
        let req = if need_collab {
            Some(TasteCollabRequest {
                positive: taste.top_positive_signals(30),
                negative: taste.negative_artists(25),
                avoided_tags: taste.avoided_tags(15),
                limit: Some(15),
                contribute: true,
            })
        } else {
            None
        };
        (cfg, missing, req, need_collab)
    };

    let mut changed = false;
    let now = now_s();

    // 1. Metadata tag enrichment (server cache when server_collab is on, else local direct Deezer).
    if !missing_tags.is_empty() {
        if let Ok(Ok(items)) = tokio::time::timeout(
            Duration::from_secs(8),
            b.taste_tags(missing_tags, cfg.server_collab),
        )
        .await
        {
            if !items.is_empty() {
                core.st().library.taste.merge_artist_tags(&items, now);
                changed = true;
            }
        }
    }

    // 2. Optional server collaborative filtering (strictly skipped if user opted out).
    if need_collab {
        if let Some(req) = collab_req {
            if let Ok(Ok(Some(resp))) =
                tokio::time::timeout(Duration::from_secs(8), b.taste_collab(req)).await
            {
                // Re-check opt-out in case the user toggled it mid-flight.
                let mut st = core.st();
                if st.config.taste.enabled && st.config.taste.server_collab {
                    st.library.taste.set_collab_cache(resp, now);
                    changed = true;
                }
            }
        }
    } else if !cfg.server_collab {
        let mut st = core.st();
        if st.library.taste.collab_cache.is_some() {
            st.library.taste.clear_collab_cache();
            changed = true;
        }
    }

    if changed {
        core.library_changed_soon();
    }
}

/// Forces a refresh of tags, collaborative recs, Daily Mixes, and Home feeds.
pub async fn taste_refresh(core: C<'_>) -> Res<Value> {
    enrich_and_sync(core, true).await;
    crate::commands::reload_feeds(core);
    Ok(taste_profile(core))
}

/// Fetches genre-diverse starter artists & tracks for the onboarding picker.
pub async fn taste_onboarding_catalog(core: C<'_>) -> Res<Value> {
    let catalog = match core.backend() {
        Some(b) => tokio::time::timeout(Duration::from_secs(6), b.taste_onboarding())
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_else(fallback_onboarding_catalog),
        None => fallback_onboarding_catalog(),
    };
    serde_json::to_value(catalog).map_err(|e| e.to_string())
}

fn fallback_onboarding_catalog() -> glyph_protocol::OnboardingCatalogResponse {
    const STARTERS: &[(&str, &str)] = &[
        ("Tame Impala", "Indie"),
        ("Arctic Monkeys", "Indie Rock"),
        ("Men I Trust", "Dream Pop"),
        ("Daft Punk", "Electronic"),
        ("Fred again..", "Electronic"),
        ("Kaytranada", "House"),
        ("Kendrick Lamar", "Hip-Hop"),
        ("Tyler, The Creator", "Hip-Hop"),
        ("Travis Scott", "Trap"),
        ("Frank Ocean", "R&B"),
        ("SZA", "R&B"),
        ("The Weeknd", "Pop"),
        ("Billie Eilish", "Pop"),
        ("Dua Lipa", "Pop"),
        ("Radiohead", "Alternative"),
        ("Linkin Park", "Rock"),
        ("Metallica", "Metal"),
        ("Deftones", "Metal"),
        ("Miles Davis", "Jazz"),
        ("Nujabes", "Lo-Fi"),
        ("Lana Del Rey", "Dream Pop"),
        ("Bon Iver", "Folk & Acoustic"),
        ("Bad Bunny", "Latin"),
        ("Burna Boy", "Afrobeats"),
        ("Hans Zimmer", "Soundtrack"),
        ("Ludovico Einaudi", "Classical"),
        ("Gorillaz", "Alternative"),
        ("Flume", "Electronic"),
    ];
    let mut genres = Vec::new();
    let mut artists = Vec::with_capacity(STARTERS.len());
    for &(name, genre) in STARTERS {
        if !genres.iter().any(|g: &String| g == genre) {
            genres.push(genre.to_string());
        }
        artists.push(glyph_protocol::OnboardingArtist {
            name: name.to_string(),
            genre: genre.to_string(),
            photo: None,
            sample_track: None,
        });
    }
    glyph_protocol::OnboardingCatalogResponse {
        genres,
        artists,
        tracks: Vec::new(),
    }
}

/// Saves onboarding favorites (artists, songs, genres), triggers enrichment
/// and feed refresh, and returns the updated Taste Profile.
pub async fn taste_onboarding_save(
    core: C<'_>,
    artists: Vec<String>,
    tracks: Vec<Track>,
    genres: Vec<String>,
    follow_artists: Option<bool>,
) -> Res<Value> {
    {
        let mut st = core.st();
        st.library
            .taste
            .set_onboarding(artists.clone(), tracks, genres);
        if follow_artists == Some(true) {
            for a in &artists {
                if !st.library.is_following(a) {
                    st.library.toggle_follow(a, None);
                }
            }
        }
    }
    core.library_changed();
    let core_bg = core.clone();
    crate::rt::spawn(async move {
        enrich_and_sync(&core_bg, true).await;
        crate::commands::reload_feeds(&core_bg);
    });
    Ok(taste_profile(core))
}

/// Marks or unmarks a single track as "Not interested / Disliked".
pub fn taste_dislike_track(core: C, track: Track, disliked: bool) -> Value {
    {
        let mut st = core.st();
        st.library.taste.set_disliked_track(&track, disliked);
        if disliked {
            let k = glyph_core::dedupe::track_key(&track);
            st.feeds
                .recs
                .retain(|t| glyph_core::dedupe::track_key(t) != k);
            for mix in &mut st.feeds.mixes {
                mix.tracks
                    .retain(|t| glyph_core::dedupe::track_key(t) != k);
            }
        }
    }
    core.library_changed();
    crate::commands::reload_feeds(core);
    taste_profile(core)
}

/// Sets explicit like (`+1`), dislike (`-1`), or neutral (`None`) for a genre or mood.
pub fn taste_tag_feedback(core: C, tag: String, preference: Option<i8>) -> Value {
    {
        let mut st = core.st();
        st.library.taste.set_tag_feedback(&tag, preference);
    }
    core.library_changed();
    crate::commands::reload_feeds(core);
    taste_profile(core)
}

/// Sets explicit prefer (`+1`), avoid (`-1`), or neutral (`None`) for a version/style.
pub fn taste_version_feedback(core: C, style: String, preference: Option<i8>) -> Value {
    {
        let mut st = core.st();
        st.library.taste.set_version_feedback(&style, preference);
    }
    core.library_changed();
    crate::commands::reload_feeds(core);
    taste_profile(core)
}

/// Resets learned taste signals and explicit taste preferences.
pub fn taste_reset(core: C) -> Value {
    {
        let mut st = core.st();
        st.library.taste.reset();
        st.library.recommendation_feedback.clear();
        st.feeds.mixes.clear();
    }
    core.library_changed();
    crate::commands::reload_feeds(core);
    taste_profile(core)
}

/// Hook called when `config.taste` changes in Settings.
pub fn on_taste_config_changed(core: &Arc<Core>, old: &TasteConfig, new: &TasteConfig) {
    if old == new {
        return;
    }
    if !new.server_collab || !new.enabled {
        let mut cleared = false;
        {
            let mut st = core.st();
            if st.library.taste.collab_cache.is_some() {
                st.library.taste.clear_collab_cache();
                cleared = true;
            }
            if !new.enabled {
                st.feeds.mixes.clear();
            }
        }
        if cleared {
            core.library_changed();
        }
    }
    if new.enabled && new.server_collab && (!old.server_collab || !old.enabled) {
        let core_bg = core.clone();
        crate::rt::spawn(async move {
            enrich_and_sync(&core_bg, true).await;
            crate::commands::reload_feeds(&core_bg);
        });
    } else {
        crate::commands::reload_feeds(core);
    }
}
