//! Account, profile, privacy and the Glyph server: the commands of
//! Settings > Account / Profile / Server, library sync with the server
//! (src/app/account_library.rs, src/ui/account.rs) and the account's device
//! list with remote playback (src/app/account_devices.rs).
//!
//! `Acct` holds the account's own state. One background task (`run`) syncs the
//! library, switches the library on sign-in / sign-out and keeps the remote
//! playback line to the other devices of the account.

use crate::controller::Core;
use futures::StreamExt;
use glyph_audio::Status;
use glyph_core::account::{is_signed_out, Stored};
use glyph_core::server_client::{server_error, PlaybackLive};
use glyph_core::{Backend, Config, Library};
use glyph_protocol::playback::{Command, Live, Output, Receipt, Send as PlaySend, State as PlayState};
use glyph_protocol::{AccountPatch, LoginPollResponse, LoginStartResponse, ProfilePatch, ProfileSettings, Track};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

type Res<T = ()> = Result<T, String>;
type C<'a> = &'a Arc<Core>;

/// How often a signed-in app syncs on its own.
const SYNC_EVERY: Duration = Duration::from_secs(5 * 60);
/// Quiet time after a library change before it is pushed.
const SYNC_DEBOUNCE: Duration = Duration::from_secs(2);
/// Recent songs for the profile are sent this long after the last change.
const RECENT_DEBOUNCE: Duration = Duration::from_secs(5);
const RECENT_SHOWN: usize = 20;
/// Last report older than this: waiting commands are refused.
const STALE_POLL: Duration = Duration::from_secs(25);
/// A received transfer is confirmed after this at the latest.
const TRANSFER_ACK_MAX: Duration = Duration::from_secs(18);
/// The controlled device may miss reports this long.
const OUTPUT_GRACE: Duration = Duration::from_secs(20);
/// Play / pause sent to the controlled device shows this long.
const OUTPUT_OPTIMISTIC: Duration = Duration::from_secs(6);
/// Polls failing in a row before the device list is given up.
const POLL_FAILURES: u32 = 3;
/// Poll interval while device states arrive over the live line.
const LIVE_POLL: u64 = 30;
/// A controlled song's position runs on this long without news.
const POSITION_RUN_ON: Duration = Duration::from_secs(40);

// ------------------------------------------------------------------ state

#[derive(Default)]
struct Inner {
    merge_pending: bool,
    signing_out: bool,
    syncing: bool,
    /// Bumped on every library change; a debounced sync only runs if unchanged.
    sync_gen: u64,
    sync_failures: u32,
    sync_pending: usize,
    /// Unix ms.
    last_sync: Option<u64>,
    sync_error: Option<String>,
    available: Option<(bool, Instant)>,
    /// Sign-ins that wait for approval (the UI only knows the id).
    logins: HashMap<String, LoginStartResponse>,
    /// Profile pictures as data URLs, by hash.
    images: HashMap<String, String>,
    own_avatar: Option<String>,
    settings: Option<ProfileSettings>,
    recent_gen: u64,
    recent_sent: Vec<String>,

    // remote playback
    outputs: Vec<Output>,
    target: Option<String>,
    live: Option<PlaybackLive>,
    live_since: Option<Instant>,
    live_states: bool,
    live_connecting: bool,
    live_checked: Option<Instant>,
    reported: Option<(PlayState, Instant)>,
    state_at: Option<Instant>,
    seen: BTreeMap<String, Option<String>>,
    transfer_ack: Option<String>,
    transfer_ack_at: Option<Instant>,
    transfer_to: Option<String>,
    error: Option<String>,
    error_at: Option<String>,
    playing_hint: Option<(bool, Instant)>,
    failures: u32,
    missing_since: Option<Instant>,
    checked: Option<Instant>,
    received: Option<Instant>,
    busy: bool,
    commands: Vec<std::sync::mpsc::Receiver<anyhow::Result<()>>>,
    /// Position and play state to restore once a moved song has loaded.
    resume: Option<(u64, bool, Instant)>,
}

pub struct Acct {
    core: Arc<Core>,
    m: Mutex<Inner>,
    sync_wake: tokio::sync::Notify,
}

static JAM_ACTIVE: AtomicBool = AtomicBool::new(false);
static DEVICES_OPEN: AtomicBool = AtomicBool::new(false);
static ON_DEVICES: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Called once at start-up from `glyph_app::setup`.
pub fn setup(core: &Arc<Core>) {
    glyph_audio::http::set_account_access_provider(Arc::new(glyph_core::account::media_access));
    let acc = Arc::new(Acct { core: core.clone(), m: Mutex::new(Inner::default()), sync_wake: tokio::sync::Notify::new() });
    core.set_acct(acc.clone());
    crate::rt::spawn(acc.run());
}

/// Wakes account sync and remote playback after the server reconnects.
pub fn wake_reconnect(core: &Core) {
    if let Some(acc) = acct(core) {
        {
            let mut g = acc.g();
            g.sync_failures = 0;
            g.live_checked = None;
            g.checked = None;
        }
        acc.sync_wake.notify_one();
    }
}

fn acct(core: &Core) -> Option<Arc<Acct>> {
    core.acct()
}

fn need_acct(core: &Core) -> Res<Arc<Acct>> {
    core.acct().ok_or_else(|| "account not ready".to_string())
}

// ------------------------------------------------------------------ hooks for controller.rs / jam.rs / the device list

/// The signed-in account for `snapshot()` and the "account" event (`null` signed out).
pub fn account_json(core: &Core) -> Value {
    match acct(core) {
        Some(a) => a.account_value(),
        None => Value::Null,
    }
}

/// The library changed (call from `Core::library_changed`): syncs soon and
/// sends the recent songs to the profile.
pub fn library_changed(core: &Core) {
    if let Some(a) = acct(core) {
        a.schedule_sync();
        a.profile_recent_changed();
    }
}

/// A jam runs (this app takes no remote commands then). Call from jam.rs.
pub fn set_jam_active(on: bool) {
    JAM_ACTIVE.store(on, Ordering::Relaxed);
}

/// The Connect panel is open (devices are polled more often).
pub fn set_devices_open(on: bool) {
    DEVICES_OPEN.store(on, Ordering::Relaxed);
}

/// The Connect panel opened or closed (the UI calls this).
pub fn account_devices_open(core: C<'_>, open: bool) {
    set_devices_open(open);
    if open {
        // Look again at once.
        if let Some(acc) = core.acct() {
            acc.g().checked = None;
        }
    }
}

/// A song is about to start: while another Glyph app is controlled it is
/// sent there instead (`true` = handled, do not play here). Call at the top
/// of `Core::start_track`.
pub fn route_play(core: &Core, track: &Track) -> bool {
    control(core, Command::Transfer { tracks: vec![track.clone()], index: 0, position_ms: 0, playing: true })
}

/// The controlled Glyph app is let go and stopped (another output is chosen).
pub fn release_output(core: &Core) {
    if let Some(a) = acct(core) {
        let controlled = a.g().target.is_some();
        if controlled {
            a.control(Command::Stop);
            a.g().target = None;
        }
    }
}

/// A song moved here could not be found: the sender hears about it.
pub fn resolve_failed(core: &Core) {
    if let Some(a) = acct(core) {
        a.g().resume = None;
        a.finish_transfer(Some("No playable source on target device".to_string()));
    }
}

/// A song moved here is still starting.
pub fn transfer_pending(core: &Core) -> bool {
    acct(core).is_some_and(|a| {
        let g = a.g();
        g.transfer_ack.is_some() || g.resume.is_some()
    })
}

/// Registers what to do when the account's devices changed (re-emit "devices").
pub fn on_devices_changed(f: impl Fn() + Send + Sync + 'static) {
    let _ = ON_DEVICES.set(Box::new(f));
}

fn devices_changed() {
    if let Some(f) = ON_DEVICES.get() {
        f();
    }
}

/// Other Glyph apps of the account as `DeviceX` entries (kind "account").
pub fn devices_json(core: &Core) -> Vec<Value> {
    let Some(a) = acct(core) else { return Vec::new() };
    let g = a.g();
    g.outputs
        .iter()
        .filter(|o| !o.device.current)
        .map(|o| {
            let track = o.state.track.as_ref().map(|t| format!("{} · {}", t.artist, t.title));
            json!({
                "id": o.device.id,
                "name": o.device.name,
                "kind": "account",
                "active": g.target.as_ref() == Some(&o.device.id),
                "offline": !o.online,
                "platform": o.device.platform.to_lowercase(),
                "playing": o.online && o.state.playing,
                "track": track,
                "busy": !o.state.available,
                "connecting": g.transfer_to.as_ref() == Some(&o.device.id),
                "error": if g.error_at.as_ref() == Some(&o.device.id) { g.error.clone() } else { None },
            })
        })
        .collect()
}

/// This app controls another device: what the player bar should show.
#[derive(Debug, Clone, Serialize)]
pub struct RemotePlayer {
    pub device: String,
    pub track: Option<Track>,
    pub playing: bool,
    /// Seconds.
    pub position: f64,
    pub duration: Option<f64>,
    pub volume: f32,
    pub error: Option<String>,
}

pub fn remote_player(core: &Core) -> Option<RemotePlayer> {
    let a = acct(core)?;
    let g = a.g();
    let id = g.target.as_ref()?;
    let o = g.outputs.iter().find(|o| &o.device.id == id)?;
    let playing = g.playing_hint.filter(|(_, at)| at.elapsed() < OUTPUT_OPTIMISTIC).map_or(o.state.playing, |(p, _)| p);
    Some(RemotePlayer {
        device: o.device.name.clone(),
        track: o.state.track.clone(),
        playing,
        position: a.output_position(&g, o).as_secs_f64(),
        duration: o.state.duration_ms.map(|d| d as f64 / 1000.0),
        volume: o.state.volume,
        error: g.error.clone(),
    })
}

/// True while another device of the account is controlled.
pub fn controlling(core: &Core) -> bool {
    acct(core).is_some_and(|a| a.g().target.is_some())
}

