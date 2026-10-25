use super::*;

impl Glyph {
    pub(super) fn with_player(&mut self, f: impl FnOnce(&PlayerController)) {
        if let Some(p) = &self.player {
            if guard(|| f(p)).is_none() {
                self.toast_info(t("toast.player_not_ready"));
            }
        } else {
            self.toast_info(t("status.player_offline"));
        }
    }

    /// A song is being found (resolve, yt-dlp) or opened by the player.
    pub fn is_loading(&self) -> bool {
        if self.account.output_target.is_some() { return false; }
        self.resolving || self.player_state.as_ref().is_some_and(|s| s.status == Status::Loading)
    }

    pub fn is_playing(&self) -> bool {
        if self.account.output_target.is_some() { return self.account_playing_hint().unwrap_or_else(|| self.account_output().is_some_and(|o| o.state.playing)); }
        self.player_state.as_ref().is_some_and(|s| s.status == Status::Playing)
    }

    pub fn position(&self) -> Duration {
        if self.account.output_target.is_some() {
            let Some(o) = self.account_output() else { return Duration::ZERO };
            return self.account_output_position(o);
        }
        if self.resolving {
            return Duration::ZERO;
        }
        self.player_state.as_ref().map(|s| s.position).unwrap_or_default()
    }

    /// Position moving smoothly between player reports (embedded players
    /// report only a few times a second); used for lyrics.
    pub fn display_position(&self) -> Duration {
        let base = self.position();
        match self.pos_anchor {
            Some((p, at)) if p == base && self.is_playing() => base + at.elapsed().min(Duration::from_millis(800)),
            _ => base,
        }
    }

    pub fn duration(&self) -> Option<Duration> {
        if self.account.output_target.is_some() { return self.account_output()?.state.duration_ms.map(Duration::from_millis); }
        if self.resolving || self.player_state.as_ref().is_some_and(|s| s.status == Status::Loading) {
            if let Some(d) = self.current.as_ref().and_then(|t| t.duration_ms) {
                return Some(Duration::from_millis(d));
            }
        }
        self.player_state
            .as_ref()
            .and_then(|s| s.duration)
            .or_else(|| self.current.as_ref()?.duration_ms.map(Duration::from_millis))
    }

    pub fn volume(&self) -> f32 {
        if self.account.output_target.is_some() { return self.account_output().map(|o| o.state.volume).unwrap_or(0.0); }
        self.player_state.as_ref().map(|s| s.volume).unwrap_or(0.8)
    }

    pub fn is_liked(&self, t: &Track) -> bool {
        self.library_ok && guard(|| self.library.is_liked(t)).unwrap_or(false)
    }

    pub fn lyrics_offset(&self) -> i64 {
        match &self.current {
            Some(t) if self.library_ok => guard(|| self.library.lyrics_offset(t)).unwrap_or(0),
            _ => 0,
        }
    }

    /// The song "previous" goes back to (the phone's cover carousel).
    pub fn previous_track(&self) -> Option<Track> {
        guard(|| self.queue.history().last().map(|t| (*t).clone())).flatten()
    }

    pub fn upcoming(&self) -> Vec<Track> {
        guard(|| self.queue.upcoming().into_iter().cloned().collect()).unwrap_or_default()
    }

    pub fn shuffle(&self) -> bool {
        guard(|| self.queue.shuffle()).unwrap_or(false)
    }

    pub fn repeat(&self) -> RepeatMode {
        guard(|| self.queue.repeat()).unwrap_or_default()
    }

    /// The window title for the current song when it changed since last set.
    pub(super) fn take_window_title(&mut self) -> Option<(gpui::AnyWindowHandle, String)> {
        let title = match &self.current {
            Some(t) => format!("{} - {}", t.artist, t.title),
            None => "Glyph".to_string(),
        };
        if title == self.window_title {
            return None;
        }
        self.window_title = title.clone();
        Some((self.window_handle, title))
    }

