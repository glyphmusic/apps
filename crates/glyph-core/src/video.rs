//! Music video search and stream resolution (native YouTube web/player APIs,
//! with `glyph-server` integration in `Inner`).

use crate::ytdlp::valid_video_id;
use anyhow::{anyhow, Result};
use glyph_protocol::{VideoCandidate, VideoKind, VideoQuality, VideoResolveResponse, VideoSearchResponse};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

const SEARCH_URL: &str = "https://www.youtube.com/youtubei/v1/search?prettyPrint=false";
const PLAYER_URL: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";
const WEB_CLIENT_VERSION: &str = "2.20260930.01.00";
const VIDEOS_ONLY: &str = "EgIQAQ==";
const VISIONOS_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

pub fn classify_video(title: &str, channel: &str) -> VideoKind {
    let low = title.to_lowercase();
    let ch = channel.to_lowercase();
    if ch.ends_with(" - topic") || low.contains("provided to youtube") {
        return VideoKind::Topic;
    }
    if low.contains("lyric") || low.contains("lyrics") || low.contains("letra") {
        return VideoKind::Lyric;
    }
    if low.contains("live")
        || low.contains("concert")
        || low.contains("tiny desk")
        || low.contains("colors show")
        || low.contains("acoustic")
        || low.contains("session")
        || low.contains("festival")
        || low.contains("unplugged")
    {
        return VideoKind::Live;
    }
    if low.contains("official music video")
        || low.contains("official video")
        || low.contains("music video")
        || low.contains("official mv")
        || low.contains("(mv)")
        || low.contains("[mv]")
        || ch.ends_with("vevo")
    {
        return VideoKind::Official;
    }
    VideoKind::Video
}

fn video_rank(item: &VideoCandidate, artist: &str, title: &str) -> i32 {
    let mut score = match item.kind {
        VideoKind::Official => 40,
        VideoKind::Video => 18,
        VideoKind::Live => 14,
        VideoKind::Lyric => 10,
        VideoKind::Topic => -30,
    };
    let low_title = item.title.to_lowercase();
    let low_ch = item.channel.to_lowercase();
    if low_title.contains("official audio") || low_title.contains("(audio)") {
        score -= 22;
    }
    if !title.trim().is_empty() && low_title.contains(&title.trim().to_lowercase()) {
        score += 18;
    }
    if !artist.trim().is_empty() {
        let a = artist.trim().to_lowercase();
        if low_title.contains(&a) || low_ch.contains(&a) {
            score += 14;
        }
    }
    score
}

fn text_of(v: &Value) -> Option<String> {
    if let Some(s) = v["simpleText"].as_str() {
        return Some(s.to_string());
    }
    let runs = v["runs"].as_array()?;
    let s: String = runs.iter().filter_map(|r| r["text"].as_str()).collect();
    (!s.is_empty()).then_some(s)
}

fn parse_length(s: &str) -> Option<u64> {
    let mut total = 0u64;
    for part in s.trim().split(':') {
        total = total * 60 + part.trim().parse::<u64>().ok()?;
    }
    (total > 0).then_some(total * 1000)
}

fn video_renderers<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            if let Some(r) = m.get("videoRenderer") {
                out.push(r);
                return;
            }
            for x in m.values() {
                video_renderers(x, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                video_renderers(x, out);
            }
        }
        _ => {}
    }
}

pub fn default_query(artist: &str, title: &str) -> String {
    format!("{artist} {title} official music video").trim().to_string()
}

pub async fn search_native(
    http: &reqwest::Client,
    query: &str,
    artist: &str,
    title: &str,
    limit: usize,
) -> Result<VideoSearchResponse> {
    let body = json!({
        "context": {"client": {"clientName": "WEB", "clientVersion": WEB_CLIENT_VERSION, "hl": "en", "gl": "US"}},
        "query": query,
        "params": VIDEOS_ONLY,
    });
    let resp = http
        .post(SEARCH_URL)
        .header("X-Youtube-Client-Name", "1")
        .header("X-Youtube-Client-Version", WEB_CLIENT_VERSION)
        .header("Origin", "https://www.youtube.com")
        .json(&body)
        .timeout(Duration::from_secs(8))
        .send()
        .await?
        .error_for_status()?;
    let v: Value = resp.json().await?;
    let mut found = Vec::new();
    video_renderers(&v, &mut found);
    let mut seen = HashSet::new();
    let mut results = Vec::new();
    for r in found {
        let Some(id) = r["videoId"].as_str().filter(|id| valid_video_id(id)) else {
            continue;
        };
        if !seen.insert(id.to_string()) {
            continue;
        }
        let Some(raw_title) = text_of(&r["title"]) else {
            continue;
        };
        let Some(ms) = text_of(&r["lengthText"]).and_then(|s| parse_length(&s)) else {
            continue;
        };
        let channel = text_of(&r["ownerText"])
            .or_else(|| text_of(&r["longBylineText"]))
            .unwrap_or_default();
        let kind = classify_video(&raw_title, &channel);
        results.push(VideoCandidate {
            id: id.to_string(),
            title: raw_title,
            channel,
            duration_ms: Some(ms),
            thumbnail: Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")),
            kind,
        });
        if results.len() >= limit {
            break;
        }
    }
    results.sort_by(|a, b| video_rank(b, artist, title).cmp(&video_rank(a, artist, title)));
    Ok(VideoSearchResponse {
        query: query.to_string(),
        results,
    })
}

