//! Local multi-source explicit rating fallback (used only when search runs
//! without a Glyph server that provides `ratings`). Mirrors the server pipeline:
//! title markers + Deezer (`explicit_lyrics` & `explicit_content_lyrics`) +
//! Apple Music / iTunes (`trackExplicitness`) + LRCLIB lyrics profanity scan.

use crate::provider::{send_json, MusicProvider};
use crate::providers::deezer::Deezer;
use crate::providers::lrclib::LrcLib;
use crate::resolver::best_candidate;
use futures::stream::{self, StreamExt};
use glyph_protocol::{ExplicitRating, ProviderId, Track};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MIN_MATCH: f32 = 0.7;
const MAX_LOOKUPS: usize = 20;
const PARALLEL: usize = 5;
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(4);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(6);
const KNOWN_TTL_S: u64 = 90 * 86_400;
const UNKNOWN_TTL_S: u64 = 7 * 86_400;
const MAX_ENTRIES: usize = 5000;
const CACHE_FILE: &str = "explicit-ratings-v1.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalRating {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deezer_id: Option<String>,
    pub rating: ExplicitRating,
    #[serde(default)]
    pub saved_at_s: u64,
}

/// Song key -> cached multi-source rating (persisted to disk under `cache_dir`).
pub type RatingCache = Arc<Mutex<HashMap<String, LocalRating>>>;

fn now_s() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn is_fresh(entry: &LocalRating, now: u64) -> bool {
    if entry.saved_at_s == 0 {
        return true;
    }
    let ttl = if entry.rating == ExplicitRating::Unknown { UNKNOWN_TTL_S } else { KNOWN_TTL_S };
    now.saturating_sub(entry.saved_at_s) < ttl
}

/// Loads persisted explicit ratings from `cache_dir/explicit-ratings-v1.json`.
pub fn load_cache() -> RatingCache {
    let path = crate::paths::cache_dir().join(CACHE_FILE);
    let now = now_s();
    let mut map: HashMap<String, LocalRating> = crate::paths::read_json(&path).unwrap_or_default();
    map.retain(|_, v| is_fresh(v, now));
    Arc::new(Mutex::new(map))
}

/// Persists the explicit-rating cache atomically in the background.
pub fn save_cache(cache: &RatingCache) {
    let now = now_s();
    let snap: HashMap<String, LocalRating> = {
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        c.retain(|_, v| is_fresh(v, now));
        if c.len() > MAX_ENTRIES {
            let mut by_age: Vec<(String, u64)> = c.iter().map(|(k, v)| (k.clone(), v.saved_at_s)).collect();
            by_age.sort_by_key(|(_, ts)| *ts);
            let drop_n = c.len() - MAX_ENTRIES;
            for (k, _) in by_age.into_iter().take(drop_n) {
                c.remove(&k);
            }
        }
        c.clone()
    };
    let path = crate::paths::cache_dir().join(CACHE_FILE);
    std::thread::spawn(move || {
        let _ = crate::paths::write_json(&path, &snap);
    });
}

/// Applies locally cached explicit ratings onto `tracks` (0 network calls).
pub fn apply_cached(cache: &RatingCache, tracks: &mut [Track]) {
    if tracks.is_empty() {
        return;
    }
    let now = now_s();
    let c = cache.lock().unwrap_or_else(|e| e.into_inner());
    if c.is_empty() {
        return;
    }
    for t in tracks.iter_mut() {
        if t.rating() == ExplicitRating::Explicit {
            continue;
        }
        let key = crate::dedupe::track_key(t);
        if let Some(hit) = c.get(&key).filter(|e| is_fresh(e, now)) {
            if let Some(updated) = apply(t, hit) {
                *t = updated;
            }
        }
    }
}