/// Sends `command` to the controlled device; false when none is controlled
/// (then the caller acts on the local player).
pub fn control(core: &Core, command: Command) -> bool {
    acct(core).is_some_and(|a| a.control(command))
}

/// A click on a Glyph device of the account: moves playback there, or takes
/// over what plays there.
pub fn connect_output(core: &Core, id: String) {
    if let Some(a) = acct(core) {
        a.connect_output(id);
    }
}

/// Back to this computer. True when an account device was controlled.
pub fn use_local(core: &Core) -> bool {
    acct(core).is_some_and(|a| a.use_local())
}

/// Snapshot of known account devices (`outputs`) and the currently controlled device id.
pub fn outputs_snapshot(core: &Core) -> (Vec<Output>, Option<String>) {
    let Some(a) = acct(core) else { return (Vec::new(), None) };
    let g = a.g();
    (g.outputs.clone(), g.target.clone())
}

/// Switches this app into remote-control mode for account device `id`.
pub fn select_output_now(core: &Core, id: String) {
    if let Some(a) = acct(core) {
        a.select_output(id);
    }
}

/// The player failed (call from the player-event pump): a moved song that
/// could not start is reported to the sender.
pub fn player_error(core: &Core, error: &str) {
    if let Some(a) = acct(core) {
        a.finish_transfer(Some(error.to_string()));
    }
}

// ------------------------------------------------------------------ small helpers

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn track_key(t: &Track) -> String {
    glyph_core::dedupe::track_key(t)
}

/// Short code the UI maps to a text (`account-state.svelte.ts` CODES).
fn code(e: &anyhow::Error) -> String {
    if is_signed_out(e) {
        return "signed_out".into();
    }
    let m = e.to_string();
    if matches!(m.as_str(), "login_failed" | "password_too_short" | "password_too_long" | "wrong_password" | "no_password" | "bad_salt") {
        return m;
    }
    match server_error(e) {
        Some((404, _)) => "not_found".into(),
        Some((_, "account_full")) => "account_full".into(),
        Some((413, _)) => "too_big".into(),
        Some((_, c))
            if matches!(c, "login_failed" | "device_limit" | "handle_taken" | "handle_invalid" | "handle_reserved" | "handle_cooldown" | "too_many_devices" | "not_found" | "rate_limited") =>
        {
            c.to_string()
        }
        _ => "generic".into(),
    }
}

fn err(e: anyhow::Error) -> String {
    code(&e)
}

fn offline() -> String {
    "backend_offline".into()
}

// i18n for the few texts Rust sends as words (errors in the status); the UI
// translates everything else from keys.

const EN: &str = include_str!("../../../assets/i18n/en.toml");

fn table(lang: &str) -> &'static str {
    match lang {
        "de" => include_str!("../../../assets/i18n/de.toml"),
        "ru" => include_str!("../../../assets/i18n/ru.toml"),
        "es" => include_str!("../../../assets/i18n/es.toml"),
        "fr" => include_str!("../../../assets/i18n/fr.toml"),
        "pt" => include_str!("../../../assets/i18n/pt.toml"),
        "it" => include_str!("../../../assets/i18n/it.toml"),
        "pl" => include_str!("../../../assets/i18n/pl.toml"),
        "tr" => include_str!("../../../assets/i18n/tr.toml"),
        "uk" => include_str!("../../../assets/i18n/uk.toml"),
        "af" => include_str!("../../../assets/i18n/af.toml"),
        _ => EN,
    }
}

fn lookup(src: &str, key: &str) -> Option<String> {
    for line in src.lines() {
        let Some(rest) = line.strip_prefix('"') else { continue };
        let Some((k, v)) = rest.split_once("\" = \"") else { continue };
        if k != key {
            continue;
        }
        let v = v.trim_end().strip_suffix('"')?;
        let mut out = String::new();
        let mut it = v.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => {}
            }
        }
        return Some(out);
    }
    None
}

fn tr(core: &Core, key: &str) -> String {
    let mut lang = core.st().config.language.trim().to_lowercase();
    if lang.is_empty() {
        lang = ["LC_ALL", "LANGUAGE", "LANG"].iter().find_map(|k| std::env::var(k).ok()).unwrap_or_default().chars().take(2).collect::<String>().to_lowercase();
    }
    lookup(table(&lang), key).or_else(|| lookup(EN, key)).unwrap_or_else(|| key.to_string())
}

