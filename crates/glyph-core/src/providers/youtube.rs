//! YouTube. With a local yt-dlp (see [`crate::ytdlp`]) search and playback
//! run on this machine: `ytsearch` results and a direct audio URL played by
//! glyph-audio (`PlaybackSource::Stream`), no API quota. Without it: Data API
//! v3 search (`videoCategoryId=10`, embeddable only) and the official IFrame
//! player (`PlaybackSource::Embed`, `embed_id` = video id). The embed is also
//! the fallback when yt-dlp fails for a video.
//!
//! Quota: `search.list` costs 100 units of 10 000 per day, so searches are
//! cached in memory (10 min) and counted against a persisted daily budget.

use crate::provider::*;
use crate::paths;
use crate::ytdlp::YtDlp;
use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::{Capabilities, PlaybackSource, ProviderId, Track};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BASE: &str = "https://www.googleapis.com/youtube/v3";
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
/// yt-dlp starts a Python bundle and solves player challenges: give it time.
const LOCAL_SEARCH_TIMEOUT: Duration = Duration::from_secs(15);
const LOCAL_SOURCE_TIMEOUT: Duration = Duration::from_secs(25);

pub struct YouTube {
    http: reqwest::Client,
    /// Data API key (search without yt-dlp, playlist import).
    key: Option<String>,
    daily_budget: u32,
    local: Option<YtDlp>,
    cache: Mutex<HashMap<String, (Instant, Vec<Candidate>)>>,
}

#[derive(Debug, Deserialize)]
struct SearchResp {
    #[serde(default)]
    items: Vec<SearchItem>,
}

#[derive(Debug, Deserialize)]
struct SearchItem {
    id: SearchId,
    snippet: Snippet,
}

#[derive(Debug, Deserialize)]
struct SearchId {
    #[serde(rename = "videoId")]
    video_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Snippet {
    #[serde(default)]
    title: String,
    #[serde(default, rename = "channelTitle")]
    channel_title: String,
    #[serde(default)]
    thumbnails: Option<Thumbs>,
}

#[derive(Debug, Deserialize)]
struct Thumbs {
    high: Option<Thumb>,
    medium: Option<Thumb>,
    default: Option<Thumb>,
}

#[derive(Debug, Deserialize)]
struct Thumb {
    url: String,
}

#[derive(Debug, Deserialize)]
struct VideosResp {
    #[serde(default)]
    items: Vec<VideoItem>,
}

#[derive(Debug, Deserialize)]
struct VideoItem {
    id: String,
    #[serde(default)]
    snippet: Option<Snippet>,
    #[serde(default, rename = "contentDetails")]
    content_details: Option<ContentDetails>,
}

#[derive(Debug, Deserialize)]
struct ContentDetails {
    #[serde(default)]
    duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PlaylistItemsResp {
    #[serde(default)]
    items: Vec<PlaylistItem>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PlaylistItem {
    snippet: PlaylistSnippet,
}

#[derive(Debug, Deserialize)]
struct PlaylistSnippet {
    #[serde(default)]
    title: String,
    #[serde(default, rename = "videoOwnerChannelTitle")]
    owner: Option<String>,
    #[serde(default, rename = "resourceId")]
    resource_id: Option<ResourceId>,
    #[serde(default)]
    thumbnails: Option<Thumbs>,
}

#[derive(Debug, Deserialize)]
struct ResourceId {
    #[serde(default, rename = "videoId")]
    video_id: Option<String>,
}

/// `PT1H2M3S` -> ms. Live streams (`P0D`) and garbage -> None.
pub fn parse_iso8601_duration(s: &str) -> Option<u64> {
    let rest = s.strip_prefix('P')?;
    let (date, time) = rest.split_once('T').unwrap_or((rest, ""));
    let mut total = 0f64;
    let mut num = String::new();
    for c in date.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            let n: f64 = num.parse().ok()?;
            num.clear();
            match c {
                'D' => total += n * 86_400.0,
                'W' => total += n * 604_800.0,
                _ => return None,
            }
        }
    }
    for c in time.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else {
            let n: f64 = num.parse().ok()?;
            num.clear();
            match c {
                'H' => total += n * 3600.0,
                'M' => total += n * 60.0,
                'S' => total += n,
                _ => return None,
            }
        }
    }
    secs_to_ms(total)
}

const NOISE_WORDS: [&str; 12] = [
    "official", "lyric", "lyrics", "audio", "video", "music video", "hd", "hq", "4k", "visualizer", "remastered",
    "explicit",
];

