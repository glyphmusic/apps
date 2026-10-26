use super::*;

#[derive(Default)]
pub struct OfflineState {
    pub offline: bool,
    checked: Option<Instant>,
    checking: bool,
    pub audio: glyph_core::OfflineAudio,
}

impl Glyph {
    pub fn offline(&self) -> bool { self.offline.offline }

    pub fn available_offline(&self, track: &Track) -> bool {
        self.is_downloaded(track) || self.offline.audio.contains(track)
    }

    pub fn playable(&self, track: &Track) -> bool {
        self.allowed(track) && (!self.offline() || self.account.output_target.is_some() || self.remote_chosen() || self.available_offline(track))
    }

    fn set_online(&mut self, online: bool, cx: &mut Context<Self>) {
        let offline = !online;
        if self.offline.offline == offline { return; }
        self.offline.offline = !online;
        if online {
            if !self.search.query.trim().is_empty() {
                self.on_query(self.search.query.clone(), false, cx);
            }
        } else {
            if self.queue.peek_next().is_some_and(|t| !self.available_offline(t)) {
                self.preloaded = None;
                self.preload_attempt = None;
                self.with_player(|p| p.drop_next());
            }
            self.search.seq += 1;
            self.search.task = None;
            self.search.lyric_task = None;
            self.search.more_task = None;
            self.search.loading = false;
            self.search.loading_more = false;
            self.search.errors.clear();
            self.search.error = None;
        }
        cx.notify();
    }

    pub(super) fn search_offline(&mut self, query: &str) {
        let downloads = self.backend.as_ref().map(|b| b.downloaded_snapshot()).unwrap_or_default();
        let candidates = self.search.results.iter()
            .chain(self.library.liked.iter())
            .chain(self.library.recent_tracks())
            .chain(self.library.playlists.iter().flat_map(|p| p.tracks.iter()))
            .chain(downloads.iter());
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut seen = HashSet::new();
        let results: Vec<_> = candidates.filter(|track| self.available_offline(track) && self.allowed(track)).filter(|track| {
            let text = format!("{} {} {}", track.title, track.artist, track.album.as_deref().unwrap_or("")).to_lowercase();
            terms.iter().all(|t| text.contains(t)) && seen.insert(track_key(track))
        }).cloned().collect();
        self.search.clear_results();
        for track in results { self.search.push_unranked(track); }
        self.search.errors.clear();
        self.search.error = None;
        self.search.lyric_hits.clear();
        self.search.loading = false;
        self.search.loading_more = false;
        self.search.exhausted = true;
    }

    pub(super) fn poll_connectivity(&mut self, cx: &mut Context<Self>) {
        if !crate::ui::compact::TOUCH { return; }
        let platform = glyph_core::connectivity::platform_online();
        let changed = platform.is_some_and(|online| self.offline.offline == online);
        if let Some(online) = platform {
            if changed {
                self.set_online(online, cx);
                self.offline.checked = None;
                cx.notify();
            }
        }
        if self.offline.checking || self.offline.checked.is_some_and(|at| at.elapsed() < Duration::from_secs(15)) { return; }
        let Some(backend) = self.backend.clone() else { return };
        self.offline.checking = true;
        self.offline.checked = Some(Instant::now());
        cx.spawn(async move |this, cx| {
            let online = backend.internet_available().await;
            let audio = cx.background_executor().spawn(async { glyph_core::OfflineAudio::scan() }).await;
            this.update(cx, |s, cx| {
                // Prefer the latest OS callback to a probe started before a
                // network switch. Never infer connectivity from server health.
                let online = glyph_core::connectivity::platform_online().unwrap_or(online);
                s.offline.audio = audio;
                s.set_online(online, cx);
                s.offline.checking = false;
                cx.notify();
            }).ok();
        }).detach();
    }

    pub(super) fn blocked_offline(&mut self, track: &Track, cx: &mut Context<Self>) -> bool {
        if self.offline() && !self.playable(track) && self.allowed(track) {
            self.toast_info(t("offline.unavailable"));
            cx.notify();
            true
        } else { false }
    }
}
