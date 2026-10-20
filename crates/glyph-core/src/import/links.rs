//! Link types read without the server: short links (followed to their real
//! address), SoundCloud, Spotify / Deezer albums, artists and single songs,
//! and public Apple Music / Bandcamp pages (only the song list on the page:
//! titles, artists, lengths; nothing is played from there).

use super::RemoteSource;
use anyhow::{anyhow, Result};
use glyph_protocol::{ProviderId, Track};
use serde_json::Value;

/// Hosts a short link may lead to, per short-link host. Anything else is
/// refused (a short link must not send the app to some other site).
pub fn short_link_targets(host: &str) -> Option<&'static [&'static str]> {
    match host {
        "deezer.page.link" | "link.deezer.com" | "dzr.page.link" => Some(&["deezer.com", "www.deezer.com"]),
        "on.soundcloud.com" => Some(&["soundcloud.com", "m.soundcloud.com", "www.soundcloud.com"]),
        "spotify.link" | "spotify.app.link" => Some(&["open.spotify.com"]),
        _ => None,
    }
}

/// Follows a short link (redirects) and returns where it ends, if that is a
/// host the short link may point to.
pub async fn expand_short_link(http: &reqwest::Client, url: &str) -> Result<String> {
    let start = reqwest::Url::parse(url)?;
    let host = start.host_str().unwrap_or("").to_lowercase();
    let allowed = short_link_targets(&host).ok_or_else(|| anyhow!("not a short link: {url}"))?;
    let resp = http.get(start).send().await?;
    let mut end = resp.url().clone();
    // Some short links answer with a page instead of a redirect; Deezer's
    // and Spotify's carry the real address in the page.
    let end_host = end.host_str().unwrap_or("").to_lowercase();
    if !allowed.contains(&end_host.as_str()) {
        let body = resp.text().await.unwrap_or_default();
        let found = allowed.iter().find_map(|h| find_url(&body, h));
        end = match found.and_then(|u| reqwest::Url::parse(&u).ok()) {
            Some(u) => u,
            None => return Err(anyhow!("this short link does not lead to {}", allowed[0])),
        };
    }
    Ok(end.to_string())
}

/// First `https://<host>/...` address in a page.
fn find_url(body: &str, host: &str) -> Option<String> {
    let needle = format!("https://{host}/");
    let start = body.find(&needle)?;
    let rest = &body[start..];
    let end = rest.find(|c: char| c == '"' || c == '\'' || c == '<' || c.is_whitespace()).unwrap_or(rest.len());
    Some(rest[..end].replace("&amp;", "&"))
}

// ------------------------------------------------------------ ISO 8601

/// `PT3M43S` (Apple) and `P00H01M05S` (Bandcamp) in ms.
pub fn duration_ms(s: &str) -> Option<u64> {
    let rest = s.trim().strip_prefix('P')?;
    let rest = rest.replace('T', "");
    let (mut total, mut num) = (0f64, String::new());
    for c in rest.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let n: f64 = num.parse().ok()?;
        num.clear();
        total += n * match c {
            'D' => 86_400.0,
            'H' => 3600.0,
            'M' => 60.0,
            'S' => 1.0,
            _ => return None,
        };
    }
    (total > 0.0).then(|| (total * 1000.0) as u64)
}

#[allow(clippy::too_many_arguments)]
fn meta_track(source: ProviderId, id: String, title: &str, artist: &str, album: Option<String>, dur: Option<u64>, cover: Option<String>, link: Option<String>) -> Option<Track> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    Some(Track {
        id,
        source,
        title: title.to_string(),
        artist: if artist.trim().is_empty() { "Unknown".into() } else { artist.trim().to_string() },
        album: album.filter(|a| !a.trim().is_empty()),
        duration_ms: dur,
        cover_url: cover,
        isrc: None,
        permalink: link,
        external_ids: Default::default(),
        explicit: false,
    })
}

// ------------------------------------------------------------ JSON-LD

/// Every `<script type="application/ld+json">` block of a page, parsed.
pub fn json_ld_blocks(html: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("application/ld+json") {
        rest = &rest[i..];
        let Some(open_end) = rest.find('>') else { break };
        let body = &rest[open_end + 1..];
        let Some(close) = body.find("</script>") else { break };
        if let Ok(v) = serde_json::from_str::<Value>(body[..close].trim()) {
            match v {
                Value::Array(a) => out.extend(a),
                v => out.push(v),
            }
        }
        rest = &body[close..];
    }
    out
}

