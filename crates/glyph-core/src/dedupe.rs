//! Text normalisation, similarity and duplicate merging.
//!
//! Two tracks are the same song when normalised title + primary artist match
//! and the durations are within ±3 s (or the ISRCs are equal).

use glyph_protocol::Track;
use std::collections::{BTreeSet, HashMap};

/// Duration tolerance for "same song".
pub const DURATION_TOLERANCE_MS: u64 = 3_000;

fn fold(c: char, out: &mut String) {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => out.push('a'),
        'ç' | 'ć' | 'č' => out.push('c'),
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ě' => out.push('e'),
        'ì' | 'í' | 'î' | 'ï' | 'ī' => out.push('i'),
        'ñ' | 'ń' => out.push('n'),
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => out.push('o'),
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => out.push('u'),
        'ý' | 'ÿ' => out.push('y'),
        'š' | 'ś' => out.push('s'),
        'ž' | 'ź' | 'ż' => out.push('z'),
        'ß' => out.push_str("ss"),
        'æ' => out.push_str("ae"),
        'œ' => out.push_str("oe"),
        c if c.is_alphanumeric() => out.push(c),
        '&' => out.push_str(" and "),
        _ => out.push(' '),
    }
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

pub(crate) fn squash(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        fold(c, &mut out);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Cuts `feat.` / `ft.` / `featuring` and everything after it.
fn cut_feat(s: &str) -> String {
    // ASCII lowercase keeps byte offsets identical to `s`.
    let lower = s.to_ascii_lowercase();
    let mut cut = lower.len();
    for pat in [" feat.", " feat ", " ft.", " ft ", " featuring ", " with "] {
        if let Some(i) = lower.find(pat) {
            cut = cut.min(i);
        }
    }
    s[..floor_boundary(s, cut)].to_string()
}

const VERSION_WORDS: [&str; 7] =
    ["remaster", "mono", "stereo", "single version", "radio edit", "album version", "version"];

/// Normalises a song title for comparison.
pub fn normalize_title(s: &str) -> String {
    let mut t = s.to_string();
    // "Song - 2011 Remaster" -> "Song"
    if let Some(i) = t.find(" - ") {
        let tail = t[i + 3..].to_lowercase();
        if VERSION_WORDS.iter().any(|w| tail.contains(w)) {
            t.truncate(i);
        }
    }
    let stripped = strip_brackets(&t);
    let base = if squash(&stripped).is_empty() { t } else { stripped };
    squash(&cut_feat(&base))
}

/// Separators between a primary artist and featured / collaborating artists.
pub const PRIMARY_ARTIST_SEPS: [&str; 11] = [
    ",", " feat.", " feat ", " ft.", " ft ", " & ", " x ", " X ", " × ", " / ", " и ",
];

/// Separators between all credited artists on a track.
pub const CREDITED_ARTIST_SEPS: [&str; 14] = [
    ",", " feat.", " feat ", " ft.", " ft ", " & ", " x ", " X ", " × ", " / ", " with ", " vs. ", " vs ", " и ",
];

/// First artist of a combined credit ("A, B", "A feat. B", "A & B", "A и B") for
/// opening an artist page when the full credit is not a single artist.
pub fn primary_artist(raw: &str) -> String {
    let mut s = raw;
    for sep in PRIMARY_ARTIST_SEPS {
        if let Some((a, _)) = s.split_once(sep) {
            s = a;
        }
    }
    s.trim().to_string()
}

/// `true` when `name` is the artist of `track_artist` or one of the artists in
/// a combined credit ("A, B", "A feat. B", "A & B", "A и B", ...).
pub fn by_artist(track_artist: &str, name: &str) -> bool {
    let n = name.trim();
    if n.is_empty() {
        return false;
    }
    if track_artist.trim().eq_ignore_ascii_case(n) {
        return true;
    }
    let n_lower = n.to_lowercase();
    if track_artist.trim().to_lowercase() == n_lower {
        return true;
    }
    let mut parts = vec![track_artist];
    for sep in CREDITED_ARTIST_SEPS {
        parts = parts.into_iter().flat_map(|p| p.split(sep)).collect();
    }
    parts.into_iter().any(|p| {
        let p = p.trim();
        p.eq_ignore_ascii_case(n) || p.to_lowercase() == n_lower
    })
}

/// Normalises the *primary* artist (first of "A feat. B", "A & B", "A, B", "A и B", ...).
pub fn normalize_artist(s: &str) -> String {
    let mut a = strip_brackets(s);
    a = a.replace(" - Topic", "").replace(" - topic", "");
    a = cut_feat(&a);
    let lower = a.to_lowercase();
    let mut cut = lower.len();
    for pat in [",", ";", " & ", " and ", " x ", " × ", " / ", " vs ", " vs. ", " и "] {
        if let Some(i) = lower.find(pat) {
            cut = cut.min(i);
        }
    }
    let primary = &lower[..floor_boundary(&lower, cut)];
    let n = squash(primary);
    n.strip_prefix("the ").map(str::to_string).unwrap_or(n)
}

/// Key used for caches / per-song settings: normalised `title|artist`.
pub fn track_key(t: &Track) -> String {
    format!("{}|{}", normalize_title(&t.title), normalize_artist(&t.artist))
}

/// Similarity of two already-normalised strings, 0.0..=1.0 (token Dice + containment).
pub fn similarity(a: &str, b: &str) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let ta: BTreeSet<&str> = a.split(' ').collect();
    let tb: BTreeSet<&str> = b.split(' ').collect();
    let inter = ta.intersection(&tb).count() as f32;
    let dice = 2.0 * inter / (ta.len() + tb.len()) as f32;
    // Containment: "song" inside "song official audio"
    let contain = inter / ta.len().min(tb.len()) as f32;
    dice.max(contain * 0.85).min(0.99)
}