/// Remembers known explicit ratings from tracks (server search, artist catalog, etc.)
/// into the local persistent cache without ever downgrading `Explicit`.
pub fn remember_tracks(cache: &RatingCache, tracks: &[Track]) {
    if tracks.is_empty() {
        return;
    }
    let now = now_s();
    let mut dirty = false;
    {
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        for t in tracks {
            let r = t.rating();
            if r == ExplicitRating::Unknown && t.explicit_rating != Some(ExplicitRating::Unknown) {
                continue;
            }
            let key = crate::dedupe::track_key(t);
            let dz_id = if t.source == ProviderId::Deezer {
                Some(t.id.clone())
            } else {
                t.external_ids.get(&ProviderId::Deezer).cloned()
            };
            match c.get_mut(&key) {
                Some(cur) if is_fresh(cur, now) => {
                    let merged = cur.rating.merge(r);
                    if merged != cur.rating || (cur.deezer_id.is_none() && dz_id.is_some()) {
                        cur.rating = merged;
                        if cur.deezer_id.is_none() {
                            cur.deezer_id = dz_id;
                        }
                        cur.saved_at_s = now;
                        dirty = true;
                    }
                }
                _ => {
                    c.insert(key, LocalRating { deezer_id: dz_id, rating: r, saved_at_s: now });
                    dirty = true;
                }
            }
        }
    }
    if dirty {
        save_cache(cache);
    }
}

/// Scans fetched lyrics and records the resulting explicit rating in the local persistent cache.
pub fn remember_lyrics(cache: &RatingCache, track: &Track, text: &str, instrumental: bool) {
    let r = if lyrics_have_explicit(text) {
        ExplicitRating::Explicit
    } else if instrumental || !text.trim().is_empty() {
        ExplicitRating::Clean
    } else {
        return;
    };
    let now = now_s();
    let key = crate::dedupe::track_key(track);
    let mut dirty = false;
    {
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        match c.get_mut(&key) {
            Some(cur) if is_fresh(cur, now) => {
                let merged = cur.rating.merge(r);
                if merged != cur.rating {
                    cur.rating = merged;
                    cur.saved_at_s = now;
                    dirty = true;
                }
            }
            _ => {
                c.insert(key, LocalRating { deezer_id: None, rating: r, saved_at_s: now });
                dirty = true;
            }
        }
    }
    if dirty {
        save_cache(cache);
    }
}