/// Removes trailing "(Official Video)"-style decorations from a video title.
pub fn clean_video_title(raw: &str) -> String {
    let mut s = unescape_html(raw).trim().to_string();
    loop {
        let trimmed = s.trim_end();
        let Some(last) = trimmed.chars().last() else { break };
        let (open, close) = match last {
            ')' => ('(', ')'),
            ']' => ('[', ']'),
            _ => break,
        };
        let Some(start) = trimmed.rfind(open) else { break };
        let inner = trimmed[start + 1..trimmed.len() - close.len_utf8()].to_lowercase();
        if NOISE_WORDS.iter().any(|w| inner.contains(w)) && start > 0 {
            s = trimmed[..start].trim_end().to_string();
        } else {
            break;
        }
    }
    s
}

/// Splits a video into (artist, title). Topic channels carry the clean
/// metadata; otherwise "Artist - Title" titles are split.
/// Returns (artist, title, is_topic).
pub fn split_video(title: &str, channel: &str) -> (String, String, bool) {
    let title = clean_video_title(title);
    let is_topic = channel.ends_with(" - Topic");
    if is_topic {
        return (channel.trim_end_matches(" - Topic").trim().to_string(), title, true);
    }
    if let Some((a, t)) = title.split_once(" - ") {
        if !a.trim().is_empty() && !t.trim().is_empty() {
            return (a.trim().to_string(), t.trim().to_string(), false);
        }
    }
    let channel = channel.trim_end_matches("VEVO").trim().to_string();
    (channel, title, false)
}

fn bonus_for(raw_title: &str, is_topic: bool) -> f32 {
    let lower = raw_title.to_lowercase();
    let mut b = 0.0;
    if is_topic {
        b += 0.12;
    }
    if lower.contains("official audio") {
        b += 0.08;
    }
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
    let bad_word = ["live", "cover", "remix", "reaction", "karaoke", "slowed", "8d", "nightcore", "tutorial"]
        .iter()
        .any(|w| words.contains(w));
    if bad_word || lower.contains("sped up") {
        b -= 0.12;
    }
    b
}

fn thumb_url(t: &Option<Thumbs>, id: &str) -> String {
    t.as_ref()
        .and_then(|t| t.high.as_ref().or(t.medium.as_ref()).or(t.default.as_ref()))
        .map(|t| t.url.clone())
        .unwrap_or_else(|| format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))
}

fn make_track(id: &str, raw_title: &str, channel: &str, thumbs: &Option<Thumbs>, dur: Option<u64>) -> (Track, f32) {
    video_track(id, raw_title, channel, Some(thumb_url(thumbs, id)), dur)
}

/// Track + resolver bonus for a video (cover defaults to the `hqdefault` still).
pub(crate) fn video_track(id: &str, raw_title: &str, channel: &str, cover: Option<String>, dur: Option<u64>) -> (Track, f32) {
    let (artist, title, is_topic) = split_video(raw_title, channel);
    let bonus = bonus_for(&unescape_html(raw_title), is_topic);
    (
        Track {
            id: id.to_string(),
            source: ProviderId::YouTube,
            title,
            artist,
            album: None,
            duration_ms: dur,
            cover_url: Some(cover.unwrap_or_else(|| format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))),
            isrc: None,
            permalink: Some(format!("https://www.youtube.com/watch?v={id}")),
            external_ids: Default::default(),
            explicit: false,
        },
        bonus,
    )
}

#[derive(Serialize, Deserialize, Default)]
struct BudgetFile {
    day: u64,
    searches: u32,
}

/// Quota day (Google resets at midnight Pacific; approximated as UTC-8).
fn quota_day() -> u64 {
    (paths::now_unix().saturating_sub(8 * 3600)) / 86_400
}

impl YouTube {
    pub fn new(http: reqwest::Client, key: Option<String>, daily_budget: u32, local: Option<YtDlp>) -> Self {
        Self { http, key, daily_budget, local, cache: Mutex::new(HashMap::new()) }
    }

    /// yt-dlp runs on this machine (search + direct audio).
    pub fn local(&self) -> Option<&YtDlp> {
        self.local.as_ref()
    }

    fn key(&self) -> Result<&str> {
        self.key.as_deref().ok_or_else(|| ProviderError::Other("YouTube API key missing".into()).into())
    }

