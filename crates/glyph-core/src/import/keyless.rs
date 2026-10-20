//! Reading lists without any key or login, so nobody has to register an
//! app anywhere:
//! - Spotify: the public embed page (`open.spotify.com/embed/...`) of a
//!   playlist, album, artist or song. Spotify caps it at 100 songs; longer
//!   lists (and Liked Songs) come over by copying the songs in the Spotify
//!   app (Ctrl+A, Ctrl+C gives one song link per line) and pasting them.
//! - YouTube: the playlist page through YouTube's own web endpoint (like the
//!   native search), single videos through oEmbed.
//! - Pasted text with many links ([`parse_pasted`]) and Google Takeout CSVs
//!   (video ids only, filled in through oEmbed).
//! Only titles, artists and lengths are read; nothing is played from there.

use anyhow::{anyhow, bail, Result};
use futures::stream::StreamExt;
use glyph_protocol::{ProviderId, Track};
use serde_json::Value;

/// Songs the Spotify embed page shows at most.
pub const SPOTIFY_EMBED_LIMIT: usize = 100;

// ------------------------------------------------------------ Spotify

/// The `entity` of an embed page's `__NEXT_DATA__`.
pub fn spotify_embed_entity(html: &str) -> Option<Value> {
    let i = html.find("id=\"__NEXT_DATA__\"")?;
    let rest = &html[i..];
    let body = &rest[rest.find('>')? + 1..];
    let end = body.find("</script>")?;
    let v: Value = serde_json::from_str(&body[..end]).ok()?;
    let e = v["props"]["pageProps"]["state"]["data"]["entity"].clone();
    e.is_object().then_some(e)
}

fn spotify_id(uri: &str) -> Option<&str> {
    uri.rsplit(':').next().filter(|id| !id.is_empty())
}

fn embed_cover(e: &Value) -> Option<String> {
    e["coverArt"]["sources"]
        .as_array()?
        .iter()
        .min_by_key(|s| s["width"].as_u64().unwrap_or(640).abs_diff(640))
        .and_then(|s| s["url"].as_str())
        .map(str::to_string)
}

fn spotify_track(id: &str, title: &str, artist: &str, album: Option<String>, dur: Option<u64>, cover: Option<String>, explicit: bool) -> Track {
    Track {
        id: id.to_string(),
        source: ProviderId::Spotify,
        title: title.trim().to_string(),
        artist: if artist.trim().is_empty() { "Unknown".into() } else { artist.trim().to_string() },
        album,
        duration_ms: dur.filter(|d| *d > 0),
        cover_url: cover,
        isrc: None,
        permalink: Some(format!("https://open.spotify.com/track/{id}")),
        external_ids: Default::default(),
        explicit,
    }
}

