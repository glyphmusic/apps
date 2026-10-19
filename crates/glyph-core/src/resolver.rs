//! Unified search + resolver.
//!
//! * Search: all enabled providers in parallel (4 s timeout each), results
//!   stream in as they arrive and duplicates are merged (normalised
//!   title + artist, duration ±3 s) into one track with a source map in
//!   `external_ids`.
//! * Resolve: best legal full source by priority. Metadata-only songs
//!   (Spotify, Last.fm, files) are searched as "Artist - Title" at the full
//!   sources, scored (ISRC > title/artist/duration, "Official Audio", Topic
//!   channels) and the mapping is cached persistently. Without a full source a
//!   30 s preview is used.
//! * Failing / quota-exhausted providers are skipped for 10 minutes.

use crate::dedupe::{artist_similarity, title_similarity, MergeOutcome, Merger};
use crate::paths;
use crate::provider::*;
use crate::providers::Providers;
use crate::Resolved;
use anyhow::Result;
use futures::stream::{FuturesUnordered, StreamExt};
use glyph_protocol::{PlaybackSource, ProviderId, SearchEvent, Track};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

/// Below this confidence a match is not used.
pub const MIN_CONFIDENCE: f32 = 0.5;
/// At/above this a match counts as "sure" (import) and YouTube is not needed.
pub const SURE_CONFIDENCE: f32 = 0.85;
/// Timeout for calls that only build a playback source.
const SOURCE_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_MAPPINGS: usize = 5000;

// ------------------------------------------------------------ mapping cache

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Mapping {
    provider: ProviderId,
    track: Track,
    confidence: f32,
    ts: u64,
}

/// Persistent mapping "song -> track at a full source" (JSON in the data dir).
pub struct MappingCache {
    path: Option<PathBuf>,
    map: Mutex<HashMap<String, Mapping>>,
    unsaved: AtomicUsize,
}

impl MappingCache {
    pub fn load(path: Option<PathBuf>) -> Self {
        let map = path
            .as_ref()
            .and_then(|p| paths::read_json::<HashMap<String, Mapping>>(p))
            .unwrap_or_default();
        Self { path, map: Mutex::new(map), unsaved: AtomicUsize::new(0) }
    }

    fn get(&self, keys: &[String]) -> Option<(String, Mapping)> {
        let m = self.map.lock().unwrap();
        keys.iter().find_map(|k| m.get(k).map(|v| (k.clone(), v.clone())))
    }

    fn remove(&self, key: &str) {
        self.map.lock().unwrap().remove(key);
        self.unsaved.fetch_add(1, Ordering::Relaxed);
    }

    fn insert(&self, keys: &[String], mapping: Mapping) {
        {
            let mut m = self.map.lock().unwrap();
            for k in keys {
                m.insert(k.clone(), mapping.clone());
            }
            if m.len() > MAX_MAPPINGS {
                let mut by_age: Vec<(String, u64)> = m.iter().map(|(k, v)| (k.clone(), v.ts)).collect();
                by_age.sort_by_key(|(_, ts)| *ts);
                for (k, _) in by_age.into_iter().take(m.len() - MAX_MAPPINGS + 200) {
                    m.remove(&k);
                }
            }
        }
        if self.unsaved.fetch_add(1, Ordering::Relaxed) + 1 >= 10 {
            self.flush();
        }
    }

    pub fn len(&self) -> usize {
        self.map.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clears all in-memory mappings without writing to disk.
    pub fn clear(&self) {
        self.map.lock().unwrap().clear();
        self.unsaved.store(0, Ordering::Relaxed);
    }

    /// Writes the cache to disk if there are unsaved changes.
    pub fn flush(&self) {
        if self.unsaved.swap(0, Ordering::Relaxed) == 0 {
            return;
        }
        if let Some(p) = &self.path {
            let snapshot = self.map.lock().unwrap().clone();
            if let Err(e) = paths::write_json(p, &snapshot) {
                tracing::warn!("could not save resolver cache: {e}");
            }
        }
    }
}

impl Drop for MappingCache {
    fn drop(&mut self) {
        self.flush();
    }
}

fn mapping_keys(t: &Track) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(i) = t.isrc.as_ref().filter(|s| !s.is_empty()) {
        v.push(format!("isrc:{}", i.to_uppercase()));
    }
    v.push(format!("tk:{}", crate::dedupe::track_key(t)));
    v
}

