//! Import & migration: public links (Spotify, Deezer, YouTube, SoundCloud,
//! Apple Music, Bandcamp, Last.fm profiles, short links) via the server or
//! directly, files (CSV, Spotify JSON export, M3U/M3U8, XSPF), and whole
//! libraries from a connected account (`accounts`). Matching is ISRC-first
//! (handled by the resolver), results are bucketed into sure / unsure / not
//! found.

pub mod accounts;
pub mod files;
pub mod keyless;
pub mod links;
pub mod local;

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
    SpotifyAlbum(String),
    /// An artist's top songs.
    SpotifyArtist(String),
    SpotifyTrack(String),
    DeezerPlaylist(String),
    DeezerUser(String),
    DeezerAlbum(String),
    DeezerArtist(String),
    DeezerTrack(String),
    YouTubePlaylist(String),
    LastFmLoved(String),
    /// Any soundcloud.com link (set, likes, a user's tracks, one track),
    /// normalised to `https://soundcloud.com/<path>`.
    SoundCloud(String),
    /// Public playlist or album page on music.apple.com.
    AppleMusic(String),
    /// Album or track page on <artist>.bandcamp.com.
    Bandcamp(String),
    /// deezer.page.link, on.soundcloud.com, spotify.link: followed first.
    ShortLink(String),
}

impl RemoteSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SpotifyPlaylist(_) => "spotify/playlist",
            Self::SpotifyAlbum(_) => "spotify/album",
            Self::SpotifyArtist(_) => "spotify/artist",
            Self::SpotifyTrack(_) => "spotify/track",
            Self::DeezerPlaylist(_) => "deezer/playlist",
            Self::DeezerUser(_) => "deezer/favorites",
            Self::DeezerAlbum(_) => "deezer/album",
            Self::DeezerArtist(_) => "deezer/artist",
            Self::DeezerTrack(_) => "deezer/track",
            Self::YouTubePlaylist(_) => "youtube/playlist",
            Self::LastFmLoved(_) => "lastfm/loved",
            Self::SoundCloud(_) => "soundcloud",
            Self::AppleMusic(_) => "apple/playlist",
            Self::Bandcamp(_) => "bandcamp/album",
            Self::ShortLink(_) => "link",
        }
    }

    /// The link types the server's `/v1/import/spotify` reads; the others are
    /// always read by the app itself.
    pub fn server_capable(&self) -> bool {
        matches!(
            self,
            Self::SpotifyPlaylist(_) | Self::DeezerPlaylist(_) | Self::DeezerUser(_) | Self::YouTubePlaylist(_) | Self::LastFmLoved(_)
        )
    }
}

/// soundcloud.com paths that are pages of the site, not music.
const SOUNDCLOUD_RESERVED: [&str; 14] = [
    "discover", "search", "you", "stream", "upload", "settings", "messages", "notifications", "charts", "pages", "mobile", "imprint", "terms-of-use", "pro",
];

