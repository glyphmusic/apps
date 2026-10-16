//! Lyrics model + LRC parsing. Fetching lives in the backend (LRCLIB /api/get,
//! fallback /api/search) and is cached locally.

use crate::dedupe::track_key;
use crate::paths;
use glyph_protocol::{LyricLine, LyricsResponse, ProviderId, Track};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Lyrics {
    /// e.g. "LRCLIB"
    pub provider: String,
    /// 0.0..=1.0 match score shown as `LRCLIB · 99.4% Match`.
    pub match_score: f32,
    pub synced: Vec<LyricLine>,
    pub plain: Option<String>,
    /// Instrumental track: nothing to show.
    pub instrumental: bool,
}

/// Fallback duration of the last line (no following timestamp).
const LAST_LINE_MS: i64 = 5_000;

impl Lyrics {
    /// Parses `[mm:ss.xx] text` LRC. Ignores metadata tags, supports multiple
    /// timestamps per line, sorts by time.
    pub fn parse_lrc(src: &str) -> Vec<LyricLine> {
        let mut out: Vec<LyricLine> = Vec::new();
        for raw in src.lines() {
            let mut rest = raw.trim_start_matches('\u{feff}').trim();
            let mut times: Vec<u64> = Vec::new();
            while let Some(stripped) = rest.strip_prefix('[') {
                let Some(end) = stripped.find(']') else { break };
                match parse_timestamp(&stripped[..end]) {
                    Some(t) => times.push(t),
                    // Metadata tag ([ar:..], [ti:..], [offset:..]) — drop the line if no time tags.
                    None => {
                        if times.is_empty() {
                            rest = "";
                            break;
                        }
                        break;
                    }
                }
                rest = stripped[end + 1..].trim_start();
            }
            if times.is_empty() {
                continue;
            }
            let text = strip_word_tags(rest);
            for t in times {
                out.push(LyricLine { time_ms: t, text: text.clone() });
            }
        }
        out.sort_by_key(|l| l.time_ms); // stable
        out
    }

    fn effective(position_ms: u64, offset_ms: i64) -> i64 {
        position_ms as i64 + offset_ms
    }

    /// Index of the active line at `position_ms` (+ offset), if any.
    pub fn active_line(&self, position_ms: u64, offset_ms: i64) -> Option<usize> {
        let pos = Self::effective(position_ms, offset_ms);
        if pos < 0 {
            return None;
        }
        let idx = self.synced.partition_point(|l| l.time_ms as i64 <= pos);
        idx.checked_sub(1)
    }

    /// 0.0..=1.0 progress inside the active line (interpolated; drives the
    /// word fill left to right).
    pub fn line_progress(&self, position_ms: u64, offset_ms: i64) -> f32 {
        let Some(i) = self.active_line(position_ms, offset_ms) else { return 0.0 };
        let pos = Self::effective(position_ms, offset_ms);
        let start = self.synced[i].time_ms as i64;
        let end = self
            .synced
            .get(i + 1)
            .map(|l| l.time_ms as i64)
            .unwrap_or(start + LAST_LINE_MS);
        let len = (end - start).max(1);
        ((pos - start) as f32 / len as f32).clamp(0.0, 1.0)
    }

    pub fn is_synced(&self) -> bool {
        !self.synced.is_empty()
    }

    /// `LRCLIB · 99.4% Match`
    pub fn match_label(&self) -> String {
        format!("{} · {:.1}% Match", self.provider, self.match_score * 100.0)
    }

    pub fn from_response(r: &LyricsResponse, match_score: f32) -> Self {
        Lyrics {
            provider: r.provider.display_name().to_string(),
            match_score,
            synced: r.synced.clone(),
            plain: r.plain.clone(),
            instrumental: r.instrumental,
        }
    }

    pub fn to_response(&self) -> LyricsResponse {
        LyricsResponse {
            provider: ProviderId::LrcLib,
            synced: self.synced.clone(),
            plain: self.plain.clone(),
            instrumental: self.instrumental,
        }
    }
}

/// `mm:ss`, `mm:ss.xx`, `mm:ss:xx`, `m:ss.xxx` -> ms. Non-time tags -> None.
fn parse_timestamp(tag: &str) -> Option<u64> {
    let mut parts = tag.splitn(2, ':');
    let min: u64 = parts.next()?.trim().parse().ok()?;
    let rest = parts.next()?.trim();
    let (sec_s, frac_s) = match rest.split_once(['.', ':']) {
        Some((s, f)) => (s, f),
        None => (rest, ""),
    };
    let sec: u64 = sec_s.parse().ok()?;
    if sec >= 100 {
        return None;
    }
    let frac_ms = if frac_s.is_empty() {
        0
    } else {
        if !frac_s.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        // ".5" = 500 ms, ".50" = 500 ms, ".500" = 500 ms
        let digits: String = frac_s.chars().take(3).collect();
        let n: u64 = digits.parse().ok()?;
        n * 10u64.pow(3 - digits.len() as u32)
    };
    Some(min * 60_000 + sec * 1000 + frac_ms)
}