// ------------------------------------------------------------------ scoring

fn duration_score(want: Option<u64>, have: Option<u64>) -> f32 {
    match (want, have) {
        (Some(w), Some(h)) => match w.abs_diff(h) {
            0..=3_000 => 1.0,
            3_001..=8_000 => 0.6,
            8_001..=15_000 => 0.25,
            _ => 0.0,
        },
        _ => 0.5,
    }
}

/// Confidence (0.0..=1.0) that `cand` is the wanted song.
pub fn score_candidate(want: &Track, cand: &Candidate) -> f32 {
    if let (Some(a), Some(b)) = (&want.isrc, &cand.track.isrc) {
        if !a.is_empty() && a.eq_ignore_ascii_case(b) {
            return 1.0;
        }
    }
    let ts = title_similarity(&want.title, &cand.track.title);
    let asim = artist_similarity(&want.artist, &cand.track.artist);
    let ds = duration_score(want.duration_ms, cand.track.duration_ms);
    let mut score = ts * 0.45 + asim * 0.30 + ds * 0.25 + cand.bonus;
    // "Official Audio" in the title (providers without custom bonus).
    if cand.bonus == 0.0 && cand.track.title.to_lowercase().contains("official audio") {
        score += 0.05;
    }
    if ts < 0.5 {
        score = score.min(0.4);
    }
    if ds == 0.0 {
        score = score.min(0.6); // clearly another cut / a live version
    }
    score.clamp(0.0, 1.0)
}

/// Picks the best candidate: highest confidence, but a higher-priority
/// provider wins if it is within 0.05.
fn rank(want: &Track, cands: Vec<(ProviderId, Candidate)>, priority: &[ProviderId]) -> Vec<(ProviderId, Candidate, f32)> {
    let prio = |p: ProviderId| priority.iter().position(|x| *x == p).unwrap_or(usize::MAX);
    let mut scored: Vec<_> = cands
        .into_iter()
        .map(|(p, c)| {
            let s = score_candidate(want, &c);
            (p, c, s)
        })
        .filter(|(_, _, s)| *s >= MIN_CONFIDENCE)
        .collect();
    scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    if let Some(top) = scored.first().map(|t| t.2) {
        // Stable partition: near-best candidates ordered by priority first.
        let (mut near, rest): (Vec<_>, Vec<_>) = scored.into_iter().partition(|t| t.2 >= top - 0.05);
        near.sort_by_key(|t| prio(t.0));
        near.extend(rest);
        return near;
    }
    scored
}

// ----------------------------------------------------------------- resolver

pub struct Resolver {
    providers: Arc<Providers>,
    cooldowns: Arc<Cooldowns>,
    priority: RwLock<Vec<ProviderId>>,
    cache: MappingCache,
    search_timeout: Duration,
}

impl Resolver {
    pub fn new(
        providers: Arc<Providers>,
        cooldowns: Arc<Cooldowns>,
        priority: Vec<ProviderId>,
        cache_path: Option<PathBuf>,
    ) -> Self {
        Self {
            providers,
            cooldowns,
            priority: RwLock::new(priority),
            cache: MappingCache::load(cache_path),
            search_timeout: SEARCH_TIMEOUT,
        }
    }

    pub fn with_search_timeout(mut self, d: Duration) -> Self {
        self.search_timeout = d;
        self
    }

    pub fn providers(&self) -> &Arc<Providers> {
        &self.providers
    }

    pub fn cooldowns(&self) -> &Arc<Cooldowns> {
        &self.cooldowns
    }

    pub fn cache(&self) -> &MappingCache {
        &self.cache
    }

    pub fn set_priority(&self, p: Vec<ProviderId>) {
        *self.priority.write().unwrap() = p;
    }

    pub fn priority(&self) -> Vec<ProviderId> {
        self.priority.read().unwrap().clone()
    }

    /// Full-playback providers: configured priority first, then any other
    /// enabled full source (e.g. Internet Archive).
    fn full_order(&self) -> Vec<ProviderId> {
        let mut order: Vec<ProviderId> = self.priority().into_iter().filter(|p| self.providers.is_full(*p)).collect();
        for id in self.providers.ids() {
            if self.providers.is_full(id) && !order.contains(&id) {
                order.push(id);
            }
        }
        order
    }