fn is_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Recognises playlist URLs (used by the search bar to detect pasted links).
pub fn parse_url(input: &str) -> Option<RemoteSource> {
    let input = input.trim();
    let uris: [(&str, fn(String) -> RemoteSource); 4] = [
        ("spotify:playlist:", RemoteSource::SpotifyPlaylist),
        ("spotify:album:", RemoteSource::SpotifyAlbum),
        ("spotify:artist:", RemoteSource::SpotifyArtist),
        ("spotify:track:", RemoteSource::SpotifyTrack),
    ];
    for (prefix, make) in uris {
        if let Some(id) = input.strip_prefix(prefix) {
            return is_id(id).then(|| make(id.to_string()));
        }
    }
    let url = reqwest::Url::parse(input).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.trim_start_matches("www.").to_lowercase();
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let after = |name: &str| segs.iter().position(|s| *s == name).and_then(|i| segs.get(i + 1)).copied();
    let digits = |id: &&str| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit());
    if links::short_link_targets(&host).is_some() {
        return (!segs.is_empty()).then(|| RemoteSource::ShortLink(input.to_string()));
    }
    if host.ends_with(".bandcamp.com") && segs.len() >= 2 && matches!(segs[0], "album" | "track") {
        return Some(RemoteSource::Bandcamp(format!("https://{host}/{}/{}", segs[0], segs[1])));
    }
    match host.as_str() {
        "open.spotify.com" | "play.spotify.com" => {
            let pick = |kind: &str, make: fn(String) -> RemoteSource| after(kind).filter(|id| is_id(id)).map(|id| make(id.to_string()));
            pick("playlist", RemoteSource::SpotifyPlaylist)
                .or_else(|| pick("album", RemoteSource::SpotifyAlbum))
                .or_else(|| pick("artist", RemoteSource::SpotifyArtist))
                .or_else(|| pick("track", RemoteSource::SpotifyTrack))
        }
        "deezer.com" => {
            let pick = |kind: &str, make: fn(String) -> RemoteSource| after(kind).filter(digits).map(|id| make(id.to_string()));
            pick("playlist", RemoteSource::DeezerPlaylist)
                .or_else(|| pick("album", RemoteSource::DeezerAlbum))
                .or_else(|| pick("artist", RemoteSource::DeezerArtist))
                .or_else(|| pick("track", RemoteSource::DeezerTrack))
                .or_else(|| pick("profile", RemoteSource::DeezerUser))
                .or_else(|| pick("user", RemoteSource::DeezerUser))
        }
        "soundcloud.com" | "m.soundcloud.com" => {
            let first = segs.first()?;
            if SOUNDCLOUD_RESERVED.contains(first) {
                return None;
            }
            Some(RemoteSource::SoundCloud(format!("https://soundcloud.com/{}", segs.join("/"))))
        }
        "music.apple.com" | "itunes.apple.com" => {
            segs.iter().any(|s| *s == "playlist" || *s == "album").then(|| RemoteSource::AppleMusic(input.to_string()))
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
    http: &reqwest::Client,
    keys: &crate::config::ApiKeys,
) -> Result<(ParsedList, Option<String>)> {
    match input {
        crate::ImportInput::File(path) => {
            let path = path.clone();
            let mut list = tokio::task::spawn_blocking(move || files::parse_file(&path)).await??;
            if list.label == files::TAKEOUT_LABEL {
                list.tracks = keyless::fill_takeout(http, list.tracks).await;
            }
            Ok((list, None))
        }
        crate::ImportInput::Url(url) => {
            let mut url = url.trim().to_string();
            let mut src = parse_url(&url).ok_or_else(|| anyhow!("not a supported playlist link: {url}"))?;
            if matches!(src, RemoteSource::ShortLink(_)) {
                url = links::expand_short_link(http, &url).await?;
                src = parse_url(&url)
                    .filter(|s| !matches!(s, RemoteSource::ShortLink(_)))
                    .ok_or_else(|| anyhow!("the short link leads to something Glyph cannot import: {url}"))?;
            }
            let label = src.label().to_string();
            if let Some(found) = links::fetch(&src, providers, http, keys).await {
                let (name, tracks) = found?;
                return Ok((ParsedList { name, label, tracks }, Some(url)));
            }
            // The server first (its keys), then the app's own keys, then the
            // ways that need no key at all.
            let sc_on_server = matches!(src, RemoteSource::SoundCloud(_));
            let mut first_err = None;
            if let (Some(server), true) = (server, src.server_capable() || sc_on_server) {
                match server.import(&url).await {
                    Ok(r) if !r.tracks.is_empty() => return Ok((ParsedList { name: r.name, label, tracks: r.tracks }, Some(url))),
                    Ok(_) => {}
                    Err(e) => first_err = Some(e),
                }
            }
            if sc_on_server {
                return Err(first_err.unwrap_or_else(|| anyhow!("SoundCloud links need the Glyph server or a SoundCloud client id")));
            }
            match fetch_direct(&src, providers).await {
                Ok(list) if !list.tracks.is_empty() => Ok((list, Some(url))),
                direct => match keyless_list(&src, http).await {
                    Some(Ok((name, tracks))) => Ok((ParsedList { name, label, tracks }, Some(url))),
                    Some(Err(e)) => Err(first_err.unwrap_or(e)),
                    None => match direct {
                        Ok(list) => Ok((list, Some(url))),
                        Err(e) => Err(first_err.unwrap_or(e)),
                    },
                },
            }
        }
    }
}

/// Playlists read without any key (Spotify embed page, YouTube web page).
async fn keyless_list(src: &RemoteSource, http: &reqwest::Client) -> Option<Result<(String, Vec<Track>)>> {
    Some(match src {
        RemoteSource::SpotifyPlaylist(id) => keyless::spotify_embed(http, "playlist", id).await,
        RemoteSource::YouTubePlaylist(id) => keyless::youtube_playlist(http, id).await,
        _ => return None,
    })
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
        other => return Err(anyhow!("{} links are read elsewhere", other.label())),
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
        ordered: Vec::with_capacity(total),
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
        result.ordered.push(m.to_track());
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

    #[test]
    fn recognises_more_links() {
        use RemoteSource::*;
        assert_eq!(parse_url("https://open.spotify.com/album/4m2880jivSbbyEGAKfITCa?si=1"), Some(SpotifyAlbum("4m2880jivSbbyEGAKfITCa".into())));
        assert_eq!(parse_url("https://open.spotify.com/intl-de/artist/4tZwfgrHOc3mvqYlEYSvVi"), Some(SpotifyArtist("4tZwfgrHOc3mvqYlEYSvVi".into())));
        assert_eq!(parse_url("https://open.spotify.com/track/2Foc5Q5nqNiosCNqttzHof"), Some(SpotifyTrack("2Foc5Q5nqNiosCNqttzHof".into())));
        assert_eq!(parse_url("spotify:album:abc"), Some(SpotifyAlbum("abc".into())));
        assert_eq!(parse_url("https://www.deezer.com/de/album/6575789"), Some(DeezerAlbum("6575789".into())));
        assert_eq!(parse_url("https://www.deezer.com/artist/27"), Some(DeezerArtist("27".into())));
        assert_eq!(parse_url("https://www.deezer.com/track/3135556"), Some(DeezerTrack("3135556".into())));
        assert_eq!(
            parse_url("https://soundcloud.com/odesza/sets/a-moment-apart?si=x"),
            Some(SoundCloud("https://soundcloud.com/odesza/sets/a-moment-apart".into()))
        );
        assert_eq!(parse_url("https://m.soundcloud.com/odesza/likes"), Some(SoundCloud("https://soundcloud.com/odesza/likes".into())));
        assert_eq!(parse_url("https://soundcloud.com/discover"), None);
        assert_eq!(parse_url("https://on.soundcloud.com/AbC12"), Some(ShortLink("https://on.soundcloud.com/AbC12".into())));
        assert_eq!(parse_url("https://deezer.page.link/xyz"), Some(ShortLink("https://deezer.page.link/xyz".into())));
        assert_eq!(parse_url("https://bit.ly/xyz"), None);
        assert!(matches!(parse_url("https://music.apple.com/us/playlist/todays-hits/pl.f4d106fed2bd41149aaacabb233eb5eb"), Some(AppleMusic(_))));
        assert!(matches!(parse_url("https://music.apple.com/us/album/random-access-memories/617154241"), Some(AppleMusic(_))));
        assert_eq!(parse_url("https://music.apple.com/us/artist/daft-punk/5468295"), None);
        assert_eq!(
            parse_url("https://c418.bandcamp.com/album/minecraft-volume-alpha?from=x"),
            Some(Bandcamp("https://c418.bandcamp.com/album/minecraft-volume-alpha".into()))
        );
        assert_eq!(parse_url("https://c418.bandcamp.com/"), None);
        assert_eq!(parse_url("ftp://open.spotify.com/playlist/abc"), None);
        assert!(SpotifyPlaylist("x".into()).server_capable());
        assert!(!SoundCloud("x".into()).server_capable());
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
            ordered: vec![],
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
        let order: Vec<&str> = res.ordered.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(order, ["t0", "t1", "t2", "t3", "t4", "t5"]);
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