    /// The song through the local yt-dlp, as a fully downloaded file in the
    /// audio cache (googlevideo refuses streams read at playback pace).
    pub async fn local_source(&self, video_id: &str) -> Result<PlaybackSource> {
        let yt = self.local.as_ref().ok_or(ProviderError::Unsupported)?;
        let path = yt.audio_file(&self.http, video_id).await?;
        let mime = Some(crate::ytdlp::mime_for_path(&path).to_string());
        Ok(PlaybackSource::Stream { provider: ProviderId::YouTube, url: path.to_string_lossy().into_owned(), mime })
    }

    fn budget_path() -> std::path::PathBuf {
        paths::data_dir().join("youtube_quota.json")
    }

    /// Counts one `search.list` call against the daily budget.
    fn spend_search(&self) -> Result<()> {
        let path = Self::budget_path();
        let mut b: BudgetFile = paths::read_json(&path).unwrap_or_default();
        let today = quota_day();
        if b.day != today {
            b = BudgetFile { day: today, searches: 0 };
        }
        if b.searches >= self.daily_budget {
            return Err(ProviderError::Quota.into());
        }
        b.searches += 1;
        let _ = paths::write_json(&path, &b);
        Ok(())
    }

    async fn durations(&self, ids: &[String]) -> HashMap<String, u64> {
        if ids.is_empty() {
            return HashMap::new();
        }
        let joined = ids.join(",");
        let req = self
            .http
            .get(format!("{BASE}/videos"))
            .query(&[("part", "contentDetails"), ("id", joined.as_str()), ("key", self.key.as_deref().unwrap_or(""))]);
        match send_json::<VideosResp>(req).await {
            Ok(r) => r
                .items
                .into_iter()
                .filter_map(|v| {
                    let d = v.content_details?.duration?;
                    Some((v.id, parse_iso8601_duration(&d)?))
                })
                .collect(),
            Err(e) => {
                tracing::debug!("youtube durations failed: {e}");
                HashMap::new()
            }
        }
    }

    /// Items of a (public) playlist, for import. Needs only the API key.
    pub async fn playlist_items(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let key = self.key()?.to_string();
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..40 {
            let mut q = vec![
                ("part", "snippet".to_string()),
                ("playlistId", playlist_id.to_string()),
                ("maxResults", "50".to_string()),
                ("key", key.clone()),
            ];
            if let Some(t) = &token {
                q.push(("pageToken", t.clone()));
            }
            let r: PlaylistItemsResp = send_json(self.http.get(format!("{BASE}/playlistItems")).query(&q)).await?;
            for it in r.items {
                let Some(id) = it.snippet.resource_id.and_then(|r| r.video_id) else { continue };
                if it.snippet.title == "Deleted video" || it.snippet.title == "Private video" {
                    continue;
                }
                let channel = it.snippet.owner.unwrap_or_default();
                let (t, _) = make_track(&id, &it.snippet.title, &channel, &it.snippet.thumbnails, None);
                out.push(t);
            }
            token = r.next_page_token;
            if token.is_none() {
                break;
            }
        }
        Ok(out)
    }

    pub async fn playlist_title(&self, playlist_id: &str) -> Result<String> {
        let req = self
            .http
            .get(format!("{BASE}/playlists"))
            .query(&[("part", "snippet"), ("id", playlist_id), ("key", self.key()?)]);
        let v: serde_json::Value = send_json(req).await?;
        Ok(v["items"][0]["snippet"]["title"].as_str().unwrap_or("YouTube playlist").to_string())
    }
}

#[async_trait]
impl MusicProvider for YouTube {
    fn id(&self) -> ProviderId {
        ProviderId::YouTube
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::FullPlayback
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        Ok(self.search_candidates(query).await?.into_iter().map(|c| c.track).collect())
    }

    fn search_timeout(&self) -> Option<Duration> {
        self.local.is_some().then_some(LOCAL_SEARCH_TIMEOUT)
    }

    fn source_timeout(&self) -> Option<Duration> {
        self.local.is_some().then_some(LOCAL_SOURCE_TIMEOUT)
    }