    fn note_error(&self, p: ProviderId, e: &anyhow::Error) {
        if should_cool_down(e) {
            self.cooldowns.trip(p);
        }
    }

    // --------------------------------------------------------------- search

    /// Starts a unified search; events arrive on the returned channel and end
    /// with [`SearchEvent::Done`].
    ///
    /// A batch contains new tracks and, for songs that were already sent by an
    /// earlier provider, the *updated* merged track (same `source`+`id` as before,
    /// now with more `external_ids`) — the UI replaces by `(source, id)`.
    pub fn search_stream(self: &Arc<Self>, query: &str, page: usize) -> mpsc::UnboundedReceiver<SearchEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let this = self.clone();
        let q = query.trim().to_string();
        tokio::spawn(async move {
            if q.is_empty() {
                let _ = tx.send(SearchEvent::Done);
                return;
            }
            let mut pending = FuturesUnordered::new();
            for p in this.providers.searchable() {
                let id = p.id();
                if this.cooldowns.is_cooling(id) {
                    continue;
                }
                let q = q.clone();
                let to = p.search_timeout().unwrap_or(this.search_timeout);
                pending.push(async move { (id, timeout(to, p.search_page(&q, page)).await) });
            }
            let mut merger = Merger::new();
            while let Some((id, res)) = pending.next().await {
                let event = match res {
                    Ok(Ok(tracks)) => {
                        let mut touched: Vec<usize> = Vec::new();
                        for t in tracks {
                            let (MergeOutcome::New(i) | MergeOutcome::Merged(i)) = merger.add(t);
                            if !touched.contains(&i) {
                                touched.push(i);
                            }
                        }
                        let tracks = touched.into_iter().map(|i| merger.tracks()[i].clone()).collect();
                        SearchEvent::Results { provider: id, tracks }
                    }
                    Ok(Err(e)) => {
                        this.note_error(id, &e);
                        SearchEvent::ProviderError { provider: id, message: e.to_string() }
                    }
                    Err(_) => {
                        this.cooldowns.trip(id);
                        SearchEvent::ProviderError { provider: id, message: "timed out".into() }
                    }
                };
                if tx.send(event).is_err() {
                    return; // receiver gone: drop the remaining searches
                }
            }
            let _ = tx.send(SearchEvent::Done);
        });
        rx
    }

    // -------------------------------------------------------------- resolve

    /// Full pipeline: known ids -> cached mapping -> search at full sources -> preview.
    pub async fn resolve(&self, track: &Track) -> Result<Option<Resolved>> {
        if let Some(r) = self.resolve_known(track).await {
            return Ok(Some(r));
        }
        if let Some(r) = self.resolve_cached(track).await {
            return Ok(Some(r));
        }
        if let Some(r) = self.resolve_by_search(track).await {
            return Ok(Some(r));
        }
        Ok(self.resolve_preview(track).await)
    }

    async fn source_of(&self, provider: ProviderId, view: &Track) -> Option<PlaybackSource> {
        let p = self.providers.get(provider)?;
        match timeout(p.source_timeout().unwrap_or(SOURCE_TIMEOUT), p.playback_source(view)).await {
            Ok(Ok(s)) => Some(s),
            Ok(Err(e)) => {
                tracing::debug!(?provider, "playback_source failed: {e}");
                self.note_error(provider, &e);
                None
            }
            Err(_) => {
                self.cooldowns.trip(provider);
                None
            }
        }
    }

    /// The track already knows an id at a full source.
    pub async fn resolve_known(&self, track: &Track) -> Option<Resolved> {
        for p in self.full_order() {
            if self.cooldowns.is_cooling(p) {
                continue;
            }
            let Some(id) = id_at(p, track) else { continue };
            let mut view = track.clone();
            view.source = p;
            view.id = id.to_string();
            if let Some(source) = self.source_of(p, &view).await {
                return Some(Resolved { track: track.clone(), source, confidence: 1.0 });
            }
        }
        None
    }

    /// Persistent mapping from an earlier resolve.
    pub async fn resolve_cached(&self, track: &Track) -> Option<Resolved> {
        let keys = mapping_keys(track);
        let (key, m) = self.cache.get(&keys)?;
        if self.cooldowns.is_cooling(m.provider) {
            return None;
        }
        match self.source_of(m.provider, &m.track).await {
            Some(source) => Some(Resolved { track: m.track, source, confidence: m.confidence }),
            None => {
                self.cache.remove(&key);
                None
            }
        }
    }

    /// The cached match of `track` at `provider`, if the mapping points there.
    pub fn cached_at(&self, track: &Track, provider: ProviderId) -> Option<(Track, f32)> {
        let (_, m) = self.cache.get(&mapping_keys(track))?;
        (m.provider == provider).then_some((m.track, m.confidence))
    }

    /// Records a mapping obtained elsewhere (e.g. from the server).
    pub fn remember(&self, wanted: &Track, matched: &Track, confidence: f32) {
        self.cache.insert(
            &mapping_keys(wanted),
            Mapping { provider: matched.source, track: matched.clone(), confidence, ts: paths::now_unix() },
        );
    }

    async fn search_many(&self, providers: Vec<ProviderId>, query: &str) -> Vec<(ProviderId, Candidate)> {
        let mut pending = FuturesUnordered::new();
        for id in providers {
            if self.cooldowns.is_cooling(id) {
                continue;
            }
            let Some(p) = self.providers.get(id) else { continue };
            let q = query.to_string();
            let to = p.search_timeout().unwrap_or(self.search_timeout);
            pending.push(async move { (id, timeout(to, p.search_candidates(&q)).await) });
        }
        let mut out = Vec::new();
        while let Some((id, res)) = pending.next().await {
            match res {
                Ok(Ok(cands)) => out.extend(cands.into_iter().map(|c| (id, c))),
                Ok(Err(e)) => self.note_error(id, &e),
                Err(_) => self.cooldowns.trip(id),
            }
        }
        out
    }

    /// Searches "Artist - Title" at the full sources and takes the best match.
    pub async fn resolve_by_search(&self, track: &Track) -> Option<Resolved> {
        let order = self.full_order();
        if order.is_empty() {
            return None;
        }
        let query = format!("{} - {}", track.artist, track.title);
        // YouTube costs 100 quota units per search: only ask it when the others
        // did not produce a sure match.
        let (cheap, expensive): (Vec<_>, Vec<_>) = order.iter().copied().partition(|p| *p != ProviderId::YouTube);
        let mut cands = self.search_many(cheap, &query).await;
        let mut ranked = rank(track, cands.clone(), &order);
        if ranked.first().map(|r| r.2).unwrap_or(0.0) < SURE_CONFIDENCE && !expensive.is_empty() {
            cands.extend(self.search_many(expensive, &query).await);
            ranked = rank(track, cands, &order);
        }
        for (provider, cand, confidence) in ranked.into_iter().take(3) {
            if let Some(source) = self.source_of(provider, &cand.track).await {
                self.remember(track, &cand.track, confidence);
                return Some(Resolved { track: cand.track, source, confidence });
            }
        }
        None
    }

    /// 30 s Deezer preview as the last resort (known id -> ISRC -> text match).
    pub async fn resolve_preview(&self, track: &Track) -> Option<Resolved> {
        let dz = ProviderId::Deezer;
        let p = self.providers.get(dz)?;
        if self.cooldowns.is_cooling(dz) {
            return None;
        }
        // 1. known Deezer id
        if let Some(id) = id_at(dz, track) {
            let mut view = track.clone();
            view.source = dz;
            view.id = id.to_string();
            if let Some(source) = self.source_of(dz, &view).await {
                return Some(Resolved { track: track.clone(), source, confidence: 1.0 });
            }
        }
        // 2. ISRC
        if let Some(isrc) = track.isrc.as_ref().filter(|s| !s.is_empty()) {
            match p.find_by_isrc(isrc).await {
                Ok(Some(t)) => {
                    if let Some(source) = self.source_of(dz, &t).await {
                        return Some(Resolved { track: t, source, confidence: 1.0 });
                    }
                }
                Ok(None) => {}
                Err(e) => self.note_error(dz, &e),
            }
        }
        // 3. text match
        let query = format!("{} - {}", track.artist, track.title);
        let cands = self.search_many(vec![dz], &query).await;
        for (provider, cand, confidence) in rank(track, cands, &[dz]).into_iter().take(2) {
            if confidence < 0.7 {
                continue;
            }
            if let Some(source) = self.source_of(provider, &cand.track).await {
                return Some(Resolved { track: cand.track, source, confidence });
            }
        }
        None
    }
}

