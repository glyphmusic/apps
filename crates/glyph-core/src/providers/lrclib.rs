//! LRCLIB: synchronized lyrics (`/api/get`, fallback `/api/search`). No key.

use crate::dedupe::{artist_similarity, title_similarity};
use crate::lyrics::Lyrics;
use crate::provider::*;
use anyhow::Result;
use async_trait::async_trait;
use glyph_protocol::{Capabilities, PlaybackSource, ProviderId, Track};
use serde::Deserialize;

const API: &str = "https://lrclib.net/api";

pub struct LrcLib {
    http: reqwest::Client,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LrcRecord {
    #[serde(deserialize_with = "de_flex_string")]
    pub id: String,
    #[serde(default)]
    pub track_name: String,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub album_name: Option<String>,
    #[serde(default, deserialize_with = "de_opt_f64")]
    pub duration: Option<f64>,
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default)]
    pub plain_lyrics: Option<String>,
    #[serde(default)]
    pub synced_lyrics: Option<String>,
}

impl LrcRecord {
    /// How well this record matches the wanted song, 0.0..=1.0.
    pub fn match_score(&self, title: &str, artist: &str, duration_ms: Option<u64>) -> f32 {
        let ts = title_similarity(title, &self.track_name);
        let a = artist_similarity(artist, &self.artist_name);
        let ds = match (duration_ms, self.duration) {
            (Some(want), Some(have)) => {
                let diff = (want as f64 / 1000.0 - have).abs();
                if diff <= 1.5 {
                    1.0
                } else {
                    (1.0 - (diff - 1.5) / 10.0).clamp(0.0, 1.0) as f32
                }
            }
            _ => 0.6,
        };
        (ts * 0.5 + a * 0.3 + ds * 0.2).clamp(0.0, 1.0)
    }

    pub fn to_lyrics(&self, score: f32) -> Lyrics {
        let synced = self.synced_lyrics.as_deref().map(Lyrics::parse_lrc).unwrap_or_default();
        Lyrics {
            provider: "LRCLIB".into(),
            match_score: score,
            synced,
            plain: self.plain_lyrics.clone().filter(|s| !s.trim().is_empty()),
            instrumental: self.instrumental,
        }
    }
}

/// Picks the best record of a `/api/search` result (needs a minimum match,
/// prefers synced lyrics on near-ties).
pub fn pick_best(records: &[LrcRecord], title: &str, artist: &str, duration_ms: Option<u64>) -> Option<(LrcRecord, f32)> {
    records
        .iter()
        .map(|r| {
            let s = r.match_score(title, artist, duration_ms);
            let rank = s + if r.synced_lyrics.as_deref().map(|l| !l.trim().is_empty()).unwrap_or(false) { 0.03 } else { 0.0 };
            (r, s, rank)
        })
        .filter(|(_, s, _)| *s >= 0.6)
        .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(r, s, _)| (r.clone(), s))
}

