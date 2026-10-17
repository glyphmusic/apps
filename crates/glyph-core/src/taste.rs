//! Listening taste and search ranking.
//!
//! The profile is built from what the user does: liked songs, followed
//! artists, what was played (and how recently), what was skipped early and
//! which artists were hidden. It weights artists 0..1 and is used to rank
//! search results, pick seeds for "Recommended for you" and order those.
//!
//! Search queries may quote phrases (`"blue in green"`): quoted parts must
//! appear exactly (case-insensitive) in the title, artist or album.

use crate::dedupe::{normalize_artist, normalize_title};
use crate::library::Library;
use glyph_protocol::Track;
use std::collections::{HashMap, HashSet};

/// Artist weights from the library.
#[derive(Debug, Default, Clone)]
pub struct Taste {
    artists: HashMap<String, f32>,
    hidden: HashSet<String>,
    liked: HashSet<String>,
}

/// Every credited artist of "A, B feat. C" (normalised).
pub fn credited(credit: &str) -> Vec<String> {
    let mut s = credit.to_lowercase();
    for sep in [" feat. ", " feat ", " ft. ", " ft ", " featuring ", " & ", " x ", " with ", ";", "/"] {
        s = s.replace(sep, ",");
    }
    s.split(',').map(normalize_artist).filter(|a| !a.is_empty()).collect()
}

fn song_key(t: &Track) -> String {
    format!("{}|{}", normalize_title(&t.title), normalize_artist(&t.artist))
}

impl Taste {
    pub fn from_library(lib: &Library) -> Self {
        let mut w: HashMap<String, f32> = HashMap::new();
        let mut add = |credit: &str, v: f32| {
            for a in credited(credit) {
                *w.entry(a).or_default() += v;
            }
        };
        for t in &lib.liked {
            add(&t.artist, 3.0);
        }
        for f in &lib.followed {
            add(&f.name, 6.0);
        }
        for (i, t) in lib.recent.iter().enumerate() {
            add(&t.artist, 1.5 * 0.96f32.powi(i as i32));
        }
        for (a, n) in &lib.artist_plays {
            add(a, (1.0 + *n as f32).ln() * 1.2);
        }
        for (a, n) in &lib.artist_skips {
            add(a, -(*n as f32).min(10.0) * 0.6);
        }
        let hidden: HashSet<String> = lib.hidden_artists.iter().flat_map(|h| credited(h)).collect();
        for h in &hidden {
            w.remove(h);
        }
        let max = w.values().cloned().fold(0.0f32, f32::max).max(1.0);
        let artists = w.into_iter().filter(|(_, v)| *v > 0.0).map(|(k, v)| (k, (v / max).min(1.0))).collect();
        let liked = lib.liked.iter().map(song_key).collect();
        Self { artists, hidden, liked }
    }

    /// 0..1, the best of the credited artists.
    pub fn artist_weight(&self, credit: &str) -> f32 {
        credited(credit).iter().filter_map(|a| self.artists.get(a)).cloned().fold(0.0, f32::max)
    }

    pub fn is_hidden(&self, credit: &str) -> bool {
        credited(credit).iter().any(|a| self.hidden.contains(a))
    }

    pub fn is_liked(&self, t: &Track) -> bool {
        self.liked.contains(&song_key(t))
    }

    /// The `n` artists with the most weight (normalised names).
    pub fn top_artists(&self, n: usize) -> Vec<String> {
        let mut v: Vec<(&String, &f32)> = self.artists.iter().collect();
        v.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(b.0)));
        v.into_iter().take(n).map(|(a, _)| a.clone()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.artists.is_empty()
    }
}

// ---------------------------------------------------------------- queries

/// Splits `query` into the plain words (quotes removed, sent to the sources)
/// and the quoted phrases that must match exactly.
pub fn parse_query(query: &str) -> (String, Vec<String>) {
    let mut plain = String::new();
    let mut phrases = Vec::new();
    let mut in_quote = false;
    let mut cur = String::new();
    for c in query.chars() {
        match c {
            '"' | '\u{201c}' | '\u{201d}' | '\u{201e}' => {
                if in_quote {
                    let p = cur.trim().to_lowercase();
                    if !p.is_empty() {
                        phrases.push(p);
                    }
                    plain.push(' ');
                    plain.push_str(cur.trim());
                    plain.push(' ');
                    cur.clear();
                }
                in_quote = !in_quote;
            }
            _ if in_quote => cur.push(c),
            _ => plain.push(c),
        }
    }
    // Unclosed quote: treat the rest as a phrase too.
    if in_quote && !cur.trim().is_empty() {
        phrases.push(cur.trim().to_lowercase());
        plain.push(' ');
        plain.push_str(cur.trim());
    }
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    (plain, phrases)
}

