//! Spotify Web API, client-credentials flow. METADATA ONLY: search, covers,
//! ISRC and public-playlist import. Never plays anything.

use crate::provider::*;
use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::{Capabilities, PlaybackSource, ProviderId, Track};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const API: &str = "https://api.spotify.com/v1";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

pub struct Spotify {
    http: reqwest::Client,
    client_id: String,
    client_secret: String,
    token: Mutex<Option<(String, Instant)>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct STrack {
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub artists: Vec<SArtist>,
    #[serde(default)]
    pub album: Option<SAlbum>,
    #[serde(default)]
    pub external_ids: Option<SExternalIds>,
    #[serde(default)]
    pub external_urls: Option<SUrls>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub is_local: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SArtist {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SAlbum {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub images: Vec<SImage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SImage {
    pub url: String,
    #[serde(default)]
    pub width: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SExternalIds {
    #[serde(default)]
    pub isrc: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SUrls {
    #[serde(default)]
    pub spotify: Option<String>,
}

pub(crate) fn convert(t: STrack) -> Option<Track> {
    if t.is_local == Some(true) || t.kind.as_deref().map(|k| k != "track").unwrap_or(false) {
        return None;
    }
    let id = t.id?;
    // Prefer the ~300-640 px image.
    let cover = t.album.as_ref().and_then(|a| {
        a.images
            .iter()
            .min_by_key(|i| i.width.unwrap_or(640).abs_diff(640))
            .map(|i| i.url.clone())
    });
    Some(Track {
        permalink: t
            .external_urls
            .and_then(|u| u.spotify)
            .or_else(|| Some(format!("https://open.spotify.com/track/{id}"))),
        id,
        source: ProviderId::Spotify,
        title: t.name,
        artist: t.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", "),
        album: t.album.and_then(|a| a.name),
        duration_ms: t.duration_ms,
        cover_url: cover,
        isrc: t.external_ids.and_then(|e| e.isrc).filter(|s| !s.is_empty()),
        external_ids: Default::default(),
    })
}

#[derive(Debug, Deserialize)]
struct SearchResp {
    tracks: Page<STrack>,
}

#[derive(Debug, Deserialize)]
struct Page<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    #[serde(default)]
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PlaylistItem {
    track: Option<STrack>,
}

pub fn parse_search(json: &str) -> Result<Vec<Track>> {
    let r: SearchResp = serde_json::from_str(json)?;
    Ok(r.tracks.items.into_iter().filter_map(convert).collect())
}

impl Spotify {
    pub fn new(http: reqwest::Client, client_id: String, client_secret: String) -> Self {
        Self { http, client_id, client_secret, token: Mutex::new(None) }
    }

    async fn token(&self) -> Result<String> {
        let mut guard = self.token.lock().await;
        if let Some((t, exp)) = guard.as_ref() {
            if *exp > Instant::now() {
                return Ok(t.clone());
            }
        }
        #[derive(Deserialize)]
        struct Tok {
            access_token: String,
            expires_in: u64,
        }
        let req = self
            .http
            .post(TOKEN_URL)
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&[("grant_type", "client_credentials")]);
        let t: Tok = send_json(req).await?;
        let exp = Instant::now() + Duration::from_secs(t.expires_in.saturating_sub(60));
        *guard = Some((t.access_token.clone(), exp));
        Ok(t.access_token)
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str, query: &[(&str, &str)]) -> Result<T> {
        let token = self.token().await?;
        let req = self.http.get(url).bearer_auth(token).query(query);
        send_json(req).await
    }

    /// Name of a public playlist.
    pub async fn playlist_name(&self, id: &str) -> Result<String> {
        let v: serde_json::Value = self.get(&format!("{API}/playlists/{id}"), &[("fields", "name")]).await?;
        Ok(v["name"].as_str().unwrap_or("Spotify playlist").to_string())
    }

    /// All tracks of a public playlist (paged).
    pub async fn playlist_tracks(&self, id: &str) -> Result<Vec<Track>> {
        let mut out = Vec::new();
        let mut url = format!("{API}/playlists/{id}/tracks");
        let mut query: Vec<(&str, &str)> = vec![("limit", "100")];
        for _ in 0..100 {
            let page: Page<PlaylistItem> = self.get(&url, &query).await?;
            out.extend(page.items.into_iter().filter_map(|i| i.track).filter_map(convert));
            match page.next {
                Some(n) => {
                    url = n;
                    query.clear();
                }
                None => break,
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl MusicProvider for Spotify {
    fn id(&self) -> ProviderId {
        ProviderId::Spotify
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::MetadataOnly
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let r: SearchResp = self
            .get(&format!("{API}/search"), &[("q", query), ("type", "track"), ("limit", "20")])
            .await?;
        Ok(r.tracks.items.into_iter().filter_map(convert).collect())
    }

    async fn track(&self, id: &str) -> Result<Track> {
        let t: STrack = self.get(&format!("{API}/tracks/{id}"), &[]).await?;
        convert(t).ok_or_else(|| ProviderError::NotFound.into())
    }

    async fn playback_source(&self, _track: &Track) -> Result<PlaybackSource> {
        Err(ProviderError::Unsupported.into())
    }

    async fn find_by_isrc(&self, isrc: &str) -> Result<Option<Track>> {
        let q = format!("isrc:{isrc}");
        let r: SearchResp = self
            .get(&format!("{API}/search"), &[("q", q.as_str()), ("type", "track"), ("limit", "1")])
            .await?;
        Ok(r.tracks.items.into_iter().filter_map(convert).next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tracks() {
        let json = r#"{"tracks":{"items":[
          {"id":"abc","name":"Song","duration_ms":201000,"type":"track",
           "artists":[{"name":"A"},{"name":"B"}],
           "album":{"name":"LP","images":[{"url":"https://i/64.jpg","width":64},{"url":"https://i/640.jpg","width":640}]},
           "external_ids":{"isrc":"USX123"},"external_urls":{"spotify":"https://open.spotify.com/track/abc"}},
          {"id":null,"name":"local","is_local":true,"artists":[]}]}}"#;
        let t = parse_search(json).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].artist, "A, B");
        assert_eq!(t[0].isrc.as_deref(), Some("USX123"));
        assert_eq!(t[0].cover_url.as_deref(), Some("https://i/640.jpg"));
    }
}
