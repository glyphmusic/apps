use super::*;

impl Glyph {
    // ------------------------------------------------------------ polling

    pub(super) fn poll(&mut self, cx: &mut Context<Self>) {
        self.ensure_account_library(cx);
        self.poll_account_outputs(cx);
        if self.download_check_at.elapsed() >= Duration::from_secs(60) {
            self.download_check_at = Instant::now();
            if let Some(backend) = &self.backend { backend.refresh_downloads(); }
        }
        let mut dirty = false;
        self.save_player_prefs(cx);
        self.tick_stats(cx);
        self.tick_sleep(cx);
        self.announce_outage();
        // AI assistants (MCP): calls that arrived, and every few seconds the
        // server / relay state (sign-in changes start or stop the relay).
        if self.poll_mcp(cx) {
            dirty = true;
        }
        if self.poll_jam(cx) {
            dirty = true;
        }
        if self.mcp_checked.elapsed() > Duration::from_secs(5) && self.config_loaded {
            self.mcp_checked = Instant::now();
            if self.config.mcp.enabled || self.mcp.relay_running {
                self.sync_mcp(cx);
            }
        }
        self.flush_activity(cx);
        if let Some(p) = self.player.clone() {
            let embed_playing = self.embed_playing();
            if embed_playing {
                self.embed.attach(&p);
            }
            // Touch the webview only while an embed is involved, so normal
            // (native) playback never triggers child-window messages.
            if embed_playing || self.embed.is_visible() {
                let vis = self.view == View::NowPlaying && embed_playing;
                self.embed.set_visible(&p, vis);
                self.embed.pump(&p);
            }
        }

        if self.poll_discovery() {
            dirty = true;
        }
        if self.account.output_target.is_some() {
            // Only the receiving app records listens and advances its player.
        } else if self.jam_listen_remote() {
            // Listening at the jam host's device: the host's song and position.
            if self.jam_remote_state() {
                dirty = true;
            }
        } else if self.remote_on() {
            if self.poll_mpd(cx) {
                dirty = true;
            }
        } else if let (Some(p), true) = (self.player.clone(), self.player_ok) {
            match guard(|| p.state_if_changed(self.player_state.as_ref())) {
                Some(Some(st)) => {
                    if self.player_state.as_ref() != Some(&st) {
                        let playing = st.status == Status::Playing;
                        if playing && !self.activity_recorded && !self.resolving && self.library_ok {
                            if let Some(track) = self.current.clone() {
                                let threshold = track.duration_ms.map(|ms| (ms / 2000).clamp(5,30)).unwrap_or(30);
                                if st.position.as_secs() >= threshold && st.track.as_ref().is_some_and(|t| glyph_core::dedupe::same_song(t,&track)) {
                                    self.library.record_listen(&track, glyph_core::paths::now_unix());
                                    if self.config.stats.enabled {
                                        self.library.stats.add_play(&track, glyph_core::stats::When::now());
                                    }
                                    if let Some(l) = self.playing_from.clone() {
                                        self.library.record_list_use(&l.key(), 1.0, glyph_core::paths::now_unix());
                                    }
                                    self.activity_recorded = true;
                                    self.save_library_activity();
                                }
                            }
                        }
                        self.glow.update(cx, |g, _| g.set_playing(playing));
                        if let Some(mk) = &self.media_keys {
                            // The OS media controls only need track, play state
                            // and a coarse position; pushing every state change
                            // (~60/s) means needless OS calls on the UI thread.
                            let key = (
                                st.track.as_ref().map(|t| (t.source, t.id.clone())),
                                playing,
                                st.position.as_secs() / 5,
                            );
                            if self.media_keys_sent.as_ref() != Some(&key) {
                                let pos = st.position;
                                let track = st.track.clone();
                                guard(|| mk.set_now_playing(track.as_ref(), playing, pos));
                                self.media_keys_sent = Some(key);
                            }
                        }
                        if self.discord.is_some() {
                            // Discord already counts the elapsed time from the
                            // start timestamp; a coarse position (and a resend on
                            // seeks) is enough. 15 s keeps us far below its
                            // activity rate limit.
                            let key = (
                                st.track.as_ref().map(|t| (t.source, t.id.clone())),
                                playing,
                                st.position.as_secs() / 15,
                            );
                            if self.discord_sent.as_ref() != Some(&key) {
                                if let Some(d) = &self.discord {
                                    d.set(st.track.as_ref(), playing, st.position);
                                }
                                self.discord_sent = Some(key);
                            }
                        }
                        if self.pos_anchor.map(|a| a.0) != Some(st.position) {
                            self.pos_anchor = Some((st.position, Instant::now()));
                        }
                        // Only the position moved: redraw when the bar or the
                        // clock can show it (every 250 ms), not ~60 times a
                        // second; the whole window redrawing that often made
                        // dragging the window stutter. Now Playing animates
                        // on its own (see `is_animating`).
                        let step = |p: Duration| p.as_millis() / 250;
                        let visible = match &self.player_state {
                            Some(old) => {
                                let mut same = old.clone();
                                same.position = st.position;
                                same != st || step(old.position) != step(st.position)
                            }
                            None => true,
                        };
                        self.player_state = Some(st);
                        if visible {
                            dirty = true;
                        }
                    }
                }
                Some(None) => {},
                None => self.player_ok = false,
            }
        }
        // A-B loop: back to the start point once the end point is passed.
        if self.loop_ab != (None, None) && self.current.as_ref().map(track_key) != self.loop_song {
            self.loop_ab = (None, None);
            dirty = true;
        }
        // Native playback loops (with a fade) in the player itself; embedded
        // players are sent back from here.
        let embed = self.embed_playing();
        if let (Some(a), Some(b)) = self.loop_ab {
            if embed && self.current.is_some() && self.position() >= b {
                self.seek_to(a);
                dirty = true;
            }
        }
        let want = match self.loop_ab {
            (Some(a), Some(b)) if !embed => Some((a, b)),
            _ => None,
        };
        if want != self.loop_sent {
            self.loop_sent = want;
            if let Some(p) = &self.player {
                p.set_loop(want);
            }
        }
        let events: Vec<PlayerEvent> = self
            .player_events
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for ev in events {
            dirty = true;
            match ev {
                PlayerEvent::Ended => self.next(true, cx),
                PlayerEvent::NearEnd => self.preload_next(cx),
                PlayerEvent::Error(e) => {
                    self.finish_account_transfer(Some(e.clone()), cx);
                    // Preview URLs are signed and expire after ~15 min. If the
                    // one we are holding has expired, resolve once more (which
                    // fetches a fresh URL) and replay instead of just failing.
                    if self.retry_expired_preview(&e, cx) {
                        tracing::info!("expired preview URL, re-resolving once");
                    } else if self.repair_broken_file(&e, cx) {
                        tracing::info!("broken local file dropped, playing online: {e}");
                    } else {
                        self.toast_info(tf("toast.player_error", &[("e", &e)]));
                    }
                }
                PlayerEvent::StatusChanged(_) => {}
            }
        }
        // Resolve and buffer the next queue item while this song plays,
        // leaving enough time for a cold companion instead of waiting to EOF.
        if self.is_playing() && !self.resolving && !self.embed_playing() {
            self.preload_next(cx);
        }
        let keys: Vec<MediaKeyEvent> = self
            .media_events
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for k in keys {
            dirty = true;
            match k {
                MediaKeyEvent::Play if self.remote_on() || self.account.output_target.is_some() => {
                    if !self.is_playing() {
                        self.toggle_play(cx)
                    }
                }
                MediaKeyEvent::Pause if self.remote_on() || self.account.output_target.is_some() => {
                    if self.is_playing() {
                        self.toggle_play(cx)
                    }
                }
                MediaKeyEvent::Play => self.with_player(|p| p.play()),
                MediaKeyEvent::Pause => self.with_player(|p| p.pause()),
                MediaKeyEvent::Toggle => self.toggle_play(cx),
                MediaKeyEvent::Next => self.next(false, cx),
                MediaKeyEvent::Previous => self.previous(cx),
                MediaKeyEvent::Stop => {
                    if !self.account_control(glyph_protocol::playback::Command::Stop) { self.with_player(|p| p.stop()); }
                }
                MediaKeyEvent::SeekForward => self.seek_relative(5.0),
                MediaKeyEvent::SeekBackward => self.seek_relative(-5.0),
            }
        }

        // Lyric line changes pulse the glow.
        if let (Some(l), Some(st)) = (&self.lyrics.lyrics, &self.player_state) {
            let off = self.lyrics_offset();
            let pos = st.position.as_millis() as u64;
            let line = guard(|| l.active_line(pos, off)).flatten();
            if line != self.lyrics.last_line {
                self.lyrics.last_line = line;
                if line.is_some() {
                    self.glow.update(cx, |g, _| g.pulse());
                }
                dirty = true;
            }
        }

        // Adaptive text ink: ease towards the better-contrast colour (~300 ms).
        crate::theme::set_lum_grid(self.glow.read(cx).lum_grid());
        if crate::theme::update_inks() {
            dirty = true;
        }
        // Requests from another launch (jump list click, second start).
        #[cfg(windows)]
        {
            for id in crate::thumbbar::take_clicks() {
                match id {
                    crate::thumbbar::LIKE => {
                        if let Some(cur) = self.current.clone() {
                            self.toggle_like(&cur, cx);
                        }
                    }
                    crate::thumbbar::PREV => self.previous(cx),
                    crate::thumbbar::PLAY => self.toggle_play(cx),
                    crate::thumbbar::NEXT => self.next(false, cx),
                    _ => {}
                }
                dirty = true;
            }
            let liked = self.current.as_ref().is_some_and(|c| self.is_liked(c));
            crate::thumbbar::set_state(self.is_playing(), liked);
        }
        if self.backend.is_some() {
            if let Some(link) = self.pending_share_link.take() {
                self.import_share_code(&link, cx);
            }
        }
        while let Some(req) = crate::instance::try_recv() {
            if let Some(n) = req.strip_prefix("dock ").and_then(|n| n.parse::<usize>().ok()) {
                self.dock_action(n, cx);
            }
            if let Some(code) = req.strip_prefix("jam ") {
                self.jam_open_link(code, cx);
            }
            if let Some(link) = req.strip_prefix("open ") {
                if self.backend.is_some() { self.import_share_code(link, cx); }
                else { self.pending_share_link = Some(link.to_string()); }
            }
            cx.activate(true);
        }
        self.update_dock_menu(cx);
        let scroll = -f32::from(self.main_scroll.offset().y);
        self.glow.update(cx, |g, cx| {
            if g.set_scroll(scroll) {
                cx.notify();
            }
        });
        let zones = self.glow.read(cx).zones();
        for (ink, lum) in self.ink.iter_mut().zip(zones) {
            let target = crate::theme::ink_for(lum);
            if (*ink - target).abs() > 0.001 {
                let step = 0.016 / 0.3;
                *ink = if target > *ink { (*ink + step).min(target) } else { (*ink - step).max(target) };
                dirty = true;
            }
        }
        if self.settings_closing.is_some_and(|at| at.elapsed() > Duration::from_millis(160)) {
            self.settings_closing = None;
            dirty = true;
        }
        let before = self.toasts.len();
        let held = self.toast_drag.map(|(id, _)| id);
        let hovered = self.toast_hover;
        self.toasts.retain(|t| match t.gone {
            Some(g) => g.elapsed().as_secs_f32() < crate::ui::toasts::GONE_S,
            None => hovered || held == Some(t.id) || t.at.elapsed() <= TOAST_FOR,
        });
        if self.toasts.len() != before {
            dirty = true;
        }
        if self.is_animating() || crate::ui::widgets::take_skeleton_seen() {
            dirty = true;
        }
        // A playing-song equalizer on screen moves at ~15 fps (a full redraw
        // per display frame would be wasted on four little bars).
        if crate::ui::widgets::take_eq_seen() {
            if self.eq_redraw.elapsed() >= Duration::from_millis(66) {
                self.eq_redraw = Instant::now();
                dirty = true;
            } else {
                // Not drawn this tick: keep the flag for the next one.
                crate::ui::widgets::keep_eq_seen();
            }
        }
        if dirty {
            cx.notify();
        }
    }

