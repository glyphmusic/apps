//! Reading lists without any key or login, so nobody has to register an
//! app anywhere:
//! - Spotify: the public embed page (`open.spotify.com/embed/...`) of a
//!   playlist, album, artist or song. Spotify caps it at 100 songs; longer
//!   lists (and Liked Songs) come over by copying the songs in the Spotify
//!   app (Ctrl+A, Ctrl+C gives one song link per line) and pasting them.
//! - YouTube: the playlist page through YouTube's own web endpoint (like the
//!   native search), single videos through oEmbed.
//! - Pasted text with many links ([`parse_pasted`]) and Google Takeout CSVs
//!   (video ids only, filled in through oEmbed).
//! Only titles, artists and lengths are read; nothing is played from there.

use anyhow::{anyhow, bail, Result};
use futures::stream::StreamExt;
use glyph_protocol::{ProviderId, Track};
use serde_json::Value;

/// Songs the Spotify embed page shows at most.
pub const SPOTIFY_EMBED_LIMIT: usize = 100;

// ------------------------------------------------------------ Spotify

/// The `entity` of an embed page's `__NEXT_DATA__`.
pub fn spotify_embed_entity(html: &str) -> Option<Value> {
    let i = html.find("id=\"__NEXT_DATA__\"")?;
    let rest = &html[i..];
    let body = &rest[rest.find('>')? + 1..];
    let end = body.find("</script>")?;
    let v: Value = serde_json::from_str(&body[..end]).ok()?;
    let e = v["props"]["pageProps"]["state"]["data"]["entity"].clone();
    e.is_object().then_some(e)
}

fn spotify_id(uri: &str) -> Option<&str> {
    uri.rsplit(':').next().filter(|id| !id.is_empty())
}

fn embed_cover(e: &Value) -> Option<String> {
    e["coverArt"]["sources"]
        .as_array()?
        .iter()
        .min_by_key(|s| s["width"].as_u64().unwrap_or(640).abs_diff(640))
        .and_then(|s| s["url"].as_str())
        .map(str::to_string)
}

fn spotify_track(id: &str, title: &str, artist: &str, album: Option<String>, dur: Option<u64>, cover: Option<String>, explicit: bool) -> Track {
    Track {
        id: id.to_string(),
        source: ProviderId::Spotify,
        title: title.trim().to_string(),
        artist: if artist.trim().is_empty() { "Unknown".into() } else { artist.trim().to_string() },
        album,
        duration_ms: dur.filter(|d| *d > 0),
        cover_url: cover,
        isrc: None,
        permalink: Some(format!("https://open.spotify.com/track/{id}")),
        external_ids: Default::default(),
        explicit,
    }
}

/// Name and songs of an embed entity (playlist, album, artist top songs or
/// one song). Artists in list rows are joined with ",\u{a0}".
pub fn parse_spotify_entity(e: &Value) -> Option<(String, Vec<Track>)> {
    let kind = e["type"].as_str().unwrap_or("");
    let name = e["name"].as_str().or(e["title"].as_str()).unwrap_or("Spotify").to_string();
    if kind == "track" {
        let id = spotify_id(e["uri"].as_str()?)?;
        let artist = e["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        let t = spotify_track(id, &name, &artist, None, e["duration"].as_u64(), embed_cover(e), e["isExplicit"].as_bool() == Some(true));
        return Some((name, vec![t]));
    }
    let album = (kind == "album").then(|| name.clone());
    let cover = (kind == "album").then(|| embed_cover(e)).flatten();
    let tracks: Vec<Track> = e["trackList"]
        .as_array()?
        .iter()
        .filter(|t| t["entityType"].as_str().is_none_or(|k| k == "track"))
        .filter_map(|t| {
            let id = spotify_id(t["uri"].as_str()?)?;
            let artist = t["subtitle"].as_str().unwrap_or("").replace(",\u{a0}", ", ");
            Some(spotify_track(id, t["title"].as_str()?, &artist, album.clone(), t["duration"].as_u64(), cover.clone(), t["isExplicit"].as_bool() == Some(true)))
        })
        .collect();
    let name = match kind {
        "album" => match e["subtitle"].as_str().filter(|s| !s.is_empty()) {
            Some(by) => format!("{name} · {}", by.replace(",\u{a0}", ", ")),
            None => name,
        },
        _ => name,
    };
    Some((name, tracks))
}

/// Outcome of fetching a single Spotify embed page.
enum EmbedFetch {
    Ok(String, Vec<Track>),
    RateLimited(Option<u64>),
    Transient(anyhow::Error),
    Fatal(anyhow::Error),
}

async fn fetch_spotify_embed_once(http: &reqwest::Client, kind: &str, id: &str) -> EmbedFetch {
    let resp = match http
        .get(format!("https://open.spotify.com/embed/{kind}/{id}"))
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (compatible; Glyph music app)")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return EmbedFetch::Transient(e.into()),
    };
    let status = resp.status().as_u16();
    match status {
        200..=299 => {}
        429 => {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok());
            return EmbedFetch::RateLimited(retry_after);
        }
        404 => return EmbedFetch::Fatal(anyhow!("this Spotify list does not exist or is private")),
        500..=599 => return EmbedFetch::Transient(anyhow!("Spotify answered HTTP {status}")),
        s => return EmbedFetch::Fatal(anyhow!("Spotify answered HTTP {s}")),
    }
    let html = match resp.text().await {
        Ok(h) => h,
        Err(e) => return EmbedFetch::Transient(e.into()),
    };
    let Some(e) = spotify_embed_entity(&html) else {
        return EmbedFetch::Fatal(anyhow!("Spotify page without a song list"));
    };
    let Some(list) = parse_spotify_entity(&e) else {
        return EmbedFetch::Fatal(anyhow!("Spotify page without a song list"));
    };
    if kind == "playlist" && list.1.len() >= SPOTIFY_EMBED_LIMIT {
        return EmbedFetch::Fatal(anyhow!(
            "Spotify only exposes the first 100 songs here. Use Spotify access authorized to read this playlist, or import a complete playlist file / copied song links."
        ));
    }
    EmbedFetch::Ok(list.0, list.1)
}

