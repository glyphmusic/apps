//! Jamendo: official API (needs `client_id`), full native playback of
//! Creative-Commons music.

use crate::provider::*;
use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::{Capabilities, PlaybackSource, ProviderId, Track};
use serde::Deserialize;

const BASE: &str = "https://api.jamendo.com/v3.0";

pub struct Jamendo {
    http: reqwest::Client,
    client_id: String,
}

#[derive(Debug, Deserialize)]
struct Resp {
    #[serde(default)]
    results: Vec<JTrack>,
}

#[derive(Debug, Deserialize)]
struct JTrack {
    #[serde(deserialize_with = "de_flex_string")]
    id: String,
    name: String,
    #[serde(default, deserialize_with = "de_opt_f64")]
    duration: Option<f64>,
    #[serde(default)]
    artist_name: Option<String>,
    #[serde(default)]
    album_name: Option<String>,
    #[serde(default)]
    album_image: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    audio: Option<String>,
    #[serde(default)]
    shareurl: Option<String>,
    #[serde(default)]
    audiodownload: Option<String>,
    #[serde(default)]
    audiodownload_allowed: Option<bool>,
}

fn convert(t: &JTrack) -> Track {
    Track {
        id: t.id.clone(),
        source: ProviderId::Jamendo,
        title: t.name.clone(),
        artist: t.artist_name.clone().unwrap_or_else(|| "Unknown".into()),
        album: t.album_name.clone().filter(|s| !s.is_empty()),
        duration_ms: t.duration.and_then(secs_to_ms),
        cover_url: t.album_image.clone().or_else(|| t.image.clone()).filter(|s| !s.is_empty()),
        isrc: None,
        permalink: t.shareurl.clone(),
        external_ids: Default::default(),
        explicit: false,
    }
}

fn download_of(t: &JTrack) -> Option<String> {
    (t.audiodownload_allowed == Some(true)).then(|| t.audiodownload.clone()).flatten().filter(|u| !u.is_empty())
}

pub fn parse_search(json: &str) -> Result<Vec<Track>> {
    let r: Resp = serde_json::from_str(json)?;
    Ok(r.results.iter().map(convert).collect())
}

impl Jamendo {
    pub fn new(http: reqwest::Client, client_id: String) -> Self {
        Self { http, client_id }
    }

    async fn fetch(&self, extra: &[(&str, &str)]) -> Result<Vec<JTrack>> {
        let req = self
            .http
            .get(format!("{BASE}/tracks/"))
            .query(&[
                ("client_id", self.client_id.as_str()),
                ("format", "json"),
                ("audioformat", "mp32"),
                ("audiodlformat", "mp32"),
                ("imagesize", "300"),
            ])
            .query(extra);
        let r: Resp = send_json(req).await?;
        Ok(r.results)
    }
}

#[async_trait]
impl MusicProvider for Jamendo {
    fn id(&self) -> ProviderId {
        ProviderId::Jamendo
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::FullPlayback
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let r = self.fetch(&[("search", query), ("limit", "20"), ("order", "relevance")]).await?;
        Ok(r.iter().map(convert).collect())
    }

    async fn track(&self, id: &str) -> Result<Track> {
        let r = self.fetch(&[("id", id), ("limit", "1")]).await?;
        r.first().map(convert).ok_or_else(|| ProviderError::NotFound.into())
    }

    async fn playback_source(&self, track: &Track) -> Result<PlaybackSource> {
        let id = id_at(ProviderId::Jamendo, track).ok_or(ProviderError::Unsupported)?;
        let r = self.fetch(&[("id", id), ("limit", "1")]).await?;
        let url = r
            .first()
            .and_then(|t| t.audio.clone())
            .filter(|u| !u.is_empty())
            .ok_or(ProviderError::NotFound)?;
        Ok(PlaybackSource::Stream { provider: ProviderId::Jamendo, url, mime: Some("audio/mpeg".into()) })
    }

    /// Only when the artist allowed downloads (`audiodownload_allowed`).
    /// Always the MP3 link; `Backend::download` switches it to the wanted
    /// quality (`downloads::jamendo_format_url`), also for server answers.
    async fn download_url(&self, track: &Track) -> Result<Option<String>> {
        let Some(id) = id_at(ProviderId::Jamendo, track) else { return Ok(None) };
        let r = self.fetch(&[("id", id), ("limit", "1")]).await?;
        Ok(r.first().and_then(download_of))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_results() {
        let json = r#"{"headers":{"status":"success"},"results":[
          {"id":"1234","name":"Song","duration":185,"artist_name":"Band","album_name":"LP",
           "album_image":"https://img/1.jpg","audio":"https://prod/1.mp3","shareurl":"https://www.jamendo.com/track/1234"}]}"#;
        let t = parse_search(json).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "1234");
        assert_eq!(t[0].duration_ms, Some(185_000));
        assert_eq!(t[0].album.as_deref(), Some("LP"));
    }

    #[test]
    fn download_needs_permission() {
        let r: Resp = serde_json::from_str(r#"{"results":[
          {"id":"1","name":"A","audiodownload":"https://dl/1","audiodownload_allowed":true},
          {"id":"2","name":"B","audiodownload":"https://dl/2","audiodownload_allowed":false},
          {"id":"3","name":"C","audiodownload":"https://dl/3"}]}"#).unwrap();
        assert_eq!(download_of(&r.results[0]).as_deref(), Some("https://dl/1"));
        assert_eq!(download_of(&r.results[1]), None);
        assert_eq!(download_of(&r.results[2]), None);
    }
}