/// Name and songs of an embed entity (playlist, album, artist top songs or
/// one song). Artists in list rows are joined with ",\u{a0}".
pub fn parse_spotify_entity(e: &Value) -> Option<(String, Vec<Track>)> {
    let kind = e["type"].as_str().unwrap_or("");
    let name = e["name"].as_str().or(e["title"].as_str()).unwrap_or("Spotify").to_string();
    if kind == "track" {
        let id = spotify_id(e["uri"].as_str()?)?;
        let artist = e["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        let t = spotify_track(id, &name, &artist, None, e["duration"].as_u64(), embed_cover(e), e["isExplicit"].as_bool() == Some(true));
        return Some((name, vec![t]));
    }
    let album = (kind == "album").then(|| name.clone());
    let cover = (kind == "album").then(|| embed_cover(e)).flatten();
    let tracks: Vec<Track> = e["trackList"]
        .as_array()?
        .iter()
        .filter(|t| t["entityType"].as_str().is_none_or(|k| k == "track"))
        .filter_map(|t| {
            let id = spotify_id(t["uri"].as_str()?)?;
            let artist = t["subtitle"].as_str().unwrap_or("").replace(",\u{a0}", ", ");
            Some(spotify_track(id, t["title"].as_str()?, &artist, album.clone(), t["duration"].as_u64(), cover.clone(), t["isExplicit"].as_bool() == Some(true)))
        })
        .collect();
    let name = match kind {
        "album" => match e["subtitle"].as_str().filter(|s| !s.is_empty()) {
            Some(by) => format!("{name} · {}", by.replace(",\u{a0}", ", ")),
            None => name,
        },
        _ => name,
    };
    Some((name, tracks))
}

/// Reads `open.spotify.com/embed/<kind>/<id>`.
pub async fn spotify_embed(http: &reqwest::Client, kind: &str, id: &str) -> Result<(String, Vec<Track>)> {
    let resp = http
        .get(format!("https://open.spotify.com/embed/{kind}/{id}"))
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (compatible; Glyph music app)")
        .send()
        .await?;
    match resp.status().as_u16() {
        200..=299 => {}
        404 => bail!("this Spotify list does not exist or is private"),
        s => bail!("Spotify answered HTTP {s}"),
    }
    let html = resp.text().await?;
    let e = spotify_embed_entity(&html).ok_or_else(|| anyhow!("Spotify page without a song list"))?;
    parse_spotify_entity(&e).ok_or_else(|| anyhow!("Spotify page without a song list"))
}

// ------------------------------------------------------------ YouTube

const YT_BROWSE: &str = "https://www.youtube.com/youtubei/v1/browse?prettyPrint=false";
const YT_CLIENT_VERSION: &str = "2.20260930.01.00";

/// Video rows of a playlist page (new `lockupViewModel` and old
/// `playlistVideoRenderer`), plus the continuation token for the next page.
pub fn parse_youtube_playlist(v: &Value) -> (Vec<Track>, Option<String>) {
    fn walk(v: &Value, out: &mut Vec<Track>, cont: &mut Option<String>) {
        match v {
            Value::Object(o) => {
                if let Some(l) = o.get("lockupViewModel") {
                    if let Some(t) = from_lockup(l) {
                        out.push(t);
                    }
                    return;
                }
                if let Some(r) = o.get("playlistVideoRenderer") {
                    if let Some(t) = from_renderer(r) {
                        out.push(t);
                    }
                    return;
                }
                if let Some(tok) = o.get("continuationCommand").and_then(|c| c["token"].as_str()).filter(|t| !t.is_empty()) {
                    cont.get_or_insert_with(|| tok.to_string());
                }
                o.values().for_each(|x| walk(x, out, cont));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out, cont)),
            _ => {}
        }
    }
    let (mut out, mut cont) = (Vec::new(), None);
    walk(v, &mut out, &mut cont);
    (out, cont)
}

fn clock_ms(s: &str) -> Option<u64> {
    let mut total = 0u64;
    for part in s.trim().split(':') {
        total = total * 60 + part.trim().parse::<u64>().ok()?;
    }
    (total > 0).then_some(total * 1000)
}

/// "Shakira and 2 more" -> "Shakira".
fn first_artist(byline: &str) -> String {
    match byline.rfind(" and ") {
        Some(i) if byline[i..].ends_with(" more") => byline[..i].to_string(),
        _ => byline.to_string(),
    }
}

fn from_lockup(l: &Value) -> Option<Track> {
    if l["contentType"].as_str() != Some("LOCKUP_CONTENT_TYPE_VIDEO") {
        return None;
    }
    let id = l["contentId"].as_str()?;
    let md = &l["metadata"]["lockupMetadataViewModel"];
    let title = md["title"]["content"].as_str()?;
    let byline = md["metadata"]["contentMetadataViewModel"]["metadataRows"][0]["metadataParts"][0]["text"]["content"].as_str().unwrap_or("");
    // The length is a thumbnail badge ("4:01").
    let mut dur = None;
    find_clock(&l["contentImage"], &mut dur);
    Some(crate::providers::youtube::video_track(id, title, &first_artist(byline), None, dur).0)
}