/// Reads `open.spotify.com/embed/<kind>/<id>`.
pub async fn spotify_embed(http: &reqwest::Client, kind: &str, id: &str) -> Result<(String, Vec<Track>)> {
    match fetch_spotify_embed_once(http, kind, id).await {
        EmbedFetch::Ok(name, tracks) => Ok((name, tracks)),
        EmbedFetch::RateLimited(_) => bail!("Spotify answered HTTP 429"),
        EmbedFetch::Transient(e) | EmbedFetch::Fatal(e) => Err(e),
    }
}

/// Coordinated adaptive concurrency + per-request pacing + shared 429 cooldown
/// for bulk track-metadata fetches (Spotify embed, YouTube oEmbed, Deezer) so
/// 1,000+ pasted links transfer smoothly without hitting upstream ~180/min or
/// 50/5s rate limits or silently dropping tracks.
struct RateGate {
    in_flight: std::sync::atomic::AtomicUsize,
    limit: std::sync::atomic::AtomicUsize,
    max_limit: usize,
    min_limit: usize,
    ok_streak: std::sync::atomic::AtomicUsize,
    base_interval_ms: u64,
    interval_ms: std::sync::atomic::AtomicU64,
    base_backoff_ms: u64,
    backoff_ms: std::sync::atomic::AtomicU64,
    paused_until: std::sync::Mutex<Option<std::time::Instant>>,
    next_slot: std::sync::Mutex<std::time::Instant>,
    last_429: std::sync::Mutex<Option<std::time::Instant>>,
    notify: tokio::sync::Notify,
}

struct RatePermit<'a>(&'a RateGate);
impl Drop for RatePermit<'_> {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        self.0.notify.notify_waiters();
    }
}

impl RateGate {
    fn with_params(initial_limit: usize, max_limit: usize, min_limit: usize, base_interval_ms: u64, base_backoff_ms: u64) -> Self {
        Self {
            in_flight: std::sync::atomic::AtomicUsize::new(0),
            limit: std::sync::atomic::AtomicUsize::new(initial_limit),
            max_limit,
            min_limit,
            ok_streak: std::sync::atomic::AtomicUsize::new(0),
            base_interval_ms,
            interval_ms: std::sync::atomic::AtomicU64::new(base_interval_ms),
            base_backoff_ms,
            backoff_ms: std::sync::atomic::AtomicU64::new(base_backoff_ms),
            paused_until: std::sync::Mutex::new(None),
            next_slot: std::sync::Mutex::new(std::time::Instant::now()),
            last_429: std::sync::Mutex::new(None),
            notify: tokio::sync::Notify::new(),
        }
    }

    /// Paced below Spotify's ~180 req/min rolling window for large pastes (>80
    /// uncached links), while staying fast for small pastes.
    fn for_spotify(uncached_count: usize) -> Self {
        if uncached_count > 80 {
            Self::with_params(4, 6, 1, 380, 3_000)
        } else {
            Self::with_params(8, 10, 1, 60, 2_000)
        }
    }

    fn for_youtube(uncached_count: usize) -> Self {
        if uncached_count > 80 {
            Self::with_params(5, 6, 1, 140, 2_000)
        } else {
            Self::with_params(8, 10, 1, 40, 2_000)
        }
    }

    /// Deezer allows 50 requests / 5 s (10 req/s); pace around ~7 req/s.
    fn for_deezer() -> Self {
        Self::with_params(4, 5, 1, 140, 5_000)
    }

    async fn acquire<'a>(&'a self, cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>) -> Option<RatePermit<'a>> {
        use std::sync::atomic::Ordering;
        loop {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return None;
            }
            let wait = {
                let g = self.paused_until.lock().unwrap_or_else(|e| e.into_inner());
                g.and_then(|until| until.checked_duration_since(std::time::Instant::now()))
            };
            if let Some(d) = wait {
                tokio::time::sleep(d.min(std::time::Duration::from_millis(200))).await;
                continue;
            }
            let notified = self.notify.notified();
            let cap = self.limit.load(Ordering::SeqCst);
            if self.in_flight.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < cap).then_some(n + 1)).is_ok() {
                let permit = RatePermit(self);
                let spacing = std::time::Duration::from_millis(self.interval_ms.load(Ordering::Relaxed));
                if !spacing.is_zero() {
                    let slot = {
                        let mut g = self.next_slot.lock().unwrap_or_else(|e| e.into_inner());
                        let now = std::time::Instant::now();
                        let start = (*g).max(now);
                        *g = start + spacing;
                        start
                    };
                    let mut interrupted_by_pause = false;
                    loop {
                        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                            return None;
                        }
                        let paused = {
                            let g = self.paused_until.lock().unwrap_or_else(|e| e.into_inner());
                            g.is_some_and(|u| u > std::time::Instant::now())
                        };
                        if paused {
                            interrupted_by_pause = true;
                            break;
                        }
                        let now = std::time::Instant::now();
                        if now >= slot {
                            break;
                        }
                        tokio::time::sleep((slot - now).min(std::time::Duration::from_millis(200))).await;
                    }
                    if interrupted_by_pause {
                        // Drop permit and re-enter after cooldown so resuming honors the lowered
                        // concurrency cap and fresh pacing instead of creating a thundering herd.
                        drop(permit);
                        continue;
                    }
                }
                return Some(permit);
            }
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
            }
        }
    }

    fn on_ok(&self) {
        use std::sync::atomic::Ordering;
        let streak = self.ok_streak.fetch_add(1, Ordering::Relaxed) + 1;
        if streak >= 40 {
            self.ok_streak.store(0, Ordering::Relaxed);
            self.backoff_ms.store(self.base_backoff_ms, Ordering::Relaxed);
            let max_lim = self.max_limit;
            let _ = self.limit.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < max_lim).then_some((n + 1).min(max_lim)));
            let cur_gap = self.interval_ms.load(Ordering::Relaxed);
            if cur_gap > self.base_interval_ms {
                let tighter = (cur_gap * 85 / 100).max(self.base_interval_ms);
                self.interval_ms.store(tighter, Ordering::Relaxed);
            }
            self.notify.notify_waiters();
        }
    }

    fn on_rate_limited(&self, retry_after_secs: Option<u64>) {
        use std::sync::atomic::Ordering;
        let now = std::time::Instant::now();
        self.ok_streak.store(0, Ordering::Relaxed);
        // Deduplicate concurrent 429s from the same in-flight burst (within 2 s)
        // so one rate-limit window does not multiply backoff/interval N times.
        let fresh_incident = {
            let mut last = self.last_429.lock().unwrap_or_else(|e| e.into_inner());
            let fresh = last.is_none_or(|t| now.saturating_duration_since(t) >= std::time::Duration::from_secs(2));
            if fresh {
                *last = Some(now);
            }
            fresh
        };
        let prev = self.backoff_ms.load(Ordering::Relaxed);
        let wait_ms = match retry_after_secs.filter(|&s| s > 0) {
            Some(s) => s.clamp(2, 120) * 1_000,
            None => prev,
        };
        if fresh_incident {
            self.backoff_ms.store(prev.saturating_mul(2).min(60_000), Ordering::Relaxed);
            let min_lim = self.min_limit;
            let _ = self.limit.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| Some((n / 2).max(min_lim)));
            let cur_gap = self.interval_ms.load(Ordering::Relaxed);
            let bumped = cur_gap.max(350).saturating_mul(3) / 2;
            self.interval_ms.store(bumped.min(2_500), Ordering::Relaxed);
        }
        let until = now + std::time::Duration::from_millis(wait_ms);
        {
            let mut g = self.paused_until.lock().unwrap_or_else(|e| e.into_inner());
            if g.is_none_or(|cur| until > cur) {
                *g = Some(until);
            }
        }
        {
            let mut slot = self.next_slot.lock().unwrap_or_else(|e| e.into_inner());
            if until > *slot {
                *slot = until;
            }
        }
    }
}

