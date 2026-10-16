//! Last.fm: `track.getSimilar` (radio), `user.getLovedTracks` (import),
//! `track.search`. Metadata only. Needs an API key.

use crate::provider::*;
use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::{Capabilities, PlaybackSource, ProviderId, Track};
use serde::Deserialize;
use serde_json::Value;

const API: &str = "https://ws.audioscrobbler.com/2.0/";

pub struct LastFm {
    http: reqwest::Client,
    key: String,
}

/// Last.fm has no track ids; `artist\u{1f}title` identifies a track.
pub fn make_id(artist: &str, title: &str) -> String {
    format!("{artist}\u{1f}{title}")
}

pub fn split_id(id: &str) -> Option<(&str, &str)> {
    id.split_once('\u{1f}')
}

#[derive(Debug, Deserialize)]
struct LTrack {
    #[serde(default)]
    name: String,
    #[serde(default)]
    artist: Option<Value>,
    #[serde(default, deserialize_with = "de_opt_f64")]
    duration: Option<f64>,
    #[serde(default)]
    url: Option<String>,
}

fn artist_of(v: &Option<Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(o)) => o
            .get("name")
            .or_else(|| o.get("#text"))
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_string(),
        _ => "Unknown".into(),
    }
}

fn convert(t: &LTrack) -> Option<Track> {
    if t.name.is_empty() {
        return None;
    }
    let artist = artist_of(&t.artist);
    Some(Track {
        id: make_id(&artist, &t.name),
        source: ProviderId::LastFm,
        title: t.name.clone(),
        artist,
        album: None,
        // Last.fm reports durations in seconds (sometimes ms in some endpoints); 0 = unknown.
        duration_ms: t.duration.filter(|d| *d > 0.0).map(|d| if d > 10_000.0 { d as u64 } else { (d * 1000.0) as u64 }),
        cover_url: None, // Last.fm image URLs are mostly placeholders; the cover chain fills this in.
        isrc: None,
        permalink: t.url.clone(),
        external_ids: Default::default(),
    })
}

/// Tracks are either a list or (for a single hit) one object.
fn track_list(v: &Value) -> Vec<LTrack> {
    match v {
        Value::Array(a) => a.iter().filter_map(|x| serde_json::from_value(x.clone()).ok()).collect(),
        Value::Object(_) => serde_json::from_value(v.clone()).ok().into_iter().collect(),
        _ => vec![],
    }
}

pub fn parse_similar(json: &str) -> Result<Vec<Track>> {
    let v: Value = serde_json::from_str(json)?;
    Ok(track_list(&v["similartracks"]["track"]).iter().filter_map(convert).collect())
}

pub fn parse_loved(json: &str) -> Result<(Vec<Track>, u32)> {
    let v: Value = serde_json::from_str(json)?;
    let pages = v["lovedtracks"]["@attr"]["totalPages"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v["lovedtracks"]["@attr"]["totalPages"].as_u64().map(|n| n as u32))
        .unwrap_or(1);
    Ok((track_list(&v["lovedtracks"]["track"]).iter().filter_map(convert).collect(), pages))
}

impl LastFm {
    pub fn new(http: reqwest::Client, key: String) -> Self {
        Self { http, key }
    }

    async fn call(&self, method: &str, extra: &[(&str, &str)]) -> Result<Value> {
        let req = self
            .http
            .get(API)
            .query(&[("method", method), ("api_key", self.key.as_str()), ("format", "json")])
            .query(extra);
        let v: Value = send_json(req).await?;
        if let Some(code) = v["error"].as_i64() {
            return Err(match code {
                29 => ProviderError::Quota, // rate limit exceeded
                6 => ProviderError::NotFound,
                _ => ProviderError::Other(v["message"].as_str().unwrap_or("last.fm error").to_string()),
            }
            .into());
        }
        Ok(v)
    }

    /// Similar tracks (radio).
    pub async fn similar(&self, artist: &str, title: &str) -> Result<Vec<Track>> {
        let v = self
            .call("track.getsimilar", &[("artist", artist), ("track", title), ("limit", "30"), ("autocorrect", "1")])
            .await?;
        Ok(track_list(&v["similartracks"]["track"]).iter().filter_map(convert).collect())
    }

    /// Loved tracks of a public profile (up to 2000).
    pub async fn loved_tracks(&self, user: &str) -> Result<Vec<Track>> {
        let mut out = Vec::new();
        let mut page = 1u32;
        loop {
            let p = page.to_string();
            let v = self
                .call("user.getlovedtracks", &[("user", user), ("limit", "200"), ("page", p.as_str())])
                .await?;
            let total = v["lovedtracks"]["@attr"]["totalPages"]
                .as_str()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(1);
            out.extend(track_list(&v["lovedtracks"]["track"]).iter().filter_map(convert));
            if page >= total || page >= 10 {
                break;
            }
            page += 1;
        }
        Ok(out)
    }

    /// Top tracks of a public profile (overall, up to 200).
    pub async fn top_tracks(&self, user: &str) -> Result<Vec<Track>> {
        let v = self
            .call("user.gettoptracks", &[("user", user), ("limit", "200"), ("period", "overall")])
            .await?;
        Ok(track_list(&v["toptracks"]["track"]).iter().filter_map(convert).collect())
    }
}

#[async_trait]
impl MusicProvider for LastFm {
    fn id(&self) -> ProviderId {
        ProviderId::LastFm
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::MetadataOnly
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let v = self.call("track.search", &[("track", query), ("limit", "20")]).await?;
        Ok(track_list(&v["results"]["trackmatches"]["track"]).iter().filter_map(convert).collect())
    }

    async fn track(&self, id: &str) -> Result<Track> {
        let (artist, title) = split_id(id).ok_or(ProviderError::NotFound)?;
        let v = self.call("track.getinfo", &[("artist", artist), ("track", title), ("autocorrect", "1")]).await?;
        convert(&serde_json::from_value(v["track"].clone())?).ok_or_else(|| ProviderError::NotFound.into())
    }

    async fn playback_source(&self, _track: &Track) -> Result<PlaybackSource> {
        Err(ProviderError::Unsupported.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_similar() {
        let json = r#"{"similartracks":{"track":[
          {"name":"Hello","duration":"215","artist":{"name":"Adele","url":"x"},"url":"https://last.fm/a"},
          {"name":"","artist":{"name":"X"}}]}}"#;
        let t = parse_similar(json).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].artist, "Adele");
        assert_eq!(t[0].duration_ms, Some(215_000));
        assert_eq!(split_id(&t[0].id), Some(("Adele", "Hello")));
    }

    #[test]
    fn parses_loved_single_object_and_pages() {
        let json = r#"{"lovedtracks":{"track":{"name":"One","artist":{"name":"Solo"}},"@attr":{"totalPages":"3"}}}"#;
        let (t, pages) = parse_loved(json).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(pages, 3);
    }
}
