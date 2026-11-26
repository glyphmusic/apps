//! Catalog facts for artist and album pages (direct/offline fallback when no
//! Glyph server is configured): fan count, service-based artist verification
//! (no arbitrary fan-count cutoff), and album release year + type.

use crate::dedupe::normalize_artist;
use crate::provider::send_json;
use anyhow::Result;
use serde::Deserialize;

const DEEZER_API: &str = "https://api.deezer.com";
const AUDIUS_API: &str = "https://api.audius.co/v1";
const ITUNES_API: &str = "https://itunes.apple.com";

#[derive(Debug, Clone, PartialEq)]
pub struct ArtistStats {
    pub fans: u64,
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    #[serde(default)]
    nb_album: Option<u64>,
    #[serde(default)]
    picture_xl: Option<String>,
    #[serde(default)]
    picture_big: Option<String>,
    #[serde(default)]
    picture_medium: Option<String>,
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

#[derive(Deserialize)]
struct AudiusUser {
    #[serde(default)]
    name: String,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    is_verified: bool,
}

#[derive(Deserialize)]
struct ItunesResp {
    #[serde(default)]
    results: Vec<ItunesArtist>,
}

#[derive(Deserialize)]
struct ItunesArtist {
    #[serde(rename = "artistName", default)]
    artist_name: String,
}

fn pick_deezer_artist(hits: Vec<DArtist>, name: &str) -> Option<DArtist> {
    let want = normalize_artist(name);
    if want.is_empty() {
        return None;
    }
    hits.into_iter()
        .filter(|a| normalize_artist(&a.name) == want)
        .max_by_key(|a| (a.nb_album.unwrap_or(0) > 0, a.nb_fan))
}

fn evaluate_service_verified(
    audius_verified: Option<bool>,
    deezer_official: Option<bool>,
    itunes_official: Option<bool>,
) -> Option<bool> {
    if audius_verified == Some(true) {
        return Some(true);
    }
    let confirmations = usize::from(deezer_official == Some(true)) + usize::from(itunes_official == Some(true));
    if confirmations >= 2 {
        return Some(true);
    }
    let answered = usize::from(audius_verified.is_some())
        + usize::from(deezer_official.is_some())
        + usize::from(itunes_official.is_some());
    if answered >= 2 {
        Some(false)
    } else {
        None
    }
}

/// Service-based artist verification + portrait lookup for direct/no-server mode.
/// Returns `verified: None` on transient network/quota errors so callers never
/// cache a temporary failure as `false`.
pub async fn verify_artist_direct(http: &reqwest::Client, name: &str) -> glyph_protocol::ArtistMetaItem {
    let want = normalize_artist(name);
    if want.is_empty() {
        return glyph_protocol::ArtistMetaItem { name: name.to_string(), photo: None, verified: Some(false) };
    }
    let dz_fut = send_json::<List<DArtist>>(
        http.get(format!("{DEEZER_API}/search/artist")).query(&[("q", name), ("limit", "10")]),
    );
    let it_fut = send_json::<ItunesResp>(
        http.get(format!("{ITUNES_API}/search")).query(&[("term", name), ("media", "music"), ("entity", "musicArtist"), ("limit", "6")]),
    );
    let au_fut = send_json::<List<AudiusUser>>(
        http.get(format!("{AUDIUS_API}/users/search")).query(&[("query", name), ("limit", "5"), ("app_name", "glyph")]),
    );
    let (dz_res, it_res, au_res) = tokio::join!(dz_fut, it_fut, au_fut);

    let dz_hit = dz_res.as_ref().ok().and_then(|l| {
        l.data
            .iter()
            .filter(|a| normalize_artist(&a.name) == want)
            .max_by_key(|a| (a.nb_album.unwrap_or(0) > 0, a.nb_fan))
    });
    let deezer_official = match &dz_res {
        Ok(_) => Some(dz_hit.is_some_and(|a| a.nb_album.unwrap_or(1) > 0)),
        Err(_) => None,
    };
    let itunes_official = match &it_res {
        Ok(r) => Some(r.results.iter().any(|a| normalize_artist(&a.artist_name) == want)),
        Err(_) => None,
    };
    let audius_verified = match &au_res {
        Ok(l) => Some(l.data.iter().any(|u| {
            let match_name = normalize_artist(&u.name) == want
                || u.handle.as_deref().is_some_and(|h| normalize_artist(h) == want);
            match_name && u.is_verified
        })),
        Err(_) => None,
    };
    let photo = dz_hit.and_then(|d| {
        d.picture_xl
            .clone()
            .or_else(|| d.picture_big.clone())
            .or_else(|| d.picture_medium.clone())
    });
    let verified = evaluate_service_verified(audius_verified, deezer_official, itunes_official);
    glyph_protocol::ArtistMetaItem { name: name.to_string(), photo, verified }
}

pub async fn artist_stats(http: &reqwest::Client, name: &str) -> Result<Option<ArtistStats>> {
    let dz_fut = send_json::<List<DArtist>>(
        http.get(format!("{DEEZER_API}/search/artist")).query(&[("q", name), ("limit", "10")]),
    );
    let it_fut = send_json::<ItunesResp>(
        http.get(format!("{ITUNES_API}/search")).query(&[("term", name), ("media", "music"), ("entity", "musicArtist"), ("limit", "6")]),
    );
    let au_fut = send_json::<List<AudiusUser>>(
        http.get(format!("{AUDIUS_API}/users/search")).query(&[("query", name), ("limit", "5"), ("app_name", "glyph")]),
    );
    let (dz_res, it_res, au_res) = tokio::join!(dz_fut, it_fut, au_fut);
    let dz_hit = pick_deezer_artist(dz_res?.data, name);
    let want = normalize_artist(name);
    let deezer_official = Some(dz_hit.as_ref().is_some_and(|a| a.nb_album.unwrap_or(1) > 0));
    let itunes_official = it_res.ok().map(|r| r.results.iter().any(|a| normalize_artist(&a.artist_name) == want));
    let audius_verified = au_res.ok().map(|l| {
        l.data.iter().any(|u| {
            let match_name = normalize_artist(&u.name) == want
                || u.handle.as_deref().is_some_and(|h| normalize_artist(h) == want);
            match_name && u.is_verified
        })
    });
    let verified = evaluate_service_verified(audius_verified, deezer_official, itunes_official).unwrap_or(false);
    Ok(dz_hit.map(|a| ArtistStats { fans: a.nb_fan, verified }).or_else(|| {
        verified.then_some(ArtistStats { fans: 0, verified: true })
    }))
}

fn squash(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

pub async fn album_meta(http: &reqwest::Client, title: &str, artist: &str) -> Result<Option<AlbumMeta>> {
    let q = format!("artist:\"{artist}\" album:\"{title}\"");
    let r: List<DAlbumHit> = send_json(http.get(format!("{DEEZER_API}/search/album")).query(&[("q", q.as_str()), ("limit", "5")])).await?;
    let want_t = squash(title);
    let want_a = normalize_artist(artist);
    let Some(hit) = r.data.into_iter().find(|h| {
        squash(&h.title) == want_t && h.artist.as_ref().is_some_and(|a| normalize_artist(&a.name) == want_a)
    }) else {
        return Ok(None);
    };
    let a: DAlbum = send_json(http.get(format!("{DEEZER_API}/album/{}", hit.id))).await?;
    let kind = a
        .record_type
        .as_deref()
        .or(hit.record_type.as_deref())
        .and_then(AlbumKind::parse)
        .unwrap_or(AlbumKind::Album);
    let year = a.release_date.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()).filter(|y| *y > 1900);
    Ok(Some(AlbumMeta { year, kind }))
}

/// The UTC day `days` days ago as `YYYY-MM-DD` (compares with catalog dates).
pub fn date_days_ago(days: u32) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (y, m, d) = civil_from_days((now / 86_400) as i64 - days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to (year, month, day) (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    #[test]
    fn civil_dates() {
        assert_eq!(super::civil_from_days(0), (1970, 1, 1));
        assert_eq!(super::civil_from_days(20_746), (2026, 10, 20));
        assert_eq!(super::civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(super::date_days_ago(0).len(), 10);
    }

    use super::*;

    #[test]
    fn service_verification_ignores_arbitrary_fan_counts() {
        assert_eq!(evaluate_service_verified(Some(true), Some(false), Some(false)), Some(true));
        assert_eq!(evaluate_service_verified(Some(false), Some(true), Some(true)), Some(true));
        assert_eq!(evaluate_service_verified(Some(false), Some(true), Some(false)), Some(false));
        assert_eq!(evaluate_service_verified(None, None, None), None);
    }

    #[test]
    fn kinds() {
        assert_eq!(AlbumKind::from_count(1), AlbumKind::Single);
        assert_eq!(AlbumKind::from_count(5), AlbumKind::Ep);
        assert_eq!(AlbumKind::from_count(11), AlbumKind::Album);
        assert_eq!(AlbumKind::parse("compile"), Some(AlbumKind::Compilation));
    }
}
