use super::*;

impl Glyph {
    pub(crate) fn prepare_account_sign_in(&mut self, cx: &mut Context<Self>) -> bool {
        let mut guest = self.config.clone();
        guest.keys = guest.persisted_keys();
        if let Err(e) = glyph_core::paths::write_json(
            &glyph_core::paths::data_dir().join("guest-settings.json"),
            &guest,
        ) {
            self.account.notice = Some((e.to_string(), false));
            cx.notify();
            return false;
        }
        let value = serde_json::json!({ "server": self.config.effective_server_url() });
        if let Err(e) =
            glyph_core::paths::write_json(&glyph_core::account::guest_merge_marker(), &value)
        {
            self.account.notice = Some((e.to_string(), false));
            cx.notify();
            return false;
        }
        true
    }
    pub(crate) fn reload_account_feeds(&mut self, cx: &mut Context<Self>) {
        self.account.data_epoch += 1;
        self.recs.clear();
        self.trending.clear();
        self.new_releases.clear();
        self.feeds_loading = false;
        self.new_releases_loading = false;
        self.load_feeds(cx);
        self.load_new_releases(cx);
    }
    pub(super) fn ensure_account_library(&mut self, cx: &mut Context<Self>) {
        if self.account.merge_pending
            && !self.account.signing_out
            && self.backend.as_ref().is_some_and(|b| b.account().is_none())
        {
            self.account_leave(cx);
            return;
        }
        if !self.library_ok
            || !self.config_loaded
            || self.account.signing_out
            || self.account.merge_pending
        {
            return;
        }
        let Some(b) = self.backend.as_ref() else {
            return;
        };
        let path = self.library.storage_path.0.as_ref();
        match b.account() {
            Some(acc) if glyph_core::account::guest_merge_pending(&acc.server) => {
                self.account_enter(cx)
            }
            None if path
                .is_some_and(|p| p.starts_with(glyph_core::paths::data_dir().join("accounts"))) =>
            {
                self.account_leave(cx)
            }
            Some(acc)
                if path.is_some_and(|p| {
                    p.starts_with(glyph_core::paths::data_dir().join("accounts"))
                        && *p != glyph_core::account::library_path_for(&acc)
                }) =>
            {
                self.account_choose_merge(false, cx)
            }
            _ => {}
        }
    }
    pub(crate) fn account_enter(&mut self, cx: &mut Context<Self>) {
        // Even a failed cache read must keep background sync blocked.
        self.account.merge_pending = true;
        if !glyph_core::sync::snapshot(&self.library, &[])
            .items
            .is_empty()
        {
            self.open_settings(Some(crate::app::SettingsPage::Account), cx);
        } else {
            self.account_choose_merge(false, cx);
        }
    }

    pub fn account_choose_merge(&mut self, merge: bool, cx: &mut Context<Self>) {
        if self.account.signing_out {
            return;
        }
        self.account.merge_pending = true;
        if self.library.storage_path.0.as_ref() == Some(&Library::guest_path()) {
            self.save_library(cx);
        }
        let Some(acc) = self.signed_in() else { return };
        let path = glyph_core::account::library_path_for(&acc);
        let mut lib = match glyph_core::Library::load_from(&path) {
            Ok(lib) => lib,
            Err(e) => {
                self.account.notice = Some((e.to_string(), false));
                cx.notify();
                return;
            }
        };
        if let Err(e) = lib.load_local_playlists() {
            self.account.notice = Some((e.to_string(), false));
            cx.notify();
            return;
        }
        if merge {
            // Keep the accepted import durable until a fresh account pull has
            // completed, so old remote values cannot overwrite guest additions.
            if let Err(e) = self
                .library
                .save_to(&path.with_file_name("pending-guest.json"))
            {
                self.account.notice = Some((e.to_string(), false));
                cx.notify();
                return;
            }
        }
        if !merge {
            let source =
                glyph_core::paths::read_json::<Config>(&path.with_file_name("settings.json"))
                    .unwrap_or_default();
            let mut cfg = self.config.clone();
            glyph_core::settings_sync::restore_local_preferences(&mut cfg, &source);
            if let Err(e) = cfg.save() {
                self.account.notice = Some((e.to_string(), false));
                cx.notify();
                return;
            }
            self.adopt_synced_config(cfg, true, cx);
        }
        if let Err(e) = lib.save_to(&path) {
            self.account.notice = Some((e.to_string(), false));
            cx.notify();
            return;
        }
        if let Err(e) = std::fs::remove_file(glyph_core::account::guest_merge_marker()) {
            if e.kind() != std::io::ErrorKind::NotFound {
                self.account.notice = Some((e.to_string(), false));
                cx.notify();
                return;
            }
        }
        self.library = lib;
        self.account.merge_pending = false;
        self.account.last_sync = None;
        self.home_activity_cache = None;
        if let Some(b) = self.backend.as_ref() {
            b.reset_library_sync();
        }
        self.save_library(cx);
        self.sync_now(cx);
        self.reload_account_feeds(cx);
        cx.notify();
    }

