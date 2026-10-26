use super::*;

impl Glyph {
    /// Replace the queue with `tracks` and start at `index`. With "Hide
    /// explicit songs" on, explicit songs neither start nor join the queue.
    pub fn play_list(&mut self, tracks: Vec<Track>, index: usize, cx: &mut Context<Self>) {
        if self.account.output_target.is_some() {
            if index < tracks.len() {
                self.account_control(glyph_protocol::playback::Command::Transfer { tracks: tracks[index..].iter().take(100).cloned().collect(), index: 0, position_ms: 0, playing: true });
            }
            return;
        }
        let Some(track) = tracks.get(index).cloned() else { return };
        if self.blocked_offline(&track, cx) { return; }
        self.skip_streak = 0;
        if self.jam_request(&track, true) {
            return;
        }
        if self.blocked_explicit(&track, cx) {
            return;
        }
        let index = tracks[..index].iter().filter(|t| self.playable(t)).count();
        let tracks: Vec<Track> = tracks.into_iter().filter(|t| self.playable(t)).collect();
        let recent = self.recent_keys();
        if guard(|| {
            self.queue.set_recent(recent);
            self.queue.set(tracks, index)
        })
        .is_none()
        {
            self.queue_ok = false;
        }
        // Started from an open list: its listening counts for the sidebar.
        self.playing_from = if self.view == View::List { self.open_list.clone() } else { None };
        self.queue_from = self.context_label();
        self.recent_artist_ctx = (self.view == View::Artist).then(|| self.open_artist.clone()).flatten();
        self.touch_artist_page(&track);
        if let Some(l) = self.playing_from.clone() {
            self.touch_list(&l, 0.5);
        }
        self.pending_recent_artist = self.artist_recent_context(&track);
        self.play_track(track, cx);
    }

    /// A song row was clicked (or Enter pressed on it): plays `tracks` from
    /// `index`. A song picked from the search results is remembered for the
    /// search dropdown; radio, play buttons and the like never land there.
    pub fn play_row(&mut self, tracks: Vec<Track>, index: usize, cx: &mut Context<Self>) {
        if self.view == View::Search {
            self.search.unpicked = None;
        }
        if self.view == View::Search && self.library_ok {
            if let Some(picked) = tracks.get(index).filter(|t| self.allowed(t)).cloned() {
                if guard(|| self.library.push_search(picked)).is_some() {
                    self.save_library(cx);
                }
            }
        }
        self.play_list(tracks, index, cx);
    }

    pub fn play_track(&mut self, track: Track, cx: &mut Context<Self>) {
        if self.account_control(glyph_protocol::playback::Command::Transfer { tracks: vec![track.clone()], index: 0, position_ms: 0, playing: true }) { return; }
        if self.jam_request(&track, true) {
            return;
        }
        if self.blocked_offline(&track, cx) { return; }
        if self.blocked_explicit(&track, cx) {
            return;
        }
        self.skip_streak = 0;
        self.start_track(track, None, cx);
    }

