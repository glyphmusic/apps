//! Import & migration: public playlist links (Spotify, Deezer, YouTube,
//! Last.fm profiles) via the server or directly, and files (CSV, Spotify JSON
//! export, M3U/M3U8, XSPF). Matching is ISRC-first (handled by the resolver),
//! results are bucketed into sure / unsure / not found.

pub mod files;

use crate::providers::Providers;
use crate::resolver::{MIN_CONFIDENCE, SURE_CONFIDENCE};
use crate::server_client::ServerClient;
use crate::{flume_like, ImportMatch, ImportProgress, ImportResult, Resolved};
use anyhow::{anyhow, Result};
use files::ParsedList;
use futures::stream::StreamExt;
use glyph_protocol::{PlaybackSource, Track};
use std::future::Future;

/// A playlist-like thing a pasted link points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteSource {
    SpotifyPlaylist(String),
    DeezerPlaylist(String),
    DeezerUser(String),
    YouTubePlaylist(String),
    LastFmLoved(String),
}

impl RemoteSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SpotifyPlaylist(_) => "spotify/playlist",
            Self::DeezerPlaylist(_) => "deezer/playlist",
            Self::DeezerUser(_) => "deezer/favorites",
            Self::YouTubePlaylist(_) => "youtube/playlist",
            Self::LastFmLoved(_) => "lastfm/loved",
        }
    }

    /// Handled by the server's `/v1/import/spotify` (Last.fm is direct only).
    pub fn server_capable(&self) -> bool {
        !matches!(self, Self::LastFmLoved(_))
    }
}

fn is_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Recognises playlist URLs (used by the search bar to detect pasted links).
pub fn parse_url(input: &str) -> Option<RemoteSource> {
    let input = input.trim();
    if let Some(id) = input.strip_prefix("spotify:playlist:") {
        return is_id(id).then(|| RemoteSource::SpotifyPlaylist(id.to_string()));
    }
    let url = reqwest::Url::parse(input).ok()?;
    let host = url.host_str()?.trim_start_matches("www.").to_lowercase();
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let after = |name: &str| segs.iter().position(|s| *s == name).and_then(|i| segs.get(i + 1)).copied();
    match host.as_str() {
        "open.spotify.com" | "play.spotify.com" => {
            after("playlist").filter(|id| is_id(id)).map(|id| RemoteSource::SpotifyPlaylist(id.to_string()))
        }
        "deezer.com" | "deezer.page.link" => {
            if let Some(id) = after("playlist").filter(|id| id.chars().all(|c| c.is_ascii_digit())) {
                Some(RemoteSource::DeezerPlaylist(id.to_string()))
            } else {
                after("profile")
                    .or_else(|| after("user"))
                    .filter(|id| id.chars().all(|c| c.is_ascii_digit()))
                    .map(|id| RemoteSource::DeezerUser(id.to_string()))
            }
        }
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtu.be" => url
            .query_pairs()
            .find(|(k, _)| k == "list")
            .map(|(_, v)| v.to_string())
            .filter(|id| is_id(id))
            .map(RemoteSource::YouTubePlaylist),
        "last.fm" | "lastfm.de" | "lastfm.com" => after("user").map(|u| RemoteSource::LastFmLoved(u.to_string())),
        _ => None,
    }
}

/// Bucket of a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    Sure,
    Unsure,
    NotFound,
}

/// Previews are never a "found" song (no full source), nothing resolved neither.
pub fn bucket_of(resolved: &Option<Resolved>) -> Bucket {
    match resolved {
        None => Bucket::NotFound,
        Some(r) if matches!(r.source, PlaybackSource::Preview { .. }) => Bucket::NotFound,
        Some(r) if r.confidence >= SURE_CONFIDENCE => Bucket::Sure,
        Some(r) if r.confidence >= MIN_CONFIDENCE => Bucket::Unsure,
        Some(_) => Bucket::NotFound,
    }
}

