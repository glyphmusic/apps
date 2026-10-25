use super::*;

impl Glyph {
    // ------------------------------------------------------------ settings modal

    /// 0..1 how far the settings modal is shown (eased open / close).
    pub fn settings_k(&self) -> f32 {
        if self.settings_open {
            let k = (self.settings_at.elapsed().as_secs_f32() / 0.18).min(1.0);
            1.0 - (1.0 - k).powi(3)
        } else if let Some(at) = self.settings_closing {
            let k = (at.elapsed().as_secs_f32() / 0.14).min(1.0);
            (1.0 - k).powi(2)
        } else {
            0.0
        }
    }

    pub fn open_settings(&mut self, page: Option<SettingsPage>, cx: &mut Context<Self>) {
        if !self.settings_open {
            self.settings_at = Instant::now();
        }
        self.settings_closing = None;
        self.settings_open = true;
        // Phone layout: a given page opens directly, otherwise the list.
        self.settings_in_page = page.is_some();
        if let Some(p) = page {
            if p != self.settings_page {
                self.settings_sub = 0;
            }
            self.settings_page = p;
        }
        self.open_select = None;
        if self.settings_page == SettingsPage::Storage {
            self.refresh_storage(cx);
        }
        if self.settings_page == SettingsPage::Plugins {
            self.refresh_plugin_catalog(false, cx);
        }
        self.refresh_output_devices(cx);
        cx.notify();
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_key_capture(cx);
        if self.settings_open {
            self.settings_closing = Some(Instant::now());
        }
        self.settings_open = false;
        self.open_select = None;
        let f = self.focus.clone();
        window.focus(&f, cx);
        cx.notify();
    }

    pub fn settings_group_k(&self, page: SettingsPage, open: bool) -> f32 {
        self.settings_group_motion.get(&(page as usize)).map(|(at,from,to)| {
            let k = (at.elapsed().as_secs_f32() / 0.18).min(1.0);
            let ease = k * k * (3.0 - 2.0 * k);
            from + (to - from) * ease
        }).unwrap_or(if open { 1.0 } else { 0.0 })
    }

    pub fn set_settings_page(&mut self, page: SettingsPage, cx: &mut Context<Self>) {
        self.set_settings_tab(page, 0, cx);
    }

    /// Opens a settings page at one of its sub-tabs.
    pub fn set_settings_tab(&mut self, page: SettingsPage, sub: u8, cx: &mut Context<Self>) {
        if (page, sub) != (self.settings_page, self.settings_sub) {
            self.settings_scroll.set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
        }
        // Any tab click leaves a plugin's detail view (and its warning).
        self.plugins.detail = None;
        self.plugins.confirm = None;
        if page != self.settings_page {
            // The page being left folds its sub-tabs away instead of vanishing.
            let old = self.settings_page;
            let from = self.settings_group_k(old, !self.settings_collapsed.contains(&(old as usize)));
            if from > 0.001 {
                self.settings_group_motion.insert(old as usize, (Instant::now(), from, 0.0));
            }
        }
        self.settings_collapsed.remove(&(page as usize));
        self.settings_group_motion.remove(&(page as usize));
        self.settings_page = page;
        self.settings_sub = sub;
        self.open_select = None;
        if page == SettingsPage::Storage {
            self.refresh_storage(cx);
        }
        if page == SettingsPage::Account {
            self.account_page_opened(cx);
        }
        if page == SettingsPage::Profile {
            self.profile_page_opened(cx);
        }
        if page == SettingsPage::Plugins {
            self.refresh_plugin_catalog(false, cx);
        }
        cx.notify();
    }

    pub fn toggle_select(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.open_select = if self.open_select == Some(id) { None } else { Some(id) };
        cx.notify();
    }