    pub(crate) fn account_leave(&mut self, cx: &mut Context<Self>) {
        let _ = std::fs::remove_file(glyph_core::account::guest_merge_marker());
        self.flush_stats(cx);
        self.save_library(cx);
        if let Some(path) = self
            .library
            .storage_path
            .0
            .as_ref()
            .filter(|p| p.starts_with(glyph_core::paths::data_dir().join("accounts")))
        {
            let mut cfg = self.config.clone();
            cfg.keys = cfg.persisted_keys();
            if let Err(e) =
                glyph_core::paths::write_json(&path.with_file_name("settings.json"), &cfg)
            {
                tracing::warn!("account settings cache: {e}");
            }
        }
        let source = glyph_core::paths::read_json::<Config>(
            &glyph_core::paths::data_dir().join("guest-settings.json"),
        )
        .unwrap_or_default();
        let mut cfg = self.config.clone();
        glyph_core::settings_sync::restore_local_preferences(&mut cfg, &source);
        if let Err(e) = cfg.save() {
            tracing::warn!("guest settings: {e}");
        }
        self.adopt_synced_config(cfg, true, cx);
        self.account.sync_gen += 1;
        self.account.signing_out = false;
        self.account.merge_pending = false;
        self.account.syncing = false;
        self.account.outputs.clear();
        self.drop_account_live();
        self.account.output_live_connecting = false;
        self.account.output_live_checked = None;
        self.account.output_seen.clear();
        self.account.transfer_ack = None;
        self.account.transfer_ack_at = None;
        self.account.transfer_to = None;
        self.account.output_error_at = None;
        self.account.output_playing.set(None);
        self.account.output_failures = 0;
        self.account.output_missing_since = None;
        self.account.output_received = None;
        self.account.sync_failures = 0;
        self.account.sync_pending = 0;
        self.account.output_target = None;
        self.account.output_checked = None;
        self.account.output_busy = false;
        self.account.transfer_resume = None;
        self.jam_leave(cx);
        self.activity_dirty = None;
        self.stats_pending = None;
        self.with_player(|p| p.stop());
        self.queue.clear();
        self.current = None;
        self.resolving = false;
        self.preloaded = None;
        self.playing_from = None;
        self.queue_from = None;
        self.palette = None;
        self.cover = None;
        self.lyrics = Default::default();
        self.search = Default::default();
        self.profile = Default::default();
        self.section = None;
        self.open_profile = None;
        self.last_load = None;
        self.history.clear();
        self.ctx_menu = None;
        self.ctx_closing = None;
        self.name_dialog = None;
        self.add_songs = None;
        self.shared_preview = None;
        self.selected_row = None;
        self.open_artist = None;
        self.pending_recent_artist = None;
        self.recent_artist_ctx = None;
        self.view = View::Home;
        self.open_list = None;
        self.home_activity_cache = None;
        let guest_path = glyph_core::Library::guest_path();
        match glyph_core::Library::load_from(&guest_path) {
            Ok(lib) => {
                self.library = lib;
                self.library_ok = true;
            }
            Err(e) => {
                tracing::warn!("guest library: {e}");
                self.library = Default::default();
                self.library.storage_path.0 = Some(guest_path);
                self.library_ok = false;
            }
        }
        if let Err(e) = self.library.load_local_playlists() {
            tracing::warn!("local playlists: {e}");
            self.library_ok = false;
        }
        self.reload_account_feeds(cx);
        cx.notify();
    }
}