fn find_clock(v: &Value, out: &mut Option<u64>) {
    if out.is_some() {
        return;
    }
    match v {
        Value::Object(o) => {
            if let Some(t) = o.get("text").and_then(|t| t.as_str()).filter(|t| t.contains(':')) {
                if let Some(ms) = clock_ms(t) {
                    *out = Some(ms);
                    return;
                }
            }
            o.values().for_each(|x| find_clock(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| find_clock(x, out)),
        _ => {}
    }
}

fn runs_text(v: &Value) -> Option<String> {
    if let Some(s) = v["simpleText"].as_str() {
        return Some(s.to_string());
    }
    let s: String = v["runs"].as_array()?.iter().filter_map(|r| r["text"].as_str()).collect();
    (!s.is_empty()).then_some(s)
}

fn from_renderer(r: &Value) -> Option<Track> {
    let id = r["videoId"].as_str()?;
    let title = runs_text(&r["title"])?;
    let channel = runs_text(&r["shortBylineText"]).unwrap_or_default();
    let dur = r["lengthSeconds"].as_str().and_then(|s| s.parse::<u64>().ok()).map(|s| s * 1000);
    Some(crate::providers::youtube::video_track(id, &title, &channel, None, dur).0)
}

fn youtube_title(v: &Value) -> Option<String> {
    v["metadata"]["playlistMetadataRenderer"]["title"]
        .as_str()
        .or(v["header"]["pageHeaderRenderer"]["pageTitle"].as_str())
        .map(str::to_string)
}

/// A public (or unlisted) YouTube / YouTube Music playlist, all pages.
pub async fn youtube_playlist(http: &reqwest::Client, list_id: &str) -> Result<(String, Vec<Track>)> {
    let context = serde_json::json!({"client": {"clientName": "WEB", "clientVersion": YT_CLIENT_VERSION, "hl": "en", "gl": "US"}});
    let mut body = serde_json::json!({"context": context, "browseId": format!("VL{list_id}")});
    let (mut name, mut tracks) = (None, Vec::new());
    for _ in 0..60 {
        let v: Value = http
            .post(YT_BROWSE)
            .header("X-Youtube-Client-Name", "1")
            .header("X-Youtube-Client-Version", YT_CLIENT_VERSION)
            .header("Origin", "https://www.youtube.com")
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if name.is_none() {
            name = youtube_title(&v);
        }
        let (page, cont) = parse_youtube_playlist(&v);
        let before = tracks.len();
        for t in page {
            if !tracks.iter().any(|x: &Track| x.id == t.id) {
                tracks.push(t);
            }
        }
        match cont {
            Some(token) if tracks.len() > before => body = serde_json::json!({"context": context, "continuation": token}),
            _ => break,
        }
    }
    if tracks.is_empty() {
        bail!("this YouTube playlist is empty or private (make it public or unlisted)");
    }
    Ok((name.unwrap_or_else(|| "YouTube playlist".into()), tracks))
}

/// Title and channel of one video (oEmbed, no key).
pub async fn youtube_video(http: &reqwest::Client, id: &str) -> Result<Track> {
    let url = format!("https://www.youtube.com/watch?v={id}");
    let v: Value = http.get("https://www.youtube.com/oembed").query(&[("url", url.as_str()), ("format", "json")]).send().await?.error_for_status()?.json().await?;
    let title = v["title"].as_str().ok_or_else(|| anyhow!("video not found"))?;
    let channel = v["author_name"].as_str().unwrap_or("");
    Ok(crate::providers::youtube::video_track(id, title, channel, None, None).0)
}

// ------------------------------------------------------------ pasted text

/// One song link of a pasted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SongLink {
    Spotify(String),
    YouTube(String),
    Deezer(String),
}

/// What a pasted text holds: single songs (in order) and whole lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pasted {
    pub songs: Vec<SongLink>,
    /// (link, what it is) of playlists, albums, profiles.
    pub lists: Vec<(String, super::RemoteSource)>,
}

impl Pasted {
    pub fn is_empty(&self) -> bool {
        self.songs.is_empty() && self.lists.is_empty()
    }
}

fn youtube_video_id(url: &reqwest::Url) -> Option<String> {
    let host = url.host_str()?.trim_start_matches("www.").to_lowercase();
    let id = match host.as_str() {
        "youtu.be" => url.path_segments()?.next().map(str::to_string),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" => url.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.to_string()),
        _ => None,
    }?;
    crate::ytdlp::valid_video_id(&id).then_some(id)
}