/// Best candidate of one provider for `want` (at least [`MIN_CONFIDENCE`]).
pub fn best_candidate(want: &Track, cands: Vec<Candidate>) -> Option<(Candidate, f32)> {
    cands
        .into_iter()
        .map(|c| {
            let s = score_candidate(want, &c);
            (c, s)
        })
        .filter(|(_, s)| *s >= MIN_CONFIDENCE)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
}

/// De-duplicates ids while keeping order.
pub fn unique<T: std::hash::Hash + Eq + Clone>(v: &[T]) -> Vec<T> {
    let mut seen = HashSet::new();
    v.iter().filter(|x| seen.insert((*x).clone())).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedupe::test_track;
    use async_trait::async_trait;
    use glyph_protocol::Capabilities;

    struct Mock {
        id: ProviderId,
        caps: Capabilities,
        results: Vec<Track>,
        fail: bool,
        delay: Duration,
        calls: Arc<AtomicUsize>,
    }

    impl Mock {
        fn new(id: ProviderId, caps: Capabilities, results: Vec<Track>) -> Self {
            Self { id, caps, results, fail: false, delay: Duration::ZERO, calls: Arc::new(AtomicUsize::new(0)) }
        }
    }

    #[async_trait]
    impl MusicProvider for Mock {
        fn id(&self) -> ProviderId {
            self.id
        }
        fn capabilities(&self) -> Capabilities {
            self.caps
        }
        async fn search(&self, _q: &str) -> Result<Vec<Track>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            if self.fail {
                return Err(ProviderError::Http(500).into());
            }
            Ok(self.results.clone())
        }
        async fn track(&self, _id: &str) -> Result<Track> {
            Err(ProviderError::NotFound.into())
        }
        async fn playback_source(&self, track: &Track) -> Result<PlaybackSource> {
            let id = id_at(self.id, track).ok_or(ProviderError::Unsupported)?.to_string();
            Ok(match self.caps {
                Capabilities::Preview => PlaybackSource::Preview { provider: self.id, url: format!("https://p/{id}") },
                Capabilities::FullPlayback if self.id == ProviderId::YouTube || self.id == ProviderId::SoundCloud => {
                    PlaybackSource::Embed { provider: self.id, embed_id: id }
                }
                Capabilities::FullPlayback => {
                    PlaybackSource::Stream { provider: self.id, url: format!("https://s/{id}"), mime: None }
                }
                Capabilities::MetadataOnly => return Err(ProviderError::Unsupported.into()),
            })
        }
        async fn find_by_isrc(&self, isrc: &str) -> Result<Option<Track>> {
            Ok(self.results.iter().find(|t| t.isrc.as_deref() == Some(isrc)).cloned())
        }
    }

    fn resolver(mocks: Vec<Mock>) -> Arc<Resolver> {
        let mut p = Providers::default();
        for m in mocks {
            p.insert(Arc::new(m));
        }
        Arc::new(
            Resolver::new(
                Arc::new(p),
                Arc::new(Cooldowns::new()),
                vec![ProviderId::Audius, ProviderId::Jamendo, ProviderId::SoundCloud, ProviderId::YouTube],
                None,
            )
            .with_search_timeout(Duration::from_millis(200)),
        )
    }

    fn t(p: ProviderId, id: &str, title: &str, artist: &str, dur: Option<u64>) -> Track {
        test_track(p, id, title, artist, dur)
    }

    #[test]
    fn scoring() {
        let want = t(ProviderId::Spotify, "s", "Believer", "Imagine Dragons", Some(204_000));
        let good = Candidate { track: t(ProviderId::YouTube, "y", "Believer", "Imagine Dragons", Some(204_500)), bonus: 0.0 };
        let wrong_song = Candidate { track: t(ProviderId::YouTube, "y", "Thunder", "Imagine Dragons", Some(204_500)), bonus: 0.0 };
        let live = Candidate { track: t(ProviderId::YouTube, "y", "Believer", "Imagine Dragons", Some(290_000)), bonus: 0.0 };
        assert!(score_candidate(&want, &good) > 0.95);
        assert!(score_candidate(&want, &wrong_song) < 0.5);
        assert!(score_candidate(&want, &live) <= 0.6);
        let mut with_isrc = want.clone();
        with_isrc.isrc = Some("ABC".into());
        let mut c = wrong_song.clone();
        c.track.isrc = Some("abc".into());
        assert_eq!(score_candidate(&with_isrc, &c), 1.0);
        // Topic bonus lifts an unknown-duration hit over a plain one
        let plain = Candidate { track: t(ProviderId::YouTube, "a", "Believer", "Imagine Dragons", None), bonus: 0.0 };
        let topic = Candidate { bonus: 0.12, ..plain.clone() };
        assert!(score_candidate(&want, &topic) > score_candidate(&want, &plain));
    }

    #[test]
    fn ranking_prefers_priority_when_close() {
        let want = t(ProviderId::Spotify, "s", "Song", "Band", Some(200_000));
        let cands = vec![
            (ProviderId::YouTube, Candidate { track: t(ProviderId::YouTube, "y", "Song", "Band", Some(200_000)), bonus: 0.0 }),
            (ProviderId::Audius, Candidate { track: t(ProviderId::Audius, "a", "Song", "Band", Some(201_000)), bonus: 0.0 }),
        ];
        let r = rank(&want, cands, &[ProviderId::Audius, ProviderId::YouTube]);
        assert_eq!(r[0].0, ProviderId::Audius);
        assert_eq!(r.len(), 2);
    }

    #[tokio::test]
    async fn search_merges_and_streams_events() {
        let r = resolver(vec![
            Mock::new(ProviderId::Spotify, Capabilities::MetadataOnly, vec![t(ProviderId::Spotify, "s1", "Song", "Band", Some(200_000))]),
            {
                let mut m = Mock::new(
                    ProviderId::Audius,
                    Capabilities::FullPlayback,
                    vec![
                        t(ProviderId::Audius, "a1", "Song", "Band", Some(201_000)),
                        t(ProviderId::Audius, "a2", "Other", "Band", Some(100_000)),
                    ],
                );
                m.delay = Duration::from_millis(40);
                m
            },
            {
                let mut m = Mock::new(ProviderId::Jamendo, Capabilities::FullPlayback, vec![]);
                m.fail = true;
                m
            },
            {
                let mut m = Mock::new(ProviderId::Deezer, Capabilities::Preview, vec![]);
                m.delay = Duration::from_millis(900); // beyond the 200 ms test timeout
                m
            },
        ]);
        let mut rx = r.search_stream("song", 0);
        let mut events = vec![];
        while let Some(e) = rx.recv().await {
            let done = e == SearchEvent::Done;
            events.push(e);
            if done {
                break;
            }
        }
        assert_eq!(events.last(), Some(&SearchEvent::Done));
        // Spotify first (fast), then merged Audius batch
        let merged = events
            .iter()
            .find_map(|e| match e {
                SearchEvent::Results { provider: ProviderId::Audius, tracks } => Some(tracks.clone()),
                _ => None,
            })
            .expect("audius results");
        assert_eq!(merged.len(), 2);
        let song = merged.iter().find(|t| t.title == "Song").unwrap();
        // identity stays the first one seen (Spotify) and gains the Audius id
        assert_eq!(song.source, ProviderId::Spotify);
        assert_eq!(song.external_ids.get(&ProviderId::Audius).map(String::as_str), Some("a1"));
        assert!(events.iter().any(|e| matches!(e, SearchEvent::ProviderError { provider: ProviderId::Jamendo, .. })));
        assert!(events.iter().any(|e| matches!(e, SearchEvent::ProviderError { provider: ProviderId::Deezer, message } if message == "timed out")));
        // failed + timed-out providers now cool down
        assert!(r.cooldowns().is_cooling(ProviderId::Jamendo));
        assert!(r.cooldowns().is_cooling(ProviderId::Deezer));
        assert!(!r.cooldowns().is_cooling(ProviderId::Audius));
    }

    #[tokio::test]
    async fn cooled_down_provider_is_skipped() {
        let m = Mock::new(ProviderId::Audius, Capabilities::FullPlayback, vec![]);
        let calls = m.calls.clone();
        let r = resolver(vec![m]);
        r.cooldowns().trip(ProviderId::Audius);
        let mut rx = r.search_stream("x", 0);
        assert_eq!(rx.recv().await, Some(SearchEvent::Done));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn resolves_known_source_by_priority() {
        let r = resolver(vec![
            Mock::new(ProviderId::Audius, Capabilities::FullPlayback, vec![]),
            Mock::new(ProviderId::YouTube, Capabilities::FullPlayback, vec![]),
        ]);
        let mut track = t(ProviderId::Spotify, "s", "Song", "Band", Some(1000));
        track.external_ids.insert(ProviderId::YouTube, "yt1".into());
        track.external_ids.insert(ProviderId::Audius, "au1".into());
        let res = r.resolve(&track).await.unwrap().unwrap();
        assert_eq!(res.source, PlaybackSource::Stream { provider: ProviderId::Audius, url: "https://s/au1".into(), mime: None });
        assert_eq!(res.confidence, 1.0);
    }

    #[tokio::test]
    async fn metadata_only_track_is_searched_and_cached() {
        let audius = Mock::new(
            ProviderId::Audius,
            Capabilities::FullPlayback,
            vec![
                t(ProviderId::Audius, "wrong", "Different Song", "Band", Some(200_000)),
                t(ProviderId::Audius, "right", "Song", "Band", Some(200_500)),
            ],
        );
        let calls = audius.calls.clone();
        let yt = Mock::new(ProviderId::YouTube, Capabilities::FullPlayback, vec![t(ProviderId::YouTube, "yt", "Song", "Band", Some(200_000))]);
        let yt_calls = yt.calls.clone();
        let r = resolver(vec![audius, yt]);
        let want = t(ProviderId::Spotify, "sp", "Song", "Band", Some(200_000));
        let res = r.resolve(&want).await.unwrap().unwrap();
        assert_eq!(res.track.id, "right");
        assert!(res.confidence >= SURE_CONFIDENCE);
        assert_eq!(yt_calls.load(Ordering::SeqCst), 0, "YouTube quota must be spared on a sure match");
        // second resolve hits the mapping cache: no new search
        let again = r.resolve(&want).await.unwrap().unwrap();
        assert_eq!(again.track.id, "right");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn falls_back_to_youtube_then_preview() {
        let r = resolver(vec![
            Mock::new(ProviderId::Audius, Capabilities::FullPlayback, vec![]),
            Mock::new(ProviderId::YouTube, Capabilities::FullPlayback, vec![t(ProviderId::YouTube, "yt", "Song", "Band", Some(200_000))]),
        ]);
        let res = r.resolve(&t(ProviderId::Spotify, "sp", "Song", "Band", Some(200_000))).await.unwrap().unwrap();
        assert!(matches!(res.source, PlaybackSource::Embed { provider: ProviderId::YouTube, .. }));

        let mut dz = t(ProviderId::Deezer, "dz1", "Song", "Band", Some(200_000));
        dz.isrc = Some("ISRC1".into());
        let r = resolver(vec![Mock::new(ProviderId::Deezer, Capabilities::Preview, vec![dz])]);
        let mut want = t(ProviderId::Spotify, "sp", "Song", "Band", Some(200_000));
        want.isrc = Some("ISRC1".into());
        let res = r.resolve(&want).await.unwrap().unwrap();
        assert!(matches!(res.source, PlaybackSource::Preview { provider: ProviderId::Deezer, .. }));
        // nothing at all
        let r = resolver(vec![]);
        assert!(r.resolve(&want).await.unwrap().is_none());
    }

    #[test]
    fn mapping_cache_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resolver_cache.json");
        let c = MappingCache::load(Some(path.clone()));
        let tr = t(ProviderId::Audius, "a", "Song", "Band", None);
        c.insert(&["k1".into()], Mapping { provider: ProviderId::Audius, track: tr, confidence: 0.9, ts: 1 });
        c.flush();
        let c2 = MappingCache::load(Some(path));
        assert_eq!(c2.get(&["k1".into()]).unwrap().1.track.id, "a");
        assert!(!c2.is_empty());
    }
}
