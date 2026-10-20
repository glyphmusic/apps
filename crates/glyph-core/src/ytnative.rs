//! YouTube on this computer without starting a program: r-dlp (Glyph's own
//! resolver, in process) for the audio URL, and YouTube's own search endpoint
//! for finding a song's video.
//!
//! Both skip what makes yt-dlp slow for a music player (a Python start-up
//! per call, a JS runtime, generic extractors): a resolve takes well under a
//! second and a search a few hundred milliseconds. yt-dlp stays the fallback
//! when either fails.

use crate::provider::{Candidate, ProviderError};
use crate::ytdlp::LocalAudio;
use anyhow::{anyhow, Result};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Version of the built-in r-dlp (shown in the settings).
pub const RDLP_VERSION: &str = "0.1.1";

/// One resolver for the whole app: it keeps the HTTP connection and the
/// anonymous visitor context between songs.
fn resolver() -> Result<Arc<r_dlp::Resolver>> {
    static R: OnceLock<Option<Arc<r_dlp::Resolver>>> = OnceLock::new();
    R.get_or_init(|| match r_dlp::Resolver::new() {
        Ok(r) => Some(Arc::new(r)),
        Err(e) => {
            tracing::warn!("r-dlp unavailable: {e}");
            None
        }
    })
    .clone()
    .ok_or_else(|| anyhow!("r-dlp could not start"))
}

/// Opens the connection ahead of the first song (best effort, background).
pub fn warm_up() {
    static DONE: OnceLock<()> = OnceLock::new();
    if DONE.set(()).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("r-dlp-warmup".into())
        .spawn(|| {
            if let Ok(r) = resolver() {
                let started = Instant::now();
                match r.warmup() {
                    Ok(()) => tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, "r-dlp warmed up"),
                    Err(e) => tracing::debug!("r-dlp warmup failed: {e}"),
                }
            }
        })
        .ok();
}

/// Direct audio URL of a video through r-dlp.
pub async fn resolve(video_id: &str) -> Result<LocalAudio> {
    let r = resolver()?;
    let id = video_id.to_string();
    let started = Instant::now();
    let audio = tokio::task::spawn_blocking(move || r.resolve(&id).map_err(|e| e.to_string()))
        .await
        .map_err(|e| anyhow!("r-dlp task failed: {e}"))?
        .map_err(|e| {
            let lower = e.to_lowercase();
            if lower.contains("unplayable") || lower.contains("unavailable") || lower.contains("private") {
                anyhow::Error::from(ProviderError::NotFound)
            } else {
                anyhow!(ProviderError::Other(format!("r-dlp: {e}")))
            }
        })?;
    tracing::info!(video_id, elapsed_ms = started.elapsed().as_millis() as u64, "r-dlp resolved");
    if !audio.url.starts_with("https://") {
        anyhow::bail!("unexpected stream URL");
    }
    let now = crate::paths::now_unix();
    Ok(LocalAudio {
        expires: crate::ytdlp::url_expiry(&audio.url).unwrap_or(now + 3600),
        video_id: audio.id,
        mime: Some(if audio.mime_type.contains("webm") { "audio/webm" } else { "audio/mp4" }.to_string()),
        url: audio.url,
        duration_ms: audio.duration_seconds.and_then(crate::provider::secs_to_ms),
        title: audio.title,
        channel: String::new(),
        // The URL is signed for the client r-dlp poses as.
        headers: vec![("User-Agent".into(), r_dlp::USER_AGENT.into())],
    })
}

// ------------------------------------------------------------------ search

const SEARCH_URL: &str = "https://www.youtube.com/youtubei/v1/search?prettyPrint=false";
/// Videos only (YouTube's own filter parameter).
const VIDEOS_ONLY: &str = "EgIQAQ==";
const WEB_CLIENT_VERSION: &str = "2.20260930.01.00";

/// After a failed search the endpoint is left alone for a while, so a
/// blocked or changed API does not slow every lookup down.
static SEARCH_BROKEN_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

fn search_paused() -> bool {
    SEARCH_BROKEN_UNTIL.lock().unwrap().is_some_and(|t| Instant::now() < t)
}

fn pause_search() {
    *SEARCH_BROKEN_UNTIL.lock().unwrap() = Some(Instant::now() + Duration::from_secs(10 * 60));
}

/// `"1:02:03"` / `"3:45"` -> ms.
fn parse_length(s: &str) -> Option<u64> {
    let mut total = 0u64;
    for part in s.trim().split(':') {
        total = total * 60 + part.trim().parse::<u64>().ok()?;
    }
    (total > 0).then_some(total * 1000)
}

fn text_of(v: &serde_json::Value) -> Option<String> {
    if let Some(s) = v["simpleText"].as_str() {
        return Some(s.to_string());
    }
    let runs = v["runs"].as_array()?;
    let s: String = runs.iter().filter_map(|r| r["text"].as_str()).collect();
    (!s.is_empty()).then_some(s)
}