fn valid_googlevideo(raw: &str) -> bool {
    raw.len() <= 60_000
        && reqwest::Url::parse(raw)
            .ok()
            .is_some_and(|u| u.scheme() == "https" && u.host_str().is_some_and(|h| h.ends_with(".googlevideo.com")))
}

/// Resolves direct progressive MP4 + adaptive HD H.264/AAC streams on this
/// device via YouTube's player endpoint (same home-IP path as `r-dlp`).
pub async fn resolve_native(http: &reqwest::Client, video_id: &str) -> Result<VideoResolveResponse> {
    if !valid_video_id(video_id) {
        return Err(anyhow!("invalid video id"));
    }
    let body = json!({
        "context": {
            "client": {
                "clientName": "VISIONOS",
                "clientVersion": "1.02",
                "deviceMake": "Apple",
                "deviceModel": "RealityDevice17,1",
                "osName": "visionOS",
                "osVersion": "26.5.23O471",
                "hl": "en",
                "gl": "US",
                "userAgent": VISIONOS_UA
            }
        },
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
        "playbackContext": {
            "contentPlaybackContext": {
                "html5Preference": "HTML5_PREF_WANTS"
            }
        }
    });
    let resp = http
        .post(PLAYER_URL)
        .header("User-Agent", VISIONOS_UA)
        .header("X-Youtube-Client-Name", "101")
        .header("X-Youtube-Client-Version", "1.02")
        .header("Origin", "https://www.youtube.com")
        .json(&body)
        .timeout(Duration::from_secs(10))
        .send()
        .await?
        .error_for_status()?;
    let info: Value = resp.json().await?;
    let status = info["playabilityStatus"]["status"].as_str().unwrap_or("");
    if status != "OK" {
        return Err(anyhow!("video unavailable ({status})"));
    }

    let details = &info["videoDetails"];
    let title = details["title"].as_str().map(str::to_string);
    let channel = details["author"].as_str().map(str::to_string);
    let duration_ms = details["lengthSeconds"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|s| s * 1000);

    let sd = &info["streamingData"];

    // 1. Progressive MP4 (video + audio together).
    let mut muxed: Vec<(u32, String, String)> = Vec::new();
    if let Some(arr) = sd["formats"].as_array() {
        for f in arr {
            let Some(url) = f["url"].as_str().filter(|u| valid_googlevideo(u)) else { continue };
            let mime = f["mimeType"].as_str().unwrap_or("");
            if !mime.starts_with("video/mp4") {
                continue;
            }
            let h = f["height"].as_u64().unwrap_or(360) as u32;
            muxed.push((h, url.to_string(), "video/mp4".to_string()));
        }
    }
    muxed.sort_by(|a, b| b.0.cmp(&a.0));

    // 2. Best direct AAC audio stream for adaptive HD video pairing.
    let mut best_audio: Option<(u64, String)> = None;
    let mut adaptive_video: Vec<(u32, u64, String)> = Vec::new();
    if let Some(arr) = sd["adaptiveFormats"].as_array() {
        for f in arr {
            let Some(url) = f["url"].as_str().filter(|u| valid_googlevideo(u)) else { continue };
            let mime = f["mimeType"].as_str().unwrap_or("");
            let bitrate = f["bitrate"].as_u64().unwrap_or(0);
            if mime.starts_with("audio/mp4") {
                if f["audioTrack"]["audioIsDefault"].as_bool() == Some(false) {
                    continue;
                }
                if best_audio.as_ref().is_none_or(|(b, _)| bitrate > *b) {
                    best_audio = Some((bitrate, url.to_string()));
                }
            } else if mime.starts_with("video/mp4") && mime.contains("avc1") {
                let h = f["height"].as_u64().unwrap_or(0) as u32;
                if (360..=1080).contains(&h) {
                    adaptive_video.push((h, bitrate, url.to_string()));
                }
            }
        }
    }

    let mut qualities: Vec<VideoQuality> = Vec::new();
    if let Some((_, ref audio_url)) = best_audio {
        // Keep the best bitrate stream per distinct height (1080p, 720p, 480p).
        adaptive_video.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        let mut seen_h = HashSet::new();
        for (h, _, url) in adaptive_video {
            if h >= 480 && seen_h.insert(h) {
                qualities.push(VideoQuality {
                    label: format!("{h}p"),
                    height: h,
                    url,
                    audio_url: Some(audio_url.clone()),
                    mime: Some("video/mp4".into()),
                });
            }
        }
    }
    for (h, url, mime) in &muxed {
        if !qualities.iter().any(|q| q.height == *h && q.audio_url.is_none()) {
            qualities.push(VideoQuality {
                label: format!("{h}p"),
                height: *h,
                url: url.clone(),
                audio_url: None,
                mime: Some(mime.clone()),
            });
        }
    }
    qualities.sort_by(|a, b| b.height.cmp(&a.height));

    let primary = qualities
        .iter()
        .find(|q| q.height == 720)
        .or_else(|| qualities.first())
        .ok_or_else(|| anyhow!("no playable MP4 video stream found"))?;

    Ok(VideoResolveResponse {
        id: video_id.to_string(),
        title,
        channel,
        duration_ms,
        url: primary.url.clone(),
        audio_url: primary.audio_url.clone(),
        mime: primary.mime.clone(),
        qualities,
    })
}