    async fn search_candidates(&self, query: &str) -> Result<Vec<Candidate>> {
        let key = query.trim().to_lowercase();
        if let Some((at, hit)) = self.cache.lock().unwrap().get(&key) {
            if at.elapsed() < CACHE_TTL {
                return Ok(hit.clone());
            }
        }
        if let Some(yt) = &self.local {
            let out = yt.search(query, crate::ytdlp::SEARCH_RESULTS).await?;
            let mut cache = self.cache.lock().unwrap();
            cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
            cache.insert(key, (Instant::now(), out.clone()));
            return Ok(out);
        }
        let api_key = self.key()?;
        self.spend_search()?;
        let req = self.http.get(format!("{BASE}/search")).query(&[
            ("part", "snippet"),
            ("type", "video"),
            ("videoCategoryId", "10"),
            ("videoEmbeddable", "true"),
            ("maxResults", "10"),
            ("q", query),
            ("key", api_key),
        ]);
        let r: SearchResp = send_json(req).await?;
        let ids: Vec<String> = r.items.iter().filter_map(|i| i.id.video_id.clone()).collect();
        let durations = self.durations(&ids).await;
        let out: Vec<Candidate> = r
            .items
            .into_iter()
            .filter_map(|i| {
                let id = i.id.video_id?;
                let (track, bonus) = make_track(
                    &id,
                    &i.snippet.title,
                    &i.snippet.channel_title,
                    &i.snippet.thumbnails,
                    durations.get(&id).copied(),
                );
                Some(Candidate { track, bonus })
            })
            .collect();
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
        cache.insert(key, (Instant::now(), out.clone()));
        Ok(out)
    }

    async fn track(&self, id: &str) -> Result<Track> {
        if let (Some(yt), None) = (&self.local, &self.key) {
            let a = yt.resolve(id).await?;
            return Ok(video_track(&a.video_id, &a.title, &a.channel, None, a.duration_ms).0);
        }
        let req = self
            .http
            .get(format!("{BASE}/videos"))
            .query(&[("part", "snippet,contentDetails"), ("id", id), ("key", self.key()?)]);
        let r: VideosResp = send_json(req).await?;
        let v = r.items.into_iter().next().ok_or(ProviderError::NotFound)?;
        let sn = v.snippet.unwrap_or_default();
        let dur = v.content_details.and_then(|c| c.duration).and_then(|d| parse_iso8601_duration(&d));
        Ok(make_track(&v.id, &sn.title, &sn.channel_title, &sn.thumbnails, dur).0)
    }

    async fn playback_source(&self, track: &Track) -> Result<PlaybackSource> {
        let id = id_at(ProviderId::YouTube, track).ok_or(ProviderError::Unsupported)?;
        if self.local.is_some() {
            match self.local_source(id).await {
                Ok(s) => return Ok(s),
                // Removed / private: the embed would fail the same way.
                Err(e) if matches!(e.downcast_ref::<ProviderError>(), Some(ProviderError::NotFound)) => return Err(e),
                Err(e) => tracing::info!("local YouTube failed, using the embed: {e:#}"),
            }
        }
        Ok(PlaybackSource::Embed { provider: ProviderId::YouTube, embed_id: id.to_string() })
    }

    async fn download_url(&self, track: &Track) -> Result<Option<String>> {
        let Some(id) = id_at(ProviderId::YouTube, track) else { return Ok(None) };
        if self.local.is_none() {
            return Ok(None);
        }
        match self.local_source(id).await? {
            PlaybackSource::Stream { url, .. } => Ok(Some(url)),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_durations() {
        assert_eq!(parse_iso8601_duration("PT3M45S"), Some(225_000));
        assert_eq!(parse_iso8601_duration("PT1H2M3S"), Some(3_723_000));
        assert_eq!(parse_iso8601_duration("PT45S"), Some(45_000));
        assert_eq!(parse_iso8601_duration("P0D"), None);
        assert_eq!(parse_iso8601_duration("nonsense"), None);
    }

    #[test]
    fn cleans_titles() {
        assert_eq!(clean_video_title("Song (Official Video)"), "Song");
        assert_eq!(clean_video_title("Song [HD] (Lyrics)"), "Song");
        assert_eq!(clean_video_title("Song (feat. X)"), "Song (feat. X)");
        assert_eq!(clean_video_title("Tom &amp; Jerry"), "Tom & Jerry");
    }

    #[test]
    fn splits_video_metadata() {
        let (a, t, topic) = split_video("Believer", "Imagine Dragons - Topic");
        assert_eq!((a.as_str(), t.as_str(), topic), ("Imagine Dragons", "Believer", true));
        let (a, t, topic) = split_video("Daft Punk - One More Time (Official Audio)", "Daft Punk");
        assert_eq!((a.as_str(), t.as_str(), topic), ("Daft Punk", "One More Time", false));
        let (a, t, _) = split_video("Just A Title", "SomeChannelVEVO");
        assert_eq!((a.as_str(), t.as_str()), ("SomeChannel", "Just A Title"));
    }

    #[test]
    fn bonuses() {
        assert!(bonus_for("Song", true) > 0.1);
        assert!(bonus_for("Song (Official Audio)", false) > 0.0);
        assert!(bonus_for("Song (Live at X)", false) < 0.0);
    }
}
