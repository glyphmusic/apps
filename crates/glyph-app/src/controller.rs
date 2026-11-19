//! The app's state and logic without GPUI: what `src/app` does for the GPUI
//! window, here for the Svelte UI. Ported step by step; `docs/TAURI.md`
//! lists what is done.
//!
//! One `Core` lives for the whole app. UI commands (`commands.rs`) call its
//! methods; it tells the UI about changes with events named `glyph://<name>`
//! (`player`, `queue`, `library`, `config`, `backend`, `toast`, ...).

use crate::dto;
use crate::host::Host;
use glyph_audio::{PlayerController, PlayerEvent, PlayerState};
use glyph_core::{Backend, Config, Library, ListKey, Queue};
use glyph_protocol::Track;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock, Weak};
use std::time::{Duration, Instant};

/// Played this long, "previous" restarts the song instead of going back.
const RESTART_AFTER: Duration = Duration::from_secs(3);

/// Unplayable songs skipped in a row before Glyph stops and says so.
const MAX_SKIPS: u32 = 5;
/// Periodic interval during active playback between position sidecar writes.
const PLAYBACK_POS_INTERVAL_MS: u64 = 5_000;

/// Where a song was started from (the open page); the UI says it with `play`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct PlayFrom {
    /// The open list: its listening counts for the sidebar.
    pub list: Option<ListKey>,
    /// The open artist page: the artist joins "recently played".
    pub artist: Option<String>,
    /// Picked from the search results: remembered in the search dropdown.
    pub search: bool,
}

/// The last search of the UI: what `search_more` adds to (src/app/search.rs).
#[derive(Default)]
pub struct SearchCache {
    pub query: String,
    pub tracks: Vec<Track>,
    pub scores: Vec<f32>,
    pub seen: HashSet<String>,
    pub page: usize,
    pub exhausted: bool,
    /// Counts searches: an older one stops when a newer one started.
    pub seq: u64,
    pub lyric_hits: Vec<(Track, String)>,
    pub errors: Vec<Value>,
    pub loading: bool,
    pub loading_more: bool,
    /// When the first results came in: later better hits no longer push
    /// the first rows around.
    pub shown_at: Option<Instant>,
}

/// Home feeds as last loaded (recommended, popular, new from followed artists).
#[derive(Default)]
pub struct Feeds {
    pub recs: Vec<Track>,
    pub trending: Vec<Track>,
    pub new_releases: Vec<glyph_core::providers::deezer::Release>,
    pub loading: bool,
    /// Counts reloads: a load started before the last reload drops its result.
    pub epoch: u64,
}

pub struct State {
    pub config: Config,
    pub library: Library,
    pub queue: Queue,
    pub current: Option<Track>,
    /// "Playing from ..." label of the queue.
    pub context: Option<String>,
    /// The current song's source is being looked up.
    pub resolving: bool,
    /// Volume before muting.
    pub muted_volume: Option<f32>,
    pub loop_ab: Option<(f64, f64)>,
    pub error: Option<String>,
    pub first_run: bool,
    /// Gapless: the next song, already handed to the player (key, resolved).
    pub preloaded: Option<(String, Track)>,
    /// Last try to preload (current key, next key, when), so a slow source
    /// is not asked again every tick.
    pub preload_attempt: Option<(String, String, Instant)>,
    /// A song was clicked while the backend still started: it loads when ready.
    pub pending_start: bool,
    /// Unplayable songs skipped in a row.
    pub skip_streak: u32,
    /// The track whose expired stream URL was already refreshed once.
    pub retried_expired: Option<String>,
    /// The track whose broken local file was already dropped.
    pub repaired_file: Option<String>,
    pub redownloaded_broken: HashSet<String>,
    /// The list the queue was started from (its listening counts).
    pub playing_from: Option<ListKey>,
    /// The artist page the queue was started from.
    pub recent_artist_ctx: Option<String>,
    /// The artist entry for "recently played" of the song being started.
    pub pending_recent_artist: Option<(String, Option<Track>)>,
    /// The listen of the current song was counted for `playing_from`.
    pub listen_counted: bool,
    /// Incremented whenever the active track changes (guards position sidecar merges).
    pub playback_seq: u64,
    /// Position restored from disk while playback is still paused and unresolved.
    pub restored_pos: Option<Duration>,
    /// Duration restored from disk as a fallback before the audio engine opens.
    pub restored_duration: Option<Duration>,
    /// Target seek offset applied once the restored track finishes resolving.
    pub resume_at: Option<Duration>,
    /// User asked to pause while the restored/new song was resolving.
    pub hold_play: bool,
    /// `player_state()` still holds a previous track (fade-out or failure) rather than `current`.
    pub player_stale: bool,
    /// Radio tracks are being fetched asynchronously after the queue ended.
    pub radio_loading: bool,
    pub search: SearchCache,
    pub feeds: Feeds,
}

pub struct Core {
    this: Weak<Self>,
    pub host: Arc<dyn Host>,
    pub backend: RwLock<Option<Arc<Backend>>>,
    pub backend_state: Mutex<Value>,
    pub player: Option<PlayerController>,
    pub st: Mutex<State>,
    acct: OnceLock<Arc<crate::account::Acct>>,
    discover: OnceLock<crate::discover::Discover>,
    transfer: OnceLock<Arc<crate::transfer::Transfer>>,
    plugins: OnceLock<Arc<crate::plugins::PluginState>>,
    library_dirty: AtomicBool,
    /// A batched library event is due at this time (`library_changed_soon`).
    library_emit_at: Mutex<Option<Instant>>,
    /// Hash of each part of the library the UI was last sent (`None`: it
    /// has nothing yet, the next event is whole). Held while sending, so
    /// events never overtake each other.
    library_sent: Mutex<Option<HashMap<String, u64>>>,
    /// The UI is in the background (Android): library events wait.
    ui_hidden: AtomicBool,
    /// A library event waited while the UI was in the background.
    library_unsent: AtomicBool,
    config_dirty: AtomicBool,
    playback_dirty: AtomicBool,
    playback_pos_dirty: AtomicBool,
    playback_pos_saved: Mutex<Option<(u64, u64)>>,
}

