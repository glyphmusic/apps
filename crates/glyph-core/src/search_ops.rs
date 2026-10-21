//! Search operators typed into the search bar.
//!
//! - Source: `yt:`, `au:`, `sc:`, `sp:`, `dz:`, `ja:`, `ia:` (the badges) or
//!   the full name (`youtube:`, `soundcloud:`, ...). Several are OR-ed:
//!   `yt: sc: believer`.
//! - Fields: `artist:` / `a:`, `title:` / `t:`, `album:` / `al:`. The value
//!   runs to the next operator: `believer artist:imagine dragons`.
//! - `-word` / `-"two words"` leaves out songs that contain it.
//! - `explicit:yes|no`.
//! - `dur:>3`, `dur:<2:30` (minutes or m:ss; also `long`, `short`).
//!
//! Everything else stays plain text (`Re:Zero` too); quoted phrases keep
//! working as in [`crate::taste::parse_query`].

use crate::dedupe::squash;
use glyph_protocol::{ProviderId, Track};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct SearchOps {
    /// The query without operators (quotes kept).
    pub text: String,
    pub providers: Vec<ProviderId>,
    pub artist: Option<String>,
    pub title: Option<String>,
    pub album: Option<String>,
    pub exclude: Vec<String>,
    pub explicit: Option<bool>,
    pub min_ms: Option<u64>,
    pub max_ms: Option<u64>,
}

