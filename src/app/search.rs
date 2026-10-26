use super::*;

impl Glyph {
    // ------------------------------------------------------------ navigation

    pub fn navigate(&mut self, view: View, window: &mut Window, cx: &mut Context<Self>) {
        if view == View::Settings {
            self.open_settings(None, cx);
            return;
        }
        // Sidebar navigation starts a fresh history.
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        self.history.clear();
        self.ctx_menu = None;
        self.selected_row = None;
        if self.view == View::Search && view != View::Search {
            self.commit_search_query(cx);
        }
        self.view = view;
        if view == View::Search {
            let f = self.search_input.read(cx).focus.clone();
            window.focus(&f, cx);
        }
        if self.current.is_none() || view == View::Settings {
            let preset = match view {
                View::Home => Preset::Strahl,
                View::Search => Preset::Band,
                View::Library => Preset::Horizont,
                View::NowPlaying => Preset::Sonnenaufgang,
                View::Settings => Preset::Horizont,
                View::List => Preset::Horizont,
                View::Artist | View::Profile | View::Releases | View::Similar | View::Stats => Preset::Sonnenaufgang,
                View::Section => Preset::Strahl,
                View::Local => Preset::Horizont,
            };
            let preset = self.glow_shape(preset);
            self.glow.update(cx, |g, _| g.set_preset(preset));
        }
        cx.notify();
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let f = self.search_input.read(cx).focus.clone();
        window.focus(&f, cx);
        cx.notify();
    }

    // ------------------------------------------------------------ search

