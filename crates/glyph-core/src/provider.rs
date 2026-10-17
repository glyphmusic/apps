//! The [`MusicProvider`] trait every source implements, plus shared helpers
//! (error type, JSON helpers, cooldown tracking).
//!
//! Hard rules: official APIs only, no scraping, no Innertube. The one
//! exception (user decision, 2026-10-17): YouTube may run through the user's
//! own yt-dlp on their machine (`crate::ytdlp`) — validated video ids and
//! search queries only, no cookies, no login, never on a server.

use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::{Capabilities, PlaybackSource, ProviderId, Track};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a failing / quota-exhausted provider is skipped.
pub const COOLDOWN: Duration = Duration::from_secs(10 * 60);
/// Per-provider search timeout.
pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(4);

/// A search hit for the resolver, with a provider-supplied score adjustment
/// (e.g. YouTube "Topic" channels, "Official Audio" titles).
#[derive(Debug, Clone)]
pub struct Candidate {
    pub track: Track,
    /// Added to the match confidence (can be negative).
    pub bonus: f32,
}

#[async_trait]
pub trait MusicProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> Capabilities; // MetadataOnly | Preview | FullPlayback
    async fn search(&self, query: &str) -> Result<Vec<Track>>;
    async fn track(&self, id: &str) -> Result<Track>;
    async fn playback_source(&self, track: &Track) -> Result<PlaybackSource>;

    /// Longer search timeout than [`SEARCH_TIMEOUT`] (local tools).
    fn search_timeout(&self) -> Option<Duration> {
        None
    }

    /// Longer timeout for `playback_source` than the resolver default.
    fn source_timeout(&self) -> Option<Duration> {
        None
    }

    /// Search used by the resolver; providers may attach score bonuses.
    async fn search_candidates(&self, query: &str) -> Result<Vec<Candidate>> {
        Ok(self.search(query).await?.into_iter().map(|track| Candidate { track, bonus: 0.0 }).collect())
    }

    /// A file URL the source allows saving for offline use, `None` when the
    /// song may only be streamed (the default for every provider).
    async fn download_url(&self, track: &Track) -> Result<Option<String>> {
        let _ = track;
        Ok(None)
    }

    /// Exact lookup by ISRC (Spotify, Deezer, MusicBrainz).
    async fn find_by_isrc(&self, isrc: &str) -> Result<Option<Track>> {
        let _ = isrc;
        Ok(None)
    }
}

/// Errors with meaning for the cooldown logic.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("quota exhausted or rate limited")]
    Quota,
    #[error("HTTP {0}")]
    Http(u16),
    #[error("not found")]
    NotFound,
    #[error("not supported by this provider")]
    Unsupported,
    #[error("timed out")]
    Timeout,
    #[error("{0}")]
    Other(String),
}

/// `true` if this error should put the provider on cooldown (a plain "not
/// found" or "unsupported" must not).
pub fn should_cool_down(err: &anyhow::Error) -> bool {
    match err.downcast_ref::<ProviderError>() {
        Some(ProviderError::NotFound) | Some(ProviderError::Unsupported) => false,
        _ => true,
    }
}

/// The id of `track` at `provider` (its own id or an entry of `external_ids`).
pub fn id_at(provider: ProviderId, track: &Track) -> Option<&str> {
    if track.source == provider {
        Some(&track.id)
    } else {
        track.external_ids.get(&provider).map(String::as_str)
    }
}

/// Requests per second (and burst) allowed per API host, so a few quick
/// searches never hit a source's rate limit.
fn host_rate(host: &str) -> (f32, f32) {
    if host.contains("musicbrainz.org") {
        (1.0, 1.0)
    } else if host.contains("audius") {
        (2.0, 4.0)
    } else if host.contains("archive.org") {
        (2.0, 4.0)
    } else if host.contains("deezer.com") {
        (8.0, 10.0)
    } else if host.contains("googleapis.com") {
        (3.0, 5.0)
    } else {
        (5.0, 8.0)
    }
}

/// Token bucket per host: waits (briefly) instead of sending too fast.
pub async fn throttle(host: &str) {
    static BUCKETS: std::sync::OnceLock<Mutex<HashMap<String, (f32, Instant)>>> = std::sync::OnceLock::new();
    let (rate, burst) = host_rate(host);
    loop {
        let wait = {
            let mut m = BUCKETS.get_or_init(Default::default).lock().unwrap();
            let now = Instant::now();
            let (tokens, at) = m.entry(host.to_string()).or_insert((burst, now));
            *tokens = (*tokens + now.duration_since(*at).as_secs_f32() * rate).min(burst);
            *at = now;
            if *tokens >= 1.0 {
                *tokens -= 1.0;
                None
            } else {
                Some(Duration::from_secs_f32((1.0 - *tokens) / rate))
            }
        };
        match wait {
            None => return,
            Some(d) => tokio::time::sleep(d).await,
        }
    }
}

