//! Search by a line of the lyrics: "which song goes «we will, we will…»".
//!
//! The lyric index (Genius' public lyric search) finds title + artist and the
//! matching line; each hit is then looked up on Deezer so it comes back as a
//! normal song with a cover that the resolver can play.

use crate::provider::{send_json, MusicProvider};
use anyhow::Result;
use glyph_protocol::Track;
use std::time::Duration;

/// A song found through its lyrics, with the line that matched.
#[derive(Debug, Clone, PartialEq)]
pub struct LyricHit {
    pub track: Track,
    pub snippet: String,
}

/// Queries that look like a lyric line (a few words, not a name).
pub fn looks_like_lyrics(q: &str) -> bool {
    let words = q.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count();
    words >= 3 && q.trim().chars().count() >= 12
}

/// (title, artist, matched line) from the lyric index's answer.
pub fn parse_hits(v: &serde_json::Value) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let sections = v["response"]["sections"].as_array().cloned().unwrap_or_default();
    for s in sections {
        for h in s["hits"].as_array().cloned().unwrap_or_default() {
            let r = &h["result"];
            let title = r["title"].as_str().unwrap_or("").trim().to_string();
            let artist = r["primary_artist"]["name"].as_str().or(r["artist_names"].as_str()).unwrap_or("").trim().to_string();
            if title.is_empty() || artist.is_empty() {
                continue;
            }
            let snippet = h["highlights"][0]["value"].as_str().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
            if !out.iter().any(|(t, a, _): &(String, String, String)| t.eq_ignore_ascii_case(&title) && a.eq_ignore_ascii_case(&artist)) {
                out.push((title, artist, snippet));
            }
        }
    }
    out
}

pub async fn search(http: &reqwest::Client, deezer: &crate::providers::deezer::Deezer, q: &str, max: usize) -> Result<Vec<LyricHit>> {
    let req = http
        .get("https://genius.com/api/search/lyric")
        .query(&[("q", q), ("per_page", "8")])
        .timeout(Duration::from_secs(6));
    let v: serde_json::Value = send_json(req).await?;
    let hits = parse_hits(&v);
    let lookups = hits.into_iter().take(max).map(|(title, artist, snippet)| async move {
        let found = deezer.search(&format!("{artist} {title}")).await.ok()?;
        let track = found
            .into_iter()
            .find(|t| crate::dedupe::by_artist(&t.artist, &artist) && crate::dedupe::title_similarity(&title, &t.title) >= 0.6)?;
        Some(LyricHit { track, snippet })
    });
    Ok(futures::future::join_all(lookups).await.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyric_like_queries() {
        assert!(looks_like_lyrics("we will we will rock you"));
        assert!(!looks_like_lyrics("queen"));
        assert!(!looks_like_lyrics("daft punk"));
    }

    #[test]
    fn parses_hits_with_highlights() {
        let v = serde_json::json!({"response": {"sections": [{"type": "lyric", "hits": [
            {"highlights": [{"value": "Buddy, you're a boy\nmake a big noise"}], "result": {"title": "We Will Rock You", "primary_artist": {"name": "Queen"}}},
            {"highlights": [], "result": {"title": "We Will Rock You", "primary_artist": {"name": "Queen"}}},
            {"result": {"title": "", "primary_artist": {"name": "X"}}}
        ]}]}});
        let h = parse_hits(&v);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].0, "We Will Rock You");
        assert_eq!(h[0].2, "Buddy, you're a boy make a big noise");
    }
}