    pub fn is_animating(&self) -> bool {
        let toast_fading = crate::ui::toasts::animating(self);
        let settings_anim = (self.settings_open && self.settings_at.elapsed() < Duration::from_millis(220)) || self.settings_closing.is_some();
        let nav_anim = self.nav_hover.is_some_and(|(v, at)| v == View::NowPlaying || at.elapsed() < Duration::from_millis(700))
            || self.nav_leave.is_some_and(|(_, at, _)| at.elapsed().as_secs_f32() < crate::ui::chrome::NAV_EASE_BACK + 0.05);
        let menu_anim = (self.ctx_menu.is_some() && self.ctx_opened.elapsed() < Duration::from_millis(200)) || self.ctx_closing.is_some();
        let tip_anim = self.side_hover.is_some_and(|(_, at)| at.elapsed() < Duration::from_millis(600));
        let info_anim = self.lyrics_info.is_some_and(|at| at.elapsed() < Duration::from_millis(300));
        toast_fading
            || crate::ui::widgets::reveal_active()
            || info_anim
            || !self.downloading.is_empty()
            || self.download_done.values().any(|at| at.elapsed().as_secs_f32() < crate::ui::widgets::DL_DONE_ANIM_S)
            || (self.settings_open && self.settings_group_motion.values().any(|(at,_,_)| at.elapsed() < Duration::from_millis(180)))
            || settings_anim
            || crate::ui::settings::select_animating()
            || crate::ui::settings::flash_animating()
            || tip_anim
            || menu_anim
            || self.splash.is_some()
            || nav_anim
            || self.bar_slide.running()
            || self.queue_slide.running()
            || crate::ui::widgets::press_running()
            || crate::ui::views::queue_anim_running()
            || crate::ui::widgets::drag_shift_running()
            || self.search.loading
            || self.is_loading()
            || self.lyrics.loading
            || self.lyrics.alts_loading
            || self.update.checking
            || self.update.installing.is_some()
            || matches!(self.conn_test, Some(ConnTest::Running))
            || self.backend_state == BackendState::Connecting
            || ((self.view == View::NowPlaying || crate::ui::compact::now_playing::is_open()) && self.is_playing())
    }