const EXACT_EXPLICIT: &[&str] = &[
    "fuck", "fucked", "fucker", "fuckers", "fucking", "fuckin", "fucks", "fucka",
    "motherfucker", "motherfuckers", "motherfucking", "motherfuckin", "motherfucka",
    "muthafucka", "muthafuckas", "muthafuckin", "muthafucking",
    "shit", "shits", "shitty", "shitting", "shittin", "shitted",
    "bullshit", "horseshit", "apeshit", "dipshit", "batshit", "shithead", "shitheads",
    "bitch", "bitches", "bitching", "bitchin", "bitchy", "bitched",
    "cunt", "cunts", "whore", "whores", "slut", "sluts", "slutty",
    "asshole", "assholes", "arsehole", "arseholes",
    "dickhead", "dickheads", "cocksucker", "cocksuckers",
    "pussy", "pussies", "nigga", "niggas", "nigger", "niggers",
    "faggot", "faggots", "twat", "twats", "wanker", "wankers",
    "blowjob", "blowjobs", "handjob", "cumshot", "jizz", "dildo", "dildos",
    "skank", "skanks", "thot", "thots",
    "fick", "ficken", "fickt", "fickte", "fickst", "gefickt",
    "verfickt", "verfickte", "verfickten", "verfickter", "verficktes",
    "abgefuckt", "abgefuckte", "abgefuckten",
    "hurensohn", "hurensöhne", "hurensohne",
    "fotze", "fotzen", "wichser", "wixer", "wichsen", "gewichst",
    "arschloch", "arschlöcher", "arschlocher",
    "missgeburt", "missgeburten", "nutte", "nutten",
    "schlampe", "schlampen", "hure", "huren",
    "scheiße", "scheisse", "scheiß", "scheiss", "scheißdreck", "scheissdreck",
    "scheißegal", "scheissegal", "beschissen", "beschissene", "beschissenen",
    "bastard", "bastarde", "spast", "spasti", "spasten", "schwanzlutscher", "bumsen",
    "бля", "блядь", "блять", "бляди", "блядство", "блядский", "блядская",
    "сука", "суки", "сучка", "сучки",
    "хуй", "хуя", "хуе", "хуйня", "хуесос", "хуёвый", "хуевый", "хуево", "хуёво",
    "нахуй", "похуй", "охуеть", "ахуеть", "охуенно", "ахуенно",
    "пизда", "пиздец", "пизды", "пизду", "пиздить", "пиздит", "пиздатый",
    "ебать", "ебал", "ебала", "ебаный", "ёбаный", "еблан", "ебанутый",
    "уебан", "уёбок", "уебок", "долбоеб", "долбоёб",
    "заебал", "заебали", "заебись", "выебать", "съебал",
    "пидор", "пидорас", "пидарас", "гандон", "мудак", "мудаки", "шлюха", "шлюхи",
    "puta", "putas", "puto", "putos", "putazo", "hijueputa",
    "mierda", "mierdas", "joder", "jodido", "jodida", "coño",
    "cabrón", "cabron", "cabrones", "chingar", "chinga", "chingada", "chingado", "chingón",
    "pendejo", "pendejos", "verga", "vergas", "culero", "culeros", "maricón", "maricon",
    "putain", "merde", "enculé", "encule", "enculer", "salope", "salopes",
    "connard", "connards", "connasse", "connasses",
    "nique", "niquer", "niqué", "pute", "putes", "couilles", "branleur",
];

const EXPLICIT_PREFIXES: &[&str] = &[
    "motherfuck", "muthafuck", "fuck", "bullshit",
    "hurensohn", "arschloch", "verfickt", "scheiß", "scheiss", "beschissen",
    "пиздец", "нахуй", "похуй", "долбоеб", "долбоёб",
];

pub fn lyrics_have_explicit(text: &str) -> bool {
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.len() < 3 {
            continue;
        }
        let w = raw.to_lowercase();
        if EXACT_EXPLICIT.contains(&w.as_str()) || EXPLICIT_PREFIXES.iter().any(|p| w.starts_with(p)) {
            return true;
        }
    }
    false
}

fn title_marks_explicit(title: &str) -> bool {
    let low = title.to_lowercase();
    if low.contains("non-explicit") || low.contains("non explicit") || low.contains("clean") || low.contains("radio edit") {
        return false;
    }
    let mut depth = 0usize;
    let mut inner = String::new();
    for c in low.chars() {
        match c {
            '(' | '[' => {
                if depth == 0 {
                    inner.clear();
                }
                depth += 1;
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if inner.split(|ch: char| !ch.is_alphanumeric()).any(|w| w == "explicit") {
                        return true;
                    }
                }
            }
            _ if depth > 0 => inner.push(c),
            _ => {}
        }
    }
    false
}

/// Applies `hit` onto `t`. Returns `Some(updated)` only when `t` changed.
pub fn apply(t: &Track, hit: &LocalRating) -> Option<Track> {
    let merged = t.rating().merge(hit.rating);
    let mut out = t.clone();
    if let Some(id) = &hit.deezer_id {
        if out.source != ProviderId::Deezer {
            out.external_ids.entry(ProviderId::Deezer).or_insert_with(|| id.clone());
        }
    }
    if merged != out.rating() || (merged == ExplicitRating::Explicit && !out.explicit) {
        out.set_rating(merged);
    }
    (out != *t).then_some(out)
}

#[derive(Deserialize)]
struct ItunesResp {
    #[serde(default)]
    results: Vec<ItunesSong>,
}