fn trf(core: &Core, key: &str, vars: &[(&str, &str)]) -> String {
    let mut s = tr(core, key);
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// Text of an error for the status lines (account.rs err_text).
fn err_text(core: &Core, e: &anyhow::Error) -> String {
    let key = match code(e).as_str() {
        "login_failed" => "account.err.login",
        "password_too_short" => "account.err.pw_short",
        "password_too_long" => "account.err.pw_long",
        "wrong_password" => "account.err.wrong_password",
        "no_password" => "account.library.escrow.nopw_desc",
        "bad_salt" => "privacy.event.bad_salt",
        "signed_out" => "account.signed_out",
        "device_limit" => "account.err.device_limit",
        "handle_taken" => "account.err.taken",
        "handle_invalid" => "account.err.invalid",
        "handle_reserved" => "account.err.reserved",
        "handle_cooldown" => "account.err.cooldown",
        "too_many_devices" => "account.err.too_many",
        "not_found" => "account.err.code",
        "rate_limited" => "account.err.slow",
        "account_full" => "account.storage.full",
        _ => "account.err.generic",
    };
    tr(core, key)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(d: &[u8]) -> String {
    let mut out = String::with_capacity(d.len().div_ceil(3) * 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub(crate) fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn data_url(path: &std::path::Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let mime = match glyph_core::sync::image_ext(&bytes) {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    };
    Some(format!("data:{mime};base64,{}", b64_encode(&bytes)))
}

fn official(cfg: &Config) -> bool {
    cfg.effective_server_url().trim_end_matches('/').eq_ignore_ascii_case(glyph_core::config::DEFAULT_SERVER_URL)
}

/// The signed-in account; before the backend is up, read from disk.
fn stored(core: &Core) -> Option<Stored> {
    if let Some(b) = core.backend() {
        return b.account();
    }
    let s: Stored = serde_json::from_slice(&std::fs::read(glyph_core::paths::data_dir().join("account.json")).ok()?).ok()?;
    let server = core.st().config.effective_server_url().trim_end_matches('/').to_string();
    (s.server.trim_end_matches('/') == server).then_some(s)
}

// ------------------------------------------------------------------ the Acct

impl Acct {
    fn g(&self) -> MutexGuard<'_, Inner> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn account_value(&self) -> Value {
        match stored(&self.core) {
            Some(s) => {
                let g = self.g();
                json!({ "handle": s.account.handle, "name": null, "avatar": g.own_avatar.clone(), "merge_pending": g.merge_pending })
            }
            None => Value::Null,
        }
    }

    fn emit_account(&self) {
        self.core.emit("account", self.account_value());
    }

    /// The background task: waits for the backend, then syncs, keeps the
    /// library of the signed-in account and the remote playback line.
    async fn run(self: Arc<Self>) {
        while self.core.backend().is_none() {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        self.emit_account();
        self.refresh_avatar();
        let me = self.clone();
        crate::rt::spawn(async move {
            loop {
                me.sync_now().await;
                let delay = {
                    let failures = me.g().sync_failures;
                    if failures > 0 {
                        Duration::from_secs((5u64 << failures.min(6)).min(300))
                    } else if me.core.st().config.storage.data_saver {
                        SYNC_EVERY * 4
                    } else {
                        SYNC_EVERY
                    }
                };
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = me.sync_wake.notified() => {}
                }
            }
        });
        loop {
            self.ensure_library().await;
            self.tick_outputs();
            self.tick_resume();
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    fn alarms(&self) {
        let server = self.core.st().config.effective_server_url().to_string();
        for e in glyph_core::privacy::take_unseen_alarms(&server) {
            let key = format!("privacy.event.{}", e.code);
            self.core.emit("toast", json!({ "text": key, "key": key, "vars": { "detail": e.detail }, "kind": "info" }));
        }
    }

    /// Own picture for the app's account badge.
    fn refresh_avatar(self: &Arc<Self>) {
        let me = self.clone();
        crate::rt::spawn(async move {
            let Some(b) = me.core.backend() else { return };
            let Some(acc) = b.account() else {
                me.g().own_avatar = None;
                return;
            };
            if let Ok(s) = b.profile_settings().await {
                let url = match &s.avatar {
                    Some(sha) => me.image_url(&b, &acc.account.handle, sha).await,
                    None => None,
                };
                let mut g = me.g();
                g.own_avatar = url;
                g.settings = Some(s);
            }
            me.emit_account();
        });
    }

    async fn image_url(&self, b: &Arc<Backend>, handle: &str, sha: &str) -> Option<String> {
        if let Some(u) = self.g().images.get(sha) {
            return Some(u.clone());
        }
        let path = b.profile_image(handle, sha).await.ok()?;
        let url = crate::rt::spawn_blocking(move || data_url(&path)).await.ok().flatten()?;
        let mut g = self.g();
        if g.images.len() > 40 {
            g.images.clear();
        }
        g.images.insert(sha.to_string(), url.clone());
        Some(url)
    }

    // ---- library of the account (account_library.rs) ----

    /// Sign-in writes the guest's settings and a marker, so the guest's data
    /// can be merged into the account afterwards.
    fn prepare_sign_in(&self) -> Result<(), String> {
        let (mut guest, server) = {
            let st = self.core.st();
            (st.config.clone(), st.config.effective_server_url().to_string())
        };
        guest.keys = guest.persisted_keys();
        glyph_core::paths::write_json(&glyph_core::paths::data_dir().join("guest-settings.json"), &guest).map_err(|e| e.to_string())?;
        glyph_core::paths::write_json(&glyph_core::account::guest_merge_marker(), &json!({ "server": server })).map_err(|e| e.to_string())
    }

    async fn ensure_library(self: &Arc<Self>) {
        let Some(b) = self.core.backend() else { return };
        let (merge_pending, signing_out) = {
            let g = self.g();
            (g.merge_pending, g.signing_out)
        };
        let acc = b.account();
        if merge_pending && !signing_out && acc.is_none() {
            self.leave().await;
            return;
        }
        if signing_out || merge_pending {
            return;
        }
        let accounts = glyph_core::paths::data_dir().join("accounts");
        let path = self.core.st().library.storage_path.0.clone();
        let in_accounts = path.as_ref().is_some_and(|p| p.starts_with(&accounts));
        match acc {
            Some(a) if glyph_core::account::guest_merge_pending(&a.server) => self.enter().await,
            None if in_accounts => self.leave().await,
            Some(a) if in_accounts && path.as_ref() != Some(&glyph_core::account::library_path_for(&a)) => {
                if let Err(e) = self.choose_merge(false).await {
                    tracing::warn!(error = %e, "account library switch");
                }
            }
            _ => {}
        }
    }

    /// Signed in: with local data the user chooses (merge or not), else the
    /// account's library is taken at once.
    async fn enter(self: &Arc<Self>) {
        self.g().merge_pending = true;
        let has_data = !glyph_core::sync::snapshot(&self.core.st().library, &[]).items.is_empty();
        if has_data {
            self.emit_account();
            // The merge question opens by itself (Settings > Account).
            self.core.emit("account_merge", json!({ "needed": true }));
        } else if let Err(e) = self.choose_merge(false).await {
            tracing::warn!(error = %e, "account library switch");
        }
    }

    async fn choose_merge(self: &Arc<Self>, merge: bool) -> Result<(), String> {
        let core = &self.core;
        {
            let mut g = self.g();
            if g.signing_out {
                return Ok(());
            }
            g.merge_pending = true;
        }
        let fail = |e: String| {
            core.emit("account", self.account_value());
            e
        };
        let from_guest = {
            let st = core.st();
            let guest = st.library.storage_path.0.as_ref() == Some(&Library::guest_path());
            if guest {
                if let Err(e) = st.library.save() {
                    tracing::warn!(error = %e, "guest library: save failed");
                }
            }
            guest
        };
        if from_guest {
            core.save_playback_now();
        }
        let Some(b) = core.backend() else { return Err(offline()) };
        let Some(acc) = b.account() else { return Ok(()) };
        let path = glyph_core::account::library_path_for(&acc);
        let mut lib = Library::load_from(&path).map_err(|e| fail(e.to_string()))?;
        lib.load_local_playlists().map_err(|e| fail(e.to_string()))?;
        if merge {
            // Kept until a fresh account pull has completed.
            let st = core.st();
            st.library.save_to(&path.with_file_name("pending-guest.json")).map_err(|e| fail(e.to_string()))?;
        } else {
            let source = glyph_core::paths::read_json::<Config>(&path.with_file_name("settings.json")).unwrap_or_default();
            let mut cfg = core.st().config.clone();
            glyph_core::settings_sync::restore_local_preferences(&mut cfg, &source);
            cfg.save().map_err(|e| fail(e.to_string()))?;
            adopt_config(core, cfg, true).await;
        }
        lib.save_to(&path).map_err(|e| fail(e.to_string()))?;
        if let Err(e) = std::fs::remove_file(glyph_core::account::guest_merge_marker()) {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(fail(e.to_string()));
            }
        }
        core.st().library = lib;
        if core.st().current.is_none() {
            let sess = glyph_core::PlaybackSession::load_for(&path);
            core.restore_playback_session(sess);
            crate::discover::reset_stats_pending(core);
            core.emit_queue();
            core.emit_player();
        } else {
            core.playback_changed();
        }
        {
            let mut g = self.g();
            g.merge_pending = false;
            g.last_sync = None;
        }
        b.reset_library_sync();
        core.library_changed();
        self.emit_account();
        self.refresh_avatar();
        self.spawn_sync();
        // Home and the other feeds are for the account now.
        core.emit("account_reset", json!({ "signed_in": true }));
        crate::commands::reload_feeds(&self.core);
        Ok(())
    }

    /// Signed out: the guest's library and settings come back, what plays stops.
    async fn leave(self: &Arc<Self>) {
        let core = &self.core;
        let _ = std::fs::remove_file(glyph_core::account::guest_merge_marker());
        crate::discover::flush_stats(core);
        core.save_playback_now();
        {
            let st = core.st();
            if let Err(e) = st.library.save() {
                tracing::warn!(error = %e, "library: save failed");
            }
            if let Some(path) = st.library.storage_path.0.as_ref().filter(|p| p.starts_with(glyph_core::paths::data_dir().join("accounts"))) {
                let mut cfg = st.config.clone();
                cfg.keys = cfg.persisted_keys();
                if let Err(e) = glyph_core::paths::write_json(&path.with_file_name("settings.json"), &cfg) {
                    tracing::warn!(error = %e, "account settings cache");
                }
            }
        }
        let source = glyph_core::paths::read_json::<Config>(&glyph_core::paths::data_dir().join("guest-settings.json")).unwrap_or_default();
        let mut cfg = core.st().config.clone();
        glyph_core::settings_sync::restore_local_preferences(&mut cfg, &source);
        if let Err(e) = cfg.save() {
            tracing::warn!(error = %e, "guest settings");
        }
        adopt_config(core, cfg, true).await;
        {
            let mut g = self.g();
            g.sync_gen += 1;
            g.signing_out = false;
            g.merge_pending = false;
            g.syncing = false;
            g.sync_failures = 0;
            g.sync_pending = 0;
            g.last_sync = None;
            g.own_avatar = None;
            g.settings = None;
            g.recent_sent.clear();
            g.logins.clear();
            self.reset_remote(&mut g);
        }
        // A jam is left with the account (the host ends it for everyone).
        crate::jam::leave_for_sign_out(core);
        if let Some(p) = &core.player {
            p.stop();
        }
        let guest_session = {
            let mut st = core.st();
            st.queue.clear();
            st.current = None;
            st.resolving = false;
            st.context = None;
            st.playing_from = None;
            st.recent_artist_ctx = None;
            st.listen_counted = false;
            st.restored_pos = None;
            st.restored_duration = None;
            st.resume_at = None;
            st.hold_play = false;
            st.loop_ab = None;
            st.error = None;
            st.preloaded = None;
            st.preload_attempt = None;
            let guest_path = Library::guest_path();
            let mut lib = match Library::load_from(&guest_path) {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(error = %e, "guest library");
                    let mut l = Library::default();
                    l.storage_path.0 = Some(guest_path);
                    l
                }
            };
            if let Err(e) = lib.load_local_playlists() {
                tracing::warn!(error = %e, "local playlists");
            }
            st.library = lib;
            st.library.storage_path.0.as_deref().and_then(glyph_core::PlaybackSession::load_for)
        };
        core.restore_playback_session(guest_session);
        core.emit_library();
        core.emit_queue();
        core.emit_player();
        core.emit("account", Value::Null);
        crate::discover::reset_stats_pending(core);
        // The UI goes back to Home and loads the guest's feeds.
        core.emit("account_reset", json!({ "signed_in": false }));
        crate::commands::reload_feeds(&self.core);
        devices_changed();
    }

    // ---- sync (ui/account.rs) ----

    fn spawn_sync(self: &Arc<Self>) {
        let me = self.clone();
        crate::rt::spawn(async move { me.sync_now().await });
    }

    /// A sync after `delay`, if `check` still holds then.
    fn sync_later(self: &Arc<Self>, delay: Duration, check: Box<dyn Fn(&Acct) -> bool + Send>) {
        let me = self.clone();
        crate::rt::spawn(async move {
            tokio::time::sleep(delay).await;
            if check(&me) {
                me.sync_now().await;
            }
        });
    }

    /// A library change: sync shortly after the last one.
    fn schedule_sync(self: &Arc<Self>) {
        let Some(b) = self.core.backend() else { return };
        if b.account().is_none() {
            return;
        }
        let generation = {
            let mut g = self.g();
            g.sync_gen += 1;
            g.sync_gen
        };
        let pending = {
            let st = self.core.st();
            b.sync_pending(&glyph_core::sync::snapshot(&st.library, &b.sync_wanted_downloads()))
        };
        self.g().sync_pending = pending;
        let me = self.clone();
        crate::rt::spawn(async move {
            tokio::time::sleep(SYNC_DEBOUNCE).await;
            if me.g().sync_gen == generation {
                me.sync_now().await;
            }
        });
    }

    /// One sync round: snapshot here, exchange on the backend, apply here.
    async fn sync_now(self: &Arc<Self>) {
        let core = &self.core;
        let Some(b) = core.backend() else { return };
        let owner = b.account();
        let Some(o) = owner.clone() else { return };
        if glyph_core::account::guest_merge_pending(&o.server) {
            return;
        }
        let generation = {
            let mut g = self.g();
            if g.syncing || g.signing_out || g.merge_pending {
                return;
            }
            g.syncing = true;
            g.sync_gen
        };
        let (snap, settings) = {
            let st = core.st();
            (
                glyph_core::sync::snapshot(&st.library, &b.sync_wanted_downloads()),
                glyph_core::settings_sync::items(&st.config, b.settings_vault().as_ref()),
            )
        };
        let r = b.sync_exchange(snap, settings).await;
        let stale = {
            let mut g = self.g();
            g.syncing = false;
            g.signing_out || g.merge_pending
        };
        if stale || b.account() != owner {
            return;
        }
        match r {
            Ok(round) => {
                self.g().sync_failures = 0;
                let (applied, merge_error, auto_before) = {
                    let mut st = core.st();
                    // Songs of auto-download playlists before the round, to
                    // find the ones another device added.
                    let auto_before: Vec<(String, HashSet<String>)> = st
                        .library
                        .playlists
                        .iter()
                        .filter(|p| p.auto_download)
                        .map(|p| (p.id.clone(), p.tracks.iter().map(track_key).collect()))
                        .collect();
                    let mut applied = b.sync_apply(&mut st.library, &round);
                    let merge_error = match b.sync_merge_guest(&mut st.library) {
                        Ok(changed) => {
                            applied.library_changed |= changed;
                            None
                        }
                        Err(e) => Some(e.to_string()),
                    };
                    (applied, merge_error, auto_before)
                };
                let mut cfg = core.st().config.clone();
                let sa = b.sync_apply_settings(&mut cfg, &round);
                if sa.changed {
                    adopt_config(core, cfg, sa.providers_changed).await;
                }
                self.alarms();
                if applied.library_changed {
                    core.library_changed();
                }
                if !applied.download.is_empty() {
                    download_silent(core, applied.download);
                }
                let removed = applied.remove_downloads.iter().filter(|t| b.remove_download(t)).count();
                if removed > 0 {
                    core.emit_library();
                }
                for (id, before) in auto_before {
                    let added: Vec<Track> = {
                        let st = core.st();
                        st.library
                            .playlist(&id)
                            .filter(|p| p.auto_download)
                            .map(|p| p.tracks.iter().filter(|t| !before.contains(&track_key(t))).cloned().collect())
                            .unwrap_or_default()
                    };
                    if !added.is_empty() {
                        download_silent(core, added);
                    }
                }
                let pending = {
                    let st = core.st();
                    b.sync_pending(&glyph_core::sync::snapshot(&st.library, &b.sync_wanted_downloads()))
                };
                let again = {
                    let mut g = self.g();
                    g.last_sync = Some(now_ms());
                    g.sync_error = merge_error.or_else(|| round.account_full.then(|| tr(core, "account.storage.full")));
                    g.sync_pending = pending;
                    round.bootstrap || g.sync_gen != generation || pending > 0
                };
                if again {
                    self.schedule_sync();
                }
                if round.retry_soon {
                    // A pairing's key or a keyring write is on its way.
                    let (b2, owner) = (b.clone(), owner.clone());
                    self.sync_later(Duration::from_secs(5), Box::new(move |_| b2.account() == owner));
                }
            }
            Err(e) => {
                let failures = {
                    let mut g = self.g();
                    g.sync_failures = g.sync_failures.saturating_add(1);
                    g.sync_failures
                };
                tracing::info!("sync: {e:#}");
                if is_signed_out(&e) {
                    self.leave().await;
                }
                self.g().sync_error = Some(err_text(core, &e));
                let b2 = b.clone();
                self.sync_later(
                    Duration::from_secs((5u64 << failures.min(6)).min(300)),
                    Box::new(move |me| b2.account() == owner && me.g().sync_failures == failures),
                );
            }
        }
    }

    // ---- profile recent songs ----

    fn profile_recent_changed(self: &Arc<Self>) {
        let Some(b) = self.core.backend() else { return };
        let owner = b.account();
        let generation = {
            let mut g = self.g();
            if owner.is_none() || g.merge_pending || g.signing_out {
                return;
            }
            g.recent_gen += 1;
            g.recent_gen
        };
        let me = self.clone();
        crate::rt::spawn(async move {
            tokio::time::sleep(RECENT_DEBOUNCE).await;
            let ok = |g: &Inner| !g.merge_pending && !g.signing_out;
            if !ok(&me.g()) || b.account() != owner {
                return;
            }
            let known = me.g().settings.as_ref().map(|p| p.show_recent);
            let on = match known {
                Some(on) => on,
                None => match b.profile_settings().await {
                    Ok(p) => {
                        let on = p.show_recent;
                        me.g().settings = Some(p);
                        on
                    }
                    Err(_) => return,
                },
            };
            let tracks: Vec<Track> = {
                let mut g = me.g();
                if !on || g.recent_gen != generation || !ok(&g) || b.account() != owner {
                    return;
                }
                let tracks: Vec<Track> = me.core.st().library.recent_tracks().take(RECENT_SHOWN).cloned().collect();
                let keys: Vec<String> = tracks.iter().map(track_key).collect();
                if keys == g.recent_sent {
                    return;
                }
                g.recent_sent = keys;
                tracks
            };
            if let Err(e) = b.profile_push_recent(tracks).await {
                tracing::info!("profile recent: {e:#}");
                me.g().recent_sent.clear();
            }
        });
    }

    // ---- remote playback (account_devices.rs) ----

    fn reset_remote(&self, g: &mut Inner) {
        g.outputs.clear();
        g.target = None;
        g.live = None;
        g.live_since = None;
        g.live_states = false;
        g.live_connecting = false;
        g.live_checked = None;
        g.reported = None;
        g.state_at = None;
        g.seen.clear();
        g.transfer_ack = None;
        g.transfer_ack_at = None;
        g.transfer_to = None;
        g.error = None;
        g.error_at = None;
        g.playing_hint = None;
        g.failures = 0;
        g.missing_since = None;
        g.checked = None;
        g.received = None;
        g.busy = false;
        g.commands.clear();
        g.resume = None;
    }

    fn drop_live(g: &mut Inner) {
        g.live = None;
        g.live_since = None;
        g.live_states = false;
        g.reported = None;
    }

    fn output_position(&self, g: &Inner, o: &Output) -> Duration {
        let elapsed = if o.state.playing { g.state_at.map(|at| at.elapsed().min(POSITION_RUN_ON)).unwrap_or_default() } else { Duration::ZERO };
        (Duration::from_millis(o.state.position_ms) + elapsed).min(o.state.duration_ms.map(Duration::from_millis).unwrap_or(Duration::MAX))
    }

    /// This app's state as the other devices see it.
    fn own_state(&self, controlling: bool) -> PlayState {
        let ps = self.core.player_state();
        let (current, resolving, volume) = {
            let st = self.core.st();
            (st.current.clone(), st.resolving, st.config.player.volume)
        };
        let playing = ps.as_ref().is_some_and(|s| s.status == Status::Playing);
        PlayState {
            acknowledgements: true,
            track: if controlling { None } else { current },
            // A song being found counts as playing.
            playing: !controlling && (playing || (resolving && !crate::jam::hold_play())),
            position_ms: if controlling { 0 } else { ps.as_ref().map_or(0, |s| s.position.as_millis() as u64) },
            duration_ms: ps.as_ref().and_then(|s| s.duration).map(|d| d.as_millis() as u64),
            volume: ps.as_ref().map_or(volume, |s| s.volume),
            available: !JAM_ACTIVE.load(Ordering::Relaxed) && !controlling,
        }
    }

    fn ack(self: &Arc<Self>, id: String, error: Option<String>) {
        let Some(b) = self.core.backend() else { return };
        crate::rt::spawn(async move {
            if let Err(e) = b.playback_ack(Receipt { id, done: true, error }).await {
                tracing::info!("playback acknowledgement: {e:#}");
            }
        });
    }

    fn finish_transfer(self: &Arc<Self>, error: Option<String>) {
        let id = {
            let mut g = self.g();
            g.transfer_ack_at = None;
            let id = g.transfer_ack.take();
            if let Some(id) = &id {
                g.seen.insert(id.clone(), error.clone());
            }
            id
        };
        if let Some(id) = id {
            self.ack(id, error);
        }
    }

    fn connect_live(self: &Arc<Self>) {
        {
            let mut g = self.g();
            if g.live.is_some() || g.live_connecting || g.live_checked.is_some_and(|at| at.elapsed() < Duration::from_secs(15)) {
                return;
            }
            g.live_connecting = true;
            g.live_checked = Some(Instant::now());
        }
        let Some(b) = self.core.backend() else { return };
        let owner = b.account();
        let me = self.clone();
        crate::rt::spawn(async move {
            let result = b.playback_live().await;
            let mut g = me.g();
            g.live_connecting = false;
            if b.account() != owner {
                return;
            }
            if let Ok(live) = result {
                g.live = Some(live);
                g.live_since = Some(Instant::now());
                g.live_states = false;
                // The new line hears this app's state first thing; commands
                // sent while it was down are fetched now.
                g.reported = None;
                g.checked = None;
            }
        });
    }

    fn control(self: &Arc<Self>, command: Command) -> bool {
        let mut g = self.g();
        let Some(target) = g.target.clone() else { return false };
        // The device reports back with its next poll: show the wish now.
        match command {
            Command::Play => g.playing_hint = Some((true, Instant::now())),
            Command::Pause | Command::Stop => g.playing_hint = Some((false, Instant::now())),
            _ => {}
        }
        if !g.signing_out {
            if let Some(b) = self.core.backend() {
                let rx = b.playback_send(PlaySend { target, command, request_id: None });
                g.commands.push(rx);
            }
        }
        drop(g);
        self.core.emit_player();
        true
    }

    fn select_output(self: &Arc<Self>, id: String) {
        let track = {
            let mut g = self.g();
            if !g.outputs.iter().any(|o| o.device.id == id && !o.device.current && o.online && o.state.available) {
                return;
            }
            g.target = Some(id.clone());
            g.error = None;
            g.error_at = None;
            g.missing_since = None;
            g.playing_hint = None;
            g.outputs.iter().find(|o| o.device.id == id).and_then(|o| o.state.track.clone())
        };
        crate::discover::flush_stats(&self.core);
        if let Some(p) = &self.core.player {
            p.pause();
        }
        // MPD and a network device stop: one output at a time.
        crate::jam::release_outputs(&self.core);
        self.core.st().current = track;
        self.core.emit_player();
        devices_changed();
    }

    fn connect_output(self: &Arc<Self>, id: String) {
        let (theirs, busy) = {
            let g = self.g();
            (
                g.outputs.iter().find(|o| o.device.id == id).is_some_and(|o| o.state.playing && o.state.track.is_some()),
                g.transfer_to.is_some() || g.target.as_ref() == Some(&id),
            )
        };
        if busy {
            return;
        }
        let playing = self.core.player_state().is_some_and(|s| s.status == Status::Playing);
        if self.core.st().current.is_none() || (theirs && !playing) {
            self.select_output(id);
        } else {
            self.transfer_output(id);
        }
    }

    /// Moves this queue and position to a signed-in app; stops here once accepted.
    fn transfer_output(self: &Arc<Self>, id: String) {
        let Some(b) = self.core.backend() else { return };
        let owner = b.account();
        let Some(track) = self.core.st().current.clone() else {
            self.select_output(id);
            return;
        };
        // Controlling another device: its song moves on, and that device
        // stops afterwards.
        let previous = self.g().target.clone();
        let mut tracks = vec![track];
        if previous.is_none() {
            tracks.extend(self.core.st().queue.upcoming().into_iter().take(99).cloned());
        }
        let position_ms = match (&previous, self.remote_position()) {
            (Some(_), Some(p)) => p.as_millis() as u64,
            _ => self.core.player_state().map_or(0, |s| s.position.as_millis() as u64),
        };
        let playing = match (&previous, self.g().outputs.iter().find(|o| Some(&o.device.id) == previous.as_ref())) {
            (Some(_), Some(o)) => o.state.playing,
            _ => self.core.player_state().is_some_and(|s| s.status == Status::Playing),
        };
        let send = PlaySend {
            request_id: None,
            target: id.clone(),
            command: Command::Transfer { tracks, index: 0, position_ms, playing },
        };
        {
            let mut g = self.g();
            g.transfer_to = Some(id.clone());
            g.error = None;
            g.error_at = None;
        }
        devices_changed();
        let me = self.clone();
        crate::rt::spawn(async move {
            let r = b.playback_command(send).await;
            if b.account() != owner {
                return;
            }
            me.g().transfer_to = None;
            match r {
                Ok(()) => {
                    if let Some(old) = previous.filter(|old| *old != id) {
                        let rx = b.playback_send(PlaySend { target: old, command: Command::Stop, request_id: None });
                        me.g().commands.push(rx);
                    }
                    me.select_output(id);
                }
                Err(e) => {
                    let mut g = me.g();
                    g.error = Some(e.to_string());
                    g.error_at = Some(id);
                }
            }
            devices_changed();
        });
    }

    fn remote_position(&self) -> Option<Duration> {
        let g = self.g();
        let o = g.outputs.iter().find(|o| Some(&o.device.id) == g.target.as_ref())?;
        Some(self.output_position(&g, o))
    }

    /// Back to this computer: the controlled device stops, its song goes on here.
    fn use_local(self: &Arc<Self>) -> bool {
        if self.g().target.is_none() {
            return false;
        }
        let ms = self.remote_position().map_or(0, |p| p.as_millis() as u64);
        let playing = {
            let g = self.g();
            g.outputs.iter().find(|o| Some(&o.device.id) == g.target.as_ref()).is_some_and(|o| o.state.playing)
        };
        self.control(Command::Stop);
        self.g().target = None;
        let track = self.core.st().current.take();
        if let Some(t) = track {
            self.g().resume = Some((ms, playing, Instant::now()));
            self.core.start_track(t);
        } else {
            self.core.emit_player();
        }
        devices_changed();
        true
    }

    /// A moved song starts at the position it had; paused when it was paused.
    fn tick_resume(self: &Arc<Self>) {
        let Some((ms, playing, since)) = self.g().resume else { return };
        let (current, resolving) = {
            let st = self.core.st();
            (st.current.clone(), st.resolving)
        };
        if current.is_none() || since.elapsed() > Duration::from_secs(60) {
            self.g().resume = None;
            return;
        }
        if resolving {
            return;
        }
        let Some(ps) = self.core.player_state() else { return };
        if !matches!(ps.status, Status::Playing | Status::Paused) || !same_track(current.as_ref(), ps.track.as_ref()) {
            return;
        }
        self.g().resume = None;
        if ms > 0 {
            self.core.seek(ms as f64 / 1000.0);
        }
        if !playing {
            if let Some(p) = &self.core.player {
                p.pause();
            }
        }
        self.core.emit_player();
    }

    fn apply_command(self: &Arc<Self>, command: Command) {
        let core = &self.core;
        self.g().target = None;
        let is_playing = || core.player_state().is_some_and(|s| s.status == Status::Playing);
        match command {
            Command::Transfer { tracks, index, position_ms, playing } => {
                // The song plays on this computer, not on MPD / a device.
                crate::jam::use_local_output(core);
                self.g().resume = Some((position_ms, playing, Instant::now()));
                core.play_list(tracks, index, None);
            }
            // While a song is being found, set the wish directly.
            Command::Play => {
                if core.st().resolving {
                    crate::jam::set_hold(false);
                } else if !is_playing() {
                    core.toggle_play();
                }
            }
            Command::Pause => {
                if core.st().resolving {
                    crate::jam::set_hold(true);
                } else if is_playing() {
                    core.toggle_play();
                }
            }
            Command::Stop => {
                if !crate::jam::remote_stop() {
                    if let Some(p) = &core.player {
                        p.stop();
                    }
                }
                {
                    let mut st = core.st();
                    st.current = None;
                    st.restored_pos = None;
                    st.restored_duration = None;
                    st.resume_at = None;
                    st.hold_play = false;
                }
                core.playback_changed();
                core.emit_player();
            }
            Command::Next => core.next(false),
            Command::Previous => core.previous(),
            Command::Seek { position_ms } => core.seek(position_ms as f64 / 1000.0),
            Command::Volume { volume } => core.set_volume(volume),
        }
    }

    /// Something came over the live line.
    fn live_event(&self, g: &mut Inner, event: Live) -> bool {
        match event {
            Live::State { device, state } => {
                g.live_states = true;
                let Some(o) = g.outputs.iter_mut().find(|o| o.device.id == device) else {
                    // A device not in the list yet: the poll brings its name.
                    g.checked = None;
                    return false;
                };
                if o.device.current {
                    return false;
                }
                o.online = true;
                o.state = state;
                if g.target.as_ref() == Some(&device) {
                    let (playing, track) = (o.state.playing, o.state.track.clone());
                    g.missing_since = None;
                    g.state_at = Some(Instant::now());
                    self.core.st().current = track;
                    if g.playing_hint.is_some_and(|(p, _)| p == playing) {
                        g.playing_hint = None;
                    }
                }
                true
            }
            // A command waits, or a device left the line: ask now.
            Live::Wake | Live::Gone { .. } | Live::Report { .. } => {
                g.checked = None;
                false
            }
        }
    }

    /// Twice a second: the live line, own state reports and the poll.
    fn tick_outputs(self: &Arc<Self>) {
        let core = &self.core;
        let Some(b) = core.backend() else { return };
        // A moved song counts as received once it runs here.
        let ack_at = self.g().transfer_ack_at;
        if let Some(at) = ack_at {
            let (current, resolving) = {
                let st = core.st();
                (st.current.clone(), st.resolving)
            };
            let started = !resolving
                && self.g().resume.is_none()
                && core.player_state().is_some_and(|p| matches!(p.status, Status::Playing | Status::Paused) && same_track(current.as_ref(), p.track.as_ref()));
            if started || at.elapsed() > TRANSFER_ACK_MAX {
                self.finish_transfer(current.is_none().then(|| "Could not start playback on target device".to_string()));
            }
        }
        // Commands sent to the controlled device: errors show in its row.
        let errors: Vec<String> = {
            let mut g = self.g();
            let mut errors = Vec::new();
            g.commands.retain(|rx| match rx.try_recv() {
                Ok(Err(e)) => {
                    errors.push(e.to_string());
                    false
                }
                Ok(Ok(())) | Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
                Err(std::sync::mpsc::TryRecvError::Empty) => true,
            });
            if !errors.is_empty() {
                // Not carried out: the device's own report counts again.
                g.playing_hint = None;
                g.error = errors.last().cloned();
                g.error_at = g.target.clone();
            }
            errors
        };
        if !errors.is_empty() {
            core.emit_player();
            devices_changed();
        }
        let (signed, blocked) = {
            let g = self.g();
            (b.account().is_some(), g.signing_out || g.merge_pending)
        };
        if !signed || blocked {
            let mut g = self.g();
            let had = !g.outputs.is_empty() || g.target.is_some();
            g.outputs.clear();
            g.target = None;
            Self::drop_live(&mut g);
            drop(g);
            if had {
                devices_changed();
            }
            return;
        }
        self.connect_live();
        // Events from the live line.
        let mut list_changed = false;
        {
            let mut g = self.g();
            let mut events = Vec::new();
            let mut closed = false;
            if let Some(live) = g.live.as_mut() {
                loop {
                    match live.events.try_recv() {
                        Ok(ev) => events.push(ev),
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                            closed = true;
                            break;
                        }
                    }
                }
            }
            if closed {
                Self::drop_live(&mut g);
            }
            for ev in events {
                list_changed |= self.live_event(&mut g, ev);
            }
        }
        if list_changed {
            core.emit_player();
            devices_changed();
        }
        self.report_state();
        // Poll interval: without the live line nothing wakes this app for a
        // command, so it asks more often.
        let playing = core.player_state().is_some_and(|s| s.status == Status::Playing);
        let (skip, owner_state) = {
            let g = self.g();
            let controlling = g.target.is_some();
            let has_peers = g.outputs.iter().any(|o| !o.device.current);
            let interval = if g.transfer_to.is_some() {
                2
            } else if DEVICES_OPEN.load(Ordering::Relaxed) || controlling {
                2
            } else if g.live.is_some() && (g.live_states || !has_peers) {
                LIVE_POLL
            } else if !has_peers && g.received.is_some() {
                30
            } else if playing {
                5
            } else if g.live.is_none() {
                6
            } else {
                15
            };
            (g.busy || g.checked.is_some_and(|at| at.elapsed() < Duration::from_secs(interval)), controlling)
        };
        if skip {
            return;
        }
        {
            let mut g = self.g();
            g.checked = Some(Instant::now());
            g.busy = true;
        }
        let owner = b.account();
        let state = self.own_state(owner_state);
        let me = self.clone();
        crate::rt::spawn(async move {
            let result = b.playback_poll(state).await;
            me.g().busy = false;
            if b.account() != owner || me.g().signing_out {
                return;
            }
            me.after_poll(result);
        });
    }

    /// Own state over the live line when it changed.
    fn report_state(&self) {
        let controlling = self.g().target.is_some();
        let state = self.own_state(controlling);
        let mut g = self.g();
        if g.live.is_none() || g.signing_out {
            return;
        }
        let changed = g.reported.as_ref().is_none_or(|(old, at)| old.differs(&state, at.elapsed().as_millis() as u64));
        if !changed {
            return;
        }
        // Full (a report still on its way): the next tick tries again.
        if g.live.as_ref().is_some_and(|l| l.reports.try_send(state.clone()).is_ok()) {
            g.reported = Some((state, Instant::now()));
        }
    }

    fn after_poll(self: &Arc<Self>, result: anyhow::Result<glyph_protocol::playback::Poll>) {
        let core = &self.core;
        let mut run: Vec<Command> = Vec::new();
        let mut deliveries = Vec::new();
        let stale;
        {
            let mut g = self.g();
            match result {
                Ok(poll) => {
                    // With the live line up since the last poll, a waiting
                    // command woke this app at once: it is fresh.
                    let live_through = g.live_since.zip(g.received).is_some_and(|(since, last)| since <= last);
                    stale = !live_through && g.received.is_none_or(|at| at.elapsed() > STALE_POLL);
                    if g.failures >= POLL_FAILURES {
                        g.error = None;
                    }
                    g.failures = 0;
                    let mut devices = poll.devices;
                    // The controlled device missed a report: keep what it said last.
                    if let Some(id) = g.target.clone() {
                        if let (Some(new), Some(old)) = (devices.iter_mut().find(|o| o.device.id == id && !o.online), g.outputs.iter().find(|o| o.device.id == id)) {
                            new.state = old.state.clone();
                        }
                    }
                    g.outputs = devices;
                    g.received = Some(Instant::now());
                    g.state_at = Some(Instant::now());
                    if let Some(id) = g.target.clone() {
                        let found = g.outputs.iter().find(|o| o.device.id == id).map(|o| (o.online, o.state.playing, o.state.track.clone()));
                        match found {
                            Some((true, playing, track)) => {
                                g.missing_since = None;
                                core.st().current = track;
                                if g.playing_hint.is_some_and(|(p, _)| p == playing) {
                                    g.playing_hint = None;
                                }
                            }
                            found => {
                                let since = *g.missing_since.get_or_insert_with(Instant::now);
                                if found.is_none() || since.elapsed() > OUTPUT_GRACE {
                                    g.target = None;
                                    g.missing_since = None;
                                    g.playing_hint = None;
                                    core.st().current = None;
                                    g.error = Some(tr(core, "connect.account.offline"));
                                    g.error_at = None;
                                }
                            }
                        }
                    }
                    if !JAM_ACTIVE.load(Ordering::Relaxed) && !stale {
                        run = poll.commands;
                    }
                    deliveries = poll.deliveries;
                }
                Err(e) => {
                    // Short outages keep the list; only repeated failures clear it.
                    stale = false;
                    g.failures += 1;
                    if g.failures >= POLL_FAILURES {
                        g.error = Some(e.to_string());
                        g.error_at = None;
                        g.outputs.clear();
                    }
                }
            }
        }
        for command in run {
            self.apply_command(command);
        }
        for d in deliveries {
            let (is_ack, seen) = {
                let g = self.g();
                (g.transfer_ack.as_ref() == Some(&d.id), g.seen.get(&d.id).cloned())
            };
            if is_ack {
                continue;
            }
            if let Some(error) = seen {
                self.ack(d.id, error);
                continue;
            }
            let error = if stale {
                Some("Command expired".to_string())
            } else if JAM_ACTIVE.load(Ordering::Relaxed) || self.g().target.is_some() {
                Some("Device is busy".to_string())
            } else {
                None
            };
            let transfer = matches!(d.command, Command::Transfer { .. });
            if error.is_none() {
                self.apply_command(d.command);
            }
            if transfer && error.is_none() {
                // A newer transfer replaces one still starting: that one is done.
                let old = {
                    let mut g = self.g();
                    let old = g.transfer_ack.replace(d.id);
                    if let Some(o) = &old {
                        g.seen.insert(o.clone(), None);
                    }
                    g.transfer_ack_at = Some(Instant::now());
                    old
                };
                if let Some(o) = old {
                    self.ack(o, None);
                }
            } else {
                self.g().seen.insert(d.id.clone(), error.clone());
                self.ack(d.id, error);
            }
        }
        {
            let mut g = self.g();
            while g.seen.len() > 256 {
                let Some(k) = g.seen.keys().next().cloned() else { break };
                g.seen.remove(&k);
            }
        }
        core.emit_player();
        devices_changed();
    }
}

fn same_track(a: Option<&Track>, b: Option<&Track>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => track_key(a) == track_key(b) || glyph_core::dedupe::same_song(a, b),
        _ => false,
    }
}

/// Settings that came from another device (or the account's cache): apply
/// them, save them quietly and rebuild the backend if keys / sources changed.
async fn adopt_config(core: &Arc<Core>, cfg: Config, providers_changed: bool) {
    core.st().config = cfg.clone();
    crate::settings::apply_to_player(core, &cfg);
    core.config_changed();
    if let Some(b) = core.backend() {
        if let Err(e) = b.set_config(cfg).await {
            tracing::warn!(error = %e, "adopt config");
        }
    }
    if providers_changed {
        core.start_backend();
    }
}

/// Songs sync asks for offline use: fetched without a toast.
fn download_silent(core: &Arc<Core>, tracks: Vec<Track>) {
    let Some(b) = core.backend() else { return };
    let have: HashSet<String> = b.downloaded_tracks().iter().map(track_key).collect();
    let mut seen = HashSet::new();
    let todo: Vec<Track> = tracks.into_iter().filter(|t| !have.contains(&track_key(t)) && seen.insert(track_key(t))).collect();
    if todo.is_empty() {
        return;
    }
    let core = core.clone();
    crate::rt::spawn(async move {
        for t in &todo {
            core.emit("download", json!({ "key": track_key(t), "progress": null, "done": false }));
        }
        let mut pending = futures::stream::iter(todo)
            .map(|t| {
                let b = b.clone();
                async move {
                    let r = b.download(&t).await;
                    (t, r)
                }
            })
            .buffer_unordered(10);
        while let Some((t, r)) = pending.next().await {
            if let Err(e) = r {
                tracing::warn!(title = %t.title, "download failed: {e:#}");
            }
            core.emit("download", json!({ "key": track_key(&t), "progress": null, "done": true }));
        }
        core.emit_library();
    });
}

fn need_backend(core: &Core) -> Res<Arc<Backend>> {
    core.backend().ok_or_else(offline)
}

// ------------------------------------------------------------------ commands: account

pub async fn account_status(core: C<'_>) -> Res<Value> {
    let acc = need_acct(core)?;
    let core = core.clone();
    let cfg = core.st().config.clone();
    let official = official(&cfg);
    let Some(b) = core.backend() else {
        return Ok(json!({
            "available": false, "official": official, "signed_in": false, "handle": "", "has_password": false,
            "device_limit": 5, "crypto": "guest", "merge_pending": false, "syncing": false, "sync_error": null,
            "last_sync": null, "sync_pending": 0, "devices": null, "devices_error": null,
        }));
    };
    let cached = acc.g().available.filter(|(ok, at)| *ok && at.elapsed() < Duration::from_secs(20)).map(|(ok, _)| ok);
    let available = match cached {
        Some(v) => v,
        None => {
            let v = tokio::time::timeout(Duration::from_secs(6), b.accounts_available()).await.unwrap_or(false);
            acc.g().available = Some((v, Instant::now()));
            v
        }
    };
    let signed = b.account();
    let mut devices: Option<Value> = None;
    let mut devices_error: Option<String> = None;
    let mut limit = signed.as_ref().map_or(5, |s| s.account.device_limit);
    if signed.is_some() {
        match tokio::time::timeout(Duration::from_secs(6), b.account_devices()).await {
            Ok(Ok(d)) => {
                limit = d.device_limit;
                devices = Some(Value::Array(
                    d.devices
                        .iter()
                        .map(|x| json!({ "id": x.id, "name": x.name, "platform": x.platform, "last_seen": x.last_seen, "current": x.current, "approximate_location": x.approximate_location }))
                        .collect(),
                ));
            }
            Ok(Err(e)) => devices_error = Some(err_text(&core, &e)),
            Err(_) => devices_error = Some(tr(&core, "account.err.generic")),
        }
    }
    let crypto = match b.library_crypto() {
        glyph_core::privacy::Library::Guest => "guest",
        glyph_core::privacy::Library::Starting => "starting",
        glyph_core::privacy::Library::Escrow => "escrow",
        glyph_core::privacy::Library::Encrypted => "encrypted",
        glyph_core::privacy::Library::Locked => "locked",
    };
    // Older servers have no size limit (and no answer): null.
    let storage = match signed.is_some() {
        true => tokio::time::timeout(Duration::from_secs(6), b.account_storage()).await.ok().and_then(Result::ok),
        false => None,
    };
    let g = acc.g();
    Ok(json!({
        "available": available,
        "official": official,
        "signed_in": signed.is_some(),
        "handle": signed.as_ref().map(|s| s.account.handle.clone()).unwrap_or_default(),
        "has_password": signed.as_ref().is_some_and(|s| s.account.has_password),
        "device_limit": limit,
        "crypto": crypto,
        "merge_pending": g.merge_pending,
        "syncing": g.syncing,
        "sync_error": g.sync_error,
        "last_sync": g.last_sync,
        "sync_pending": g.sync_pending,
        "devices": devices,
        "devices_error": devices_error,
        "storage": storage,
    }))
}

pub async fn account_login_start(core: C<'_>) -> Res<Value> {
    let acc = need_acct(core)?;
    let b = need_backend(&core)?;
    acc.prepare_sign_in()?;
    let start = b.account_login_start().await.map_err(err)?;
    let out = json!({ "login_id": start.login_id, "user_code": start.user_code, "url": start.url, "expires_in_s": start.expires_in_s });
    acc.g().logins.insert(start.login_id.clone(), start);
    Ok(out)
}

pub async fn account_login_poll(core: C<'_>, login_id: String) -> Res<Value> {
    let acc = need_acct(core)?;
    let b = need_backend(&core)?;
    let start = acc.g().logins.get(&login_id).cloned().ok_or_else(|| "not_found".to_string())?;
    match b.account_login_poll(&start).await.map_err(err)? {
        LoginPollResponse::Pending => Ok(json!({ "state": "pending" })),
        LoginPollResponse::DeviceLimit => Ok(json!({ "state": "device_limit" })),
        LoginPollResponse::Expired => {
            acc.g().logins.remove(&login_id);
            Ok(json!({ "state": "expired" }))
        }
        LoginPollResponse::Done { session } => {
            acc.g().logins.remove(&login_id);
            acc.enter().await;
            acc.alarms();
            acc.emit_account();
            acc.refresh_avatar();
            Ok(json!({ "state": "done", "handle": session.account.handle }))
        }
    }
}

pub async fn account_password_submit(core: C<'_>, handle: String, password: String, register: bool) -> Res<Value> {
    let acc = need_acct(core)?;
    let b = need_backend(&core)?;
    acc.prepare_sign_in()?;
    let handle = handle.trim().to_string();
    let r = if register {
        b.account_password_signup(&handle, &password).await.map(Some)
    } else {
        b.account_password_login(&handle, &password).await.map(|_| None)
    };
    match r {
        Ok(recovery) => {
            let handle = b.account().map(|a| a.account.handle).unwrap_or(handle);
            acc.enter().await;
            acc.alarms();
            acc.emit_account();
            acc.refresh_avatar();
            Ok(json!({ "handle": handle, "recovery_code": recovery }))
        }
        Err(e) => {
            acc.alarms();
            Err(err(e))
        }
    }
}

pub async fn account_unlock_keys(core: C<'_>, password: String) -> Res {
    let acc = need_acct(core)?;
    let b = need_backend(&core)?;
    let r = b.account_unlock_keys(&password).await;
    acc.alarms();
    r.map_err(err)?;
    // The next round uses the key.
    acc.spawn_sync();
    Ok(())
}

pub async fn account_change_password(core: C<'_>, password: String) -> Res {
    let acc = need_acct(core)?;
    let b = need_backend(&core)?;
    b.account_set_password(&password).await.map_err(err)?;
    // Wraps the library's key with the new password right away.
    acc.spawn_sync();
    Ok(())
}

pub async fn account_link_passkey(core: C<'_>) -> Res<Value> {
    let b = need_backend(&core)?;
    let r = b.account_link_passkey().await.map_err(err)?;
    Ok(json!({ "url": r.url }))
}

pub async fn account_rename(core: C<'_>, handle: String) -> Res<Value> {
    let acc = need_acct(core)?;
    let b = need_backend(&core)?;
    let a = b.account_update(AccountPatch { handle: Some(handle.trim().to_string()), ..Default::default() }).await.map_err(err)?;
    acc.emit_account();
    Ok(json!({ "handle": a.handle }))
}

pub async fn account_approve(core: C<'_>, code: String) -> Res<Value> {
    let b = need_backend(&core)?;
    let r = b.account_approve(code.trim()).await.map_err(err)?;
    Ok(json!({ "device_name": r.device_name, "device_limit_reached": r.device_limit_reached }))
}

pub async fn account_recovery(core: C<'_>) -> Res<String> {
    let b = need_backend(&core)?;
    b.account_new_recovery_code().await.map_err(err)
}

pub async fn account_remove_device(core: C<'_>, id: String) -> Res {
    let b = need_backend(&core)?;
    b.account_remove_device(&id).await.map_err(err)
}

async fn sign_out_with(core: Arc<Core>, acc: Arc<Acct>, all: bool) -> Res {
    let b = need_backend(&core)?;
    {
        let mut g = acc.g();
        if g.signing_out {
            return Ok(());
        }
        g.signing_out = true;
    }
    if let Err(e) = core.st().library.save() {
        tracing::warn!(error = %e, "library: save failed");
    }
    let res = if all { b.account_sign_out_all().await } else { b.account_sign_out().await };
    match res {
        Ok(()) => {
            acc.leave().await;
            Ok(())
        }
        Err(e) => {
            acc.g().signing_out = false;
            if b.account().is_none() {
                acc.leave().await;
            }
            Err(err(e))
        }
    }
}

pub async fn account_sign_out(core: C<'_>) -> Res {
    let acc = need_acct(core)?;
    sign_out_with(core.clone(), acc, false).await
}

pub async fn account_sign_out_all(core: C<'_>) -> Res {
    let acc = need_acct(core)?;
    sign_out_with(core.clone(), acc, true).await
}

pub async fn account_delete(core: C<'_>, handle: String) -> Res {
    let acc = need_acct(core)?;
    let core = core.clone();
    let b = need_backend(&core)?;
    let typed = handle.trim().trim_start_matches('@').to_lowercase();
    let Some(me) = b.account() else { return Err("signed_out".into()) };
    if typed != me.account.handle {
        return Err("mismatch".into());
    }
    b.account_delete(&typed).await.map_err(err)?;
    acc.leave().await;
    Ok(())
}

pub async fn account_sync_now(core: C<'_>) -> Res {
    let acc = need_acct(core)?;
    acc.sync_now().await;
    Ok(())
}

pub async fn account_merge(core: C<'_>, yes: bool) -> Res {
    let acc = need_acct(core)?;
    acc.choose_merge(yes).await
}

// ------------------------------------------------------------------ commands: profile

/// Songs and picture links of a profile as the UI shows it.
async fn profile_value(core: &Arc<Core>, acc: &Arc<Acct>, handle: &str) -> Res<Value> {
    let b = need_backend(core)?;
    let h = handle.trim().trim_start_matches('@').to_ascii_lowercase();
    let p = b.profile(&h).await.map_err(err)?;
    let avatar = match &p.avatar {
        Some(sha) => acc.image_url(&b, &p.handle, sha).await,
        None => None,
    };
    let banner = match &p.banner {
        Some(sha) => acc.image_url(&b, &p.handle, sha).await,
        None => None,
    };
    let mut playlists = Vec::new();
    for pl in &p.playlists {
        // Own playlists come from the library; others show what the server sent.
        let local = p.own.then(|| core.st().library.playlist(&pl.id).map(|l| l.tracks.clone())).flatten();
        let mut tracks = local.unwrap_or_else(|| pl.preview.clone());
        if !p.own && (tracks.len() as u32) < pl.songs && playlists.len() < 8 {
            if let Ok(full) = b.profile_playlist(&p.handle, &pl.id).await {
                tracks = full.tracks;
            }
        }
        let cover = match &pl.cover {
            Some(sha) => acc.image_url(&b, &p.handle, sha).await,
            None => None,
        };
        playlists.push(json!({ "id": pl.id, "name": pl.name, "tracks": tracks, "cover": cover }));
    }
    let sections = if !p.sections.is_empty() {
        glyph_core::config::normalize_profile_sections(&p.sections)
    } else if p.own {
        glyph_core::config::normalize_profile_sections(&core.st().config.sharing.profile_sections)
    } else {
        glyph_core::config::default_profile_sections()
    };
    Ok(json!({
        "handle": p.handle,
        "name": p.handle,
        "avatar": avatar,
        "banner": banner,
        "bio": null,
        "followers": 0,
        "following": 0,
        "playlists": playlists,
        "top_artists": [],
        "top_tracks": [],
        "is_me": p.own,
        "follows": false,
        "private": p.private,
        "recent": p.recent,
        "sections": sections,
    }))
}

pub async fn profile(core: C<'_>, handle: String) -> Res<Value> {
    let acc = need_acct(core)?;
    profile_value(core, &acc, &handle).await
}

pub async fn profile_playlist(core: C<'_>, handle: String, id: String) -> Res<Vec<Track>> {
    let b = need_backend(&core)?;
    let p = b.profile_playlist(handle.trim().trim_start_matches('@'), &id).await.map_err(err)?;
    Ok(p.tracks)
}

/// The server has no follow for people (the GPUI app has none): nothing to do.
pub fn profile_follow(handle: String, on: bool) {
    let _ = (handle, on);
}

fn effective_sections(core: &Core, remote: &[String]) -> Vec<String> {
    if !remote.is_empty() {
        glyph_core::config::normalize_profile_sections(remote)
    } else {
        glyph_core::config::normalize_profile_sections(&core.st().config.sharing.profile_sections)
    }
}

async fn settings_value(core: &Arc<Core>, acc: &Arc<Acct>, s: &ProfileSettings) -> Value {
    let (mut avatar_url, mut banner_url) = (None, None);
    if let (Some(b), Some(me)) = (core.backend(), stored(core)) {
        if let Some(sha) = &s.avatar {
            avatar_url = acc.image_url(&b, &me.account.handle, sha).await;
        }
        if let Some(sha) = &s.banner {
            banner_url = acc.image_url(&b, &me.account.handle, sha).await;
        }
    }
    json!({
        "avatar": s.avatar, "banner": s.banner, "public": s.public, "show_recent": s.show_recent,
        "playlists": s.playlists, "sections": effective_sections(core, &s.sections),
        "avatar_url": avatar_url, "banner_url": banner_url,
    })
}

pub async fn profile_settings(core: C<'_>) -> Res<Value> {
    let acc = need_acct(core)?;
    let core = core.clone();
    let b = need_backend(&core)?;
    let s = b.profile_settings().await.map_err(err)?;
    acc.g().settings = Some(s.clone());
    Ok(settings_value(&core, &acc, &s).await)
}

/// Sends a change and keeps the server's answer.
async fn patch_profile(core: &Arc<Core>, acc: &Arc<Acct>, mut patch: ProfilePatch) -> Res {
    let b = need_backend(core)?;
    let recent_on = patch.show_recent == Some(true);
    if let Some(sec) = patch.sections.take() {
        let norm = glyph_core::config::normalize_profile_sections(&sec);
        let cfg = {
            let mut st = core.st();
            st.config.sharing.profile_sections = norm.clone();
            st.config.clone()
        };
        adopt_config(core, cfg, false).await;
        patch.sections = Some(norm);
    }
    // Shown at once; the server's answer replaces it.
    if let Some(s) = acc.g().settings.as_mut() {
        if let Some(v) = patch.public {
            s.public = v;
        }
        if let Some(v) = patch.show_recent {
            s.show_recent = v;
        }
        if let Some(v) = patch.playlists.clone() {
            s.playlists = v;
        }
        if let Some(v) = patch.sections.clone() {
            s.sections = v;
        }
    }
    match b.profile_update(patch).await {
        Ok(p) => {
            acc.g().settings = Some(p);
            if recent_on {
                acc.g().recent_sent.clear();
                acc.profile_recent_changed();
            }
            Ok(())
        }
        Err(e) => {
            acc.g().settings = None;
            Err(err(e))
        }
    }
}

pub async fn profile_update(core: C<'_>, public: Option<bool>, show_recent: Option<bool>, playlists: Option<Vec<String>>, sections: Option<Vec<String>>) -> Res {
    let acc = need_acct(core)?;
    patch_profile(core, &acc, ProfilePatch { public, show_recent, playlists, sections }).await
}

async fn current_settings(core: &Arc<Core>, acc: &Arc<Acct>) -> Res<ProfileSettings> {
    if let Some(s) = acc.g().settings.clone() {
        return Ok(s);
    }
    let s = need_backend(core)?.profile_settings().await.map_err(err)?;
    acc.g().settings = Some(s.clone());
    Ok(s)
}

pub async fn profile_reorder_playlist(core: C<'_>, order: Vec<String>, from: usize, to: usize) -> Res {
    let acc = need_acct(core)?;
    let core = core.clone();
    let s = current_settings(&core, &acc).await?;
    if from == to || from >= order.len() || to >= order.len() {
        return Ok(());
    }
    let mut order = order;
    let id = order.remove(from);
    order.insert(to, id);
    let ids: Vec<String> = order.into_iter().filter(|i| s.playlists.contains(i)).collect();
    if ids != s.playlists {
        patch_profile(&core, &acc, ProfilePatch { playlists: Some(ids), ..Default::default() }).await?;
    }
    Ok(())
}

pub async fn profile_toggle_playlist(core: C<'_>, id: String) -> Res {
    let acc = need_acct(core)?;
    let core = core.clone();
    if core.st().library.playlist(&id).is_none_or(|p| p.local_only) {
        return Ok(());
    }
    let s = current_settings(&core, &acc).await?;
    let mut ids = s.playlists.clone();
    match ids.iter().position(|x| *x == id) {
        Some(i) => {
            ids.remove(i);
        }
        None => ids.push(id),
    }
    // Playlists deleted meanwhile drop out.
    {
        let st = core.st();
        ids.retain(|i| st.library.playlist(i).is_some());
    }
    patch_profile(&core, &acc, ProfilePatch { playlists: Some(ids), ..Default::default() }).await
}

/// Cuts a picture from the UI (already cropped) down to the upload limit.
fn prepare_upload(bytes: &[u8], banner: bool) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 > glyph_core::profile_images::MAX_SOURCE_BYTES {
        return Err("too_big".into());
    }
    let img = image::load_from_memory(bytes).map_err(|_| "generic".to_string())?;
    let img = if banner { img.thumbnail(1500, 500) } else { img.thumbnail(512, 512) };
    for quality in [82.0, 70.0, 58.0] {
        let out = glyph_core::profile_images::encode_webp(&img, quality).map_err(|_| "generic".to_string())?;
        if out.len() <= glyph_core::profile_images::MAX_UPLOAD_BYTES {
            return Ok(out);
        }
    }
    Err("too_big".into())
}