    // ------------------------------------------------------------ helpers

    /// The official server went out (offline, or Cloudflare's daily limit):
    /// says once that search and playback run locally in the meantime. No
    /// internet on this device is announced as that, without a switch.
    fn announce_outage(&mut self) {
        use glyph_core::outage::{self, Kind};
        let Some(o) = outage::current() else { return };
        if o.seq == self.outage_seen || !outage::is_official(self.config.effective_server_url()) { return; }
        self.outage_seen = o.seq;
        let (key, sub) = match o.kind {
            Kind::Offline => ("toast.server_out.offline", "toast.server_out.local"),
            Kind::DailyLimit => ("toast.server_out.limit", "toast.server_out.local"),
            Kind::NoInternet => ("toast.no_internet", "toast.no_internet.sub"),
        };
        self.push_toast(format!("{} · {}", t(key), t(sub)), ToastIcon::Info);
    }

    /// Confirmation toast. "Title · detail" is split at the first dot.
    pub fn toast(&mut self, msg: impl Into<String>) {
        self.push_toast(msg.into(), ToastIcon::Check);
    }

    /// Neutral / problem toast (info icon instead of the check).
    pub fn toast_info(&mut self, msg: impl Into<String>) {
        self.push_toast(msg.into(), ToastIcon::Info);
    }