impl LrcLib {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    async fn get_exact(&self, title: &str, artist: &str, album: Option<&str>, duration_ms: Option<u64>) -> Result<Option<LrcRecord>> {
        let mut q: Vec<(&str, String)> = vec![("track_name", title.to_string()), ("artist_name", artist.to_string())];
        if let Some(a) = album {
            q.push(("album_name", a.to_string()));
        }
        if let Some(d) = duration_ms {
            q.push(("duration", (d / 1000).to_string()));
        }
        match send_json::<LrcRecord>(self.http.get(format!("{API}/get")).query(&q)).await {
            Ok(r) => Ok(Some(r)),
            Err(e) if matches!(e.downcast_ref::<ProviderError>(), Some(ProviderError::NotFound)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn search_records(&self, title: &str, artist: &str) -> Result<Vec<LrcRecord>> {
        let q = [("track_name", title), ("artist_name", artist)];
        send_json(self.http.get(format!("{API}/search")).query(&q)).await
    }

    /// Looks up lyrics: `/api/get` first, `/api/search` as fallback.
    pub async fn lookup(
        &self,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_ms: Option<u64>,
    ) -> Result<Option<Lyrics>> {
        let exact = self.get_exact(title, artist, album, duration_ms).await;
        if let Ok(Some(rec)) = &exact {
            let score = rec.match_score(title, artist, duration_ms);
            return Ok(Some(rec.to_lyrics(score)));
        }
        let search = self.search_records(title, artist).await;
        match search {
            Ok(records) => Ok(pick_best(&records, title, artist, duration_ms).map(|(r, s)| r.to_lyrics(s))),
            Err(e) => match exact {
                Err(first) => Err(first),
                Ok(_) => Err(e),
            },
        }
    }
}

#[async_trait]
impl MusicProvider for LrcLib {
    fn id(&self) -> ProviderId {
        ProviderId::LrcLib
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::MetadataOnly
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let records: Vec<LrcRecord> = send_json(self.http.get(format!("{API}/search")).query(&[("q", query)])).await?;
        Ok(records
            .into_iter()
            .take(20)
            .map(|r| Track {
                id: r.id,
                source: ProviderId::LrcLib,
                title: r.track_name,
                artist: r.artist_name,
                album: r.album_name,
                duration_ms: r.duration.and_then(secs_to_ms),
                cover_url: None,
                isrc: None,
                permalink: None,
                external_ids: Default::default(),
            })
            .collect())
    }

    async fn track(&self, id: &str) -> Result<Track> {
        let r: LrcRecord = send_json(self.http.get(format!("{API}/get/{id}"))).await?;
        Ok(Track {
            id: r.id,
            source: ProviderId::LrcLib,
            title: r.track_name,
            artist: r.artist_name,
            album: r.album_name,
            duration_ms: r.duration.and_then(secs_to_ms),
            cover_url: None,
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
        })
    }

    async fn playback_source(&self, _track: &Track) -> Result<PlaybackSource> {
        Err(ProviderError::Unsupported.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(title: &str, artist: &str, dur: f64, synced: bool) -> LrcRecord {
        LrcRecord {
            id: "1".into(),
            track_name: title.into(),
            artist_name: artist.into(),
            album_name: None,
            duration: Some(dur),
            instrumental: false,
            plain_lyrics: Some("la".into()),
            synced_lyrics: synced.then(|| "[00:01.00] la".to_string()),
        }
    }

    #[test]
    fn scores_exact_match_high() {
        let r = rec("Believer", "Imagine Dragons", 204.0, true);
        assert!(r.match_score("Believer", "Imagine Dragons", Some(204_000)) > 0.98);
        assert!(r.match_score("Other Song", "Someone", Some(100_000)) < 0.4);
    }

    #[test]
    fn picks_best_with_threshold() {
        let recs = vec![rec("Believer (Live)", "Imagine Dragons", 260.0, true), rec("Believer", "Imagine Dragons", 204.0, false)];
        let (best, score) = pick_best(&recs, "Believer", "Imagine Dragons", Some(204_000)).unwrap();
        assert_eq!(best.track_name, "Believer");
        assert!(score > 0.9);
        assert!(pick_best(&recs, "Completely Different", "Nobody", Some(10_000)).is_none());
    }

    #[test]
    fn parses_api_json_and_synced() {
        let json = r#"{"id":379,"trackName":"Believer","artistName":"Imagine Dragons","albumName":"Evolve",
            "duration":204.0,"instrumental":false,"plainLyrics":"First","syncedLyrics":"[00:01.50] First\n[00:03.00] Second"}"#;
        let r: LrcRecord = serde_json::from_str(json).unwrap();
        let l = r.to_lyrics(0.99);
        assert_eq!(l.synced.len(), 2);
        assert_eq!(l.synced[0].time_ms, 1500);
    }
}