/// Reads the track list of a link or file.
pub async fn fetch_list(
    input: &crate::ImportInput,
    providers: &Providers,
    server: Option<&ServerClient>,
) -> Result<(ParsedList, Option<String>)> {
    match input {
        crate::ImportInput::File(path) => {
            let path = path.clone();
            let list = tokio::task::spawn_blocking(move || files::parse_file(&path)).await??;
            Ok((list, None))
        }
        crate::ImportInput::Url(url) => {
            let url = url.trim().to_string();
            let src = parse_url(&url).ok_or_else(|| anyhow!("not a supported playlist link: {url}"))?;
            if src.server_capable() {
                if let Some(server) = server {
                    match server.import(&url).await {
                        Ok(r) => {
                            let label = format!("{}/playlist", format!("{:?}", r.source).to_lowercase());
                            return Ok((ParsedList { name: r.name, label, tracks: r.tracks }, Some(url)));
                        }
                        Err(e) => tracing::warn!("server import failed, going direct: {e}"),
                    }
                }
            }
            let list = fetch_direct(&src, providers).await?;
            Ok((list, Some(url)))
        }
    }
}

async fn fetch_direct(src: &RemoteSource, p: &Providers) -> Result<ParsedList> {
    let label = src.label().to_string();
    let (name, tracks) = match src {
        RemoteSource::SpotifyPlaylist(id) => {
            let sp = p.spotify.as_ref().ok_or_else(|| anyhow!("Spotify client id/secret missing (config.toml or GLYPH_SPOTIFY_*)"))?;
            (sp.playlist_name(id).await.unwrap_or_else(|_| "Spotify playlist".into()), sp.playlist_tracks(id).await?)
        }
        RemoteSource::DeezerPlaylist(id) => {
            let dz = p.deezer.as_ref().ok_or_else(|| anyhow!("Deezer is disabled"))?;
            dz.playlist(id).await?
        }
        RemoteSource::DeezerUser(id) => {
            let dz = p.deezer.as_ref().ok_or_else(|| anyhow!("Deezer is disabled"))?;
            dz.user_favorites(id).await?
        }
        RemoteSource::YouTubePlaylist(id) => {
            let yt = p.youtube.as_ref().ok_or_else(|| anyhow!("YouTube API key missing (config.toml or GLYPH_YOUTUBE_KEY)"))?;
            (yt.playlist_title(id).await.unwrap_or_else(|_| "YouTube playlist".into()), yt.playlist_items(id).await?)
        }
        RemoteSource::LastFmLoved(user) => {
            let lf = p.lastfm.as_ref().ok_or_else(|| anyhow!("Last.fm API key missing (config.toml or GLYPH_LASTFM_KEY)"))?;
            (format!("{user} · loved tracks"), lf.loved_tracks(user).await?)
        }
    };
    Ok(ParsedList { name, label, tracks })
}

/// Matches every track with `resolve` (4 in parallel, order preserved),
/// reporting progress as `IMPORT <label> ██████░░ done/total`.
pub async fn match_all<F, Fut>(
    list: ParsedList,
    origin: Option<String>,
    progress: flume_like::Sender<ImportProgress>,
    resolve: F,
) -> ImportResult
where
    F: Fn(Track) -> Fut,
    Fut: Future<Output = Option<Resolved>>,
{
    let total = list.tracks.len();
    let label = format!("IMPORT {}", list.label);
    let _ = progress.send(ImportProgress { label: label.clone(), done: 0, total });
    let mut result = ImportResult {
        name: list.name,
        sure: vec![],
        unsure: vec![],
        not_found: vec![],
        origin,
    };
    let mut stream = futures::stream::iter(list.tracks.into_iter().map(|t| {
        let fut = resolve(t.clone());
        async move { (t, fut.await) }
    }))
    .buffered(4);
    let mut done = 0;
    while let Some((original, resolved)) = stream.next().await {
        done += 1;
        let m = ImportMatch { original, resolved };
        match bucket_of(&m.resolved) {
            Bucket::Sure => result.sure.push(m),
            Bucket::Unsure => result.unsure.push(m),
            Bucket::NotFound => result.not_found.push(m),
        }
        let _ = progress.send(ImportProgress { label: label.clone(), done, total });
    }
    result
}

/// `IMPORT spotify/playlist ██████░░ 98/142`
pub fn format_progress(p: &ImportProgress, width: usize) -> String {
    let filled = if p.total == 0 { width } else { (p.done * width / p.total).min(width) };
    format!(
        "{} {}{} {}/{}",
        p.label,
        "█".repeat(filled),
        "░".repeat(width - filled),
        p.done,
        p.total
    )
}

