//! Audius: open API, full native playback.
//!
//! Host discovery via `https://api.audius.co`, then `/v1/tracks/search` and
//! `/v1/tracks/{id}/stream` (redirects to the audio file). Tracks gated behind
//! paid access / stream conditions are skipped (no circumventing).

use crate::provider::*;
use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::{Capabilities, PlaybackSource, ProviderId, Track};
use rand::seq::SliceRandom;
use serde::Deserialize;
use tokio::sync::RwLock;

const DISCOVERY: &str = "https://api.audius.co";
const APP_NAME: &str = "Glyph";

pub struct Audius {
    http: reqwest::Client,
    host: RwLock<Option<String>>,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct ATrack {
    id: String,
    title: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    permalink: Option<String>,
    #[serde(default)]
    artwork: Option<Artwork>,
    #[serde(default)]
    user: Option<AUser>,
    #[serde(default)]
    access: Option<Access>,
    #[serde(default)]
    is_streamable: Option<bool>,
    #[serde(default)]
    stream_conditions: Option<serde_json::Value>,
    #[serde(default)]
    isrc: Option<String>,
    #[serde(default)]
    is_delete: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct Artwork {
    #[serde(rename = "480x480")]
    medium: Option<String>,
    #[serde(rename = "150x150")]
    small: Option<String>,
    #[serde(rename = "1000x1000")]
    large: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AUser {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    handle: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Access {
    #[serde(default)]
    stream: Option<bool>,
}

fn convert(t: ATrack) -> Option<Track> {
    if t.is_delete == Some(true) || t.stream_conditions.as_ref().map(|v| !v.is_null()).unwrap_or(false) {
        return None;
    }
    let streamable = t.access.as_ref().and_then(|a| a.stream).or(t.is_streamable).unwrap_or(true);
    if !streamable {
        return None;
    }
    let user = t.user;
    let artist = user
        .as_ref()
        .and_then(|u| u.name.clone().filter(|n| !n.is_empty()).or_else(|| u.handle.clone()))
        .unwrap_or_else(|| "Unknown".into());
    Some(Track {
        id: t.id,
        source: ProviderId::Audius,
        title: t.title,
        artist,
        album: None,
        duration_ms: t.duration.and_then(secs_to_ms),
        cover_url: t.artwork.and_then(|a| a.medium.or(a.large).or(a.small)),
        isrc: t.isrc.filter(|s| !s.is_empty()),
        permalink: t.permalink.map(|p| format!("https://audius.co{p}")),
        external_ids: Default::default(),
    })
}

/// Parses an Audius `{"data":[...]}` search response.
pub fn parse_search(json: &str) -> Result<Vec<Track>> {
    let env: Envelope<Vec<ATrack>> = serde_json::from_str(json)?;
    Ok(env.data.into_iter().filter_map(convert).collect())
}

impl Audius {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http, host: RwLock::new(None) }
    }

    async fn host(&self) -> Result<String> {
        if let Some(h) = self.host.read().await.clone() {
            return Ok(h);
        }
        let env: serde_json::Value = send_json(self.http.get(DISCOVERY)).await?;
        let hosts: Vec<String> = env["data"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.trim_end_matches('/').to_string())).collect())
            .unwrap_or_default();
        let host = hosts
            .choose(&mut rand::thread_rng())
            .cloned()
            .ok_or_else(|| ProviderError::Other("no Audius discovery host".into()))?;
        *self.host.write().await = Some(host.clone());
        Ok(host)
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T> {
        let mut last_err = None;
        for _ in 0..2 {
            let host = self.host().await?;
            let req = self
                .http
                .get(format!("{host}{path}"))
                .query(&[("app_name", APP_NAME)])
                .query(query);
            match send_json::<T>(req).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    if matches!(e.downcast_ref::<ProviderError>(), Some(ProviderError::NotFound)) {
                        return Err(e);
                    }
                    // Host may be down: forget it and rediscover once.
                    *self.host.write().await = None;
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap())
    }
}

#[async_trait]
impl MusicProvider for Audius {
    fn id(&self) -> ProviderId {
        ProviderId::Audius
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::FullPlayback
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let env: Envelope<Vec<ATrack>> =
            self.get("/v1/tracks/search", &[("query", query), ("limit", "20")]).await?;
        Ok(env.data.into_iter().filter_map(convert).collect())
    }

    async fn track(&self, id: &str) -> Result<Track> {
        let env: Envelope<ATrack> = self.get(&format!("/v1/tracks/{id}"), &[]).await?;
        convert(env.data).ok_or_else(|| ProviderError::NotFound.into())
    }

    async fn playback_source(&self, track: &Track) -> Result<PlaybackSource> {
        let id = id_at(ProviderId::Audius, track).ok_or(ProviderError::Unsupported)?;
        let host = self.host().await?;
        Ok(PlaybackSource::Stream {
            provider: ProviderId::Audius,
            url: format!("{host}/v1/tracks/{id}/stream?app_name={APP_NAME}"),
            mime: Some("audio/mpeg".into()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_and_skips_gated() {
        let json = r#"{"data":[
          {"id":"95wro","title":"Stars","duration":71,"permalink":"/Lofibeat/stars-1",
           "artwork":{"150x150":"https://x/150.jpg","480x480":"https://x/480.jpg"},
           "user":{"name":"Lofi Beat","handle":"Lofibeat"},"is_streamable":false,"access":{"stream":true,"download":true},
           "stream_conditions":null,"isrc":null},
          {"id":"gated","title":"Paid","duration":10,"user":{"name":"X"},"stream_conditions":{"usdc_purchase":{"price":100}}},
          {"id":"nostream","title":"No","duration":10,"user":{"name":"X"},"access":{"stream":false}}
        ]}"#;
        let tracks = parse_search(json).unwrap();
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert_eq!(t.id, "95wro");
        assert_eq!(t.artist, "Lofi Beat");
        assert_eq!(t.duration_ms, Some(71_000));
        assert_eq!(t.cover_url.as_deref(), Some("https://x/480.jpg"));
        assert_eq!(t.permalink.as_deref(), Some("https://audius.co/Lofibeat/stars-1"));
    }
}