/// Removes enhanced-LRC word tags (`<00:12.34>`).
fn strip_word_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            let mut closed = false;
            for n in chars.by_ref() {
                if n == '>' {
                    closed = true;
                    break;
                }
                tag.push(n);
            }
            if closed && parse_timestamp(&tag).is_some() {
                continue;
            }
            out.push('<');
            out.push_str(&tag);
            if closed {
                out.push('>');
            }
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cache file key for a song's lyrics (normalised title|artist + duration in s).
pub fn cache_key(t: &Track) -> String {
    paths::hash_hex(&format!("{}|{}", track_key(t), t.duration_ms.unwrap_or(0) / 1000))
}

/// How long "no lyrics found" is remembered.
const NEGATIVE_TTL_SECS: u64 = 24 * 3600;

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    fetched: u64,
    score: f32,
    lyrics: Option<LyricsResponse>,
}

/// Local lyrics cache (one JSON file per song in the data dir).
pub struct LyricsCache {
    dir: PathBuf,
}

impl LyricsCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `Some(Some(l))` hit, `Some(None)` cached "not found", `None` unknown/expired.
    pub fn get(&self, key: &str) -> Option<Option<Lyrics>> {
        let e: CacheEntry = paths::read_json(&self.dir.join(format!("{key}.json")))?;
        match e.lyrics {
            Some(r) => Some(Some(Lyrics::from_response(&r, e.score))),
            None if paths::now_unix().saturating_sub(e.fetched) < NEGATIVE_TTL_SECS => Some(None),
            None => None,
        }
    }

    pub fn put(&self, key: &str, lyrics: Option<&Lyrics>) {
        let e = CacheEntry {
            fetched: paths::now_unix(),
            score: lyrics.map(|l| l.match_score).unwrap_or(0.0),
            lyrics: lyrics.map(Lyrics::to_response),
        };
        if let Err(err) = paths::write_json(&self.dir.join(format!("{key}.json")), &e) {
            tracing::debug!("lyrics cache write failed: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lyrics(src: &str) -> Lyrics {
        Lyrics {
            provider: "LRCLIB".into(),
            match_score: 0.994,
            synced: Lyrics::parse_lrc(src),
            plain: None,
            instrumental: false,
        }
    }

    #[test]
    fn parses_basic_and_metadata() {
        let src = "[ar:Artist]\n[ti:Title]\n[offset:+100]\n[00:12.00] Hello\n[00:15.5]World\n[01:02:25] Third";
        let l = Lyrics::parse_lrc(src);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0], LyricLine { time_ms: 12_000, text: "Hello".into() });
        assert_eq!(l[1].time_ms, 15_500);
        assert_eq!(l[2].time_ms, 62_250);
    }

    #[test]
    fn multiple_timestamps_sorted_and_word_tags() {
        let src = "[00:30.00][00:10.00] Chorus <00:10.20>la <00:10.50>la\n[00:20.00]Verse\n[00:25.00]";
        let l = Lyrics::parse_lrc(src);
        assert_eq!(
            l.iter().map(|x| (x.time_ms, x.text.as_str())).collect::<Vec<_>>(),
            vec![(10_000, "Chorus la la"), (20_000, "Verse"), (25_000, ""), (30_000, "Chorus la la")]
        );
    }

    #[test]
    fn ignores_garbage() {
        assert!(Lyrics::parse_lrc("just text\n[abc] nope\n").is_empty());
        assert!(Lyrics::parse_lrc("").is_empty());
    }

    #[test]
    fn active_line_and_offset() {
        let l = lyrics("[00:10.00] a\n[00:20.00] b\n[00:30.00] c");
        assert_eq!(l.active_line(5_000, 0), None);
        assert_eq!(l.active_line(10_000, 0), Some(0));
        assert_eq!(l.active_line(19_999, 0), Some(0));
        assert_eq!(l.active_line(20_000, 0), Some(1));
        assert_eq!(l.active_line(99_000, 0), Some(2));
        // +500 ms offset shifts lines earlier
        assert_eq!(l.active_line(19_600, 500), Some(1));
        assert_eq!(l.active_line(500, -1_000), None);
    }

    #[test]
    fn progress_interpolates() {
        let l = lyrics("[00:10.00] a\n[00:20.00] b");
        assert_eq!(l.line_progress(5_000, 0), 0.0);
        assert!((l.line_progress(15_000, 0) - 0.5).abs() < 1e-4);
        // last line uses the fallback length
        assert!((l.line_progress(22_500, 0) - 0.5).abs() < 1e-4);
        assert_eq!(l.line_progress(60_000, 0), 1.0);
    }

    #[test]
    fn cache_roundtrip_and_negative() {
        let dir = tempfile::tempdir().unwrap();
        let c = LyricsCache::new(dir.path().to_path_buf());
        assert!(c.get("k").is_none());
        let mut l = lyrics("[00:01.00] x");
        l.match_score = 0.5;
        c.put("k", Some(&l));
        let got = c.get("k").unwrap().unwrap();
        assert_eq!(got.synced, l.synced);
        assert_eq!(got.match_score, 0.5);
        c.put("none", None);
        assert_eq!(c.get("none"), Some(None));
    }

    #[test]
    fn label() {
        assert_eq!(lyrics("[00:01.00] x").match_label(), "LRCLIB · 99.4% Match");
    }
}
