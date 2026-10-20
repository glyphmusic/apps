//! Playlist file parsers: CSV (Exportify & co.), Spotify JSON data export,
//! M3U/M3U8 and XSPF. All produce metadata-only tracks.
//!
//! Tracks parsed from Spotify exports carry `ProviderId::Spotify`; everything
//! else uses `ProviderId::MusicBrainz` as the neutral "metadata only" source.

use glyph_protocol::{ProviderId, Track};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedList {
    pub name: String,
    /// e.g. "spotify/playlist", "file/csv"
    pub label: String,
    pub tracks: Vec<Track>,
}

fn mk(source: ProviderId, idx: usize, title: &str, artist: &str, album: Option<&str>, dur: Option<u64>) -> Option<Track> {
    let (title, artist) = (title.trim(), artist.trim());
    if title.is_empty() {
        return None;
    }
    Some(Track {
        id: format!("file:{idx}"),
        source,
        title: title.to_string(),
        artist: if artist.is_empty() { "Unknown".into() } else { artist.to_string() },
        album: album.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
        duration_ms: dur.filter(|d| *d > 0),
        cover_url: None,
        isrc: None,
        permalink: None,
        external_ids: Default::default(),
        explicit: false,
    })
}

/// Parses a dropped file by extension.
pub fn parse_file(path: &Path) -> anyhow::Result<ParsedList> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported playlist").to_string();
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "csv" | "tsv" => parse_csv(text, &stem),
        "json" => parse_spotify_json(text, &stem),
        "m3u" | "m3u8" => Ok(parse_m3u(text, &stem)),
        "xspf" => Ok(parse_xspf(text, &stem)),
        _ => Err(anyhow::anyhow!("unsupported playlist file: .{ext} (CSV, JSON, M3U, M3U8, XSPF)")),
    }
}

// --------------------------------------------------------------------- CSV

/// RFC-4180-ish CSV parser (quotes, doubled quotes, newlines in fields).
pub fn parse_csv_rows(text: &str, delim: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.is_empty() {
            in_quotes = true;
        } else if c == delim {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            if row.iter().any(|f| !f.is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
        } else {
            field.push(c);
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        if row.iter().any(|f| !f.is_empty()) {
            rows.push(row);
        }
    }
    rows
}

fn find_col(header: &[String], names: &[&str]) -> Option<usize> {
    header.iter().position(|h| {
        let h = h.trim().to_lowercase();
        names.iter().any(|n| h == *n)
    })
}

/// Label of a Google Takeout playlist (video ids that still need titles).
pub const TAKEOUT_LABEL: &str = "youtube/takeout";

pub fn parse_csv(text: &str, name: &str) -> anyhow::Result<ParsedList> {
    let first_line = text.lines().next().unwrap_or("");
    let delim = [',', ';', '\t']
        .into_iter()
        .max_by_key(|d| first_line.matches(*d).count())
        .unwrap_or(',');
    let rows = parse_csv_rows(text, delim);
    let header = rows.first().ok_or_else(|| anyhow::anyhow!("empty CSV"))?;
    let title = find_col(header, &["track name", "name", "title", "track", "song", "song name"]);
    let artist = find_col(header, &["artist name(s)", "artist name", "artist names", "artists", "artist"]);
    let album = find_col(header, &["album name", "album"]);
    let dur = find_col(header, &["duration (ms)", "duration_ms", "duration ms", "track duration (ms)"]);
    let dur_s = find_col(header, &["duration", "duration (s)", "length"]);
    let isrc = find_col(header, &["isrc"]);
    let uri = find_col(header, &["track uri", "spotify uri", "uri", "spotify id"]);
    // Google Takeout (YouTube / YouTube Music playlists): only video ids;
    // `fetch_list` fills in titles.
    if let (None, Some(vid)) = (title, find_col(header, &["video id", "video-id", "videoid"])) {
        let tracks = rows
            .iter()
            .skip(1)
            .filter_map(|row| row.get(vid).map(|s| s.trim().to_string()))
            .filter(|id| crate::ytdlp::valid_video_id(id))
            .map(|id| Track {
                permalink: Some(format!("https://www.youtube.com/watch?v={id}")),
                id,
                source: ProviderId::YouTube,
                title: String::new(),
                artist: String::new(),
                album: None,
                duration_ms: None,
                cover_url: None,
                isrc: None,
                external_ids: Default::default(),
                explicit: false,
            })
            .collect();
        return Ok(ParsedList { name: name.to_string(), label: TAKEOUT_LABEL.into(), tracks });
    }
    let title = title.ok_or_else(|| anyhow::anyhow!("CSV has no track name column"))?;

    let get = |row: &Vec<String>, c: Option<usize>| c.and_then(|i| row.get(i)).map(|s| s.trim().to_string());
    let mut tracks = Vec::new();
    let mut spotify_ids = false;
    for (n, row) in rows.iter().enumerate().skip(1) {
        let d = get(row, dur)
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| get(row, dur_s).and_then(|s| s.parse::<f64>().ok()).map(|s| (s * 1000.0) as u64));
        let a = get(row, artist).unwrap_or_default().replace(";", ", ");
        let Some(mut t) = mk(
            ProviderId::MusicBrainz,
            n,
            &get(row, Some(title)).unwrap_or_default(),
            &a,
            get(row, album).as_deref(),
            d,
        ) else {
            continue;
        };
        t.isrc = get(row, isrc).filter(|s| !s.is_empty());
        if let Some(u) = get(row, uri) {
            if let Some(id) = u.strip_prefix("spotify:track:") {
                t.source = ProviderId::Spotify;
                t.id = id.to_string();
                t.permalink = Some(format!("https://open.spotify.com/track/{id}"));
                spotify_ids = true;
            }
        }
        tracks.push(t);
    }
    Ok(ParsedList {
        name: name.to_string(),
        label: if spotify_ids { "spotify/csv".into() } else { "file/csv".into() },
        tracks,
    })
}

