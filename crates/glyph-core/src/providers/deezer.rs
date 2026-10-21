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
pub(crate) struct DTrack {
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
    #[serde(default)]
    explicit_lyrics: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DArtist {
    #[serde(default)]
    name: String,
    #[serde(default, deserialize_with = "de_opt_flex_string")]
    id: Option<String>,
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
pub(crate) struct DList {
    #[serde(default)]
    pub data: Vec<DTrack>,
    #[serde(default)]
    pub next: Option<String>,
}

pub(crate) fn convert(t: DTrack) -> Option<Track> {
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
        explicit: t.explicit_lyrics == Some(true),
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

/// One release of an artist (album, EP, single, compilation).
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
pub struct Release {
    /// Deezer album id.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub cover_url: Option<String>,
    pub year: Option<u16>,
    /// Release day `YYYY-MM-DD`, when the catalog says.
    #[serde(default)]
    pub released: Option<String>,
    pub kind: crate::catalog::AlbumKind,
}

impl Release {
    /// A stand-in song for the release's cover (cover cache, menus).
    pub fn cover_track(&self) -> Track {
        Track {
            id: format!("album:{}", self.id),
            source: ProviderId::Deezer,
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: Some(self.title.clone()),
            duration_ms: None,
            cover_url: self.cover_url.clone(),
            isrc: None,
            permalink: Some(format!("https://www.deezer.com/album/{}", self.id)),
            external_ids: Default::default(),
            explicit: false,
        }
    }
}

#[derive(Debug, Deserialize)]
struct DArtistHit {
    #[serde(deserialize_with = "de_flex_string")]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    nb_fan: u64,
}

#[derive(Debug, Deserialize)]
struct DRelease {
    #[serde(deserialize_with = "de_flex_string")]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cover_xl: Option<String>,
    #[serde(default)]
    cover_big: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    record_type: Option<String>,
}

impl Deezer {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// The Deezer artist for a name: exact (normalised) name, most fans.
    pub async fn artist_id(&self, name: &str) -> Result<Option<String>> {
        let v = self.get(&format!("{API}/search/artist"), &[("q", name), ("limit", "10")]).await?;
        let hits: Vec<DArtistHit> = serde_json::from_value(v["data"].clone()).unwrap_or_default();
        let want = crate::dedupe::normalize_artist(name);
        Ok(hits.into_iter().filter(|a| crate::dedupe::normalize_artist(&a.name) == want).max_by_key(|a| a.nb_fan).map(|a| a.id))
    }

    /// Official profile photo, selected by exact artist name rather than a song cover.
    pub async fn artist_picture(&self, name: &str) -> Result<Option<String>> {
        let v = self.get(&format!("{API}/search/artist"), &[("q", name), ("limit", "10")]).await?;
        let want = crate::dedupe::normalize_artist(name);
        Ok(v["data"].as_array().into_iter().flatten()
            .filter(|a| a["name"].as_str().is_some_and(|n| crate::dedupe::normalize_artist(n) == want))
            .filter_map(|a| {
                let url = ["picture_xl", "picture_big", "picture_medium", "picture"].into_iter()
                    .find_map(|key| a[key].as_str().filter(|u| u.starts_with("https://")))?;
                Some((a["nb_fan"].as_u64().unwrap_or(0), url.to_string()))
            }).max_by_key(|(fans, _)| *fans).map(|(_, url)| url))
    }

    /// Every release of an artist (the whole discography, not only what a
    /// song search happens to find), newest first.
    pub async fn artist_releases(&self, name: &str) -> Result<Vec<Release>> {
        let Some(id) = self.artist_id(name).await? else { return Ok(Vec::new()) };
        let mut out: Vec<Release> = Vec::new();
        let mut index = 0usize;
        // Paged by 100; four pages cover even very large catalogs.
        for _ in 0..4 {
            let idx = index.to_string();
            let v = self.get(&format!("{API}/artist/{id}/albums"), &[("limit", "100"), ("index", idx.as_str())]).await?;
            let page: Vec<DRelease> = serde_json::from_value(v["data"].clone()).unwrap_or_default();
            let n = page.len();
            for r in page {
                if r.title.trim().is_empty() || out.iter().any(|o| o.id == r.id) {
                    continue;
                }
                out.push(Release {
                    id: r.id,
                    title: r.title,
                    artist: name.to_string(),
                    cover_url: r.cover_xl.or(r.cover_big),
                    year: r.release_date.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()).filter(|y| *y > 1900),
                    released: r.release_date.clone().filter(|d| d.len() == 10 && !d.starts_with("0000")),
                    kind: r.record_type.as_deref().and_then(crate::catalog::AlbumKind::parse).unwrap_or(crate::catalog::AlbumKind::Album),
                });
            }
            if n < 100 || v["next"].is_null() {
                break;
            }
            index += 100;
        }
        // Same title twice (clean + explicit, deluxe re-issues under the same
        // name): keep the newest.
        out.sort_by(|a, b| b.released.cmp(&a.released).then(b.year.cmp(&a.year)));
        let mut seen = std::collections::HashSet::new();
        out.retain(|r| seen.insert(r.title.to_lowercase()));
        Ok(out)
    }

    /// The songs of a release, in order.
    pub async fn album_tracks(&self, release: &Release) -> Result<Vec<Track>> {
        let v = self.get(&format!("{API}/album/{}/tracks", release.id), &[("limit", "200")]).await?;
        let list: Vec<DTrack> = serde_json::from_value(v["data"].clone()).unwrap_or_default();
        Ok(list
            .into_iter()
            .filter_map(convert)
            .map(|mut t| {
                t.album = Some(release.title.clone());
                if t.cover_url.is_none() {
                    t.cover_url = release.cover_url.clone();
                }
                t
            })
            .collect())
    }

    pub(crate) async fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<serde_json::Value> {
        let v: serde_json::Value = send_json(self.http.get(url).query(query)).await?;
        check_error(&v)?;
        Ok(v)
    }

    /// Songs like `track` without any key: top songs of the artists Deezer
    /// lists as related, interleaved (one per artist per round).
    pub async fn related_tracks(&self, track: &Track) -> Result<Vec<Track>> {
        // The artist's Deezer id: from the song itself or a search.
        let artist_id = match id_at(ProviderId::Deezer, track) {
            Some(id) => self.fetch_track(id).await.ok().and_then(|t| t.artist.and_then(|a| a.id)),
            None => None,
        };
        let artist_id = match artist_id {
            Some(a) => a,
            None => {
                let q = format!("{} {}", track.artist, track.title);
                let v = self.get(&format!("{API}/search"), &[("q", q.trim()), ("limit", "5")]).await?;
                let l: DList = serde_json::from_value(v)?;
                let want = crate::dedupe::normalize_artist(&track.artist);
                l.data
                    .into_iter()
                    .filter_map(|t| t.artist)
                    .find(|a| crate::dedupe::normalize_artist(&a.name) == want)
                    .and_then(|a| a.id)
                    .ok_or(ProviderError::NotFound)?
            }
        };
        #[derive(Deserialize)]
        struct Related {
            #[serde(default)]
            data: Vec<DArtist>,
        }
        let v = self.get(&format!("{API}/artist/{artist_id}/related"), &[("limit", "8")]).await?;
        let rel: Related = serde_json::from_value(v)?;
        let mut per_artist: Vec<Vec<Track>> = Vec::new();
        for a in rel.data.into_iter().filter_map(|a| a.id).take(6) {
            if let Ok(v) = self.get(&format!("{API}/artist/{a}/top"), &[("limit", "3")]).await {
                if let Ok(l) = serde_json::from_value::<DList>(v) {
                    per_artist.push(l.data.into_iter().filter_map(convert).collect());
                }
            }
        }
        let mut out = Vec::new();
        for round in 0..3 {
            for list in &per_artist {
                if let Some(t) = list.get(round) {
                    out.push(t.clone());
                }
            }
        }
        Ok(out)
    }

    /// Artists Deezer lists as related to `artist` (needs no key).
    pub async fn related_artists(&self, artist: &str) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Related {
            #[serde(default)]
            data: Vec<DArtist>,
        }
        let id = self.artist_id(artist.trim()).await?.ok_or(ProviderError::NotFound)?;
        let v = self.get(&format!("{API}/artist/{id}/related"), &[("limit", "20")]).await?;
        let rel: Related = serde_json::from_value(v)?;
        Ok(rel.data.into_iter().map(|a| a.name).filter(|n| !n.trim().is_empty()).collect())
    }

    /// Popular right now (Deezer's public chart).
    pub async fn chart(&self, limit: usize) -> Result<Vec<Track>> {
        let n = limit.clamp(1, 100).to_string();
        let v = self.get(&format!("{API}/chart/0/tracks"), &[("limit", n.as_str())]).await?;
        let l: DList = serde_json::from_value(v)?;
        Ok(l.data.into_iter().filter_map(convert).collect())
    }

    async fn fetch_track(&self, id: &str) -> Result<DTrack> {
        let v = self.get(&format!("{API}/track/{id}"), &[]).await?;
        Ok(serde_json::from_value(v)?)
    }

    pub(crate) async fn paged(&self, first: String) -> Result<Vec<Track>> {
        let mut out = Vec::new();
        let mut url = first;
        let mut query: Vec<(&str, &str)> = vec![("limit", "100")];
        let max_tracks = crate::library::playlist_track_limit();
        let max_pages = max_tracks.div_ceil(100);
        for page_index in 0..=max_pages {
            let v = self.get(&url, &query).await?;
            let l: DList = serde_json::from_value(v)?;
            out.extend(l.data.into_iter().filter_map(convert));
            match l.next {
                Some(n) => {
                    if out.len() > max_tracks || page_index == max_pages {
                        anyhow::bail!("Playlist exceeds this server's limit of {max_tracks} songs");
                    }
                    let next = reqwest::Url::parse(&n)?;
                    if next.scheme() != "https" || next.host_str() != Some("api.deezer.com")
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

    pub async fn playlist(&self, id: &str) -> Result<(String, Vec<Track>)> {
        let v = self.get(&format!("{API}/playlist/{id}"), &[]).await?;
        let name = v["title"].as_str().unwrap_or("Deezer playlist").to_string();
        let tracks = self.paged(format!("{API}/playlist/{id}/tracks")).await?;
        Ok((name, tracks))
    }

    /// Title, artist and songs of an album.
    pub async fn album(&self, id: &str) -> Result<(String, String, Vec<Track>)> {
        let v = self.get(&format!("{API}/album/{id}"), &[]).await?;
        let release = Release {
            id: id.to_string(),
            title: v["title"].as_str().unwrap_or("Deezer album").to_string(),
            artist: v["artist"]["name"].as_str().unwrap_or("").to_string(),
            cover_url: v["cover_xl"].as_str().or(v["cover_big"].as_str()).map(str::to_string),
            year: None,
            released: None,
            kind: crate::catalog::AlbumKind::Album,
        };
        let tracks = self.album_tracks(&release).await?;
        Ok((release.title, release.artist, tracks))
    }

    /// An artist's name and top songs.
    pub async fn artist_top(&self, id: &str) -> Result<(String, Vec<Track>)> {
        let v = self.get(&format!("{API}/artist/{id}"), &[]).await?;
        let name = v["name"].as_str().unwrap_or("Deezer artist").to_string();
        let top = self.get(&format!("{API}/artist/{id}/top"), &[("limit", "50")]).await?;
        let list: Vec<DTrack> = serde_json::from_value(top["data"].clone()).unwrap_or_default();
        Ok((name, list.into_iter().filter_map(convert).collect()))
    }

    /// One song by id.
    pub async fn track_by_id(&self, id: &str) -> Result<Track> {
        convert(self.fetch_track(id).await?).ok_or_else(|| ProviderError::NotFound.into())
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
        self.search_page(query, 0).await
    }

    async fn search_page(&self, query: &str, page: usize) -> Result<Vec<Track>> {
        let index = (page * 20).to_string();
        let v = self.get(&format!("{API}/search"), &[("q", query), ("limit", "20"), ("index", index.as_str())]).await?;
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