async fn set_image(core: &Arc<Core>, acc: &Arc<Acct>, banner: bool, bytes: Option<Vec<u8>>) -> Res {
    let b = need_backend(core)?;
    let p = b.profile_set_image(banner, bytes).await.map_err(err)?;
    acc.g().settings = Some(p);
    acc.refresh_avatar();
    Ok(())
}

/// `image`: the cropped picture as a data URL; `null` removes it.
pub async fn profile_set_image(core: C<'_>, banner: bool, image: Option<String>) -> Res {
    let acc = need_acct(core)?;
    let bytes = match image {
        None => None,
        Some(url) => {
            let raw = b64_decode(url.split_once(',').map_or(url.as_str(), |x| x.1)).ok_or_else(|| "generic".to_string())?;
            let out = crate::rt::spawn_blocking(move || prepare_upload(&raw, banner)).await.map_err(|e| e.to_string())??;
            Some(out)
        }
    };
    set_image(core, &acc, banner, bytes).await
}

pub async fn profile_remove_image(core: C<'_>, banner: bool) -> Res {
    let acc = need_acct(core)?;
    set_image(core, &acc, banner, None).await
}

/// File dialog, then the picture is centred, cut and uploaded.
pub async fn profile_pick_image(core: C<'_>, banner: bool) -> Res {
    let acc = need_acct(core)?;
    let core = core.clone();
    let Some(path) = core.host.pick_file(None, "Images", &["jpg", "jpeg", "png", "webp"]).await? else {
        return Ok(());
    };
    let bytes = crate::rt::spawn_blocking(move || glyph_core::profile_images::prepare(&path, banner).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())??;
    set_image(&core, &acc, banner, Some(bytes)).await
}