// ------------------------------------------------------------ Spotify JSON

fn s(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str)).map(str::to_string)
}

fn spotify_entry(v: &Value, idx: usize) -> Option<Track> {
    // Playlist items: {"track": {...}}; library: {"artist","album","track","uri"};
    // streaming history: {"artistName","trackName","msPlayed"}.
    let inner = v.get("track").filter(|t| t.is_object()).unwrap_or(v);
    let title = s(inner, &["trackName", "track", "name", "master_metadata_track_name"])?;
    let artist = s(inner, &["artistName", "artist", "master_metadata_album_artist_name"]).unwrap_or_default();
    let album = s(inner, &["albumName", "album", "master_metadata_album_album_name"]);
    let mut t = mk(ProviderId::MusicBrainz, idx, &title, &artist, album.as_deref(), None)?;
    if let Some(uri) = s(inner, &["trackUri", "uri", "spotify_track_uri"]) {
        if let Some(id) = uri.strip_prefix("spotify:track:") {
            t.source = ProviderId::Spotify;
            t.id = id.to_string();
            t.permalink = Some(format!("https://open.spotify.com/track/{id}"));
        }
    }
    Some(t)
}

pub fn parse_spotify_json(text: &str, name: &str) -> anyhow::Result<ParsedList> {
    let v: Value = serde_json::from_str(text)?;
    let mut list_name = name.to_string();
    let mut tracks: Vec<Track> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |t: Option<Track>, tracks: &mut Vec<Track>| {
        if let Some(t) = t {
            // Streaming history repeats songs; keep each once.
            if seen.insert(crate::dedupe::track_key(&t)) {
                tracks.push(t);
            }
        }
    };
    if let Some(pls) = v.get("playlists").and_then(Value::as_array) {
        // Playlist1.json: take the first playlist, or all merged if several.
        if pls.len() == 1 {
            if let Some(n) = s(&pls[0], &["name"]) {
                list_name = n;
            }
        }
        for pl in pls {
            for (i, it) in pl.get("items").and_then(Value::as_array).into_iter().flatten().enumerate() {
                push(spotify_entry(it, i), &mut tracks);
            }
        }
    } else if let Some(arr) = v.get("tracks").and_then(Value::as_array).or_else(|| v.as_array()) {
        for (i, it) in arr.iter().enumerate() {
            push(spotify_entry(it, i), &mut tracks);
        }
    } else {
        return Err(anyhow::anyhow!("unrecognised Spotify JSON export"));
    }
    Ok(ParsedList { name: list_name, label: "spotify/export".into(), tracks })
}

