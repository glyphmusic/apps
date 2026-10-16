//! SoundCloud: official API (needs a registered app's `client_id`). Only
//! tracks marked streamable/playable are returned; playback is the official
//! embed widget (`PlaybackSource::Embed`, `embed_id` = numeric track id; the
//! player builds `https://w.soundcloud.com/player/?url=https://api.soundcloud.com/tracks/{id}`).

use crate::provider::*;
use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::{Capabilities, PlaybackSource, ProviderId, Track};
use serde::Deserialize;

const BASE: &str = "https://api.soundcloud.com";

pub struct SoundCloud {
    http: reqwest::Client,
    client_id: String,
}

#[derive(Debug, Deserialize)]
struct SCTrack {
    #[serde(deserialize_with = "de_flex_string")]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default, deserialize_with = "de_opt_f64")]
    duration: Option<f64>,
    #[serde(default)]
    artwork_url: Option<String>,
    #[serde(default)]
    permalink_url: Option<String>,
    #[serde(default)]
    streamable: Option<bool>,
    #[serde(default)]
    access: Option<String>,
    #[serde(default)]
    isrc: Option<String>,
    #[serde(default)]
    user: Option<SCUser>,
    #[serde(default)]
    publisher_metadata: Option<Publisher>,
}

#[derive(Debug, Deserialize)]
struct SCUser {
    #[serde(default)]
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Publisher {
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    isrc: Option<String>,
}

fn convert(t: SCTrack) -> Option<Track> {
    if t.streamable == Some(false) || t.access.as_deref().map(|a| a != "playable").unwrap_or(false) {
        return None;
    }
    let artist = t
        .publisher_metadata
        .as_ref()
        .and_then(|p| p.artist.clone())
        .filter(|a| !a.is_empty())
        .or_else(|| t.user.and_then(|u| u.username))
        .unwrap_or_else(|| "Unknown".into());
    let isrc = t
        .isrc
        .or_else(|| t.publisher_metadata.and_then(|p| p.isrc))
        .filter(|s| !s.is_empty());
    Some(Track {
        id: t.id,
        source: ProviderId::SoundCloud,
        title: t.title,
        artist,
        album: None,
        duration_ms: t.duration.filter(|d| *d > 0.0).map(|d| d as u64),
        // "-large.jpg" is 100x100; ask for the 500 px variant.
        cover_url: t.artwork_url.map(|u| u.replace("-large.", "-t500x500.")),
        isrc,
        permalink: t.permalink_url,
        external_ids: Default::default(),
    })
}

pub fn parse_search(json: &str) -> Result<Vec<Track>> {
    let v: Vec<SCTrack> = serde_json::from_str(json)?;
    Ok(v.into_iter().filter_map(convert).collect())
}

impl SoundCloud {
    pub fn new(http: reqwest::Client, client_id: String) -> Self {
        Self { http, client_id }
    }
}

#[async_trait]
impl MusicProvider for SoundCloud {
    fn id(&self) -> ProviderId {
        ProviderId::SoundCloud
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::FullPlayback
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let req = self.http.get(format!("{BASE}/tracks")).query(&[
            ("q", query),
            ("access", "playable"),
            ("limit", "20"),
            ("client_id", self.client_id.as_str()),
        ]);
        let v: Vec<SCTrack> = send_json(req).await?;
        Ok(v.into_iter().filter_map(convert).collect())
    }

    async fn track(&self, id: &str) -> Result<Track> {
        let req = self.http.get(format!("{BASE}/tracks/{id}")).query(&[("client_id", self.client_id.as_str())]);
        let t: SCTrack = send_json(req).await?;
        convert(t).ok_or_else(|| ProviderError::NotFound.into())
    }

    async fn playback_source(&self, track: &Track) -> Result<PlaybackSource> {
        let id = id_at(ProviderId::SoundCloud, track).ok_or(ProviderError::Unsupported)?;
        Ok(PlaybackSource::Embed { provider: ProviderId::SoundCloud, embed_id: id.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_streamable_tracks() {
        let json = r#"[
          {"id":1,"title":"Ok","duration":200000,"artwork_url":"https://i1.sndcdn.com/a-large.jpg",
           "permalink_url":"https://soundcloud.com/a/ok","streamable":true,"access":"playable","user":{"username":"user1"}},
          {"id":2,"title":"Blocked","streamable":false,"user":{"username":"u"}},
          {"id":3,"title":"Preview only","streamable":true,"access":"preview","user":{"username":"u"}}]"#;
        let t = parse_search(json).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "1");
        assert_eq!(t[0].duration_ms, Some(200_000));
        assert!(t[0].cover_url.as_deref().unwrap().contains("t500x500"));
    }
}
