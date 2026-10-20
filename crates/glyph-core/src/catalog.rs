//! Catalog facts for artist and album pages from Deezer's public API (no key):
//! fan count + "verified" for artists, release year + type for albums.
//!
//! "Verified" means the name has an official artist profile in the catalog
//! (exact name match with a real audience), not a user upload.

use crate::dedupe::normalize_artist;
use crate::provider::send_json;
use anyhow::Result;
use serde::Deserialize;

const API: &str = "https://api.deezer.com";
/// Fans needed before a matching profile counts as the official one.
const VERIFIED_FANS: u64 = 1000;

#[derive(Debug, Clone, PartialEq)]
pub struct ArtistStats {
    pub fans: u64,
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumKind {
    Album,
    Ep,
    Single,
    Compilation,
}

impl AlbumKind {
    /// Guess from the number of songs (streaming-store rules of thumb).
    pub fn from_count(n: usize) -> Self {
        match n {
            0..=3 => AlbumKind::Single,
            4..=6 => AlbumKind::Ep,
            _ => AlbumKind::Album,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "album" => Some(AlbumKind::Album),
            "ep" => Some(AlbumKind::Ep),
            "single" => Some(AlbumKind::Single),
            "compile" | "compilation" => Some(AlbumKind::Compilation),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlbumMeta {
    pub year: Option<u16>,
    pub kind: AlbumKind,
}

#[derive(Deserialize)]
struct List<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

#[derive(Deserialize)]
struct DArtist {
    #[serde(default)]
    name: String,
    #[serde(default)]
    nb_fan: u64,
}

#[derive(Deserialize)]
struct DAlbumHit {
    id: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    record_type: Option<String>,
    #[serde(default)]
    artist: Option<DArtist>,
}

#[derive(Deserialize)]
struct DAlbum {
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    record_type: Option<String>,
}

fn pick_artist(hits: Vec<DArtist>, name: &str) -> Option<ArtistStats> {
    let want = normalize_artist(name);
    hits.into_iter()
        .filter(|a| normalize_artist(&a.name) == want)
        .max_by_key(|a| a.nb_fan)
        .map(|a| ArtistStats { fans: a.nb_fan, verified: a.nb_fan >= VERIFIED_FANS })
}

pub async fn artist_stats(http: &reqwest::Client, name: &str) -> Result<Option<ArtistStats>> {
    let r: List<DArtist> = send_json(http.get(format!("{API}/search/artist")).query(&[("q", name), ("limit", "5")])).await?;
    Ok(pick_artist(r.data, name))
}

fn squash(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

pub async fn album_meta(http: &reqwest::Client, title: &str, artist: &str) -> Result<Option<AlbumMeta>> {
    let q = format!("artist:\"{artist}\" album:\"{title}\"");
    let r: List<DAlbumHit> = send_json(http.get(format!("{API}/search/album")).query(&[("q", q.as_str()), ("limit", "5")])).await?;
    let want_t = squash(title);
    let want_a = normalize_artist(artist);
    let Some(hit) = r.data.into_iter().find(|h| {
        squash(&h.title) == want_t && h.artist.as_ref().is_some_and(|a| normalize_artist(&a.name) == want_a)
    }) else {
        return Ok(None);
    };
    let a: DAlbum = send_json(http.get(format!("{API}/album/{}", hit.id))).await?;
    let kind = a
        .record_type
        .as_deref()
        .or(hit.record_type.as_deref())
        .and_then(AlbumKind::parse)
        .unwrap_or(AlbumKind::Album);
    let year = a.release_date.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()).filter(|y| *y > 1900);
    Ok(Some(AlbumMeta { year, kind }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_needs_exact_name_and_audience() {
        let hits = vec![
            DArtist { name: "Oliver Tree".into(), nb_fan: 812_000 },
            DArtist { name: "Oliver Tree Fan".into(), nb_fan: 9_000_000 },
        ];
        assert_eq!(pick_artist(hits, "oliver tree"), Some(ArtistStats { fans: 812_000, verified: true }));
        let small = vec![DArtist { name: "Bedroom Act".into(), nb_fan: 12 }];
        assert_eq!(pick_artist(small, "Bedroom Act").map(|s| s.verified), Some(false));
    }

    #[test]
    fn kinds() {
        assert_eq!(AlbumKind::from_count(1), AlbumKind::Single);
        assert_eq!(AlbumKind::from_count(5), AlbumKind::Ep);
        assert_eq!(AlbumKind::from_count(11), AlbumKind::Album);
        assert_eq!(AlbumKind::parse("compile"), Some(AlbumKind::Compilation));
    }
}