// -------------------------------------------------------------------- M3U

pub fn parse_m3u(text: &str, name: &str) -> ParsedList {
    let mut list_name = name.to_string();
    let mut tracks = Vec::new();
    let mut pending: Option<(String, Option<u64>)> = None; // (display, duration)
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("#PLAYLIST:") {
            list_name = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (dur, disp) = rest.split_once(',').unwrap_or(("", rest));
            let secs = dur.split_whitespace().next().and_then(|d| d.parse::<f64>().ok()).filter(|d| *d > 0.0);
            pending = Some((disp.trim().to_string(), secs.map(|d| (d * 1000.0) as u64)));
        } else if line.starts_with('#') {
            continue;
        } else {
            let idx = tracks.len();
            let (disp, dur) = pending.take().unwrap_or_else(|| (name_from_path(line), None));
            let (artist, title) = split_artist_title(&disp);
            if let Some(t) = mk(ProviderId::MusicBrainz, idx, &title, &artist, None, dur) {
                tracks.push(t);
            }
        }
    }
    ParsedList { name: list_name, label: "file/m3u".into(), tracks }
}

fn name_from_path(p: &str) -> String {
    let base = p.rsplit(['/', '\\']).next().unwrap_or(p);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    // "01 - Artist - Title" / "01. Title" track numbers
    let trimmed = stem.trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '_' | ' '));
    if trimmed.is_empty() { stem } else { trimmed }.replace('_', " ")
}

/// "Artist - Title" -> (artist, title); no separator -> ("", whole).
pub fn split_artist_title(s: &str) -> (String, String) {
    match s.split_once(" - ") {
        Some((a, t)) => (a.trim().to_string(), t.trim().to_string()),
        None => (String::new(), s.trim().to_string()),
    }
}

// -------------------------------------------------------------------- XSPF

fn unescape_xml(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("<![CDATA[").and_then(|x| x.strip_suffix("]]>")).unwrap_or(s);
    crate::provider::unescape_html(s)
}

/// Text of the first `<tag>..</tag>` inside `block`.
fn tag_text(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&format!("</{tag}>"))? + start;
    Some(unescape_xml(&block[start..end]))
}