// ------------------------------------------------------------------ commands: privacy

pub fn privacy_report(core: C) -> Value {
    use glyph_core::privacy::{report, Inputs, Library as Crypto};
    let cfg = core.st().config.clone();
    let signed = stored(&core);
    let b = core.backend();
    let library = match (&b, &signed) {
        (Some(b), Some(_)) => b.library_crypto(),
        (None, Some(_)) => Crypto::Starting,
        _ => Crypto::Guest,
    };
    let mcp_relay = cfg.mcp.enabled && cfg.mcp.mode == glyph_core::config::McpMode::Server && signed.is_some();
    let lines = report(&Inputs {
        official: official(&cfg),
        library,
        profile_playlists: b.as_ref().is_some_and(|b| b.profile_shows_playlists()),
        mcp_relay,
    });
    let lines: Vec<Value> = lines.iter().map(|l| json!({ "area": l.area, "level": l.level.name(), "note": l.note })).collect();
    let events: Vec<Value> = glyph_core::privacy::events(cfg.effective_server_url())
        .into_iter()
        .take(20)
        .map(|e| json!({ "at": e.at, "code": e.code, "detail": e.detail, "alarm": e.alarm }))
        .collect();
    json!({ "lines": lines, "events": events, "has_password": signed.is_some_and(|s| s.account.has_password) })
}