    /// Toast with an explicit detail part.
    pub fn toast_sub(&mut self, title: impl Into<String>, sub: impl Into<String>) {
        self.push_toast(format!("{} · {}", title.into(), sub.into()), ToastIcon::Check);
    }

    pub(super) fn push_toast(&mut self, msg: String, icon: ToastIcon) {
        tracing::info!("{msg}");
        let (title, sub) = match msg.split_once(" · ") {
            Some((a, b)) if !b.trim().is_empty() => (a.to_string(), Some(b.to_string())),
            _ => (msg, None),
        };
        let now = Instant::now();
        // The same message again (e.g. one error repeating): bring the
        // existing toast to the front instead of stacking copies.
        if let Some(i) = self.toasts.iter().position(|t| t.gone.is_none() && t.title == title && t.sub == sub) {
            let mut t = self.toasts.remove(i);
            t.at = now;
            self.toasts.insert(0, t);
            return;
        }
        self.toast_seq += 1;
        self.toasts.insert(
            0,
            Toast {
                id: self.toast_seq,
                title,
                sub,
                icon,
                at: now,
                y: f32::NAN,
                target: 0.0,
                drag: 0.0,
                depth: 0.0,
                depth_target: 0.0,
                gone: None,
            },
        );
        // Over the limit: the oldest visible ones swipe away.
        let mut live = 0;
        for t in self.toasts.iter_mut() {
            if t.gone.is_none() {
                live += 1;
                if live > MAX_TOASTS {
                    t.gone = Some(now);
                }
            }
        }
    }