#[derive(Deserialize)]
struct ItunesSong {
    #[serde(rename = "trackName", default)]
    track_name: String,
    #[serde(rename = "artistName", default)]
    artist_name: String,
    #[serde(rename = "trackTimeMillis", default)]
    track_time_millis: Option<u64>,
    #[serde(rename = "trackExplicitness")]
    track_explicitness: Option<String>,
    #[serde(rename = "collectionExplicitness")]
    collection_explicitness: Option<String>,
}

async fn lookup_itunes(http: &reqwest::Client, t: &Track) -> Option<Option<ExplicitRating>> {
    let q = format!("{} {}", t.artist, t.title);
    let resp: ItunesResp = tokio::time::timeout(
        LOOKUP_TIMEOUT,
        send_json(http.get("https://itunes.apple.com/search").query(&[
            ("term", q.trim()),
            ("media", "music"),
            ("entity", "song"),
            ("limit", "6"),
        ])),
    )
    .await
    .ok()?
    .ok()?;
    let want_clean = {
        let low = t.title.to_lowercase();
        low.contains("radio edit") || low.contains("(clean") || low.contains("[clean") || low.contains("clean version") || low.contains("censored")
    };
    let mut any_explicit = false;
    let mut best_rating: Option<ExplicitRating> = None;
    let mut best_sc = 0.0f32;
    for item in resp.results {
        if item.track_name.trim().is_empty() || item.artist_name.trim().is_empty() {
            continue;
        }
        let cand = Track {
            id: "itunes".into(),
            source: ProviderId::MusicBrainz,
            title: item.track_name,
            artist: item.artist_name,
            album: None,
            duration_ms: item.track_time_millis,
            cover_url: None,
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false,
            explicit_rating: None,
        };
        let Some((_, sc)) = best_candidate(t, vec![crate::provider::Candidate { track: cand, bonus: 0.0 }]) else {
            continue;
        };
        if sc < MIN_MATCH {
            continue;
        }
        let ex = item.track_explicitness.as_deref().or(item.collection_explicitness.as_deref()).unwrap_or("");
        let r = match ex {
            "explicit" => ExplicitRating::Explicit,
            "notExplicit" | "cleaned" => ExplicitRating::Clean,
            _ => continue,
        };
        if r == ExplicitRating::Explicit {
            any_explicit = true;
        }
        if sc > best_sc {
            best_sc = sc;
            best_rating = Some(r);
        }
    }
    if !want_clean && any_explicit {
        return Some(Some(ExplicitRating::Explicit));
    }
    Some(best_rating)
}

async fn look_up(http: &reqwest::Client, dz: &Deezer, lrc: &LrcLib, t: &Track) -> Option<LocalRating> {
    if title_marks_explicit(&t.title) {
        return Some(LocalRating { deezer_id: None, rating: ExplicitRating::Explicit, saved_at_s: now_s() });
    }
    let need_dz = !t.sources().contains(&ProviderId::Deezer) || t.explicit_rating == Some(ExplicitRating::Unknown);
    let dz_fut = async {
        if !need_dz {
            return Some(None);
        }
        let query = format!("{} {}", t.artist, t.title);
        let cands = tokio::time::timeout(LOOKUP_TIMEOUT, dz.search_candidates(&query)).await.ok()?.ok()?;
        Some(best_candidate(t, cands).filter(|(_, c)| *c >= MIN_MATCH).map(|(c, _)| {
            let rating = c.track.rating();
            (c.track.id, rating)
        }))
    };
    let it_fut = lookup_itunes(http, t);
    let (dz_res, it_res) = tokio::join!(dz_fut, it_fut);

    let mut any_ok = false;
    let mut deezer_id = None;
    let mut rating = ExplicitRating::Unknown;
    if let Some(hit) = dz_res {
        any_ok = true;
        if let Some((id, r)) = hit {
            deezer_id = Some(id);
            rating = rating.merge(r);
        }
    }
    if let Some(hit) = it_res {
        any_ok = true;
        if let Some(r) = hit {
            rating = rating.merge(r);
        }
    }
    if rating == ExplicitRating::Explicit {
        return Some(LocalRating { deezer_id, rating, saved_at_s: now_s() });
    }
    if let Ok(Ok(Some(ly))) = tokio::time::timeout(
        Duration::from_secs(3),
        lrc.lookup(&t.title, &t.artist, t.album.as_deref(), t.duration_ms),
    )
    .await
    {
        any_ok = true;
        let mut text = ly.plain.unwrap_or_default();
        for l in &ly.synced {
            text.push('\n');
            text.push_str(&l.text);
        }
        if lyrics_have_explicit(&text) {
            rating = ExplicitRating::Explicit;
        } else if ly.instrumental || !text.trim().is_empty() {
            rating = rating.merge(ExplicitRating::Clean);
        }
    }
    any_ok.then_some(LocalRating { deezer_id, rating, saved_at_s: now_s() })
}