pub fn title_similarity(a: &str, b: &str) -> f32 {
    similarity(&normalize_title(a), &normalize_title(b))
}

pub fn artist_similarity(a: &str, b: &str) -> f32 {
    let (na, nb) = (normalize_artist(a), normalize_artist(b));
    let direct = similarity(&na, &nb);
    // The candidate's artist text often wraps the artist ("Artist Official", ...).
    let (la, lb) = (squash(a), squash(b));
    let contained = if (!na.is_empty() && lb.contains(&na)) || (!nb.is_empty() && la.contains(&nb)) {
        0.9
    } else {
        0.0
    };
    direct.max(contained)
}

pub fn durations_match(a: Option<u64>, b: Option<u64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.abs_diff(b) <= DURATION_TOLERANCE_MS,
        _ => true, // unknown on either side does not block a merge
    }
}

/// `true` if both describe the same song.
pub fn same_song(a: &Track, b: &Track) -> bool {
    if let (Some(x), Some(y)) = (&a.isrc, &b.isrc) {
        if !x.is_empty() && x.eq_ignore_ascii_case(y) {
            return true;
        }
    }
    if !durations_match(a.duration_ms, b.duration_ms) {
        return false;
    }
    let (ta, tb) = (normalize_title(&a.title), normalize_title(&b.title));
    !ta.is_empty()
        && ta == tb
        && normalize_artist(&a.artist) == normalize_artist(&b.artist)
}

/// Pre-normalised lookup table for `same_entry` checks in O(1) amortised time.
#[derive(Debug, Default, Clone)]
pub struct TrackIndex {
    by_id: std::collections::HashSet<(glyph_protocol::ProviderId, String)>,
    by_isrc: std::collections::HashSet<String>,
    by_key: HashMap<String, Vec<Option<u64>>>,
}

impl TrackIndex {
    pub fn from_tracks(tracks: &[Track]) -> Self {
        let mut idx = Self {
            by_id: std::collections::HashSet::with_capacity(tracks.len() * 2),
            by_isrc: std::collections::HashSet::new(),
            by_key: HashMap::with_capacity(tracks.len()),
        };
        for t in tracks {
            idx.insert(t);
        }
        idx
    }

    pub fn insert(&mut self, t: &Track) {
        self.by_id.insert((t.source, t.id.clone()));
        for (&p, id) in &t.external_ids {
            self.by_id.insert((p, id.clone()));
        }
        if let Some(isrc) = t.isrc.as_ref().filter(|s| !s.is_empty()) {
            self.by_isrc.insert(isrc.to_ascii_uppercase());
        }
        let title = normalize_title(&t.title);
        if !title.is_empty() {
            let key = format!("{title}|{}", normalize_artist(&t.artist));
            self.by_key.entry(key).or_default().push(t.duration_ms);
        }
    }

    pub fn contains(&self, t: &Track) -> bool {
        if self.by_id.contains(&(t.source, t.id.clone())) {
            return true;
        }
        if t.external_ids.iter().any(|(&p, id)| self.by_id.contains(&(p, id.clone()))) {
            return true;
        }
        if let Some(isrc) = t.isrc.as_ref().filter(|s| !s.is_empty()) {
            if self.by_isrc.contains(&isrc.to_ascii_uppercase()) {
                return true;
            }
        }
        let title = normalize_title(&t.title);
        if title.is_empty() {
            return false;
        }
        let key = format!("{title}|{}", normalize_artist(&t.artist));
        self.by_key
            .get(&key)
            .is_some_and(|durs| durs.iter().any(|&d| durations_match(d, t.duration_ms)))
    }
}