impl Core {
    pub fn new(host: Arc<dyn Host>) -> Arc<Self> {
        crate::host::set(host.clone());
        let first_run = !Config::path().exists();
        let config = Config::load().unwrap_or_else(|e| {
            tracing::warn!(error = %e, "config: load failed, using defaults");
            Config::default()
        });
        let library = Library::load().unwrap_or_else(|e| {
            tracing::warn!(error = %e, "library: load failed, starting empty");
            Library::default()
        });
        let session = library
            .storage_path
            .0
            .as_deref()
            .and_then(glyph_core::PlaybackSession::load_for);
        let player = match PlayerController::new() {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::error!(error = %e, "player: could not start");
                None
            }
        };
        let mut queue = Queue::new();
        queue.set_shuffle_mode(shuffle_mode(&config));
        queue.set_shuffle(config.player.shuffle);
        queue.set_repeat(dto::repeat_from(&config.player.repeat));
        if let Some(p) = &player {
            p.set_volume(config.player.volume);
        }
        let core = Arc::new_cyclic(|this| Self {
            this: this.clone(),
            host,
            backend: RwLock::new(None),
            backend_state: Mutex::new(json!({ "kind": "connecting" })),
            player,
            st: Mutex::new(State {
                config,
                library,
                queue,
                current: None,
                context: None,
                resolving: false,
                muted_volume: None,
                loop_ab: None,
                error: None,
                first_run,
                preloaded: None,
                preload_attempt: None,
                pending_start: false,
                skip_streak: 0,
                retried_expired: None,
                repaired_file: None,
                redownloaded_broken: HashSet::new(),
                playing_from: None,
                recent_artist_ctx: None,
                pending_recent_artist: None,
                listen_counted: false,
                playback_seq: 0,
                restored_pos: None,
                restored_duration: None,
                resume_at: None,
                hold_play: false,
                player_stale: false,
                radio_loading: false,
                search: SearchCache::default(),
                feeds: Feeds::default(),
            }),
            acct: OnceLock::new(),
            discover: OnceLock::new(),
            transfer: OnceLock::new(),
            plugins: OnceLock::new(),
            library_dirty: AtomicBool::new(false),
            library_emit_at: Mutex::new(None),
            library_sent: Mutex::new(None),
            ui_hidden: AtomicBool::new(false),
            library_unsent: AtomicBool::new(false),
            config_dirty: AtomicBool::new(false),
            playback_dirty: AtomicBool::new(false),
            playback_pos_dirty: AtomicBool::new(false),
            playback_pos_saved: Mutex::new(None),
        });
        core.restore_playback_session(session);
        core
    }

    pub fn st(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn backend(&self) -> Option<Arc<Backend>> {
        self.backend.read().ok().and_then(|b| b.clone())
    }

    pub fn backend_or_err(&self) -> Result<Arc<Backend>, String> {
        self.backend().ok_or_else(|| "backend_offline".to_string())
    }

    /// Starts glyph-core in the background; the UI shows "connecting" until then.
    pub fn start_backend(self: &Arc<Self>) {
        self.ensure_reconnect_monitor();
        let core = self.clone();
        let config = self.st().config.clone();
        crate::rt::spawn(async move {
            match Backend::new(config).await {
                Ok(b) => {
                    let mode = match b.mode() {
                        glyph_core::BackendMode::Direct => "direct".to_string(),
                        m => format!("{m:?}").split([' ', '{']).next().unwrap_or("server").to_lowercase(),
                    };
                    // How the player reaches the server, the account's
                    // media and yt-dlp streams (src/app/boot.rs backend_ready).
                    glyph_audio::http::set_server_access(b.server_media_credentials());
                    glyph_audio::http::set_account_access_provider(Arc::new(glyph_core::account::media_access));
                    glyph_audio::http::set_local_access_provider(Arc::new(glyph_core::ytdlp::local_stream_access));
                    glyph_audio::http::set_cache_dir(glyph_core::paths::cache_dir().join("audio-ranges"));
                    glyph_core::set_stream_warmer(Arc::new(glyph_audio::http::prewarm));
                    glyph_core::cover::set_data_saver(core.st().config.storage.data_saver);
                    *core.backend.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(b));
                    core.set_backend_state(json!({ "kind": "ready", "mode": mode }));
                    core.on_backend_ready();
                    crate::account::wake_reconnect(&core);
                }
                Err(e) => {
                    tracing::error!(error = %e, "backend: could not start");
                    core.set_backend_state(json!({ "kind": "offline", "reason": e.to_string() }));
                    core.on_backend_failed();
                }
            }
        });
    }

    /// Automatically reconnects when backend startup failed or a configured server
    /// was temporarily offline (e.g. during a server update), so the user does
    /// not have to open Server settings and click Apply manually.
    fn ensure_reconnect_monitor(self: &Arc<Self>) {
        static STARTED: AtomicBool = AtomicBool::new(false);
        if STARTED.swap(true, Ordering::AcqRel) {
            return;
        }
        let core = self.clone();
        crate::rt::spawn(async move {
            let mut backoff_secs = 5u64;
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let connecting = core.backend_state.lock().unwrap_or_else(|e| e.into_inner())["kind"] == "connecting";
                if connecting {
                    continue;
                }
                let Some(b) = core.backend() else {
                    if core.st().config.effective_server_url().trim().is_empty() {
                        continue;
                    }
                    tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                    let still_missing = core.backend().is_none()
                        && core.backend_state.lock().unwrap_or_else(|e| e.into_inner())["kind"] != "connecting";
                    if still_missing {
                        backoff_secs = (backoff_secs * 2).min(30);
                        core.set_backend_state(json!({ "kind": "connecting" }));
                        core.start_backend();
                    }
                    continue;
                };
                let server_offline = matches!(
                    b.mode(),
                    glyph_core::BackendMode::Server { ref server_version, .. } if server_version == "offline"
                );
                let outage_offline = glyph_core::outage::current()
                    .is_some_and(|o| o.kind == glyph_core::outage::Kind::Offline);
                if !server_offline && !outage_offline {
                    backoff_secs = 5;
                    continue;
                }
                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                match b.refresh_server().await {
                    Ok(true) => {
                        backoff_secs = 5;
                        glyph_audio::http::set_server_access(b.server_media_credentials());
                        core.set_backend_state(json!({ "kind": "ready", "mode": "server" }));
                        core.on_backend_ready();
                        crate::account::wake_reconnect(&core);
                    }
                    Ok(false) => {
                        backoff_secs = (backoff_secs * 2).min(30);
                    }
                    Err(e) => {
                        backoff_secs = 30;
                        core.set_backend_state(json!({ "kind": "offline", "reason": e.to_string() }));
                    }
                }
            }
        });
    }

    pub fn set_backend_state(&self, v: Value) {
        *self.backend_state.lock().unwrap_or_else(|e| e.into_inner()) = v.clone();
        self.emit("backend", v);
    }

    pub fn arc(&self) -> Option<Arc<Self>> {
        self.this.upgrade()
    }

    pub fn acct(&self) -> Option<Arc<crate::account::Acct>> {
        self.acct.get().cloned()
    }

    pub fn set_acct(&self, v: Arc<crate::account::Acct>) {
        let _ = self.acct.set(v);
    }

    pub fn discover(&self) -> Option<&crate::discover::Discover> {
        self.discover.get()
    }

    pub fn set_discover(&self, v: crate::discover::Discover) {
        let _ = self.discover.set(v);
    }

    pub fn transfer(&self) -> Option<Arc<crate::transfer::Transfer>> {
        self.transfer.get().cloned()
    }

    pub fn set_transfer(&self, v: Arc<crate::transfer::Transfer>) {
        let _ = self.transfer.set(v);
    }

    pub fn plugins(&self) -> Option<Arc<crate::plugins::PluginState>> {
        self.plugins.get().cloned()
    }

    pub fn set_plugins(&self, v: Arc<crate::plugins::PluginState>) {
        let _ = self.plugins.set(v);
    }

    // ---- events to the UI ----

    pub fn emit<T: Serialize + Clone>(&self, ev: &str, payload: T) {
        match serde_json::to_value(payload) {
            Ok(v) => self.host.emit(&format!("glyph://{ev}"), v),
            Err(e) => tracing::warn!(event = ev, error = %e, "emit failed"),
        }
    }

    /// A short message; `key` is an i18n key the UI translates.
    pub fn toast_key(&self, key: &str, kind: &str) {
        self.emit("toast", json!({ "key": key, "text": key, "kind": kind }));
    }

    /// What plays: the network device's or the jam host's state when one
    /// is on, else this player's.
    pub fn player_state(&self) -> Option<PlayerState> {
        crate::jam::player_state(self)
    }

    pub fn player_dto(&self) -> dto::Player {
        // Another Glyph app of the account plays: show that one.
        if let Some(r) = crate::account::remote_player(self) {
            let st = self.st();
            return dto::Player {
                status: if r.playing { "playing" } else { "paused" },
                current: r.track.or_else(|| st.current.clone()),
                position: r.position,
                duration: r.duration,
                volume: r.volume,
                muted: false,
                shuffle: st.queue.shuffle(),
                repeat: dto::repeat_name(st.queue.repeat()),
                source: None,
                preview: false,
                error: r.error,
                loop_a: None,
                loop_b: None,
            };
        }
        let ps = self.player_state();
        let st = self.st();
        let volume = st.muted_volume.unwrap_or_else(|| ps.as_ref().map_or(st.config.player.volume, |s| s.volume));
        let active_ps = (!st.player_stale).then_some(ps.as_ref()).flatten();
        dto::player(dto::PlayerInput {
            state: active_ps,
            current: st.current.as_ref(),
            resolving: st.resolving,
            radio_loading: st.radio_loading,
            muted: st.muted_volume.is_some(),
            volume,
            queue: &st.queue,
            error: st.error.as_ref(),
            loop_ab: st.loop_ab,
            restored_pos: st.restored_pos,
            restored_duration: st.restored_duration,
            resume_at: st.resume_at,
        })
    }

    pub fn queue_dto(&self) -> Value {
        let st = self.st();
        dto::queue(&st.queue, st.context.as_ref())
    }

    pub fn library_dto(&self) -> Value {
        let downloads = self.backend().map(|b| b.downloaded_tracks()).unwrap_or_default();
        dto::library(&self.st().library, &downloads)
    }

    pub fn emit_player(&self) {
        self.emit("player", self.player_dto());
    }
    pub fn emit_queue(&self) {
        self.emit("queue", self.queue_dto());
    }
    /// Sends the library to the UI: only the parts that changed since the
    /// last event (`{"__patch": {..}}`), the whole library when the UI has none
    /// yet. Opening a list or an artist only moves the sidebar order; before,
    /// every such step sent the whole library (all liked songs and
    /// playlists), which made phones wait on every page.
    pub fn emit_library(&self) {
        if self.ui_hidden.load(Ordering::Acquire) {
            // The page is frozen in the background: one whole event on return.
            self.library_unsent.store(true, Ordering::Release);
            *self.library_sent.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return;
        }
        let mut sent = self.library_sent.lock().unwrap_or_else(|e| e.into_inner());
        let dto = self.library_dto();
        let Value::Object(parts) = dto else {
            self.emit("library", dto);
            return;
        };
        let hashes: HashMap<String, u64> = parts.iter().map(|(k, v)| (k.clone(), value_hash(v))).collect();
        let payload = match sent.as_ref() {
            Some(old) => {
                let mut patch = serde_json::Map::new();
                for (k, v) in &parts {
                    if old.get(k) != hashes.get(k) {
                        patch.insert(k.clone(), v.clone());
                    }
                }
                if patch.is_empty() {
                    return;
                }
                json!({ "__patch": Value::Object(patch) })
            }
            None => Value::Object(parts),
        };
        *sent = Some(hashes);
        self.emit("library", payload);
    }

    /// The UI went to the background (`true`) or came back. While away,
    /// library events wait and one whole library is sent on return.
    pub fn set_ui_hidden(&self, hidden: bool) {
        self.ui_hidden.store(hidden, Ordering::Release);
        if !hidden && self.library_unsent.swap(false, Ordering::AcqRel) {
            self.emit_library();
        }
    }

    /// The UI lost library events (Android, too many waited while away):
    /// send it the whole library.
    pub fn resend_library(&self) {
        self.library_resent();
        self.emit_library();
    }

    /// The UI reloads its whole state (`snapshot`): the next library event
    /// is whole again (a second window or a reloaded page has no parts yet).
    fn library_resent(&self) {
        *self.library_sent.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Library changed: tell the UI now, write the file soon (`pump`).
    pub fn library_changed(&self) {
        crate::account::library_changed(self);
        self.library_dirty.store(true, Ordering::Relaxed);
        self.emit_library();
    }

    /// Like `library_changed`, for small background fixes (album names and
    /// covers found while covers load): many of them in a row send the UI
    /// one library event a second later instead of one per cover.
    pub fn library_changed_soon(&self) {
        crate::account::library_changed(self);
        self.library_dirty.store(true, Ordering::Relaxed);
        let mut at = self.library_emit_at.lock().unwrap_or_else(|e| e.into_inner());
        if at.is_none() {
            *at = Some(Instant::now() + Duration::from_millis(1000));
        }
    }

    /// Sends a batched library event once it is due (called by the pump).
    fn flush_library_soon(&self) {
        let due = {
            let mut at = self.library_emit_at.lock().unwrap_or_else(|e| e.into_inner());
            match *at {
                Some(t) if Instant::now() >= t => { *at = None; true }
                _ => false,
            }
        };
        if due {
            self.emit_library();
        }
    }

    pub fn config_changed(&self) {
        self.config_dirty.store(true, Ordering::Relaxed);
        let cfg = serde_json::to_value(&self.st().config).unwrap_or(Value::Null);
        self.emit("config", cfg);
    }

    pub fn snapshot(&self) -> Value {
        self.library_resent();
        let (config, first_run) = {
            let st = self.st();
            (serde_json::to_value(&st.config).unwrap_or(Value::Null), st.first_run)
        };
        json!({
            "config": config,
            "library": self.library_dto(),
            "queue": self.queue_dto(),
            "player": self.player_dto(),
            "backend": self.backend_state.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            "account": crate::account::account_json(self),
            "version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
            "first_run": first_run,
            "output_devices": PlayerController::output_devices(),
            "update": Value::Null,
        })
    }

    /// What can be behind after the UI was in the background: the player
    /// and the queue (light, unlike `snapshot`).
    pub fn resume_state(&self) -> Value {
        json!({ "player": self.player_dto(), "queue": self.queue_dto() })
    }

    // ---- rules shared by everything that starts or queues songs ----

    /// Not filtered out by "Hide explicit songs" (explicit, or an unrated
    /// YouTube song). Everything that puts songs into the queue must ask.
    pub fn allowed(&self, t: &Track) -> bool {
        !(t.maybe_explicit() && self.st().config.audio.hide_explicit)
    }

    /// Allowed, and while offline only what is saved on the device (unless
    /// another device or an MPD / network output plays it).
    pub fn playable(&self, t: &Track) -> bool {
        if !self.allowed(t) {
            return false;
        }
        !crate::storage::is_offline()
            || crate::account::controlling(self)
            || crate::jam::remote_on()
            || crate::storage::available_offline(self, t)
    }

    /// Offline and `t` is not saved: say so (it does not play).
    fn blocked_offline(&self, t: &Track) -> bool {
        if crate::storage::is_offline() && !self.playable(t) && self.allowed(t) {
            self.toast_key("offline.unavailable", "info");
            return true;
        }
        false
    }

    /// "Hide explicit songs" is on and `t` is explicit: say so (it does not play).
    fn blocked_explicit(&self, t: &Track) -> bool {
        if self.allowed(t) {
            return false;
        }
        self.st().pending_recent_artist = None;
        self.toast_key("toast.explicit_blocked", "info");
        true
    }

    /// Songs played lately (smart shuffle deals them last).
    fn recent_keys(st: &State) -> Vec<String> {
        st.library.recent_tracks().take(30).map(glyph_core::dedupe::track_key).collect()
    }

    /// A toast with values in it.
    pub fn toast_vars(&self, key: &str, vars: Value, kind: &str) {
        self.emit("toast", json!({ "key": key, "text": key, "vars": vars, "kind": kind }));
    }

    /// A toast "Title · sub" (sub is shown as it is).
    pub fn toast_sub(&self, key: &str, sub: &str, kind: &str) {
        self.emit("toast", json!({ "key": key, "text": key, "sub": sub, "kind": kind }));
    }

    // ---- playback (src/app/playback.rs) ----

    /// Replace the queue with `tracks` and start at `index`.
    pub fn play_list(self: &Arc<Self>, tracks: Vec<Track>, index: usize, context: Option<String>) {
        self.play_list_from(tracks, index, context, PlayFrom::default());
    }

    /// [`Core::play_list`] started from a page: `from` says which one (the
    /// open list, artist page or search results).
    pub fn play_list_from(self: &Arc<Self>, tracks: Vec<Track>, index: usize, context: Option<String>, from: PlayFrom) {
        let Some(track) = tracks.get(index).cloned() else { return };
        // Another Glyph app of the account plays: send it the songs.
        if crate::account::controlling(self) {
            crate::account::control(
                self,
                glyph_protocol::playback::Command::Transfer {
                    tracks: tracks[index..].iter().take(100).cloned().collect(),
                    index: 0,
                    position_ms: 0,
                    playing: true,
                },
            );
            return;
        }
        if self.blocked_offline(&track) {
            return;
        }
        self.st().skip_streak = 0;
        // In someone else's jam: ask the host instead.
        if crate::jam::route_request(self, &track, true) {
            return;
        }
        if self.blocked_explicit(&track) {
            return;
        }
        let keep: Vec<bool> = tracks.iter().map(|t| self.playable(t)).collect();
        let index = keep[..index].iter().filter(|k| **k).count();
        let tracks: Vec<Track> = tracks.into_iter().zip(keep).filter(|(_, k)| *k).map(|(t, _)| t).collect();
        let mut library_touched = false;
        {
            let mut st = self.st();
            let recent = Self::recent_keys(&st);
            st.queue.set_recent(recent);
            st.queue.set(tracks, index);
            st.context = context;
            // Picked from the search results: remembered for the search dropdown.
            if from.search {
                st.library.push_search(track.clone());
                library_touched = true;
            }
            // The open list's use for the sidebar is counted by
            // `discover::set_playing_from` (the UI calls it from `play`).
            let now = glyph_core::paths::now_unix();
            // Played from an artist's page: it counts towards the artist's
            // sidebar place (and gives the entry a picture).
            if let Some(a) = from.artist.as_ref() {
                st.library.push_recent_artist_page(a, Some(track.clone()));
                st.library.record_use(&glyph_core::Library::artist_id(a), 1.0, now);
                library_touched = true;
            }
            st.playing_from = from.list;
            st.recent_artist_ctx = from.artist.clone();
            st.pending_recent_artist = from.artist.map(|a| (a, Some(track.clone())));
        }
        if library_touched {
            self.library_changed_soon();
        }
        self.playback_changed();
        self.start_track(track);
    }

    /// One song started on its own (previous, a queue row): the same
    /// checks as `play_list`.
    pub fn play_track(self: &Arc<Self>, track: Track) {
        if crate::account::controlling(self) {
            crate::account::control(
                self,
                glyph_protocol::playback::Command::Transfer { tracks: vec![track], index: 0, position_ms: 0, playing: true },
            );
            return;
        }
        if crate::jam::route_request(self, &track, true) {
            return;
        }
        if self.blocked_offline(&track) || self.blocked_explicit(&track) {
            return;
        }
        self.st().skip_streak = 0;
        self.start_track(track);
    }

    /// Plays `tracks` from a random song with shuffle on: the whole list is
    /// dealt, the first song included.
    pub fn play_shuffled(self: &Arc<Self>, tracks: Vec<Track>, context: Option<String>, from: PlayFrom) {
        let tracks: Vec<Track> = tracks.into_iter().filter(|t| self.playable(t)).collect();
        if tracks.is_empty() {
            return;
        }
        let first = {
            let mut st = self.st();
            let recent = Self::recent_keys(&st);
            let mode = shuffle_mode(&st.config);
            st.queue.set_recent(recent);
            st.queue.set_shuffle_mode(mode);
            let first = st.queue.set_shuffled(tracks).cloned();
            st.config.player.shuffle = true;
            st.context = context;
            st.playing_from = from.list;
            st.recent_artist_ctx = from.artist.clone();
            st.pending_recent_artist = from.artist.zip(first.clone()).map(|(a, t)| (a, Some(t)));
            first
        };
        self.config_dirty.store(true, Ordering::Relaxed);
        let Some(first) = first else { return };
        self.play_track(first);
    }

    /// Plays `track`: shows it at once, looks up its source, then loads it.
    pub fn start_track(self: &Arc<Self>, track: Track) {
        self.start_track_with(track, None);
    }

    /// `adopted`: the player already runs this song (gapless preload), so it
    /// is only shown, not looked up and loaded again.
    fn start_track_with(self: &Arc<Self>, track: Track, adopted: Option<Track>) {
        // Playing while another account device is controlled: it plays there.
        if adopted.is_none() && crate::account::route_play(self, &track) {
            return;
        }
        crate::jam::on_track_changed(self);
        crate::discover::on_track_started(self, &track);
        {
            let mut st = self.st();
            if adopted.is_none() {
                st.preloaded = None;
            }
            st.preload_attempt = None;
            st.restored_pos = None;
            st.restored_duration = None;
            st.resume_at = None;
            st.hold_play = false;
            st.playback_seq += 1;
            st.current = Some(track.clone());
            st.resolving = adopted.is_none();
            st.radio_loading = false;
            st.player_stale = adopted.is_none();
            st.error = None;
            st.loop_ab = None;
            st.listen_counted = false;
            st.retried_expired = None;
            st.repaired_file = None;
            st.pending_start = false;
            let artist_ctx = st.pending_recent_artist.take();
            // Played from an artist's page: the artist entry in "recently
            // played" stands for their songs (no row per song on top of it).
            let from_artist = st.recent_artist_ctx.as_ref().is_some_and(|a| glyph_core::dedupe::by_artist(&track.artist, a))
                && !st.queue.current_is_manual();
            if !from_artist {
                let from_list = (!st.queue.current_is_manual())
                    .then(|| st.playing_from.clone())
                    .flatten()
                    .filter(|l| !matches!(l, glyph_core::ListKey::Recent));
                st.library.push_recent_from(track.clone(), from_list);
            }
            st.library.record_play(&track);
            if let Some((name, art)) = artist_ctx {
                st.library.push_recent_artist(&name, art);
            }
        }
        // Only "recently played" and play counts changed: the whole library
        // goes to the UI a moment later (one event), not before the song starts.
        if adopted.is_some() {
            *self.playback_pos_saved.lock().unwrap_or_else(|e| e.into_inner()) = None;
            self.playback_changed();
            self.library_changed_soon();
            self.emit_queue();
            self.emit_player();
            if let Some(b) = self.backend() {
                self.prefetch_upcoming(&b);
            }
            return;
        }
        if let Some(p) = &self.player {
            // Fade the old song out right away instead of letting it run
            // until the new one has loaded.
            p.pause();
            p.set_loop(None);
            p.drop_next();
        }
        *self.playback_pos_saved.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.playback_changed();
        self.emit_player();
        // Look up the source first; the rest of the UI updates meanwhile.
        self.load_current(track);
        self.emit_queue();
        self.library_changed_soon();
    }

    /// Finds the source of the current song, then plays it. Before the
    /// backend is up the song waits (`pending_start`) instead of failing:
    /// a click right after the start used to do nothing.
    fn load_current(self: &Arc<Self>, track: Track) {
        let Some(backend) = self.backend() else {
            let connecting = self.backend_state.lock().unwrap_or_else(|e| e.into_inner())["kind"] == "connecting";
            if connecting {
                self.st().pending_start = true;
                return;
            }
            {
                let mut st = self.st();
                st.resolving = false;
                st.resume_at = None;
                st.hold_play = false;
                st.player_stale = true;
                st.error = Some("offline".into());
            }
            if let Some(p) = &self.player {
                p.stop();
            }
            self.toast_key("status.backend_offline", "info");
            self.emit_player();
            return;
        };
        let core = self.clone();
        crate::rt::spawn(async move {
            let key = glyph_core::dedupe::track_key(&track);
            let started = Instant::now();
            let mut r = backend.resolve(&track).await;
            // A short network gap (Wi-Fi to mobile data, a phone waking up)
            // used to skip song after song within a second and then stop.
            // Wait a little and ask again for the same song first.
            for wait in [2u64, 4, 6] {
                let transient = matches!(&r, Err(e) if transient_net_error(&e.to_string()));
                if !transient {
                    break;
                }
                tracing::info!(wait_s = wait, "playback: network gap while resolving, retrying");
                tokio::time::sleep(Duration::from_secs(wait)).await;
                let still = core.st().current.as_ref().map(glyph_core::dedupe::track_key).as_deref() == Some(key.as_str());
                if !still {
                    return; // another song was started meanwhile
                }
                r = backend.resolve(&track).await;
            }
            tracing::info!(elapsed_ms = started.elapsed().as_millis() as u64, success = r.is_ok(), "playback: source resolved");
            {
                let mut st = core.st();
                if st.current.as_ref().map(glyph_core::dedupe::track_key).as_deref() != Some(key.as_str()) {
                    return; // another song was started meanwhile
                }
                st.resolving = false;
            }
            match r {
                Ok(Some(res)) => {
                    core.output_load(res);
                    core.prefetch_upcoming(&backend);
                }
                Ok(None) => {
                    {
                        let mut st = core.st();
                        st.resume_at = None;
                        st.hold_play = false;
                        st.player_stale = true;
                    }
                    crate::jam::no_source(&core, &key);
                    if !core.skip_unplayable() {
                        core.st().error = Some("no_source".into());
                        if let Some(p) = &core.player {
                            p.stop();
                        }
                        core.toast_key("toast.no_source", "info");
                    }
                }
                Err(e) => {
                    {
                        let mut st = core.st();
                        st.resume_at = None;
                        st.hold_play = false;
                        st.player_stale = true;
                    }
                    let e = e.to_string();
                    if !core.skip_unplayable() {
                        core.st().error = Some(e.clone());
                        if let Some(p) = &core.player {
                            p.stop();
                        }
                        let (key, vars) = resolve_error(&e);
                        core.toast_vars(key, vars, "info");
                    } else {
                        tracing::warn!(error = %e, "playback: resolve failed, skipped");
                    }
                }
            }
            core.emit_player();
        });
    }

    /// Hands a found source to MPD / a network device, or this player.
    fn output_load(&self, res: glyph_core::Resolved) {
        let (resume_at, hold) = {
            let mut st = self.st();
            st.player_stale = false;
            (st.resume_at.take(), std::mem::take(&mut st.hold_play))
        };
        if !crate::jam::route_load(self, &res.track, &res.source) {
            if let Some(p) = &self.player {
                p.load(res.track, res.source);
                if crate::jam::start_paused() || hold {
                    p.pause();
                } else {
                    p.play();
                }
                if let Some(at) = resume_at {
                    if at > Duration::ZERO {
                        p.seek(at);
                    }
                }
            }
        }
    }

    /// The next songs of the queue: their sources are looked up in one batch
    /// (local yt-dlp only), instead of one wait per song.
    fn prefetch_upcoming(&self, backend: &Arc<Backend>) {
        let upcoming: Vec<Track> = self.st().queue.upcoming().into_iter().take(3).cloned().collect();
        backend.prefetch(upcoming);
    }

    /// Called once the backend is up: a song clicked meanwhile plays now.
    pub fn on_backend_ready(self: &Arc<Self>) {
        // Feeds and everything that failed while it started load now.
        crate::commands::reload_feeds(self);
        let waiting = std::mem::take(&mut self.st().pending_start);
        if waiting {
            let cur = self.st().current.clone();
            if let Some(t) = cur {
                self.load_current(t);
            }
        }
    }

    /// The backend could not start: a waiting song gives up.
    pub fn on_backend_failed(&self) {
        let waiting = std::mem::take(&mut self.st().pending_start);
        if waiting {
            {
                let mut st = self.st();
                st.resolving = false;
                st.player_stale = true;
                st.error = Some("offline".into());
            }
            if let Some(p) = &self.player {
                p.stop();
            }
            self.toast_key("status.backend_offline", "info");
            self.emit_player();
        }
    }

    /// The current song cannot play (no source, or the player refused it):
    /// go on with the next one instead of stopping. Not where someone else
    /// decides what plays (jam guest, another device), not after
    /// `MAX_SKIPS` failures in a row and not when switched off in
    /// Settings > Playback. `true` if the next song was started.
    pub fn skip_unplayable(self: &Arc<Self>) -> bool {
        let (on, streak) = {
            let st = self.st();
            (st.config.playback.skip_unplayable, st.skip_streak)
        };
        if !on || crate::account::controlling(self) || crate::jam::skip_blocked(self) || streak >= MAX_SKIPS {
            return false;
        }
        let (failed, next) = {
            let mut st = self.st();
            let Some(failed) = st.current.clone() else { return false };
            let Some(next) = st.queue.skip_next().cloned() else { return false };
            // Repeat one, or a list of just this song: nothing else to go to.
            if glyph_core::dedupe::track_key(&next) == glyph_core::dedupe::track_key(&failed) {
                return false;
            }
            st.skip_streak += 1;
            (failed, next)
        };
        self.toast_sub("toast.skipped", &failed.title, "info");
        self.start_track(next);
        true
    }

    /// The local player holds no song right now (it refused to start one).
    fn player_idle(&self) -> bool {
        self.player.as_ref().map(|p| p.state().status) == Some(glyph_audio::Status::Idle)
    }

    /// The player reported an error (src/app/poll.rs).
    fn on_player_error(self: &Arc<Self>, e: String) {
        tracing::warn!(error = %e, "player error");
        crate::account::player_error(self, &e);
        // Preview URLs are signed and expire; an expired one is asked for once more.
        if self.retry_expired_preview(&e) {
            tracing::info!("expired preview URL, re-resolving once");
            return;
        }
        if self.repair_broken_file(&e) {
            tracing::info!("broken local file dropped, playing online: {e}");
            return;
        }
        let resolving = self.st().resolving;
        // It never started (a song that fails midway ends and moves on by
        // itself): go on with the next one.
        if !output_error(&e) && !resolving && self.player_idle() && self.skip_unplayable() {
            tracing::warn!("playback error, skipped: {e}");
            return;
        }
        let ended = self.player_state().is_some_and(|s| s.status == glyph_audio::Status::Ended);
        if output_error(&e) || self.player_idle() || ended {
            let mut st = self.st();
            st.error = Some(e.clone());
            if ended {
                st.player_stale = true;
            }
        }
        self.toast_vars("toast.player_error", json!({ "e": e }), "info");
        self.emit_player();
    }

    /// `true` if the error was an expired / denied stream URL and a fresh
    /// resolve was started for the current song. Once per song, so a
    /// provider that is down does not loop.
    fn retry_expired_preview(self: &Arc<Self>, error: &str) -> bool {
        // "HTTP 403" when opening, "(403 Forbidden)" from a later range request.
        if !["HTTP 403", "HTTP 401", "403 Forbidden", "401 Unauthorized"].iter().any(|s| error.contains(s)) {
            return false;
        }
        let Some(backend) = self.backend() else { return false };
        let (current, key) = {
            let mut st = self.st();
            let Some(cur) = st.current.clone() else { return false };
            let key = glyph_core::dedupe::track_key(&cur);
            if st.retried_expired.as_deref() == Some(key.as_str()) {
                return false;
            }
            st.retried_expired = Some(key.clone());
            (cur, key)
        };
        // Local YouTube URLs expire too (after hours): fetch fresh ones
        // instead of dropping to a preview.
        let youtube = self.player_state().and_then(|s| s.provider) == Some(glyph_protocol::ProviderId::YouTube);
        let core = self.clone();
        crate::rt::spawn(async move {
            let r = if youtube {
                backend.refresh_source(&current).await
            } else {
                match backend.refresh_preview(&current).await {
                    Ok(Some(r)) => Ok(Some(r)),
                    _ => backend.resolve(&current).await,
                }
            };
            core.reload_after_repair(&key, r);
        });
        true
    }

    /// `true` if the current song played from a local file that cannot be
    /// decoded (truncated or damaged download / cached stream): the file is
    /// dropped, the song plays online instead and a damaged download is
    /// fetched again (once; a second broken copy is only removed).
    fn repair_broken_file(self: &Arc<Self>, error: &str) -> bool {
        // Only file problems; a missing audio device must not cost a download.
        const FILE_ERRORS: [&str; 6] =
            ["cannot open stream", "unsupported audio format", "unsupported codec", "read error", "stream error", "decode error"];
        if !FILE_ERRORS.iter().any(|p| error.starts_with(p)) {
            return false;
        }
        let Some(backend) = self.backend() else { return false };
        let (current, key) = {
            let st = self.st();
            let Some(cur) = st.current.clone() else { return false };
            let key = glyph_core::dedupe::track_key(&cur);
            if st.repaired_file.as_deref() == Some(key.as_str()) {
                return false;
            }
            (cur, key)
        };
        let Some(kind) = backend.forget_broken_file(&current) else { return false };
        let again = {
            let mut st = self.st();
            st.repaired_file = Some(key.clone());
            kind == glyph_core::BrokenFile::Download && st.redownloaded_broken.insert(key.clone())
        };
        tracing::warn!(track = %key, ?kind, "playback: local file broken, dropped it");
        if kind == glyph_core::BrokenFile::Download {
            if again {
                self.toast_key("toast.dl_broken_retry", "info");
                crate::storage::download_tracks(self, vec![current.clone()], true);
            } else {
                self.toast_key("toast.dl_broken_removed", "info");
            }
            self.emit_library();
        }
        let core = self.clone();
        crate::rt::spawn(async move {
            let r = backend.resolve(&current).await;
            core.reload_after_repair(&key, r);
        });
        true
    }

    /// A fresh source for the current song after a repair: load it.
    fn reload_after_repair(self: &Arc<Self>, key: &str, r: anyhow::Result<Option<glyph_core::Resolved>>) {
        if self.st().current.as_ref().map(glyph_core::dedupe::track_key).as_deref() != Some(key) {
            return;
        }
        match r {
            Ok(Some(res)) => self.output_load(res),
            Ok(None) => self.toast_key("toast.no_source", "info"),
            Err(e) => self.toast_vars("toast.resolve_error", json!({ "e": e.to_string() }), "info"),
        }
        self.emit_player();
    }

    pub fn next(self: &Arc<Self>, auto: bool) {
        if !auto && crate::account::control(self, glyph_protocol::playback::Command::Next) {
            return;
        }
        // Another Glyph app plays: only it advances its queue.
        if auto && crate::account::controlling(self) {
            return;
        }
        if crate::jam::route_next(self, auto) {
            return;
        }
        let ps_now = self.player_state();
        let skipped = !auto && ps_now.as_ref().is_some_and(|s| s.position < Duration::from_secs(30));
        let playing_track = ps_now
            .filter(|s| !matches!(s.status, glyph_audio::Status::Ended | glyph_audio::Status::Idle))
            .and_then(|s| s.track);
        let offline = crate::storage::is_offline();
        let relaxed = crate::account::controlling(self) || crate::jam::remote_on();
        let mut offline_blocked: Option<Track> = None;
        let (next, preloaded) = {
            let mut st = self.st();
            if skipped {
                if let Some(cur) = st.current.clone() {
                    st.library.record_skip(&cur);
                }
            }
            let preloaded = st.preloaded.take();
            let preloaded_playing = auto
                && preloaded.as_ref().is_some_and(|(k, r)| {
                    st.queue.peek_next().is_some_and(|n| glyph_core::dedupe::track_key(n) == *k)
                        && playing_track.as_ref().is_some_and(|p| p.id == r.id && p.source == r.source)
                });
            let mut next = None;
            if offline && !relaxed && !preloaded_playing {
                let hide = st.config.audio.hide_explicit;
                let ok = |t: &Track| !(t.maybe_explicit() && hide) && crate::storage::available_offline(self, t);
                let repeat = st.queue.repeat();
                let has_offline = if auto && repeat == glyph_core::RepeatMode::One {
                    st.queue.current().is_some_and(ok)
                } else {
                    st.queue.upcoming().into_iter().any(ok)
                        || (repeat == glyph_core::RepeatMode::All
                            && (st.queue.history().into_iter().any(ok) || st.queue.current().is_some_and(ok)))
                };
                if has_offline {
                    let mut cand = if auto { st.queue.next().cloned() } else { st.queue.skip_next().cloned() };
                    let limit = st.queue.upcoming().len() + 1;
                    for _ in 0..limit {
                        if !cand.as_ref().is_some_and(|t| !ok(t)) {
                            break;
                        }
                        cand = st.queue.skip_next().cloned();
                    }
                    if cand.as_ref().is_some_and(&ok) {
                        next = cand;
                    }
                } else {
                    // Keep the rest of the queue intact instead of skipping over all online songs.
                    offline_blocked = if auto { st.queue.next().cloned() } else { st.queue.skip_next().cloned() };
                    if let Some(ref blocked) = offline_blocked {
                        st.preloaded = None;
                        st.preload_attempt = None;
                        st.resolving = false;
                        st.radio_loading = false;
                        st.player_stale = true;
                        st.error = Some("offline".into());
                        st.hold_play = false;
                        st.resume_at = None;
                        st.restored_pos = None;
                        st.restored_duration = None;
                        st.playback_seq += 1;
                        st.current = Some(blocked.clone());
                    }
                }
            } else {
                next = if auto { st.queue.next().cloned() } else { st.queue.skip_next().cloned() };
            }
            (next, preloaded)
        };
        if offline_blocked.is_some() {
            if skipped {
                crate::discover::on_skip(self);
                self.library_changed();
            }
            if let Some(p) = &self.player {
                p.stop();
            }
            *self.playback_pos_saved.lock().unwrap_or_else(|e| e.into_inner()) = None;
            self.playback_changed();
            self.toast_key("offline.unavailable", "info");
            self.emit_queue();
            self.emit_player();
            return;
        }
        if skipped {
            crate::discover::on_skip(self);
            self.library_changed_soon();
        }
        match next {
            Some(t) => {
                // Ended on its own and the player already runs the preloaded
                // next song: follow it instead of loading it again. A player
                // that ended without taking the next song still names the old
                // one, which on repeat one is this very song: load it.
                let playing = self
                    .player_state()
                    .filter(|s| !matches!(s.status, glyph_audio::Status::Ended | glyph_audio::Status::Idle))
                    .and_then(|s| s.track);
                let adopted = preloaded
                    .filter(|(k, r)| {
                        auto && *k == glyph_core::dedupe::track_key(&t)
                            && playing.as_ref().is_some_and(|p| p.id == r.id && p.source == r.source)
                    })
                    .map(|(_, r)| r);
                self.start_track_with(t, adopted);
            }
            None => {
                // Radio goes on, or the sleep timer ("end of song") ends here.
                if auto && !crate::discover::on_queue_end(self) {
                    self.toast_key("toast.queue_end", "info");
                }
                self.emit_queue();
                self.emit_player();
            }
        }
    }

    pub fn previous(self: &Arc<Self>) {
        if crate::account::control(self, glyph_protocol::playback::Command::Previous) {
            return;
        }
        if crate::jam::route_previous(self) {
            return;
        }
        if Duration::from_secs_f64(self.player_dto().position) > RESTART_AFTER {
            self.seek(0.0);
            return;
        }
        let prev = self.st().queue.previous().cloned();
        if let Some(t) = prev {
            self.play_track(t);
        }
    }

    /// The song before this one, even when this one has played a while
    /// (swiping back on a phone's cover). Remote and jam: as `previous`.
    pub fn previous_song(self: &Arc<Self>) {
        let early = self.player_state().is_none_or(|s| s.position <= RESTART_AFTER);
        if early || crate::account::controlling(self) || crate::jam::jam_active() {
            self.previous();
            return;
        }
        let prev = self.st().queue.previous().cloned();
        if let Some(t) = prev {
            self.play_track(t);
        }
    }

    pub fn toggle_play(self: &Arc<Self>) {
        // Another Glyph app of the account is controlled: send it there.
        if crate::account::controlling(self) {
            let playing = crate::account::remote_player(self).is_some_and(|r| r.playing);
            let c = if playing { glyph_protocol::playback::Command::Pause } else { glyph_protocol::playback::Command::Play };
            if crate::account::control(self, c) {
                return;
            }
        }
        if crate::jam::route_toggle_play(self) {
            return;
        }
        let has_current = self.st().current.is_some();
        if !has_current {
            self.play_recent();
            return;
        }
        let (resume, resolving) = {
            let mut st = self.st();
            if let Some(pos) = st.restored_pos.take() {
                st.resume_at = (pos > Duration::ZERO).then_some(pos);
                st.resolving = true;
                st.hold_play = false;
                st.error = None;
                (st.current.clone(), false)
            } else {
                if st.resolving {
                    st.hold_play = !st.hold_play;
                }
                (None, st.resolving)
            }
        };
        if let Some(t) = resume {
            self.emit_player();
            self.load_current(t);
            return;
        }
        if resolving {
            self.emit_player();
            return;
        }
        let (stale, has_err) = {
            let st = self.st();
            (st.player_stale, st.error.is_some())
        };
        let was_playing = !stale && self.player_state().is_some_and(|s| s.status == glyph_audio::Status::Playing);
        if let Some(p) = &self.player {
            let idle = stale || has_err || self.player_state().map_or(true, |s| matches!(s.status, glyph_audio::Status::Idle | glyph_audio::Status::Ended));
            if idle {
                // Nothing loaded (after a failed song, or a restart): load it.
                let cur = self.st().current.clone();
                if let Some(t) = cur {
                    self.start_track(t);
                }
                return;
            }
            p.toggle();
            if was_playing {
                self.playback_pos_changed();
            }
        }
        self.emit_player();
    }

    /// Play button with nothing loaded: the songs played lately, from the last one.
    pub fn play_recent(self: &Arc<Self>) {
        let list: Vec<Track> = self.st().library.recent_tracks().cloned().collect();
        if !list.is_empty() {
            self.play_list(list, 0, None);
        }
    }

    /// Plays the `i`-th recently played song (jump list / dock menu).
    pub fn play_recent_at(self: &Arc<Self>, i: usize) {
        let list: Vec<Track> = self.st().library.recent_tracks().cloned().collect();
        if i < list.len() {
            self.play_list(list, i, None);
        }
    }

    pub fn seek(&self, secs: f64) {
        let secs = secs.max(0.0);
        let position_ms = (secs * 1000.0) as u64;
        if crate::account::control(self, glyph_protocol::playback::Command::Seek { position_ms }) {
            return;
        }
        if crate::jam::route_seek(self, secs) {
            self.emit_player();
            return;
        }
        let handled = {
            let dur = self.player_dto().duration;
            let clamped = Duration::from_secs_f64(dur.map_or(secs, |d| secs.min(d.max(0.0))));
            let mut st = self.st();
            if st.restored_pos.is_some() {
                st.restored_pos = Some(clamped);
                true
            } else if st.resolving {
                st.resume_at = Some(clamped);
                true
            } else {
                false
            }
        };
        if !handled {
            if let Some(p) = &self.player {
                p.seek(Duration::from_secs_f64(secs));
            }
        }
        self.playback_pos_changed();
        self.emit_player();
    }

    pub fn set_volume(&self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        if crate::account::control(self, glyph_protocol::playback::Command::Volume { volume: v }) {
            return;
        }
        // A network device's own volume.
        if crate::jam::route_volume(self, v) {
            self.emit_player();
            return;
        }
        if let Some(p) = &self.player {
            p.set_volume(v);
        }
        {
            let mut st = self.st();
            st.muted_volume = None;
            st.config.player.volume = v;
        }
        self.config_dirty.store(true, Ordering::Relaxed);
        self.emit_player();
    }

    pub fn toggle_mute(&self) {
        let restore = {
            let mut st = self.st();
            match st.muted_volume.take() {
                Some(v) => Some(v),
                None => {
                    st.muted_volume = Some(self.player_state().map_or(st.config.player.volume, |s| s.volume));
                    None
                }
            }
        };
        let v = restore.unwrap_or(0.0);
        if crate::account::control(self, glyph_protocol::playback::Command::Volume { volume: v }) {
            return;
        }
        if !crate::jam::route_volume(self, v) {
            if let Some(p) = &self.player {
                p.set_volume(v);
            }
        }
        self.emit_player();
    }

    /// Shuffle on or off; the style (smart / random) is Settings > Playback.
    pub fn set_shuffle(&self, on: bool) {
        {
            let mut st = self.st();
            let recent = Self::recent_keys(&st);
            st.queue.set_recent(recent);
            st.queue.set_shuffle(on);
            st.config.player.shuffle = on;
        }
        self.config_dirty.store(true, Ordering::Relaxed);
        self.queue_changed();
        self.emit_queue();
        self.emit_player();
    }

    /// Settings > Playback: smart or truly random shuffle.
    pub fn apply_shuffle_style(&self) {
        {
            let mut st = self.st();
            let recent = Self::recent_keys(&st);
            let mode = shuffle_mode(&st.config);
            st.queue.set_recent(recent);
            st.queue.set_shuffle_mode(mode);
        }
        self.queue_changed();
        self.emit_queue();
    }

    pub fn set_repeat(&self, mode: &str) {
        {
            let mut st = self.st();
            st.queue.set_repeat(dto::repeat_from(mode));
            st.config.player.repeat = mode.to_string();
        }
        self.config_dirty.store(true, Ordering::Relaxed);
        self.queue_changed();
        self.emit_player();
    }

    pub fn set_loop(&self, a: Option<f64>, b: Option<f64>) {
        let ab = a.zip(b).filter(|(a, b)| b > a);
        self.st().loop_ab = ab;
        if let Some(p) = &self.player {
            // Embedded players cannot loop by themselves: the pump seeks them back.
            let embed = self.player_state().is_some_and(|s| s.backend == Some(glyph_audio::BackendKind::Embed));
            p.set_loop(if embed { None } else { ab.map(|(a, b)| (Duration::from_secs_f64(a), Duration::from_secs_f64(b))) });
        }
        self.emit_player();
    }

    // ---- queue ----

    pub fn queue_add(&self, tracks: Vec<Track>, next: bool) {
        // Offline: only saved songs; hidden explicit songs never join.
        let mut blocked = false;
        let tracks: Vec<Track> = tracks
            .into_iter()
            .filter(|t| {
                // In someone else's jam the songs are asked for, not queued here.
                if crate::jam::route_request(self, t, false) {
                    return false;
                }
                if self.playable(t) {
                    return true;
                }
                blocked = true;
                false
            })
            .collect();
        if blocked {
            let key = if crate::storage::is_offline() { "offline.unavailable" } else { "toast.explicit_blocked" };
            self.toast_key(key, "info");
        }
        if tracks.is_empty() {
            return;
        }
        {
            let mut st = self.st();
            if next {
                for t in tracks.into_iter().rev() {
                    st.queue.play_next(t);
                }
            } else {
                for t in tracks {
                    st.queue.push(t);
                }
            }
        }
        self.queue_changed();
        self.emit_queue();
    }

    pub fn queue_remove(&self, ui_index: usize) {
        if crate::jam::route_queue_edit(self, Some(ui_index)) {
            return;
        }
        {
            let mut st = self.st();
            if let Some(i) = dto::upcoming_index(&st.queue, ui_index) {
                st.queue.remove(i);
            }
        }
        self.queue_changed();
        self.emit_queue();
    }

    pub fn queue_move(&self, from: usize, to: usize) {
        if crate::jam::route_queue_edit(self, None) {
            return;
        }
        {
            let mut st = self.st();
            if let (Some(f), Some(t)) = (dto::upcoming_index(&st.queue, from), dto::upcoming_index(&st.queue, to)) {
                st.queue.move_item(f, t);
            }
        }
        self.queue_changed();
        self.emit_queue();
    }

    pub fn queue_jump(self: &Arc<Self>, ui_index: usize) {
        let upcoming = {
            let st = self.st();
            dto::upcoming_index(&st.queue, ui_index).and_then(|i| st.queue.upcoming().get(i).map(|t| (*t).clone()))
        };
        if let Some(t) = &upcoming {
            if self.blocked_offline(t) {
                return;
            }
        }
        if crate::jam::route_queue_jump(self, ui_index) {
            return;
        }
        let t = {
            let mut st = self.st();
            dto::upcoming_index(&st.queue, ui_index).and_then(|i| st.queue.jump_to_upcoming(i).cloned())
        };
        if let Some(t) = t {
            self.play_track(t);
        }
    }

    pub fn queue_clear(&self) {
        if crate::jam::route_queue_edit(self, None) {
            return;
        }
        {
            let mut st = self.st();
            let upcoming = st.queue.upcoming().len();
            for i in (0..upcoming).rev() {
                st.queue.remove(i);
            }
        }
        self.queue_changed();
        self.emit_queue();
    }

    // ---- gapless ----

    /// Looks up the next song while this one plays and hands it to the
    /// player, so it starts without a gap (from `src/app/playback.rs`).
    pub fn preload_next(self: &Arc<Self>) {
        // Sleep timer "end of this song": the next one must not start on its own.
        // MPD and network devices get the next song when this one ends; at
        // another Glyph app of the account nothing plays here.
        if crate::discover::sleep_blocks_preload(self) || crate::jam::remote_on() || crate::account::controlling(self) {
            return;
        }
        let Some(backend) = self.backend() else { return };
        let (cur, next) = {
            let st = self.st();
            let Some(cur) = st.current.as_ref().map(glyph_core::dedupe::track_key) else { return };
            let Some(next) = st.queue.peek_next().cloned() else { return };
            if st.resolving {
                return;
            }
            (cur, next)
        };
        if !self.playable(&next) {
            return;
        }
        let want = glyph_core::dedupe::track_key(&next);
        let attempt = Instant::now();
        {
            let mut st = self.st();
            if st.preloaded.as_ref().is_some_and(|(k, _)| k == &want) {
                return;
            }
            if st.preload_attempt.as_ref().is_some_and(|(c, n, at)| c == &cur && n == &want && at.elapsed() < Duration::from_secs(10)) {
                return;
            }
            st.preload_attempt = Some((cur.clone(), want.clone(), attempt));
        }
        let core = self.clone();
        crate::rt::spawn(async move {
            let Ok(Some(res)) = backend.resolve(&next).await else { return };
            let ok = core.playable(&next);
            let mut st = core.st();
            // Another song started or the queue changed meanwhile.
            let now_cur = st.current.as_ref().map(glyph_core::dedupe::track_key);
            let now_next = st.queue.peek_next().map(glyph_core::dedupe::track_key);
            if now_cur.as_deref() != Some(cur.as_str())
                || now_next.as_deref() != Some(want.as_str())
                || !ok
                || st.preload_attempt.as_ref().map(|(_, _, at)| *at) != Some(attempt)
            {
                return;
            }
            st.preloaded = Some((want, res.track.clone()));
            drop(st);
            if let Some(p) = &core.player {
                p.preload_next(res.track, res.source);
            }
        });
    }

    /// The audio engine dropped a preloaded track after an error: clear `preloaded` so it can retry.
    pub fn on_preload_failed(&self) {
        self.st().preloaded = None;
    }

    /// After a queue edit: a preloaded song that is no longer next must not
    /// start after the current one.
    pub fn queue_changed(&self) {
        let stale = {
            let mut st = self.st();
            let next = st.queue.peek_next().map(glyph_core::dedupe::track_key);
            let stale = st.preloaded.as_ref().is_some_and(|(k, _)| next.as_deref() != Some(k.as_str()));
            if stale {
                st.preloaded = None;
            }
            stale
        };
        if stale {
            if let Some(p) = &self.player {
                p.drop_next();
            }
        }
        self.playback_changed();
    }

    /// The connection went (phones): a preloaded song that is not saved on
    /// the device is dropped, so it does not start from the network.
    pub fn on_offline_changed(&self, offline: bool) {
        if !offline {
            glyph_core::outage::clear();
            if let Some(b) = self.backend() {
                b.clear_cooldowns();
            }
            return;
        }
        let stale = {
            let mut st = self.st();
            let next_unsaved = st.queue.peek_next().is_some_and(|t| !crate::storage::available_offline(self, t));
            if next_unsaved {
                st.preloaded = None;
                st.preload_attempt = None;
            }
            next_unsaved
        };
        if stale {
            if let Some(p) = &self.player {
                p.drop_next();
            }
        }
    }

    // ---- background work ----

    /// Player events, state changes and delayed saves, ten times a second.
    pub fn spawn_pump(self: &Arc<Self>) {
        let core = self.clone();
        std::thread::Builder::new()
            .name("glyph-pump".into())
            .spawn(move || {
                let events = core.player.as_ref().map(|p| p.events());
                let mut last: Option<PlayerState> = None;
                // What the UI was last told, and when: it counts the time on
                // its own, so a moving position alone is not sent ten times a
                // second (each report redrew the player everywhere).
                let mut sent: Option<(PlayerState, Instant)> = None;
                let mut ticks: u32 = 0;
                let mut outage_seen: u64 = 0;
                let mut was_playing = false;
                loop {
                    std::thread::sleep(Duration::from_millis(100));
                    ticks = ticks.wrapping_add(1);
                    core.flush_library_soon();
                    if let Some(rx) = &events {
                        while let Ok(ev) = rx.try_recv() {
                            match ev {
                                PlayerEvent::Ended => {
                                    core.playback_pos_changed();
                                    crate::discover::on_song_ended(&core);
                                    core.next(true);
                                }
                                PlayerEvent::Error(e) => core.on_player_error(e),
                                PlayerEvent::StatusChanged(_) => core.emit_player(),
                                PlayerEvent::NearEnd => core.preload_next(),
                                PlayerEvent::PreloadFailed => core.on_preload_failed(),
                            }
                        }
                    }
                    if let Some(p) = &core.player {
                        if let Some(s) = p.state_if_changed(last.as_ref()) {
                            if player_report_due(sent.as_ref(), &s) {
                                sent = Some((s.clone(), Instant::now()));
                                core.emit_player();
                            }
                            last = Some(s);
                        }
                        // Open the next audio stream near the end (~30 s left) so mobile links/TCP do not stall while locked.
                        if last.as_ref().is_some_and(|s| {
                            s.status == glyph_audio::Status::Playing
                                && s.duration.is_some_and(|d| d.saturating_sub(s.position) <= Duration::from_secs(30))
                        }) {
                            core.preload_next();
                        }
                    }
                    core.tick_playback(last.as_ref(), &mut was_playing);
                    // Every minute: songs deleted or added outside Glyph.
                    if ticks % 600 == 0 {
                        if let Some(b) = core.backend() {
                            b.refresh_downloads();
                        }
                    }
                    if ticks % 10 == 0 {
                        core.announce_outage(&mut outage_seen);
                    }
                    // The embed player and the OS parts run on the main thread.
                    let c = core.clone();
                    core.host.run_on_main_thread(Box::new(move || {
                        if let Some(p) = &c.player {
                            p.pump_embed();
                        }
                        c.host.on_tick(&c);
                    }));
                    crate::discover::on_tick(&core);
                    // Remote commands from the account wait while a jam runs.
                    crate::account::set_jam_active(crate::jam::jam_active());
                    // Writes wait a moment so a burst of changes is one save.
                    if ticks % 10 == 0 {
                        core.save_if_dirty();
                    }
                }
            })
            .expect("spawn pump thread");
    }

    /// Per tick: the skip streak ends, playback position sidecar is marked
    /// dirty on transitions/intervals, and an A-B loop runs in embedded
    /// players (native ones loop in the player itself).
    fn tick_playback(self: &Arc<Self>, ps: Option<&PlayerState>, was_playing: &mut bool) {
        let Some(ps) = ps else {
            *was_playing = false;
            return;
        };
        let playing = ps.status == glyph_audio::Status::Playing;
        let ended = ps.status == glyph_audio::Status::Ended;
        // A song that plays ends the skip streak (the listen itself and
        // the list it came from are counted in `discover::on_tick`).
        let (has_current, resolving, seq) = {
            let mut st = self.st();
            if playing && !st.resolving {
                st.skip_streak = 0;
            }
            (st.current.is_some(), st.resolving, st.playback_seq)
        };
        if !resolving && has_current {
            if (*was_playing && !playing) || ended {
                self.playback_pos_changed();
            } else if playing {
                let pos_ms = ps.position.as_millis() as u64;
                let saved = *self.playback_pos_saved.lock().unwrap_or_else(|e| e.into_inner());
                let due = match saved {
                    Some((s, saved_ms)) if s == seq => pos_ms.abs_diff(saved_ms) >= PLAYBACK_POS_INTERVAL_MS,
                    _ => pos_ms >= PLAYBACK_POS_INTERVAL_MS,
                };
                if due {
                    self.playback_pos_dirty.store(true, Ordering::Relaxed);
                }
            }
        }
        *was_playing = playing;
        if ps.backend == Some(glyph_audio::BackendKind::Embed) {
            let ab = self.st().loop_ab;
            if let Some((a, b)) = ab {
                if ps.position.as_secs_f64() >= b {
                    self.seek(a);
                }
            }
        }
    }

    /// The official Glyph server went out (or this device has no internet):
    /// says once that search and playback run locally in the meantime.
    fn announce_outage(&self, seen: &mut u64) {
        use glyph_core::outage::{self, Kind};
        let Some(o) = outage::current() else { return };
        let official = outage::is_official(self.st().config.effective_server_url());
        if o.seq == *seen || !official {
            return;
        }
        *seen = o.seq;
        let (key, sub) = match o.kind {
            Kind::Offline => ("toast.server_out.offline", "toast.server_out.local"),
            Kind::DailyLimit => ("toast.server_out.limit", "toast.server_out.local"),
            Kind::NoInternet => ("toast.no_internet", "toast.no_internet.sub"),
        };
        self.emit("toast", json!({ "key": key, "text": key, "sub": crate::i18n::tr(self, sub), "kind": "info" }));
    }

    fn can_persist_playback(&self) -> bool {
        !crate::account::controlling(self) && !crate::jam::is_guest()
    }

    pub fn playback_changed(&self) {
        if self.can_persist_playback() {
            self.playback_dirty.store(true, Ordering::Relaxed);
        }
    }

    pub fn playback_pos_changed(&self) {
        if self.can_persist_playback() && self.st().current.is_some() {
            self.playback_pos_dirty.store(true, Ordering::Relaxed);
        }
    }

    fn playback_session_snapshot(&self) -> Option<(std::path::PathBuf, glyph_core::PlaybackSession)> {
        if !self.can_persist_playback() {
            return None;
        }
        let ps = self.player_state();
        let st = self.st();
        let ps = if st.player_stale { None } else { ps };
        let path = st.library.storage_path.0.clone().unwrap_or_else(Library::path);
        let current = st.current.clone();
        let (position_ms, duration_ms, activity_recorded) = if current.is_some() {
            let ended = ps.as_ref().is_some_and(|s| s.status == glyph_audio::Status::Ended);
            let mut pos_ms = if st.resolving {
                st.resume_at.map_or(0, |d| d.as_millis() as u64)
            } else if let Some(pos) = st.restored_pos {
                pos.as_millis() as u64
            } else {
                ps.as_ref().map_or(0, |s| s.position.as_millis() as u64)
            };
            let dur_ms = ps
                .as_ref()
                .and_then(|s| s.duration)
                .or(st.restored_duration)
                .map(|d| d.as_millis() as u64)
                .or_else(|| current.as_ref().and_then(|t| t.duration_ms));
            if ended || dur_ms.is_some_and(|d| d > 0 && pos_ms >= d.saturating_sub(2_000)) {
                pos_ms = 0;
            }
            (pos_ms, dur_ms, st.listen_counted)
        } else {
            (0, None, false)
        };
        let sess = glyph_core::PlaybackSession {
            current,
            position_ms,
            duration_ms,
            seq: st.playback_seq,
            activity_recorded,
            queue: st.queue.snapshot(),
            playing_from: st.playing_from.clone(),
            queue_from: st.context.clone(),
            recent_artist_ctx: st.recent_artist_ctx.clone(),
            saved_at: glyph_core::paths::now_unix(),
        };
        Some((path, sess))
    }

    fn playback_position_snapshot(&self) -> Option<(std::path::PathBuf, glyph_core::PlaybackPosition)> {
        if !self.can_persist_playback() {
            return None;
        }
        let ps = self.player_state();
        let st = self.st();
        let ps = if st.player_stale { None } else { ps };
        let track = st.current.as_ref()?;
        let path = st.library.storage_path.0.clone().unwrap_or_else(Library::path);
        let ended = ps.as_ref().is_some_and(|s| s.status == glyph_audio::Status::Ended);
        let mut pos_ms = if st.resolving {
            st.resume_at.map_or(0, |d| d.as_millis() as u64)
        } else if let Some(pos) = st.restored_pos {
            pos.as_millis() as u64
        } else {
            ps.as_ref().map_or(0, |s| s.position.as_millis() as u64)
        };
        let dur_ms = ps
            .as_ref()
            .and_then(|s| s.duration)
            .or(st.restored_duration)
            .map(|d| d.as_millis() as u64)
            .or(track.duration_ms);
        if ended || dur_ms.is_some_and(|d| d > 0 && pos_ms >= d.saturating_sub(2_000)) {
            pos_ms = 0;
        }
        Some((
            path,
            glyph_core::PlaybackPosition {
                track_key: glyph_core::dedupe::track_key(track),
                seq: st.playback_seq,
                position_ms: pos_ms,
                duration_ms: dur_ms,
                activity_recorded: st.listen_counted,
            },
        ))
    }

    pub fn restore_playback_session(&self, session: Option<glyph_core::PlaybackSession>) {
        let Some(sess) = session else { return };
        if sess.is_empty() {
            return;
        }
        let mut restored_queue = glyph_core::Queue::new();
        restored_queue.restore(sess.queue);
        let Some(track) = sess.current.or_else(|| restored_queue.current().cloned()) else {
            return;
        };
        let dur_ms = sess.duration_ms.or(track.duration_ms);
        let pos_ms = if dur_ms.is_some_and(|d| d > 0 && sess.position_ms >= d.saturating_sub(2_000)) {
            0
        } else {
            sess.position_ms
        };
        let seq = {
            let mut st = self.st();
            if st.current.is_some() {
                return;
            }
            let recent = Self::recent_keys(&st);
            st.queue = restored_queue;
            st.queue.set_recent(recent);
            st.playback_seq = sess.seq.max(1);
            st.current = Some(track);
            st.context = sess.queue_from;
            st.playing_from = sess.playing_from;
            st.recent_artist_ctx = sess.recent_artist_ctx;
            st.listen_counted = sess.activity_recorded;
            st.restored_pos = Some(Duration::from_millis(pos_ms));
            st.restored_duration = dur_ms.map(Duration::from_millis);
            st.resume_at = None;
            st.hold_play = false;
            st.resolving = false;
            st.playback_seq
        };
        *self.playback_pos_saved.lock().unwrap_or_else(|e| e.into_inner()) = Some((seq, pos_ms));
    }

    pub fn save_playback_now(&self) {
        self.playback_dirty.store(false, Ordering::Relaxed);
        self.playback_pos_dirty.store(false, Ordering::Relaxed);
        if let Some((path, snap)) = self.playback_session_snapshot() {
            if snap.current.is_some() {
                *self.playback_pos_saved.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some((snap.seq, snap.position_ms));
            }
            if let Err(e) = snap.save_to(&path) {
                tracing::warn!(error = %e, "playback session: save failed");
            }
        }
    }

    pub fn save_all_now(&self) {
        self.save_if_dirty();
        self.save_playback_now();
    }

    pub fn save_if_dirty(&self) {
        if self.library_dirty.swap(false, Ordering::Relaxed) {
            let lib = {
                let mut st = self.st();
                st.library.checkpoint_playlists();
                st.library.clone()
            };
            if let Err(e) = lib.save() {
                tracing::warn!(error = %e, "library: save failed");
            }
        }
        if self.config_dirty.swap(false, Ordering::Relaxed) {
            let cfg = self.st().config.clone();
            if let Err(e) = cfg.save() {
                tracing::warn!(error = %e, "config: save failed");
            }
        }
        if self.playback_dirty.swap(false, Ordering::Relaxed) {
            self.playback_pos_dirty.store(false, Ordering::Relaxed);
            if let Some((path, snap)) = self.playback_session_snapshot() {
                if snap.current.is_some() {
                    *self.playback_pos_saved.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some((snap.seq, snap.position_ms));
                }
                if let Err(e) = snap.save_to(&path) {
                    tracing::warn!(error = %e, "playback session: save failed");
                }
            }
        } else if self.playback_pos_dirty.swap(false, Ordering::Relaxed) {
            if let Some((path, pos)) = self.playback_position_snapshot() {
                *self.playback_pos_saved.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some((pos.seq, pos.position_ms));
                if let Err(e) = pos.save_to(&path) {
                    tracing::warn!(error = %e, "playback position: save failed");
                }
            }
        }
    }

    /// Shrinks transient Rust caches and signals the UI to trim image/page
    /// caches when the process tree approaches the RAM limit.
    pub fn trim_memory(&self, aggressive: bool) {
        if let Some(b) = self.backend() {
            b.trim_caches(aggressive);
        }
        crate::commands::trim_verified_cache(aggressive);
        if aggressive {
            crate::commands::trim_artist_cache();
        }
        if aggressive {
            let mut st = self.st();
            st.preload_attempt = None;
            st.search.seen.shrink_to_fit();
            st.search.tracks.shrink_to_fit();
            st.search.scores.shrink_to_fit();
            st.feeds.recs.shrink_to_fit();
            st.feeds.trending.shrink_to_fit();
            st.feeds.new_releases.shrink_to_fit();
        }
        self.emit(
            "memory_pressure",
            json!({ "level": if aggressive { "hard" } else { "soft" } }),
        );
    }
}