/// Sends a request and parses the JSON body, mapping HTTP statuses to [`ProviderError`].
pub async fn send_json<T: DeserializeOwned>(req: reqwest::RequestBuilder) -> Result<T> {
    let (client, req) = req.build_split();
    let req = req?;
    if let Some(host) = req.url().host_str() {
        throttle(host).await;
    }
    let resp = client.execute(req).await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            429 | 403 => ProviderError::Quota,
            404 => ProviderError::NotFound,
            s => ProviderError::Http(s),
        }
        .into());
    }
    Ok(resp.json::<T>().await?)
}

// ------------------------------------------------------------ serde helpers

fn value_to_string(v: serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// String or number -> String.
pub fn de_flex_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    value_to_string(v).ok_or_else(|| serde::de::Error::custom("expected string or number"))
}

/// Optional string/number; null or other types -> None.
pub fn de_opt_flex_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(value_to_string(serde_json::Value::deserialize(d)?))
}

/// String, or array of strings (first element) -> Option<String>.
pub fn de_flex_first_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Array(a) => a.into_iter().find_map(value_to_string),
        other => value_to_string(other),
    })
}

/// Number or numeric string -> Option<f64>.
pub fn de_opt_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

/// Seconds (possibly fractional) -> milliseconds.
pub fn secs_to_ms(s: f64) -> Option<u64> {
    (s.is_finite() && s > 0.0).then(|| (s * 1000.0).round() as u64)
}

/// Minimal HTML entity decoding for API titles (`&amp;`, `&#39;`, ...).
pub fn unescape_html(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if let Some(end) = rest.find(';').filter(|e| *e <= 10) {
            let ent = &rest[1..end];
            let rep = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => ent
                    .strip_prefix('#')
                    .and_then(|n| {
                        if let Some(h) = n.strip_prefix('x').or_else(|| n.strip_prefix('X')) {
                            u32::from_str_radix(h, 16).ok()
                        } else {
                            n.parse().ok()
                        }
                    })
                    .and_then(char::from_u32),
            };
            if let Some(c) = rep {
                out.push(c);
                rest = &rest[end + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------- cooldowns

/// Tracks providers that are skipped for [`COOLDOWN`] after errors / exhausted quota.
#[derive(Debug, Default)]
pub struct Cooldowns {
    until: Mutex<HashMap<ProviderId, Instant>>,
}

impl Cooldowns {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trip(&self, p: ProviderId) {
        self.trip_for(p, COOLDOWN);
    }

    pub fn trip_for(&self, p: ProviderId, d: Duration) {
        tracing::warn!(provider = ?p, secs = d.as_secs(), "provider put on cooldown");
        self.until.lock().unwrap().insert(p, Instant::now() + d);
    }

    pub fn is_cooling(&self, p: ProviderId) -> bool {
        let mut m = self.until.lock().unwrap();
        match m.get(&p) {
            Some(t) if *t > Instant::now() => true,
            Some(_) => {
                m.remove(&p);
                false
            }
            None => false,
        }
    }

    pub fn clear(&self, p: ProviderId) {
        self.until.lock().unwrap().remove(&p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_expires() {
        let c = Cooldowns::new();
        assert!(!c.is_cooling(ProviderId::Audius));
        c.trip_for(ProviderId::Audius, Duration::from_millis(30));
        assert!(c.is_cooling(ProviderId::Audius));
        std::thread::sleep(Duration::from_millis(50));
        assert!(!c.is_cooling(ProviderId::Audius));
        c.trip(ProviderId::Jamendo);
        assert!(c.is_cooling(ProviderId::Jamendo));
        assert!(!c.is_cooling(ProviderId::Audius));
    }

    #[test]
    fn cooldown_policy() {
        assert!(!should_cool_down(&ProviderError::NotFound.into()));
        assert!(should_cool_down(&ProviderError::Quota.into()));
        assert!(should_cool_down(&anyhow::anyhow!("connection reset")));
    }

    #[test]
    fn unescape() {
        assert_eq!(unescape_html("Tom &amp; Jerry&#39;s &quot;x&quot;"), "Tom & Jerry's \"x\"");
        assert_eq!(unescape_html("a & b"), "a & b");
    }
}
