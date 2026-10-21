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
    #[serde(default)]
    pub explicit: Option<bool>,
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
        explicit: t.explicit == Some(true),
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
pub(crate) struct Page<T> {
    #[serde(default = "Vec::new")]
    pub items: Vec<T>,
    #[serde(default)]
    pub next: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlaylistItem {
    #[serde(default)]
    pub item: Option<STrack>,
    #[serde(default)]
    pub track: Option<STrack>,
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

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(&self, url: &str, query: &[(&str, &str)]) -> Result<T> {
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
        let mut url = format!("{API}/playlists/{id}/items");
        let mut query: Vec<(&str, &str)> = vec![("limit", "50")];
        let max_tracks = crate::library::playlist_track_limit();
        let max_pages = max_tracks.div_ceil(50);
        for page_index in 0..=max_pages {
            let page: Page<PlaylistItem> = self.get(&url, &query).await?;
            out.extend(page.items.into_iter().filter_map(|i| i.item.or(i.track)).filter_map(convert));
            match page.next {
                Some(n) => {
                    if out.len() > max_tracks || page_index == max_pages {
                        anyhow::bail!("Playlist exceeds this server's limit of {max_tracks} songs");
                    }
                    let next = reqwest::Url::parse(&n)?;
                    if next.scheme() != "https" || next.host_str() != Some("api.spotify.com")
                        || next.port_or_known_default() != Some(443) || !next.username().is_empty() || next.password().is_some() {
                        anyhow::bail!("invalid provider pagination URL");
                    }
                    url = n;
                    query.clear();
                }
                None => break,
            }
        }
        Ok(out)
    }
}

/// Album songs come without album name and pictures; fill them in.
pub(crate) fn album_tracks_from(v: &serde_json::Value) -> Vec<Track> {
    let name = v["name"].as_str().map(str::to_string);
    let cover = v["images"].as_array().and_then(|imgs| {
        imgs.iter()
            .min_by_key(|i| i["width"].as_u64().unwrap_or(640).abs_diff(640))
            .and_then(|i| i["url"].as_str())
            .map(str::to_string)
    });
    v["tracks"]["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|i| serde_json::from_value::<STrack>(i.clone()).ok())
                .filter_map(convert)
                .map(|mut t| {
                    t.album = t.album.or_else(|| name.clone());
                    t.cover_url = t.cover_url.or_else(|| cover.clone());
                    t
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Spotify {
    /// Name and songs of an album (first 50 come inline, the rest paged).
    pub async fn album(&self, id: &str) -> Result<(String, String, Vec<Track>)> {
        let v: serde_json::Value = self.get(&format!("{API}/albums/{id}"), &[]).await?;
        let name = v["name"].as_str().unwrap_or("Spotify album").to_string();
        let artist = v["artists"][0]["name"].as_str().unwrap_or("").to_string();
        let mut tracks = album_tracks_from(&v);
        let mut next = v["tracks"]["next"].as_str().map(str::to_string);
        let mut guard = 0;
        while let Some(url) = next.take() {
            guard += 1;
            if guard > 20 {
                break;
            }
            let page: serde_json::Value = self.get(&url, &[]).await?;
            let wrapped = serde_json::json!({ "name": v["name"], "images": v["images"], "tracks": page });
            tracks.extend(album_tracks_from(&wrapped));
            next = page["next"].as_str().map(str::to_string);
        }
        Ok((name, artist, tracks))
    }

    /// Up to 50 songs by id in one request.
    pub async fn tracks_by_ids(&self, ids: &[&str]) -> Result<Vec<Track>> {
        let joined = ids.join(",");
        let v: serde_json::Value = self.get(&format!("{API}/tracks"), &[("ids", joined.as_str())]).await?;
        Ok(v["tracks"]
            .as_array()
            .map(|l| l.iter().filter_map(|t| serde_json::from_value::<STrack>(t.clone()).ok()).filter_map(convert).collect())
            .unwrap_or_default())
    }

    /// An artist's name and top songs.
    pub async fn artist_top(&self, id: &str) -> Result<(String, Vec<Track>)> {
        let a: serde_json::Value = self.get(&format!("{API}/artists/{id}"), &[]).await?;
        let name = a["name"].as_str().unwrap_or("Spotify artist").to_string();
        let v: serde_json::Value = self.get(&format!("{API}/artists/{id}/top-tracks"), &[("market", "US")]).await?;
        let tracks = v["tracks"]
            .as_array()
            .map(|l| l.iter().filter_map(|t| serde_json::from_value::<STrack>(t.clone()).ok()).filter_map(convert).collect())
            .unwrap_or_default();
        Ok((name, tracks))
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
            .get(&format!("{API}/search"), &[("q", query), ("type", "track"), ("limit", "10")])
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
    fn playlist_rows_accept_current_and_legacy_spotify_fields() {
        for json in [
            r#"{"item":{"id":"a","name":"Song","artists":[{"name":"Artist"}]}}"#,
            r#"{"track":{"id":"a","name":"Song","artists":[{"name":"Artist"}]}}"#,
            r#"{"item":{"id":"a","name":"Song","artists":[{"name":"Artist"}]},"track":null}"#,
        ] {
            let row: PlaylistItem = serde_json::from_str(json).unwrap();
            let track = convert(row.item.or(row.track).unwrap()).unwrap();
            assert_eq!(track.id, "a");
        }
    }

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
