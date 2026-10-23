//! Explicit rating for YouTube uploads: YouTube does not say whether a song
//! is explicit, so search results no rating source knows are looked up at
//! Deezer ("Artist Title", scored like the resolver). Only songs that are not
//! found there keep the "?" ([`Track::explicit_unknown`]).

use crate::provider::MusicProvider;
use crate::providers::deezer::Deezer;
use crate::resolver::best_candidate;
use futures::stream::{self, StreamExt};
use glyph_protocol::{ProviderId, Track};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A Deezer match below this is not trusted to rate the song.
const MIN_MATCH: f32 = 0.7;
/// Songs looked up per search, lookups at once, time per lookup.
const MAX_LOOKUPS: usize = 25;
const PARALLEL: usize = 6;
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(4);
/// All lookups of one search together.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(6);

/// Song key -> Deezer id + explicit, `None` = not found (stays "?").
pub type RatingCache = Arc<Mutex<HashMap<String, Option<(String, bool)>>>>;

/// `t` with the Deezer match's id and rating, or `None` when there is none.
pub fn apply(t: &Track, hit: &Option<(String, bool)>) -> Option<Track> {
    let (id, explicit) = hit.as_ref()?;
    let mut out = t.clone();
    out.external_ids.insert(ProviderId::Deezer, id.clone());
    out.explicit = *explicit;
    Some(out)
}

async fn look_up(dz: &Deezer, t: &Track) -> Option<Option<(String, bool)>> {
    let query = format!("{} {}", t.artist, t.title);
    let cands = tokio::time::timeout(LOOKUP_TIMEOUT, dz.search_candidates(&query)).await.ok()?.ok()?;
    Some(best_candidate(t, cands).filter(|(_, c)| *c >= MIN_MATCH).map(|(c, _)| (c.track.id, c.track.explicit)))
}

/// Rates the unrated YouTube songs in `tracks`; each one that Deezer knows is
/// handed to `found` (same `source` + `id`, now with a Deezer id and rating).
/// Failed lookups are not remembered, so the next search asks again.
pub async fn rate_unknown(http: &reqwest::Client, cache: &RatingCache, tracks: &[Track], mut found: impl FnMut(Track)) {
    let mut todo = Vec::new();
    for t in tracks.iter().filter(|t| t.explicit_unknown()) {
        let key = crate::dedupe::track_key(t);
        match cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            Some(hit) => found_or_skip(t, hit, &mut found),
            None if todo.len() < MAX_LOOKUPS => todo.push((key, t)),
            None => {}
        }
    }
    if todo.is_empty() {
        return;
    }
    let dz = Deezer::new(http.clone());
    let mut lookups = stream::iter(todo).map(|(key, t)| {
        let dz = &dz;
        async move { (key, t, look_up(dz, t).await) }
    }).buffer_unordered(PARALLEL);
    // The search only ends after this: give up on what is still open.
    let deadline = tokio::time::Instant::now() + TOTAL_TIMEOUT;
    while let Ok(Some((key, t, res))) = tokio::time::timeout_at(deadline, lookups.next()).await {
        let Some(hit) = res else { continue };
        found_or_skip(t, &hit, &mut found);
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        if c.len() > 5000 {
            c.clear();
        }
        c.insert(key, hit);
    }
}

fn found_or_skip(t: &Track, hit: &Option<(String, bool)>, found: &mut impl FnMut(Track)) {
    if let Some(rated) = apply(t, hit) {
        found(rated);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deezer_match_rates_the_upload() {
        let t: Track = serde_json::from_str(r#"{"id":"v","source":"youtube","title":"t","artist":"a"}"#).unwrap();
        assert!(apply(&t, &None).is_none());
        let r = apply(&t, &Some(("42".into(), true))).unwrap();
        assert!(r.explicit && !r.explicit_unknown() && r.id == "v" && r.source == ProviderId::YouTube);
        let r = apply(&t, &Some(("42".into(), false))).unwrap();
        assert!(!r.maybe_explicit());
    }
}