/// Every `videoRenderer` in a search response, in order.
fn video_renderers<'a>(v: &'a serde_json::Value, out: &mut Vec<&'a serde_json::Value>) {
    match v {
        serde_json::Value::Object(m) => {
            if let Some(r) = m.get("videoRenderer") {
                out.push(r);
                return;
            }
            for x in m.values() {
                video_renderers(x, out);
            }
        }
        serde_json::Value::Array(a) => {
            for x in a {
                video_renderers(x, out);
            }
        }
        _ => {}
    }
}

/// Parses a search response into resolver candidates (live streams and
/// broken entries left out).
pub fn parse_search(json: &serde_json::Value, n: usize) -> Vec<Candidate> {
    let mut found = Vec::new();
    video_renderers(json, &mut found);
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for r in found {
        let Some(id) = r["videoId"].as_str().filter(|id| crate::ytdlp::valid_video_id(id)) else { continue };
        if !seen.insert(id.to_string()) {
            continue;
        }
        let Some(title) = text_of(&r["title"]) else { continue };
        // No length text: a live stream or an upcoming premiere.
        let Some(dur) = r["lengthText"].as_object().and_then(|_| text_of(&r["lengthText"])).and_then(|s| parse_length(&s)) else { continue };
        let channel = text_of(&r["ownerText"]).or_else(|| text_of(&r["longBylineText"])).unwrap_or_default();
        let (track, bonus) = crate::providers::youtube::video_track(id, &title, &channel, None, Some(dur));
        out.push(Candidate { track, bonus });
        if out.len() >= n {
            break;
        }
    }
    out
}

/// YouTube search through its own web endpoint (no program, no API key).
pub async fn search(http: &reqwest::Client, query: &str, n: usize) -> Result<Vec<Candidate>> {
    if search_paused() {
        anyhow::bail!("native YouTube search paused after an error");
    }
    let body = serde_json::json!({
        "context": {"client": {"clientName": "WEB", "clientVersion": WEB_CLIENT_VERSION, "hl": "en", "gl": "US"}},
        "query": query,
        "params": VIDEOS_ONLY,
    });
    let started = Instant::now();
    let res = async {
        let resp = http
            .post(SEARCH_URL)
            .header("X-Youtube-Client-Name", "1")
            .header("X-Youtube-Client-Version", WEB_CLIENT_VERSION)
            .header("Origin", "https://www.youtube.com")
            .json(&body)
            .timeout(Duration::from_secs(6))
            .send()
            .await?
            .error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        Ok::<_, anyhow::Error>(parse_search(&json, n))
    }
    .await;
    match res {
        Ok(out) if !out.is_empty() => {
            tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, results = out.len(), "native YouTube search");
            Ok(out)
        }
        Ok(_) => Err(ProviderError::NotFound.into()),
        Err(e) => {
            pause_search();
            Err(e.context("native YouTube search failed"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert_eq!(parse_length("3:45"), Some(225_000));
        assert_eq!(parse_length("1:02:03"), Some(3_723_000));
        assert_eq!(parse_length("LIVE"), None);
    }

    #[test]
    fn parses_video_renderers_anywhere() {
        let json = serde_json::json!({
            "contents": {"twoColumnSearchResultsRenderer": {"primaryContents": {"sectionListRenderer": {"contents": [
                {"itemSectionRenderer": {"contents": [
                    {"videoRenderer": {"videoId": "dQw4w9WgXcQ", "title": {"runs": [{"text": "Rick Astley - Never Gonna Give You Up (Official Video)"}]},
                        "ownerText": {"runs": [{"text": "Rick Astley"}]}, "lengthText": {"simpleText": "3:33"}}},
                    {"videoRenderer": {"videoId": "liveliveliv", "title": {"runs": [{"text": "24/7 radio"}]}}},
                    {"shelfRenderer": {"content": {"verticalListRenderer": {"items": [
                        {"videoRenderer": {"videoId": "9bZkp7q19f0", "title": {"simpleText": "PSY - GANGNAM STYLE"},
                            "longBylineText": {"runs": [{"text": "officialpsy"}]}, "lengthText": {"simpleText": "4:13"}}}
                    ]}}}}
                ]}}
            ]}}}}
        });
        let c = parse_search(&json, 10);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].track.id, "dQw4w9WgXcQ");
        assert_eq!(c[0].track.artist, "Rick Astley");
        assert_eq!(c[0].track.title, "Never Gonna Give You Up");
        assert_eq!(c[0].track.duration_ms, Some(213_000));
        assert_eq!(c[1].track.id, "9bZkp7q19f0");
        assert_eq!(parse_search(&json, 1).len(), 1);
    }
}