/// Settings > Playback: smart or truly random shuffle.
pub fn shuffle_mode(cfg: &Config) -> glyph_core::queue::ShuffleMode {
    match cfg.playback.shuffle {
        glyph_core::config::ShuffleStyle::Smart => glyph_core::queue::ShuffleMode::Smart,
        glyph_core::config::ShuffleStyle::Random => glyph_core::queue::ShuffleMode::Random,
    }
}

/// A problem of the audio output itself: every song would fail the same
/// way, so skipping to the next one does not help.
/// A resolve error from the network itself (no connection, DNS, timeout),
/// which usually heals within seconds, not from the song or the source.
fn transient_net_error(e: &str) -> bool {
    let e = e.to_ascii_lowercase();
    ["error sending request", "connection", "timed out", "timeout", "dns", "network is unreachable", "no route to host", "temporary failure", "backend_offline"]
        .iter()
        .any(|p| e.contains(p))
}

fn output_error(e: &str) -> bool {
    ["no audio output device", "audio output not available", "The selected audio output"].iter().any(|p| e.starts_with(p))
}

/// A failed resolve as a toast key with values: the known YouTube cases get
/// their own sentence instead of the raw (English) error.
fn resolve_error(e: &str) -> (&'static str, Value) {
    if e.contains(glyph_core::ERR_NOT_ON_YOUTUBE) {
        ("toast.not_on_youtube", Value::Null)
    } else if e.contains(glyph_core::ERR_YOUTUBE_FAILED) {
        ("toast.youtube_failed", Value::Null)
    } else {
        ("toast.resolve_error", json!({ "e": e }))
    }
}