pub fn parse_xspf(text: &str, name: &str) -> ParsedList {
    let head_end = text.find("<trackList").unwrap_or(text.len());
    let list_name = tag_text(&text[..head_end], "title").filter(|t| !t.is_empty()).unwrap_or_else(|| name.to_string());
    let mut tracks = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("<track>") {
        let after = &rest[i + 7..];
        let Some(end) = after.find("</track>") else { break };
        let block = &after[..end];
        let dur = tag_text(block, "duration").and_then(|d| d.trim().parse::<u64>().ok());
        let creator = tag_text(block, "creator").unwrap_or_default();
        let title = tag_text(block, "title").unwrap_or_default();
        let album = tag_text(block, "album");
        if let Some(t) = mk(ProviderId::MusicBrainz, tracks.len(), &title, &creator, album.as_deref(), dur) {
            tracks.push(t);
        }
        rest = &after[end + 8..];
    }
    ParsedList { name: list_name, label: "file/xspf".into(), tracks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_google_takeout() {
        let csv = "Video ID,Playlist Video Creation Timestamp\ndQw4w9WgXcQ,2024-01-01T00:00:00+00:00\nbad,2024\n";
        let l = parse_csv(csv, "Liked music").unwrap();
        assert_eq!(l.label, TAKEOUT_LABEL);
        assert_eq!(l.tracks.len(), 1);
        assert_eq!((l.tracks[0].id.as_str(), l.tracks[0].source), ("dQw4w9WgXcQ", ProviderId::YouTube));
    }

    #[test]
    fn csv_exportify() {
        let csv = "\u{feff}Track URI,Track Name,Album Name,Artist Name(s),Duration (ms),ISRC\n\
            spotify:track:abc,\"Hello, World\",\"The \"\"LP\"\"\",A;B,201000,USX1\n\
            spotify:track:def,Second,Alb,C,100,\n";
        let l = parse_csv(csv.trim_start_matches('\u{feff}'), "pl").unwrap();
        assert_eq!(l.label, "spotify/csv");
        assert_eq!(l.tracks.len(), 2);
        let t = &l.tracks[0];
        assert_eq!((t.title.as_str(), t.artist.as_str()), ("Hello, World", "A, B"));
        assert_eq!(t.album.as_deref(), Some("The \"LP\""));
        assert_eq!(t.duration_ms, Some(201_000));
        assert_eq!(t.isrc.as_deref(), Some("USX1"));
        assert_eq!((t.source, t.id.as_str()), (ProviderId::Spotify, "abc"));
        assert!(l.tracks[1].isrc.is_none());
    }

    #[test]
    fn csv_semicolon_and_multiline() {
        let csv = "title;artist\n\"Multi\nline\";Band\nPlain;Other\n";
        let l = parse_csv(csv, "x").unwrap();
        assert_eq!(l.tracks.len(), 2);
        assert_eq!(l.tracks[0].title, "Multi\nline");
        assert!(parse_csv("foo,bar\n1,2\n", "x").is_err());
    }

    #[test]
    fn spotify_json_variants() {
        let pl = r#"{"playlists":[{"name":"Mine","items":[
            {"track":{"trackName":"A","artistName":"X","albumName":"L","trackUri":"spotify:track:1"}},
            {"track":null}]}]}"#;
        let l = parse_spotify_json(pl, "f").unwrap();
        assert_eq!(l.name, "Mine");
        assert_eq!(l.tracks.len(), 1);
        assert_eq!(l.tracks[0].id, "1");

        let hist = r#"[{"endTime":"x","artistName":"X","trackName":"A","msPlayed":10},
                       {"endTime":"y","artistName":"X","trackName":"A","msPlayed":20},
                       {"artistName":"Y","trackName":"B","msPlayed":5}]"#;
        assert_eq!(parse_spotify_json(hist, "h").unwrap().tracks.len(), 2);

        let lib = r#"{"tracks":[{"artist":"Q","album":"R","track":"S","uri":"spotify:track:z"}]}"#;
        let t = &parse_spotify_json(lib, "l").unwrap().tracks[0];
        assert_eq!((t.title.as_str(), t.artist.as_str(), t.id.as_str()), ("S", "Q", "z"));
        assert!(parse_spotify_json("{\"a\":1}", "x").is_err());
    }

    #[test]
    fn m3u_with_and_without_extinf() {
        let m = "#EXTM3U\n#PLAYLIST:Road\n#EXTINF:215,Daft Punk - One More Time\n/music/a.mp3\n/music/03 - Some Title.flac\nhttp://x/y.mp3\n";
        let l = parse_m3u(m, "file");
        assert_eq!(l.name, "Road");
        assert_eq!(l.tracks.len(), 3);
        assert_eq!(l.tracks[0].artist, "Daft Punk");
        assert_eq!(l.tracks[0].duration_ms, Some(215_000));
        assert_eq!(l.tracks[1].title, "Some Title");
    }

    #[test]
    fn xspf() {
        let x = r#"<?xml version="1.0"?><playlist version="1" xmlns="http://xspf.org/ns/0/">
          <title>Mix &amp; Match</title><trackList>
          <track><location>file:///a.mp3</location><title>One</title><creator>Band</creator><album>LP</album><duration>180000</duration></track>
          <track><title><![CDATA[Two <3]]></title><creator>Other</creator></track>
          </trackList></playlist>"#;
        let l = parse_xspf(x, "f");
        assert_eq!(l.name, "Mix & Match");
        assert_eq!(l.tracks.len(), 2);
        assert_eq!(l.tracks[0].duration_ms, Some(180_000));
        assert_eq!(l.tracks[1].title, "Two <3");
    }
}