async fn fetch_spotify_track_with_retry(
    http: &reqwest::Client,
    id: &str,
    gate: &RateGate,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Option<Track> {
    use std::sync::atomic::Ordering;
    let mut rate_limits = 0u32;
    let mut transients = 0u32;
    loop {
        let Some(permit) = gate.acquire(cancel).await else { return None };
        match fetch_spotify_embed_once(http, "track", id).await {
            EmbedFetch::Ok(_, tracks) => {
                gate.on_ok();
                return tracks.into_iter().next();
            }
            EmbedFetch::RateLimited(retry_after) => {
                gate.on_rate_limited(retry_after);
                drop(permit);
                rate_limits += 1;
                if rate_limits >= 16 {
                    return None;
                }
            }
            EmbedFetch::Transient(_) => {
                drop(permit);
                transients += 1;
                if transients >= 3 || cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                    return None;
                }
                tokio::time::sleep(std::time::Duration::from_millis(400 * u64::from(transients))).await;
            }
            EmbedFetch::Fatal(_) => return None,
        }
    }
}

async fn fetch_youtube_oembed_once(http: &reqwest::Client, id: &str) -> EmbedFetch {
    let url = format!("https://www.youtube.com/watch?v={id}");
    let resp = match http
        .get("https://www.youtube.com/oembed")
        .query(&[("url", url.as_str()), ("format", "json")])
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return EmbedFetch::Transient(e.into()),
    };
    match resp.status().as_u16() {
        200..=299 => {}
        429 => {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok());
            return EmbedFetch::RateLimited(retry_after);
        }
        500..=599 => return EmbedFetch::Transient(anyhow!("YouTube oEmbed HTTP {}", resp.status().as_u16())),
        s => return EmbedFetch::Fatal(anyhow!("YouTube oEmbed HTTP {s}")),
    }
    let v: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return EmbedFetch::Transient(e.into()),
    };
    let Some(title) = v["title"].as_str() else {
        return EmbedFetch::Fatal(anyhow!("video not found"));
    };
    let channel = v["author_name"].as_str().unwrap_or("");
    EmbedFetch::Ok(title.to_string(), vec![crate::providers::youtube::video_track(id, title, channel, None, None).0])
}

async fn fetch_youtube_track_with_retry(
    http: &reqwest::Client,
    id: &str,
    gate: &RateGate,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Option<Track> {
    use std::sync::atomic::Ordering;
    let mut rate_limits = 0u32;
    let mut transients = 0u32;
    loop {
        let Some(permit) = gate.acquire(cancel).await else { return None };
        match fetch_youtube_oembed_once(http, id).await {
            EmbedFetch::Ok(_, tracks) => {
                gate.on_ok();
                return tracks.into_iter().next();
            }
            EmbedFetch::RateLimited(retry_after) => {
                gate.on_rate_limited(retry_after);
                drop(permit);
                rate_limits += 1;
                if rate_limits >= 12 {
                    return None;
                }
            }
            EmbedFetch::Transient(_) => {
                drop(permit);
                transients += 1;
                if transients >= 3 || cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                    return None;
                }
                tokio::time::sleep(std::time::Duration::from_millis(350 * u64::from(transients))).await;
            }
            EmbedFetch::Fatal(_) => return None,
        }
    }
}

async fn fetch_deezer_track_with_retry(
    deezer: &crate::providers::deezer::Deezer,
    id: &str,
    gate: &RateGate,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Option<Track> {
    use crate::provider::ProviderError;
    use std::sync::atomic::Ordering;
    let mut rate_limits = 0u32;
    let mut transients = 0u32;
    loop {
        let Some(permit) = gate.acquire(cancel).await else { return None };
        match tokio::time::timeout(std::time::Duration::from_secs(10), deezer.track_by_id(id)).await {
            Ok(Ok(track)) => {
                gate.on_ok();
                return Some(track);
            }
            Ok(Err(e)) if matches!(e.downcast_ref::<ProviderError>(), Some(ProviderError::Quota)) => {
                gate.on_rate_limited(Some(5));
                drop(permit);
                rate_limits += 1;
                if rate_limits >= 12 {
                    return None;
                }
            }
            Ok(Err(e)) if matches!(e.downcast_ref::<ProviderError>(), Some(ProviderError::NotFound)) => {
                return None;
            }
            Ok(Err(_)) | Err(_) => {
                drop(permit);
                transients += 1;
                if transients >= 3 || cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                    return None;
                }
                tokio::time::sleep(std::time::Duration::from_millis(400 * u64::from(transients))).await;
            }
        }
    }
}

// ------------------------------------------------------------ YouTube