/// Every link in a pasted text (one per line, or separated by spaces /
/// commas), deduplicated, in order. `spotify:track:` URIs work too.
pub fn parse_pasted(text: &str) -> Pasted {
    let mut out = Pasted::default();
    for word in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let word = word.trim().trim_matches(|c| c == '"' || c == '\'' || c == '<' || c == '>');
        if word.is_empty() {
            continue;
        }
        let song = if let Some(id) = word.strip_prefix("spotify:track:") {
            Some(SongLink::Spotify(id.to_string()))
        } else if let Ok(url) = reqwest::Url::parse(word) {
            let host = url.host_str().unwrap_or("").trim_start_matches("www.").to_lowercase();
            let segs: Vec<&str> = url.path_segments().map(|s| s.filter(|x| !x.is_empty()).collect()).unwrap_or_default();
            let after = |name: &str| segs.iter().position(|s| *s == name).and_then(|i| segs.get(i + 1)).map(|s| s.to_string());
            match host.as_str() {
                "open.spotify.com" => after("track").map(SongLink::Spotify),
                "deezer.com" => after("track").filter(|id| id.chars().all(|c| c.is_ascii_digit())).map(SongLink::Deezer),
                _ => youtube_video_id(&url).filter(|_| !url.query_pairs().any(|(k, _)| k == "list")).map(SongLink::YouTube),
            }
        } else {
            None
        };
        match song {
            Some(s) => {
                if !out.songs.contains(&s) {
                    out.songs.push(s);
                }
            }
            None => {
                if let Some(src) = super::parse_url(word) {
                    if !out.lists.iter().any(|(_, s)| *s == src) {
                        out.lists.push((word.to_string(), src));
                    }
                }
            }
        }
    }
    out
}

/// Metadata of pasted song links, in order (songs that cannot be read are
/// left out). Spotify songs use the app's Spotify key when there is one,
/// else their embed pages; 4 requests at a time.
pub async fn read_songs(
    songs: &[SongLink],
    spotify: Option<&std::sync::Arc<crate::providers::spotify::Spotify>>,
    deezer: &crate::providers::deezer::Deezer,
    http: &reqwest::Client,
) -> Vec<Track> {
    // Spotify with a key: 50 per request.
    let mut known: std::collections::HashMap<String, Track> = Default::default();
    if let Some(sp) = spotify {
        let ids: Vec<&str> = songs.iter().filter_map(|s| if let SongLink::Spotify(id) = s { Some(id.as_str()) } else { None }).collect();
        for chunk in ids.chunks(50) {
            match sp.tracks_by_ids(chunk).await {
                Ok(list) => known.extend(list.into_iter().map(|t| (t.id.clone(), t))),
                Err(e) => {
                    tracing::info!("Spotify batch lookup failed, using embed pages: {e:#}");
                    break;
                }
            }
        }
    }
    let known = &known;
    futures::stream::iter(songs.iter().cloned())
        .map(|s| async move {
            match s {
                SongLink::Spotify(id) => match known.get(&id) {
                    Some(t) => Some(t.clone()),
                    None => spotify_embed(http, "track", &id).await.ok().and_then(|(_, t)| t.into_iter().next()),
                },
                SongLink::YouTube(id) => youtube_video(http, &id).await.ok(),
                SongLink::Deezer(id) => deezer.track_by_id(&id).await.ok(),
            }
        })
        .buffered(4)
        .filter_map(futures::future::ready)
        .collect()
        .await
}

