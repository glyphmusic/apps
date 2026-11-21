//! Glyph desktop app on Tauri: the Svelte UI (`glyph-ui`) in a window,
//! glyph-core and glyph-audio behind it; see `docs/TAURI.md`.

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod clip;
mod desktop;
mod host;
mod tauri_commands;

#[allow(dead_code)]
mod autostart;
#[allow(dead_code)]
mod discord;
#[allow(dead_code)]
mod instance;
#[allow(dead_code)]
mod integrate;
#[cfg(windows)]
#[allow(dead_code)]
mod jumplist;
mod memory_guard;
#[cfg(windows)]
#[allow(dead_code)]
mod playback_qos;
#[cfg(windows)]
#[allow(dead_code)]
mod thumbbar;
#[cfg(windows)]
#[allow(dead_code)]
mod uninstall;

use glyph_app::controller::Core;
use std::sync::Arc;
use tauri::Manager;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    glyph_app::guard::install_panic_hook();
    // The uninstaller deleting all app data: only clean up, never start.
    #[cfg(windows)]
    if uninstall::requested() {
        uninstall::forget_credentials();
        return;
    }
    if !desktop::before_start() {
        return;
    }
    memory_guard::configure_browser_env();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // glyph:// links: macOS hands them to the running app; Windows and
        // Linux start Glyph again with the link (instance.rs forwards it).
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            glyph_app::rt::set_handle(tauri::async_runtime::block_on(async {
                tokio::runtime::Handle::current()
            }));
            // Covers and artist photos are cached by glyph-core; the webview
            // may load files from there and nowhere else on disk.
            let scope = app.asset_protocol_scope();
            for dir in [glyph_core::paths::cache_dir(), glyph_core::paths::data_dir()] {
                if let Err(e) = scope.allow_directory(&dir, true) {
                    tracing::warn!(dir = %dir.display(), error = %e, "asset scope");
                }
            }
            let core = Core::new(Arc::new(host::TauriHost::new(app.handle().clone())));
            let win_cfg = app.config().app.windows[0].clone();
            let wb = tauri::WebviewWindowBuilder::from_config(app.handle(), &win_cfg)?;
            #[cfg(target_os = "macos")]
            let wb = wb
                .decorations(true)
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
                .traffic_light_position(tauri::LogicalPosition::new(14.0, 24.0));
            let w = wb.build()?;
            desktop::harden_webview(&w);
            // The YouTube embed player draws into the main window.
            if let Some(p) = &core.player {
                use raw_window_handle::HasWindowHandle;
                if let Ok(h) = w.window_handle() {
                    if let Err(e) = p.attach_embed_host(h.as_raw()) {
                        tracing::warn!(error = %e, "embed host");
                    }
                }
            }
            glyph_app::setup(&core);
            desktop::setup(app, &core);
            memory_guard::start(app.handle().clone(), core.clone());
            app.manage(core);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(core) = window.app_handle().try_state::<Arc<Core>>() {
                    core.save_all_now();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            tauri_commands::snapshot,
            tauri_commands::resume_state,
            tauri_commands::play,
            tauri_commands::play_shuffled,
            tauri_commands::play_recent,
            tauri_commands::toggle_play,
            tauri_commands::next,
            tauri_commands::previous,
            tauri_commands::seek,
            tauri_commands::set_volume,
            tauri_commands::toggle_mute,
            tauri_commands::set_shuffle,
            tauri_commands::set_repeat,
            tauri_commands::set_loop,
            tauri_commands::queue_add,
            tauri_commands::queue_remove,
            tauri_commands::queue_move,
            tauri_commands::queue_jump,
            tauri_commands::queue_clear,
            tauri_commands::like,
            tauri_commands::like_many,
            tauri_commands::clear_liked,
            tauri_commands::playlist_create,
            tauri_commands::playlist_rename,
            tauri_commands::playlist_delete,
            tauri_commands::playlist_restore,
            tauri_commands::playlist_purge,
            tauri_commands::playlist_add,
            tauri_commands::playlist_remove,
            tauri_commands::playlist_remove_many,
            tauri_commands::playlist_move,
            tauri_commands::list_insert,
            tauri_commands::pin,
            tauri_commands::save_album,
            tauri_commands::follow,
            tauri_commands::hide_artist,
            tauri_commands::hide_sidebar,
            tauri_commands::recent_remove,
            tauri_commands::recent_remove_track,
            tauri_commands::list_opened,
            tauri_commands::search_history_add,
            tauri_commands::search_history_remove,
            tauri_commands::lyrics_offset,
            tauri_commands::search,
            tauri_commands::search_more,
            tauri_commands::release_search,
            tauri_commands::home,
            tauri_commands::artist,
            tauri_commands::album,
            tauri_commands::lyrics,
            tauri_commands::cover_path,
            tauri_commands::track_album,
            tauri_commands::set_config,
            tauri_commands::output_devices,
            tauri_commands::open_url,
            tauri_commands::reveal_data_dir,
            tauri_commands::downloaded_bytes,
            desktop::mini_show,
            desktop::ui_ready,
            tauri_commands::set_radio,
            tauri_commands::radio_state,
            tauri_commands::list_radio,
            tauri_commands::artist_radio,
            tauri_commands::song_radio,
            tauri_commands::add_suggestions,
            tauri_commands::recommend_feedback,
            tauri_commands::lyrics_alternatives,
            tauri_commands::lyrics_pick,
            tauri_commands::stats_summary,
            tauri_commands::stats_recap,
            tauri_commands::share_songs,
            tauri_commands::share,
            tauri_commands::share_system,
            tauri_commands::copy_text,
            tauri_commands::copy_artist_link,
            tauri_commands::hidden_artists,
            tauri_commands::toggle_hidden_artist,
            tauri_commands::onboarding_seen,
            tauri_commands::providers_cooling,
            tauri_commands::playback_status,
            tauri_commands::set_sleep,
            tauri_commands::download,
            tauri_commands::download_cancel,
            tauri_commands::download_remove,
            tauri_commands::playlist_storage,
            tauri_commands::playlist_auto_download,
            tauri_commands::liked_auto_download,
            tauri_commands::playlist_restore_version,
            tauri_commands::playlist_set_cover,
            tauri_commands::playlist_remove_cover,
            tauri_commands::album_meta,
            tauri_commands::storage_stats,
            tauri_commands::storage_clear,
            tauri_commands::storage_set_download_dir,
            tauri_commands::pick_folders,
            tauri_commands::local_scan,
            tauri_commands::local_page,
            tauri_commands::local_play,
            tauri_commands::settings_code,
            tauri_commands::settings_file,
            tauri_commands::settings_import,
            tauri_commands::settings_undo,
            tauri_commands::eq_code,
            tauri_commands::eq_file,
            tauri_commands::eq_import,
            tauri_commands::eq_preset_save,
            tauri_commands::set_eq_live,
            tauri_commands::set_autostart,
            tauri_commands::diagnostics,
            tauri_commands::retry_backend,
            tauri_commands::mcp_info,
            tauri_commands::mcp_rotate_token,
            tauri_commands::mcp_server_token,
            tauri_commands::ytdlp_status,
            tauri_commands::ytdlp_install,
            tauri_commands::update_state,
            tauri_commands::app_outdated,
            tauri_commands::update_check,
            tauri_commands::update_install,
            tauri_commands::update_restart,
            tauri_commands::mcp_approve,
            tauri_commands::mcp_sessions,
            tauri_commands::mcp_rename_session,
            tauri_commands::mcp_revoke_session,
            tauri_commands::mcp_claim_oauth,
            tauri_commands::previous_song,
            tauri_commands::search_paste_kind,
            tauri_commands::artist_photo,
            tauri_commands::artist_verified,
            tauri_commands::plugin_lyrics,
            tauri_commands::share_save,
            tauri_commands::mcp_pending,
            tauri_commands::account_devices_open,
            tauri_commands::set_playing_from,
            tauri_commands::reveal_path,
            tauri_commands::prefetch_hover,
            tauri_commands::account_approve,
            tauri_commands::account_change_password,
            tauri_commands::account_delete,
            tauri_commands::account_link_passkey,
            tauri_commands::account_login_poll,
            tauri_commands::account_login_start,
            tauri_commands::account_merge,
            tauri_commands::account_password_submit,
            tauri_commands::account_recovery,
            tauri_commands::account_remove_device,
            tauri_commands::admin_call,
            tauri_commands::account_rename,
            tauri_commands::account_sign_out,
            tauri_commands::account_sign_out_all,
            tauri_commands::account_status,
            tauri_commands::account_sync_now,
            tauri_commands::account_unlock_keys,
            tauri_commands::profile,
            tauri_commands::profile_follow,
            tauri_commands::profile_image,
            tauri_commands::profile_pick_image,
            tauri_commands::profile_playlist,
            tauri_commands::profile_remove_image,
            tauri_commands::profile_reorder_playlist,
            tauri_commands::profile_set_image,
            tauri_commands::profile_settings,
            tauri_commands::profile_toggle_playlist,
            tauri_commands::profile_update,
            tauri_commands::privacy_clear,
            tauri_commands::privacy_record_web_login,
            tauri_commands::privacy_report,
            tauri_commands::server_info,
            tauri_commands::server_store_token,
            tauri_commands::server_test,
            tauri_commands::jam_state,
            tauri_commands::jam_start,
            tauri_commands::jam_join,
            tauri_commands::jam_leave,
            tauri_commands::jam_set_perms,
            tauri_commands::jam_set_approve,
            tauri_commands::jam_admit,
            tauri_commands::jam_kick,
            tauri_commands::jam_vote,
            tauri_commands::jam_remove,
            tauri_commands::jam_react,
            tauri_commands::jam_set_listen_here,
            tauri_commands::devices_rescan,
            tauri_commands::device_select,
            tauri_commands::select_device,
            tauri_commands::device_connect,
            tauri_commands::device_use_local,
            tauri_commands::import_link,
            tauri_commands::import_parse_url,
            tauri_commands::import_playlist_file,
            tauri_commands::import_text,
            tauri_commands::migrate_cancel,
            tauri_commands::migrate_discard,
            tauri_commands::migrate_overview,
            tauri_commands::migrate_pasted,
            tauri_commands::migrate_progress,
            tauri_commands::migrate_resume_state,
            tauri_commands::migrate_start,
            tauri_commands::plugin_action,
            tauri_commands::plugin_enable,
            tauri_commands::plugin_install,
            tauri_commands::plugin_set_value,
            tauri_commands::plugin_uninstall,
            tauri_commands::plugins_catalog,
            tauri_commands::plugins_folder,
            tauri_commands::plugins_reload,
            tauri_commands::plugins_state,
            tauri_commands::backgrounds_list,
            tauri_commands::background_add,
            tauri_commands::background_remove,
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Glyph")
        .run(|app, event| match event {
            // macOS: a click on the Dock icon opens the window Glyph started without.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => desktop::show_window(app),
            // macOS: audio files opened with Glyph (Finder "Open With", a
            // double click once Glyph is the default player).
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Opened { urls } => {
                let files: Vec<String> = urls
                    .iter()
                    .filter(|u| u.scheme() == "file")
                    .filter_map(|u| u.to_file_path().ok())
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                if let Some(req) = instance::files_request(&files) {
                    instance::post(&req);
                }
            }
            tauri::RunEvent::Exit => {
                if let Some(core) = app.try_state::<Arc<Core>>() {
                    core.save_if_dirty();
                }
            }
            _ => {}
        });
}