const YT_BROWSE: &str = "https://www.youtube.com/youtubei/v1/browse?prettyPrint=false";
const YT_CLIENT_VERSION: &str = "2.20260930.01.00";

/// Video rows of a playlist page (new `lockupViewModel` and old
/// `playlistVideoRenderer`), plus the continuation token for the next page.
pub fn parse_youtube_playlist(v: &Value) -> (Vec<Track>, Option<String>) {
    fn walk(v: &Value, out: &mut Vec<Track>, cont: &mut Option<String>) {
        match v {
            Value::Object(o) => {
                if let Some(l) = o.get("lockupViewModel") {
                    if let Some(t) = from_lockup(l) {
                        out.push(t);
                    }
                    return;
                }
                if let Some(r) = o.get("playlistVideoRenderer") {
                    if let Some(t) = from_renderer(r) {
                        out.push(t);
                    }
                    return;
                }
                if let Some(tok) = o.get("continuationCommand").and_then(|c| c["token"].as_str()).filter(|t| !t.is_empty()) {
                    cont.get_or_insert_with(|| tok.to_string());
                }
                o.values().for_each(|x| walk(x, out, cont));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out, cont)),
            _ => {}
        }
    }
    let (mut out, mut cont) = (Vec::new(), None);
    walk(v, &mut out, &mut cont);
    (out, cont)
}

fn clock_ms(s: &str) -> Option<u64> {
    let mut total = 0u64;
    for part in s.trim().split(':') {
        total = total * 60 + part.trim().parse::<u64>().ok()?;
    }
    (total > 0).then_some(total * 1000)
}

/// "Shakira and 2 more" -> "Shakira".
fn first_artist(byline: &str) -> String {
    match byline.rfind(" and ") {
        Some(i) if byline[i..].ends_with(" more") => byline[..i].to_string(),
        _ => byline.to_string(),
    }
}

fn from_lockup(l: &Value) -> Option<Track> {
    if l["contentType"].as_str() != Some("LOCKUP_CONTENT_TYPE_VIDEO") {
        return None;
    }
    let id = l["contentId"].as_str()?;
    let md = &l["metadata"]["lockupMetadataViewModel"];
    let title = md["title"]["content"].as_str()?;
    let byline = md["metadata"]["contentMetadataViewModel"]["metadataRows"][0]["metadataParts"][0]["text"]["content"].as_str().unwrap_or("");
    // The length is a thumbnail badge ("4:01").
    let mut dur = None;
    find_clock(&l["contentImage"], &mut dur);
    Some(crate::providers::youtube::video_track(id, title, &first_artist(byline), None, dur).0)
}