    /// Hands a found song to where it plays: this computer, MPD or a
    /// network device (see `net_devices`).
    pub(super) fn output_load(&mut self, tr: Track, src: glyph_protocol::PlaybackSource, start: bool) {
        self.output_load_at(tr, src, start, 0.0);
    }

    /// Starts / stops the MPD output as the settings say.
    pub fn sync_mpd(&mut self, cx: &mut Context<Self>) {
        let want = self.config.mpd.enabled.then(|| crate::mpd::Address::parse(&self.config.mpd.address)).flatten();
        if want.as_ref() == self.mpd.as_ref().map(|m| &m.address) {
            return;
        }
        if let Some(old) = self.mpd.take() {
            old.send(crate::mpd::Cmd::Stop);
        }
        self.mpd_was_playing = false;
        if let Some(a) = want {
            // One remote at a time.
            self.drop_device_output();
            self.local_fallback = false;
            tracing::info!(host = %a.host, port = a.port, "MPD output on");
            self.with_player(|p| p.pause());
            self.mpd = Some(crate::mpd::Output::start(a));
        }
        cx.notify();
    }

    /// Settings: play on MPD instead of this computer.
    pub fn set_mpd_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.mpd.enabled = on;
        self.save_config(cx);
        self.sync_mpd(cx);
    }

    /// Player state from MPD's (or the network device's) status; its song
    /// ending moves the queue on.
    pub(super) fn poll_mpd(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(st) = self.remote_status() else { return false };
        self.note_device_failure(&st.state);
        let status = match st.state.as_str() {
            "play" => Status::Playing,
            "pause" => Status::Paused,
            "load" => Status::Loading,
            _ if self.resolving => Status::Loading,
            _ => Status::Idle,
        };
        let playing = status == Status::Playing;
        let state = PlayerState {
            status,
            track: self.current.clone(),
            provider: self.current_source,
            backend: Some(glyph_audio::BackendKind::Native),
            is_preview: self.current_preview,
            position: Duration::from_secs_f64(st.elapsed.unwrap_or(0.0).max(0.0)),
            duration: st.duration.filter(|d| *d > 0.0).map(Duration::from_secs_f64),
            volume: st.volume.map_or(self.volume(), |v| v as f32 / 100.0),
        };
        let ended = self.mpd_was_playing && st.state == "stop" && !self.resolving;
        self.mpd_was_playing = playing;
        let changed = self.player_state.as_ref() != Some(&state);
        if changed {
            if self.pos_anchor.map(|a| a.0) != Some(state.position) {
                self.pos_anchor = Some((state.position, Instant::now()));
            }
            self.player_state = Some(state);
        }
        if ended {
            self.next(true, cx);
        }
        changed || ended
    }
}