    /// `adopted`: the player already moved on to this song gaplessly (as the
    /// given resolved track), so only the UI follows; nothing is reloaded.
    pub(super) fn start_track(&mut self, track: Track, adopted: Option<Track>, cx: &mut Context<Self>) {
        self.account.transfer_resume = None;
        // The sleep timer faded the last song out and paused: back to normal.
        if self.sleep.is_none() {
            self.end_sleep_fade();
        }
        let key = track_key(&track);
        self.preload_attempt = None;
        self.current = Some(track.clone());
        self.activity_recorded = false;
        self.current_source = Some(adopted.as_ref().map_or(track.source, |a| a.source));
        self.current_preview = adopted.is_some() && self.player_state.as_ref().is_some_and(|s| s.is_preview);
        self.retried_expired = None;
        self.repaired_file = None;
        self.resolving = adopted.is_none();
        self.hold_play = false;
        if adopted.is_none() {
            self.preloaded = None;
            // Fade the old song out right away instead of letting it run
            // until the new one has loaded.
            self.with_player(|p| p.pause());
        }
        let artist_ctx = self.pending_recent_artist.take();
        // Played from an artist's page: the artist entry in "recently played"
        // stands for their songs (no row per song on top of it).
        let from_artist = self.recent_artist_ctx.as_ref().is_some_and(|a| by_artist(a, &track.artist))
            && guard(|| self.queue.current_is_manual()) != Some(true);
        if self.library_ok {
            let logged = if from_artist { Some(()) } else { guard(|| self.library.push_recent(track.clone())) };
            if logged.is_some() {
                self.library.record_play(&track);
                if let Some((name, art)) = artist_ctx {
                    guard(|| self.library.push_recent_artist(&name, art));
                }
                self.save_library_activity();
                self.profile_recent_changed(cx);
            }
        }
        let seed = format!("{}{}", track.artist, track.title);
        let palette = Palette::from_seed(&seed);
        self.palette = Some(palette);
        let preset = self.glow_shape(Preset::from_seed(&seed));
        self.glow.update(cx, |g, _| {
            g.set_palette(Some(palette));
            g.set_preset(preset);
        });
        // The player bar and Now Playing cover "ASCII in" again with every new
        // song, also when that cover was already shown somewhere before.
        crate::ui::widgets::reset_reveals(&format!("bar-meta:{key}"));
        for size in [crate::ui::player_bar::COVER, crate::ui::now_playing::COVER] {
            crate::ui::widgets::reset_reveals(&format!("cover:{key}:{size}"));
        }
        self.cover = Some(CoverArt {
            key: key.clone(),
            path: None,
            ascii: ascii::pattern(&seed, 34, 17),
        });
        self.lyrics = LyricsState {
            key: Some(key.clone()),
            loading: true,
            ..Default::default()
        };
        cx.notify();

        let Some(backend) = self.backend.clone() else {
            self.resolving = false;
            self.lyrics.loading = false;
            self.toast_info(t("status.backend_offline"));
            return;
        };

        // Resolve → load → play.
        if adopted.is_some() {
            self.prefetch_upcoming();
        } else {
            let b = backend.clone();
            let t = track.clone();
            let k = key.clone();
            let owner_path = self.library.storage_path.0.clone();
            cx.spawn(async move |this, cx| {
                let started = Instant::now();
                tracing::info!(provider = ?t.source, "playback: resolving source");
                let r = guard_async(b.resolve(&t)).await;
                tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, success = r.is_ok(), "playback: source resolved");
                this.update(cx, |s, cx| {
                    if s.account.output_target.is_some() || s.library.storage_path.0 != owner_path { return; }
                    if s.current.as_ref().map(track_key).as_deref() != Some(k.as_str()) {
                        return;
                    }
                    s.resolving = false;
                    match r {
                        Ok(Some(res)) => {
                            s.current_source = Some(res.source.provider());
                            s.current_preview =
                                matches!(res.source, glyph_protocol::PlaybackSource::Preview { .. });
                            let src = res.source.clone();
                            let tr = res.track.clone();
                            let start = !s.hold_play;
                            if let Some((ms, playing)) = s.account.transfer_resume.take() {
                                s.output_load_at(tr, src, playing, ms as f64 / 1000.0);
                            } else {
                                s.output_load(tr, src, start);
                            }
                            // Now that this song runs, get the next ones ready
                            // (local yt-dlp only; one batch run, not one wait per song).
                            s.prefetch_upcoming();
                        }
                        Ok(None) => {
                            s.finish_account_transfer(Some("No playable source on target device".into()), cx);
                            let transfer = s.account.transfer_resume.take().is_some();
                            s.jam_no_source(&k);
                            if transfer || !s.skip_unplayable(cx) {
                                s.toast_info(crate::i18n::t("toast.no_source"));
                            }
                        }
                        Err(e) => {
                            s.finish_account_transfer(Some(e.to_string()), cx);
                            let transfer = s.account.transfer_resume.take().is_some();
                            if transfer || !s.skip_unplayable(cx) {
                                s.toast_info(tf("toast.resolve_error", &[("e", &e)]));
                            } else {
                                tracing::warn!(error = %e, "playback: resolve failed, skipped");
                            }
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }

        // Lyrics.
        let b = backend.clone();
        let t = track.clone();
        let k = key.clone();
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.lyrics(&t)).await;
            this.update(cx, |s, cx| {
                if s.lyrics.key.as_deref() != Some(k.as_str()) {
                    return;
                }
                s.lyrics.loading = false;
                match r {
                    Ok(l) => s.lyrics.lyrics = l,
                    Err(e) => s.lyrics.error = Some(e),
                }
                s.refresh_plugin_lyrics();
                cx.notify();
            })
            .ok();
        })
        .detach();

        // Cover → palette + ASCII art.
        let b = backend;
        let t = track;
        cx.spawn(async move |this, cx| {
            let Ok(Some(path)) = guard_async(b.cover(&t)).await else { return };
            let p2 = path.clone();
            let (pal, art) = cx
                .background_executor()
                .spawn(async move { (Palette::from_image(&p2), ascii::from_image(&p2, 34, 17)) })
                .await;
            this.update(cx, |s, cx| {
                if s.current.as_ref().map(track_key).as_deref() != Some(key.as_str()) {
                    return;
                }
                if let Some(p) = pal {
                    s.palette = Some(p);
                    s.glow.update(cx, |g, _| g.set_palette(Some(p)));
                }
                s.cover = Some(CoverArt {
                    key: key.clone(),
                    path: Some(path),
                    ascii: art.unwrap_or_else(|| ascii::pattern(&key, 34, 17)),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The current song cannot play (no source, or the player refused it):
    /// go on with the next one instead of stopping. Not where someone else
    /// decides what plays (jam guest, another device), and not after
    /// [`MAX_SKIPS`] failures in a row. `true` if the next song was started.
    pub(super) fn skip_unplayable(&mut self, cx: &mut Context<Self>) -> bool {
        if self.account.output_target.is_some() || self.jam_guest() || self.skip_streak >= MAX_SKIPS {
            return false;
        }
        let Some(failed) = self.current.clone() else { return false };
        let Some(Some(next)) = guard(|| self.queue.skip_next().cloned()) else { return false };
        // Repeat one, or a list of just this song: nothing else to go to.
        if track_key(&next) == track_key(&failed) {
            return false;
        }
        self.skip_streak += 1;
        self.toast_info(format!("{} · {}", t("toast.skipped"), failed.title));
        // Paused while it was being found: the next song waits paused too.
        let hold = self.hold_play;
        self.start_track(next, None, cx);
        self.hold_play = hold;
        cx.notify();
        true
    }

    /// The local player holds no song right now (it refused to start one).
    pub(super) fn player_idle(&self) -> bool {
        self.player.as_ref().and_then(|p| guard(|| p.state().status)) == Some(Status::Idle)
    }

    pub fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if self.account_control(if self.is_playing() { glyph_protocol::playback::Command::Pause } else { glyph_protocol::playback::Command::Play }) { return; }
        use glyph_protocol::jam::Control;
        if self.jam_control(if self.is_playing() { Control::Pause } else { Control::Play }) {
            return;
        }
        if self.current.is_none() {
            let recent: Vec<Track> = self.library.recent_tracks().cloned().collect();
            if !recent.is_empty() {
                self.play_list(recent, 0, cx);
            }
            return;
        }
        // The new song is still being found: the player holds the old one,
        // so toggling it would resume that. Remember the wish instead.
        if self.resolving {
            self.hold_play = !self.hold_play;
            cx.notify();
            return;
        }
        if self.sleep.is_none() && !self.is_playing() {
            self.end_sleep_fade();
        }
        if !self.remote_send(crate::mpd::Cmd::Pause(self.is_playing())) {
            self.with_player(|p| p.toggle());
        }
        cx.notify();
    }

    pub fn seek_to(&mut self, pos: Duration) {
        if self.account_control(glyph_protocol::playback::Command::Seek { position_ms: pos.as_millis() as u64 }) { return; }
        if self.jam_control(glyph_protocol::jam::Control::Seek { ms: pos.as_millis() as u64 }) {
            return;
        }
        if self.remote_send(crate::mpd::Cmd::Seek(pos.as_secs_f64())) {
            return;
        }
        self.with_player(|p| p.seek(pos));
    }

    pub fn seek_relative(&mut self, secs: f32) {
        let cur = self.position().as_secs_f32();
        let max = self.duration().map(|d| d.as_secs_f32()).unwrap_or(f32::MAX);
        let to = (cur + secs).clamp(0.0, max);
        self.seek_to(Duration::from_secs_f32(to));
    }

    pub fn set_volume(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        if self.account_control(glyph_protocol::playback::Command::Volume { volume: v }) { return; }
        if v > 0.001 {
            self.unmute_volume = v;
        }
        if self.sleep_fade_from.is_some() {
            // Set by hand: while the timer fades, the new level to fade
            // from; once it is over, simply the volume.
            self.sleep_fade_from = self.sleep.is_some().then_some(v);
        }
        if self.remote_send(crate::mpd::Cmd::Volume((v * 100.0).round() as u8)) {
            return;
        }
        self.with_player(|p| p.set_volume(v));
    }

    /// Speaker icon: mute, or back to the volume before muting.
    pub fn toggle_mute(&mut self) {
        let muted = self.volume() <= 0.001;
        if !muted {
            self.unmute_volume = self.volume();
        }
        let v = if muted { self.unmute_volume } else { 0.0 };
        if self.account_control(glyph_protocol::playback::Command::Volume { volume: v }) { return; }
        if self.remote_send(crate::mpd::Cmd::Volume((v * 100.0).round() as u8)) {
            return;
        }
        self.with_player(|p| p.set_volume(v));
    }

    pub fn next(&mut self, auto: bool, cx: &mut Context<Self>) {
        if self.account.output_target.is_some() {
            if !auto { self.account_control(glyph_protocol::playback::Command::Next); }
            return;
        }
        // In a jam the host decides what comes next. A guest's own player
        // still moves on when a song ends (its queue is the host's, so the
        // next song is the same); the host's next state corrects it if not.
        if self.jam_guest() && (!auto || self.jam_listen_remote()) {
            if !auto {
                self.jam_control(glyph_protocol::jam::Control::Next);
            }
            return;
        }
        if !auto && self.library_ok && self.position() < Duration::from_secs(30) {
            if let Some(cur) = self.current.clone() {
                self.library.record_skip(&cur);
                if self.config.stats.enabled {
                    self.library.stats.add_skip(glyph_core::stats::When::now());
                }
                self.save_library_activity();
            }
        }
        let preloaded = self.preloaded.take();
        let mut next = guard(|| if auto { self.queue.next().cloned() } else { self.queue.skip_next().cloned() });
        // A queue created online can still contain unavailable songs after a
        // network loss. Bound the scan even when repeat-all wraps around.
        if self.offline() {
            let limit = self.queue.upcoming().len() + 1;
            for _ in 0..limit {
                if !matches!(&next, Some(Some(t)) if !self.playable(t)) { break; }
                next = guard(|| self.queue.skip_next().cloned());
            }
            if matches!(&next, Some(Some(t)) if !self.playable(t)) { next = Some(None); }
        }
        match next {
            Some(Some(t)) => {
                // Ended on its own and the player already runs the preloaded
                // next song: follow it instead of resolving and loading again.
                // A player that ended without taking the next song still names
                // the old one, which on repeat one is this very song: load it.
                let playing = self.player.as_ref().and_then(|p| guard(|| p.state()))
                    .filter(|s| !matches!(s.status, Status::Ended | Status::Idle))
                    .and_then(|s| s.track);
                let adopted = preloaded.filter(|(k, r)| {
                    auto && *k == track_key(&t) && playing.as_ref().is_some_and(|p| p.id == r.id && p.source == r.source)
                });
                self.start_track(t, adopted.map(|(_, r)| r), cx);
                if auto {
                    self.sleep_after_song(cx);
                }
            }
            Some(None) => {
                if auto && self.sleep == Some(Sleep::EndOfSong) {
                    // The queue ended anyway: nothing more to stop.
                    self.sleep = None;
                } else if self.radio {
                    self.start_radio(cx);
                } else if auto {
                    self.toast_info(t("toast.queue_end"));
                }
            }
            None => {
                self.queue_ok = false;
                self.toast_info("[ QUEUE OFFLINE ]");
            }
        }
        cx.notify();
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) {
        if self.account_control(glyph_protocol::playback::Command::Previous) { return; }
        if self.jam_control(glyph_protocol::jam::Control::Previous) {
            return;
        }
        if self.position() > Duration::from_secs(3) {
            self.seek_to(Duration::ZERO);
            return;
        }
        if let Some(Some(t)) = guard(|| self.queue.previous().cloned()) {
            self.play_track(t, cx);
        }
    }

    /// The song before this one, even when this one has played a while
    /// (swiping back on the phone's cover). Remote and jam: as [`Self::previous`].
    pub fn previous_song(&mut self, cx: &mut Context<Self>) {
        if self.position() <= Duration::from_secs(3) || self.account.output_target.is_some() || self.jam_active() {
            self.previous(cx);
            return;
        }
        if let Some(Some(t)) = guard(|| self.queue.previous().cloned()) {
            self.play_track(t, cx);
        }
    }

    /// `true` if the error was an expired/denied stream URL and a fresh
    /// resolve was started for the current track.
    ///
    /// Deezer signs preview URLs with a ~15 min lifetime, so a URL fetched
    /// during preload can be dead by the time it plays. Only retried once per
    /// track to avoid a loop when the provider itself is unavailable.
    pub(super) fn retry_expired_preview(&mut self, error: &str, cx: &mut Context<Self>) -> bool {
        // "HTTP 403" when opening, "(403 Forbidden)" from a later range request.
        let refused = ["HTTP 403", "HTTP 401", "403 Forbidden", "401 Unauthorized"].iter().any(|s| error.contains(s));
        if !refused {
            return false;
        }
        let (Some(backend), Some(current)) = (self.backend.clone(), self.current.clone()) else {
            return false;
        };
        if self.retried_expired.replace(current.clone()).is_some() {
            return false;
        }
        let key = track_key(&current);
        let b = backend;
        // Local YouTube URLs expire too (after hours): fetch fresh ones
        // instead of dropping to a preview.
        let youtube = self.current_source == Some(ProviderId::YouTube);
        cx.spawn(async move |this, cx| {
            let r = if youtube {
                guard_async(b.refresh_source(&current)).await
            } else {
                match guard_async(b.refresh_preview(&current)).await {
                    Ok(Some(r)) => Ok(Some(r)),
                    _ => guard_async(b.resolve(&current)).await,
                }
            };
            this.update(cx, |s, cx| {
                if s.current.as_ref().map(track_key).as_deref() != Some(key.as_str()) {
                    return;
                }
                match r {
                    Ok(Some(res)) => {
                        let src = res.source.clone();
                        let tr = res.track.clone();
                        s.output_load(tr, src, true);
                    }
                    Ok(None) => s.toast_info(crate::i18n::t("toast.no_source")),
                    Err(e) => s.toast_info(tf("toast.resolve_error", &[("e", &e)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        true
    }

    /// `true` if the current song played from a local file that cannot be
    /// decoded (truncated or damaged download / cached stream): the file is
    /// dropped, the song plays online instead and a download is fetched
    /// again (once; a second broken copy is only removed). Once per track.
    pub(super) fn repair_broken_file(&mut self, error: &str, cx: &mut Context<Self>) -> bool {
        // Only file problems; a missing audio device must not cost a download.
        const FILE_ERRORS: [&str; 6] =
            ["cannot open stream", "unsupported audio format", "unsupported codec", "read error", "stream error", "decode error"];
        if !FILE_ERRORS.iter().any(|p| error.starts_with(p)) {
            return false;
        }
        let (Some(backend), Some(current)) = (self.backend.clone(), self.current.clone()) else {
            return false;
        };
        let key = track_key(&current);
        if self.repaired_file.as_deref() == Some(key.as_str()) {
            return false;
        }
        let Some(kind) = backend.forget_broken_file(&current) else {
            return false;
        };
        self.repaired_file = Some(key.clone());
        tracing::warn!(track = %key, ?kind, "playback: local file broken, dropped it");
        if kind == glyph_core::BrokenFile::Download {
            if self.redownloaded_broken.insert(key.clone()) {
                self.toast_info(t("toast.dl_broken_retry"));
                self.download_tracks_with(vec![current.clone()], true, cx);
            } else {
                self.toast_info(t("toast.dl_broken_removed"));
            }
        }
        let b = backend;
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.resolve(&current)).await;
            this.update(cx, |s, cx| {
                if s.current.as_ref().map(track_key).as_deref() != Some(key.as_str()) {
                    return;
                }
                match r {
                    Ok(Some(res)) => {
                        let src = res.source.clone();
                        let tr = res.track.clone();
                        s.output_load(tr, src, true);
                    }
                    Ok(None) => s.toast_info(t("toast.no_source")),
                    Err(e) => s.toast_info(tf("toast.resolve_error", &[("e", &e)])),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        true
    }

    pub(super) fn preload_next(&mut self, cx: &mut Context<Self>) {
        if self.account.output_target.is_some() { return; }
        // Listening at the jam host's device: nothing plays here.
        if self.jam_listen_remote() {
            return;
        }
        // MPD and network devices play one song at a time; the next one is
        // sent when it ends.
        if self.remote_chosen() {
            return;
        }
        // Sleep timer "end of this song": the next one must not start on its own.
        if self.sleep == Some(Sleep::EndOfSong) {
            return;
        }
        let Some(Some(next)) = guard(|| self.queue.peek_next().cloned()) else { return };
        if !self.playable(&next) { return; }
        let Some(b) = self.backend.clone() else { return };
        let cur = self.current.as_ref().map(track_key);
        let want = track_key(&next);
        if self.preloaded.as_ref().is_some_and(|(key, _)| key == &want) { return; }
        let Some(current_key) = cur.as_ref() else { return; };
        if self.preload_attempt.as_ref().is_some_and(|(current, next, at)|
            current == current_key && next == &want && at.elapsed() < Duration::from_secs(15)) { return; }
        let attempt = Instant::now();
        self.preload_attempt = Some((current_key.clone(), want.clone(), attempt));
        cx.spawn(async move |this, cx| {
            if let Ok(Some(res)) = guard_async(b.resolve(&next)).await {
                this.update(cx, |s, _| {
                    // Resolving can take a while: if another song started or
                    // the queue changed meanwhile, this one is not next anymore.
                    if s.current.as_ref().map(track_key) != cur || s.next_key().as_deref() != Some(want.as_str())
                        || !s.playable(&next)
                        || s.preload_attempt.as_ref().map(|(_, _, at)| *at) != Some(attempt) {
                        return;
                    }
                    s.preloaded = Some((want, res.track.clone()));
                    s.with_player(|p| p.preload_next(res.track, res.source));
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn next_key(&self) -> Option<String> {
        guard(|| self.queue.peek_next().map(track_key)).flatten()
    }

    /// After a queue edit: a preloaded song that is no longer next must not
    /// start after the current one (the player then asks for the new next).
    pub(super) fn queue_changed(&mut self) {
        let stale = self.preloaded.as_ref().is_some_and(|(k, _)| self.next_key().as_deref() != Some(k.as_str()));
        if stale {
            self.preloaded = None;
            self.with_player(|p| p.drop_next());
        }
    }

    /// Asks the backend once whether radio can really deliver similar songs.
    /// A server without a (valid) Last.fm key answers `/v1/similar` with an
    /// empty list; the switch is then greyed out with a hover explanation.
    pub fn probe_radio(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else {
            self.radio_ok = None;
            return;
        };
        self.radio_ok = None;
        cx.spawn(async move |this, cx| {
            let ok = guard_async(b.radio_available()).await.unwrap_or(false);
            this.update(cx, |s, cx| {
                s.radio_ok = Some(ok);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Queue finished: continue with songs similar to the last few that
    /// played (Last.fm getSimilar). They join the end of the queue, so
    /// "previous" still goes back into the list, and nothing from the list
    /// comes back.
    pub(super) fn start_radio(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.current.clone() else { return };
        let played: Vec<Track> = guard(|| self.queue.history().into_iter().cloned().collect()).unwrap_or_default();
        let exclude: HashSet<String> = played.iter().map(glyph_core::dedupe::track_key).collect();
        let seeds: Vec<Track> = std::iter::once(current.clone()).chain(played.into_iter().rev().take(4)).collect();
        self.radio_from_seeds(seeds, exclude, current.title, true, cx);
    }

    /// Radio from a playlist / album: songs similar to the list as a whole.
    pub fn list_radio(&mut self, l: &ListRef, cx: &mut Context<Self>) {
        let title = self.list_title(l).to_string();
        let tracks = self.list_tracks(l);
        self.start_radio_from(tracks, title, cx);
    }

    /// Radio from an artist: seeded with their songs Glyph already knows
    /// (their page, liked and recently played songs, the followed entry).
    pub fn artist_radio(&mut self, name: &str, cx: &mut Context<Self>) {
        let mut seeds: Vec<Track> = self.artists.get(name).map(|p| p.tracks.iter().take(5).cloned().collect()).unwrap_or_default();
        if seeds.is_empty() {
            let known = self.library.liked.iter().chain(self.library.recent_tracks());
            seeds = known.filter(|t| glyph_core::dedupe::by_artist(&t.artist, name)).take(5).cloned().collect();
        }
        if seeds.is_empty() {
            seeds.extend(self.library.followed.iter().find(|f| f.name.eq_ignore_ascii_case(name)).and_then(|f| f.track.clone()));
        }
        if seeds.is_empty() {
            self.toast_info(t("toast.radio_empty"));
            return;
        }
        self.start_radio_from(seeds, name.to_string(), cx);
    }

    /// Plays songs similar to `seeds` (up to five, spread over the list):
    /// songs suggested for several seeds come first, the seeds themselves
    /// are left out and one artist never plays twice in a row.
    pub fn start_radio_from(&mut self, seeds: Vec<Track>, title: String, cx: &mut Context<Self>) {
        self.radio_from_seeds(seeds, HashSet::new(), title, false, cx);
    }

    /// [`Self::start_radio_from`], also leaving out `exclude` (song keys).
    /// `append`: the songs go on after the ended queue instead of replacing it.
    fn radio_from_seeds(&mut self, seeds: Vec<Track>, exclude: HashSet<String>, title: String, append: bool, cx: &mut Context<Self>) {
        const SEEDS: usize = 5;
        let Some(b) = self.backend.clone() else { return };
        if seeds.is_empty() {
            self.toast_info(t("toast.radio_empty"));
            return;
        }
        let step = (seeds.len() as f32 / SEEDS as f32).max(1.0);
        let picked: Vec<Track> = (0..SEEDS.min(seeds.len())).map(|i| seeds[(i as f32 * step) as usize].clone()).collect();
        let mut skip = exclude;
        skip.extend(seeds.iter().map(glyph_core::dedupe::track_key));
        let ended_on = self.current.as_ref().map(track_key);
        self.toast(tf("toast.radio", &[("title", &title)]));
        cx.spawn(async move |this, cx| {
            let mut found: Vec<(Track, f32)> = Vec::new();
            let mut error = None;
            // All seeds at once: one wait instead of one per seed.
            let answers = futures::future::join_all(picked.iter().map(|seed| guard_async(b.similar(seed)))).await;
            for answer in answers {
                match answer {
                    Ok(sim) => {
                        for (i, t) in sim.into_iter().take(40).enumerate() {
                            let key = glyph_core::dedupe::track_key(&t);
                            if skip.contains(&key) {
                                continue;
                            }
                            let w = 1.0 - i as f32 / 50.0;
                            match found.iter_mut().find(|(x, _)| glyph_core::dedupe::track_key(x) == key) {
                                Some(f) => f.1 += w,
                                None => found.push((t, w)),
                            }
                        }
                    }
                    Err(e) => error = Some(e),
                }
            }
            this.update(cx, |s, cx| {
                found.retain(|(t, _)| !s.is_hidden_artist(&t.artist) && s.allowed(t));
                found.sort_by(|a, b| b.1.total_cmp(&a.1));
                let tracks = spread_artists(found.into_iter().map(|(t, _)| t).take(60).collect());
                match (tracks.is_empty(), error) {
                    (false, _) if append => {
                        // Something else started meanwhile: that wins.
                        let still_ended = s.current.as_ref().map(track_key) == ended_on && guard(|| s.queue.peek_next().is_none()) == Some(true);
                        if still_ended {
                            s.radio_continue(tracks, tf("queue.radio_from", &[("title", &title)]).to_string(), cx);
                        }
                    }
                    (false, _) => {
                        let label = tf("queue.radio_from", &[("title", &title)]).to_string();
                        s.play_list(tracks, 0, cx);
                        s.queue_from = Some(label);
                        s.recent_artist_ctx = None;
                        s.playing_from = None;
                    }
                    (true, Some(e)) => s.toast_info(tf("toast.radio_error", &[("e", &e)])),
                    (true, None) => s.toast_info(t("toast.radio_empty")),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Radio songs join the end of the ended queue and the first one starts.
    fn radio_continue(&mut self, tracks: Vec<Track>, label: String, cx: &mut Context<Self>) {
        let first = guard(|| {
            self.queue.append(tracks);
            self.queue.skip_next().cloned()
        });
        let Some(Some(first)) = first else { return };
        self.queue_from = Some(label);
        self.recent_artist_ctx = None;
        self.playing_from = None;
        self.start_track(first, None, cx);
        cx.notify();
    }

    pub fn shuffle_mode(&self) -> glyph_core::queue::ShuffleMode {
        match self.config.playback.shuffle {
            glyph_core::config::ShuffleStyle::Smart => glyph_core::queue::ShuffleMode::Smart,
            glyph_core::config::ShuffleStyle::Random => glyph_core::queue::ShuffleMode::Random,
        }
    }

    /// Settings > Playback: smart or truly random shuffle.
    pub fn set_shuffle_style(&mut self, style: glyph_core::config::ShuffleStyle, cx: &mut Context<Self>) {
        self.config.playback.shuffle = style;
        let mode = self.shuffle_mode();
        let recent = self.recent_keys();
        guard(|| {
            self.queue.set_recent(recent);
            self.queue.set_shuffle_mode(mode);
        });
        self.queue_changed();
        self.save_config(cx);
        cx.notify();
    }

    /// Turns shuffle on with `style` (right-click menu of the shuffle button).
    pub fn shuffle_with(&mut self, style: glyph_core::config::ShuffleStyle, cx: &mut Context<Self>) {
        if self.config.playback.shuffle != style {
            self.set_shuffle_style(style, cx);
        }
        if !self.shuffle() {
            self.toggle_shuffle(cx);
        }
    }

    pub fn set_repeat_mode(&mut self, mode: RepeatMode, cx: &mut Context<Self>) {
        guard(|| self.queue.set_repeat(mode));
        self.queue_changed();
        cx.notify();
    }

    pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        let on = !self.shuffle();
        let recent = self.recent_keys();
        guard(|| self.queue.set_recent(recent));
        guard(|| self.queue.set_shuffle(on));
        self.queue_changed();
        cx.notify();
    }

    pub fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        let next = match self.repeat() {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        guard(|| self.queue.set_repeat(next));
        self.queue_changed();
        cx.notify();
    }

    /// Shows or hides the mini player. Its window is kept hidden rather than
    /// closed where possible: opening a new window takes a moment.
    pub fn toggle_mini(&mut self, cx: &mut Context<Self>) {
        use crate::ui::mini;
        if self.mini_shown {
            self.mini_shown = false;
            if let Some(h) = self.mini_window {
                if !h.update(cx, |_, w, cx| mini::set_visible(w, false, cx)).unwrap_or(false) {
                    let _ = h.update(cx, |_, w, _| w.remove_window());
                    self.mini_window = None;
                }
            }
            cx.notify();
            return;
        }
        self.mini_shown = true;
        cx.notify();
        if let Some(h) = self.mini_window {
            if h.update(cx, |_, w, cx| mini::set_visible(w, true, cx)).unwrap_or(false) {
                return;
            }
            self.mini_window = None;
        }
        let me = cx.entity();
        cx.defer(move |cx| {
            let h = mini::MiniPlayer::open(me.clone(), cx);
            me.update(cx, |g, cx| {
                g.mini_shown = h.is_some();
                g.mini_window = h;
                cx.notify();
            });
        });
    }

    /// Settings -> Appearance: bar or card. An open mini player switches at once.
    pub fn set_mini_big(&mut self, big: bool, cx: &mut Context<Self>) {
        if self.mini_big == big {
            return;
        }
        self.set_mini_big_flag(big, cx);
        if let Some(h) = self.mini_window {
            let sz = crate::ui::mini::mini_size(big, (self.mini_card_w, self.mini_card_h));
            let _ = h.update(cx, |_, w, cx| crate::ui::mini::fit_window(w, sz, cx));
        }
    }

    /// Bar or card, remembered (the caller resizes an open window).
    pub fn set_mini_big_flag(&mut self, big: bool, cx: &mut Context<Self>) {
        self.mini_big = big;
        self.save_player_prefs(cx);
        cx.notify();
    }

    /// The card was resized: remember its size.
    pub fn set_mini_card_size(&mut self, w: f32, h: f32, cx: &mut Context<Self>) {
        self.mini_card_w = w;
        self.mini_card_h = h;
        self.save_player_prefs(cx);
    }

    pub fn queue_add(&mut self, t: Track, next: bool, cx: &mut Context<Self>) {
        if self.blocked_offline(&t, cx) { return; }
        if self.jam_request(&t, false) {
            return;
        }
        if self.blocked_explicit(&t, cx) {
            return;
        }
        let title = t.title.clone();
        let ok = if next {
            guard(|| self.queue.play_next(t))
        } else {
            guard(|| self.queue.push(t))
        };
        match ok {
            Some(()) => self.toast_info(tf(if next { "toast.play_next" } else { "toast.queued" }, &[("title", &title)])),
            None => self.toast_info("[ QUEUE OFFLINE ]"),
        }
        self.queue_changed();
        cx.notify();
    }

    pub fn queue_remove(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.jam_queue_edit(Some(i), cx) {
            return;
        }
        guard(|| self.queue.remove(i));
        self.queue_changed();
        cx.notify();
    }

    pub fn queue_move(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if self.jam_queue_edit(None, cx) {
            return;
        }
        guard(|| self.queue.move_item(from, to));
        self.queue_changed();
        cx.notify();
    }
}

/// Unplayable songs skipped in a row before Glyph stops and says so.
pub(super) const MAX_SKIPS: u32 = 5;

/// A problem of the audio output itself: every song would fail the same
/// way, so skipping to the next one does not help.
pub(super) fn output_error(e: &str) -> bool {
    ["no audio output device", "audio output not available", "The selected audio output"].iter().any(|p| e.starts_with(p))
}

/// Same order, but a song whose artist just played moves back behind the
/// next song by someone else (when there is one).
fn spread_artists(mut rest: Vec<Track>) -> Vec<Track> {
    let mut out: Vec<Track> = Vec::with_capacity(rest.len());
    while !rest.is_empty() {
        let last = out.last().map(|t| glyph_core::dedupe::primary_artist(&t.artist).to_lowercase());
        let i = rest.iter().position(|t| Some(glyph_core::dedupe::primary_artist(&t.artist).to_lowercase()) != last).unwrap_or(0);
        out.push(rest.remove(i));
    }
    out
}
