use super::*;

impl Glyph {
    pub fn new(window: &mut Window, config: Config, cx: &mut Context<Self>) -> Self {
        let glow = cx.new(|cx| GlowView::new(window, cx));
        let search_input =
            cx.new(|cx| TextInput::new("search.placeholder", cx));
        let server_input = cx.new(|cx| TextInput::new("settings.server.placeholder", cx));
        let server_bearer_input = cx.new(|cx| TextInput::new("settings.server.bearer.placeholder", cx).masked());
        let discord_id_input = cx.new(|cx| TextInput::new("sp.discord.id.placeholder", cx));
        let settings_search = cx.new(|cx| TextInput::new("settings.search", cx));
        let playlist_name = cx.new(|cx| TextInput::new("dialog.playlist.placeholder", cx));
        let playlist_link = cx.new(|cx| TextInput::new("dialog.playlist.link", cx));
        let add_input = cx.new(|cx| TextInput::new("add.search", cx));
        let mpd_input = cx.new(|cx| TextInput::new("sp.mpd.placeholder", cx));
        let key_inputs: Vec<(&'static str, Entity<TextInput>)> = [
            "youtube",
            "jamendo_client_id",
            "soundcloud_client_id",
            "spotify_client_id",
            "spotify_client_secret",
            "lastfm",
        ]
        .into_iter()
        .map(|k| (k, cx.new(|cx| TextInput::new("settings.keys.unset", cx).masked())))
        .collect();

        let mut subs = vec![cx.subscribe(&search_input, |this, _, ev: &InputEvent, cx| {
            match ev {
                InputEvent::Changed(q) => this.on_query(q.clone(), false, cx),
                InputEvent::Submit(q) => this.on_query(q.clone(), true, cx),
                InputEvent::Cancel => {}
            }
        })];
        subs.push(cx.subscribe(&add_input, |this, _, ev: &InputEvent, cx| match ev {
            InputEvent::Changed(q) | InputEvent::Submit(q) => this.add_search(q.clone(), cx),
            InputEvent::Cancel => this.close_add_songs(cx),
        }));
        subs.push(cx.subscribe(&playlist_name, |this, _, ev: &InputEvent, cx| match ev {
            InputEvent::Submit(_) => this.submit_name_dialog(cx),
            InputEvent::Cancel => this.close_name_dialog(cx),
            InputEvent::Changed(_) => {}
        }));
        subs.push(cx.subscribe(&playlist_link, |this, _, ev: &InputEvent, cx| match ev {
            InputEvent::Submit(_) => this.submit_name_dialog(cx),
            InputEvent::Cancel => this.close_name_dialog(cx),
            InputEvent::Changed(_) => {
                // A new link clears the last error.
                if this.link_import.as_ref().is_some_and(|l| l.error.is_some()) {
                    this.link_import = None;
                }
                cx.notify();
            }
        }));
        subs.push(cx.subscribe(&server_input, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::Changed(v) | InputEvent::Submit(v) = ev {
                this.config.server_url = v.trim().to_string();
                this.conn_test = None;
                this.server_auth_required = false;
                this.server_bearer_input.update(cx, |i,cx| i.set_value(String::new(),cx));
                this.server_probe_seq += 1;
                let seq = this.server_probe_seq;
                let base = this.config.effective_server_url().to_string();
                cx.spawn(async move |this,cx| {
                    cx.background_executor().timer(Duration::from_millis(600)).await;
                    if !this.update(cx, |s,_| s.server_probe_seq == seq && s.config.effective_server_url() == base).unwrap_or(false) { return; }
                    let probe_base = base.clone();
                    let required = cx.background_executor().spawn(async move {
                        glyph_core::server_access::probe(&probe_base).ok()
                    }).await;
                    this.update(cx, |s,cx| { if s.server_probe_seq == seq && s.config.effective_server_url() == base { s.server_auth_required = required.unwrap_or(false); cx.notify(); } }).ok();
                }).detach();
                if matches!(ev, InputEvent::Submit(_)) {
                    this.save_config(cx);
                }
            }
        }));
        subs.push(cx.subscribe(&mpd_input, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::Changed(v) | InputEvent::Submit(v) = ev {
                this.config.mpd.address = v.trim().to_string();
                if matches!(ev, InputEvent::Submit(_)) {
                    this.save_config(cx);
                    this.sync_mpd(cx);
                } else {
                    this.config_dirty = Some((Instant::now(), false));
                }
            }
        }));
        subs.push(cx.subscribe(&discord_id_input, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::Changed(v) | InputEvent::Submit(v) = ev {
                this.config.integrations.discord_client_id = v.trim().to_string();
                if matches!(ev, InputEvent::Submit(_)) {
                    this.save_config(cx);
                    this.sync_discord();
                } else {
                    // Saved quietly once typing stops; the worker follows on Enter.
                    this.config_dirty = Some((Instant::now(), false));
                }
            }
        }));
        for (name, input) in &key_inputs {
            let name = *name;
            subs.push(cx.subscribe(input, move |this, _, ev: &InputEvent, cx| {
                if let InputEvent::Changed(v) | InputEvent::Submit(v) = ev {
                    let v = (!v.trim().is_empty()).then(|| v.trim().to_string());
                    let k = &mut this.config.keys;
                    match name {
                        "youtube" => k.youtube = v,
                        "jamendo_client_id" => k.jamendo_client_id = v,
                        "soundcloud_client_id" => k.soundcloud_client_id = v,
                        "spotify_client_id" => k.spotify_client_id = v,
                        "spotify_client_secret" => k.spotify_client_secret = v,
                        _ => k.lastfm = v,
                    }
                    if matches!(ev, InputEvent::Submit(_)) {
                        this.save_config(cx);
                    }
                }
            }));
        }

        let hwnd = window_hwnd(window);
        // Like / previous / play-pause / next under the taskbar preview.
        #[cfg(windows)]
        if let Some(h) = hwnd {
            crate::thumbbar::init(
                h,
                [t("ctx.like").to_string(), t("dock.previous").to_string(), t("dock.play_pause").to_string(), t("dock.next").to_string()],
            );
        }

        let poll_window = window.window_handle();
        let poll = cx.spawn(async move |this, cx| {
            let mut every = Duration::from_millis(16);
            loop {
                cx.background_executor().timer(every).await;
                let Ok(title) = this.update(cx, |s, cx| {
                    s.poll(cx);
                    s.take_window_title()
                }) else {
                    break;
                };
                // Rendering pauses while the window is in the background; the
                // title (taskbar preview, alt-tab) must follow the song anyway.
                if let Some((handle, title)) = title {
                    let _ = cx.update_window(handle, |_, w, _| w.set_window_title(&title));
                }
                // Phones: in the background (music playing for hours) a tick
                // every 100 ms is plenty; 60 wake-ups a second drained and
                // warmed the phone.
                if crate::ui::compact::TOUCH {
                    let active = cx.update_window(poll_window, |_, w, _| w.is_window_active()).unwrap_or(true);
                    every = Duration::from_millis(if active { 16 } else { 100 });
                }
            }
        });

        let mut this = Self {
            focus: cx.focus_handle(),
            view: View::Home,
            open_list: None,
            open_artist: None,
            open_profile: None,
            section: None,
            selected_row: None,
            artists: Default::default(),
            artist_photos: Default::default(),
            history: Vec::new(),
            ctx_menu: None,
            control_menu: None,
            control_closing: None,
            ctx_opened: Instant::now(),
            ctx_closing: None,
            heroes: Default::default(),
            stale_heroes: Vec::new(),
            main_scroll: gpui::ScrollHandle::new(),
            settings_scroll: gpui::ScrollHandle::new(),
            queue_scroll: gpui::ScrollHandle::new(),
            lyrics_scroll: gpui::ScrollHandle::new(),
            lyrics_user_scroll: None,
            pos_anchor: None,
            name_dialog: None,
            add_songs: None,
            add_input,
            add_scroll: gpui::ScrollHandle::new(),
            nav_hover: None,
            album_task: None,
            // Read straight from disk: the async config load comes too late for the first frame.
            dock_keys: Vec::new(),
            pending_recent_artist: None,
            downloading: HashSet::new(),
            download_done: Default::default(),
            album_meta: Default::default(),
            output_devices: Vec::new(),
            recs: Vec::new(),
            trending: Vec::new(),
            feeds_loading: false,
            new_releases: Vec::new(),
            new_releases_loading: false,
            splash: config.visuals.startup_animation.then(crate::ui::splash::Splash::new),
            nav_leave: None,
            lyrics_info: None,
            side_hover: None,
            playlist_name,
            playlist_link,
            link_import: None,
            link_import_seq: 0,
            glow,
            search_input,
            server_input,
            server_bearer_input,
            discord_id_input,
            server_auth_required: false,
            server_probe_seq: 0,
            outage_seen: 0,
            key_inputs,
            config: Config::default(),
            backend: None,
            backend_state: BackendState::Connecting,
            player: None,
            player_ok: false,
            player_error: None,
            player_state: None,
            player_events: None,
            media_keys: None,
            media_events: None,
            hwnd,
            discord: None,
            discord_id: None,
            discord_sent: None,
            queue: Queue::default(),
            queue_ok: true,
            library: Library::default(),
            library_ok: false,
            current: None,
            activity_recorded: false,
            home_activity_cache: None,
            current_source: None,
            current_preview: false,
            retried_expired: None,
            repaired_file: None,
            redownloaded_broken: HashSet::new(),
            resolving: false,
            hold_play: false,
            sleep: None,
            preloaded: None,
            preload_attempt: None,
            download_check_at: Instant::now(),
            unmute_volume: 0.8,
            palette: None,
            cover: None,
            search: SearchState::default(),
            lyrics: LyricsState::default(),
            update: UpdateState::default(),
            conn_test: None,
            ytdlp: Default::default(),
            show_queue: false,
            show_devices: false,
            devices_jam: false,
            devices_scroll: gpui::ScrollHandle::new(),
            show_lyrics: true,
            lyrics_only: false,
            bar_slide: Slide::default(),
            queue_slide: Slide::default(),
            loop_ab: (None, None),
            loop_song: None,
            loop_sent: None,
            queue_drop: None,
            list_drop: None,
            radio: true,
            radio_ok: None,
            mini_on_top: true,
            mini_window: None,
            mini_shown: false,
            mini_big: false,
            mini_card_w: crate::ui::mini::CARD_W,
            mini_card_h: crate::ui::mini::CARD_H,
            nav_quiet: false,
            playing_from: None,
            queue_from: None,
            recent_artist_ctx: None,
            stats_range: glyph_core::stats::Range::ThisMonth,
            recap_year: None,
            stats_pending: None,
            stats_tick: None,
            stats_flushed: Instant::now(),
            stats_synced: Instant::now(),
            window_title: String::new(),
            eq_redraw: Instant::now(),
            slow_redraw: Instant::now(),
            mcp: Default::default(),
            jam: Default::default(),
            mpd: None,
            mpd_input,
            mpd_was_playing: false,
            discovery: None,
            discovery_gen: 0,
            device_out: None,
            device_failed: false,
            local_fallback: false,
            last_load: None,
            mcp_checked: Instant::now() - Duration::from_secs(60),
            verified: Default::default(),
            verified_inflight: 0,
            window_handle: window.window_handle(),
            config_loaded: false,
            config_dirty: None,
            activity_dirty: None,
            library_save: None,
            settings_open: false,
            settings_in_page: false,
            drawer_open: false,
            drawer_slide: Slide::default(),
            lib_filter: 0,
            settings_at: Instant::now(),
            settings_closing: None,
            settings_page: SettingsPage::Playback,
            account: Default::default(),
            shared_preview: None,
            pending_share_link: None,
            profile: Default::default(),
            transfer: Default::default(),
            migrate: Default::default(),
            settings_search,
            open_select: None,
            settings_sub: 0,
            settings_collapsed: HashSet::new(),
            settings_group_motion: Default::default(),
            storage: None,
            // Downloads are offline songs: opt-in, never cleared by default.
            storage_selected: [true, false, true, true, true],
            toasts: Vec::new(),
            toast_seq: 0,
            toast_drag: None,
            toast_hover: false,
            toast_hover_at: None,
            lang: crate::i18n::lang(),
            ink: [0.0; 3],
            covers: Default::default(),
            covers_loading: Default::default(),
            ascii_covers: Default::default(),
            embed: crate::embed::EmbedHost::new(raw_handle(window)),
            media_keys_sent: None,
            media_art_sent: None,
            started: Instant::now(),
            _subs: subs,
            _poll: poll,
            _save_on_quit: cx.on_app_quit(|s: &mut Glyph, _| {
                let pending = s.config_dirty.take().is_some().then(|| s.config.clone());
                let pending_act = (s.activity_dirty.take().is_some() && s.library_ok)
                    .then(|| s.library.activity_snapshot());
                let backend = s.backend.clone();
                let library_save = s.library_save.take();
                async move {
                    if let Some(save) = library_save { save.await; }
                    if let Some(b) = backend {
                        let _ = guard(|| b.flush());
                    }
                    if let Some(act) = pending_act {
                        if let Err(e) = guard_res(|| act.save()) {
                            tracing::warn!("saving library activity on quit: {e}");
                        }
                    }
                    if let Some(cfg) = pending {
                        if let Err(e) = guard_res(|| cfg.save()) {
                            tracing::warn!("saving config on quit: {e}");
                        }
                    }
                }
            }),
        };
        this.apply_config(config.clone(), cx);
        this.boot(config, cx);
        this
    }

    // ------------------------------------------------------------ startup

    pub(super) fn boot(&mut self, config: Config, cx: &mut Context<Self>) {
        // Backend remains off the UI thread; config was already loaded once.
        cx.spawn(async move |this, cx| {
            let backend = cx
                .background_executor()
                .spawn(async move { guard_async(Backend::new(config)).await })
                .await;
            this.update(cx, |s, cx| s.backend_ready(backend, cx)).ok();
        })
        .detach();

        // Publish the library without waiting for audio device initialization.
        cx.spawn(async move |this, cx| {
            let lib = cx
                .background_executor()
                .spawn(async { guard_res(Library::load) })
                .await;
            this.update(cx, |s, cx| {
                tracing::info!(elapsed_ms = s.started.elapsed().as_millis() as u64, "startup: library ready");
                match lib {
                    Ok(l) => {
                        s.library = l;
                        s.home_activity_cache = None;
                        s.library_ok = true;
                        s.load_feeds(cx);
                        s.load_new_releases(cx);
                        s.sync_now(cx);
                    }
                    Err(e) => tracing::warn!("library: {e}"),
                }
                cx.notify();
            }).ok();
        }).detach();

        cx.spawn(async move |this, cx| {
            let player = cx.background_executor()
                .spawn(async { guard_res(PlayerController::new) }).await;
            this.update(cx, |s, cx| {
                tracing::info!(elapsed_ms = s.started.elapsed().as_millis() as u64, "startup: player ready");
                match player {
                    Ok(p) => {
                        let p = Arc::new(p);
                        p.set_crossfade(Duration::from_secs(s.config.audio.crossfade_s as u64));
                        p.set_loudness(level_of(s.config.audio.level), s.config.audio.normalize);
                        p.set_equalizer(s.config.audio.eq_enabled, eq_gains(&s.config.audio.eq_gains));
                        p.set_channel_mode(channels_of(s.config.audio.channels));
                        p.set_output_device(s.config.audio.output_device.clone());
                        p.set_volume(s.config.player.volume);
                        p.set_mini_player_options(MiniPlayerOptions { always_on_top: s.mini_on_top, position: None });
                        s.player_events = guard(|| p.events());
                        s.player_ok = true;
                        s.glow.update(cx, |g, _| g.set_player(Some(p.clone())));
                        // The embed webview is attached lazily in `poll` once an
                        // embed actually plays: a child webview makes Windows send
                        // synchronous messages to our window while GPUI holds its
                        // state ("RefCell already borrowed" on titlebar hover).
                        s.player = Some(p);
                    }
                    Err(e) => {
                        tracing::warn!("player: {e}");
                        s.player_error = Some(e);
                    }
                }
                s.init_media_keys();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn init_media_keys(&mut self) {
        let hwnd = self.hwnd.map(|h| h as *mut std::ffi::c_void);
        match guard_res(|| MediaKeys::new(hwnd)) {
            Ok(mk) => {
                self.media_events = guard(|| mk.events());
                self.media_keys = Some(mk);
            }
            Err(e) => tracing::warn!("media keys: {e}"),
        }
    }

    /// Starts, restarts or stops the Discord presence worker as the config
    /// (Settings > Integrations) says.
    pub(super) fn sync_discord(&mut self) {
        let id = crate::discord::client_id(&self.config);
        match (self.config.integrations.discord_presence, id) {
            (true, Some(id)) => {
                if self.discord.is_some() && self.discord_id.as_deref() == Some(id.as_str()) {
                    return;
                }
                self.discord = Some(crate::discord::DiscordPresence::new(id.clone()));
                self.discord_id = Some(id);
                self.discord_sent = None;
                self.push_discord_now();
                tracing::info!("discord presence: on");
            }
            _ => {
                if self.discord.take().is_some() {
                    tracing::info!("discord presence: off");
                }
                self.discord_id = None;
                self.discord_sent = None;
            }
        }
    }

    /// Pushes the cached player state right away, so a freshly started worker
    /// does not wait for the next player event.
    pub(super) fn push_discord_now(&mut self) {
        let Some(d) = &self.discord else { return };
        let Some(st) = &self.player_state else { return };
        let playing = st.status == Status::Playing;
        let key = (
            st.track.as_ref().map(|t| (t.source, t.id.clone())),
            playing,
            st.position.as_secs() / 15,
        );
        d.set(st.track.as_ref(), playing, st.position);
        self.discord_sent = Some(key);
    }

    /// Discord Rich Presence on / off (Settings > Integrations).
    pub fn set_discord_presence(&mut self, on: bool, cx: &mut Context<Self>) {
        if on && crate::discord::client_id(&self.config).is_none() {
            self.toast_info(t("sp.discord.needs_id"));
        }
        self.config.integrations.discord_presence = on;
        self.save_config(cx);
        self.sync_discord();
        cx.notify();
    }

    /// Volume, shuffle, repeat and panels as saved in `config.player`.
    pub(super) fn restore_player_prefs(&mut self) {
        let prefs = self.config.player.clone();
        let repeat = match prefs.repeat.as_str() {
            "all" => RepeatMode::All,
            "one" => RepeatMode::One,
            _ => RepeatMode::Off,
        };
        let mode = self.shuffle_mode();
        guard(|| {
            self.queue.set_shuffle_mode(mode);
            self.queue.set_shuffle(prefs.shuffle);
            self.queue.set_repeat(repeat);
        });
        // Phones: the queue is a sheet, not reopened on start.
        self.show_queue = prefs.show_queue && !crate::ui::compact::TOUCH;
        self.mini_on_top = prefs.mini_on_top;
        self.mini_big = prefs.mini_large;
        self.mini_card_w = prefs.mini_card_w;
        self.mini_card_h = prefs.mini_card_h;
        if let Some(p) = &self.player {
            p.set_volume(prefs.volume);
            p.set_mini_player_options(MiniPlayerOptions { always_on_top: prefs.mini_on_top, position: None });
        }
        // The cached state is read before the next refresh: keep it in step.
        if let Some(st) = self.player_state.as_mut() {
            st.volume = prefs.volume;
        }
        self.config_loaded = true;
    }

    /// Player prefs as they are now (`None` before the config is loaded).
    pub(super) fn current_player_prefs(&self) -> Option<glyph_core::config::PlayerPrefs> {
        if !self.config_loaded {
            return None;
        }
        let volume = self.player_state.as_ref().map_or(self.config.player.volume, |s| s.volume);
        let repeat = match self.repeat() {
            RepeatMode::Off => "off",
            RepeatMode::All => "all",
            RepeatMode::One => "one",
        };
        Some(glyph_core::config::PlayerPrefs {
            volume,
            shuffle: self.shuffle(),
            repeat: repeat.into(),
            show_queue: self.show_queue,
            mini_on_top: self.mini_on_top,
            mini_large: self.mini_big,
            mini_card_w: self.mini_card_w,
            mini_card_h: self.mini_card_h,
        })
    }

    /// Notes changed player prefs (saved quietly by `flush_config`).
    pub(super) fn save_player_prefs(&mut self, cx: &mut Context<Self>) {
        if let Some(prefs) = self.current_player_prefs() {
            if prefs != self.config.player {
                self.config.player = prefs;
                // Keeps a pending settings toast; restarts the delay.
                let toast = self.config_dirty.is_some_and(|(_, t)| t);
                self.config_dirty = Some((Instant::now(), toast));
            }
        }
        self.flush_config(cx);
    }

    pub(crate) fn apply_config(&mut self, config: Config, cx: &mut Context<Self>) {
        self.server_input
            .update(cx, |i, cx| i.set_value(config.server_url.clone(), cx));
        // Empty field + built-in app ID = nothing to enter; the placeholder
        // says "already set" (Settings > Integrations).
        let built_in = config.integrations.discord_client_id.trim().is_empty()
            && crate::discord::client_id(&config).is_some();
        let placeholder = if built_in {
            "sp.discord.id.builtin.placeholder"
        } else {
            "sp.discord.id.placeholder"
        };
        self.discord_id_input.update(cx, |i, cx| {
            i.placeholder = placeholder.into();
            i.set_value(config.integrations.discord_client_id.clone(), cx);
        });
        let mpd_addr = config.mpd.address.clone();
        self.mpd_input.update(cx, |i, cx| i.set_value(mpd_addr, cx));
        for (name, input) in &self.key_inputs {
            let k = &config.keys;
            let v = match *name {
                "youtube" => &k.youtube,
                "jamendo_client_id" => &k.jamendo_client_id,
                "soundcloud_client_id" => &k.soundcloud_client_id,
                "spotify_client_id" => &k.spotify_client_id,
                "spotify_client_secret" => &k.spotify_client_secret,
                _ => &k.lastfm,
            };
            let v = v.clone().unwrap_or_default();
            input.update(cx, |i, cx| i.set_value(v, cx));
        }
        let lang = crate::config_ext::language(&config);
        crate::i18n::set_lang(lang);
        self.lang = lang;
        let reactivity = config.visuals.music_reactivity;
        self.glow.update(cx, |g, _| g.set_music_reactivity(reactivity));
        let intensity = config.visuals.intensity;
        self.glow.update(cx, |g, _| g.set_intensity(intensity));
        let throttle = config.visuals.throttle_unfocused;
        self.glow.update(cx, |g, _| g.set_throttle_inactive(throttle));
        let glow = crate::theme::glow_palette(&config.visuals.theme);
        self.glow.update(cx, |g, _| g.set_theme(glow));
        self.config = config;
        self.restore_player_prefs();
        if let Some(p) = &self.player {
            p.set_crossfade(Duration::from_secs(self.config.audio.crossfade_s as u64));
            p.set_loudness(level_of(self.config.audio.level), self.config.audio.normalize);
            p.set_equalizer(self.config.audio.eq_enabled, eq_gains(&self.config.audio.eq_gains));
            p.set_channel_mode(channels_of(self.config.audio.channels));
        }
        self.sync_discord();
        self.sync_mpd(cx);
        self.sync_discovery(cx);
        cx.notify();
    }

    pub(super) fn backend_ready(&mut self, res: Result<Backend, String>, cx: &mut Context<Self>) {
        match res {
            Ok(b) => {
                if b.config().effective_server_url() != self.config.effective_server_url() { return; }
                self.server_auth_required = b.server_auth_required();
                glyph_audio::http::set_server_access(b.server_media_credentials());
                glyph_audio::http::set_account_access_provider(Arc::new(glyph_core::account::media_access));
                glyph_audio::http::set_local_access_provider(Arc::new(glyph_core::ytdlp::local_stream_access));
                glyph_audio::http::set_cache_dir(glyph_core::paths::cache_dir().join("audio-ranges"));
                glyph_core::set_stream_warmer(Arc::new(glyph_audio::http::prewarm));
                let b = Arc::new(b);
                self.backend_state = match guard(|| b.mode()) {
                    Some(m) => BackendState::Ready(m),
                    None => BackendState::Ready(BackendMode::Direct),
                };
                self.backend = Some(b);
                glyph_core::cover::set_data_saver(self.config.storage.data_saver);
                self.load_feeds(cx);
                self.load_new_releases(cx);
                // The server may not have a working Last.fm key: check once so
                // the radio switch can be honest about it.
                self.probe_radio(cx);
                // Misses recorded without a backend are not real misses.
                self.covers.retain(|_, v| v.is_some());
                self.ascii_covers.clear();
                if self.config.updates.check_on_start {
                    self.check_updates(false, cx);
                }
                self.start_sync_loop(cx);
            }
            Err(e) => {
                tracing::warn!("backend offline: {e}");
                self.backend_state = BackendState::Offline(e);
                // Unknown instead of "broken": the switch stays usable.
                self.radio_ok = None;
            }
        }
        cx.notify();
    }

    pub fn retry_backend(&mut self, cx: &mut Context<Self>) {
        self.backend_state = BackendState::Connecting;
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            let backend = cx
                .background_executor()
                .spawn(async move { guard_async(Backend::new(config)).await })
                .await;
            this.update(cx, |s, cx| s.backend_ready(backend, cx)).ok();
        })
        .detach();
        cx.notify();
    }
}
