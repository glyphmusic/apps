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
pub(crate) struct SCTrack {
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

pub(crate) fn convert(t: SCTrack) -> Option<Track> {
    if !playable(&t) {
        return None;
    }
    Some(convert_meta(t))
}

fn playable(t: &SCTrack) -> bool {
    !(t.streamable == Some(false) || t.access.as_deref().map(|a| a != "playable").unwrap_or(false))
}

/// Any track for import: playable ones keep SoundCloud as their source (the
/// embed plays them), the rest are metadata only (`MusicBrainz`, like files)
/// and get matched elsewhere.
pub(crate) fn convert_import(t: SCTrack) -> Track {
    let ok = playable(&t);
    let mut track = convert_meta(t);
    if !ok {
        track.id = format!("sc:{}", track.id);
        track.source = ProviderId::MusicBrainz;
    }
    track
}

fn convert_meta(t: SCTrack) -> Track {
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
    Track {
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
        explicit: false,
    }
}

pub fn parse_search(json: &str) -> Result<Vec<Track>> {
    let v: Vec<SCTrack> = serde_json::from_str(json)?;
    Ok(v.into_iter().filter_map(convert).collect())
}

impl SoundCloud {
    pub fn new(http: reqwest::Client, client_id: String) -> Self {
        Self { http, client_id }
    }

    /// What a `soundcloud.com/...` link points to (`GET /resolve`): a track,
    /// a playlist (set) or a user.
    pub async fn resolve_url(&self, url: &str) -> Result<serde_json::Value> {
        let req = self.http.get(format!("{BASE}/resolve")).query(&[("url", url), ("client_id", self.client_id.as_str())]);
        send_json(req).await
    }

    /// GET of an API path (or a `next_href`) with the app's client id.
    pub(crate) async fn get(&self, path_or_url: &str, query: &[(&str, &str)]) -> Result<serde_json::Value> {
        let url = if path_or_url.starts_with("http") { path_or_url.to_string() } else { format!("{BASE}{path_or_url}") };
        let mut req = self.http.get(url).query(query);
        if !path_or_url.contains("client_id=") {
            req = req.query(&[("client_id", self.client_id.as_str())]);
        }
        send_json(req).await
    }

    /// Every item of a paged list (`linked_partitioning`, 200 per page, at
    /// most `max` items). Plain arrays (older endpoints) work too.
    pub(crate) async fn paged(&self, path: &str, max: usize) -> Result<Vec<serde_json::Value>> {
        let mut out = Vec::new();
        let mut next: Option<String> = None;
        for _ in 0..50 {
            let v = match &next {
                Some(n) => self.get(n, &[]).await?,
                None => self.get(path, &[("linked_partitioning", "true"), ("limit", "200")]).await?,
            };
            match v {
                serde_json::Value::Array(items) => {
                    out.extend(items);
                    break;
                }
                v => {
                    if let Some(items) = v["collection"].as_array() {
                        out.extend(items.iter().cloned());
                    }
                    next = v["next_href"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
                }
            }
            if next.is_none() || out.len() >= max {
                break;
            }
        }
        out.truncate(max);
        Ok(out)
    }

    /// Songs of a list endpoint (likes return `{track: ...}` wrappers on some
    /// endpoints, plain tracks on others).
    pub(crate) async fn tracks_of(&self, path: &str) -> Result<Vec<Track>> {
        let items = self.paged(path, 10_000).await?;
        Ok(items
            .into_iter()
            .filter_map(|v| {
                let v = if v.get("track").is_some_and(|t| t.is_object()) { v["track"].clone() } else { v };
                serde_json::from_value::<SCTrack>(v).ok()
            })
            .filter(|t| !t.title.is_empty())
            .map(convert_import)
            .collect())
    }

    /// The song list a soundcloud.com link stands for: a set, a user's likes
    /// (`/<user>/likes`), a user's own tracks (`/<user>` or `/<user>/tracks`)
    /// or one track. Returns (name, songs).
    pub async fn list_from_link(&self, url: &str) -> Result<(String, Vec<Track>)> {
        let parsed = reqwest::Url::parse(url)?;
        let segs: Vec<String> = parsed.path_segments().map(|s| s.filter(|x| !x.is_empty()).map(str::to_string).collect()).unwrap_or_default();
        let likes = segs.get(1).is_some_and(|s| s == "likes");
        let user_tracks = segs.len() == 1 || segs.get(1).is_some_and(|s| s == "tracks");
        // For likes / tracks pages resolve the profile itself.
        let target = if likes || user_tracks { format!("https://soundcloud.com/{}", segs.first().cloned().unwrap_or_default()) } else { url.to_string() };
        let v = self.resolve_url(&target).await?;
        let id = v["id"].as_i64().map(|n| n.to_string()).or_else(|| v["id"].as_str().map(str::to_string)).unwrap_or_default();
        let kind = v["kind"].as_str().unwrap_or("").to_string();
        match kind.as_str() {
            "track" => {
                let t: SCTrack = serde_json::from_value(v)?;
                let t = convert_import(t);
                Ok((t.title.clone(), vec![t]))
            }
            "playlist" => {
                let name = v["title"].as_str().unwrap_or("SoundCloud set").to_string();
                let tracks = self.tracks_of(&format!("/playlists/{id}/tracks")).await?;
                Ok((name, tracks))
            }
            "user" => {
                let user = v["username"].as_str().unwrap_or("SoundCloud").to_string();
                if likes {
                    Ok((format!("{user} · likes"), self.tracks_of(&format!("/users/{id}/likes/tracks")).await?))
                } else {
                    Ok((format!("{user} · tracks"), self.tracks_of(&format!("/users/{id}/tracks")).await?))
                }
            }
            other => Err(ProviderError::Other(format!("unsupported SoundCloud link ({other})")).into()),
        }
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