/// Merges `incoming` into `into` (same song): records the other source id and
/// fills missing metadata.
pub fn merge_track(into: &mut Track, incoming: &Track) {
    into.explicit |= incoming.explicit;
    if incoming.source != into.source {
        into.external_ids.entry(incoming.source).or_insert_with(|| incoming.id.clone());
    }
    for (p, id) in &incoming.external_ids {
        if *p != into.source {
            into.external_ids.entry(*p).or_insert_with(|| id.clone());
        }
    }
    if into.album.is_none() {
        into.album = incoming.album.clone();
    }
    if into.duration_ms.is_none() {
        into.duration_ms = incoming.duration_ms;
    }
    if into.cover_url.is_none() {
        into.cover_url = incoming.cover_url.clone();
    }
    if into.isrc.is_none() {
        into.isrc = incoming.isrc.clone();
    }
    if into.permalink.is_none() {
        into.permalink = incoming.permalink.clone();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeOutcome {
    /// New track at this index.
    New(usize),
    /// Merged into the existing track at this index.
    Merged(usize),
}

/// Incremental de-duplicator used by the unified search.
#[derive(Debug, Default)]
pub struct Merger {
    tracks: Vec<Track>,
    by_key: HashMap<String, Vec<usize>>,
    by_isrc: HashMap<String, usize>,
}

impl Merger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn add(&mut self, t: Track) -> MergeOutcome {
        let isrc = t.isrc.as_ref().filter(|s| !s.is_empty()).map(|s| s.to_uppercase());
        let key = track_key(&t);
        let mut hit = isrc.as_ref().and_then(|i| self.by_isrc.get(i).copied());
        if hit.is_none() {
            hit = self.by_key.get(&key).and_then(|idxs| {
                idxs.iter().copied().find(|&i| durations_match(self.tracks[i].duration_ms, t.duration_ms))
            });
        }
        match hit {
            Some(i) => {
                merge_track(&mut self.tracks[i], &t);
                if let Some(isrc) = self.tracks[i].isrc.as_ref() {
                    self.by_isrc.entry(isrc.to_uppercase()).or_insert(i);
                }
                MergeOutcome::Merged(i)
            }
            None => {
                let i = self.tracks.len();
                if let Some(isrc) = isrc {
                    self.by_isrc.insert(isrc, i);
                }
                self.by_key.entry(key).or_default().push(i);
                self.tracks.push(t);
                MergeOutcome::New(i)
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn test_track(
    source: glyph_protocol::ProviderId,
    id: &str,
    title: &str,
    artist: &str,
    dur: Option<u64>,
) -> Track {
    Track {
        id: id.into(),
        source,
        title: title.into(),
        artist: artist.into(),
        album: None,
        duration_ms: dur,
        cover_url: None,
        isrc: None,
        permalink: None,
        external_ids: Default::default(),
        explicit: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glyph_protocol::ProviderId;

    fn t(source: ProviderId, id: &str, title: &str, artist: &str, dur: Option<u64>) -> Track {
        test_track(source, id, title, artist, dur)
    }

    #[test]
    fn normalizes_titles() {
        assert_eq!(normalize_title("Hello (Remastered 2011)"), "hello");
        assert_eq!(normalize_title("Hello - 2011 Remaster"), "hello");
        assert_eq!(normalize_title("Hello feat. Bob"), "hello");
        assert_eq!(normalize_title("Héllo, World!"), "hello world");
        assert_eq!(normalize_title("(Only Brackets)"), "only brackets");
    }

    #[test]
    fn normalizes_artists() {
        assert_eq!(normalize_artist("The Beatles"), "beatles");
        assert_eq!(normalize_artist("Daft Punk - Topic"), "daft punk");
        assert_eq!(normalize_artist("Artist A feat. B"), "artist a");
        assert_eq!(normalize_artist("A & B"), "a");
        assert_eq!(normalize_artist("Баста и Гуф"), "баста");
        assert_eq!(primary_artist("Баста и Гуф"), "Баста");
        assert!(by_artist("Баста и Гуф", "Гуф"));
        assert!(by_artist("Баста и Гуф", "гуф"));
        assert!(!by_artist("Кино", "К"));
    }

    #[test]
    fn merges_duplicates_with_duration_tolerance() {
        let mut m = Merger::new();
        assert_eq!(
            m.add(t(ProviderId::Spotify, "s1", "Song (Remastered)", "Band", Some(200_000))),
            MergeOutcome::New(0)
        );
        assert_eq!(
            m.add(t(ProviderId::Audius, "a1", "Song", "Band", Some(202_500))),
            MergeOutcome::Merged(0)
        );
        // 10 s off: different recording
        assert_eq!(m.add(t(ProviderId::Deezer, "d1", "Song", "Band", Some(210_000))), MergeOutcome::New(1));
        let first = &m.tracks()[0];
        assert_eq!(first.external_ids.get(&ProviderId::Audius).map(String::as_str), Some("a1"));
        assert_eq!(first.sources(), vec![ProviderId::Spotify, ProviderId::Audius]);
    }

    #[test]
    fn merges_by_isrc() {
        let mut m = Merger::new();
        let mut a = t(ProviderId::Spotify, "s", "X", "Y", Some(1000));
        a.isrc = Some("USABC1234567".into());
        let mut b = t(ProviderId::Deezer, "d", "Totally different title", "Z", Some(900_000));
        b.isrc = Some("usabc1234567".into());
        m.add(a);
        assert_eq!(m.add(b), MergeOutcome::Merged(0));
    }

    #[test]
    fn similarity_scores() {
        assert_eq!(similarity("a b", "a b"), 1.0);
        assert!(similarity("hello world", "hello") > 0.7);
        assert!(similarity("foo", "bar") < 0.1);
        assert!(artist_similarity("Daft Punk - Topic", "Daft Punk") > 0.9);
    }
}