fn find_clock(v: &Value, out: &mut Option<u64>) {
    if out.is_some() {
        return;
    }
    match v {
        Value::Object(o) => {
            if let Some(t) = o.get("text").and_then(|t| t.as_str()).filter(|t| t.contains(':')) {
                if let Some(ms) = clock_ms(t) {
                    *out = Some(ms);
                    return;
                }
            }
            o.values().for_each(|x| find_clock(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| find_clock(x, out)),
        _ => {}
    }
}

fn runs_text(v: &Value) -> Option<String> {
    if let Some(s) = v["simpleText"].as_str() {
        return Some(s.to_string());
    }
    let s: String = v["runs"].as_array()?.iter().filter_map(|r| r["text"].as_str()).collect();
    (!s.is_empty()).then_some(s)
}

fn from_renderer(r: &Value) -> Option<Track> {
    let id = r["videoId"].as_str()?;
    let title = runs_text(&r["title"])?;
    let channel = runs_text(&r["shortBylineText"]).unwrap_or_default();
    let dur = r["lengthSeconds"].as_str().and_then(|s| s.parse::<u64>().ok()).map(|s| s * 1000);
    Some(crate::providers::youtube::video_track(id, &title, &channel, None, dur).0)
}

fn youtube_title(v: &Value) -> Option<String> {
    v["metadata"]["playlistMetadataRenderer"]["title"]
        .as_str()
        .or(v["header"]["pageHeaderRenderer"]["pageTitle"].as_str())
        .map(str::to_string)
}

/// A public (or unlisted) YouTube / YouTube Music playlist, all pages.
pub async fn youtube_playlist(http: &reqwest::Client, list_id: &str) -> Result<(String, Vec<Track>)> {
    youtube_playlist_with_progress(http, list_id, None, None).await
}

/// Same as [`youtube_playlist`], reporting tracks read per page and stopping early when cancelled.
pub async fn youtube_playlist_with_progress(
    http: &reqwest::Client,
    list_id: &str,
    progress: Option<&crate::flume_like::Sender<crate::ImportProgress>>,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(String, Vec<Track>)> {
    use std::sync::atomic::Ordering;
    let context = serde_json::json!({"client": {"clientName": "WEB", "clientVersion": YT_CLIENT_VERSION, "hl": "en", "gl": "US"}});
    let mut body = serde_json::json!({"context": context, "browseId": format!("VL{list_id}")});
    let (mut name, mut tracks) = (None, Vec::new());
    let mut seen_ids = std::collections::HashSet::new();
    let max_tracks = crate::library::playlist_track_limit();
    let max_pages = max_tracks.div_ceil(50);
    for page_index in 0..=max_pages {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            bail!("cancelled");
        }
        let v: Value = http
            .post(YT_BROWSE)
            .header("X-Youtube-Client-Name", "1")
            .header("X-Youtube-Client-Version", YT_CLIENT_VERSION)
            .header("Origin", "https://www.youtube.com")
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if name.is_none() {
            name = youtube_title(&v);
        }
        let (page, cont) = parse_youtube_playlist(&v);
        let before = tracks.len();
        for t in page {
            if seen_ids.insert(t.id.clone()) {
                tracks.push(t);
            }
        }
        if tracks.len() > max_tracks { bail!("Playlist exceeds this server's limit of {max_tracks} songs"); }
        if let Some(tx) = progress {
            let total = if cont.is_none() { tracks.len() } else { 0 };
            let _ = tx.send(crate::ImportProgress { label: "IMPORT youtube/playlist".into(), done: tracks.len(), total });
        }
        match cont {
            Some(token) => {
                if tracks.len() == before || page_index == max_pages { bail!("YouTube did not complete the playlist; retry the import"); }
                body = serde_json::json!({"context": context, "continuation": token});
            }
            None => break,
        }
    }
    if tracks.is_empty() {
        bail!("this YouTube playlist is empty or private (make it public or unlisted)");
    }
    Ok((name.unwrap_or_else(|| "YouTube playlist".into()), tracks))
}

/// Fast, bounded metadata preview for a pasted list link (title, optional cover, optional count).
/// Used to show real names and artwork in the transfer chooser without reading every track page.
pub async fn peek_link_meta(
    http: &reqwest::Client,
    url: &str,
    src: &super::RemoteSource,
) -> Option<(String, Option<String>, Option<usize>)> {
    use super::RemoteSource as R;
    let fut = async {
        match src {
            R::SpotifyPlaylist(_) | R::SpotifyAlbum(_) | R::SpotifyArtist(_) => {
                let v: Value = http
                    .get("https://open.spotify.com/oembed")
                    .query(&[("url", url)])
                    .send()
                    .await
                    .ok()?
                    .error_for_status()
                    .ok()?
                    .json()
                    .await
                    .ok()?;
                let title = v["title"].as_str()?.trim().to_string();
                if title.is_empty() { return None; }
                let cover = v["thumbnail_url"].as_str().map(str::to_string);
                Some((title, cover, None))
            }
            R::YouTubePlaylist(id) => {
                let canon = format!("https://www.youtube.com/playlist?list={id}");
                let v: Value = http
                    .get("https://www.youtube.com/oembed")
                    .query(&[("url", canon.as_str()), ("format", "json")])
                    .send()
                    .await
                    .ok()?
                    .error_for_status()
                    .ok()?
                    .json()
                    .await
                    .ok()?;
                let title = v["title"].as_str()?.trim().to_string();
                if title.is_empty() { return None; }
                let cover = v["thumbnail_url"].as_str().map(str::to_string);
                Some((title, cover, None))
            }
            R::DeezerPlaylist(id) => {
                let v: Value = http
                    .get(format!("https://api.deezer.com/playlist/{id}"))
                    .send()
                    .await
                    .ok()?
                    .error_for_status()
                    .ok()?
                    .json()
                    .await
                    .ok()?;
                let title = v["title"].as_str()?.trim().to_string();
                if title.is_empty() { return None; }
                let cover = v["picture_big"].as_str().or(v["picture_medium"].as_str()).map(str::to_string);
                let count = v["nb_tracks"].as_u64().map(|n| n as usize);
                Some((title, cover, count))
            }
            R::DeezerAlbum(id) => {
                let v: Value = http
                    .get(format!("https://api.deezer.com/album/{id}"))
                    .send()
                    .await
                    .ok()?
                    .error_for_status()
                    .ok()?
                    .json()
                    .await
                    .ok()?;
                let title = v["title"].as_str()?.trim().to_string();
                if title.is_empty() { return None; }
                let cover = v["cover_big"].as_str().or(v["cover_medium"].as_str()).map(str::to_string);
                let count = v["nb_tracks"].as_u64().map(|n| n as usize);
                Some((title, cover, count))
            }
            _ => None,
        }
    };
    tokio::time::timeout(std::time::Duration::from_millis(2500), fut).await.ok().flatten()
}

/// Title and channel of one video (oEmbed, no key).
pub async fn youtube_video(http: &reqwest::Client, id: &str) -> Result<Track> {
    let url = format!("https://www.youtube.com/watch?v={id}");
    let v: Value = http.get("https://www.youtube.com/oembed").query(&[("url", url.as_str()), ("format", "json")]).send().await?.error_for_status()?.json().await?;
    let title = v["title"].as_str().ok_or_else(|| anyhow!("video not found"))?;
    let channel = v["author_name"].as_str().unwrap_or("");
    Ok(crate::providers::youtube::video_track(id, title, channel, None, None).0)
}

// ------------------------------------------------------------ pasted text

/// One song link of a pasted text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SongLink {
    Spotify(String),
    YouTube(String),
    Deezer(String),
}

impl SongLink {
    fn cache_key(&self) -> String {
        match self {
            Self::Spotify(id) => format!("sp:{id}"),
            Self::YouTube(id) => format!("yt:{id}"),
            Self::Deezer(id) => format!("dz:{id}"),
        }
    }
}

/// What a pasted text holds: single songs (in order) and whole lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pasted {
    pub songs: Vec<SongLink>,
    /// (link, what it is) of playlists, albums, profiles.
    pub lists: Vec<(String, super::RemoteSource)>,
}

impl Pasted {
    pub fn is_empty(&self) -> bool {
        self.songs.is_empty() && self.lists.is_empty()
    }
}

fn youtube_video_id(url: &reqwest::Url) -> Option<String> {
    let host = url.host_str()?.trim_start_matches("www.").to_lowercase();
    let id = match host.as_str() {
        "youtu.be" => url.path_segments()?.next().map(str::to_string),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" => url.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.to_string()).or_else(|| {
            // youtube.com/shorts/<id>, /live/<id>
            let segs: Vec<&str> = url.path_segments()?.collect();
            matches!(segs.first(), Some(&"shorts") | Some(&"live")).then(|| segs.get(1).map(|s| s.to_string())).flatten()
        }),
        _ => None,
    }?;
    crate::ytdlp::valid_video_id(&id).then_some(id)
}

