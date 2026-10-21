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
            };
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
        self.search.query = q.clone();
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
            self.search.results.clear();
            self.search.seen.clear();
            self.search.errors.clear();
            self.search.error = None;
            cx.notify();
            return;
        }
        self.search.loading = true;
        cx.notify();
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
                    s.search.results.clear();
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
                                    if s.search.seen.insert((t.source, t.id.clone())) {
                                        // Ranked insert: text match, odd versions, the user's taste.
                                        let sc = glyph_core::taste::score(&t, &plain, &taste);
                                        let at = s
                                            .search
                                            .results
                                            .iter()
                                            .position(|x| glyph_core::taste::score(x, &plain, &taste) < sc)
                                            .unwrap_or(s.search.results.len());
                                        s.search.results.insert(at, t);
                                    } else if let Some(x) = s.search.results.iter_mut().find(|x| x.source == t.source && x.id == t.id) {
                                        *x = t;
                                    }
                                }
                            }
                            SearchEvent::ProviderError { provider, message } => {
                                s.search.errors.push((provider, message))
                            }
                            SearchEvent::Done => {
                                s.search.loading = false;
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
                                    s.search.results.push(t);
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

    /// Open an artist picked from search (remembered in recent searches).
    pub fn pick_search_artist(&mut self, name: &str, track: Option<Track>, window: &mut Window, cx: &mut Context<Self>) {
        if self.library_ok && guard(|| self.library.push_search_artist(name, track)).is_some() {
            self.save_library(cx);
        }
        let f = self.focus.clone();
        window.focus(&f, cx);
        self.open_artist(name, cx);
    }

    /// Play a song picked from the search dropdown and close it.
    pub fn play_suggestion(&mut self, track: Track, window: &mut Window, cx: &mut Context<Self>) {
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
