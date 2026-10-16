//! Deezer public API (no key): metadata + 30 s previews only.

use crate::provider::*;
use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::{Capabilities, PlaybackSource, ProviderId, Track};
use serde::Deserialize;

const API: &str = "https://api.deezer.com";

pub struct Deezer {
    http: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct DTrack {
    #[serde(deserialize_with = "de_flex_string")]
    id: String,
    #[serde(default)]
    readable: Option<bool>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    isrc: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(default, deserialize_with = "de_opt_f64")]
    duration: Option<f64>,
    #[serde(default)]
    preview: Option<String>,
    #[serde(default)]
    artist: Option<DArtist>,
    #[serde(default)]
    album: Option<DAlbum>,
}

#[derive(Debug, Deserialize)]
struct DArtist {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct DAlbum {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    cover_xl: Option<String>,
    #[serde(default)]
    cover_big: Option<String>,
    #[serde(default)]
    cover_medium: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DList {
    #[serde(default)]
    data: Vec<DTrack>,
    #[serde(default)]
    next: Option<String>,
}

fn convert(t: DTrack) -> Option<Track> {
    if t.readable == Some(false) || t.title.is_empty() {
        return None;
    }
    let album = t.album;
    Some(Track {
        permalink: t.link.or_else(|| Some(format!("https://www.deezer.com/track/{}", t.id))),
        id: t.id,
        source: ProviderId::Deezer,
        title: t.title,
        artist: t.artist.map(|a| a.name).unwrap_or_else(|| "Unknown".into()),
        cover_url: album.as_ref().and_then(|a| a.cover_xl.clone().or(a.cover_big.clone()).or(a.cover_medium.clone())),
        album: album.and_then(|a| a.title),
        duration_ms: t.duration.and_then(secs_to_ms),
        isrc: t.isrc.filter(|s| !s.is_empty()),
        external_ids: Default::default(),
    })
}

pub fn parse_search(json: &str) -> Result<Vec<Track>> {
    let l: DList = serde_json::from_str(json)?;
    Ok(l.data.into_iter().filter_map(convert).collect())
}

/// Deezer reports errors as HTTP 200 + `{"error":{...}}`. Code 4 = quota.
fn check_error(v: &serde_json::Value) -> Result<()> {
    if let Some(e) = v.get("error") {
        let code = e["code"].as_i64().unwrap_or(0);
        return Err(match code {
            4 => ProviderError::Quota,
            800 => ProviderError::NotFound,
            _ => ProviderError::Other(e["message"].as_str().unwrap_or("deezer error").to_string()),
        }
        .into());
    }
    Ok(())
}

impl Deezer {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    async fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<serde_json::Value> {
        let v: serde_json::Value = send_json(self.http.get(url).query(query)).await?;
        check_error(&v)?;
        Ok(v)
    }

    async fn fetch_track(&self, id: &str) -> Result<DTrack> {
        let v = self.get(&format!("{API}/track/{id}"), &[]).await?;
        Ok(serde_json::from_value(v)?)
    }

    async fn paged(&self, first: String) -> Result<Vec<Track>> {
        let mut out = Vec::new();
        let mut url = first;
        let mut query: Vec<(&str, &str)> = vec![("limit", "100")];
        for _ in 0..60 {
            let v = self.get(&url, &query).await?;
            let l: DList = serde_json::from_value(v)?;
            out.extend(l.data.into_iter().filter_map(convert));
            match l.next {
                Some(n) => {
                    url = n;
                    query.clear();
                }
                None => break,
            }
        }
        Ok(out)
    }

    pub async fn playlist(&self, id: &str) -> Result<(String, Vec<Track>)> {
        let v = self.get(&format!("{API}/playlist/{id}"), &[]).await?;
        let name = v["title"].as_str().unwrap_or("Deezer playlist").to_string();
        let tracks = self.paged(format!("{API}/playlist/{id}/tracks")).await?;
        Ok((name, tracks))
    }

    /// Public favourites of a user id.
    pub async fn user_favorites(&self, user_id: &str) -> Result<(String, Vec<Track>)> {
        let name = match self.get(&format!("{API}/user/{user_id}"), &[]).await {
            Ok(v) => format!("{} (Deezer)", v["name"].as_str().unwrap_or("favorites")),
            Err(_) => "Deezer favorites".to_string(),
        };
        let tracks = self.paged(format!("{API}/user/{user_id}/tracks")).await?;
        Ok((name, tracks))
    }
}

#[async_trait]
impl MusicProvider for Deezer {
    fn id(&self) -> ProviderId {
        ProviderId::Deezer
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::Preview
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let v = self.get(&format!("{API}/search"), &[("q", query), ("limit", "20")]).await?;
        let l: DList = serde_json::from_value(v)?;
        Ok(l.data.into_iter().filter_map(convert).collect())
    }

    async fn track(&self, id: &str) -> Result<Track> {
        convert(self.fetch_track(id).await?).ok_or_else(|| ProviderError::NotFound.into())
    }

    /// 30 s preview. Preview URLs are signed and expire, so always re-fetch.
    async fn playback_source(&self, track: &Track) -> Result<PlaybackSource> {
        let id = id_at(ProviderId::Deezer, track).ok_or(ProviderError::Unsupported)?;
        let t = self.fetch_track(id).await?;
        let url = t.preview.filter(|u| !u.is_empty()).ok_or(ProviderError::NotFound)?;
        Ok(PlaybackSource::Preview { provider: ProviderId::Deezer, url })
    }

    async fn find_by_isrc(&self, isrc: &str) -> Result<Option<Track>> {
        match self.get(&format!("{API}/track/isrc:{isrc}"), &[]).await {
            Ok(v) => Ok(serde_json::from_value::<DTrack>(v).ok().and_then(convert)),
            Err(e) if matches!(e.downcast_ref::<ProviderError>(), Some(ProviderError::NotFound)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search() {
        let json = r#"{"data":[{"id":67238732,"readable":true,"title":"Instant Crush","isrc":"USQX91300105",
          "link":"https://www.deezer.com/track/67238732","duration":337,"preview":"https://cdn/p.mp3",
          "artist":{"name":"Daft Punk"},"album":{"title":"RAM","cover_xl":"https://c/xl.jpg"}}],"total":1}"#;
        let t = parse_search(json).unwrap();
        assert_eq!(t[0].id, "67238732");
        assert_eq!(t[0].duration_ms, Some(337_000));
        assert_eq!(t[0].isrc.as_deref(), Some("USQX91300105"));
        assert_eq!(t[0].album.as_deref(), Some("RAM"));
    }

    #[test]
    fn error_mapping() {
        let q: serde_json::Value = serde_json::json!({"error":{"type":"Exception","message":"Quota limit exceeded","code":4}});
        let e = check_error(&q).unwrap_err();
        assert!(matches!(e.downcast_ref::<ProviderError>(), Some(ProviderError::Quota)));
        assert!(check_error(&serde_json::json!({"data":[]})).is_ok());
    }
}