fn squash(s: &str) -> String {
    s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every quoted phrase appears in the title, artist or album.
pub fn matches_phrases(t: &Track, phrases: &[String]) -> bool {
    if phrases.is_empty() {
        return true;
    }
    let hay = [squash(&t.title), squash(&t.artist), t.album.as_deref().map(squash).unwrap_or_default()];
    phrases.iter().all(|p| {
        let p = squash(p);
        hay.iter().any(|h| h.contains(&p))
    })
}

/// Words that mark a version the user rarely wants unless they typed it.
const ODD_VERSIONS: [&str; 9] = ["karaoke", "instrumental", "8d", "nightcore", "sped up", "slowed", "cover", "tribute", "remake"];

/// Rank of `t` for `query` (higher first): text match, odd versions, taste.
pub fn score(t: &Track, query: &str, taste: &Taste) -> f32 {
    let q = squash(query);
    let title = squash(&t.title);
    let artist = squash(&t.artist);
    let mut s = 0.0;
    if !q.is_empty() {
        if title == q {
            s += 3.0;
        } else if title.starts_with(&q) {
            s += 2.0;
        } else if title.contains(&q) {
            s += 1.2;
        }
        if artist == q {
            s += 2.5;
        } else if artist.contains(&q) || q.contains(&artist) {
            s += 1.0;
        }
        // "artist title" typed together.
        if !artist.is_empty() && q.contains(&artist) && q.contains(&title) {
            s += 2.0;
        }
    }
    for w in ODD_VERSIONS {
        if title.contains(w) && !q.contains(w) {
            s -= 1.5;
        }
    }
    s += taste.artist_weight(&t.artist) * 2.0;
    if taste.is_liked(t) {
        s += 1.0;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use glyph_protocol::ProviderId;

    fn tr(title: &str, artist: &str) -> Track {
        Track {
            id: title.into(),
            source: ProviderId::Deezer,
            title: title.into(),
            artist: artist.into(),
            album: None,
            duration_ms: None,
            cover_url: None,
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false,
        }
    }

    #[test]
    fn quotes_are_exact_phrases() {
        let (plain, ph) = parse_query("miles \"Blue in Green\" davis");
        assert_eq!(plain, "miles Blue in Green davis");
        assert_eq!(ph, vec!["blue in green".to_string()]);
        assert!(matches_phrases(&tr("Blue in Green", "Miles Davis"), &ph));
        assert!(!matches_phrases(&tr("Green in Blue", "Miles Davis"), &ph));
        let (_, open) = parse_query("\"so what");
        assert_eq!(open, vec!["so what".to_string()]);
    }

    #[test]
    fn taste_and_text_rank() {
        let mut lib = Library::default();
        lib.liked.push(tr("Song", "Liked Band"));
        lib.followed.push(crate::library::FollowedArtist { name: "Liked Band".into(), track: None });
        let taste = Taste::from_library(&lib);
        assert!(taste.artist_weight("Liked Band feat. X") > 0.9);
        let a = score(&tr("Hello", "Liked Band"), "hello", &taste);
        let b = score(&tr("Hello", "Other"), "hello", &taste);
        let c = score(&tr("Hello (Karaoke)", "Other"), "hello", &taste);
        assert!(a > b && b > c);
        assert_eq!(taste.top_artists(1), vec!["liked band".to_string()]);
    }

    #[test]
    fn skips_and_hidden_lower_weight() {
        let mut lib = Library::default();
        lib.recent.push(tr("A", "Skipped"));
        lib.recent.push(tr("B", "Kept"));
        lib.artist_skips.insert("skipped".into(), 8);
        lib.hidden_artists.push("Gone".into());
        lib.liked.push(tr("C", "Gone"));
        let taste = Taste::from_library(&lib);
        assert_eq!(taste.artist_weight("Skipped"), 0.0);
        assert!(taste.artist_weight("Kept") > 0.0);
        assert_eq!(taste.artist_weight("Gone"), 0.0);
        assert!(taste.is_hidden("Gone"));
    }
}
