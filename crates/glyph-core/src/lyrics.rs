//! Lyrics model + LRC parsing. Fetching lives in the backend (LRCLIB /api/get,
//! fallback /api/search) and is cached locally.

use glyph_protocol::LyricLine;

#[derive(Debug, Clone, PartialEq)]
pub struct Lyrics {
    /// e.g. "LRCLIB"
    pub provider: String,
    /// 0.0..=1.0 match score shown as `LRCLIB · 99.4% Match`.
    pub match_score: f32,
    pub synced: Vec<LyricLine>,
    pub plain: Option<String>,
}

impl Lyrics {
    /// Parses `[mm:ss.xx] text` LRC. Ignores metadata tags, supports multiple
    /// timestamps per line, sorts by time.
    pub fn parse_lrc(src: &str) -> Vec<LyricLine> {
        let _ = src;
        todo!()
    }

    /// Index of the active line at `position_ms` (+ offset), if any.
    pub fn active_line(&self, position_ms: u64, offset_ms: i64) -> Option<usize> {
        let _ = (position_ms, offset_ms);
        todo!()
    }

    /// 0.0..=1.0 progress inside the active line (interpolated; drives the
    /// word fill left to right).
    pub fn line_progress(&self, position_ms: u64, offset_ms: i64) -> f32 {
        let _ = (position_ms, offset_ms);
        todo!()
    }
}