    /// Adds the time the current song played to the statistics (opt-in):
    /// counted while it plays, written every 15 s and when the song changes.
    pub(super) fn tick_stats(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let last = self.stats_tick.replace(now);
        if !self.config.stats.enabled || !self.library_ok || self.account.output_target.is_some() {
            self.stats_pending = None;
            return;
        }
        let playing = self.is_playing() && !self.resolving;
        if let (true, Some(cur), Some(last)) = (playing, self.current.as_ref(), last) {
            // Long gaps (sleep, a stalled frame) are not listening.
            let dt = now.duration_since(last).min(Duration::from_millis(500)).as_millis() as u64;
            let key = track_key(cur);
            match &mut self.stats_pending {
                Some((t, ms)) if track_key(t) == key => *ms += dt,
                _ => {
                    let cur = cur.clone();
                    self.flush_stats(cx);
                    self.stats_pending = Some((cur, dt));
                }
            }
        }
        if self.stats_flushed.elapsed() > Duration::from_secs(15) {
            self.flush_stats(cx);
        }
    }

    pub(crate) fn flush_stats(&mut self, cx: &mut Context<Self>) {
        self.stats_flushed = Instant::now();
        let Some((t, ms)) = self.stats_pending.take() else { return };
        if ms == 0 {
            return;
        }
        self.library.stats.add_time(&t, ms, glyph_core::stats::When::now());
        self.save_library_activity();
        // Other devices see it within a few minutes (not after every song).
        if self.stats_synced.elapsed() > Duration::from_secs(300) {
            self.stats_synced = Instant::now();
            self.schedule_sync(cx);
        }
    }