    pub(super) fn on_query(&mut self, q: String, immediate: bool, cx: &mut Context<Self>) {
        // Cancel pending work before handling pasted links/codes as well.
        self.search.seq += 1;
        self.search.task = None;
        self.search.lyric_task = None;
        self.search.more_task = None;
        self.search.loading = false;
        self.search.loading_more = false;
        // A pasted settings code opens the import in settings.
        if glyph_core::settings_share::is_code(&q) {
            let code = q.clone();
            self.search_input.update(cx, |i, cx| i.set_value(String::new(), cx));
            self.import_settings_text(&code, cx);
            return;
        }
        // A pasted jam code joins the jam.
        if glyph_protocol::jam::is_jam_text(&q) {
            let code = q.clone();
            self.search_input.update(cx, |i, cx| i.set_value(String::new(), cx));
            self.jam_join(&code, cx);
            return;
        }
        // A pasted playlist code or link imports instead of searching.
        if glyph_core::share::is_code(&q) || glyph_core::share::link_id(&q).is_some() {
            let code = q.clone();
            self.search_input.update(cx, |i, cx| i.set_value(String::new(), cx));
            self.import_share_code(&code, cx);
            return;
        }
        // A pasted (or submitted) Spotify / SoundCloud / ... link imports it
        // as a playlist; typing a URL letter by letter does not.
        let pasted = immediate || q.trim().len() >= self.search.query.trim().len() + 8;
        // Many links at once (songs copied in Spotify): Transfer music.
        if pasted && q.split_whitespace().count() > 1 {
            let (songs, lists) = glyph_core::Backend::pasted_links(&q);
            if songs + lists > 1 {
                let text = q.clone();
                self.search_input.update(cx, |i, cx| i.set_value(String::new(), cx));
                self.migrate_open_pasted(text, cx);
                return;
            }
        }
        use glyph_core::import::RemoteSource as R;
        if pasted && glyph_core::import::parse_url(&q).is_some_and(|s| !matches!(s, R::SpotifyTrack(_) | R::DeezerTrack(_))) {
            let link = q.trim().to_string();
            self.search_input.update(cx, |i, cx| i.set_value(String::new(), cx));
            self.import_link_in_dialog(&link, None, cx);
            return;
        }
        if glyph_core::eq::is_code(&q) {
            let code = q.clone();
            self.search_input.update(cx, |i, cx| i.set_value(String::new(), cx));
            self.import_eq_code(&code, cx);
            return;
        }
        // Another query (not the same one typed on or shortened): the old
        // one counts as searched.
        let related = |a: &str, b: &str| {
            let (a, b) = (a.trim().to_lowercase(), b.trim().to_lowercase());
            a.starts_with(&b) || b.starts_with(&a)
        };
        if self.search.unpicked.as_deref().is_some_and(|u| q.trim().is_empty() || !related(u, &q)) {
            self.commit_search_query(cx);
        }
        self.search.query = q.clone();
        // Submitted with Enter: counts even before results come in.
        if immediate && !q.trim().is_empty() {
            self.search.unpicked = Some(q.trim().to_string());
        }
        self.selected_row = None;
        let seq = self.search.seq;
        if self.view != View::Search && !q.trim().is_empty() {
            self.view = View::Search;
            self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        }
        if q.trim().is_empty() {
            self.search.task = None;
            self.search.lyric_task = None;
            self.search.lyric_hits.clear();
            self.search.loading = false;
            self.search.clear_results();
            self.search.seen.clear();
            self.search.errors.clear();
            self.search.error = None;
            cx.notify();
            return;
        }
        self.search.loading = true;
        cx.notify();
        if self.offline() {
            self.search.task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SEARCH_DEBOUNCE).await;
                this.update(cx, |s, cx| {
                    if s.search.seq != seq { return; }
                    s.search_offline(&q);
                    cx.notify();
                }).ok();
            }));
            return;
        }
        // Lines of lyrics also find songs (separate section, own request).
        self.search.lyric_task = None;
        let lq = glyph_core::search_ops::SearchOps::parse(&q).text;
        if glyph_core::lyricsearch::looks_like_lyrics(&lq) {
            self.search.lyric_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SEARCH_DEBOUNCE).await;
                let Some(b) = this.update(cx, |s, _| (s.search.seq == seq).then(|| s.backend.clone()).flatten()).ok().flatten() else { return };
                let r = guard_async(b.search_lyrics(&lq)).await;
                this.update(cx, |s, cx| {
                    if s.search.seq != seq {
                        return;
                    }
                    s.search.lyric_hits = r
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|h| s.allowed(&h.track) && !s.is_hidden_artist(&h.track.artist))
                        .map(|h| (h.track, h.snippet))
                        .collect();
                    cx.notify();
                })
                .ok();
            }));
        } else {
            self.search.lyric_hits.clear();
        }
        self.search.task = Some(cx.spawn(async move |this, cx| {
            // Wait until typing pauses: one request per query, not per key.
            // Submit/paste must not bypass the quiet period either.
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            let stream = this
                .update(cx, |s, _| {
                    if s.search.seq != seq {
                        return Err(None);
                    }
                    s.search.clear_results();
                    s.search.seen.clear();
                    s.search.errors.clear();
                    s.search.error = None;
                    s.search.page = 0;
                    s.search.exhausted = false;
                    s.search.loading_more = false;
                    s.search.more_task = None;
                    let Some(b) = s.backend.clone() else {
                        return Err(Some(t("status.backend_offline").to_string()));
                    };
                    let (plain, _) = glyph_core::taste::parse_query(&glyph_core::search_ops::SearchOps::parse(q.trim()).query());
                    guard(|| b.search(&plain)).ok_or(Some(t("error.not_ready").to_string()))
                })
                .ok();
            let mut stream = match stream {
                Some(Ok(s)) => s,
                Some(Err(Some(e))) => {
                    this.update(cx, |s, cx| {
                        s.search.loading = false;
                        s.search.error = Some(e);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
                _ => return,
            };
            use futures::{FutureExt, StreamExt};
            loop {
                let next = std::panic::AssertUnwindSafe(stream.next()).catch_unwind().await;
                let ev = match next {
                    Ok(Some(ev)) => ev,
                    Ok(None) => SearchEvent::Done,
                    Err(_) => SearchEvent::Done,
                };
                let done = matches!(ev, SearchEvent::Done);
                let alive = this
                    .update(cx, |s, cx| {
                        if s.search.seq != seq {
                            return false;
                        }
                        match ev {
                            SearchEvent::Results { tracks, .. } => {
                                let ops = glyph_core::search_ops::SearchOps::parse(&s.search.query);
                                let (plain, phrases) = glyph_core::taste::parse_query(&ops.query());
                                let taste = s.taste();
                                for t in tracks {
                                    if s.is_hidden_artist(&t.artist) || !s.allowed(&t) || !glyph_core::taste::matches_phrases(&t, &phrases) || !ops.matches(&t) {
                                        continue;
                                    }
                                    // Ranked by text match, odd versions and the user's
                                    // taste; each song is scored once.
                                    let sc = glyph_core::taste::score(&t, &plain, &taste);
                                    if s.search.seen.insert((t.source, t.id.clone())) {
                                        s.search.insert_ranked(t, sc);
                                    } else {
                                        // Sent again with better data: ranked again.
                                        s.search.replace_ranked(t, sc);
                                    }
                                }
                            }
                            SearchEvent::ProviderError { provider, message } => {
                                s.search.errors.push((provider, message))
                            }
                            SearchEvent::Done => {
                                s.search.loading = false;
                                if !s.search.query.trim().is_empty() {
                                    s.search.unpicked = Some(s.search.query.trim().to_string());
                                }
                                // The top hits are the likely clicks: find
                                // their sources now so they start at once.
                                if let Some(b) = s.backend.clone() {
                                    let top: Vec<Track> = s.search.results.iter().take(2).cloned().collect();
                                    guard(|| b.warm(top));
                                }
                            }
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if done || !alive {
                    break;
                }
            }
        }));
    }

    /// Scrolled near the end of the results: fetch the next page and append
    /// it (sources that can page; stops when a page brings nothing new).
    pub fn search_load_more(&mut self, cx: &mut Context<Self>) {
        if self.offline() { return; }
        let s = &self.search;
        if s.loading || s.loading_more || s.exhausted || s.query.trim().is_empty() || s.results.is_empty() || s.page >= 20 {
            return;
        }
        let Some(b) = self.backend.clone() else { return };
        self.search.page += 1;
        self.search.loading_more = true;
        let (seq, page) = (self.search.seq, self.search.page);
        let (plain, _) = glyph_core::taste::parse_query(&glyph_core::search_ops::SearchOps::parse(self.search.query.trim()).query());
        let Some(mut stream) = guard(|| b.search_page(&plain, page)) else {
            self.search.loading_more = false;
            return;
        };
        cx.notify();
        self.search.more_task = Some(cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            let mut added = 0usize;
            while let Some(ev) = stream.next().await {
                let done = matches!(ev, SearchEvent::Done);
                let alive = this
                    .update(cx, |s, cx| {
                        if s.search.seq != seq {
                            return false;
                        }
                        if let SearchEvent::Results { tracks, .. } = ev {
                            let ops = glyph_core::search_ops::SearchOps::parse(&s.search.query);
                            let (_, phrases) = glyph_core::taste::parse_query(&ops.query());
                            for t in tracks {
                                if s.is_hidden_artist(&t.artist) || !s.allowed(&t) || !glyph_core::taste::matches_phrases(&t, &phrases) || !ops.matches(&t) {
                                    continue;
                                }
                                // Later pages go below what is already shown.
                                if s.search.seen.insert((t.source, t.id.clone())) {
                                    s.search.push_unranked(t);
                                    added += 1;
                                }
                            }
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if done || !alive {
                    break;
                }
            }
            this.update(cx, |s, cx| {
                if s.search.seq == seq {
                    s.search.loading_more = false;
                    s.search.exhausted = added == 0;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    // ------------------------------------------------------------ playback

    /// Artists in the current results, best matches for the query first:
    /// (name, a song of theirs for the picture).
    pub fn search_artists(&self, max: usize) -> Vec<(String, Track)> {
        let q = glyph_core::taste::parse_query(&glyph_core::search_ops::SearchOps::parse(self.search.query.trim()).query()).0.to_lowercase();
        let mut found: Vec<(String, Track, usize)> = Vec::new();
        for tr in &self.search.results {
            let name = primary_artist(&tr.artist);
            if name.is_empty() {
                continue;
            }
            match found.iter_mut().find(|(n, _, _)| n.eq_ignore_ascii_case(&name)) {
                Some(f) => f.2 += 1,
                None => found.push((name, tr.clone(), 1)),
            }
        }
        let score = |n: &str, count: usize| -> i32 {
            let l = n.to_lowercase();
            let m = if l == q {
                1000
            } else if l.starts_with(&q) {
                500
            } else if l.contains(&q) || (!l.is_empty() && q.contains(&l)) {
                200
            } else {
                0
            };
            m + count as i32
        };
        // Only artists that match the query, or that clearly dominate the results.
        found.retain(|(n, _, c)| (score(n, 0) > 0 || *c >= 3) && !self.library.is_hidden(n));
        found.sort_by_key(|(n, _, c)| -score(n, *c));
        found.into_iter().take(max).map(|(n, t, _)| (n, t)).collect()
    }

    /// Query completions taken from the current results (titles and artists
    /// that contain what was typed), for the chips above the results.
    pub fn search_completions(&self, max: usize) -> Vec<String> {
        let q = glyph_core::taste::parse_query(&glyph_core::search_ops::SearchOps::parse(self.search.query.trim()).query()).0.to_lowercase();
        if q.len() < 2 {
            return Vec::new();
        }
        let mut out: Vec<String> = Vec::new();
        for tr in &self.search.results {
            for cand in [tr.title.as_str(), primary_artist(&tr.artist).as_str()] {
                // Drop bracketed extras ("(Radio Edit)", "[Official]") for a clean suggestion.
                let clean = cand.split(['(', '[']).next().unwrap_or(cand).trim().to_string();
                let l = clean.to_lowercase();
                if l.len() <= q.len() || !l.contains(&q) || clean.chars().count() > 40 {
                    continue;
                }
                if !out.iter().any(|o| o.to_lowercase() == l) {
                    out.push(clean);
                }
            }
            if out.len() >= max {
                break;
            }
        }
        out
    }

    /// Runs a search for `q` as if it had been typed.
    pub fn search_for(&mut self, q: String, cx: &mut Context<Self>) {
        self.search_input.update(cx, |i, cx| i.set_value(q.clone(), cx));
        self.on_query(q, true, cx);
    }

    /// Songs for the search dropdown: search history, else recently played.
    pub fn search_suggestions(&self) -> (bool, Vec<glyph_core::SearchItem>) {
        if !self.library.search_items.is_empty() {
            (true, self.library.search_items.iter().take(8).cloned().collect())
        } else {
            (false, self.library.recent_tracks().take(8).map(|t| glyph_core::SearchItem::Song { track: t.clone() }).collect())
        }
    }

    /// The query searched without picking anything goes to recent searches.
    pub fn commit_search_query(&mut self, cx: &mut Context<Self>) {
        let Some(q) = self.search.unpicked.take() else { return };
        if self.library_ok && guard(|| self.library.push_search_query(&q)).is_some() {
            self.save_library(cx);
        }
    }

    /// Open an artist picked from search (remembered in recent searches).
    pub fn pick_search_artist(&mut self, name: &str, track: Option<Track>, window: &mut Window, cx: &mut Context<Self>) {
        self.search.unpicked = None;
        if self.library_ok && guard(|| self.library.push_search_artist(name, track)).is_some() {
            self.save_library(cx);
        }
        let f = self.focus.clone();
        window.focus(&f, cx);
        self.open_artist(name, cx);
    }

    /// Play a song picked from the search dropdown and close it.
    pub fn play_suggestion(&mut self, track: Track, window: &mut Window, cx: &mut Context<Self>) {
        self.search.unpicked = None;
        if self.library_ok {
            let picked = track.clone();
            if guard(|| self.library.push_search(picked)).is_some() {
                self.save_library(cx);
            }
        }
        self.play_list(vec![track], 0, cx);
        let f = self.focus.clone();
        window.focus(&f, cx);
    }

    pub fn remove_search_history(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.library_ok && guard(|| self.library.remove_search(index)).is_some() {
            self.save_library(cx);
        }
        cx.notify();
    }

    /// "Hide explicit songs" is on and `t` is explicit: say so (it does not play).
    pub(super) fn blocked_explicit(&mut self, tr: &Track, cx: &mut Context<Self>) -> bool {
        if self.allowed(tr) {
            return false;
        }
        self.pending_recent_artist = None;
        self.toast_info(t("toast.explicit_blocked").to_string());
        cx.notify();
        true
    }

    /// The open artist page, when playback starts from it: the artist is
    /// remembered in "recently played" alongside the song.
    pub(super) fn artist_recent_context(&self, track: &Track) -> Option<(String, Option<Track>)> {
        (self.view == View::Artist)
            .then(|| self.open_artist.clone())
            .flatten()
            .map(|name| (name, Some(track.clone())))
    }
}

/// Once results have been on screen this long, the first rows stay where
/// they are: a better hit arriving late goes below them instead of pushing
/// away the row that is about to be clicked.
const SEARCH_SETTLE: Duration = Duration::from_millis(800);
const SEARCH_PINNED: usize = 3;

impl SearchState {
    pub(super) fn clear_results(&mut self) {
        self.results.clear();
        self.scores.clear();
        self.shown_at = None;
    }

    /// Rows that keep their place.
    fn pinned(&self) -> usize {
        match self.shown_at {
            Some(at) if at.elapsed() >= SEARCH_SETTLE => SEARCH_PINNED.min(self.results.len()),
            _ => 0,
        }
    }

    /// Inserts `t` (score `sc`) below everything that scores at least as well.
    pub(super) fn insert_ranked(&mut self, t: Track, sc: f32) {
        let at = self.scores.iter().position(|&x| x < sc).unwrap_or(self.scores.len()).max(self.pinned());
        self.results.insert(at, t);
        self.scores.insert(at, sc);
        self.shown_at.get_or_insert_with(Instant::now);
    }

    /// `t` came again (another provider, better data): it replaces its row
    /// and moves to where its new score puts it (a pinned row stays).
    pub(super) fn replace_ranked(&mut self, t: Track, sc: f32) {
        let Some(i) = self.results.iter().position(|x| x.source == t.source && x.id == t.id) else { return };
        if i < self.pinned() {
            self.results[i] = t;
            self.scores[i] = sc;
            return;
        }
        self.results.remove(i);
        self.scores.remove(i);
        self.insert_ranked(t, sc);
    }

    /// A later page: below everything already shown.
    pub(super) fn push_unranked(&mut self, t: Track) {
        self.results.push(t);
        self.scores.push(f32::NEG_INFINITY);
    }

    pub(super) fn retain_results(&mut self, keep: impl Fn(&Track) -> bool) {
        let mut i = 0;
        while i < self.results.len() {
            if keep(&self.results[i]) {
                i += 1;
            } else {
                self.results.remove(i);
                self.scores.remove(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(id: &str) -> Track {
        Track {
            id: id.into(),
            source: ProviderId::Audius,
            title: id.into(),
            artist: "a".into(),
            album: None,
            duration_ms: None,
            cover_url: None,
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false,
        }
    }

    fn ids(s: &SearchState) -> Vec<&str> {
        s.results.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn ranked_results_stay_in_step_and_settle() {
        let mut s = SearchState::default();
        s.insert_ranked(tr("b"), 2.0);
        s.insert_ranked(tr("a"), 3.0);
        s.insert_ranked(tr("c"), 1.0);
        assert_eq!(ids(&s), ["a", "b", "c"]);
        // "c" comes again, now the best match.
        s.replace_ranked(tr("c"), 5.0);
        assert_eq!(ids(&s), ["c", "a", "b"]);
        s.retain_results(|t| t.id != "a");
        assert_eq!(ids(&s), ["c", "b"]);
        assert_eq!(s.scores, [5.0, 2.0]);
        // On screen for a while: a late better hit no longer jumps on top.
        s.shown_at = Some(Instant::now() - SEARCH_SETTLE);
        s.insert_ranked(tr("d"), 9.0);
        assert_eq!(ids(&s), ["c", "b", "d"]);
        s.push_unranked(tr("e"));
        s.insert_ranked(tr("f"), 0.5);
        assert_eq!(ids(&s), ["c", "b", "d", "f", "e"]);
        assert_eq!(s.results.len(), s.scores.len());
    }
}