pub fn privacy_clear(core: C) {
    let server = core.st().config.effective_server_url().to_string();
    glyph_core::privacy::clear(&server);
}

/// The user signed in on the server's own web page: a notice.
pub fn privacy_record_web_login(core: C) {
    let server = core.st().config.effective_server_url().to_string();
    glyph_core::privacy::record(&server, "", false, "web_login", "");
}

// ------------------------------------------------------------------ commands: server

pub fn server_info(core: C) -> Value {
    let cfg = core.st().config.clone();
    let discord_builtin = core.host.discord_builtin();
    let auth_required = core.backend().is_some_and(|b| b.server_auth_required());
    let state = core.backend_state.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut v = match core.backend().map(|b| b.mode()) {
        Some(glyph_core::BackendMode::Server { url, server_version }) => json!({ "kind": "server", "url": url, "version": server_version }),
        Some(glyph_core::BackendMode::Direct) => json!({ "kind": "direct" }),
        Some(glyph_core::BackendMode::Incompatible { url, protocol_version }) => json!({ "kind": "incompatible", "url": url, "protocol": protocol_version }),
        None => json!({ "kind": "offline", "reason": state["reason"].as_str().unwrap_or_default() }),
    };
    if v["url"].is_null() && v["kind"] == "offline" {
        v["url"] = json!(cfg.effective_server_url());
    }
    v["auth_required"] = json!(auth_required);
    v["discord_builtin"] = json!(discord_builtin);
    v
}