    /// Opt-in listening statistics on / off. Off deletes them here and (on
    /// the next sync) from the account.
    pub fn set_stats_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.stats.enabled = on;
        self.stats_pending = None;
        if !on && self.library_ok {
            self.library.stats = Default::default();
            self.save_library(cx);
            self.toast_info(t("stats.deleted"));
        }
        self.save_config(cx);
        cx.notify();
    }

    /// The year recap (statistics page, only with statistics on).
    pub fn open_recap(&mut self, year: String, cx: &mut Context<Self>) {
        if !self.config.stats.enabled {
            return;
        }
        if self.view != View::Stats {
            self.open_stats(cx);
        }
        self.recap_year = Some(year);
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        cx.notify();
    }

    pub fn close_recap(&mut self, cx: &mut Context<Self>) {
        self.recap_year = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        cx.notify();
    }

    /// The recap's top songs as a new playlist, opened right away.
    pub fn save_recap_playlist(&mut self, year: &str, tracks: Vec<Track>, cx: &mut Context<Self>) {
        let name = tf("recap.playlist", &[("year", &year)]).to_string();
        match guard(|| self.library.create_playlist(&name).id.clone()) {
            Some(id) => {
                self.save_library(cx);
                self.add_many_to_playlist(&id, tracks, cx);
                self.open_list(ListRef::User(id), cx);
            }
            None => self.toast_info(t("status.library_offline")),
        }
        cx.notify();
    }

    pub fn open_stats(&mut self, cx: &mut Context<Self>) {
        if self.view == View::Stats {
            return;
        }
        self.recap_year = None;
        self.push_route();
        self.view = View::Stats;
        self.selected_row = None;
        self.main_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        self.ctx_menu = None;
        cx.notify();
    }

    /// Is `name` a verified artist? Unknown names are looked up in the
    /// background (a few at a time) and the badge appears once known.
    pub fn artist_verified(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let key = primary_artist(name).to_lowercase();
        if key.is_empty() {
            return false;
        }
        // The open artist page already knows.
        if let Some(v) = self.artists.values().find(|p| p.name.eq_ignore_ascii_case(&key)).and_then(|p| p.stats.as_ref()).map(|s| s.verified) {
            return v;
        }
        match self.verified.get(&key) {
            Some(v) => v.unwrap_or(false),
            None => {
                if self.verified_inflight >= 4 {
                    return false;
                }
                let Some(b) = self.backend.clone() else { return false };
                if self.verified.len() > 2000 {
                    self.verified.retain(|_, v| v.is_some());
                }
                self.verified.insert(key.clone(), None);
                self.verified_inflight += 1;
                let lookup = primary_artist(name);
                cx.spawn(async move |this, cx| {
                    let r = guard_async(b.artist_stats(&lookup)).await;
                    this.update(cx, |s, cx| {
                        s.verified_inflight = s.verified_inflight.saturating_sub(1);
                        let v = matches!(r, Ok(Some(st)) if st.verified);
                        s.verified.insert(key, Some(v));
                        if v {
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
                false
            }
        }
    }

    /// Songs queued by hand that are still to come (top of the queue panel).
    pub fn manual_queue_len(&self) -> usize {
        guard(|| self.queue.manual_len()).unwrap_or(0)
    }

    /// Plays the `i`-th upcoming song (queue panel click) and continues the
    /// queue from there.
    pub fn queue_jump(&mut self, i: usize, cx: &mut Context<Self>) {
        // A jam guest asks the host for the song instead.
        if self.jam_queue_jump(i) {
            cx.notify();
            return;
        }
        let Some(Some(t)) = guard(|| self.queue.jump_to_upcoming(i).cloned()) else { return };
        self.play_track(t, cx);
        cx.notify();
    }

    /// What the queue will play from when started from the current view.
    pub(super) fn context_label(&self) -> Option<String> {
        match self.view {
            View::List => self.open_list.as_ref().map(|l| match l {
                ListRef::ArtistLiked(a) => format!("{} · {}", t("library.liked"), a),
                l => self.list_title(l),
            }),
            View::Artist | View::Releases | View::Similar => self.open_artist.clone(),
            View::Search if !self.search.query.trim().is_empty() => Some(format!("\u{201c}{}\u{201d}", self.search.query.trim())),
            View::Section => self.section.as_ref().map(|(title, _)| title.clone()),
            View::Local => Some(t("local.title").to_string()),
            View::Profile => self.open_profile.clone().map(|h| format!("@{}", h.trim_start_matches('@'))),
            _ => None,
        }
    }

    /// Songs played lately (smart shuffle deals them last).
    pub(super) fn recent_keys(&self) -> Vec<String> {
        self.library.recent_tracks().take(30).map(glyph_core::dedupe::track_key).collect()
    }

    pub fn spinner(&self) -> char {
        const S: [char; 4] = ['|', '/', '-', '\\'];
        S[(self.started.elapsed().as_millis() / 110 % 4) as usize]
    }

    pub fn active_home_tracks(&mut self) -> Vec<Track> {
        let now = glyph_core::paths::now_unix();
        let minute = now / 60;
        if self.home_activity_cache.as_ref().is_none_or(|(at, _, _)| *at != minute) {
            self.home_activity_cache = Some((minute, self.library.active_tracks(500, now), self.library.active_playlists(now)));
        }
        self.home_activity_cache.as_ref().unwrap().1.iter()
            .filter(|track| self.allowed(track)).take(5).cloned().collect()
    }

    /// Own playlists by recent activity (ids, most active first).
    pub fn active_home_playlists(&mut self) -> Vec<String> {
        self.active_home_tracks();
        self.home_activity_cache.as_ref().map(|c| c.2.clone()).unwrap_or_default()
    }

    pub(crate) fn save_library(&mut self, cx: &mut Context<Self>) {
        self.home_activity_cache = None;
        self.activity_dirty = None;
        if !self.library_ok {
            return;
        }
        self.library.checkpoint_playlists();
        let lib = self.library.clone();
        let previous = self.library_save.take();
        self.library_save = Some(cx.background_executor()
            .spawn(async move {
                if let Some(save) = previous { save.await; }
                if let Err(e) = guard_res(|| lib.save()) {
                    tracing::warn!("library save: {e}");
                }
            }));
        self.schedule_sync(cx);
    }

    pub(crate) fn save_library_activity(&mut self) {
        self.home_activity_cache = None;
        if !self.library_ok {
            return;
        }
        self.activity_dirty = Some(Instant::now());
    }

    pub(super) fn flush_activity(&mut self, cx: &mut Context<Self>) {
        let Some(at) = self.activity_dirty else { return };
        if at.elapsed() < ACTIVITY_SAVE_DELAY {
            return;
        }
        self.activity_dirty = None;
        if !self.library_ok {
            return;
        }
        let act = self.library.activity_snapshot();
        let path = glyph_core::Library::activity_path_for(&self.library.storage_path.0.clone().unwrap_or_else(glyph_core::Library::path));
        let previous = self.library_save.take();
        self.library_save = Some(cx.background_executor()
            .spawn(async move {
                if let Some(save) = previous { save.await; }
                if let Err(e) = guard_res(|| act.save_to(&path)) {
                    tracing::warn!("library activity save: {e}");
                }
            }));
        self.schedule_sync(cx);
    }

    /// Saves the config once changes stop for a moment (a slider drag is
    /// one save, not one per step); see [`Glyph::flush_config`].
    /// Data saver on / off (Settings -> Storage).
    pub fn set_data_saver(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.storage.data_saver = on;
        glyph_core::cover::set_data_saver(on);
        self.save_config(cx);
        cx.notify();
    }

    pub fn save_config(&mut self, cx: &mut Context<Self>) {
        self.config_dirty = Some((Instant::now(), true));
        // Settings follow the account (debounced like library changes).
        self.schedule_sync(cx);
    }

    pub fn set_youtube_only(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.config.providers.youtube_only = enabled;
        if enabled && !self.config.providers.enabled.contains(&ProviderId::YouTube) {
            self.config.providers.enabled.push(ProviderId::YouTube);
        }
        self.preloaded = None;
        self.save_config(cx);
        self.push_config_to_backend(cx);
        cx.notify();
    }

    /// Settings > Sources: yt-dlp or the built-in r-dlp finds YouTube audio.
    pub fn set_extractor(&mut self, e: glyph_core::config::Extractor, cx: &mut Context<Self>) {
        if self.config.providers.extractor == e {
            return;
        }
        self.config.providers.extractor = e;
        self.preloaded = None;
        self.save_config(cx);
        self.push_config_to_backend(cx);
        cx.notify();
    }

    pub fn set_youtube_mode(&mut self, mode: glyph_core::config::YouTubeMode, cx: &mut Context<Self>) {
        self.config.providers.youtube_mode = mode;
        self.preloaded = None;
        self.save_config(cx);
        self.push_config_to_backend(cx);
        cx.notify();
    }

    pub fn set_local_search(&mut self, local: bool, cx: &mut Context<Self>) {
        self.config.providers.local_search = local;
        self.save_config(cx);
        self.search.seq += 1;
        self.search.task = None;
        self.search.more_task = None;
        self.search.lyric_task = None;
        self.search.loading = false;
        self.search.loading_more = false;
        let Some(backend) = self.backend.clone() else { cx.notify(); return };
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            match guard_async(backend.set_config(config)).await {
                Ok(_) => {
                    this.update(cx, |s, cx| {
                        if s.config.providers.local_search == local {
                            let query = s.search.query.clone();
                            s.on_query(query, false, cx);
                        }
                    }).ok();
                }
                Err(e) => tracing::warn!("applying search mode: {e}"),
            }
        }).detach();
        cx.notify();
    }

    /// Settings that arrived from another device: apply and save them quietly.
    pub(crate) fn adopt_synced_config(&mut self, config: Config, providers_changed: bool, cx: &mut Context<Self>) {
        self.apply_config(config, cx);
        let toast = self.config_dirty.is_some_and(|(_, t)| t);
        self.config_dirty = Some((Instant::now(), toast));
        if providers_changed {
            // Sources and keys are read when the backend is built.
            self.retry_backend(cx);
        }
    }

    /// Writes a pending config change after [`CONFIG_SAVE_DELAY`] without
    /// further changes. `toast`: a settings change (not player prefs).
    pub(super) fn flush_config(&mut self, cx: &mut Context<Self>) {
        let Some((at, toast)) = self.config_dirty else { return };
        if at.elapsed() < CONFIG_SAVE_DELAY {
            return;
        }
        self.config_dirty = None;
        let cfg = self.config.clone();
        if !toast {
            cx.background_executor()
                .spawn(async move {
                    if let Err(e) = guard_res(|| cfg.save()) {
                        tracing::warn!("saving player prefs: {e}");
                    }
                })
                .detach();
            return;
        }
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_executor()
                .spawn(async move { guard_res(|| cfg.save()) })
                .await;
            this.update(cx, |s, cx| {
                match r {
                    Ok(()) => s.toast_info(t("toast.config_saved")),
                    Err(e) => s.toast_info(tf("toast.config_error", &[("e", &e)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
