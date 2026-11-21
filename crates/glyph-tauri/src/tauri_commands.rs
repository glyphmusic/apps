//! Thin `#[tauri::command]` forwarders to `glyph_app`.

use glyph_app::controller::{Core, PlayFrom};
use glyph_app::jam::{Perms, Reaction};
use glyph_app::transfer::PickedFile;
use glyph_app::{account, commands, discover, jam, mcp_tools, plugins, storage, system, transfer};
use glyph_core::{ListKey, SearchItem};
use glyph_protocol::mcp::{Approval, Session as McpSession};
use glyph_protocol::Track;
use serde_json::Value;
use std::sync::Arc;
use tauri::State;

type Res<T = ()> = Result<T, String>;
type C<'a> = State<'a, Arc<Core>>;

// ---- core playback / library / search / browse ----

#[tauri::command(rename_all = "snake_case")]
pub fn snapshot(core: C) -> Value {
    commands::snapshot(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn resume_state(core: C) -> Value {
    core.resume_state()
}

#[tauri::command(rename_all = "snake_case")]
pub fn play(core: C, tracks: Vec<Track>, index: usize, context: Option<String>, from: Option<PlayFrom>) {
    commands::play(&core, tracks, index, context, from)
}

#[tauri::command(rename_all = "snake_case")]
pub fn play_shuffled(core: C, tracks: Vec<Track>, context: Option<String>, from: Option<PlayFrom>) {
    commands::play_shuffled(&core, tracks, context, from)
}

#[tauri::command(rename_all = "snake_case")]
pub fn previous_song(core: C) {
    commands::previous_song(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn play_recent(core: C) {
    commands::play_recent(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn toggle_play(core: C) {
    commands::toggle_play(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn next(core: C) {
    commands::next(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn previous(core: C) {
    commands::previous(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn seek(core: C, position: f64) {
    commands::seek(&core, position)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_volume(core: C, volume: f32) {
    commands::set_volume(&core, volume)
}

#[tauri::command(rename_all = "snake_case")]
pub fn toggle_mute(core: C) {
    commands::toggle_mute(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_shuffle(core: C, on: bool) {
    commands::set_shuffle(&core, on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_repeat(core: C, mode: String) {
    commands::set_repeat(&core, mode)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_loop(core: C, a: Option<f64>, b: Option<f64>) {
    commands::set_loop(&core, a, b)
}

#[tauri::command(rename_all = "snake_case")]
pub fn queue_add(core: C, tracks: Vec<Track>, next: bool) {
    commands::queue_add(&core, tracks, next)
}

#[tauri::command(rename_all = "snake_case")]
pub fn queue_remove(core: C, index: usize) {
    commands::queue_remove(&core, index)
}

#[tauri::command(rename_all = "snake_case")]
pub fn queue_move(core: C, from: usize, to: usize) {
    commands::queue_move(&core, from, to)
}

#[tauri::command(rename_all = "snake_case")]
pub fn queue_jump(core: C, index: usize) {
    commands::queue_jump(&core, index)
}

#[tauri::command(rename_all = "snake_case")]
pub fn queue_clear(core: C) {
    commands::queue_clear(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn like(core: C, track: Track, on: bool) {
    commands::like(&core, track, on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn like_many(core: C, tracks: Vec<Track>, on: bool) {
    commands::like_many(&core, tracks, on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn clear_liked(core: C) {
    commands::clear_liked(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_create(core: C, name: String, tracks: Vec<Track>) -> String {
    commands::playlist_create(&core, name, tracks)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_rename(core: C, id: String, name: String) {
    commands::playlist_rename(&core, id, name)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_delete(core: C, id: String) {
    commands::playlist_delete(&core, id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_restore(core: C, id: String) -> bool {
    commands::playlist_restore(&core, id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_purge(core: C, id: String) -> bool {
    commands::playlist_purge(&core, id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_add(core: C, id: String, tracks: Vec<Track>) -> usize {
    commands::playlist_add(&core, id, tracks)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_remove(core: C, id: String, index: usize) {
    commands::playlist_remove(&core, id, index)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_remove_many(core: C, id: String, indices: Vec<usize>) {
    commands::playlist_remove_many(&core, id, indices)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_move(core: C, id: String, from: usize, to: usize) {
    commands::playlist_move(&core, id, from, to)
}

#[tauri::command(rename_all = "snake_case")]
pub fn list_insert(core: C, list: ListKey, index: usize, track: Track) {
    commands::list_insert(&core, list, index, track)
}

#[tauri::command(rename_all = "snake_case")]
pub fn pin(core: C, list: ListKey, on: bool) {
    commands::pin(&core, list, on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn save_album(core: C, list: ListKey, on: bool) {
    commands::save_album(&core, list, on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn follow(core: C, artist: String, on: bool) {
    commands::follow(&core, artist, on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn hide_artist(core: C, artist: String, on: bool) {
    commands::hide_artist(&core, artist, on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn hide_sidebar(core: C, id: String) {
    commands::hide_sidebar(&core, id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn recent_remove(core: C, index: usize) {
    commands::recent_remove(&core, index)
}

#[tauri::command(rename_all = "snake_case")]
pub fn recent_remove_track(core: C, track: Track) {
    commands::recent_remove_track(&core, track)
}

#[tauri::command(rename_all = "snake_case")]
pub fn list_opened(core: C, list: ListKey) {
    commands::list_opened(&core, list)
}

#[tauri::command(rename_all = "snake_case")]
pub fn search_history_add(core: C, item: SearchItem) {
    commands::search_history_add(&core, item)
}

#[tauri::command(rename_all = "snake_case")]
pub fn search_history_remove(core: C, index: usize) {
    commands::search_history_remove(&core, index)
}

#[tauri::command(rename_all = "snake_case")]
pub fn lyrics_offset(core: C, track: Track, offset_ms: i64) {
    commands::lyrics_offset(&core, track, offset_ms)
}

#[tauri::command(rename_all = "snake_case")]
pub fn search_paste_kind(query: String) -> Option<&'static str> {
    commands::search_paste_kind(query)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn search(core: C<'_>, query: String, page: Option<usize>) -> Res<Value> {
    commands::search(&core, query, page).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn search_more(core: C<'_>, query: String, page: Option<usize>) -> Res<Value> {
    commands::search_more(&core, query, page).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn release_search(core: C) {
    commands::release_search(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn home(core: C<'_>) -> Res<Value> {
    commands::home(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn artist(core: C<'_>, name: String, quiet: Option<bool>) -> Res<Value> {
    commands::artist(&core, name, quiet).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn artist_photo(core: C<'_>, name: String) -> Res<Option<String>> {
    commands::artist_photo(&core, name).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn artist_verified(core: C<'_>, name: String) -> Res<bool> {
    commands::artist_verified(&core, name).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn album(core: C<'_>, title: String, artist: String) -> Res<Vec<Track>> {
    commands::album(&core, title, artist).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn lyrics(core: C<'_>, track: Track) -> Res<Option<Value>> {
    commands::lyrics(&core, track).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn cover_path(core: C<'_>, track: Track) -> Res<Option<String>> {
    commands::cover_path(&core, track).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn track_album(core: C<'_>, track: Track) -> Res<Option<String>> {
    commands::track_album(&core, track).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn set_config(core: C<'_>, config: Value) -> Res {
    commands::set_config(&core, config).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn output_devices() -> Vec<String> {
    commands::output_devices()
}

#[tauri::command(rename_all = "snake_case")]
pub fn open_url(url: String) -> Res {
    commands::open_url(url)
}

#[tauri::command(rename_all = "snake_case")]
pub fn reveal_data_dir() -> Res {
    commands::reveal_data_dir()
}

#[tauri::command(rename_all = "snake_case")]
pub fn downloaded_bytes(core: C, tracks: Vec<Track>) -> u64 {
    commands::downloaded_bytes(&core, tracks)
}

// ---- discover ----

#[tauri::command(rename_all = "snake_case")]
pub fn set_radio(core: C<'_>, on: bool) {
    discover::set_radio(&core, on)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn radio_state(core: C<'_>) -> Res<Value> {
    discover::radio_state(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn list_radio(core: C, list: ListKey) {
    discover::list_radio(&core, list)
}

#[tauri::command(rename_all = "snake_case")]
pub fn artist_radio(core: C, artist: String, page_tracks: Option<Vec<Track>>) {
    discover::artist_radio(&core, artist, page_tracks)
}

#[tauri::command(rename_all = "snake_case")]
pub fn song_radio(core: C, track: Track, title: String) {
    discover::song_radio(&core, track, title)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn add_suggestions(core: C<'_>, id: String) -> Res<Vec<Track>> {
    discover::add_suggestions(&core, id).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn recommend_feedback(core: C, artist: String, preference: Option<i8>) {
    discover::recommend_feedback(&core, artist, preference)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn lyrics_alternatives(core: C<'_>, track: Track) -> Res<Vec<Value>> {
    discover::lyrics_alternatives(&core, track).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn lyrics_pick(core: C<'_>, track: Track, lyrics: Value) -> Res {
    discover::lyrics_pick(&core, track, lyrics).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn stats_summary(core: C, range: String) -> Value {
    discover::stats_summary(&core, range)
}

#[tauri::command(rename_all = "snake_case")]
pub fn stats_recap(core: C, year: String) -> Value {
    discover::stats_recap(&core, year)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_playing_from(core: C, list: Option<ListKey>) {
    discover::set_playing_from(&core, list)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn share_songs(core: C<'_>, title: String, tracks: Vec<Track>, kind: Option<String>, copy: Option<bool>) -> Res<String> {
    discover::share_songs(&core, title, tracks, kind, copy).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn share(core: C<'_>, track: Option<Track>, list: Option<ListKey>, copy: Option<bool>) -> Res<String> {
    discover::share(&core, track, list, copy).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn share_system(text: String, title: Option<String>) -> Res {
    commands::share_system(text, title)
}

#[tauri::command(rename_all = "snake_case")]
pub fn copy_text(text: String) -> Res {
    commands::copy_text(text)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn copy_artist_link(core: C<'_>, artist: String, copy: Option<bool>) -> Res<String> {
    discover::copy_artist_link(&core, artist, copy).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn hidden_artists(core: C) -> Vec<String> {
    discover::hidden_artists(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn toggle_hidden_artist(core: C, name: String) {
    discover::toggle_hidden_artist(&core, name)
}

#[tauri::command(rename_all = "snake_case")]
pub fn onboarding_seen() {
    discover::onboarding_seen()
}

#[tauri::command(rename_all = "snake_case")]
pub fn providers_cooling(core: C) -> Vec<Value> {
    discover::providers_cooling(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playback_status() -> Value {
    discover::playback_status()
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_sleep(core: C, mins: Option<u32>, end_of_song: bool) {
    discover::set_sleep(&core, mins, end_of_song)
}

// ---- storage ----

#[tauri::command(rename_all = "snake_case")]
pub fn download(core: C, tracks: Vec<Track>) {
    storage::download(&core, tracks)
}

#[tauri::command(rename_all = "snake_case")]
pub fn download_remove(core: C, track: Track) {
    storage::download_remove(&core, track)
}

#[tauri::command(rename_all = "snake_case")]
pub fn reveal_path(path: String) -> Res {
    storage::reveal_path(path)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_storage(core: C, id: String, mode: u8) {
    storage::playlist_storage(&core, id, mode)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_auto_download(core: C, id: String, on: bool) {
    storage::playlist_auto_download(&core, id, on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn liked_auto_download(core: C, on: bool) {
    storage::liked_auto_download(&core, on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_restore_version(core: C, revision: String) -> bool {
    storage::playlist_restore_version(&core, revision)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_set_cover(core: C, id: String, data: String) -> Res {
    storage::playlist_set_cover(&core, id, data)
}

#[tauri::command(rename_all = "snake_case")]
pub fn playlist_remove_cover(core: C, id: String) {
    storage::playlist_remove_cover(&core, id)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn album_meta(core: C<'_>, title: String, artist: String) -> Res<Value> {
    storage::album_meta(&core, title, artist).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn storage_stats(core: C<'_>) -> Res<Value> {
    storage::storage_stats(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn storage_clear(core: C<'_>, kind: String) -> Res {
    storage::storage_clear(&core, kind).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn storage_set_download_dir(core: C<'_>, pick: Option<bool>, dir: Option<String>) -> Res {
    storage::storage_set_download_dir(&core, pick, dir).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pick_folders(core: C<'_>, prompt: Option<String>) -> Res<Option<Vec<String>>> {
    storage::pick_folders(&core, prompt).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn local_scan(core: C<'_>, folders: Vec<String>) -> Res<Value> {
    storage::local_scan(&core, folders).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn local_page(
    core: C<'_>,
    tab: u8,
    dir: Option<String>,
    artist: Option<String>,
    album: Option<(String, String)>,
    query: Option<String>,
) -> Res<Value> {
    storage::local_page(&core, tab, dir, artist, album, query).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn local_play(
    core: C<'_>,
    tab: u8,
    dir: Option<String>,
    artist: Option<String>,
    album: Option<(String, String)>,
    query: Option<String>,
    shuffle: Option<bool>,
) -> Res {
    storage::local_play(&core, tab, dir, artist, album, query, shuffle).await
}

// ---- system ----

#[tauri::command(rename_all = "snake_case")]
pub fn settings_code(core: C) -> String {
    system::settings_code(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn settings_file(core: C) -> Value {
    system::settings_file(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn settings_import(core: C<'_>, text: String) -> Res<Value> {
    system::settings_import(&core, text).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn settings_undo(core: C<'_>) -> Res {
    system::settings_undo(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_eq_live(core: C, enabled: bool, gains: Vec<f32>) {
    system::set_eq_live(&core, enabled, gains)
}

#[tauri::command(rename_all = "snake_case")]
pub fn eq_code(name: String, gains: Vec<f32>) -> String {
    system::eq_code(name, gains)
}

#[tauri::command(rename_all = "snake_case")]
pub fn eq_file(name: String, gains: Vec<f32>) -> Value {
    system::eq_file(name, gains)
}

#[tauri::command(rename_all = "snake_case")]
pub fn eq_import(text: String) -> Res<Option<Value>> {
    system::eq_import(text)
}

#[tauri::command(rename_all = "snake_case")]
pub fn eq_preset_save(core: C, name: String) {
    system::eq_preset_save(&core, name)
}

#[tauri::command(rename_all = "snake_case")]
pub fn set_autostart(on: bool) -> Res {
    system::set_autostart(on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn diagnostics(core: C) -> String {
    system::diagnostics(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn retry_backend(core: C) {
    system::retry_backend(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn mcp_info(core: C) -> Value {
    system::mcp_info(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn mcp_rotate_token(core: C) {
    system::mcp_rotate_token(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mcp_server_token(core: C<'_>, name: Option<String>) -> Res {
    commands::mcp_server_token(&core, name).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn ytdlp_status(core: C<'_>, force: Option<bool>) -> Res<Value> {
    system::ytdlp_status(&core, force).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn ytdlp_install(core: C<'_>) -> Res<Value> {
    system::ytdlp_install(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn prefetch_hover(core: C, track: Track) {
    system::prefetch_hover(&core, track)
}

#[tauri::command(rename_all = "snake_case")]
pub fn app_outdated() -> Value {
    system::app_outdated()
}

#[tauri::command(rename_all = "snake_case")]
pub fn update_state() -> Value {
    system::update_state()
}

#[tauri::command(rename_all = "snake_case")]
pub async fn update_check(core: C<'_>, manual: Option<bool>) -> Res<Value> {
    system::update_check(&core, manual).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn update_install(core: C<'_>) -> Res {
    system::update_install(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn update_restart(core: C) {
    system::update_restart(&core)
}

// ---- mcp ----

#[tauri::command(rename_all = "snake_case")]
pub fn mcp_pending() -> Vec<Approval> {
    mcp_tools::mcp_pending()
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mcp_approve(core: C<'_>, id: String, allow: bool) -> Res {
    mcp_tools::mcp_approve(&core, id, allow).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mcp_sessions(core: C<'_>) -> Res<Vec<McpSession>> {
    commands::mcp_sessions(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mcp_rename_session(core: C<'_>, id: String, name: String) -> Res<McpSession> {
    commands::mcp_rename_session(&core, id, name).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mcp_revoke_session(core: C<'_>, id: String) -> Res {
    commands::mcp_revoke_session(&core, id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn mcp_claim_oauth(core: C<'_>, id: String) -> Res<Approval> {
    commands::mcp_claim_oauth(&core, id).await
}

// ---- account / profile / privacy / server ----

#[tauri::command(rename_all = "snake_case")]
pub fn account_devices_open(core: C<'_>, open: bool) {
    account::account_devices_open(&core, open)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_status(core: C<'_>) -> Res<Value> {
    account::account_status(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_login_start(core: C<'_>) -> Res<Value> {
    account::account_login_start(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_login_poll(core: C<'_>, login_id: String) -> Res<Value> {
    account::account_login_poll(&core, login_id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_password_submit(core: C<'_>, handle: String, password: String, register: bool) -> Res<Value> {
    account::account_password_submit(&core, handle, password, register).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_unlock_keys(core: C<'_>, password: String) -> Res {
    account::account_unlock_keys(&core, password).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_change_password(core: C<'_>, password: String) -> Res {
    account::account_change_password(&core, password).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_link_passkey(core: C<'_>) -> Res<Value> {
    account::account_link_passkey(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_rename(core: C<'_>, handle: String) -> Res<Value> {
    account::account_rename(&core, handle).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_approve(core: C<'_>, code: String) -> Res<Value> {
    account::account_approve(&core, code).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_recovery(core: C<'_>) -> Res<String> {
    account::account_recovery(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn admin_call(core: C<'_>, method: String, path: String, body: Option<Value>) -> Res<Value> {
    account::admin_call(&core, method, path, body).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_remove_device(core: C<'_>, id: String) -> Res {
    account::account_remove_device(&core, id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_sign_out(core: C<'_>) -> Res {
    account::account_sign_out(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_sign_out_all(core: C<'_>) -> Res {
    account::account_sign_out_all(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_delete(core: C<'_>, handle: String) -> Res {
    account::account_delete(&core, handle).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_sync_now(core: C<'_>) -> Res {
    account::account_sync_now(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_merge(core: C<'_>, yes: bool) -> Res {
    account::account_merge(&core, yes).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn profile(core: C<'_>, handle: String) -> Res<Value> {
    account::profile(&core, handle).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn profile_image(core: C<'_>, handle: String, sha: String) -> Res<Option<String>> {
    account::profile_image(&core, handle, sha).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn profile_playlist(core: C<'_>, handle: String, id: String) -> Res<Vec<Track>> {
    account::profile_playlist(&core, handle, id).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn profile_follow(handle: String, on: bool) {
    account::profile_follow(handle, on)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn profile_settings(core: C<'_>) -> Res<Value> {
    account::profile_settings(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn profile_update(
    core: C<'_>,
    public: Option<bool>,
    show_recent: Option<bool>,
    playlists: Option<Vec<String>>,
    sections: Option<Vec<String>>,
) -> Res {
    account::profile_update(&core, public, show_recent, playlists, sections).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn profile_reorder_playlist(core: C<'_>, order: Vec<String>, from: usize, to: usize) -> Res {
    account::profile_reorder_playlist(&core, order, from, to).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn profile_toggle_playlist(core: C<'_>, id: String) -> Res {
    account::profile_toggle_playlist(&core, id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn profile_set_image(core: C<'_>, banner: bool, image: Option<String>) -> Res {
    account::profile_set_image(&core, banner, image).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn profile_remove_image(core: C<'_>, banner: bool) -> Res {
    account::profile_remove_image(&core, banner).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn profile_pick_image(core: C<'_>, banner: bool) -> Res {
    account::profile_pick_image(&core, banner).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn privacy_report(core: C) -> Value {
    account::privacy_report(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn privacy_clear(core: C) {
    account::privacy_clear(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn privacy_record_web_login(core: C) {
    account::privacy_record_web_login(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn server_info(core: C) -> Value {
    account::server_info(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn server_test(core: C<'_>, url: String) -> Res<Value> {
    account::server_test(&core, url).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn server_store_token(url: String, token: String) -> Res {
    account::server_store_token(url, token).await
}

// ---- jam / devices ----

#[tauri::command(rename_all = "snake_case")]
pub async fn devices_rescan(core: C<'_>, now: Option<bool>) -> Res<Value> {
    jam::devices_rescan(&core, now).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn device_select(core: C<'_>, id: String) -> Res<Value> {
    jam::device_select(&core, id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn select_device(core: C<'_>, id: String) -> Res<Value> {
    jam::select_device(&core, id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn device_connect(core: C<'_>, id: String) -> Res<Value> {
    jam::device_connect(&core, id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn device_use_local(core: C<'_>) -> Res<Value> {
    jam::device_use_local(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn jam_state(core: C<'_>, queue_all: Option<bool>) -> Value {
    jam::jam_state(&core, queue_all)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn jam_start(core: C<'_>) -> Res<Value> {
    jam::jam_start(&core).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn jam_join(core: C<'_>, code: String) -> Res<Value> {
    jam::jam_join(&core, code).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn jam_leave(core: C<'_>, code: Option<String>) {
    jam::jam_leave(&core, code)
}

#[tauri::command(rename_all = "snake_case")]
pub fn jam_set_perms(core: C<'_>, perms: Perms) -> Value {
    jam::jam_set_perms(&core, perms)
}

#[tauri::command(rename_all = "snake_case")]
pub fn jam_set_approve(core: C<'_>, on: bool) -> Value {
    jam::jam_set_approve(&core, on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn jam_admit(core: C<'_>, handle: String, allow: bool) -> Value {
    jam::jam_admit(&core, handle, allow)
}

#[tauri::command(rename_all = "snake_case")]
pub fn jam_kick(core: C<'_>, handle: String) -> Value {
    jam::jam_kick(&core, handle)
}

#[tauri::command(rename_all = "snake_case")]
pub fn jam_vote(core: C<'_>, id: String) -> Value {
    jam::jam_vote(&core, id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn jam_remove(core: C<'_>, id: String) -> Value {
    jam::jam_remove(&core, id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn jam_react(core: C<'_>, reaction: Reaction) -> Value {
    jam::jam_react(&core, reaction)
}

#[tauri::command(rename_all = "snake_case")]
pub fn jam_set_listen_here(core: C<'_>, here: bool) -> Value {
    jam::jam_set_listen_here(&core, here)
}

// ---- transfer / import / migrate ----

#[tauri::command(rename_all = "snake_case")]
pub fn import_parse_url(url: String) -> Option<String> {
    transfer::import_parse_url(url)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn import_link(
    core: C<'_>,
    url: String,
    name: Option<String>,
    playlist_id: Option<String>,
) -> Res<Value> {
    transfer::import_link(&core, url, name, playlist_id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn import_playlist_file(core: C<'_>, playlist_id: Option<String>) -> Res {
    transfer::import_playlist_file(&core, playlist_id).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn import_text(core: C<'_>, text: String, ask: Option<bool>) -> Res<Value> {
    transfer::import_text(&core, text, ask).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn share_save(core: C<'_>, code: String, link: Option<String>, author: Option<String>) -> Option<String> {
    transfer::share_save(&core, code, link, author)
}

#[tauri::command(rename_all = "snake_case")]
pub fn migrate_resume_state() -> Option<Value> {
    transfer::migrate_resume_state()
}

#[tauri::command(rename_all = "snake_case")]
pub fn migrate_discard(core: C<'_>) {
    transfer::migrate_discard(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn migrate_pasted(text: String) -> (usize, usize) {
    transfer::migrate_pasted(text)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn migrate_overview(
    core: C<'_>,
    service: String,
    profile: Option<String>,
    pasted: Option<String>,
    files: Option<Vec<PickedFile>>,
    resume: Option<bool>,
) -> Res<Value> {
    transfer::migrate_overview(&core, service, profile, pasted, files, resume).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn migrate_start(
    core: C<'_>,
    service: String,
    selected: Vec<String>,
    targets: Option<std::collections::HashMap<String, transfer::MigrateTarget>>,
) -> Res {
    transfer::migrate_start(&core, service, selected, targets).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn migrate_progress(core: C<'_>) -> Value {
    transfer::migrate_progress(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn migrate_cancel(core: C<'_>) {
    transfer::migrate_cancel(&core)
}

// ---- plugins ----

#[tauri::command(rename_all = "snake_case")]
pub fn plugins_state(core: C<'_>) -> Value {
    plugins::plugins_state(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn plugins_catalog(core: C<'_>, force: Option<bool>) -> Res<Value> {
    plugins::plugins_catalog(&core, force).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn plugin_install(core: C<'_>, id: String, repo: String, path: Option<String>, rev: Option<String>) -> Res {
    plugins::plugin_install(&core, id, repo, path, rev).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn plugin_uninstall(core: C<'_>, id: String) -> Res {
    plugins::plugin_uninstall(&core, id).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn plugin_enable(core: C<'_>, id: String, on: bool) {
    plugins::plugin_enable(&core, id, on)
}

#[tauri::command(rename_all = "snake_case")]
pub fn plugins_reload(core: C<'_>) {
    plugins::plugins_reload(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub fn plugins_folder() -> Res {
    plugins::plugins_folder()
}

#[tauri::command(rename_all = "snake_case")]
pub fn backgrounds_list(core: C<'_>) -> Value {
    glyph_app::backgrounds::backgrounds_list(&core)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn background_add(data: String) -> Res<String> {
    glyph_app::backgrounds::background_add(data).await
}

#[tauri::command(rename_all = "snake_case")]
pub fn background_remove(id: String) -> Res {
    glyph_app::backgrounds::background_remove(id)
}

#[tauri::command(rename_all = "snake_case")]
pub fn plugin_set_value(core: C<'_>, plugin: String, item: String, value: Value) {
    plugins::plugin_set_value(&core, plugin, item, value)
}

#[tauri::command(rename_all = "snake_case")]
pub fn plugin_action(core: C<'_>, plugin: String, item: String) {
    plugins::plugin_action(&core, plugin, item)
}

#[tauri::command(rename_all = "snake_case")]
pub fn plugin_lyrics(core: C<'_>, key: String) -> Vec<Value> {
    plugins::plugin_lyrics(&core, key)
}
