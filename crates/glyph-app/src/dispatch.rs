use crate::controller::{Core, PlayFrom};
use crate::jam::{Perms, Reaction};
use crate::transfer::PickedFile;
use crate::{account, commands, discover, jam, mcp_tools, plugins, storage, system, taste, transfer, video, wrong_song};
use glyph_core::{ListKey, SearchItem};
use glyph_protocol::{ProviderId, Track};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

fn req<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
    let v = args.get(key).cloned().unwrap_or(Value::Null);
    serde_json::from_value(v).map_err(|e| format!("invalid arg `{key}`: {e}"))
}

fn opt<T: DeserializeOwned>(args: &Value, key: &str) -> Result<Option<T>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value::<Option<T>>(v.clone())
            .map_err(|e| format!("invalid arg `{key}`: {e}")),
    }
}

fn ok<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn res<T: Serialize>(r: Result<T, String>) -> Result<Value, String> {
    r.and_then(ok)
}

/// Dispatches any UI command by Tauri command name and snake_case JSON args.
pub async fn dispatch(core: &Arc<Core>, cmd: &str, args: Value) -> Result<Value, String> {
    match cmd {
        // ---- desktop shell no-ops for mobile / non-desktop hosts ----
        "mini_show" | "ui_ready" => Ok(Value::Null),

        // ---- account / profile / privacy / server ----
        "account_devices_open" => {
            account::account_devices_open(core, req(&args, "open")?);
            Ok(Value::Null)
        }
        "account_status" => res(account::account_status(core).await),
        "account_login_start" => res(account::account_login_start(core).await),
        "account_login_poll" => res(account::account_login_poll(core, req(&args, "login_id")?).await),
        "account_password_submit" => res(account::account_password_submit(
            core,
            req(&args, "handle")?,
            req(&args, "password")?,
            req(&args, "register")?,
        )
        .await),
        "account_unlock_keys" => res(account::account_unlock_keys(core, req(&args, "password")?).await),
        "account_change_password" => res(account::account_change_password(core, req(&args, "password")?).await),
        "account_link_passkey" => res(account::account_link_passkey(core).await),
        "account_rename" => res(account::account_rename(core, req(&args, "handle")?).await),
        "account_approve" => res(account::account_approve(core, req(&args, "code")?).await),
        "account_recovery" => res(account::account_recovery(core).await),
        "account_remove_device" => res(account::account_remove_device(core, req(&args, "id")?).await),
        "admin_call" => res(account::admin_call(core, req(&args, "method")?, req(&args, "path")?, opt(&args, "body")?).await),
        "account_sign_out" => res(account::account_sign_out(core).await),
        "account_sign_out_all" => res(account::account_sign_out_all(core).await),
        "account_delete" => res(account::account_delete(core, req(&args, "handle")?).await),
        "account_sync_now" => res(account::account_sync_now(core).await),
        "account_merge" => res(account::account_merge(core, req(&args, "yes")?).await),
        "profile" => res(account::profile(core, req(&args, "handle")?).await),
        "profile_image" => res(account::profile_image(core, req(&args, "handle")?, req(&args, "sha")?).await),
        "profile_playlist" => res(account::profile_playlist(core, req(&args, "handle")?, req(&args, "id")?).await),
        "profile_follow" => {
            account::profile_follow(req(&args, "handle")?, req(&args, "on")?);
            Ok(Value::Null)
        }
        "profile_settings" => res(account::profile_settings(core).await),
        "profile_update" => res(account::profile_update(
            core,
            opt(&args, "public")?,
            opt(&args, "show_recent")?,
            opt(&args, "playlists")?,
            opt(&args, "sections")?,
        )
        .await),
        "profile_reorder_playlist" => res(account::profile_reorder_playlist(
            core,
            req(&args, "order")?,
            req(&args, "from")?,
            req(&args, "to")?,
        )
        .await),
        "profile_toggle_playlist" => res(account::profile_toggle_playlist(core, req(&args, "id")?).await),
        "profile_set_image" => res(account::profile_set_image(core, req(&args, "banner")?, opt(&args, "image")?).await),
        "profile_remove_image" => res(account::profile_remove_image(core, req(&args, "banner")?).await),
        "profile_pick_image" => res(account::profile_pick_image(core, req(&args, "banner")?).await),
        "privacy_report" => Ok(account::privacy_report(core)),
        "privacy_clear" => {
            account::privacy_clear(core);
            Ok(Value::Null)
        }
        "privacy_record_web_login" => {
            account::privacy_record_web_login(core);
            Ok(Value::Null)
        }
        "server_info" => Ok(account::server_info(core)),
        "server_test" => res(account::server_test(core, req(&args, "url")?).await),
        "server_store_token" => res(account::server_store_token(req(&args, "url")?, req(&args, "token")?).await),

        // ---- core playback / library / search / browse ----
        "snapshot" => Ok(commands::snapshot(core)),
        "resume_state" => Ok(core.resume_state()),
        "play" => {
            commands::play(
                core,
                req::<Vec<Track>>(&args, "tracks")?,
                req(&args, "index")?,
                opt(&args, "context")?,
                opt::<PlayFrom>(&args, "from")?,
            );
            Ok(Value::Null)
        }
        "play_shuffled" => {
            commands::play_shuffled(
                core,
                req::<Vec<Track>>(&args, "tracks")?,
                opt(&args, "context")?,
                opt::<PlayFrom>(&args, "from")?,
            );
            Ok(Value::Null)
        }
        "previous_song" => {
            commands::previous_song(core);
            Ok(Value::Null)
        }
        "play_recent" => {
            commands::play_recent(core);
            Ok(Value::Null)
        }
        "toggle_play" => {
            commands::toggle_play(core);
            Ok(Value::Null)
        }
        "next" => {
            commands::next(core);
            Ok(Value::Null)
        }
        "previous" => {
            commands::previous(core);
            Ok(Value::Null)
        }
        "seek" => {
            commands::seek(core, req(&args, "position")?);
            Ok(Value::Null)
        }
        "set_volume" => {
            commands::set_volume(core, req(&args, "volume")?);
            Ok(Value::Null)
        }
        "toggle_mute" => {
            commands::toggle_mute(core);
            Ok(Value::Null)
        }
        "set_shuffle" => {
            commands::set_shuffle(core, req(&args, "on")?);
            Ok(Value::Null)
        }
        "set_repeat" => {
            commands::set_repeat(core, req(&args, "mode")?);
            Ok(Value::Null)
        }
        "set_loop" => {
            commands::set_loop(core, opt(&args, "a")?, opt(&args, "b")?);
            Ok(Value::Null)
        }
        "queue_add" => {
            commands::queue_add(core, req(&args, "tracks")?, req(&args, "next")?);
            Ok(Value::Null)
        }
        "queue_remove" => {
            commands::queue_remove(core, req(&args, "index")?);
            Ok(Value::Null)
        }
        "queue_move" => {
            commands::queue_move(core, req(&args, "from")?, req(&args, "to")?);
            Ok(Value::Null)
        }
        "queue_jump" => {
            commands::queue_jump(core, req(&args, "index")?);
            Ok(Value::Null)
        }
        "queue_clear" => {
            commands::queue_clear(core);
            Ok(Value::Null)
        }
        "like" => {
            commands::like(core, req(&args, "track")?, req(&args, "on")?);
            Ok(Value::Null)
        }
        "like_many" => {
            commands::like_many(core, req(&args, "tracks")?, req(&args, "on")?);
            Ok(Value::Null)
        }
        "clear_liked" => {
            commands::clear_liked(core);
            Ok(Value::Null)
        }
        "playlist_create" => ok(commands::playlist_create(core, req(&args, "name")?, req(&args, "tracks")?)),
        "playlist_rename" => {
            commands::playlist_rename(core, req(&args, "id")?, req(&args, "name")?);
            Ok(Value::Null)
        }
        "playlist_delete" => {
            commands::playlist_delete(core, req(&args, "id")?);
            Ok(Value::Null)
        }
        "playlist_restore" => ok(commands::playlist_restore(core, req(&args, "id")?)),
        "playlist_purge" => ok(commands::playlist_purge(core, req(&args, "id")?)),
        "playlist_add" => ok(commands::playlist_add(core, req(&args, "id")?, req(&args, "tracks")?)),
        "playlist_remove" => {
            commands::playlist_remove(core, req(&args, "id")?, req(&args, "index")?);
            Ok(Value::Null)
        }
        "playlist_remove_many" => {
            commands::playlist_remove_many(core, req(&args, "id")?, req(&args, "indices")?);
            Ok(Value::Null)
        }
        "playlist_move" => {
            commands::playlist_move(core, req(&args, "id")?, req(&args, "from")?, req(&args, "to")?);
            Ok(Value::Null)
        }
        "list_insert" => {
            commands::list_insert(core, req::<ListKey>(&args, "list")?, req(&args, "index")?, req(&args, "track")?);
            Ok(Value::Null)
        }
        "pin" => {
            commands::pin(core, req::<ListKey>(&args, "list")?, req(&args, "on")?);
            Ok(Value::Null)
        }
        "save_album" => {
            commands::save_album(core, req::<ListKey>(&args, "list")?, req(&args, "on")?);
            Ok(Value::Null)
        }
        "follow" => {
            commands::follow(core, req(&args, "artist")?, req(&args, "on")?);
            Ok(Value::Null)
        }
        "hide_artist" => {
            commands::hide_artist(core, req(&args, "artist")?, req(&args, "on")?);
            Ok(Value::Null)
        }
        "hide_sidebar" => {
            commands::hide_sidebar(core, req(&args, "id")?);
            Ok(Value::Null)
        }
        "recent_remove" => {
            commands::recent_remove(core, req(&args, "index")?);
            Ok(Value::Null)
        }
        "recent_remove_track" => {
            commands::recent_remove_track(core, req(&args, "track")?);
            Ok(Value::Null)
        }
        "list_opened" => {
            commands::list_opened(core, req::<ListKey>(&args, "list")?);
            Ok(Value::Null)
        }
        "search_history_add" => {
            commands::search_history_add(core, req::<SearchItem>(&args, "item")?);
            Ok(Value::Null)
        }
        "search_history_remove" => {
            commands::search_history_remove(core, req(&args, "index")?);
            Ok(Value::Null)
        }
        "lyrics_offset" => {
            commands::lyrics_offset(core, req(&args, "track")?, req(&args, "offset_ms")?);
            Ok(Value::Null)
        }
        "search_paste_kind" => ok(commands::search_paste_kind(req(&args, "query")?)),
        "search" => res(commands::search(core, req(&args, "query")?, opt(&args, "page")?).await),
        "search_more" => res(commands::search_more(core, req(&args, "query")?, opt(&args, "page")?).await),
        "release_search" => {
            commands::release_search(core);
            Ok(Value::Null)
        }
        "home" => res(commands::home(core).await),
        "artist" => res(commands::artist(core, req(&args, "name")?, opt(&args, "quiet")?).await),
        "artist_photo" => res(commands::artist_photo(core, req(&args, "name")?).await),
        "artist_verified" => res(commands::artist_verified(core, req(&args, "name")?).await),
        "album" => res(commands::album(core, req(&args, "title")?, req(&args, "artist")?).await),
        "lyrics" => res(commands::lyrics(core, req(&args, "track")?).await),
        "cover_path" => res(commands::cover_path(core, req(&args, "track")?).await),
        "track_album" => res(commands::track_album(core, req(&args, "track")?).await),
        "set_config" => res(commands::set_config(core, req(&args, "config")?).await),
        "output_devices" => ok(commands::output_devices()),
        "open_url" => res(commands::open_url(req(&args, "url")?)),
        "reveal_data_dir" => res(commands::reveal_data_dir()),
        "downloaded_bytes" => ok(commands::downloaded_bytes(core, req(&args, "tracks")?)),

        // ---- discover / radio / stats / share / sleep ----
        "set_radio" => {
            discover::set_radio(core, req(&args, "on")?);
            Ok(Value::Null)
        }
        "radio_state" => res(discover::radio_state(core).await),
        "list_radio" => {
            discover::list_radio(core, req::<ListKey>(&args, "list")?);
            Ok(Value::Null)
        }
        "artist_radio" => {
            discover::artist_radio(core, req(&args, "artist")?, opt(&args, "page_tracks")?);
            Ok(Value::Null)
        }
        "song_radio" => {
            discover::song_radio(core, req(&args, "track")?, req(&args, "title")?);
            Ok(Value::Null)
        }
        "add_suggestions" => res(discover::add_suggestions(core, req(&args, "id")?).await),
        "recommend_feedback" => {
            discover::recommend_feedback(core, req(&args, "artist")?, opt(&args, "preference")?);
            Ok(Value::Null)
        }
        "taste_profile" => Ok(taste::taste_profile(core)),
        "taste_mixes" => Ok(taste::taste_mixes(core)),
        "taste_refresh" => res(taste::taste_refresh(core).await),
        "taste_onboarding_catalog" => res(taste::taste_onboarding_catalog(core).await),
        "taste_onboarding_save" => res(taste::taste_onboarding_save(
            core,
            req(&args, "artists")?,
            opt(&args, "tracks")?.unwrap_or_default(),
            opt(&args, "genres")?.unwrap_or_default(),
            opt(&args, "follow_artists")?,
        )
        .await),
        "taste_dislike_track" => Ok(taste::taste_dislike_track(
            core,
            req(&args, "track")?,
            req(&args, "disliked")?,
        )),
        "taste_tag_feedback" => Ok(taste::taste_tag_feedback(
            core,
            req(&args, "tag")?,
            opt(&args, "preference")?,
        )),
        "taste_version_feedback" => Ok(taste::taste_version_feedback(
            core,
            req(&args, "style")?,
            opt(&args, "preference")?,
        )),
        "taste_reset" => Ok(taste::taste_reset(core)),
        "lyrics_alternatives" => res(discover::lyrics_alternatives(core, req(&args, "track")?).await),
        "lyrics_pick" => res(discover::lyrics_pick(core, req(&args, "track")?, req(&args, "lyrics")?).await),
        "stats_summary" => Ok(discover::stats_summary(core, req(&args, "range")?)),
        "stats_recap" => Ok(discover::stats_recap(core, req(&args, "year")?)),
        "set_playing_from" => {
            discover::set_playing_from(core, opt::<ListKey>(&args, "list")?);
            Ok(Value::Null)
        }
        "share_songs" => res(discover::share_songs(core, req(&args, "title")?, req(&args, "tracks")?, opt(&args, "kind")?, opt(&args, "copy")?).await),
        "share" => res(discover::share(core, opt(&args, "track")?, opt::<ListKey>(&args, "list")?, opt(&args, "copy")?).await),
        "share_system" => res(commands::share_system(req(&args, "text")?, opt(&args, "title")?)),
        "copy_text" => res(commands::copy_text(req(&args, "text")?)),
        "copy_artist_link" => res(discover::copy_artist_link(core, req(&args, "artist")?, opt(&args, "copy")?).await),
        "hidden_artists" => ok(discover::hidden_artists(core)),
        "toggle_hidden_artist" => {
            discover::toggle_hidden_artist(core, req(&args, "name")?);
            Ok(Value::Null)
        }
        "onboarding_seen" => {
            discover::onboarding_seen();
            Ok(Value::Null)
        }
        "providers_cooling" => ok(discover::providers_cooling(core)),
        "playback_status" => Ok(discover::playback_status()),
        "set_sleep" => {
            discover::set_sleep(core, opt(&args, "mins")?, req(&args, "end_of_song")?);
            Ok(Value::Null)
        }

        // ---- devices & jam ----
        "devices_rescan" => res(jam::devices_rescan(core, opt(&args, "now")?).await),
        "device_select" => res(jam::device_select(core, req(&args, "id")?)),
        "select_device" => res(jam::select_device(core, req(&args, "id")?)),
        "device_connect" => res(jam::device_connect(core, req(&args, "id")?)),
        "device_use_local" => res(jam::device_use_local(core)),
        "jam_state" => Ok(jam::jam_state(core, opt(&args, "queue_all")?)),
        "jam_start" => res(jam::jam_start(core).await),
        "jam_join" => res(jam::jam_join(core, req(&args, "code")?).await),
        "jam_leave" => {
            jam::jam_leave(core, opt(&args, "code")?);
            Ok(Value::Null)
        }
        "jam_set_perms" => Ok(jam::jam_set_perms(core, req::<Perms>(&args, "perms")?)),
        "jam_set_approve" => Ok(jam::jam_set_approve(core, req(&args, "on")?)),
        "jam_admit" => Ok(jam::jam_admit(core, req(&args, "handle")?, req(&args, "allow")?)),
        "jam_kick" => Ok(jam::jam_kick(core, req(&args, "handle")?)),
        "jam_vote" => Ok(jam::jam_vote(core, req(&args, "id")?)),
        "jam_remove" => Ok(jam::jam_remove(core, req(&args, "id")?)),
        "jam_react" => Ok(jam::jam_react(core, req::<Reaction>(&args, "reaction")?)),
        "jam_set_listen_here" => Ok(jam::jam_set_listen_here(core, req(&args, "here")?)),

        // ---- mcp ----
        "mcp_pending" => ok(mcp_tools::mcp_pending()),
        "mcp_approve" => res(mcp_tools::mcp_approve(core, req(&args, "id")?, req(&args, "allow")?).await),

        // ---- plugins ----
        "plugins_state" => Ok(plugins::plugins_state(core)),
        "plugins_catalog" => res(plugins::plugins_catalog(core, opt(&args, "force")?).await),
        "plugin_install" => res(plugins::plugin_install(
            core,
            req(&args, "id")?,
            req(&args, "repo")?,
            opt(&args, "path")?,
            opt(&args, "rev")?,
        )
        .await),
        "plugin_uninstall" => res(plugins::plugin_uninstall(core, req(&args, "id")?).await),
        "plugin_enable" => {
            plugins::plugin_enable(core, req(&args, "id")?, req(&args, "on")?);
            Ok(Value::Null)
        }
        "plugins_reload" => {
            plugins::plugins_reload(core);
            Ok(Value::Null)
        }
        "plugins_folder" => res(plugins::plugins_folder()),
        "backgrounds_list" => Ok(crate::backgrounds::backgrounds_list(core)),
        "background_add" => res(crate::backgrounds::background_add(req(&args, "data")?).await),
        "background_remove" => res(crate::backgrounds::background_remove(req(&args, "id")?)),
        "plugin_set_value" => {
            plugins::plugin_set_value(core, req(&args, "plugin")?, req(&args, "item")?, req(&args, "value")?);
            Ok(Value::Null)
        }
        "plugin_action" => {
            plugins::plugin_action(core, req(&args, "plugin")?, req(&args, "item")?);
            Ok(Value::Null)
        }
        "plugin_lyrics" => ok(plugins::plugin_lyrics(core, req(&args, "key")?)),

        // ---- storage / downloads / local library ----
        "download" => {
            storage::download(core, req(&args, "tracks")?);
            Ok(Value::Null)
        }
        "download_cancel" => {
            storage::download_cancel(core, req(&args, "tracks")?);
            Ok(Value::Null)
        }
        "download_remove" => {
            storage::download_remove(core, req(&args, "track")?);
            Ok(Value::Null)
        }
        "reveal_path" => res(storage::reveal_path(req(&args, "path")?)),
        "playlist_storage" => {
            storage::playlist_storage(core, req(&args, "id")?, req(&args, "mode")?);
            Ok(Value::Null)
        }
        "playlist_auto_download" => {
            storage::playlist_auto_download(core, req(&args, "id")?, req(&args, "on")?);
            Ok(Value::Null)
        }
        "liked_auto_download" => {
            storage::liked_auto_download(core, req(&args, "on")?);
            Ok(Value::Null)
        }
        "playlist_restore_version" => ok(storage::playlist_restore_version(core, req(&args, "revision")?)),
        "playlist_set_cover" => res(storage::playlist_set_cover(core, req(&args, "id")?, req(&args, "data")?)),
        "playlist_remove_cover" => {
            storage::playlist_remove_cover(core, req(&args, "id")?);
            Ok(Value::Null)
        }
        "album_meta" => res(storage::album_meta(core, req(&args, "title")?, req(&args, "artist")?).await),
        "storage_stats" => res(storage::storage_stats(core).await),
        "storage_clear" => res(storage::storage_clear(core, req(&args, "kind")?).await),
        "storage_set_download_dir" => res(storage::storage_set_download_dir(core, opt(&args, "pick")?, opt(&args, "dir")?).await),
        "pick_folders" => res(storage::pick_folders(core, opt(&args, "prompt")?).await),
        "local_scan" => res(storage::local_scan(core, req(&args, "folders")?).await),
        "local_page" => res(storage::local_page(
            core,
            req(&args, "tab")?,
            opt(&args, "dir")?,
            opt(&args, "artist")?,
            opt(&args, "album")?,
            opt(&args, "query")?,
        )
        .await),
        "local_play" => res(storage::local_play(
            core,
            req(&args, "tab")?,
            opt(&args, "dir")?,
            opt(&args, "artist")?,
            opt(&args, "album")?,
            opt(&args, "query")?,
            opt(&args, "shuffle")?,
        )
        .await),

        // ---- system / eq / updates ----
        "settings_code" => ok(system::settings_code(core)),
        "settings_file" => Ok(system::settings_file(core)),
        "settings_import" => res(system::settings_import(core, req(&args, "text")?).await),
        "settings_undo" => res(system::settings_undo(core).await),
        "set_eq_live" => {
            system::set_eq_live(core, req(&args, "enabled")?, req(&args, "gains")?);
            Ok(Value::Null)
        }
        "eq_code" => ok(system::eq_code(req(&args, "name")?, req(&args, "gains")?)),
        "eq_file" => Ok(system::eq_file(req(&args, "name")?, req(&args, "gains")?)),
        "eq_import" => res(system::eq_import(req(&args, "text")?)),
        "eq_preset_save" => {
            system::eq_preset_save(core, req(&args, "name")?);
            Ok(Value::Null)
        }
        "set_autostart" => res(system::set_autostart(req(&args, "on")?)),
        "diagnostics" => ok(system::diagnostics(core)),
        "retry_backend" => {
            system::retry_backend(core);
            Ok(Value::Null)
        }
        "mcp_info" => Ok(system::mcp_info(core)),
        "mcp_rotate_token" => {
            system::mcp_rotate_token(core);
            Ok(Value::Null)
        }
        "mcp_server_token" => {
            mcp_tools::request_server_token(core, opt(&args, "name")?).await;
            Ok(Value::Null)
        }
        "mcp_sessions" => res(mcp_tools::sessions(core).await),
        "mcp_rename_session" => res(mcp_tools::rename_session(core, req(&args, "id")?, req(&args, "name")?).await),
        "mcp_revoke_session" => res(mcp_tools::revoke_session(core, req(&args, "id")?).await),
        "ytdlp_status" => res(system::ytdlp_status(core, opt(&args, "force")?).await),
        "ytdlp_install" => res(system::ytdlp_install(core).await),
        "prefetch_hover" => {
            system::prefetch_hover(core, req(&args, "track")?);
            Ok(Value::Null)
        }
        "update_state" => Ok(system::update_state()),
        "app_outdated" => Ok(system::app_outdated()),
        "update_check" => res(system::update_check(core, opt(&args, "manual")?).await),
        "update_install" => res(system::update_install(core).await),
        "update_restart" => {
            system::update_restart(core);
            Ok(Value::Null)
        }

        // ---- transfer / import / migrate ----
        "import_parse_url" => ok(transfer::import_parse_url(req(&args, "url")?)),
        "import_link" => res(transfer::import_link(core, req(&args, "url")?, opt(&args, "name")?, opt(&args, "playlist_id")?).await),
        "import_playlist_file" => res(transfer::import_playlist_file(core, opt(&args, "playlist_id")?).await),
        "import_text" => res(transfer::import_text(core, req(&args, "text")?, opt(&args, "ask")?).await),
        "share_save" => ok(transfer::share_save(core, req(&args, "code")?, opt(&args, "link")?, opt(&args, "author")?)),
        "share_read" => res(transfer::share_read(core, req(&args, "text")?).await),
        "share_read_files" => ok(transfer::share_read_files(core, req(&args, "paths")?).await),
        "share_save_list" => ok(transfer::share_save_list(core, req(&args, "name")?, opt(&args, "origin")?, req(&args, "tracks")?)),
        "migrate_resume_state" => Ok(ok(transfer::migrate_resume_state())?),
        "migrate_discard" => {
            transfer::migrate_discard(core);
            Ok(Value::Null)
        }
        "migrate_pasted" => ok(transfer::migrate_pasted(req(&args, "text")?)),
        "migrate_overview" => res(transfer::migrate_overview(
            core,
            req(&args, "service")?,
            opt(&args, "profile")?,
            opt(&args, "pasted")?,
            opt::<Vec<PickedFile>>(&args, "files")?,
            opt(&args, "resume")?,
        )
        .await),
        "migrate_start" => {
            res(transfer::migrate_start(core, req(&args, "service")?, req(&args, "selected")?, opt(&args, "targets")?).await)
        }
        "migrate_progress" => Ok(transfer::migrate_progress(core)),
        "migrate_cancel" => {
            transfer::migrate_cancel(core);
            Ok(Value::Null)
        }

        // ---- music video ----
        "video_search" => res(video::search(core, req(&args, "track")?, opt(&args, "query")?).await),
        "video_resolve" => res(video::resolve(core, req(&args, "id")?).await),

        // ---- wrong song ("Not the right one") ----
        "wrong_song_alternatives" => res(wrong_song::alternatives(core, req(&args, "track")?, opt(&args, "query")?).await),
        "wrong_song_preview" => {
            wrong_song::preview_start(core, req(&args, "track")?, opt(&args, "preview_url")?).await?;
            Ok(Value::Null)
        }
        "wrong_song_preview_stop" => {
            wrong_song::preview_stop(core);
            Ok(Value::Null)
        }
        "wrong_song_select" => res(
            wrong_song::select(
                core,
                req(&args, "original")?,
                req(&args, "chosen")?,
                opt::<ProviderId>(&args, "wrong_provider")?,
                opt(&args, "wrong_id")?,
            )
            .await,
        ),

        other => Err(format!("unknown command: {other}")),
    }
}