/// `matched: 139/142 (97%)` (sure + unsure count as matched).
pub fn summary(r: &ImportResult) -> String {
    let total = r.sure.len() + r.unsure.len() + r.not_found.len();
    let matched = r.sure.len() + r.unsure.len();
    let pct = if total == 0 { 100 } else { matched * 100 / total };
    format!("matched: {matched}/{total} ({pct}%)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedupe::test_track;
    use glyph_protocol::ProviderId;

    #[test]
    fn recognises_links() {
        use RemoteSource::*;
        assert_eq!(
            parse_url("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc"),
            Some(SpotifyPlaylist("37i9dQZF1DXcBWIGoYBM5M".into()))
        );
        assert_eq!(
            parse_url("https://open.spotify.com/intl-de/playlist/37i9dQZF1DXcBWIGoYBM5M"),
            Some(SpotifyPlaylist("37i9dQZF1DXcBWIGoYBM5M".into()))
        );
        assert_eq!(parse_url("spotify:playlist:abc123"), Some(SpotifyPlaylist("abc123".into())));
        assert_eq!(parse_url("https://www.deezer.com/en/playlist/1313621735"), Some(DeezerPlaylist("1313621735".into())));
        assert_eq!(parse_url("https://www.deezer.com/fr/profile/5"), Some(DeezerUser("5".into())));
        assert_eq!(
            parse_url("https://music.youtube.com/playlist?list=PLabc-_123"),
            Some(YouTubePlaylist("PLabc-_123".into()))
        );
        assert_eq!(parse_url("https://www.youtube.com/watch?v=x"), None);
        assert_eq!(parse_url("https://www.last.fm/user/rj/loved"), Some(LastFmLoved("rj".into())));
        assert_eq!(parse_url("https://example.com/playlist/1"), None);
        assert_eq!(parse_url("daft punk"), None);
    }

    fn resolved(conf: f32, preview: bool) -> Option<Resolved> {
        Some(Resolved {
            track: test_track(ProviderId::Audius, "a", "t", "a", None),
            source: if preview {
                PlaybackSource::Preview { provider: ProviderId::Deezer, url: "u".into() }
            } else {
                PlaybackSource::Stream { provider: ProviderId::Audius, url: "u".into(), mime: None }
            },
            confidence: conf,
        })
    }

    #[test]
    fn buckets() {
        assert_eq!(bucket_of(&resolved(0.95, false)), Bucket::Sure);
        assert_eq!(bucket_of(&resolved(0.85, false)), Bucket::Sure);
        assert_eq!(bucket_of(&resolved(0.6, false)), Bucket::Unsure);
        assert_eq!(bucket_of(&resolved(0.3, false)), Bucket::NotFound);
        assert_eq!(bucket_of(&resolved(1.0, true)), Bucket::NotFound);
        assert_eq!(bucket_of(&None), Bucket::NotFound);
    }

    #[test]
    fn progress_format() {
        let p = ImportProgress { label: "IMPORT spotify/playlist".into(), done: 98, total: 142 };
        assert_eq!(format_progress(&p, 8), "IMPORT spotify/playlist █████░░░ 98/142");
        let r = ImportResult {
            name: "x".into(),
            sure: vec![ImportMatch { original: test_track(ProviderId::Spotify, "1", "a", "b", None), resolved: resolved(0.9, false) }; 139],
            unsure: vec![],
            not_found: vec![ImportMatch { original: test_track(ProviderId::Spotify, "1", "a", "b", None), resolved: None }; 3],
            origin: None,
        };
        assert_eq!(summary(&r), "matched: 139/142 (97%)");
    }

    #[tokio::test]
    async fn match_all_buckets_in_order_and_reports_progress() {
        let list = ParsedList {
            name: "L".into(),
            label: "file/csv".into(),
            tracks: (0..6).map(|i| test_track(ProviderId::Spotify, &i.to_string(), &format!("t{i}"), "a", None)).collect(),
        };
        let (tx, mut rx) = flume_like::channel();
        let res = match_all(list, Some("o".into()), tx, |t: Track| async move {
            match t.id.as_str() {
                "0" | "1" => resolved(0.95, false),
                "2" => resolved(0.6, false),
                "3" => resolved(1.0, true),
                _ => None,
            }
        })
        .await;
        assert_eq!((res.sure.len(), res.unsure.len(), res.not_found.len()), (2, 1, 3));
        assert_eq!(res.sure[0].original.id, "0");
        assert_eq!(res.origin.as_deref(), Some("o"));
        let mut last = None;
        let mut n = 0;
        while let Ok(p) = rx.try_recv() {
            n += 1;
            last = Some(p);
        }
        assert_eq!(n, 7);
        assert_eq!(last.unwrap().done, 6);
    }
}