/// Rates unrated and unverified songs locally (only used when no server rates).
pub async fn rate_unknown(http: &reqwest::Client, cache: &RatingCache, tracks: &[Track], mut found: impl FnMut(Track)) {
    let now = now_s();
    let mut todo_unknown = Vec::new();
    let mut todo_clean = Vec::new();
    for t in tracks.iter().filter(|t| t.rating() != ExplicitRating::Explicit) {
        let key = crate::dedupe::track_key(t);
        match cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key).filter(|e| is_fresh(e, now)).cloned() {
            Some(hit) => {
                if let Some(rated) = apply(t, &hit) {
                    found(rated);
                }
            }
            None => {
                if t.explicit_unknown() {
                    todo_unknown.push((key, t));
                } else {
                    todo_clean.push((key, t));
                }
            }
        }
    }
    let mut todo: Vec<_> = todo_unknown.into_iter().take(MAX_LOOKUPS).collect();
    let rem = MAX_LOOKUPS.saturating_sub(todo.len());
    if rem > 0 {
        todo.extend(todo_clean.into_iter().take(rem));
    }
    if todo.is_empty() {
        return;
    }
    let dz = Deezer::new(http.clone());
    let lrc = LrcLib::new(http.clone());
    let pending: Vec<_> = todo
        .into_iter()
        .map(|(key, t)| {
            let dz = &dz;
            let lrc = &lrc;
            async move { (key, t, look_up(http, dz, lrc, t).await) }
        })
        .collect();
    let mut lookups = stream::iter(pending).buffer_unordered(PARALLEL);
    let deadline = tokio::time::Instant::now() + TOTAL_TIMEOUT;
    let mut dirty = false;
    while let Ok(Some((key, t, res))) = tokio::time::timeout_at(deadline, lookups.next()).await {
        let Some(hit) = res else { continue };
        if let Some(rated) = apply(t, &hit) {
            found(rated);
        }
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        c.insert(key, hit);
        dirty = true;
    }
    if dirty {
        save_cache(cache);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deezer_match_rates_the_upload() {
        let t: Track = serde_json::from_str(r#"{"id":"v","source":"youtube","title":"t","artist":"a"}"#).unwrap();
        assert!(apply(&t, &LocalRating { deezer_id: None, rating: ExplicitRating::Unknown, saved_at_s: 0 }).is_none());
        let r = apply(&t, &LocalRating { deezer_id: Some("42".into()), rating: ExplicitRating::Explicit, saved_at_s: 0 }).unwrap();
        assert!(r.explicit && !r.explicit_unknown() && r.id == "v" && r.source == ProviderId::YouTube);
        let r = apply(&t, &LocalRating { deezer_id: Some("42".into()), rating: ExplicitRating::Clean, saved_at_s: 0 }).unwrap();
        assert!(!r.maybe_explicit());
    }
}
