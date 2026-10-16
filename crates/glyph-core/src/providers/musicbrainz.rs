//! MusicBrainz (metadata, ISRC lookup) + Cover Art Archive (covers).
//! Rate limit: 1 request/second, descriptive User-Agent (set on the shared client).

use crate::provider::*;
use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::{Capabilities, PlaybackSource, ProviderId, Track};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const API: &str = "https://musicbrainz.org/ws/2";
const MIN_INTERVAL: Duration = Duration::from_millis(1100);

pub struct MusicBrainz {
    http: reqwest::Client,
    last: Mutex<Option<Instant>>,
}

#[derive(Debug, Deserialize)]
struct RecordingList {
    #[serde(default)]
    recordings: Vec<Recording>,
}

#[derive(Debug, Deserialize)]
struct Recording {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    length: Option<u64>,
    #[serde(default, rename = "artist-credit")]
    artist_credit: Vec<ArtistCredit>,
    #[serde(default)]
    releases: Vec<Release>,
    #[serde(default)]
    isrcs: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ArtistCredit {
    #[serde(default)]
    name: String,
    #[serde(default)]
    joinphrase: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Release {
    id: String,
    #[serde(default)]
    title: Option<String>,
}

/// Cover Art Archive front image (500 px) for a release MBID. 404 when none exists.
pub fn caa_front_url(release_mbid: &str) -> String {
    format!("https://coverartarchive.org/release/{release_mbid}/front-500")
}

fn artist_name(credits: &[ArtistCredit]) -> String {
    let mut s = String::new();
    for c in credits {
        s.push_str(&c.name);
        s.push_str(c.joinphrase.as_deref().unwrap_or(""));
    }
    if s.trim().is_empty() {
        "Unknown".into()
    } else {
        s
    }
}

fn convert(r: &Recording) -> Track {
    Track {
        id: r.id.clone(),
        source: ProviderId::MusicBrainz,
        title: r.title.clone(),
        artist: artist_name(&r.artist_credit),
        album: r.releases.first().and_then(|rel| rel.title.clone()),
        duration_ms: r.length.filter(|l| *l > 0),
        cover_url: r.releases.first().map(|rel| caa_front_url(&rel.id)),
        isrc: r.isrcs.first().cloned(),
        permalink: Some(format!("https://musicbrainz.org/recording/{}", r.id)),
        external_ids: Default::default(),
    }
}

pub fn parse_recordings(json: &str) -> Result<Vec<Track>> {
    let l: RecordingList = serde_json::from_str(json)?;
    Ok(l.recordings.iter().map(convert).collect())
}

fn lucene_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', " ").replace('"', " "))
}

impl MusicBrainz {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http, last: Mutex::new(None) }
    }

    /// Serialises requests to at most one per [`MIN_INTERVAL`].
    async fn throttle(&self) {
        let mut last = self.last.lock().await;
        if let Some(t) = *last {
            let el = t.elapsed();
            if el < MIN_INTERVAL {
                tokio::time::sleep(MIN_INTERVAL - el).await;
            }
        }
        *last = Some(Instant::now());
    }

    async fn recordings(&self, query: &str, limit: &str) -> Result<Vec<Recording>> {
        self.throttle().await;
        let req = self
            .http
            .get(format!("{API}/recording"))
            .query(&[("query", query), ("fmt", "json"), ("limit", limit)]);
        let l: RecordingList = send_json(req).await?;
        Ok(l.recordings)
    }

    /// Candidate Cover Art Archive URLs (best first) for a song.
    pub async fn cover_urls(&self, artist: &str, title: &str) -> Result<Vec<String>> {
        let q = format!("recording:{} AND artist:{}", lucene_quote(title), lucene_quote(artist));
        let recs = self.recordings(&q, "5").await?;
        let mut urls = Vec::new();
        for r in recs {
            for rel in r.releases.iter().take(2) {
                let u = caa_front_url(&rel.id);
                if !urls.contains(&u) {
                    urls.push(u);
                }
            }
            if urls.len() >= 4 {
                break;
            }
        }
        Ok(urls)
    }
}

#[async_trait]
impl MusicProvider for MusicBrainz {
    fn id(&self) -> ProviderId {
        ProviderId::MusicBrainz
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::MetadataOnly
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        Ok(self.recordings(query, "10").await?.iter().map(convert).collect())
    }

    async fn track(&self, id: &str) -> Result<Track> {
        self.throttle().await;
        let req = self
            .http
            .get(format!("{API}/recording/{id}"))
            .query(&[("fmt", "json"), ("inc", "artist-credits+releases+isrcs")]);
        let r: Recording = send_json(req).await?;
        Ok(convert(&r))
    }

    async fn playback_source(&self, _track: &Track) -> Result<PlaybackSource> {
        Err(ProviderError::Unsupported.into())
    }

    async fn find_by_isrc(&self, isrc: &str) -> Result<Option<Track>> {
        self.throttle().await;
        let req = self
            .http
            .get(format!("{API}/isrc/{isrc}"))
            .query(&[("fmt", "json"), ("inc", "artist-credits+releases")]);
        match send_json::<RecordingList>(req).await {
            Ok(l) => Ok(l.recordings.first().map(convert)),
            Err(e) if matches!(e.downcast_ref::<ProviderError>(), Some(ProviderError::NotFound)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_recordings() {
        let json = r#"{"recordings":[{"id":"r1","title":"Song","length":201000,
          "artist-credit":[{"name":"A","joinphrase":" & "},{"name":"B"}],
          "releases":[{"id":"rel1","title":"LP"}]}]}"#;
        let t = parse_recordings(json).unwrap();
        assert_eq!(t[0].artist, "A & B");
        assert_eq!(t[0].album.as_deref(), Some("LP"));
        assert_eq!(t[0].cover_url.as_deref(), Some("https://coverartarchive.org/release/rel1/front-500"));
        assert_eq!(lucene_quote("a\"b"), "\"a b\"");
    }
}