/// Every link in a pasted text (one per line, or separated by spaces /
/// commas), deduplicated, in order. `spotify:track:` URIs work too.
pub fn parse_pasted(text: &str) -> Pasted {
    let mut out = Pasted::default();
    let mut seen_songs: std::collections::HashSet<SongLink> = std::collections::HashSet::new();
    for word in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let word = word.trim().trim_matches(|c| c == '"' || c == '\'' || c == '<' || c == '>');
        if word.is_empty() {
            continue;
        }
        let song = if let Some(id) = word.strip_prefix("spotify:track:") {
            Some(SongLink::Spotify(id.to_string()))
        } else if let Ok(url) = reqwest::Url::parse(word) {
            let host = url.host_str().unwrap_or("").trim_start_matches("www.").to_lowercase();
            let segs: Vec<&str> = url.path_segments().map(|s| s.filter(|x| !x.is_empty()).collect()).unwrap_or_default();
            let after = |name: &str| segs.iter().position(|s| *s == name).and_then(|i| segs.get(i + 1)).map(|s| s.to_string());
            match host.as_str() {
                "open.spotify.com" => after("track").map(SongLink::Spotify),
                "deezer.com" => after("track").filter(|id| id.chars().all(|c| c.is_ascii_digit())).map(SongLink::Deezer),
                _ => youtube_video_id(&url).filter(|_| !url.query_pairs().any(|(k, _)| k == "list")).map(SongLink::YouTube),
            }
        } else {
            None
        };
        match song {
            Some(s) => {
                if seen_songs.insert(s.clone()) {
                    out.songs.push(s);
                }
            }
            None => {
                if let Some(src) = super::parse_url(word) {
                    if !out.lists.iter().any(|(_, s)| *s == src) {
                        out.lists.push((word.to_string(), src));
                    }
                }
            }
        }
    }
    out
}

const IMPORT_CACHE_MAX: usize = 25_000;

fn import_track_cache_path() -> std::path::PathBuf {
    crate::paths::cache_dir().join("import-tracks.json")
}

fn load_import_track_cache(path: &std::path::Path) -> std::collections::HashMap<String, Track> {
    crate::paths::read_json(path).unwrap_or_default()
}

fn save_import_track_cache(path: &std::path::Path, mut map: std::collections::HashMap<String, Track>) {
    if map.len() > IMPORT_CACHE_MAX {
        let drop_n = map.len() - IMPORT_CACHE_MAX + 500;
        let keys: Vec<String> = map.keys().take(drop_n).cloned().collect();
        for k in keys {
            map.remove(&k);
        }
    }
    let _ = crate::paths::write_json(path, &map);
}

/// Metadata of pasted song links, in order (songs that cannot be read are
/// left out). Reuses cached metadata from earlier imports, batches Spotify API
/// lookups (50 per call, 3 in parallel) when a key is present, and otherwise
/// reads embed/oEmbed pages with per-provider adaptive pacing, coordinated 429
/// cooldowns and periodic disk-cache checkpoints so 1,000+ songs never hit
/// upstream ~180/min blocks or lose progress.
pub async fn read_songs(
    songs: &[SongLink],
    spotify: Option<&std::sync::Arc<crate::providers::spotify::Spotify>>,
    deezer: &crate::providers::deezer::Deezer,
    http: &reqwest::Client,
) -> Vec<Track> {
    read_songs_with_progress(songs, spotify, deezer, http, None, None).await
}

/// Same as [`read_songs`], emitting live progress and honoring `cancel`.
pub async fn read_songs_with_progress(
    songs: &[SongLink],
    spotify: Option<&std::sync::Arc<crate::providers::spotify::Spotify>>,
    deezer: &crate::providers::deezer::Deezer,
    http: &reqwest::Client,
    progress: Option<&crate::flume_like::Sender<crate::ImportProgress>>,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Vec<Track> {
    use std::sync::atomic::Ordering;
    let total = songs.len();
    if let Some(tx) = progress {
        let _ = tx.send(crate::ImportProgress { label: "IMPORT reading songs".into(), done: 0, total });
    }
    if total == 0 {
        return Vec::new();
    }
    let cache_path = import_track_cache_path();
    let cache_load_path = cache_path.clone();
    let mut disk_cache = tokio::task::spawn_blocking(move || load_import_track_cache(&cache_load_path))
        .await
        .unwrap_or_default();

    let mut slots: Vec<Option<Track>> = vec![None; total];
    let mut missing: Vec<(usize, SongLink)> = Vec::new();
    let mut done = 0usize;
    for (idx, s) in songs.iter().enumerate() {
        if let Some(t) = disk_cache.get(&s.cache_key()) {
            slots[idx] = Some(t.clone());
            done += 1;
        } else {
            missing.push((idx, s.clone()));
        }
    }
    if done > 0 {
        if let Some(tx) = progress {
            let _ = tx.send(crate::ImportProgress { label: "IMPORT reading songs".into(), done, total });
        }
    }
    if missing.is_empty() {
        return slots.into_iter().flatten().collect();
    }

    // Spotify with a key: 50 per request, 3 batches in parallel (throttled & retried on 429).
    let mut known: std::collections::HashMap<String, Track> = Default::default();
    if let Some(sp) = spotify {
        let sp_ids: Vec<String> = missing
            .iter()
            .filter_map(|(_, s)| if let SongLink::Spotify(id) = s { Some(id.clone()) } else { None })
            .collect();
        let chunks: Vec<Vec<String>> = sp_ids.chunks(50).map(|c| c.to_vec()).collect();
        let mut batch_stream = futures::stream::iter(chunks)
            .map(|chunk| async move {
                if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                    return None;
                }
                let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
                match sp.tracks_by_ids(&refs).await {
                    Ok(list) => Some(list),
                    Err(e) => {
                        tracing::info!("Spotify batch lookup failed, using embed pages: {e:#}");
                        None
                    }
                }
            })
            .buffer_unordered(3);
        while let Some(batch) = batch_stream.next().await {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Vec::new();
            }
            if let Some(list) = batch {
                for t in list {
                    known.insert(t.id.clone(), t);
                }
            }
        }
    }

    let mut unsaved_cache = 0usize;
    let mut remaining: Vec<(usize, SongLink)> = Vec::new();
    for (idx, s) in missing {
        if let SongLink::Spotify(ref id) = s {
            if let Some(t) = known.get(id) {
                disk_cache.insert(s.cache_key(), t.clone());
                unsaved_cache += 1;
                slots[idx] = Some(t.clone());
                done += 1;
                continue;
            }
        }
        remaining.push((idx, s));
    }
    if unsaved_cache > 0 {
        if let Some(tx) = progress {
            let _ = tx.send(crate::ImportProgress { label: "IMPORT reading songs".into(), done, total });
        }
    }

    let sp_uncached = remaining.iter().filter(|(_, s)| matches!(s, SongLink::Spotify(_))).count();
    let yt_uncached = remaining.iter().filter(|(_, s)| matches!(s, SongLink::YouTube(_))).count();
    let sp_gate = std::sync::Arc::new(RateGate::for_spotify(sp_uncached));
    let yt_gate = std::sync::Arc::new(RateGate::for_youtube(yt_uncached));
    let dz_gate = std::sync::Arc::new(RateGate::for_deezer());
    let mut stream = futures::stream::iter(remaining)
        .map(|(idx, s)| {
            let sp_gate = sp_gate.clone();
            let yt_gate = yt_gate.clone();
            let dz_gate = dz_gate.clone();
            async move {
                if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                    return (idx, s, None);
                }
                let track = match &s {
                    SongLink::Spotify(id) => fetch_spotify_track_with_retry(http, id, &sp_gate, cancel).await,
                    SongLink::YouTube(id) => fetch_youtube_track_with_retry(http, id, &yt_gate, cancel).await,
                    SongLink::Deezer(id) => fetch_deezer_track_with_retry(deezer, id, &dz_gate, cancel).await,
                };
                (idx, s, track)
            }
        })
        .buffer_unordered(10);
    while let Some((idx, link, track)) = stream.next().await {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            break;
        }
        if let Some(ref t) = track {
            disk_cache.insert(link.cache_key(), t.clone());
            unsaved_cache += 1;
        }
        slots[idx] = track;
        done += 1;
        if let Some(tx) = progress {
            let _ = tx.send(crate::ImportProgress { label: "IMPORT reading songs".into(), done, total });
        }
        // Checkpoint periodically so cancelling/restarting a 1,000-song paste
        // reuses already-fetched tracks immediately without re-hitting the API.
        if unsaved_cache >= 50 {
            unsaved_cache = 0;
            let snap = disk_cache.clone();
            let p = cache_path.clone();
            let _ = tokio::task::spawn_blocking(move || save_import_track_cache(&p, snap)).await;
        }
    }
    if unsaved_cache > 0 {
        let _ = tokio::task::spawn_blocking(move || save_import_track_cache(&cache_path, disk_cache)).await;
    }
    slots.into_iter().flatten().collect()
}