impl Core {
    /// Drops cached search results and bumps `seq` so in-flight responses are ignored.
    pub fn clear_search_state(&self) {
        {
            let mut st = self.st();
            let seq = st.search.seq.wrapping_add(1);
            st.search = SearchCache::default();
            st.search.seq = seq;
        }
        crate::mcp_tools::remember_search(&[]);
    }
}

/// The player changed in a way the UI cannot work out itself: another song,
/// status, volume..., or the position is off from where the UI's own clock
/// has it (a seek, buffering) by more than 0.75 s. Every 5 s it is sent
/// anyway, to keep the UI's clock in step.
fn player_report_due(sent: Option<&(PlayerState, Instant)>, now: &PlayerState) -> bool {
    let Some((old, at)) = sent else { return true };
    if !now.same_but_position(old) {
        return true;
    }
    let elapsed = at.elapsed();
    if elapsed >= Duration::from_secs(5) {
        return true;
    }
    let expected = if old.status == glyph_audio::Status::Playing { old.position + elapsed } else { old.position };
    let off = if now.position > expected { now.position - expected } else { expected - now.position };
    off > Duration::from_millis(750)
}

#[cfg(test)]
mod player_report_tests {
    use super::*;

    fn state(status: glyph_audio::Status, pos_ms: u64) -> PlayerState {
        PlayerState {
            status,
            track: None,
            provider: None,
            backend: None,
            is_preview: false,
            position: Duration::from_millis(pos_ms),
            duration: Some(Duration::from_secs(200)),
            volume: 1.0,
        }
    }

    #[test]
    fn position_alone_is_not_reported_every_tick() {
        let sent = (state(glyph_audio::Status::Playing, 10_000), Instant::now());
        // 100 ms later the song is 100 ms further: the UI knows that already.
        assert!(!player_report_due(Some(&sent), &state(glyph_audio::Status::Playing, 10_100)));
    }

    #[test]
    fn first_report_status_change_and_seek_are_sent() {
        assert!(player_report_due(None, &state(glyph_audio::Status::Playing, 0)));
        let sent = (state(glyph_audio::Status::Playing, 10_000), Instant::now());
        assert!(player_report_due(Some(&sent), &state(glyph_audio::Status::Paused, 10_000)));
        // Jumped 30 s ahead (a seek).
        assert!(player_report_due(Some(&sent), &state(glyph_audio::Status::Playing, 40_000)));
    }
}

/// Stable hash of a JSON value (library parts: did this part change?).
fn value_hash(v: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(v).unwrap_or_default().hash(&mut h);
    h.finish()
}
