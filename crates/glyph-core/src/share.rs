//! Playlist share codes: `glyph:playlist:` + base64url(deflate(JSON)).
//!
//! The code carries the name and, per song, title, artist, album, source id,
//! ISRC and length: enough for any Glyph to rebuild the playlist and resolve
//! every song again. Paste it into the search bar to import.
//!
//! On the public server the code is stored there and shared as a short link
//! (`https://glyph.my/p/<id>`, see [`link_id`]); pasting the link imports too.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use glyph_protocol::{ProviderId, Track};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

pub const PREFIX: &str = "glyph:playlist:";
/// Bigger codes are refused on import (a playlist of ~10 000 songs).
const MAX_JSON: usize = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Code {
    v: u8,
    n: String,
    t: Vec<Song>,
}

#[derive(Serialize, Deserialize)]
struct Song {
    t: String,
    a: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    al: Option<String>,
    s: ProviderId,
    i: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    d: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    isrc: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    x: bool,
}

pub fn encode(name: &str, tracks: &[Track]) -> String {
    let code = Code {
        v: 1,
        n: name.to_string(),
        t: tracks
            .iter()
            .map(|t| Song {
                t: t.title.clone(),
                a: t.artist.clone(),
                al: t.album.clone(),
                s: t.source,
                i: t.id.clone(),
                d: t.duration_ms,
                isrc: t.isrc.clone(),
                x: t.explicit,
            })
            .collect(),
    };
    let json = serde_json::to_vec(&code).unwrap_or_default();
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    let _ = z.write_all(&json);
    let packed = z.finish().unwrap_or_default();
    format!("{PREFIX}{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(packed))
}

/// Hosts whose `/p/<id>` links are Glyph playlists.
pub const LINK_HOSTS: [&str; 2] = ["glyph.my", "api.glyph.my"];

/// The id of a short playlist link (`https://glyph.my/p/AbC123xy`).
pub fn link_id(s: &str) -> Option<String> {
    let s = s.trim();
    let canonical;
    let s = if let Some(id) = s.strip_prefix("glyph://p/") {
        if !((id.len() == 8 || id.len() == 12) && id.bytes().all(|b| b.is_ascii_alphanumeric())) { return None; }
        canonical = format!("https://glyph.my/p/{id}");
        &canonical
    } else { s };
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
    let (host, path) = rest.split_once('/')?;
    if !LINK_HOSTS.iter().any(|h| host.eq_ignore_ascii_case(h)) {
        return None;
    }
    let id = path.strip_prefix("p/")?.trim_end_matches('/');
    let id = id.split(['?', '#']).next().unwrap_or_default();
    ((id.len() == 8 || id.len() == 12) && id.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| id.to_string())
}

/// Both codes hold the same songs in the same order (names may differ).
pub fn same_songs(a: &str, b: &str) -> bool {
    match (decode(a), decode(b)) {
        (Ok((_, x)), Ok((_, y))) => x.len() == y.len() && x.iter().zip(&y).all(|(p, q)| p.source == q.source && p.id == q.id),
        _ => false,
    }
}

/// Text that looks like a share code (whitespace around it is fine).
pub fn is_code(s: &str) -> bool {
    s.trim().starts_with(PREFIX)
}

pub fn decode(s: &str) -> Result<(String, Vec<Track>)> {
    let body = s.trim().strip_prefix(PREFIX).ok_or_else(|| anyhow!("not a Glyph playlist code"))?;
    let packed = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(body.trim_end_matches('='))
        .context("broken playlist code")?;
    let mut json = Vec::new();
    flate2::read::DeflateDecoder::new(&packed[..]).take(MAX_JSON as u64 + 1).read_to_end(&mut json).context("broken playlist code")?;
    if json.len() > MAX_JSON {
        return Err(anyhow!("playlist code too large"));
    }
    let code: Code = serde_json::from_slice(&json).context("broken playlist code")?;
    if code.t.len() > crate::library::playlist_track_limit() {
        return Err(anyhow!("Playlist exceeds this server's limit of {} songs", crate::library::playlist_track_limit()));
    }
    let tracks = code
        .t
        .into_iter()
        .filter(|s| !s.t.trim().is_empty())
        .map(|s| Track {
            id: s.i,
            source: s.s,
            title: s.t,
            artist: s.a,
            album: s.al,
            duration_ms: s.d,
            cover_url: None,
            isrc: s.isrc,
            permalink: None,
            external_ids: Default::default(),
            explicit: s.x,
        })
        .collect();
    let name = code.n.trim().chars().take(100).collect::<String>();
    Ok((if name.is_empty() { "Playlist".into() } else { name }, tracks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let t = Track {
            id: "42".into(),
            source: ProviderId::Audius,
            title: "Song".into(),
            artist: "Band".into(),
            album: Some("LP".into()),
            duration_ms: Some(180_000),
            cover_url: Some("https://x".into()),
            isrc: None,
            permalink: Some("https://y".into()),
            external_ids: Default::default(),
            explicit: true,
        };
        let code = encode("Road trip", &[t.clone(), t.clone()]);
        assert!(is_code(&format!("  {code}\n")));
        let (name, back) = decode(&code).unwrap();
        assert_eq!(name, "Road trip");
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].title, "Song");
        assert_eq!(back[0].album.as_deref(), Some("LP"));
        assert!(back[0].explicit);
        assert!(decode("glyph:playlist:@@@").is_err());
    }

    #[test]
    fn short_links() {
        assert_eq!(link_id(" https://glyph.my/p/AbC123xy \n").as_deref(), Some("AbC123xy"));
        assert_eq!(link_id("glyph.my/p/AbC123xyAbC1/").as_deref(), Some("AbC123xyAbC1"));
        assert_eq!(link_id("glyph://p/AbC123xy").as_deref(), Some("AbC123xy"));
        for bad in ["glyph://p/AbC123xy?exec=bad", "glyph://p/AbC123xy/other", "glyph://evil/AbC123xy", "glyph://p/AbC123xy\nactivate", "https://glyph.my.evil/p/AbC123xy"] {
            assert_eq!(link_id(bad), None, "{bad}");
        }
        assert_eq!(link_id("https://evil.example/p/AbC123xy"), None);
        assert_eq!(link_id("https://glyph.my/p/short"), None);
    }

    #[test]
    fn same_songs_ignores_the_name() {
        let t = Track {
            id: "1".into(), source: ProviderId::Audius, title: "S".into(), artist: "A".into(), album: None, duration_ms: None,
            cover_url: None, isrc: None, permalink: None, external_ids: Default::default(), explicit: false,
        };
        assert!(same_songs(&encode("a", &[t.clone()]), &encode("b", &[t.clone()])));
        assert!(!same_songs(&encode("a", &[t.clone()]), &encode("a", &[t.clone(), t])));
    }
}