/// Fills in YouTube songs of a Google Takeout CSV (ids only) through oEmbed.
pub async fn fill_takeout(http: &reqwest::Client, tracks: Vec<Track>) -> Vec<Track> {
    fill_takeout_with_progress(http, tracks, None, None).await
}

/// Same as [`fill_takeout`], emitting live progress and honoring `cancel`.
pub async fn fill_takeout_with_progress(
    http: &reqwest::Client,
    tracks: Vec<Track>,
    progress: Option<&crate::flume_like::Sender<crate::ImportProgress>>,
    cancel: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Vec<Track> {
    let songs: Vec<SongLink> = tracks.iter().map(|t| SongLink::YouTube(t.id.clone())).collect();
    let dz = crate::providers::deezer::Deezer::new(http.clone());
    let found = read_songs_with_progress(&songs, None, &dz, http, progress, cancel).await;
    let found: std::collections::HashMap<_, _> = found.into_iter().map(|t| (t.id.clone(), t)).collect();
    // A temporary oEmbed failure must not remove a row from the user's export.
    tracks.into_iter().map(|t| found.get(&t.id).cloned().unwrap_or(t)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_embed_playlist() {
        let html = r#"<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"state":{"data":{"entity":{
            "type":"playlist","name":"Today's Top Hits","trackList":[
              {"uri":"spotify:track:3XmwAZpIMNpTJy8faw3ajD","title":"expectations (feat. Chappell Roan)","subtitle":"Olivia Rodrigo, Chappell Roan","duration":223935,"isExplicit":false,"entityType":"track"},
              {"uri":"spotify:episode:x","title":"Podcast","subtitle":"Someone","entityType":"episode"}]}}}}}}</script>"#;
        let e = spotify_embed_entity(html).unwrap();
        let (name, t) = parse_spotify_entity(&e).unwrap();
        assert_eq!(name, "Today's Top Hits");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "3XmwAZpIMNpTJy8faw3ajD");
        assert_eq!(t[0].artist, "Olivia Rodrigo, Chappell Roan");
        assert_eq!(t[0].duration_ms, Some(223_935));
        assert_eq!(t[0].source, ProviderId::Spotify);
    }

    #[test]
    fn spotify_embed_album_and_track() {
        let album: Value = serde_json::from_str(
            r#"{"type":"album","name":"RAM","subtitle":"Daft Punk","coverArt":{"sources":[{"url":"https://i/640","width":640}]},
                "trackList":[{"uri":"spotify:track:a1","title":"Give Life Back to Music","subtitle":"Daft Punk","duration":274000}]}"#,
        )
        .unwrap();
        let (name, t) = parse_spotify_entity(&album).unwrap();
        assert_eq!(name, "RAM · Daft Punk");
        assert_eq!((t[0].album.as_deref(), t[0].cover_url.as_deref()), (Some("RAM"), Some("https://i/640")));
        let track: Value = serde_json::from_str(
            r#"{"type":"track","name":"Get Lucky","uri":"spotify:track:2Foc","duration":247632,"artists":[{"name":"Daft Punk"},{"name":"Pharrell Williams"}]}"#,
        )
        .unwrap();
        let (_, t) = parse_spotify_entity(&track).unwrap();
        assert_eq!((t[0].id.as_str(), t[0].artist.as_str()), ("2Foc", "Daft Punk, Pharrell Williams"));
    }

    #[test]
    fn youtube_playlist_page() {
        let v: Value = serde_json::from_str(
            r#"{"header":{"pageHeaderRenderer":{"pageTitle":"Top 100"}},"contents":[
              {"lockupViewModel":{"contentId":"fcnDmrtj6Sk","contentType":"LOCKUP_CONTENT_TYPE_VIDEO",
                "contentImage":{"thumbnailViewModel":{"overlays":[{"thumbnailBottomOverlayViewModel":{"badges":[{"thumbnailBadgeViewModel":{"text":"4:01"}}]}}]}},
                "metadata":{"lockupMetadataViewModel":{"title":{"content":"Shakira, Burna Boy - Dai Dai (Official Video)"},
                  "metadata":{"contentMetadataViewModel":{"metadataRows":[{"metadataParts":[{"text":{"content":"Shakira and 2 more"}}]}]}}}}}},
              {"lockupViewModel":{"contentId":"PLx","contentType":"LOCKUP_CONTENT_TYPE_PLAYLIST"}},
              {"playlistVideoRenderer":{"videoId":"abcdefghijk","title":{"runs":[{"text":"Old Row"}]},"shortBylineText":{"runs":[{"text":"Band - Topic"}]},"lengthSeconds":"200"}},
              {"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"NEXT"}}}}]}"#,
        )
        .unwrap();
        let (t, cont) = parse_youtube_playlist(&v);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].id, "fcnDmrtj6Sk");
        assert_eq!(t[0].duration_ms, Some(241_000));
        assert_eq!(t[1].duration_ms, Some(200_000));
        assert_eq!(cont.as_deref(), Some("NEXT"));
        assert_eq!(youtube_title(&v).as_deref(), Some("Top 100"));
        assert_eq!(first_artist("Shakira and 2 more"), "Shakira");
        assert_eq!(first_artist("Simon and Garfunkel"), "Simon and Garfunkel");
    }

    #[test]
    fn pasted_text() {
        let text = "https://open.spotify.com/track/3XmwAZpIMNpTJy8faw3ajD\r\nhttps://open.spotify.com/track/2Foc5Q5nqNiosCNqttzHof?si=1\n\
                    spotify:track:3XmwAZpIMNpTJy8faw3ajD https://youtu.be/dQw4w9WgXcQ, https://www.deezer.com/track/3135556\n\
                    https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M  hello https://www.youtube.com/playlist?list=PLabc";
        let p = parse_pasted(text);
        assert_eq!(
            p.songs,
            [
                SongLink::Spotify("3XmwAZpIMNpTJy8faw3ajD".into()),
                SongLink::Spotify("2Foc5Q5nqNiosCNqttzHof".into()),
                SongLink::YouTube("dQw4w9WgXcQ".into()),
                SongLink::Deezer("3135556".into()),
            ]
        );
        assert_eq!(p.lists.len(), 2);
        assert!(parse_pasted("just words").is_empty());
    }

    #[test]
    fn parse_pasted_1500_spotify_links_is_fast_and_preserves_order() {
        let mut text = String::new();
        for i in 0..1500 {
            text.push_str(&format!("https://open.spotify.com/track/trackid{i:015}\n"));
        }
        // Duplicate the first 200 to verify O(N) dedup preserves first occurrence order.
        for i in 0..200 {
            text.push_str(&format!("spotify:track:trackid{i:015}\n"));
        }
        let started = std::time::Instant::now();
        let parsed = parse_pasted(&text);
        let elapsed = started.elapsed();
        assert_eq!(parsed.songs.len(), 1500);
        assert_eq!(parsed.songs[0], SongLink::Spotify("trackid000000000000000".into()));
        assert_eq!(parsed.songs[1499], SongLink::Spotify("trackid000000000001499".into()));
        assert!(elapsed < std::time::Duration::from_millis(500), "parse_pasted took {elapsed:?}");
    }

    #[tokio::test]
    async fn read_songs_reuses_persistent_cache_for_1500_tracks() {
        let _g = crate::paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GLYPH_CACHE_DIR", dir.path());
        let mut seeded = std::collections::HashMap::new();
        let mut links = Vec::with_capacity(1500);
        for i in 0..1500 {
            let id = format!("id{i:020}");
            let link = SongLink::Spotify(id.clone());
            seeded.insert(link.cache_key(), spotify_track(&id, &format!("Song {i}"), "Artist", None, Some(180_000), None, false));
            links.push(link);
        }
        save_import_track_cache(&import_track_cache_path(), seeded);
        let http = reqwest::Client::new();
        let dz = crate::providers::deezer::Deezer::new(http.clone());
        let started = std::time::Instant::now();
        let tracks = read_songs(&links, None, &dz, &http).await;
        let elapsed = started.elapsed();
        assert_eq!(tracks.len(), 1500);
        assert_eq!(tracks[0].title, "Song 0");
        assert_eq!(tracks[1499].title, "Song 1499");
        assert!(elapsed < std::time::Duration::from_secs(1), "cached 1500 read took {elapsed:?}");
        std::env::remove_var("GLYPH_CACHE_DIR");
    }

    #[test]
    fn rate_gate_paces_large_batches_dedupes_429_and_recovers() {
        use std::sync::atomic::Ordering;
        // Large 1,000-song Spotify paste starts proactively paced under 180 req/min.
        let big = RateGate::for_spotify(1_000);
        assert_eq!(big.limit.load(Ordering::SeqCst), 4);
        assert!(big.interval_ms.load(Ordering::Relaxed) >= 350, "must pace under ~180 req/min");

        // Concurrent 429s from the same burst deduplicate instead of collapsing immediately.
        big.on_rate_limited(Some(45));
        assert_eq!(big.limit.load(Ordering::SeqCst), 2);
        let gap_after_first = big.interval_ms.load(Ordering::Relaxed);
        assert!(gap_after_first > 380);
        let paused = big.paused_until.lock().unwrap().unwrap();
        // Honors full Retry-After (45 s), not prematurely clamped to 15 s.
        assert!(paused.saturating_duration_since(std::time::Instant::now()) >= std::time::Duration::from_secs(40));
        // Second 429 within the 2 s burst window does not double-penalize concurrency or gap.
        big.on_rate_limited(Some(10));
        assert_eq!(big.limit.load(Ordering::SeqCst), 2);
        assert_eq!(big.interval_ms.load(Ordering::Relaxed), gap_after_first);

        // Next distinct 429 incident drops concurrency all the way to 1.
        *big.last_429.lock().unwrap() = Some(std::time::Instant::now() - std::time::Duration::from_secs(5));
        big.on_rate_limited(None);
        assert_eq!(big.limit.load(Ordering::SeqCst), 1);

        // Clean streak recovers concurrency (+1) and tightens pacing gently.
        for _ in 0..40 {
            big.on_ok();
        }
        assert_eq!(big.limit.load(Ordering::SeqCst), 2);
        assert!(big.interval_ms.load(Ordering::Relaxed) >= big.base_interval_ms);

        // Deezer gate stays safely under 50 req / 5 s.
        let dz = RateGate::for_deezer();
        assert!(dz.interval_ms.load(Ordering::Relaxed) >= 120);
    }
}