fn names(v: &Value) -> String {
    match v {
        Value::Array(a) => a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", "),
        Value::Object(_) => v["name"].as_str().unwrap_or("").to_string(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

fn image(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => a.first().and_then(image),
        Value::Object(_) => v["url"].as_str().map(str::to_string),
        _ => None,
    }
}

/// A `MusicPlaylist` / `MusicAlbum` (schema.org) on a page: name and songs.
/// Songs without their own artist get the album's.
pub fn parse_json_ld(html: &str, source_label: &str) -> Option<(String, Vec<Track>)> {
    let block = json_ld_blocks(html).into_iter().find(|v| matches!(v["@type"].as_str(), Some("MusicPlaylist") | Some("MusicAlbum")))?;
    let is_album = block["@type"] == "MusicAlbum";
    let name = block["name"].as_str().unwrap_or("Imported playlist").to_string();
    let album_artist = names(&block["byArtist"]);
    let cover = image(&block["image"]);
    let raw: Vec<Value> = match &block["track"] {
        Value::Array(a) => a.clone(),
        Value::Object(o) if o.contains_key("itemListElement") => o["itemListElement"]
            .as_array()
            .map(|l| l.iter().map(|e| if e["item"].is_object() { e["item"].clone() } else { e.clone() }).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let tracks = raw
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let artist = Some(names(&t["byArtist"])).filter(|a| !a.is_empty()).unwrap_or_else(|| album_artist.clone());
            let link = t["url"].as_str().or(t["@id"].as_str()).map(str::to_string);
            let album = if is_album { Some(name.clone()) } else { t["inAlbum"]["name"].as_str().map(str::to_string) };
            meta_track(
                ProviderId::MusicBrainz,
                format!("{source_label}:{i}"),
                t["name"].as_str().unwrap_or(""),
                &artist,
                album,
                t["duration"].as_str().and_then(duration_ms),
                if is_album { cover.clone() } else { image(&t["image"]) },
                link,
            )
        })
        .collect::<Vec<_>>();
    (!tracks.is_empty()).then_some((name, tracks))
}

// ------------------------------------------------------------ Apple Music

/// Song rows of an Apple Music page (`serialized-server-data`): these carry
/// the artists, which the page's JSON-LD leaves out for playlists.
pub fn parse_apple_server_data(html: &str) -> Vec<Track> {
    let Some(i) = html.find("id=\"serialized-server-data\"") else { return Vec::new() };
    let rest = &html[i..];
    let Some(open_end) = rest.find('>') else { return Vec::new() };
    let body = &rest[open_end + 1..];
    let Some(close) = body.find("</script>") else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<Value>(&body[..close]) else { return Vec::new() };
    let mut out = Vec::new();
    collect_apple_songs(&v, &mut out);
    out
}

fn collect_apple_songs(v: &Value, out: &mut Vec<Track>) {
    match v {
        Value::Object(o) => {
            if o.get("contentDescriptor").and_then(|c| c["kind"].as_str()) == Some("song") && o.get("title").is_some_and(|t| t.is_string()) {
                let artist = o.get("subtitleLinks").and_then(|l| l.as_array()).map(|l| l.iter().filter_map(|x| x["title"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                let album = o.get("tertiaryLinks").and_then(|l| l[0]["title"].as_str()).map(str::to_string);
                let cover = o.get("artwork").and_then(|a| a["dictionary"]["url"].as_str()).map(|u| u.replace("{w}x{h}bb.{f}", "600x600bb.jpg"));
                let id = o["contentDescriptor"]["identifiers"]["storeAdamID"].as_str().unwrap_or("").to_string();
                let link = o["contentDescriptor"]["url"].as_str().map(str::to_string);
                if let Some(t) = meta_track(ProviderId::MusicBrainz, format!("apple:{id}"), o["title"].as_str().unwrap_or(""), &artist, album, o.get("duration").and_then(|d| d.as_u64()), cover, link) {
                    if !out.iter().any(|x: &Track| x.id == t.id) {
                        out.push(t);
                    }
                }
                return;
            }
            for x in o.values() {
                collect_apple_songs(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_apple_songs(x, out)),
        _ => {}
    }
}

async fn fetch_page(http: &reqwest::Client, url: &str) -> Result<String> {
    let resp = http
        .get(url)
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (compatible; Glyph music app)")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en")
        .send()
        .await?;
    match resp.status().as_u16() {
        200..=299 => Ok(resp.text().await?),
        401 | 403 => Err(anyhow!("this list is private")),
        404 => Err(anyhow!("this list does not exist (any more)")),
        s => Err(anyhow!("HTTP {s}")),
    }
}

/// Name and songs of a public Apple Music playlist or album.
pub async fn apple_music(http: &reqwest::Client, url: &str) -> Result<(String, Vec<Track>)> {
    let html = fetch_page(http, url).await?;
    let rows = parse_apple_server_data(&html);
    let ld = parse_json_ld(&html, "apple");
    match (ld, rows) {
        (Some((name, _)), rows) if !rows.is_empty() => Ok((name, rows)),
        (Some(found), _) => Ok(found),
        (None, rows) if !rows.is_empty() => Ok(("Apple Music".into(), rows)),
        _ => Err(anyhow!("no song list found on this Apple Music page (only public playlists and albums work)")),
    }
}

/// Name and songs of a Bandcamp album or track page.
pub async fn bandcamp(http: &reqwest::Client, url: &str) -> Result<(String, Vec<Track>)> {
    let html = fetch_page(http, url).await?;
    if let Some(found) = parse_json_ld(&html, "bandcamp") {
        return Ok(found);
    }
    // A single track page: a `MusicRecording` block.
    let one = json_ld_blocks(&html).into_iter().find(|v| v["@type"] == "MusicRecording");
    if let Some(v) = one {
        let artist = names(&v["byArtist"]);
        let album = v["inAlbum"]["name"].as_str().map(str::to_string);
        if let Some(t) = meta_track(ProviderId::MusicBrainz, "bandcamp:0".into(), v["name"].as_str().unwrap_or(""), &artist, album, v["duration"].as_str().and_then(duration_ms), image(&v["image"]), Some(url.to_string())) {
            return Ok((t.title.clone(), vec![t]));
        }
    }
    Err(anyhow!("no song list found on this Bandcamp page"))
}

// ------------------------------------------------------------ direct fetch

/// Reads the link types of this module (`None`: not one of them).
pub async fn fetch(
    src: &RemoteSource,
    p: &crate::providers::Providers,
    http: &reqwest::Client,
    keys: &crate::config::ApiKeys,
) -> Option<Result<(String, Vec<Track>)>> {
    use crate::providers::{deezer::Deezer, soundcloud::SoundCloud};
    use std::sync::Arc;
    // Spotify with the app's key when there is one, else (or when the API
    // refuses) the public embed page, which needs none.
    let embed = |kind: &'static str, id: String| async move { super::keyless::spotify_embed(http, kind, &id).await };
    let deezer = || p.deezer.clone().unwrap_or_else(|| Arc::new(Deezer::new(http.clone())));
    let soundcloud = || {
        keys.soundcloud_client_id
            .clone()
            .filter(|k| !k.trim().is_empty())
            .map(|k| SoundCloud::new(http.clone(), k))
    };
    let with_artist = |name: String, artist: String| if artist.is_empty() { name } else { format!("{name} · {artist}") };
    let r = match src {
        RemoteSource::SpotifyAlbum(id) => match &p.spotify {
            Some(s) => match s.album(id).await {
                Ok((n, a, t)) => Ok((with_artist(n, a), t)),
                Err(_) => embed("album", id.clone()).await,
            },
            None => embed("album", id.clone()).await,
        },
        RemoteSource::SpotifyArtist(id) => match &p.spotify {
            Some(s) => match s.artist_top(id).await {
                Ok(r) => Ok(r),
                Err(_) => embed("artist", id.clone()).await,
            },
            None => embed("artist", id.clone()).await,
        },
        RemoteSource::SpotifyTrack(id) => {
            use crate::provider::MusicProvider;
            match &p.spotify {
                Some(s) => match s.track(id).await {
                    Ok(t) => Ok((t.title.clone(), vec![t])),
                    Err(_) => embed("track", id.clone()).await,
                },
                None => embed("track", id.clone()).await,
            }
        }
        RemoteSource::DeezerAlbum(id) => deezer().album(id).await.map(|(n, a, t)| (with_artist(n, a), t)),
        RemoteSource::DeezerArtist(id) => deezer().artist_top(id).await,
        RemoteSource::DeezerTrack(id) => deezer().track_by_id(id).await.map(|t| (t.title.clone(), vec![t])),
        // Without an own SoundCloud key the server reads it (`fetch_list`).
        RemoteSource::SoundCloud(url) => match soundcloud() {
            Some(sc) => sc.list_from_link(url).await.map_err(private_hint),
            None => return None,
        },
        RemoteSource::AppleMusic(url) => apple_music(http, url).await,
        RemoteSource::Bandcamp(url) => bandcamp(http, url).await,
        _ => return None,
    };
    Some(r)
}

/// 401/403 from a source means a private list, not an exhausted quota.
fn private_hint(e: anyhow::Error) -> anyhow::Error {
    match e.downcast_ref::<crate::provider::ProviderError>() {
        Some(crate::provider::ProviderError::Quota) => anyhow!("this list is private or the source refused it (HTTP 403)"),
        Some(crate::provider::ProviderError::NotFound) => anyhow!("this list does not exist (any more)"),
        _ => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration_ms("PT3M43S"), Some(223_000));
        assert_eq!(duration_ms("P00H01M05S"), Some(65_000));
        assert_eq!(duration_ms("PT1H2M"), Some(3_720_000));
        assert_eq!(duration_ms("3:43"), None);
    }

    #[test]
    fn bandcamp_album_json_ld() {
        let html = r#"<html><script type="application/ld+json">{"@type":"MusicAlbum","name":"Volume Alpha",
          "byArtist":{"@type":"MusicGroup","name":"C418"},"image":"https://f4.bcbits.com/img/a_10.jpg",
          "track":{"@type":"ItemList","numberOfItems":2,"itemListElement":[
            {"@type":"ListItem","position":1,"item":{"@type":"MusicRecording","@id":"https://c418.bandcamp.com/track/key","name":"Key","duration":"P00H01M05S"}},
            {"@type":"ListItem","position":2,"item":{"@type":"MusicRecording","name":"Door","duration":"P00H01M51S"}}]}}</script></html>"#;
        let (name, t) = parse_json_ld(html, "bandcamp").unwrap();
        assert_eq!(name, "Volume Alpha");
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].title.as_str(), t[0].artist.as_str()), ("Key", "C418"));
        assert_eq!(t[0].duration_ms, Some(65_000));
        assert_eq!(t[0].album.as_deref(), Some("Volume Alpha"));
        assert_eq!(t[0].permalink.as_deref(), Some("https://c418.bandcamp.com/track/key"));
        assert_ne!(t[0].id, t[1].id);
    }

    #[test]
    fn apple_playlist_rows_carry_artists() {
        let html = r#"<script id=schema:music-playlist type="application/ld+json">{"@type":"MusicPlaylist","name":"Today's Hits",
          "track":[{"@type":"MusicRecording","name":"Housewife","duration":"PT3M28S"}]}</script>
          <script type="application/json" id="serialized-server-data">{"data":[{"items":[{"id":"x","title":"Housewife",
          "duration":208810,"contentDescriptor":{"kind":"song","identifiers":{"storeAdamID":"6822497774"},"url":"https://music.apple.com/us/album/housewife/6822497770?i=6822497774"},
          "subtitleLinks":[{"title":"Tate McRae"}],"tertiaryLinks":[{"title":"Housewife - Single"}],
          "artwork":{"dictionary":{"url":"https://is1-ssl.mzstatic.com/x/{w}x{h}bb.{f}"}}}]}]}</script>"#;
        let rows = parse_apple_server_data(html);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].title.as_str(), rows[0].artist.as_str()), ("Housewife", "Tate McRae"));
        assert_eq!(rows[0].duration_ms, Some(208_810));
        assert_eq!(rows[0].cover_url.as_deref(), Some("https://is1-ssl.mzstatic.com/x/600x600bb.jpg"));
        let (name, ld) = parse_json_ld(html, "apple").unwrap();
        assert_eq!(name, "Today's Hits");
        assert_eq!(ld[0].artist, "Unknown");
    }

    #[test]
    fn short_links_only_lead_to_their_service() {
        assert!(short_link_targets("on.soundcloud.com").unwrap().contains(&"soundcloud.com"));
        assert!(short_link_targets("deezer.page.link").unwrap().contains(&"www.deezer.com"));
        assert!(short_link_targets("bit.ly").is_none());
        assert_eq!(
            find_url(r#"<a href="https://www.deezer.com/en/playlist/123?utm=x&amp;a=b">"#, "www.deezer.com").as_deref(),
            Some("https://www.deezer.com/en/playlist/123?utm=x&a=b")
        );
    }
}
