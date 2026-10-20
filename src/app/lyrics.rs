use super::*;

impl Glyph {
    /// Looks up other versions of the current song's lyrics (wrong lyrics:
    /// the user picks the right ones).
    pub fn load_lyrics_alternatives(&mut self, cx: &mut Context<Self>) {
        if self.lyrics.alts_loading || self.lyrics.alts_loaded {
            return;
        }
        let (Some(b), Some(track), Some(key)) = (self.backend.clone(), self.current.clone(), self.lyrics.key.clone()) else { return };
        self.lyrics.alts_loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.lyrics_candidates(&track)).await;
            this.update(cx, |s, cx| {
                if s.lyrics.key.as_deref() != Some(key.as_str()) {
                    return;
                }
                s.lyrics.alts_loading = false;
                s.lyrics.alts_loaded = true;
                let mut alts = r.unwrap_or_default();
                // What is shown now stays one of the choices (first if unknown).
                if let Some(cur) = s.lyrics.lyrics.clone() {
                    let known = alts.iter().any(|a| (!cur.source_id.is_empty() && a.source_id == cur.source_id) || (a.synced == cur.synced && a.plain == cur.plain));
                    if !known {
                        alts.insert(0, cur);
                    }
                }
                s.lyrics.alts = alts;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Index of the shown lyrics among the alternatives.
    pub fn lyrics_alt_index(&self) -> Option<usize> {
        let cur = self.lyrics.lyrics.as_ref()?;
        self.lyrics
            .alts
            .iter()
            .position(|a| (!cur.source_id.is_empty() && a.source_id == cur.source_id) || (a.synced == cur.synced && a.plain == cur.plain))
    }

    /// Shows version `i` and keeps it for this song.
    pub fn pick_lyrics(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(l) = self.lyrics.alts.get(i).cloned() else { return };
        if let (Some(b), Some(t)) = (self.backend.clone(), self.current.clone()) {
            guard(|| b.pin_lyrics(&t, &l));
        }
        self.lyrics.lyrics = Some(l);
        self.lyrics.last_line = None;
        self.lyrics.error = None;
        self.lyrics_user_scroll = None;
        cx.notify();
    }

    /// Next / previous version (`step` ±1), looked up first if needed.
    pub fn cycle_lyrics(&mut self, step: isize, cx: &mut Context<Self>) {
        if !self.lyrics.alts_loaded {
            self.load_lyrics_alternatives(cx);
            return;
        }
        let n = self.lyrics.alts.len();
        if n < 2 {
            return;
        }
        let cur = self.lyrics_alt_index().unwrap_or(0) as isize;
        let next = (cur + step).rem_euclid(n as isize) as usize;
        self.pick_lyrics(next, cx);
    }

    pub fn reset_lyrics_offset(&mut self, cx: &mut Context<Self>) {
        let cur = self.lyrics_offset();
        if cur != 0 {
            self.shift_lyrics_offset(-cur, cx);
        }
    }

    pub fn shift_lyrics_offset(&mut self, delta_ms: i64, cx: &mut Context<Self>) {
        let Some(t) = self.current.clone() else { return };
        let cur = self.lyrics_offset();
        if self.library_ok && guard(|| self.library.set_lyrics_offset(&t, cur + delta_ms)).is_some()
        {
            self.save_library(cx);
        }
        cx.notify();
    }
}