/// Fills in YouTube songs of a Google Takeout CSV (ids only) through oEmbed.
pub async fn fill_takeout(http: &reqwest::Client, tracks: Vec<Track>) -> Vec<Track> {
    let songs: Vec<SongLink> = tracks.iter().map(|t| SongLink::YouTube(t.id.clone())).collect();
    let dz = crate::providers::deezer::Deezer::new(http.clone());
    read_songs(&songs, None, &dz, http).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_embed_playlist() {
        let html = r#"<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"state":{"data":{"entity":{
            "type":"playlist","name":"Today's Top Hits","trackList":[
              {"uri":"spotify:track:3XmwAZpIMNpTJy8faw3ajD","title":"expectations (feat. Chappell Roan)","subtitle":"Olivia Rodrigo, Chappell Roan","duration":223935,"isExplicit":false,"entityType":"track"},
              {"uri":"spotify:episode:x","title":"Podcast","subtitle":"Someone","entityType":"episode"}]}}}}}}</script>"#;
        let e = spotify_embed_entity(html).unwrap();
        let (name, t) = parse_spotify_entity(&e).unwrap();
        assert_eq!(name, "Today's Top Hits");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "3XmwAZpIMNpTJy8faw3ajD");
        assert_eq!(t[0].artist, "Olivia Rodrigo, Chappell Roan");
        assert_eq!(t[0].duration_ms, Some(223_935));
        assert_eq!(t[0].source, ProviderId::Spotify);
    }

    #[test]
    fn spotify_embed_album_and_track() {
        let album: Value = serde_json::from_str(
            r#"{"type":"album","name":"RAM","subtitle":"Daft Punk","coverArt":{"sources":[{"url":"https://i/640","width":640}]},
                "trackList":[{"uri":"spotify:track:a1","title":"Give Life Back to Music","subtitle":"Daft Punk","duration":274000}]}"#,
        )
        .unwrap();
        let (name, t) = parse_spotify_entity(&album).unwrap();
        assert_eq!(name, "RAM · Daft Punk");
        assert_eq!((t[0].album.as_deref(), t[0].cover_url.as_deref()), (Some("RAM"), Some("https://i/640")));
        let track: Value = serde_json::from_str(
            r#"{"type":"track","name":"Get Lucky","uri":"spotify:track:2Foc","duration":247632,"artists":[{"name":"Daft Punk"},{"name":"Pharrell Williams"}]}"#,
        )
        .unwrap();
        let (_, t) = parse_spotify_entity(&track).unwrap();
        assert_eq!((t[0].id.as_str(), t[0].artist.as_str()), ("2Foc", "Daft Punk, Pharrell Williams"));
    }

    #[test]
    fn youtube_playlist_page() {
        let v: Value = serde_json::from_str(
            r#"{"header":{"pageHeaderRenderer":{"pageTitle":"Top 100"}},"contents":[
              {"lockupViewModel":{"contentId":"fcnDmrtj6Sk","contentType":"LOCKUP_CONTENT_TYPE_VIDEO",
                "contentImage":{"thumbnailViewModel":{"overlays":[{"thumbnailBottomOverlayViewModel":{"badges":[{"thumbnailBadgeViewModel":{"text":"4:01"}}]}}]}},
                "metadata":{"lockupMetadataViewModel":{"title":{"content":"Shakira, Burna Boy - Dai Dai (Official Video)"},
                  "metadata":{"contentMetadataViewModel":{"metadataRows":[{"metadataParts":[{"text":{"content":"Shakira and 2 more"}}]}]}}}}}},
              {"lockupViewModel":{"contentId":"PLx","contentType":"LOCKUP_CONTENT_TYPE_PLAYLIST"}},
              {"playlistVideoRenderer":{"videoId":"abcdefghijk","title":{"runs":[{"text":"Old Row"}]},"shortBylineText":{"runs":[{"text":"Band - Topic"}]},"lengthSeconds":"200"}},
              {"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"NEXT"}}}}]}"#,
        )
        .unwrap();
        let (t, cont) = parse_youtube_playlist(&v);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].id, "fcnDmrtj6Sk");
        assert_eq!(t[0].duration_ms, Some(241_000));
        assert_eq!(t[1].duration_ms, Some(200_000));
        assert_eq!(cont.as_deref(), Some("NEXT"));
        assert_eq!(youtube_title(&v).as_deref(), Some("Top 100"));
        assert_eq!(first_artist("Shakira and 2 more"), "Shakira");
        assert_eq!(first_artist("Simon and Garfunkel"), "Simon and Garfunkel");
    }

    #[test]
    fn pasted_text() {
        let text = "https://open.spotify.com/track/3XmwAZpIMNpTJy8faw3ajD\r\nhttps://open.spotify.com/track/2Foc5Q5nqNiosCNqttzHof?si=1\n\
                    spotify:track:3XmwAZpIMNpTJy8faw3ajD https://youtu.be/dQw4w9WgXcQ, https://www.deezer.com/track/3135556\n\
                    https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M  hello https://www.youtube.com/playlist?list=PLabc";
        let p = parse_pasted(text);
        assert_eq!(
            p.songs,
            [
                SongLink::Spotify("3XmwAZpIMNpTJy8faw3ajD".into()),
                SongLink::Spotify("2Foc5Q5nqNiosCNqttzHof".into()),
                SongLink::YouTube("dQw4w9WgXcQ".into()),
                SongLink::Deezer("3135556".into()),
            ]
        );
        assert_eq!(p.lists.len(), 2);
        assert!(parse_pasted("just words").is_empty());
    }
}