fn provider(key: &str) -> Option<ProviderId> {
    use ProviderId as P;
    Some(match key {
        "yt" | "youtube" => P::YouTube,
        "au" | "audius" => P::Audius,
        "sc" | "soundcloud" => P::SoundCloud,
        "sp" | "spotify" => P::Spotify,
        "dz" | "deezer" => P::Deezer,
        "ja" | "jamendo" => P::Jamendo,
        "ia" | "archive" => P::Archive,
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Artist,
    Title,
    Album,
}

fn field(key: &str) -> Option<Field> {
    Some(match key {
        "artist" | "a" | "by" => Field::Artist,
        "title" | "t" | "song" => Field::Title,
        "album" | "al" => Field::Album,
        _ => return None,
    })
}

/// Whitespace-separated tokens; a quoted part stays inside its token.
fn tokens(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in q.chars() {
        match c {
            '"' | '\u{201c}' | '\u{201d}' | '\u{201e}' => {
                quoted = !quoted;
                cur.push('"');
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `key:rest` with a known key (lowercased), else `None`.
fn split_op(tok: &str) -> Option<(String, &str)> {
    let (k, rest) = tok.split_once(':')?;
    let k = k.to_ascii_lowercase();
    let known = provider(&k).is_some() || field(&k).is_some() || matches!(k.as_str(), "explicit" | "e" | "dur" | "duration" | "len");
    known.then_some((k, rest))
}

fn unquote(s: &str) -> String {
    s.replace('"', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "3" / "3.5" minutes or "2:30".
fn minutes_ms(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches(['m', 'M']);
    if let Some((m, sec)) = s.split_once(':') {
        return Some((m.parse::<u64>().ok()? * 60 + sec.parse::<u64>().ok()?) * 1000);
    }
    let m: f64 = s.replace(',', ".").parse().ok()?;
    (m >= 0.0).then(|| (m * 60_000.0) as u64)
}

impl SearchOps {
    pub fn parse(query: &str) -> Self {
        let mut ops = SearchOps::default();
        let mut text: Vec<String> = Vec::new();
        // The field whose value is still being typed, and its words.
        let mut open: Option<(Field, Vec<String>)> = None;
        let close = |ops: &mut SearchOps, open: &mut Option<(Field, Vec<String>)>| {
            if let Some((f, words)) = open.take() {
                let v = unquote(&words.join(" "));
                if !v.is_empty() {
                    let slot = match f {
                        Field::Artist => &mut ops.artist,
                        Field::Title => &mut ops.title,
                        Field::Album => &mut ops.album,
                    };
                    *slot = Some(v);
                }
            }
        };
        for tok in tokens(query) {
            if let Some((key, rest)) = split_op(&tok) {
                close(&mut ops, &mut open);
                if let Some(p) = provider(&key) {
                    if !ops.providers.contains(&p) {
                        ops.providers.push(p);
                    }
                    if !rest.is_empty() {
                        text.push(rest.to_string());
                    }
                } else if let Some(f) = field(&key) {
                    open = Some((f, if rest.is_empty() { Vec::new() } else { vec![rest.to_string()] }));
                } else if matches!(key.as_str(), "explicit" | "e") {
                    ops.explicit = match rest.to_ascii_lowercase().as_str() {
                        "yes" | "y" | "true" | "1" | "ja" | "on" => Some(true),
                        "no" | "n" | "false" | "0" | "nein" | "off" => Some(false),
                        _ => None,
                    };
                } else {
                    let r = rest.trim();
                    match r.to_ascii_lowercase().as_str() {
                        "long" => ops.min_ms = Some(6 * 60_000),
                        "short" => ops.max_ms = Some(2 * 60_000),
                        _ => {
                            if let Some(v) = r.strip_prefix('>').map(|v| v.trim_start_matches('=')) {
                                ops.min_ms = minutes_ms(v);
                            } else if let Some(v) = r.strip_prefix('<').map(|v| v.trim_start_matches('=')) {
                                ops.max_ms = minutes_ms(v);
                            } else if let Some((a, b)) = r.split_once('-') {
                                ops.min_ms = minutes_ms(a);
                                ops.max_ms = minutes_ms(b);
                            }
                        }
                    }
                }
                continue;
            }
            if let Some(ex) = tok.strip_prefix('-').filter(|e| !e.is_empty() && !e.starts_with('-')) {
                let ex = unquote(ex);
                if !ex.is_empty() {
                    ops.exclude.push(squash(&ex));
                }
                continue;
            }
            match &mut open {
                Some((_, words)) => words.push(tok),
                None => text.push(tok),
            }
        }
        close(&mut ops, &mut open);
        ops.text = text.join(" ");
        ops
    }

    /// Any operator in use.
    pub fn is_filtered(&self) -> bool {
        *self != SearchOps { text: self.text.clone(), ..Default::default() }
    }

    /// What the sources are asked for: the text plus the field values.
    pub fn query(&self) -> String {
        [Some(&self.text), self.artist.as_ref(), self.title.as_ref(), self.album.as_ref()]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn matches(&self, t: &Track) -> bool {
        if !self.providers.is_empty() && !t.sources().iter().any(|p| self.providers.contains(p)) {
            return false;
        }
        let has = |hay: &str, v: &Option<String>| v.as_ref().map_or(true, |v| squash(hay).contains(&squash(v)));
        if !has(&t.artist, &self.artist) || !has(&t.title, &self.title) || !has(t.album.as_deref().unwrap_or(""), &self.album) {
            return false;
        }
        if !self.exclude.is_empty() {
            // Whole words: `-live` keeps "Deliver".
            let hay = format!(" {} {} {} ", squash(&t.title), squash(&t.artist), t.album.as_deref().map(squash).unwrap_or_default());
            if self.exclude.iter().any(|x| hay.contains(&format!(" {x} "))) {
                return false;
            }
        }
        if self.explicit.is_some_and(|e| e != t.explicit) {
            return false;
        }
        if self.min_ms.is_some() || self.max_ms.is_some() {
            let Some(d) = t.duration_ms else { return false };
            if self.min_ms.is_some_and(|m| d < m) || self.max_ms.is_some_and(|m| d > m) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_operators() {
        let o = SearchOps::parse("YT: believer artist:imagine dragons -live dur:<4");
        assert_eq!(o.providers, vec![ProviderId::YouTube]);
        assert_eq!(o.text, "believer");
        assert_eq!(o.artist.as_deref(), Some("imagine dragons"));
        assert_eq!(o.exclude, vec!["live".to_string()]);
        assert_eq!(o.max_ms, Some(240_000));
        assert_eq!(o.query(), "believer imagine dragons");
    }

    #[test]
    fn unknown_keys_stay_text() {
        let o = SearchOps::parse("Re:Zero \"blue in green\"");
        assert_eq!(o.text, "Re:Zero \"blue in green\"");
        assert!(!o.is_filtered());
    }

    #[test]
    fn provider_glued_to_text() {
        let o = SearchOps::parse("sc:drake au:");
        assert_eq!(o.providers, vec![ProviderId::SoundCloud, ProviderId::Audius]);
        assert_eq!(o.text, "drake");
    }
}
