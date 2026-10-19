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
    for sep in [" feat. ", " feat ", " ft. ", " ft ", " featuring ", " & ", " x ", " × ", " with ", " vs. ", " vs ", " и ", ";", "/"] {
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

/// Folded words without punctuation ("Beyoncé - Halo" -> beyonce halo).
fn words(s: &str) -> Vec<String> {
    crate::dedupe::squash(s).split_whitespace().map(str::to_string).collect()
}

/// Joiners people type between title and artist ("Song by Artist").
const JOINERS: [&str; 8] = ["by", "von", "feat", "ft", "featuring", "x", "and", "und"];

/// Song and artist typed together, in any order or with joiners: "believer
/// imagine dragons", "Imagine Dragons - Believer", "believer von imagine dragons".
/// Rewards both being present and punishes query words the track lacks, so the
/// wrong artist's song with the same title drops below the right one.
fn title_artist_match(t: &Track, q: &str) -> f32 {
    let qw = words(q);
    if qw.len() < 2 {
        return 0.0;
    }
    let all_in_query = |ws: &[String]| !ws.is_empty() && ws.iter().all(|w| qw.contains(w));
    let core_title = words(&normalize_title(&t.title));
    let full_title = words(&t.title);
    let credits: Vec<String> = credited(&t.artist).iter().flat_map(|a| words(a)).collect();
    // Any one credited artist is enough ("A feat. B" found by typing A).
    // YouTube uploads often read "Artist - Title" under a channel name: then
    // both halves of the title typed count as title plus artist.
    let halves: Vec<&str> = t.title.split(" - ").collect();
    let artist_typed = credited(&t.artist).iter().any(|a| all_in_query(&words(a)))
        || (halves.len() == 2 && halves.iter().all(|h| all_in_query(&words(&normalize_title(h)))));
    let mut s = 0.0;
    if all_in_query(&core_title) && artist_typed {
        s += 4.0;
    }
    let album = t.album.as_deref().map(words).unwrap_or_default();
    let missing = qw
        .iter()
        .filter(|w| !JOINERS.contains(&w.as_str()) && !full_title.contains(w) && !credits.contains(w) && !album.contains(w))
        .count();
    s - (missing as f32 * 0.6).min(2.4)
}

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
        } else if !artist.is_empty() && (artist.contains(&q) || q.contains(&artist)) {
            s += 1.0;
        }
        s += title_artist_match(t, &q);
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
    fn title_plus_artist_picks_the_right_song() {
        let taste = Taste::default();
        let right = tr("Believer", "Imagine Dragons");
        let cover = tr("Believer", "Some Cover Band");
        let other = tr("Thunder", "Imagine Dragons");
        for q in ["believer imagine dragons", "imagine dragons believer", "Believer - Imagine Dragons", "believer von imagine dragons", "believer by imagine dragons"] {
            let r = score(&right, q, &taste);
            assert!(r > score(&cover, q, &taste), "{q}");
            assert!(r > score(&other, q, &taste), "{q}");
        }
        // Featured artists, accents and channel uploads still match.
        assert!(score(&tr("Halo", "Beyoncé"), "halo beyonce", &taste) > score(&tr("Halo", "Other"), "halo beyonce", &taste));
        let upload = tr("Imagine Dragons - Believer (Official Music Video)", "ImagineDragonsVEVO");
        assert!(score(&upload, "imagine dragons believer", &taste) > score(&cover, "imagine dragons believer", &taste));
        let feat = tr("Stay", "Justin Bieber feat. The Kid LAROI");
        assert!(score(&feat, "stay justin bieber", &taste) > score(&tr("Stay", "Rihanna"), "stay justin bieber", &taste));
        // A title alone ranks as before.
        assert!(score(&right, "believer", &taste) > score(&other, "believer", &taste));
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