    /// Measures the storage folders off the UI thread.
    pub fn refresh_storage(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let stats = cx.background_executor().spawn(async { glyph_core::storage::stats() }).await;
            this.update(cx, |s, cx| {
                s.storage = Some(stats);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn clear_storage(&mut self, kind: glyph_core::storage::StorageKind, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { glyph_core::storage::clear(kind) }).await;
            this.update(cx, |s, cx| {
                if let Err(e) = res {
                    s.toast_info(format!("{e:#}"));
                }
                if kind == glyph_core::storage::StorageKind::Covers {
                    s.covers.clear();
                    for (_, im) in s.ascii_covers.drain() {
                        if let Some(im) = im {
                            s.stale_heroes.push(im);
                        }
                    }
                }
                if kind == glyph_core::storage::StorageKind::Catalog {
                    s.album_meta.clear();
                    let open = s.open_artist.clone();
                    s.artists.retain(|k, p| Some(k) == open.as_ref() || p.loading || p.info_loading);
                    if let Some(b) = &s.backend {
                        b.clear_catalog_caches();
                    }
                }
                if kind == glyph_core::storage::StorageKind::Downloads {
                    if let Some(b) = &s.backend {
                        b.reload_downloads();
                    }
                }
                s.refresh_storage(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Cover cache limit in MB (0 = no limit); applies on the next backend build.
    pub fn set_cache_limit(&mut self, mb: u64, cx: &mut Context<Self>) {
        self.config.storage.cache_limit_mb = mb;
        self.save_config(cx);
        self.push_config_to_backend(cx);
        cx.notify();
    }

    /// Crossfade between songs, 0 (off) to 20 s.
    pub fn set_crossfade(&mut self, secs: u32, cx: &mut Context<Self>) {
        if self.config.audio.crossfade_s == secs.min(20) {
            return;
        }
        self.config.audio.crossfade_s = secs.min(20);
        if let Some(p) = &self.player {
            p.set_crossfade(Duration::from_secs(self.config.audio.crossfade_s as u64));
        }
        self.save_config(cx);
        cx.notify();
    }

    pub fn set_volume_level(&mut self, l: glyph_core::config::VolumeLevel, cx: &mut Context<Self>) {
        self.config.audio.level = l;
        self.open_select = None;
        if let Some(p) = &self.player {
            p.set_loudness(level_of(l), self.config.audio.normalize);
        }
        self.save_config(cx);
        cx.notify();
    }

    /// Equalizer changes apply live; the config is written when asked
    /// (`save`), so dragging a band does not write the file on every move.
    pub fn set_eq(&mut self, enabled: bool, preset: &str, gains: [f32; 10], save: bool, cx: &mut Context<Self>) {
        self.config.audio.eq_enabled = enabled;
        self.config.audio.eq_preset = preset.to_string();
        self.config.audio.eq_gains = gains.to_vec();
        self.open_select = None;
        if let Some(p) = &self.player {
            p.set_equalizer(enabled, gains);
        }
        if save {
            self.save_config(cx);
        }
        cx.notify();
    }

    /// Bass shelf and headroom of the equalizer (dB); like [`Self::set_eq`]
    /// they apply live and are written when asked.
    pub fn set_eq_extras(&mut self, bass_db: f32, headroom_db: f32, save: bool, cx: &mut Context<Self>) {
        let a = &mut self.config.audio;
        a.eq_bass_db = (bass_db.clamp(-12.0, 12.0) * 2.0).round() / 2.0;
        a.eq_headroom_db = (headroom_db.clamp(0.0, 12.0) * 2.0).round() / 2.0;
        if let Some(p) = &self.player {
            p.set_eq_extras(a.eq_bass_db, a.eq_headroom_db);
        }
        if save {
            self.save_config(cx);
        }
        cx.notify();
    }

    /// The curve as it is now, named after its preset (for sharing).
    pub(super) fn current_eq_preset(&self) -> glyph_core::eq::EqPreset {
        let name = crate::ui::settings::eq_preset_label(&self.config.audio.eq_preset);
        glyph_core::eq::EqPreset::new(&name, eq_gains(&self.config.audio.eq_gains))
    }

    /// Asks for a name and saves the current curve as an own preset.
    pub fn open_eq_save_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.config.audio.eq_preset.strip_prefix("user:").unwrap_or_default().to_string();
        self.playlist_name.update(cx, |i, cx| i.set_value(current, cx));
        self.ctx_menu = None;
        self.open_select = None;
        self.name_dialog = Some(NameDialog { tracks: Vec::new(), rename: None, eq_preset: true, initial_playlist: None });
        let f = self.playlist_name.read(cx).focus.clone();
        window.focus(&f, cx);
        cx.notify();
    }

    /// Adds an own preset (one with the same name is replaced) and selects it.
    pub fn add_eq_preset(&mut self, p: glyph_core::eq::EqPreset, cx: &mut Context<Self>) {
        let gains = p.gains();
        let list = &mut self.config.audio.eq_user_presets;
        match list.iter_mut().find(|e| e.name.eq_ignore_ascii_case(&p.name)) {
            Some(e) => *e = p.clone(),
            None => list.push(p.clone()),
        }
        self.set_eq(true, &format!("user:{}", p.name), gains, true, cx);
    }

    /// Removes an own preset; the curve stays as it is.
    pub fn delete_eq_preset(&mut self, name: &str, cx: &mut Context<Self>) {
        self.config.audio.eq_user_presets.retain(|p| p.name != name);
        if self.config.audio.eq_preset.strip_prefix("user:") == Some(name) {
            self.config.audio.eq_preset = "custom".into();
        }
        self.open_select = None;
        self.save_config(cx);
        self.toast_sub(t("toast.eq_deleted"), name.to_string());
        cx.notify();
    }

    /// Copies a share code for the current curve.
    pub fn copy_eq_code(&mut self, cx: &mut Context<Self>) {
        let p = self.current_eq_preset();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(glyph_core::eq::encode(&p)));
        self.open_select = None;
        self.toast_sub(t("toast.eq_copied"), p.name);
        cx.notify();
    }

    /// Imports an equalizer share code as an own preset; false when the
    /// text is no such code.
    pub fn import_eq_code(&mut self, code: &str, cx: &mut Context<Self>) -> bool {
        if !glyph_core::eq::is_code(code) && !code.trim().starts_with('{') {
            return false;
        }
        match glyph_core::eq::decode(code) {
            Ok(p) => {
                let name = p.name.clone();
                self.add_eq_preset(p, cx);
                self.toast_sub(t("toast.eq_imported"), name);
            }
            Err(e) => self.toast_info(format!("{e:#}")),
        }
        cx.notify();
        true
    }

    /// Imports the equalizer code on the clipboard.
    pub fn paste_eq_code(&mut self, cx: &mut Context<Self>) {
        self.open_select = None;
        let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
        if !self.import_eq_code(&text, cx) {
            self.toast_info(t("toast.eq_no_code"));
            cx.notify();
        }
    }

    /// Saves the current curve as a `.glyph-eq` file.
    pub fn export_eq_file(&mut self, cx: &mut Context<Self>) {
        self.open_select = None;
        let p = self.current_eq_preset();
        let file_name = format!("{}.{}", p.name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-"), glyph_core::eq::FILE_EXT);
        let dir = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(glyph_core::paths::data_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(&file_name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let body = glyph_core::eq::to_file(&p);
            let saved = cx.background_executor().spawn(async move { std::fs::write(&path, body).map_err(|e| e.to_string()) }).await;
            this.update(cx, |s, cx| {
                match saved {
                    Ok(()) => s.toast_sub(t("toast.eq_exported"), p.name.clone()),
                    Err(e) => s.toast_info(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Imports a `.glyph-eq` file as an own preset.
    pub fn import_eq_file(&mut self, cx: &mut Context<Self>) {
        self.open_select = None;
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("sp.eq.import_file").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let text = cx.background_executor().spawn(async move { std::fs::read_to_string(path) }).await;
            this.update(cx, |s, cx| {
                match text {
                    Ok(text) if s.import_eq_code(&text, cx) => {}
                    Ok(_) => s.toast_info(t("toast.eq_no_code")),
                    Err(e) => s.toast_info(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Re-reads the list of audio outputs (off the UI thread).
    pub fn refresh_output_devices(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let list = cx.background_executor().spawn(async move { glyph_audio::PlayerController::output_devices() }).await;
            this.update(cx, |s, cx| {
                s.output_devices = list;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn set_output_device(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.config.audio.output_device = name.clone();
        self.open_select = None;
        if let Some(p) = &self.player {
            p.set_output_device(name);
        }
        self.save_config(cx);
        cx.notify();
    }

    /// Mono, stereo or spatial sound (Settings > Audio, volume menu).
    pub fn set_channel_mode(&mut self, mode: glyph_core::config::ChannelMode, cx: &mut Context<Self>) {
        self.config.audio.channels = mode;
        self.open_select = None;
        if let Some(p) = &self.player {
            p.set_channel_mode(channels_of(mode));
        }
        self.save_config(cx);
        cx.notify();
    }

    pub fn toggle_normalize(&mut self, cx: &mut Context<Self>) {
        self.config.audio.normalize = !self.config.audio.normalize;
        if let Some(p) = &self.player {
            p.set_loudness(level_of(self.config.audio.level), self.config.audio.normalize);
        }
        self.save_config(cx);
        cx.notify();
    }

    pub fn set_audio_quality(&mut self, q: glyph_core::config::AudioQuality, cx: &mut Context<Self>) {
        self.config.audio.quality = q;
        if let Some(player) = &self.player {
            player.set_stream_quality(match q {
                glyph_core::config::AudioQuality::Auto => "auto",
                glyph_core::config::AudioQuality::Low => "low",
                glyph_core::config::AudioQuality::Standard => "standard",
                glyph_core::config::AudioQuality::High => "high",
            });
        }
        self.open_select = None;
        self.save_config(cx);
        self.push_config_to_backend(cx);
        cx.notify();
    }

    /// Quality of new downloads (songs already saved stay as they are).
    pub fn set_download_quality(&mut self, q: glyph_core::config::AudioQuality, cx: &mut Context<Self>) {
        self.config.audio.download_quality = q;
        self.open_select = None;
        self.save_config(cx);
        self.push_config_to_backend(cx);
        cx.notify();
    }

    /// Version, OS and setup summary for bug reports (no keys, no personal data).
    pub fn diagnostics(&self) -> String {
        let mode = match &self.backend_state {
            BackendState::Ready(BackendMode::Server { url, server_version }) => format!("server {url} v{server_version}"),
            BackendState::Ready(m) => format!("{m:?}"),
            other => format!("{other:?}"),
        };
        format!(
            "Glyph {}
OS: {} {}
Target: {}
Protocol: v{}
Backend: {}
Providers: {:?}
Language: {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            glyph_core::updater::current_target(),
            glyph_protocol::PROTOCOL_VERSION,
            mode,
            self.config.providers.enabled,
            self.lang.code(),
        )
    }

    pub fn set_music_reactivity(&mut self, v: f32, cx: &mut Context<Self>) {
        self.config.visuals.music_reactivity = v.clamp(0.0, 1.0);
        self.glow.update(cx, |g, _| g.set_music_reactivity(v));
        self.save_config(cx);
        cx.notify();
    }

    /// Settings -> Appearance -> colour theme.
    pub fn set_theme(&mut self, id: &str, cx: &mut Context<Self>) {
        self.config.visuals.theme = id.to_string();
        crate::theme::set_theme(id);
        let glow = crate::theme::glow_palette(id);
        self.glow.update(cx, |g, _| g.set_theme(glow));
        self.save_config(cx);
        cx.notify();
    }

    /// Background shape: the one picked in Settings > Appearance, else
    /// `fallback` (the song's or the page's own).
    pub(super) fn glow_shape(&self, fallback: Preset) -> Preset {
        Preset::from_name(&self.config.visuals.glow_shape).unwrap_or(fallback)
    }

    /// Settings > Appearance: one background shape always ("" = a new one
    /// with every song).
    pub fn set_glow_shape(&mut self, name: &str, cx: &mut Context<Self>) {
        self.config.visuals.glow_shape = name.to_string();
        let seed = self.current.as_ref().map(|t| format!("{}{}", t.artist, t.title));
        let preset = self.glow_shape(seed.map(|s| Preset::from_seed(&s)).unwrap_or(Preset::Strahl));
        self.glow.update(cx, |g, _| g.set_preset(preset));
        self.save_config(cx);
        cx.notify();
    }

    /// Settings > Appearance: text size factor (0.9 ..= 1.3).
    pub fn set_text_scale(&mut self, v: f32, cx: &mut Context<Self>) {
        self.config.ui.text_scale = ((v.clamp(0.9, 1.3)) * 20.0).round() / 20.0;
        crate::theme::set_text_scale(self.config.ui.text_scale());
        self.save_config(cx);
        cx.notify();
    }

    /// Settings > Player: one of the device-local player options changed.
    pub fn set_ui(&mut self, f: impl FnOnce(&mut glyph_core::config::UiConfig), cx: &mut Context<Self>) {
        let before = self.config.ui.keys.clone();
        f(&mut self.config.ui);
        if before != self.config.ui.keys {
            self.rebind_keys(cx);
        }
        self.show_lyrics = self.config.ui.np_lyrics;
        self.save_config(cx);
        cx.notify();
    }

    /// Shortcuts live in GPUI's keymap: bind everything again.
    fn rebind_keys(&mut self, cx: &mut Context<Self>) {
        cx.clear_key_bindings();
        crate::ui::bind_keys(cx, &self.config.ui);
    }

    /// Settings › About › Shortcuts: the next key pressed becomes the key of
    /// shortcut `id` (Esc cancels, Delete / Backspace removes it).
    pub fn start_key_capture(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.key_capture = Some(id);
        // Nothing may fire while the key is recorded.
        cx.clear_key_bindings();
        let f = self.focus.clone();
        window.focus(&f, cx);
        cx.notify();
    }

    /// Stops recording (no change).
    pub fn cancel_key_capture(&mut self, cx: &mut Context<Self>) {
        if self.key_capture.take().is_some() {
            self.rebind_keys(cx);
            cx.notify();
        }
    }

    /// A key pressed while recording.
    pub fn capture_key(&mut self, k: &gpui::Keystroke, cx: &mut Context<Self>) {
        use crate::ui::keys;
        let Some(id) = self.key_capture else { return };
        let plain = !(k.modifiers.control || k.modifiers.alt || k.modifiers.platform || k.modifiers.shift || k.modifiers.function);
        if plain && k.key == "escape" {
            self.cancel_key_capture(cx);
            return;
        }
        let key = if plain && matches!(k.key.as_str(), "delete" | "backspace") {
            String::new()
        } else {
            // A modifier on its own: wait for the key that goes with it.
            let Some(key) = keys::from_keystroke(k) else { return };
            if keys::FIXED.iter().any(|f| keys::same_key(f, &key)) {
                self.toast_info(tf("toast.key_fixed", &[("key", &keys::key_label(&key))]));
                cx.notify();
                return;
            }
            key
        };
        // Another shortcut had this key: it gives it up.
        if !key.is_empty() {
            for s in keys::SHORTCUTS.iter().filter(|s| s.id != id) {
                if keys::same_key(&keys::key_of(&self.config.ui, s.id), &key) {
                    self.config.ui.keys.insert(s.id.to_string(), String::new());
                    self.toast_info(tf("toast.key_moved", &[("key", &keys::key_label(&key)), ("other", &t(s.label))]));
                }
            }
        }
        let default = keys::shortcut(id).map_or("", |s| s.default);
        if keys::same_key(&key, default) || (key.is_empty() && default.is_empty()) {
            self.config.ui.keys.remove(id);
        } else {
            self.config.ui.keys.insert(id.to_string(), key);
        }
        self.key_capture = None;
        self.rebind_keys(cx);
        self.save_config(cx);
        cx.notify();
    }

    /// Puts one shortcut (or, with `None`, all) back to its default key.
    pub fn reset_keys(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        match id {
            Some(id) => {
                self.config.ui.keys.remove(id);
                // A shortcut that took this key meanwhile gives it up.
                let key = crate::ui::keys::key_of(&self.config.ui, id);
                for s in crate::ui::keys::SHORTCUTS.iter().filter(|s| s.id != id) {
                    if crate::ui::keys::same_key(&crate::ui::keys::key_of(&self.config.ui, s.id), &key) {
                        self.config.ui.keys.insert(s.id.to_string(), String::new());
                    }
                }
            }
            None => self.config.ui.keys.clear(),
        }
        self.key_capture = None;
        self.rebind_keys(cx);
        self.save_config(cx);
        cx.notify();
    }

    /// Settings > Storage: move the downloads to a folder the user picks.
    pub fn pick_download_dir(&mut self, cx: &mut Context<Self>) {
        self.open_select = None;
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t("sp.storage.dl_dir.pick").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(dir) = paths.into_iter().next() else { return };
            this.update(cx, |s, cx| s.set_download_dir(Some(dir), cx)).ok();
        })
        .detach();
    }

    /// Moves the downloads to `dir` (`None`: back to Glyph's own folder).
    pub fn set_download_dir(&mut self, dir: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        if !self.downloading.is_empty() {
            self.toast_info(t("toast.dl_dir_busy"));
            cx.notify();
            return;
        }
        self.toast(t("toast.dl_dir_moving"));
        cx.spawn(async move |this, cx| {
            let res = guard_async(b.set_download_dir(dir.clone())).await;
            this.update(cx, |s, cx| {
                match res {
                    Ok(r) => {
                        s.config.storage.download_dir = dir.map(|d| d.to_string_lossy().into_owned());
                        s.save_config(cx);
                        if r.failed > 0 {
                            s.toast_info(tf("toast.dl_dir_failed", &[("n", &r.failed)]));
                        } else {
                            s.toast(tf("toast.dl_dir_done", &[("n", &r.moved)]).to_string());
                        }
                        s.refresh_storage(cx);
                    }
                    Err(e) => s.toast_info(format!("{e:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Settings > Appearance: background frame rate while Glyph is in the background.
    pub fn set_throttle_unfocused(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.visuals.throttle_unfocused = on;
        self.glow.update(cx, |g, _| g.set_throttle_inactive(on));
        self.save_config(cx);
        cx.notify();
    }

    /// Start with the computer on / off (Settings > System, desktop only).
    pub fn set_autostart(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Err(e) = crate::autostart::set(on) {
            tracing::warn!("autostart: {e:#}");
            self.toast(tf("sp.system.autostart.error", &[("e", &e)]));
            return;
        }
        self.config.desktop.autostart = on;
        self.save_config(cx);
        cx.notify();
    }

    /// A start with the computer opens the window (false) or stays in the tray.
    pub fn set_start_in_tray(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.desktop.start_in_tray = on;
        self.save_config(cx);
        cx.notify();
    }

    /// Brings the main window up (tray "Open", a second start of Glyph);
    /// it may never have been shown after a start in the background.
    pub(super) fn show_window(&mut self, cx: &mut Context<Self>) {
        let handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        });
    }

    pub fn set_intensity(&mut self, v: f32, cx: &mut Context<Self>) {
        self.config.visuals.intensity = v.clamp(0.0, 1.0);
        self.glow.update(cx, |g, _| g.set_intensity(v));
        self.save_config(cx);
        cx.notify();
    }

    pub fn toggle_provider(&mut self, p: ProviderId, cx: &mut Context<Self>) {
        let en = &mut self.config.providers.enabled;
        if let Some(i) = en.iter().position(|x| *x == p) {
            en.remove(i);
        } else {
            en.push(p);
        }
        self.save_config(cx);
        cx.notify();
    }

    /// Drag-and-drop reorder of the source priority list.
    pub fn reorder_priority(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let pr = &mut self.config.providers.priority;
        if from == to || from >= pr.len() || to >= pr.len() {
            return;
        }
        let p = pr.remove(from);
        pr.insert(to, p);
        let p = pr.clone();
        if let Some(b) = &self.backend {
            guard(|| b.set_priority(p));
        }
        self.save_config(cx);
        cx.notify();
    }

    pub fn move_priority(&mut self, i: usize, up: bool, cx: &mut Context<Self>) {
        let pr = &mut self.config.providers.priority;
        let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < pr.len()) };
        if let Some(j) = j {
            pr.swap(i, j);
            let p = pr.clone();
            if let Some(b) = &self.backend {
                guard(|| b.set_priority(p));
            }
            self.save_config(cx);
        }
        cx.notify();
    }

    /// Connection test: builds a throw-away backend for the entered URL and
    /// reports the decided mode.
    pub fn test_connection(&mut self, cx: &mut Context<Self>) {
        let mut cfg = self.config.clone();
        cfg.server_url = self.server_input.read(cx).value.trim().to_string();
        if cfg.effective_server_url().is_empty() {
            self.conn_test = Some(ConnTest::Ok(t("settings.server.direct").to_string()));
            cx.notify();
            return;
        }
        self.server_probe_seq += 1;
        let seq = self.server_probe_seq;
        self.conn_test = Some(ConnTest::Running);
        cx.spawn(async move |this, cx| {
            let url = cfg.effective_server_url().to_string();
            let r = cx
                .background_executor()
                .spawn(async move {
                    let b = guard_async(Backend::new(cfg)).await?;
                    Ok::<_, String>((b.mode(), b.server_auth_required()))
                })
                .await;
            this.update(cx, |s, cx| {
                if s.server_probe_seq != seq || s.config.effective_server_url() != url { return; }
                if let Ok((_, required)) = &r { s.server_auth_required = *required; }
                s.conn_test = Some(match r {
                    Ok((BackendMode::Server { server_version, .. }, required)) => {
                        if server_version == "offline" {
                            s.conn_test = Some(ConnTest::Failed(if required { t("settings.server.bearer.required").to_string() } else { t("settings.server.unreachable").to_string() }));
                            cx.notify();
                            return;
                        }
                        ConnTest::Ok(tf("settings.server.ok", &[("url", &url), ("v", &server_version)]).to_string())
                    }
                    Ok((BackendMode::Incompatible { protocol_version, .. }, _)) => ConnTest::Failed(
                        tf("status.incompatible", &[("v", &protocol_version)]).to_string(),
                    ),
                    Ok((BackendMode::Direct, _)) => {
                        ConnTest::Failed(tf("settings.server.unreachable", &[("url", &url)]).to_string())
                    }
                    Err(e) => ConnTest::Failed(e),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn apply_server(&mut self, cx: &mut Context<Self>) {
        self.config.server_url = self.server_input.read(cx).value.trim().to_string();
        self.save_config(cx);
        let base = self.config.effective_server_url().to_string();
        let token = self.server_bearer_input.read(cx).value.trim().to_string();
        if self.server_auth_required && !token.is_empty() {
            let expected_base = base.clone();
            cx.spawn(async move |this,cx| {
                let result = cx.background_executor().spawn(async move { glyph_core::server_access::store(&base,&token).map_err(|e| e.to_string()) }).await;
                this.update(cx,|s,cx| {
                    if s.config.effective_server_url() != expected_base { return; }
                    match result {
                    Ok(()) => { s.server_bearer_input.update(cx, |i,cx| i.set_value(String::new(),cx)); s.retry_backend(cx); },
                    Err(e) => { s.conn_test = Some(ConnTest::Failed(e)); cx.notify(); }
                    }
                }).ok();
            }).detach();
        } else { self.retry_backend(cx); }
    }

    pub fn check_updates(&mut self, manual: bool, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else {
            self.update.error = Some(t("status.backend_offline").to_string());
            cx.notify();
            return;
        };
        self.update.checking = true;
        self.update.error = None;
        cx.spawn(async move |this, cx| {
            let r = if manual { guard_async(b.check_for_update()).await } else { guard_async(b.check_for_update_auto()).await };
            this.update(cx, |s, cx| {
                s.update.checking = false;
                match r {
                    Ok(UpdateStatus::Skipped) if !manual => {}
                    Ok(st) => {
                        let auto = s.config.updates.auto_install;
                        if let (UpdateStatus::Available(info), true) = (&st, auto) {
                            if info.can_self_install && !manual {
                                let info = info.clone();
                                s.update.status = Some(st);
                                s.install_update(info, cx);
                                return;
                            }
                        }
                        s.update.status = Some(st);
                    }
                    Err(e) => s.update.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn available_update(&self) -> Option<&UpdateInfo> {
        match &self.update.status {
            Some(UpdateStatus::Available(i)) => Some(i),
            _ => None,
        }
    }

    pub fn install_update(&mut self, info: UpdateInfo, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        self.update.installing = Some((0, 0));
        let (tx, mut rx) = flume_like::channel::<(u64, u64)>();
        cx.spawn(async move |this, cx| {
            while let Some(p) = rx.recv().await {
                if this
                    .update(cx, |s, cx| {
                        s.update.installing = Some(p);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.install_update(info, tx)).await;
            this.update(cx, |s, cx| {
                s.update.installing = None;
                match r {
                    Ok(()) => {
                        s.update.installed = true;
                        // Android: the system installer (or first its
                        // permission screen) takes over from here.
                        if cfg!(target_os = "android") {
                            s.toast_info(t("update.android_confirm"));
                        }
                    }
                    Err(e) => s.update.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Rebuilds the backend with the current config (sources, qualities,
    /// cache limit, update channel). The future must be awaited: dropping it
    /// unpolled changed nothing.
    pub(super) fn push_config_to_backend(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        let cfg = self.config.clone();
        cx.spawn(async move |_, _| {
            if let Err(e) = guard_async(b.set_config(cfg)).await {
                tracing::warn!("applying settings to the backend: {e}");
            }
        })
        .detach();
    }

    pub fn set_update_channel(&mut self, prerelease: bool, cx: &mut Context<Self>) {
        self.config.updates.channel = if prerelease { "prerelease" } else { "stable" }.into();
        self.save_config(cx);
        // The update check reads the backend's config.
        self.push_config_to_backend(cx);
        cx.notify();
    }

    pub fn toggle_auto_update(&mut self, cx: &mut Context<Self>) {
        self.config.updates.auto_install = !self.config.updates.auto_install;
        self.save_config(cx);
        cx.notify();
    }

    pub fn restart(&mut self, cx: &mut App) {
        cx.restart();
    }
}