pub async fn server_test(core: C<'_>, url: String) -> Res<Value> {
    let core = core.clone();
    let mut cfg = core.st().config.clone();
    cfg.server_url = url.trim().to_string();
    let base = cfg.effective_server_url().to_string();
    if base.is_empty() {
        return Ok(json!({ "ok": true, "text": tr(&core, "settings.server.direct"), "auth_required": false }));
    }
    // A throw-away backend, built away from the async runtime (it owns one).
    let probe_base = base.clone();
    let r = crate::rt::spawn_blocking(move || {
        let probe = glyph_core::server_access::probe(&probe_base).ok();
        let r = futures::executor::block_on(crate::guard::guard_async(Backend::new(cfg)));
        let out = r.map(|b| (b.mode(), b.server_auth_required()));
        (out, probe)
    })
    .await
    .map_err(|e| e.to_string())?;
    let (out, probe) = r;
    let text = |k: &str, vars: &[(&str, &str)]| trf(&core, k, vars);
    let v = match out {
        Ok((glyph_core::BackendMode::Server { server_version, .. }, required)) => {
            if server_version == "offline" {
                let key = if required { "settings.server.bearer.required" } else { "settings.server.unreachable" };
                json!({ "ok": false, "text": text(key, &[]), "auth_required": required })
            } else {
                json!({ "ok": true, "text": text("settings.server.ok", &[("url", &base), ("v", &server_version)]), "auth_required": required })
            }
        }
        Ok((glyph_core::BackendMode::Incompatible { protocol_version, .. }, required)) => {
            json!({ "ok": false, "text": text("status.incompatible", &[("v", &protocol_version.to_string())]), "auth_required": required })
        }
        Ok((glyph_core::BackendMode::Direct, required)) => json!({ "ok": false, "text": text("settings.server.unreachable", &[("url", &base)]), "auth_required": required }),
        Err(e) => json!({ "ok": false, "text": e, "auth_required": probe.unwrap_or(false) }),
    };
    Ok(v)
}

/// Saves the server's access token in the OS keychain (never in settings).
pub async fn server_store_token(url: String, token: String) -> Res {
    let cfg = Config { server_url: url.trim().to_string(), ..Config::default() };
    let base = cfg.effective_server_url().to_string();
    let token = token.trim().to_string();
    crate::rt::spawn_blocking(move || glyph_core::server_access::store(&base, &token).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}
