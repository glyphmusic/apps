//! Jam (listening together) and the playback devices (Google Cast, AirPlay,
//! UPnP / DLNA / Sonos, MPD). From `src/app/jam.rs` and `src/app/net_devices.rs`
//! (plus the device parts of `playback.rs` / `poll.rs`) without GPUI.
//!
//! The host's app owns the jam: its hand-queued songs ("Up next") are the
//! jam's queue; it publishes what plays, what is next and what happened, and
//! runs the friends' requests as far as the host allows. A guest follows the
//! host's song and moment (or only shows it, when listening at the host).
//!
//! The controller does not know about any of this except through the
//! `route_*` hooks below; each returns `true` when it handled the thing and
//! the controller must not do it locally. Everything that runs on its own
//! (clock, publishing, device polling) is driven by a thread started in
//! `setup`.
//!
//! Lock order: the controller must never hold `core.st()` while calling a
//! hook here. Inside: jam -> out -> core.st().

use crate::controller::Core;
use crate::devices::{Device, Discovery, Kind};
use crate::guard::guard;
use crate::mpd::Cmd;
use futures::future::{select, Either};
use glyph_audio::{BackendKind, PlayerState, Status};
use glyph_core::dedupe::{same_song, track_key};
use glyph_core::jam::{ClockSync, Step};
use glyph_core::server_client::LiveEvent;
use glyph_core::{Backend, Queue};
use glyph_protocol::jam::{
    push_feed, Control, Event, Item, Live, Member, Now, Op, OpFrom, OpsResponse, State, What, HOST_TIMEOUT_MS,
    LIVE_CLOSE_AGAIN, LIVE_HOST_EVERY_MS, MAX_AFTER, MAX_MEMBERS, MAX_QUEUE,
};
use glyph_protocol::{PlaybackSource, Track};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub use glyph_protocol::jam::{Perms, Reaction};

type C<'a> = &'a Arc<Core>;
type Res<T = Value> = Result<T, String>;

/// Host: publish at least this often while a song plays.
const HEARTBEAT: Duration = Duration::from_secs(15);
/// The same with pushes.
const HEARTBEAT_LIVE: Duration = Duration::from_secs(60);
/// After the push socket closed or failed, long poll this long first.
const LIVE_RETRY: Duration = Duration::from_secs(300);
/// Host: a position this far from the published one was a seek.
const SEEK_JUMP_MS: i64 = 2000;
/// Host: how often to look for something to publish.
const PUBLISH_CHECK: Duration = Duration::from_millis(100);
/// Guest: how often the position is checked against the host.
const SYNC_EVERY: Duration = Duration::from_millis(500);
/// Guest: after starting a song or a jump, give the stream this long.
const SETTLE: Duration = Duration::from_secs(3);
/// Clock samples: a few quick ones at the start, then one a minute.
const CLOCK_FIRST: usize = 4;
const CLOCK_QUICK: Duration = Duration::from_millis(700);
const CLOCK_EVERY: Duration = Duration::from_secs(60);
/// Asking again while waiting to be let in.
const KNOCK_EVERY: Duration = Duration::from_secs(3);
/// One reaction per person this often (host side).
const REACT_GAP: Duration = Duration::from_millis(1500);

// ------------------------------------------------------------ shared state

#[derive(Default)]
struct Rt {
    out: Mutex<Out>,
    jam: Mutex<JamUi>,
    /// Bumped to stop the running jam loops.
    gen: AtomicU64,
    /// The song being found should load paused.
    hold: AtomicBool,
    /// Player state of a remote output / the jam host, shown instead of the
    /// local player's.
    shown: Mutex<Option<PlayerState>>,
    /// Profile pictures by hash: local file once fetched.
    images: Mutex<HashMap<String, Option<PathBuf>>>,
}

static RT: OnceLock<Rt> = OnceLock::new();

fn rt() -> &'static Rt {
    RT.get_or_init(Rt::default)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn cur_gen() -> u64 {
    rt().gen.load(Ordering::Relaxed)
}

fn bump() -> u64 {
    rt().gen.fetch_add(1, Ordering::Relaxed) + 1
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// A toast with an i18n key, optional `{vars}` and a second line.
fn toast(core: &Core, key: &str, vars: Value, sub: Option<String>, kind: &str) {
    core.emit("toast", json!({ "key": key, "text": key, "vars": vars, "sub": sub, "kind": kind }));
}

fn toast_info(core: &Core, key: &str) {
    toast(core, key, Value::Null, None, "info");
}

// ------------------------------------------------------------ remote outputs

/// Where songs play when it is not this computer.
#[derive(Default)]
struct Out {
    mpd: Option<crate::mpd::Output>,
    device: Option<crate::devices::Output>,
    discovery: Option<Discovery>,
    disc_gen: u64,
    /// The song cannot play on the output: it plays here.
    local_fallback: bool,
    device_failed: bool,
    mpd_was_playing: bool,
    /// The song loaded last, to move it to another output.
    last_load: Option<(Track, PlaybackSource)>,
}

impl Out {
    /// Songs play somewhere else, not here.
    fn remote_on(&self) -> bool {
        (self.mpd.is_some() || self.device.is_some()) && !self.local_fallback
    }

    /// A remote output is chosen (even while this song plays here).
    fn chosen(&self) -> bool {
        self.mpd.is_some() || self.device.is_some()
    }

    /// Sends a command to the remote output; `false` when playing here.
    fn send(&self, c: Cmd) -> bool {
        if self.local_fallback {
            return false;
        }
        if let Some(m) = &self.mpd {
            m.send(c);
            return true;
        }
        if let Some(d) = &self.device {
            d.send(c);
            return true;
        }
        false
    }

    fn status(&self) -> Option<crate::mpd::Status> {
        if self.local_fallback {
            return None;
        }
        if let Some(m) = &self.mpd {
            return m.status();
        }
        self.device.as_ref().and_then(|d| d.status())
    }

    /// Hands a found song to where it plays, `at` seconds in. `false`: it
    /// plays here (nothing chosen, or the output cannot play it).
    fn load_at(&mut self, core: &Core, tr: Track, src: PlaybackSource, start: bool, at: f64) -> bool {
        self.last_load = Some((tr.clone(), src.clone()));
        self.device_failed = false;
        self.local_fallback = false;
        let dev = self.device.as_ref().map(|d| (d.load(&tr, &src, start, at), d.device.name.clone()));
        if let Some((ok, name)) = dev {
            if ok {
                self.mpd_was_playing = false;
                // Nothing plays here at the same time.
                if let Some(p) = &core.player {
                    p.stop();
                }
                return true;
            }
            self.local_fallback = true;
            toast(core, "toast.device_unsupported", json!({ "name": name }), None, "info");
        } else if let Some(m) = &self.mpd {
            let url = match &src {
                PlaybackSource::Stream { url, .. } | PlaybackSource::Preview { url, .. } => m.url_for(url),
                _ => None,
            };
            match url {
                Some(url) => {
                    m.send(Cmd::Play { url, start });
                    if at > 1.0 {
                        m.send(Cmd::Seek(at));
                    }
                    self.mpd_was_playing = false;
                    if let Some(p) = &core.player {
                        p.stop();
                    }
                    return true;
                }
                None => {
                    self.local_fallback = true;
                    toast_info(core, "toast.mpd_unsupported");
                }
            }
        }
        false
    }

    /// After the output changed: the current song goes on where it is now
    /// played, from the same position.
    fn continue_on_new_output(&mut self, core: &Core, pos: f64, playing: bool) {
        let (cur, resolving) = {
            let st = core.st();
            (st.current.clone(), st.resolving)
        };
        if resolving {
            return;
        }
        let (Some(cur), Some((tr, src))) = (cur, self.last_load.clone()) else { return };
        if !same_song(&cur, &tr) {
            return;
        }
        if self.load_at(core, tr.clone(), src.clone(), playing, pos) {
            return;
        }
        if let Some(p) = &core.player {
            p.load(tr, src);
            if playing {
                p.play();
            }
            if pos > 1.0 {
                p.seek(Duration::from_secs_f64(pos));
            }
        }
    }

    /// Stops the network device output (if any) without touching MPD.
    fn drop_device(&mut self) {
        if let Some(old) = self.device.take() {
            old.send(Cmd::Stop);
        }
        self.device_failed = false;
    }

    /// Starts / stops the MPD output as the settings say.
    fn sync_mpd(&mut self, core: &Core) {
        let want = {
            let st = core.st();
            st.config.mpd.enabled.then(|| crate::mpd::Address::parse(&st.config.mpd.address)).flatten()
        };
        if want.as_ref() == self.mpd.as_ref().map(|m| &m.address) {
            return;
        }
        if let Some(old) = self.mpd.take() {
            old.send(Cmd::Stop);
        }
        self.mpd_was_playing = false;
        if let Some(a) = want {
            // One remote at a time.
            self.drop_device();
            self.local_fallback = false;
            tracing::info!(host = %a.host, port = a.port, "MPD output on");
            if let Some(p) = &core.player {
                p.pause();
            }
            self.mpd = Some(crate::mpd::Output::start(a));
        }
    }

    /// Starts / stops the device search as the settings say.
    fn sync_discovery(&mut self, core: &Core) {
        let want = core.st().config.devices.discovery;
        if want == self.discovery.is_some() {
            return;
        }
        self.discovery = want.then(Discovery::start);
    }
}

fn local_state(core: &Core) -> Option<PlayerState> {
    core.player.as_ref().map(|p| p.state())
}

/// The player's state as shown: a remote output's or the jam host's when
/// one is on, else the local player's. `Core::player_state` should return this.
pub fn player_state(core: &Core) -> Option<PlayerState> {
    lock(&rt().shown).clone().or_else(|| local_state(core))
}

fn is_playing(core: &Core) -> bool {
    player_state(core).is_some_and(|s| s.status == Status::Playing)
}

fn position(core: &Core) -> Duration {
    player_state(core).map_or(Duration::ZERO, |s| s.position)
}

fn last_load() -> Option<(Track, PlaybackSource)> {
    lock(&rt().out).last_load.clone()
}

fn remote_send(c: Cmd) -> bool {
    lock(&rt().out).send(c)
}

fn remote_chosen() -> bool {
    lock(&rt().out).chosen()
}

fn ui_changed(a: &PlayerState, b: &PlayerState) -> bool {
    a.status != b.status
        || a.track != b.track
        || a.provider != b.provider
        || a.backend != b.backend
        || a.is_preview != b.is_preview
        || a.duration != b.duration
        || a.volume != b.volume
        || a.position.as_millis() / 100 != b.position.as_millis() / 100
}

// ------------------------------------------------------------ hooks for the controller

/// `start_track`, at the top: a new song starts, not paused.
pub fn on_track_changed(_core: &Core) {
    rt().hold.store(false, Ordering::Relaxed);
}

/// `start_track`, where the source is found (before `p.load`): hands the
/// song to the MPD / network device. `true` = handled, do not play here.
/// When `false`, play here and use [`start_paused`] to decide `p.play()`.
pub fn route_load(core: &Core, track: &Track, src: &PlaybackSource) -> bool {
    let start = !rt().hold.load(Ordering::Relaxed);
    let handled = lock(&rt().out).load_at(core, track.clone(), src.clone(), start, 0.0);
    if handled {
        rt().hold.store(false, Ordering::Relaxed);
    }
    handled
}

/// After a local `p.load(..)`: `true` = do not call `p.play()` (a paused
/// host song, or pause pressed while it was being found). Clears the wish.
pub fn start_paused() -> bool {
    rt().hold.swap(false, Ordering::Relaxed)
}

/// `toggle_play`, first thing. `true` = handled.
pub fn route_toggle_play(core: &Core) -> bool {
    let playing = is_playing(core);
    if control(core, if playing { Control::Pause } else { Control::Play }) {
        return true;
    }
    // The new song is still being found: remember the wish instead.
    if core.st().resolving {
        rt().hold.fetch_xor(true, Ordering::Relaxed);
        core.emit_player();
        return true;
    }
    remote_send(Cmd::Pause(playing))
}

/// `pause`, first thing. `true` = handled.
pub fn route_pause(core: &Core) -> bool {
    if control(core, Control::Pause) {
        return true;
    }
    let (resolving, changed) = {
        let mut st = core.st();
        let was = st.hold_play;
        if st.resolving {
            st.hold_play = true;
        }
        (st.resolving, !was)
    };
    if resolving {
        let was_rt = rt().hold.swap(true, Ordering::Relaxed);
        if changed || !was_rt {
            core.emit_player();
        }
        return true;
    }
    remote_send(Cmd::Pause(true))
}

/// `resume`, first thing. `true` = handled.
pub fn route_resume(core: &Core) -> bool {
    if control(core, Control::Play) {
        return true;
    }
    let resolving = {
        let mut st = core.st();
        if st.resolving {
            st.hold_play = false;
        }
        st.resolving
    };
    if resolving {
        rt().hold.store(false, Ordering::Relaxed);
        core.emit_player();
        return true;
    }
    remote_send(Cmd::Pause(false))
}

/// `seek`, first thing. `true` = handled.
pub fn route_seek(core: &Core, secs: f64) -> bool {
    let secs = secs.max(0.0);
    if control(core, Control::Seek { ms: (secs * 1000.0) as u64 }) {
        return true;
    }
    remote_send(Cmd::Seek(secs))
}

/// `next(auto)`, first thing. `true` = handled (a guest: the host decides).
pub fn route_next(core: &Core, auto: bool) -> bool {
    let (guest, remote) = {
        let u = lock(&rt().jam);
        (u.jam.as_ref().is_some_and(|j| !j.host), u.jam.as_ref().is_some_and(|j| !j.host && !j.listen_here))
    };
    // A guest's own player still moves on when a song ends (its queue is
    // the host's); the host's next state corrects it if not.
    if guest && (!auto || remote) {
        if !auto {
            control(core, Control::Next);
        }
        return true;
    }
    false
}

/// `previous`, first thing. `true` = handled.
pub fn route_previous(core: &Core) -> bool {
    control(core, Control::Previous)
}

/// Volume set (`set_volume`, and `toggle_mute` with 0 / the old level).
/// `true` = sent to the remote output; skip the local player and the config.
pub fn route_volume(_core: &Core, v: f32) -> bool {
    remote_send(Cmd::Volume((v.clamp(0.0, 1.0) * 100.0).round() as u8))
}

/// A song picked (`play`, `play = true`) or queued (`play = false`): a guest
/// asks the host instead. `true` = handled. Call it with the song at `index`.
pub fn route_request(core: &Core, track: &Track, play: bool) -> bool {
    let key;
    {
        let mut u = lock(&rt().jam);
        let Some(j) = u.jam.as_mut().filter(|j| !j.host) else { return false };
        // The host's song itself: nothing to ask for.
        if j.state.now.as_ref().is_some_and(|n| same_song(&n.track, track)) {
            return true;
        }
        let perms = j.state.perms;
        if play && perms.play_now {
            j.outbox.push(Op::PlayNow { track: track.clone() });
            key = "jam.play_requested";
        } else if perms.add {
            j.outbox.push(Op::Add { track: track.clone() });
            key = "jam.requested";
        } else {
            drop(u);
            toast_info(core, "jam.not_allowed");
            return true;
        }
    }
    toast(core, key, Value::Null, Some(track.title.clone()), "info");
    true
}

/// `queue_jump(ui_index)`. `true` = handled (a guest asks the host).
pub fn route_queue_jump(core: &Core, ui_index: usize) -> bool {
    if !is_guest() {
        return false;
    }
    let t = {
        let st = core.st();
        crate::dto::upcoming_index(&st.queue, ui_index).and_then(|i| st.queue.upcoming().get(i).map(|t| (*t).clone()))
    };
    if let Some(t) = t {
        route_request(core, &t, true);
    }
    true
}

/// `queue_remove(ui_index)` (`Some`) or `queue_move` / `queue_clear` (`None`).
/// `true` = handled: a guest may only remove its own songs.
pub fn route_queue_edit(core: &Core, remove: Option<usize>) -> bool {
    if !is_guest() {
        return false;
    }
    let item = remove.and_then(|ui| {
        let st = core.st();
        let i = crate::dto::upcoming_index(&st.queue, ui)?;
        drop(st);
        lock(&rt().jam).jam.as_ref().and_then(|j| j.state.queue.get(i).cloned())
    });
    match item {
        Some(item) => remove_item(core, &item.id),
        None => toast_info(core, "jam.not_allowed"),
    }
    true
}

/// `start_track`: the song has no source here (`track_key`). For a guest
/// the jam page then says the host's song is unavailable.
pub fn no_source(core: &Core, key: &str) {
    // A song moved here from another Glyph app: tell the sender it failed.
    crate::account::resolve_failed(core);
    if let Some(j) = lock(&rt().jam).jam.as_mut().filter(|j| !j.host) {
        j.unavailable = Some(key.to_string());
    }
}

/// In someone else's jam: skipping unplayable songs is up to the host.
pub fn is_guest() -> bool {
    lock(&rt().jam).jam.as_ref().is_some_and(|j| !j.host)
}

/// A jam is on (sleep timer, "available" for account devices).
pub fn jam_active() -> bool {
    lock(&rt().jam).jam.is_some()
}

/// Songs play on MPD or a network device (media keys: volume, no preload).
pub fn remote_on() -> bool {
    lock(&rt().out).remote_on()
}

/// Discord: the jam as a party `(id, people, max)`.
pub fn discord_party() -> Option<(String, usize, usize)> {
    use std::hash::{Hash, Hasher};
    let u = lock(&rt().jam);
    let j = u.jam.as_ref()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    j.code.hash(&mut h);
    Some((format!("jam-{:x}", h.finish()), j.member_count(), MAX_MEMBERS))
}

/// Plugins may start / stop / seek playback only when no one else decides
/// what plays (a jam guest follows the host).
pub fn allows_control() -> bool {
    !is_guest()
}

/// Skipping an unplayable song is not up to this app: a jam guest, another
/// Glyph app being controlled, or a song moved here that failed.
pub fn skip_blocked(core: &Core) -> bool {
    is_guest() || crate::account::controlling(core) || crate::account::transfer_pending(core)
}

/// The song being found should load paused (pause pressed meanwhile).
pub fn hold_play() -> bool {
    rt().hold.load(Ordering::Relaxed)
}

pub fn set_hold(on: bool) {
    rt().hold.store(on, Ordering::Relaxed);
}

/// Stops MPD / the network device (the account's device list takes over).
pub fn remote_stop() -> bool {
    remote_send(Cmd::Stop)
}

/// Another Glyph app is chosen: MPD and the network device are let go.
pub fn release_outputs(core: &Core) {
    let mut out = lock(&rt().out);
    out.drop_device();
    let was_on = std::mem::replace(&mut core.st().config.mpd.enabled, false);
    if was_on {
        core.config_changed();
    }
    out.sync_mpd(core);
    out.local_fallback = false;
    out.mpd_was_playing = false;
}

/// A song moved here from another Glyph app plays on this computer.
pub fn use_local_output(core: &Core) {
    use_local(core);
}

/// Signing out: the jam is left (the host ends it).
pub fn leave_for_sign_out(core: &Core) {
    if jam_active() {
        leave(core);
    }
    {
        let mut u = lock(&rt().jam);
        u.waiting = None;
        u.resume = None;
        u.resume_at = None;
        u.resume_tries = 0;
    }
    clear_saved_jam();
    bump();
    emit_jam(core);
}

/// Tells the UI the jam changed (it fetches the snapshot itself).
fn emit_jam(core: &Core) {
    core.emit("jam", snapshot(core, false));
}

/// A `glyph://jam/...` link or jam code reached the app (deep link, second
/// instance, pasted into the search bar). Joins it when the backend is up.
pub fn open_link(core: &Arc<Core>, link: &str) {
    if glyph_protocol::jam::normalize_code(link).is_none() {
        return;
    }
    {
        let mut u = lock(&rt().jam);
        u.resume = None;
        u.resume_at = None;
        u.resume_tries = 0;
    }
    if core.backend().is_none() {
        lock(&rt().jam).pending = Some(link.to_string());
        return;
    }
    let (core, link) = (core.clone(), link.to_string());
    crate::rt::spawn(async move {
        // The Connect panel shows the Jam page, as in the GPUI app.
        core.emit("jam_open", json!({}));
        match join(&core, &link).await {
            Ok(()) => emit_jam(&core),
            Err(e) => toast(&core, &problem_key(&e), Value::Null, None, "info"),
        }
    });
}

/// Status for Settings > Playback (`playback_status`).
pub fn playback_status() -> Value {
    let out = lock(&rt().out);
    let mpd = match &out.mpd {
        Some(m) => json!({ "connected": m.status().map(|_| m.address.label()), "error": m.error() }),
        None => json!({ "connected": null, "error": null }),
    };
    let devices = out.discovery.as_ref().map(|d| {
        json!({
            "found": d.devices().len(),
            "searching": d.searching(),
            "playing": out.device.as_ref().map(|o| o.device.name.clone()),
            "error": out.device.as_ref().and_then(|o| o.error()),
        })
    });
    json!({ "mpd": mpd, "devices": devices })
}

// ------------------------------------------------------------ setup and the background thread

/// Called once at start-up from `glyph_app::setup`.
pub fn setup(core: &Arc<Core>) {
    {
        let mut out = lock(&rt().out);
        out.sync_discovery(core);
        out.sync_mpd(core);
    }
    if let Some(saved) = load_saved_jam() {
        lock(&rt().jam).resume = Some(saved);
    }
    let core = core.clone();
    let _ = std::thread::Builder::new().name("glyph-net".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_millis(100));
        tick(&core);
    });
}

/// Every 100 ms: settings, found devices, the jam, the remote output's state.
fn tick(core: &Arc<Core>) {
    let mut devices_changed = false;
    {
        let mut out = lock(&rt().out);
        out.sync_discovery(core);
        out.sync_mpd(core);
        let g = out.discovery.as_ref().map_or(0, |d| d.generation());
        if g != out.disc_gen {
            out.disc_gen = g;
            devices_changed = true;
        }
    }
    if devices_changed {
        core.emit("devices", devices_json(core));
    }
    jam_tick(core);

    let listen_remote = lock(&rt().jam).jam.as_ref().is_some_and(|j| !j.host && !j.listen_here);
    let mut ended = false;
    let new = if listen_remote {
        remote_state(core)
    } else {
        let remote = lock(&rt().out).remote_on();
        if remote {
            let (s, e) = poll_remote(core);
            ended = e;
            s.or_else(|| lock(&rt().shown).clone())
        } else {
            None
        }
    };
    let changed = {
        let mut sh = lock(&rt().shown);
        let changed = match (&*sh, &new) {
            (None, None) => false,
            (Some(a), Some(b)) => ui_changed(a, b),
            _ => true,
        };
        *sh = new;
        changed
    };
    if changed || ended {
        core.emit_player();
    }
    if ended {
        core.next(true);
    }
}

/// Player state from MPD's (or the network device's) status; its song
/// ending moves the queue on. `(state, ended)`.
fn poll_remote(core: &Core) -> (Option<PlayerState>, bool) {
    let mut out = lock(&rt().out);
    let Some(st) = out.status() else { return (None, false) };
    let (current, resolving, cfg_vol) = {
        let s = core.st();
        (s.current.clone(), s.resolving, s.config.player.volume)
    };
    if st.state == "error" && !out.device_failed && !resolving {
        out.device_failed = true;
        let name = match (&out.device, &out.mpd) {
            (Some(d), _) => Some(d.device.name.clone()),
            (None, Some(m)) => Some(m.address.label()),
            _ => None,
        };
        if let Some(name) = name {
            toast(core, "toast.device_failed", json!({ "name": name }), None, "info");
        }
    }
    let status = match st.state.as_str() {
        "play" => Status::Playing,
        "pause" => Status::Paused,
        "load" => Status::Loading,
        _ if resolving => Status::Loading,
        _ => Status::Idle,
    };
    let playing = status == Status::Playing;
    let preview = out.last_load.as_ref().is_some_and(|(_, s)| matches!(s, PlaybackSource::Preview { .. }));
    let volume = st.volume.map_or_else(|| local_state(core).map_or(cfg_vol, |s| s.volume), |v| v as f32 / 100.0);
    let state = PlayerState {
        status,
        provider: current.as_ref().map(|t| t.source),
        track: current,
        backend: Some(BackendKind::Native),
        is_preview: preview,
        position: Duration::from_secs_f64(st.elapsed.unwrap_or(0.0).max(0.0)),
        duration: st.duration.filter(|d| *d > 0.0).map(Duration::from_secs_f64),
        volume,
    };
    let ended = out.mpd_was_playing && st.state == "stop" && !resolving;
    out.mpd_was_playing = playing;
    (Some(state), ended)
}

// ------------------------------------------------------------ devices (commands)

fn kind_json(k: Kind) -> (&'static str, &'static str) {
    match k {
        Kind::Chromecast => ("cast", "Google Cast"),
        Kind::AirPlay => ("airplay", "AirPlay"),
        Kind::Sonos => ("upnp", "Sonos"),
        Kind::Upnp => ("upnp", "DLNA"),
        Kind::Mpd => ("upnp", "MPD"),
    }
}

/// The Connect panel's list (`DeviceX[]`): this computer, then what the
/// network search found.
pub fn devices_json(core: &Core) -> Value {
    let out = lock(&rt().out);
    let mpd_addr = out.mpd.as_ref().map(|m| m.address.clone());
    let selected = out.device.as_ref().map(|o| o.device.id.clone());
    let account = crate::account::devices_json(core);
    let controlling = crate::account::controlling(core);
    let mut list = vec![json!({ "id": "this", "name": "This computer", "kind": "this", "active": !out.chosen() && !controlling })];
    let found = out.discovery.as_ref().map(|d| d.devices()).unwrap_or_default();
    let mut mpd_seen = false;
    for d in &found {
        let (kind, protocol) = kind_json(d.kind);
        let is_mpd = d.kind == Kind::Mpd;
        let active = if is_mpd {
            mpd_addr.as_ref().is_some_and(|a| crate::mpd::Address::parse(&d.mpd_address()).is_some_and(|x| x.host == a.host && x.port == a.port))
        } else {
            selected.as_deref() == Some(d.id.as_str())
        };
        mpd_seen |= is_mpd && active;
        let mut v = json!({ "id": d.id, "name": d.name, "kind": kind, "protocol": protocol, "icon": d.icon, "active": active });
        if active {
            let (error, status) = if is_mpd {
                (out.mpd.as_ref().and_then(|m| m.error()), out.mpd.as_ref().and_then(|m| m.status()).is_some())
            } else {
                (out.device.as_ref().and_then(|o| o.error()), out.device.as_ref().and_then(|o| o.status()).is_some())
            };
            v["unreachable"] = json!(error.is_some());
            v["connecting"] = json!(error.is_none() && !status);
            v["error"] = json!(error);
        }
        list.push(v);
    }
    // The MPD of the settings, found or not.
    if let (Some(m), false) = (&out.mpd, mpd_seen) {
        let error = m.error();
        list.push(json!({
            "id": format!("mpd:{}:{}", m.address.host, m.address.port), "name": m.address.label(), "kind": "upnp", "protocol": "MPD",
            "icon": "radio", "active": true, "unreachable": error.is_some(), "connecting": error.is_none() && m.status().is_none(), "error": error,
        }));
    }
    // A device in use that the search does not list (any more).
    if let Some(o) = &out.device {
        if !found.iter().any(|d| d.id == o.device.id) {
            let (kind, protocol) = kind_json(o.device.kind);
            let error = o.error();
            list.push(json!({
                "id": o.device.id, "name": o.device.name, "kind": kind, "protocol": protocol, "icon": o.device.icon, "active": true,
                "unreachable": error.is_some(), "connecting": error.is_none() && o.status().is_none(), "error": error,
            }));
        }
    }
    // Other Glyph apps of the account.
    list.extend(account);
    Value::Array(list)
}

/// The id is one of the account's Glyph apps.
fn is_account_device(core: &Core, id: &str) -> bool {
    crate::account::devices_json(core).iter().any(|d| d["id"].as_str() == Some(id))
}

pub async fn devices_rescan(core: C<'_>, now: Option<bool>) -> Res {
    {
        let out = lock(&rt().out);
        if let Some(d) = &out.discovery {
            if now.unwrap_or(false) {
                d.rescan_now();
            } else {
                d.rescan();
            }
        }
    }
    // Resolves when the search is over.
    let t0 = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let searching = lock(&rt().out).discovery.as_ref().is_some_and(|d| d.searching());
        if (!searching && t0.elapsed() > Duration::from_millis(400)) || t0.elapsed() > Duration::from_secs(10) {
            break;
        }
    }
    let v = devices_json(&core);
    core.emit("devices", v.clone());
    Ok(v)
}

/// Plays on a found device; MPD servers go to the MPD output.
fn select_by_id(core: &Core, id: &str) -> Res {
    let dev = lock(&rt().out).discovery.as_ref().and_then(|d| d.find(id));
    let Some(dev) = dev else {
        // Already in use (an MPD from the settings, or a device gone from the list).
        if lock(&rt().out).chosen() {
            return Ok(devices_json(core));
        }
        return Err("not_found".into());
    };
    select_device_inner(core, dev);
    let v = devices_json(core);
    core.emit("devices", v.clone());
    Ok(v)
}

fn select_device_inner(core: &Core, dev: Device) {
    // A Glyph app of the account was controlled: it stops, this output takes over.
    crate::account::release_output(core);
    let (pos, playing) = (position(core).as_secs_f64(), is_playing(core));
    let mut out = lock(&rt().out);
    if out.device.as_ref().is_some_and(|d| d.device.id == dev.id) {
        return;
    }
    if dev.kind == Kind::Mpd && out.mpd.as_ref().is_some_and(|m| crate::mpd::Address::parse(&dev.mpd_address()).as_ref() == Some(&m.address)) {
        return;
    }
    if dev.kind == Kind::Mpd {
        out.drop_device();
        {
            let mut st = core.st();
            st.config.mpd.address = dev.mpd_address();
            st.config.mpd.enabled = true;
        }
        core.config_changed();
        out.local_fallback = false;
        out.sync_mpd(core);
    } else {
        let was_on = {
            let mut st = core.st();
            std::mem::replace(&mut st.config.mpd.enabled, false)
        };
        if was_on {
            core.config_changed();
            out.sync_mpd(core);
        }
        out.drop_device();
        tracing::info!(device = %dev.name, kind = ?dev.kind, "network output on");
        if let Some(p) = &core.player {
            p.pause();
        }
        out.local_fallback = false;
        out.mpd_was_playing = false;
        out.device = Some(crate::devices::Output::start(dev));
    }
    out.continue_on_new_output(core, pos, playing);
}

pub fn device_select(core: C<'_>, id: String) -> Res {
    select_output_id(&core, id)
}

/// Same as `device_select` (the compact UI uses this name).
pub fn select_device(core: C<'_>, id: String) -> Res {
    select_output_id(&core, id)
}

pub fn select_output_id(core: &Core, id: String) -> Res {
    if is_account_device(core, &id) {
        crate::account::connect_output(core, id);
        let v = devices_json(core);
        core.emit("devices", v.clone());
        return Ok(v);
    }
    select_by_id(core, &id)
}

/// Controls / takes over another Glyph app of the account. Network devices
/// are the same as `device_select`; account devices are not ported (they
/// belong to the account's live connection).
pub fn device_connect(core: C<'_>, id: String) -> Res {
    select_output_id(&core, id)
}

/// Back to this computer.
pub fn device_use_local(core: C<'_>) -> Res {
    use_local_output_now(&core)
}

pub fn use_local_output_now(core: &Core) -> Res {
    crate::account::use_local(core);
    use_local(core);
    let v = devices_json(core);
    core.emit("devices", v.clone());
    Ok(v)
}

fn use_local(core: &Core) {
    let (pos, playing) = (position(core).as_secs_f64(), is_playing(core));
    let mut out = lock(&rt().out);
    if !out.chosen() {
        return;
    }
    let was_remote = out.remote_on();
    let was_on = std::mem::replace(&mut core.st().config.mpd.enabled, false);
    if was_on {
        core.config_changed();
    }
    out.sync_mpd(core);
    out.drop_device();
    out.local_fallback = false;
    out.mpd_was_playing = false;
    if was_remote {
        out.continue_on_new_output(core, pos, playing);
    }
}

// ------------------------------------------------------------ jam: state

/// Persisted so restarting the app automatically rejoins an active Jam.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct SavedJam {
    code: String,
    handle: String,
    #[serde(default = "default_true")]
    listen_here: bool,
    /// Pre-jam queue, so leaving after an app restart does not keep the jam queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    saved_queue: Option<glyph_core::QueueSnapshot>,
}

fn default_true() -> bool {
    true
}

fn saved_jam_path() -> PathBuf {
    glyph_core::paths::data_dir().join("jam.json")
}

fn load_saved_jam() -> Option<SavedJam> {
    let s: SavedJam = glyph_core::paths::read_json(&saved_jam_path())?;
    let code = glyph_protocol::jam::normalize_code(&s.code)?;
    if s.handle.trim().is_empty() {
        return None;
    }
    Some(SavedJam { code, handle: s.handle, listen_here: s.listen_here, saved_queue: s.saved_queue })
}

fn save_jam(code: &str, handle: &str, listen_here: bool, saved_queue: Option<&Queue>) {
    let saved = SavedJam {
        code: code.to_string(),
        handle: handle.to_string(),
        listen_here,
        saved_queue: saved_queue.map(Queue::snapshot),
    };
    if let Err(e) = glyph_core::paths::write_json(&saved_jam_path(), &saved) {
        tracing::info!("jam: save failed: {e}");
    }
}

fn queue_from_snapshot(snap: Option<glyph_core::QueueSnapshot>) -> Option<Queue> {
    snap.map(|s| {
        let mut q = Queue::new();
        q.restore(s);
        q
    })
}

fn clear_saved_jam() {
    let _ = std::fs::remove_file(saved_jam_path());
}

/// Jam state of the app.
#[derive(Default)]
struct JamUi {
    jam: Option<Jam>,
    /// Starting or joining right now.
    busy: bool,
    /// Waiting for the host to let us in (code).
    waiting: Option<String>,
    /// A jam link that arrived before the backend was ready.
    pending: Option<String>,
    /// Saved jam from the previous run, waiting for backend + account to resume.
    resume: Option<SavedJam>,
    /// Startup resume request is currently in flight.
    resuming: bool,
    /// Next retry time when resuming failed transiently (e.g. offline at launch).
    resume_at: Option<Instant>,
    resume_tries: u32,
}

struct Jam {
    code: String,
    host: bool,
    /// Own handle.
    me: String,
    /// Last state seen (guest) or published (host).
    state: State,
    /// Guest: the music plays here (else only at the host's device).
    listen_here: bool,
    /// Guest: the host's song has no source for this app (song key).
    unavailable: Option<String>,
    /// Host: people waiting to be let in (handle, picture).
    knocks: Vec<(String, Option<String>)>,
    clock: ClockSync,
    /// Rough server minus local clock until `clock` has samples (ms).
    offset_ms: i64,
    clock_at: Option<Instant>,
    clock_busy: bool,
    // ---- host
    by: HashMap<String, String>,
    votes: HashMap<String, BTreeSet<String>>,
    guests: Vec<String>,
    avatars: HashMap<String, String>,
    perms: Perms,
    approve: bool,
    feed: Vec<Event>,
    reacted: HashMap<String, Instant>,
    /// After a takeover: continue the song from here (ms) once it is loaded.
    resume_at: Option<u64>,
    /// What was published last: signature, position (ms), when.
    published: Option<(u64, u64, Instant)>,
    publishing: bool,
    /// The push socket is open (fewer heartbeats).
    live: bool,
    checked: Option<Instant>,
    /// The server takes position-only updates (`/now`).
    light: bool,
    // ---- guest
    outbox: Vec<Op>,
    sending: bool,
    synced_at: Option<Instant>,
    settle_until: Option<Instant>,
    /// Tempo set for catching up (1.0 = normal).
    speed: f32,
    /// The own queue, back when leaving.
    saved_queue: Option<Queue>,
    host_gone: Option<Instant>,
    claiming: bool,
    /// Newest feed entry already shown (server ms).
    feed_seen: u64,
}

impl Jam {
    fn new(code: String, host: bool, me: String, state: State) -> Self {
        let avatars = state.members.iter().filter_map(|m| Some((m.handle.clone(), m.avatar.clone()?))).collect();
        let feed_seen = state.feed.iter().map(|e| e.at_ms).max().unwrap_or(0);
        Self {
            code,
            host,
            me,
            offset_ms: state.server_ms as i64 - unix_ms() as i64,
            perms: state.perms,
            approve: state.approve_joins,
            feed: state.feed.clone(),
            state,
            listen_here: true,
            unavailable: None,
            knocks: Vec::new(),
            clock: ClockSync::default(),
            clock_at: None,
            clock_busy: false,
            by: HashMap::new(),
            votes: HashMap::new(),
            guests: Vec::new(),
            avatars,
            reacted: HashMap::new(),
            resume_at: None,
            published: None,
            publishing: false,
            live: false,
            checked: None,
            light: true,
            outbox: Vec::new(),
            sending: false,
            synced_at: None,
            settle_until: None,
            speed: 1.0,
            saved_queue: None,
            host_gone: None,
            claiming: false,
            feed_seen,
        }
    }

    /// The server clock now (ms).
    fn server_now(&self) -> u64 {
        (unix_ms() as i64 + self.clock.offset().unwrap_or(self.offset_ms)).max(0) as u64
    }

    fn perms(&self) -> Perms {
        if self.host {
            self.perms
        } else {
            self.state.perms
        }
    }

    fn approve_joins(&self) -> bool {
        if self.host {
            self.approve
        } else {
            self.state.approve_joins
        }
    }

    fn member_count(&self) -> usize {
        if self.host {
            1 + self.guests.len()
        } else {
            self.state.members.len()
        }
    }

    /// The current member list (host first).
    fn members(&self) -> Vec<Member> {
        if !self.host {
            return self.state.members.clone();
        }
        std::iter::once(Member { handle: self.me.clone(), host: true, avatar: self.avatars.get(&self.me).cloned() })
            .chain(self.guests.iter().map(|g| Member { handle: g.clone(), host: false, avatar: self.avatars.get(g).cloned() }))
            .collect()
    }

    fn avatar_of(&self, handle: &str) -> Option<String> {
        if self.host {
            self.avatars.get(handle).cloned()
        } else {
            self.state.members.iter().find(|m| m.handle == handle).and_then(|m| m.avatar.clone())
        }
    }

    fn feed(&self) -> &[Event] {
        if self.host {
            &self.feed
        } else {
            &self.state.feed
        }
    }

    fn host_online(&self) -> bool {
        self.host || self.state.host_online
    }

    /// Link that invites to this jam (the QR code is drawn by the UI).
    fn link(&self) -> String {
        self.state.link.clone().unwrap_or_else(|| glyph_protocol::jam::share_text(&self.code))
    }

    /// Position of the host's song now (ms).
    fn host_position(&self) -> Option<u64> {
        self.state.position_at(self.server_now())
    }

    fn event(&mut self, who: &str, what: What) {
        let at_ms = self.server_now();
        push_feed(&mut self.feed, Event { at_ms, who: who.to_string(), what });
    }
}

/// A server answer with its error code kept (`jam_ended`, `not_in_jam`, ...).
async fn call<T>(fut: impl std::future::Future<Output = anyhow::Result<T>>) -> Result<T, (Option<String>, String)> {
    use futures::FutureExt;
    match std::panic::AssertUnwindSafe(fut).catch_unwind().await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) if glyph_core::account::is_signed_out(&e) => Err((Some("signed_out".into()), format!("{e:#}"))),
        Ok(Err(e)) => Err((glyph_core::server_client::server_error(&e).map(|(_, c)| c.to_string()), format!("{e:#}"))),
        Err(_) => Err((None, "not implemented".into())),
    }
}

/// The error text a command returns: the known codes are kept as they are
/// (the UI turns them into words), anything else is the message.
fn problem(code: Option<&str>, msg: &str) -> String {
    match code {
        Some(c @ ("jam_ended" | "not_in_jam" | "jam_removed" | "jam_denied" | "jam_full" | "not_found" | "signed_out")) => c.to_string(),
        _ => msg.to_string(),
    }
}

/// The i18n key for a problem text (or the text itself when it is none).
fn problem_key(p: &str) -> String {
    match p {
        "jam_ended" => "jam.err.ended",
        "not_in_jam" | "jam_removed" => "jam.err.removed",
        "jam_denied" => "jam.err.denied",
        "jam_full" => "jam.err.full",
        "not_found" => "jam.err.server",
        "signed_out" => "jam.sign_in",
        other => other,
    }
    .to_string()
}

/// The call failed because the jam is gone for this app.
fn gone(code: Option<&str>) -> bool {
    matches!(code, Some("jam_ended" | "not_in_jam" | "jam_removed" | "signed_out"))
}

fn reaction_key(r: Reaction) -> &'static str {
    match r {
        Reaction::Heart => "jam.r.heart",
        Reaction::Fire => "jam.r.fire",
        Reaction::Sparkles => "jam.r.sparkles",
        Reaction::ThumbsUp => "jam.r.thumbs_up",
    }
}

/// How a push session ended.
enum LiveEnd {
    /// The loop is over (jam left, ended, or handed over).
    Stop,
    /// Connect again right away (someone took over as host).
    Again,
    /// Long poll for [`LIVE_RETRY`], then try again.
    Later,
    /// No pushes for this jam.
    Never,
}

impl LiveEnd {
    fn closed(code: Option<u16>) -> Self {
        if code == Some(LIVE_CLOSE_AGAIN) {
            LiveEnd::Again
        } else {
            LiveEnd::Later
        }
    }

    fn refused(code: Option<&str>) -> Self {
        match code {
            Some("jam_push_off" | "jam_push_busy" | "websocket_required" | "not_found") => LiveEnd::Never,
            _ => LiveEnd::Later,
        }
    }
}

/// What came of asking the server.
#[derive(PartialEq)]
enum Asked {
    Done,
    /// Try again after a pause.
    Failed,
    /// The loop is over.
    Over,
}

// ------------------------------------------------------------ jam: commands

fn account_handle(core: &Core) -> Result<String, String> {
    core.backend_or_err()?.account().map(|a| a.account.handle).ok_or_else(|| "signed_out".to_string())
}

/// The picture of a member: the local file when fetched (else fetched now).
fn avatar_url(core: &Core, handle: &str, sha: Option<String>) -> Option<String> {
    let sha = sha?;
    {
        let mut imgs = lock(&rt().images);
        match imgs.get(&sha) {
            Some(Some(p)) => return Some(crate::host::asset_url(p)),
            Some(None) => return None,
            None => {
                imgs.insert(sha.clone(), None);
            }
        }
    }
    let b = core.backend()?;
    let h = handle.to_string();
    crate::rt::spawn(async move {
        let r = b.profile_image(&h, &sha).await;
        if let Err(e) = &r {
            tracing::info!("jam: profile image: {e:#}");
        }
        lock(&rt().images).insert(sha.clone(), r.ok());
    });
    None
}

/// The first `limit` songs of the host's jam queue (its hand-queued songs).
fn host_items(q: &Queue, j: &Jam, limit: usize) -> Vec<Item> {
    q.upcoming()
        .into_iter()
        .take(q.manual_len().min(MAX_QUEUE).min(limit))
        .map(|tr| {
            let id = track_key(tr);
            Item {
                by: j.by.get(&id).cloned().unwrap_or_else(|| j.me.clone()),
                votes: j.votes.get(&id).map(|v| v.iter().cloned().collect()).unwrap_or_default(),
                id,
                track: tr.clone(),
            }
        })
        .collect()
}

/// What plays now, as published.
fn host_now(core: &Core, j: &Jam) -> Option<Now> {
    let (cur, resolving, restored_pos) = {
        let st = core.st();
        (st.current.clone(), st.resolving, st.restored_pos)
    };
    let resolved = match (&cur, last_load()) {
        (Some(c), Some((tr, _))) if same_song(c, &tr) => Some(tr),
        _ => None,
    };
    let hold = rt().hold.load(Ordering::Relaxed);
    let playing = is_playing(core);
    let pos = if !playing && !resolving {
        restored_pos.unwrap_or_else(|| position(core))
    } else {
        position(core)
    };
    cur.map(|track| Now {
        track,
        position_ms: pos.as_millis() as u64,
        playing: playing || (resolving && !hold),
        // The server fills it in while the clock is not synced yet.
        at_ms: if j.clock.is_empty() { 0 } else { j.server_now() },
        resolved,
    })
}

fn feed_json(e: &Event) -> Value {
    let (what, title, reaction) = match &e.what {
        What::Joined => ("joined", None, None),
        What::Left => ("left", None, None),
        What::Added { title } => ("added", Some(title.clone()), None),
        What::Played { title } => ("played", Some(title.clone()), None),
        What::React { reaction } => ("react", None, Some(*reaction)),
        What::Host => ("host", None, None),
    };
    json!({ "at_ms": e.at_ms, "who": e.who, "what": what, "title": title, "reaction": reaction })
}

/// `{ jam, waiting }` as the Jam page wants it.
fn snapshot(core: &Core, queue_all: bool) -> Value {
    let u = lock(&rt().jam);
    let waiting = u.waiting.clone();
    let Some(j) = u.jam.as_ref() else { return json!({ "jam": null, "waiting": waiting }) };
    let limit = if queue_all { usize::MAX } else { 20 };
    let (items, queue_len, now) = if j.host {
        let st = core.st();
        (host_items(&st.queue, j, limit), st.queue.manual_len().min(MAX_QUEUE), None)
    } else {
        let now = j.state.now.as_ref().map(|n| {
            json!({ "track": n.track, "playing": n.playing, "position_ms": j.host_position().unwrap_or(n.position_ms) })
        });
        (j.state.queue.iter().take(limit).cloned().collect(), j.state.queue.len(), now)
    };
    let now = if j.host {
        host_now(core, j).map(|n| json!({ "track": n.track, "playing": n.playing, "position_ms": n.position_ms }))
    } else {
        now
    };
    let host_handle = if j.host { j.me.clone() } else { j.state.host.clone() };
    let members: Vec<Value> = j
        .members()
        .into_iter()
        .map(|m| json!({ "handle": m.handle, "host": m.host, "avatar": avatar_url(core, &m.handle, m.avatar.clone()) }))
        .collect();
    let queue: Vec<Value> = items
        .iter()
        .map(|i| json!({ "id": i.id, "track": i.track, "by": i.by, "votes": i.votes, "voted": i.votes.iter().any(|v| *v == j.me) }))
        .collect();
    let knocks: Vec<Value> = j.knocks.iter().map(|(h, a)| json!({ "handle": h, "avatar": avatar_url(core, h, a.clone()) })).collect();
    let feed: Vec<Value> = j.feed().iter().map(feed_json).collect();
    let _ = j.avatar_of("");
    json!({
        "jam": {
            "code": j.code, "host": j.host, "me": j.me, "host_handle": host_handle, "link": j.link(),
            "members": members, "perms": j.perms(), "approve_joins": j.approve_joins(),
            "listen_here": j.listen_here, "unavailable": j.unavailable.is_some(), "host_online": j.host_online(),
            "now": now, "queue": queue, "queue_len": queue_len, "knocks": knocks, "feed": feed, "server_ms": j.server_now(),
        },
        "waiting": waiting,
    })
}

pub fn jam_state(core: C<'_>, queue_all: Option<bool>) -> Value {
    snapshot(&core, queue_all.unwrap_or(false))
}

pub async fn jam_start(core: C<'_>) -> Res {
    let core = core.clone();
    let b = core.backend_or_err()?;
    let me = account_handle(&core)?;
    {
        let mut u = lock(&rt().jam);
        if u.busy {
            drop(u);
            return Ok(snapshot(&core, false));
        }
        u.resume = None;
        u.resume_at = None;
        u.resume_tries = 0;
    }
    if jam_active() {
        leave(&core);
    }
    lock(&rt().jam).busy = true;
    let r = call(b.jam_start()).await;
    lock(&rt().jam).busy = false;
    match r {
        Ok(state) => {
            let code = state.code.clone();
            let mut j = Jam::new(code.clone(), true, me.clone(), state);
            // Always start a new jam with all guest controls enabled by default
            // (even against older servers that return restrictive defaults).
            j.perms = Perms::default();
            // Save the host's own queue and start the jam with an empty jam queue.
            let (saved_q, cleared) = {
                let mut st = core.st();
                let saved = st.queue.clone();
                let had_manual = st.queue.manual_len() > 0;
                if had_manual {
                    st.queue.clear_manual();
                }
                (saved, had_manual)
            };
            if cleared {
                core.queue_changed();
                core.emit_queue();
            }
            save_jam(&code, &me, true, Some(&saved_q));
            j.saved_queue = Some(saved_q);
            lock(&rt().jam).jam = Some(j);
            bump();
            host_loop(&core);
            Ok(snapshot(&core, false))
        }
        Err((c, msg)) => Err(problem(c.as_deref(), &msg)),
    }
}

/// Joins the jam with this code. When the host lets people in by hand, this
/// returns `waiting` and keeps asking until they do.
async fn join(core: &Arc<Core>, text: &str) -> Result<(), String> {
    let Some(code) = glyph_protocol::jam::normalize_code(text) else { return Err("That is not a Jam code.".into()) };
    if lock(&rt().jam).jam.as_ref().is_some_and(|j| j.code == code) {
        return Ok(());
    }
    let b = core.backend_or_err()?;
    let me = account_handle(core)?;
    {
        let mut u = lock(&rt().jam);
        if u.busy {
            return Ok(());
        }
        u.busy = true;
        u.waiting = None;
        u.resume = None;
        u.resume_at = None;
        u.resume_tries = 0;
    }
    if jam_active() {
        leave(core);
    }
    let gen = bump();
    let r = call(b.jam_join(&code)).await;
    lock(&rt().jam).busy = false;
    match r {
        Ok(res) if res.waiting => {
            lock(&rt().jam).waiting = Some(code.clone());
            knock_loop(core.clone(), code, me, gen);
            Ok(())
        }
        Ok(res) => match res.state {
            // Own jam (another device of this account): not as a guest.
            Some(state) if state.host == me => Err("That is your own Jam.".into()),
            Some(state) => {
                enter_guest(core, code, me, state, true, None, false);
                Ok(())
            }
            None => Err("not_found".into()),
        },
        Err((c, msg)) => Err(problem(c.as_deref(), &msg)),
    }
}

/// Asks again every few seconds until the host lets us in (or says no).
fn knock_loop(core: Arc<Core>, code: String, me: String, gen: u64) {
    crate::rt::spawn(async move {
        loop {
            tokio::time::sleep(KNOCK_EVERY).await;
            if cur_gen() != gen {
                return;
            }
            let Some(b) = core.backend() else { return };
            let r = call(b.jam_join(&code)).await;
            if cur_gen() != gen {
                return;
            }
            match r {
                Ok(res) if res.waiting => continue,
                Ok(res) => {
                    lock(&rt().jam).waiting = None;
                    match res.state {
                        Some(state) if state.host == me => toast_info(&core, "jam.err.own"),
                        Some(state) => enter_guest(&core, code.clone(), me.clone(), state, true, None, true),
                        None => toast_info(&core, "jam.err.server"),
                    }
                    return;
                }
                Err((c, msg)) => {
                    lock(&rt().jam).waiting = None;
                    toast(&core, &problem_key(&problem(c.as_deref(), &msg)), Value::Null, None, "info");
                    return;
                }
            }
        }
    });
}

pub async fn jam_join(core: C<'_>, code: String) -> Res {
    let core = core.clone();
    join(&core, &code).await?;
    Ok(snapshot(&core, false))
}

/// In as a guest: the queue shows the host's (the own one comes back when
/// leaving), playback follows the host.
fn enter_guest(core: &Arc<Core>, code: String, me: String, state: State, listen_here: bool, saved_queue: Option<Queue>, say: bool) {
    let host = state.host.clone();
    let mut j = Jam::new(code.clone(), false, me.clone(), state);
    j.listen_here = listen_here;
    let saved_q = saved_queue.unwrap_or_else(|| core.st().queue.clone());
    save_jam(&code, &me, listen_here, Some(&saved_q));
    j.saved_queue = Some(saved_q);
    {
        let mut u = lock(&rt().jam);
        u.jam = Some(j);
        u.waiting = None;
        u.resume = None;
        u.resume_at = None;
        u.resume_tries = 0;
    }
    bump();
    mirror_queue(core);
    guest_loop(core);
    sync(core);
    emit_jam(core);
    if say {
        toast(core, "jam.joined_you", Value::Null, Some(format!("@{host}")), "ok");
    }
}

/// Resumes an active jam as its host after the app restarted.
fn resume_host(core: &Arc<Core>, me: String, state: State, saved_queue: Option<Queue>) {
    let code = state.code.clone();
    let mut j = Jam::new(code.clone(), true, me.clone(), state.clone());
    j.guests = state.members.iter().filter(|m| !m.host && m.handle != j.me).map(|m| m.handle.clone()).collect();
    for item in &state.queue {
        j.by.insert(item.id.clone(), item.by.clone());
        if !item.votes.is_empty() {
            j.votes.insert(item.id.clone(), item.votes.iter().cloned().collect());
        }
    }
    let restore_queue = {
        let mut st = core.st();
        if st.queue.manual_len() == 0 && !state.queue.is_empty() {
            let manual: Vec<Track> = state.queue.iter().map(|i| i.track.clone()).collect();
            let after: Vec<Track> = if st.queue.upcoming().is_empty() {
                state.after.clone()
            } else {
                st.queue.upcoming_context().into_iter().cloned().collect()
            };
            guard(|| st.queue.mirror(manual, after));
            true
        } else {
            for tr in st.queue.upcoming().into_iter().take(st.queue.manual_len()) {
                j.by.entry(track_key(tr)).or_insert_with(|| j.me.clone());
            }
            false
        }
    };
    if restore_queue {
        core.queue_changed();
        core.emit_queue();
    }
    let restore_now = {
        let st = core.st();
        st.current.is_none().then(|| state.now.clone()).flatten()
    };
    save_jam(&code, &me, true, saved_queue.as_ref());
    j.saved_queue = saved_queue;
    if let Some(n) = restore_now {
        j.resume_at = state.position_at(j.server_now());
        let track = n.resolved.unwrap_or(n.track);
        lock(&rt().jam).jam = Some(j);
        bump();
        core.start_track(track);
        rt().hold.store(true, Ordering::Relaxed);
    } else {
        lock(&rt().jam).jam = Some(j);
        bump();
    }
    host_loop(core);
    emit_jam(core);
}

/// Checks if the saved jam from the previous session is still live and rejoins it.
fn resume_saved(core: &Arc<Core>, b: Arc<Backend>, saved: SavedJam, gen: u64) {
    let core = core.clone();
    crate::rt::spawn(async move {
        let r = call(b.jam_state(&saved.code, 0)).await;
        enum Action {
            Host(State),
            Guest(State),
            Clear,
            None,
        }
        let act = {
            let mut u = lock(&rt().jam);
            u.resuming = false;
            if cur_gen() != gen
                || u.jam.is_some()
                || u.waiting.is_some()
                || u.resume.as_ref().map(|s| &s.code) != Some(&saved.code)
            {
                Action::None
            } else {
                match r {
                    Ok(state) => {
                        u.resume = None;
                        u.resume_at = None;
                        u.resume_tries = 0;
                        if state.host == saved.handle {
                            Action::Host(state)
                        } else {
                            Action::Guest(state)
                        }
                    }
                    Err((c, _)) if gone(c.as_deref()) => {
                        u.resume = None;
                        u.resume_at = None;
                        u.resume_tries = 0;
                        Action::Clear
                    }
                    Err((_, msg)) => {
                        tracing::info!("jam resume: {msg}");
                        u.resume_tries = u.resume_tries.saturating_add(1);
                        let delay = (1u64 << u.resume_tries.min(5)).min(30);
                        u.resume_at = Some(Instant::now() + Duration::from_secs(delay));
                        Action::None
                    }
                }
            }
        };
        let saved_q = queue_from_snapshot(saved.saved_queue);
        match act {
            Action::Host(state) => resume_host(&core, saved.handle, state, saved_q),
            Action::Guest(state) => enter_guest(&core, state.code.clone(), saved.handle, state, saved.listen_here, saved_q, false),
            Action::Clear => {
                clear_saved_jam();
                // The jam ended while closed: restore pre-jam queue so any
                // host-persisted jam queue does not linger.
                if let Some(q) = saved_q {
                    let should_restore = core.st().queue.manual_len() > 0;
                    if should_restore {
                        core.st().queue = q;
                        core.queue_changed();
                        core.emit_queue();
                    }
                }
            }
            Action::None => {}
        }
    });
}

pub fn jam_leave(core: C<'_>, code: Option<String>) {
    let waiting = lock(&rt().jam).waiting.clone();
    if waiting.is_some() && (code.is_none() || code == waiting) && !jam_active() {
        // Stops waiting to be let in.
        lock(&rt().jam).waiting = None;
        bump();
        if let (Some(b), Some(code)) = (core.backend(), waiting) {
            crate::rt::spawn(async move {
                let _ = call(b.jam_leave(&code)).await;
            });
        }
        return;
    }
    leave(core);
}

/// Leaves the jam; the host ends it for everyone.
fn leave(core: &Core) {
    let Some(j) = lock(&rt().jam).jam.take() else { return };
    bump();
    clear_saved_jam();
    let (code, host) = (j.code.clone(), j.host);
    after(core, j);
    if let Some(b) = core.backend() {
        crate::rt::spawn(async move {
            let _ = call(b.jam_leave(&code)).await;
        });
    }
    toast_info(core, if host { "jam.ended_you" } else { "jam.left" });
    emit_jam(core);
}

/// The jam is gone for this app (ended, removed): say why.
fn lost(core: &Core, why: String) {
    let Some(j) = lock(&rt().jam).jam.take() else { return };
    bump();
    clear_saved_jam();
    after(core, j);
    toast(core, &problem_key(&why), Value::Null, None, "info");
    emit_jam(core);
}

/// Someone else took over this app's jam: back in as a guest.
fn demote(core: &Arc<Core>) {
    let Some(j) = lock(&rt().jam).jam.take() else { return };
    bump();
    let code = j.code.clone();
    after(core, j);
    toast_info(core, "jam.taken_over");
    emit_jam(core);
    let core = core.clone();
    crate::rt::spawn(async move {
        if let Err(e) = join(&core, &code).await {
            if gone(Some(e.as_str())) {
                clear_saved_jam();
            }
            toast(&core, &problem_key(&e), Value::Null, None, "info");
        }
    });
}

/// Back to normal after a jam: tempo, the own queue (never keeping the jam
/// queue), nothing shown that is not loaded here.
fn after(core: &Core, j: Jam) {
    if !j.host {
        if (j.speed - 1.0).abs() > f32::EPSILON {
            if let Some(p) = &core.player {
                p.set_speed(1.0);
            }
        }
        if !j.listen_here {
            clear_remote(core);
        }
    }
    if let Some(q) = j.saved_queue {
        core.st().queue = q;
        core.queue_changed();
        core.emit_queue();
    } else if core.st().queue.manual_len() > 0 {
        core.st().queue.clear_manual();
        core.queue_changed();
        core.emit_queue();
    }
}

/// The host's song shown while listening at the host is not loaded here:
/// forget it.
fn clear_remote(core: &Core) {
    {
        let mut st = core.st();
        st.current = None;
        st.restored_pos = None;
        st.restored_duration = None;
    }
    *lock(&rt().shown) = None;
    core.emit_player();
}

// ------------------------------------------------------------ jam: host settings and members

pub fn jam_set_perms(core: C<'_>, perms: Perms) -> Value {
    if let Some(j) = lock(&rt().jam).jam.as_mut().filter(|j| j.host) {
        j.perms = perms;
    }
    snapshot(&core, false)
}

pub fn jam_set_approve(core: C<'_>, on: bool) -> Value {
    if let Some(j) = lock(&rt().jam).jam.as_mut().filter(|j| j.host) {
        j.approve = on;
    }
    snapshot(&core, false)
}

/// Host: lets someone in who knocked, or not.
pub fn jam_admit(core: C<'_>, handle: String, allow: bool) -> Value {
    let code = {
        let mut u = lock(&rt().jam);
        u.jam.as_mut().filter(|j| j.host).map(|j| {
            j.knocks.retain(|(h, _)| *h != handle);
            j.code.clone()
        })
    };
    if let (Some(code), Some(b)) = (code, core.backend()) {
        // Let in: the server sends the usual `Join`.
        let h = handle.clone();
        crate::rt::spawn(async move {
            if let Err((_, e)) = call(b.jam_admit(&code, &h, allow)).await {
                tracing::info!("jam admit: {e}");
            }
        });
    }
    snapshot(&core, false)
}

/// Host: removes someone from the jam (they cannot join again).
pub fn jam_kick(core: C<'_>, handle: String) -> Value {
    let code = {
        let mut u = lock(&rt().jam);
        u.jam.as_mut().filter(|j| j.host).map(|j| {
            j.guests.retain(|g| *g != handle);
            j.code.clone()
        })
    };
    if let (Some(code), Some(b)) = (code, core.backend()) {
        let h = handle.clone();
        crate::rt::spawn(async move {
            if let Err((_, e)) = call(b.jam_kick(&code, &h)).await {
                tracing::info!("jam kick: {e}");
            }
        });
    }
    snapshot(&core, false)
}

/// Votes for (or takes the vote back from) a queued song.
pub fn jam_vote(core: C<'_>, id: String) -> Value {
    enum Do {
        Host(String, bool),
        Nothing,
        NotAllowed,
    }
    let act = {
        let mut u = lock(&rt().jam);
        match u.jam.as_mut() {
            None => Do::Nothing,
            Some(j) if j.host => {
                let up = !j.votes.get(&id).is_some_and(|v| v.contains(&j.me));
                Do::Host(j.me.clone(), up)
            }
            Some(j) if j.state.perms.add => {
                // Show it at once; the host's next state confirms it.
                let me = j.me.clone();
                let mut up = true;
                if let Some(i) = j.state.queue.iter_mut().find(|i| i.id == id) {
                    up = !i.votes.contains(&me);
                    if up {
                        i.votes.push(me);
                    } else {
                        i.votes.retain(|v| *v != me);
                    }
                }
                j.outbox.push(Op::Vote { item: id.clone(), up });
                Do::Nothing
            }
            Some(_) => Do::NotAllowed,
        }
    };
    match act {
        Do::Host(me, up) => host_vote(&core, &id, &me, up),
        Do::NotAllowed => toast_info(&core, "jam.not_allowed"),
        Do::Nothing => {}
    }
    snapshot(&core, false)
}

/// Takes a song out of the jam queue (host: any, guest: own).
pub fn jam_remove(core: C<'_>, id: String) -> Value {
    remove_item(&core, &id);
    snapshot(&core, false)
}

fn remove_item(core: &Core, id: &str) {
    enum Do {
        Host,
        Guest,
        NotAllowed,
        Nothing,
    }
    let act = {
        let mut u = lock(&rt().jam);
        match u.jam.as_mut() {
            None => Do::Nothing,
            Some(j) if j.host => Do::Host,
            Some(j) if j.state.queue.iter().any(|i| i.id == id && i.by == j.me) => {
                j.state.queue.retain(|i| i.id != id);
                j.outbox.push(Op::Remove { item: id.to_string() });
                Do::Guest
            }
            Some(_) => Do::NotAllowed,
        }
    };
    match act {
        Do::Host => host_remove(core, id),
        Do::Guest => mirror_queue(core),
        Do::NotAllowed => toast_info(core, "jam.not_allowed"),
        Do::Nothing => {}
    }
}

/// A reaction to the song playing.
pub fn jam_react(core: C<'_>, reaction: Reaction) -> Value {
    if let Some(j) = lock(&rt().jam).jam.as_mut() {
        if !j.reacted.get(&j.me).is_some_and(|at| at.elapsed() < REACT_GAP) {
            j.reacted.insert(j.me.clone(), Instant::now());
            if j.host {
                let me = j.me.clone();
                j.event(&me, What::React { reaction });
            } else {
                j.outbox.push(Op::React { reaction });
            }
        }
    }
    snapshot(&core, false)
}

/// Guest: the music plays here, or only at the host's device.
pub fn jam_set_listen_here(core: C<'_>, here: bool) -> Value {
    let core = core;
    let changed = {
        let mut u = lock(&rt().jam);
        match u.jam.as_mut().filter(|j| !j.host && j.listen_here != here) {
            Some(j) => {
                j.listen_here = here;
                j.synced_at = None;
                j.settle_until = None;
                j.speed = 1.0;
                Some((j.code.clone(), j.me.clone(), j.saved_queue.clone()))
            }
            None => None,
        }
    };
    if let Some((code, me, saved_q)) = changed {
        save_jam(&code, &me, here, saved_q.as_ref());
        if here {
            // `sync` starts the host's song here.
            clear_remote(core);
        } else if !remote_send(Cmd::Pause(true)) {
            if let Some(p) = &core.player {
                p.set_speed(1.0);
                p.stop();
            }
        }
        sync(core);
    }
    snapshot(core, false)
}

// ------------------------------------------------------------ jam: loops

/// Host: picks up the friends' requests until the jam ends, pushed where
/// the server can, else by long polling.
fn host_loop(core: &Arc<Core>) {
    let Some(b) = core.backend() else { return };
    let Some(code) = lock(&rt().jam).jam.as_ref().map(|j| j.code.clone()) else { return };
    let (core, gen) = (core.clone(), cur_gen());
    crate::rt::spawn(async move {
        let mut failures = 0u32;
        let mut live_at = Some(Instant::now());
        loop {
            if cur_gen() != gen {
                break;
            }
            if live_at.is_some_and(|t| t <= Instant::now()) {
                match host_live(&core, &b, gen, &code).await {
                    LiveEnd::Stop => break,
                    LiveEnd::Again => continue,
                    LiveEnd::Later => live_at = Some(Instant::now() + LIVE_RETRY),
                    LiveEnd::Never => live_at = None,
                }
            }
            let r = call(b.jam_ops(&code, false)).await;
            match host_ops(&core, gen, r) {
                Asked::Done => failures = 0,
                Asked::Over => break,
                Asked::Failed => {
                    failures += 1;
                    tokio::time::sleep(Duration::from_secs((1u64 << failures.min(5)).min(30))).await;
                }
            }
        }
    });
}

fn set_live(gen: u64, on: bool) {
    if cur_gen() != gen {
        return;
    }
    if let Some(j) = lock(&rt().jam).jam.as_mut() {
        j.live = on;
    }
}

/// Host on push: asks for the requests when the server rings, and every
/// [`LIVE_HOST_EVERY_MS`] anyway.
async fn host_live(core: &Arc<Core>, b: &Backend, gen: u64, code: &str) -> LiveEnd {
    let mut rx = match call(b.jam_live(code)).await {
        Ok(rx) => rx,
        Err((c, msg)) => {
            tracing::info!("jam live: {msg}");
            return LiveEnd::refused(c.as_deref());
        }
    };
    set_live(gen, true);
    let end = loop {
        let tick = tokio::time::sleep(Duration::from_millis(LIVE_HOST_EVERY_MS));
        let ev = match select(Box::pin(rx.recv()), Box::pin(tick)).await {
            Either::Left((ev, _)) => ev,
            Either::Right(_) => Some(LiveEvent::Msg(Live::Ops)),
        };
        match ev {
            None => break LiveEnd::Later,
            Some(LiveEvent::Closed(c)) => break LiveEnd::closed(c),
            Some(LiveEvent::Msg(Live::Fallback)) => break LiveEnd::Never,
            Some(LiveEvent::Msg(Live::Ops)) => {
                let r = call(b.jam_ops(code, true)).await;
                if host_ops(core, gen, r) == Asked::Over {
                    break LiveEnd::Stop;
                }
            }
            Some(LiveEvent::Msg(_)) => {}
        }
    };
    set_live(gen, false);
    end
}

/// The host's answer to asking for requests: runs them.
fn host_ops(core: &Arc<Core>, gen: u64, r: Result<OpsResponse, (Option<String>, String)>) -> Asked {
    if cur_gen() != gen {
        return Asked::Over;
    }
    match r {
        Ok(r) => {
            if !r.ops.is_empty() {
                apply_ops(core, r.ops);
            }
            Asked::Done
        }
        Err((c, _)) if c.as_deref() == Some("not_jam_host") => {
            demote(core);
            Asked::Over
        }
        Err((c, msg)) if gone(c.as_deref()) => {
            lost(core, problem(c.as_deref(), &msg));
            Asked::Over
        }
        Err((_, msg)) => {
            tracing::info!("jam: {msg}");
            Asked::Failed
        }
    }
}

/// Guest: follows the published state until the jam ends, pushed where the
/// server can, else by long polling.
fn guest_loop(core: &Arc<Core>) {
    let Some(b) = core.backend() else { return };
    let Some((code, mut rev)) = lock(&rt().jam).jam.as_ref().map(|j| (j.code.clone(), j.state.rev)) else { return };
    let (core, gen) = (core.clone(), cur_gen());
    crate::rt::spawn(async move {
        let mut failures = 0u32;
        let mut live_at = Some(Instant::now());
        loop {
            if cur_gen() != gen {
                break;
            }
            if live_at.is_some_and(|t| t <= Instant::now()) {
                match guest_live(&core, &b, gen, &code, &mut rev).await {
                    LiveEnd::Stop => break,
                    LiveEnd::Again => continue,
                    LiveEnd::Later => live_at = Some(Instant::now() + LIVE_RETRY),
                    LiveEnd::Never => live_at = None,
                }
            }
            let r = call(b.jam_state(&code, rev)).await;
            match guest_got(&core, gen, r, &mut rev) {
                Asked::Done => failures = 0,
                Asked::Over => break,
                Asked::Failed => {
                    failures += 1;
                    tokio::time::sleep(Duration::from_secs((1u64 << failures.min(5)).min(30))).await;
                }
            }
        }
    });
}

/// Guest on push. While the host is not on push too (an older app, or
/// gone), it also long polls: that is how it learns that the host is
/// offline and when to take over.
async fn guest_live(core: &Arc<Core>, b: &Backend, gen: u64, code: &str, rev: &mut u64) -> LiveEnd {
    let mut rx = match call(b.jam_live(code)).await {
        Ok(rx) => rx,
        Err((c, msg)) => {
            tracing::info!("jam live: {msg}");
            return LiveEnd::refused(c.as_deref());
        }
    };
    // What was published before the socket was open.
    let r = call(b.jam_state(code, 0)).await;
    if guest_got(core, gen, r, rev) == Asked::Over {
        return LiveEnd::Stop;
    }
    let mut host_live = false;
    loop {
        let ev = if host_live {
            rx.recv().await
        } else {
            match select(Box::pin(rx.recv()), Box::pin(call(b.jam_state(code, *rev)))).await {
                Either::Left((ev, _)) => ev,
                Either::Right((r, _)) => {
                    match guest_got(core, gen, r, rev) {
                        Asked::Over => return LiveEnd::Stop,
                        Asked::Failed => tokio::time::sleep(Duration::from_secs(5)).await,
                        Asked::Done => {}
                    }
                    continue;
                }
            }
        };
        match ev {
            None => return LiveEnd::Later,
            Some(LiveEvent::Closed(c)) => return LiveEnd::closed(c),
            Some(LiveEvent::Msg(Live::Fallback)) => return LiveEnd::Never,
            Some(LiveEvent::Msg(Live::State { state })) => {
                if guest_got(core, gen, Ok(*state), rev) == Asked::Over {
                    return LiveEnd::Stop;
                }
            }
            Some(LiveEvent::Msg(Live::Host { online })) => {
                host_live = online;
                if online && cur_gen() == gen {
                    if let Some(j) = lock(&rt().jam).jam.as_mut() {
                        j.state.host_online = true;
                        j.host_gone = None;
                    }
                }
            }
            Some(LiveEvent::Msg(Live::Now { rev: r, now, server_ms })) => {
                if guest_now(core, gen, r, now, server_ms, rev) == Asked::Over {
                    return LiveEnd::Stop;
                }
            }
            Some(LiveEvent::Msg(Live::Ops)) => {}
        }
    }
}

/// Pushed: only the host's position moved (revision `r`).
fn guest_now(core: &Arc<Core>, gen: u64, r: u64, now: Now, server_ms: u64, rev: &mut u64) -> Asked {
    if cur_gen() != gen {
        return Asked::Over;
    }
    {
        let mut u = lock(&rt().jam);
        let Some(j) = u.jam.as_mut() else { return Asked::Over };
        if r <= *rev {
            return Asked::Done;
        }
        *rev = r;
        j.offset_ms = server_ms as i64 - unix_ms() as i64;
        if j.state.now.as_ref().map(|n| track_key(&n.track)) != Some(track_key(&now.track)) {
            j.unavailable = None;
        }
        j.state.rev = r;
        j.state.now = Some(now);
        j.state.host_online = true;
        j.host_gone = None;
        j.synced_at = None;
    }
    sync(core);
    Asked::Done
}

/// A guest's answer about the state: shows it.
fn guest_got(core: &Arc<Core>, gen: u64, r: Result<State, (Option<String>, String)>, rev: &mut u64) -> Asked {
    if cur_gen() != gen {
        return Asked::Over;
    }
    match r {
        Ok(state) => {
            // An older state than the one shown (a slow answer) changes nothing.
            if state.rev >= *rev {
                *rev = state.rev;
                apply_state(core, state);
            }
            Asked::Done
        }
        Err((c, msg)) if gone(c.as_deref()) => {
            lost(core, problem(c.as_deref(), &msg));
            Asked::Over
        }
        Err((_, msg)) => {
            tracing::info!("jam: {msg}");
            Asked::Failed
        }
    }
}

fn apply_state(core: &Arc<Core>, state: State) {
    let fresh: Vec<(String, Reaction)>;
    {
        let mut u = lock(&rt().jam);
        let Some(j) = u.jam.as_mut() else { return };
        j.offset_ms = state.server_ms as i64 - unix_ms() as i64;
        // Reactions of others since the last state pop up.
        fresh = state
            .feed
            .iter()
            .filter(|e| e.at_ms > j.feed_seen && e.who != j.me)
            .filter_map(|e| match e.what {
                What::React { reaction } => Some((e.who.clone(), reaction)),
                _ => None,
            })
            .collect();
        j.feed_seen = state.feed.iter().map(|e| e.at_ms).max().unwrap_or(j.feed_seen);
        if state.host_online {
            j.host_gone = None;
        } else if j.host_gone.is_none() {
            j.host_gone = Some(Instant::now());
        }
        let new_song = j.state.now.as_ref().map(|n| track_key(&n.track)) != state.now.as_ref().map(|n| track_key(&n.track));
        if new_song {
            j.unavailable = None;
        }
        j.state = state;
        j.synced_at = None;
    }
    for (who, r) in fresh {
        toast(core, reaction_key(r), Value::Null, Some(format!("@{who}")), "info");
    }
    mirror_queue(core);
    sync(core);
}

// ------------------------------------------------------------ jam: every 100 ms

fn jam_tick(core: &Arc<Core>) {
    let pending = {
        let mut u = lock(&rt().jam);
        if u.pending.is_some() && core.backend().is_some() && !u.busy {
            u.resume = None;
            u.resume_at = None;
            u.resume_tries = 0;
            u.pending.take()
        } else {
            None
        }
    };
    if let Some(link) = pending {
        open_link(core, &link);
    }
    let to_resume = {
        let mut u = lock(&rt().jam);
        if u.jam.is_none()
            && u.waiting.is_none()
            && !u.busy
            && !u.resuming
            && u.resume.is_some()
            && u.resume_at.is_none_or(|t| Instant::now() >= t)
        {
            if let Some(b) = core.backend() {
                if let Some(acc) = b.account() {
                    let saved = u.resume.clone().unwrap();
                    if acc.account.handle != saved.handle {
                        u.resume = None;
                        u.resume_at = None;
                        u.resume_tries = 0;
                        Some(Err(()))
                    } else {
                        u.resuming = true;
                        Some(Ok((b, saved, cur_gen())))
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };
    match to_resume {
        Some(Ok((b, saved, gen))) => resume_saved(core, b, saved, gen),
        Some(Err(())) => clear_saved_jam(),
        None => {}
    }
    let (host, clock_due) = {
        let u = lock(&rt().jam);
        let Some(j) = u.jam.as_ref() else { return };
        let due = !j.clock_busy && j.clock_at.is_none_or(|t| t.elapsed() >= if j.clock.len() < CLOCK_FIRST { CLOCK_QUICK } else { CLOCK_EVERY });
        (j.host, due)
    };
    if clock_due {
        clock_sample(core);
    }
    if host {
        resume(core);
        maybe_publish(core);
        return;
    }
    let (flush, do_sync, take_over) = {
        let u = lock(&rt().jam);
        let Some(j) = u.jam.as_ref() else { return };
        (
            !j.sending && !j.outbox.is_empty(),
            j.synced_at.is_none_or(|t| t.elapsed() >= SYNC_EVERY),
            !j.claiming
                && j.host_gone.is_some_and(|since| {
                    let offline = HOST_TIMEOUT_MS + since.elapsed().as_millis() as u64;
                    glyph_core::jam::should_take_over(&j.state.members, &j.me, offline)
                }),
        )
    };
    if flush {
        flush_outbox(core);
    }
    if do_sync {
        sync(core);
    }
    if take_over {
        take_over_host(core);
    }
}

/// One timed request for the clock sync.
fn clock_sample(core: &Arc<Core>) {
    let Some(b) = core.backend() else { return };
    let code = {
        let mut u = lock(&rt().jam);
        let Some(j) = u.jam.as_mut() else { return };
        j.clock_busy = true;
        j.clock_at = Some(Instant::now());
        j.code.clone()
    };
    let gen = cur_gen();
    crate::rt::spawn(async move {
        let r = call(b.jam_time(&code)).await;
        if cur_gen() != gen {
            return;
        }
        if let Some(j) = lock(&rt().jam).jam.as_mut() {
            j.clock_busy = false;
            if let Ok((sent, server, recv)) = r {
                j.clock.add(sent, server, recv);
            }
        }
    });
}

// ------------------------------------------------------------ jam: host

/// Fingerprint of what the host would publish (without the position): what
/// plays and from where, the queue with its votes, what comes after, who is
/// in, the feed and the settings.
fn host_sig(core: &Core, j: &Jam) -> u64 {
    use std::hash::{BuildHasher, Hash, Hasher};
    fn song(t: &Track, h: &mut impl Hasher) {
        t.source.hash(h);
        t.id.hash(h);
    }
    // Order-free sum over a map's entries.
    fn map_sum<K: Hash, V: Hash>(m: &HashMap<K, V>) -> u64 {
        let b = std::hash::BuildHasherDefault::<std::collections::hash_map::DefaultHasher>::default();
        m.iter().map(|e| b.hash_one(e)).fold(0u64, u64::wrapping_add)
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let load = last_load();
    let playing = is_playing(core);
    let hold = rt().hold.load(Ordering::Relaxed);
    let st = core.st();
    if let Some(c) = &st.current {
        song(c, &mut h);
        (playing || (st.resolving && !hold)).hash(&mut h);
        if let Some((tr, _)) = load.as_ref().filter(|(tr, _)| same_song(c, tr)) {
            song(tr, &mut h);
        }
    }
    let n = st.queue.manual_len().min(MAX_QUEUE);
    for t in st.queue.upcoming().into_iter().take(n) {
        song(t, &mut h);
    }
    map_sum(&j.votes).hash(&mut h);
    for t in st.queue.upcoming_context().into_iter().take(MAX_AFTER) {
        song(t, &mut h);
    }
    j.guests.hash(&mut h);
    map_sum(&j.avatars).hash(&mut h);
    j.feed.last().map(|e| e.at_ms).hash(&mut h);
    j.feed.len().hash(&mut h);
    (j.perms.add, j.perms.play_now, j.perms.skip, j.perms.seek, j.perms.pause, j.approve).hash(&mut h);
    h.finish()
}

fn host_state(core: &Core, j: &Jam) -> State {
    let now = host_now(core, j);
    let st = core.st();
    State {
        code: j.code.clone(),
        rev: j.state.rev,
        host: j.me.clone(),
        members: j.members(),
        perms: j.perms,
        approve_joins: j.approve,
        now,
        queue: host_items(&st.queue, j, MAX_QUEUE),
        after: st.queue.upcoming_context().into_iter().take(MAX_AFTER).cloned().collect(),
        feed: j.feed.clone(),
        host_online: true,
        server_ms: 0,
        link: j.state.link.clone(),
    }
}

/// After a takeover: the song continues where the old host was.
fn resume(core: &Core) {
    let Some(at) = lock(&rt().jam).jam.as_ref().and_then(|j| j.resume_at) else { return };
    let (resolving, has) = {
        let st = core.st();
        (st.resolving, st.current.is_some())
    };
    if resolving || !has || player_state(core).is_none_or(|s| s.status == Status::Loading) {
        return;
    }
    if let Some(j) = lock(&rt().jam).jam.as_mut() {
        j.resume_at = None;
    }
    core.seek(at as f64 / 1000.0);
}

/// Publishes when something changed, after a seek, or as a heartbeat.
/// Looks every [`PUBLISH_CHECK`]; the state itself is only built when it
/// goes out.
fn maybe_publish(core: &Arc<Core>) {
    let Some(b) = core.backend() else { return };
    let gen = cur_gen();
    let (playing, pos, has) = {
        let (resolving, has, restored_pos) = {
            let st = core.st();
            (st.resolving, st.current.is_some(), st.restored_pos)
        };
        let hold = rt().hold.load(Ordering::Relaxed);
        let playing = has && (is_playing(core) || (resolving && !hold));
        let p = if has {
            if !playing && !resolving {
                restored_pos.unwrap_or_else(|| position(core)).as_millis() as u64
            } else {
                position(core).as_millis() as u64
            }
        } else {
            0
        };
        (playing, p, has)
    };
    let _ = has;
    let mut u = lock(&rt().jam);
    let Some(j) = u.jam.as_mut().filter(|j| j.host) else { return };
    if j.publishing || j.checked.is_some_and(|t| t.elapsed() < PUBLISH_CHECK) {
        return;
    }
    j.checked = Some(Instant::now());
    let sig = host_sig(core, j);
    let due = match j.published {
        None => true,
        Some((s, _, _)) if s != sig => true,
        Some((_, p, at)) => {
            let expected = p as i64 + if playing { at.elapsed().as_millis() as i64 } else { 0 };
            (pos as i64 - expected).abs() > SEEK_JUMP_MS || at.elapsed() >= if j.live { HEARTBEAT_LIVE } else { HEARTBEAT }
        }
    };
    if !due {
        return;
    }
    // Only the position moved (a seek, or the heartbeat): send just that.
    let light = j.light && j.published.is_some_and(|(s, _, _)| s == sig);
    if light {
        let now = host_now(core, j);
        j.published = Some((sig, pos, Instant::now()));
        // Nothing plays: nothing moves.
        let Some(now) = now else { return };
        j.publishing = true;
        j.state.now = Some(now.clone());
        let code = j.code.clone();
        drop(u);
        let core = core.clone();
        crate::rt::spawn(async move {
            let r = call(b.jam_now(&code, now)).await;
            published(&core, gen, r);
        });
        return;
    }
    let state = host_state(core, j);
    j.publishing = true;
    j.published = Some((sig, pos, Instant::now()));
    j.state = state.clone();
    drop(u);
    let core = core.clone();
    crate::rt::spawn(async move {
        let r = call(b.jam_publish(state)).await;
        published(&core, gen, r);
    });
}

/// A publish came back.
fn published(core: &Arc<Core>, gen: u64, r: Result<(), (Option<String>, String)>) {
    if cur_gen() != gen {
        return;
    }
    if let Some(j) = lock(&rt().jam).jam.as_mut() {
        j.publishing = false;
        if let Err((c, _)) = &r {
            // An older server without `/now`: whole states from now on.
            if matches!(c.as_deref(), Some("not_found" | "method_not_allowed")) {
                j.light = false;
            }
            // Try again on a later check.
            j.published = None;
        }
    }
    match r {
        Err((c, _)) if c.as_deref() == Some("not_jam_host") => demote(core),
        Err((c, msg)) if gone(c.as_deref()) => lost(core, problem(c.as_deref(), &msg)),
        Err((_, msg)) => tracing::info!("jam publish: {msg}"),
        Ok(()) => {}
    }
}

/// What a friend's request makes the host do (after the jam state changed).
enum Act {
    None,
    Add(Track),
    PlayNow(Track),
    Vote(String, bool),
    Remove(String),
    Control(Control),
}

/// Runs the friends' requests, as far as the host allows them.
fn apply_ops(core: &Arc<Core>, ops: Vec<OpFrom>) {
    for OpFrom { from, op, avatar } in ops {
        let act = {
            let mut u = lock(&rt().jam);
            let Some(j) = u.jam.as_mut().filter(|j| j.host) else { return };
            let member = j.guests.contains(&from);
            let from_server = matches!(op, Op::Join | Op::Leave | Op::Knock);
            if (!member && !from_server) || !j.perms.allows_op(&op) {
                continue;
            }
            match op {
                Op::Join => {
                    if let Some(a) = avatar {
                        j.avatars.insert(from.clone(), a);
                    }
                    j.knocks.retain(|(h, _)| *h != from);
                    if !member && j.guests.len() + 1 < MAX_MEMBERS && from != j.me {
                        j.guests.push(from.clone());
                        j.event(&from, What::Joined);
                        toast(core, "jam.joined", Value::Null, Some(format!("@{from}")), "info");
                    }
                    Act::None
                }
                Op::Leave => {
                    if member {
                        j.guests.retain(|g| *g != from);
                        j.event(&from, What::Left);
                        toast(core, "jam.left_them", Value::Null, Some(format!("@{from}")), "info");
                    }
                    Act::None
                }
                Op::Knock => {
                    if !j.knocks.iter().any(|(h, _)| *h == from) {
                        j.knocks.push((from.clone(), avatar));
                        toast(core, "jam.knock", Value::Null, Some(format!("@{from}")), "info");
                    }
                    Act::None
                }
                Op::Add { track } => Act::Add(track),
                Op::PlayNow { track } => Act::PlayNow(track),
                Op::Vote { item, up } => Act::Vote(item, up),
                Op::Remove { item } => {
                    if j.by.get(&item) == Some(&from) {
                        Act::Remove(item)
                    } else {
                        Act::None
                    }
                }
                Op::Control { control } => Act::Control(control),
                Op::React { reaction } => {
                    if j.reacted.get(&from).is_none_or(|at| at.elapsed() >= REACT_GAP) {
                        j.reacted.insert(from.clone(), Instant::now());
                        j.event(&from, What::React { reaction });
                        toast(core, reaction_key(reaction), Value::Null, Some(format!("@{from}")), "info");
                    }
                    Act::None
                }
            }
        };
        match act {
            Act::None => {}
            Act::Add(track) => host_add(core, track, &from),
            Act::PlayNow(track) => host_play_now(core, track, &from),
            Act::Vote(item, up) => host_vote(core, &item, &from, up),
            Act::Remove(item) => host_remove(core, &item),
            Act::Control(c) => run_control(core, c),
        }
    }
}

fn allowed(core: &Core, t: &Track) -> bool {
    // The same rule as every other way into the queue (explicit, offline).
    core.allowed(t)
}

/// A friend's song goes to the end of the jam queue (a song already queued
/// gets their vote instead).
fn host_add(core: &Core, track: Track, from: &str) {
    if !allowed(core, &track) {
        return;
    }
    let id = track_key(&track);
    let queued: Vec<String> = {
        let st = core.st();
        st.queue.upcoming().into_iter().take(st.queue.manual_len()).map(track_key).collect()
    };
    if queued.contains(&id) {
        host_vote(core, &id, from, true);
        return;
    }
    if queued.len() >= MAX_QUEUE {
        return;
    }
    let title = track.title.clone();
    if guard(|| core.st().queue.push(track)).is_none() {
        return;
    }
    core.queue_changed();
    core.emit_queue();
    if let Some(j) = lock(&rt().jam).jam.as_mut() {
        j.by.insert(id, from.to_string());
        j.event(from, What::Added { title: title.clone() });
    }
    toast(core, "jam.added", json!({ "who": format!("@{from}") }), Some(title), "info");
}

/// A friend's song plays right away.
fn host_play_now(core: &Core, track: Track, from: &str) {
    if !allowed(core, &track) {
        return;
    }
    let title = track.title.clone();
    if let Some(j) = lock(&rt().jam).jam.as_mut() {
        j.by.insert(track_key(&track), from.to_string());
        j.event(from, What::Played { title: title.clone() });
    }
    if guard(|| core.st().queue.play_next(track)).is_none() {
        return;
    }
    core.queue_changed();
    core.emit_queue();
    next_song(core);
    toast(core, "jam.played", json!({ "who": format!("@{from}") }), Some(title), "info");
}

/// `Core::next` needs the `Arc`: `Core::arc()` upgrades its weak self-reference.
fn next_song(core: &Core) {
    if let Some(c) = core.arc() {
        c.next(false);
    }
}

/// A vote moves the song up among the jam's songs.
fn host_vote(core: &Core, item: &str, from: &str, up: bool) {
    let votes: HashMap<String, usize> = {
        let mut u = lock(&rt().jam);
        let Some(j) = u.jam.as_mut() else { return };
        let v = j.votes.entry(item.to_string()).or_default();
        if up {
            v.insert(from.to_string());
        } else {
            v.remove(from);
        }
        j.votes.iter().map(|(k, v)| (k.clone(), v.len())).collect()
    };
    {
        let mut st = core.st();
        let keys: Vec<String> = st.queue.upcoming().into_iter().take(st.queue.manual_len()).map(track_key).collect();
        let want = glyph_core::jam::vote_order(&keys, &votes);
        if want == keys {
            return;
        }
        let mut cur = keys;
        for (i, k) in want.iter().enumerate() {
            if let Some(from_i) = cur.iter().position(|c| c == k) {
                if from_i != i {
                    guard(|| st.queue.move_item(from_i, i));
                    let moved = cur.remove(from_i);
                    cur.insert(i, moved);
                }
            }
        }
    }
    core.queue_changed();
    core.emit_queue();
}

fn host_remove(core: &Core, item: &str) {
    let removed = {
        let mut st = core.st();
        let n = st.queue.manual_len();
        let pos = st.queue.upcoming().into_iter().take(n).position(|tr| track_key(tr) == item);
        if let Some(i) = pos {
            guard(|| st.queue.remove(i));
        }
        pos.is_some()
    };
    if removed {
        core.queue_changed();
        core.emit_queue();
    }
    if let Some(j) = lock(&rt().jam).jam.as_mut() {
        j.by.remove(item);
        j.votes.remove(item);
    }
}

fn run_control(core: &Core, c: Control) {
    let Some(core) = core.arc() else { return };
    let playing = is_playing(&core);
    match c {
        Control::Play if !playing => core.toggle_play(),
        Control::Pause if playing => core.toggle_play(),
        Control::Play | Control::Pause => {}
        Control::Next => core.next(false),
        Control::Previous => core.previous(),
        Control::Seek { ms } => core.seek(ms as f64 / 1000.0),
    }
}

// ------------------------------------------------------------ jam: guest

/// Intercepts a playback control of a guest: it goes to the host when
/// allowed, never plays here. `true` = handled.
fn control(core: &Core, c: Control) -> bool {
    let allowed = {
        let mut u = lock(&rt().jam);
        let Some(j) = u.jam.as_mut().filter(|j| !j.host) else { return false };
        let ok = j.state.perms.allows(&c);
        if ok {
            j.outbox.push(Op::Control { control: c });
        }
        ok
    };
    if !allowed {
        toast_info(core, "jam.not_allowed");
    }
    true
}

/// The guest's queue shows the host's: the jam's songs, then the host's own
/// list.
fn mirror_queue(core: &Core) {
    let (manual, after) = {
        let u = lock(&rt().jam);
        let Some(j) = u.jam.as_ref().filter(|j| !j.host) else { return };
        (j.state.queue.iter().map(|i| i.track.clone()).collect::<Vec<Track>>(), j.state.after.clone())
    };
    {
        let mut st = core.st();
        let same = {
            let have: Vec<String> = st.queue.upcoming().into_iter().map(track_key).collect();
            let want: Vec<String> = manual.iter().chain(after.iter()).map(track_key).collect();
            have == want && st.queue.manual_len() == manual.len()
        };
        if same {
            return;
        }
        guard(|| st.queue.mirror(manual, after));
    }
    core.emit_queue();
}

/// Listening at the host: the player shows the host's song and where it is,
/// nothing plays here. Returns the state to show.
fn remote_state(core: &Core) -> Option<PlayerState> {
    let (track, source, playing, pos) = {
        let u = lock(&rt().jam);
        let j = u.jam.as_ref().filter(|j| !j.host && !j.listen_here)?;
        let now = j.state.now.as_ref()?;
        (now.track.clone(), now.resolved.as_ref().unwrap_or(&now.track).source, now.playing, Duration::from_millis(j.host_position().unwrap_or(0)))
    };
    let volume = {
        let mut st = core.st();
        if !st.current.as_ref().is_some_and(|c| same_song(c, &track)) {
            st.current = Some(track.clone());
            st.resolving = false;
            drop(st);
            core.emit_queue();
        }
        core.st().config.player.volume
    };
    Some(PlayerState {
        status: if playing { Status::Playing } else { Status::Paused },
        duration: track.duration_ms.map(Duration::from_millis),
        track: Some(track),
        provider: Some(source),
        backend: Some(BackendKind::Native),
        is_preview: false,
        position: pos,
        volume,
    })
}

/// Plays the host's song here (from the host's source when it can), at the
/// host's moment, playing or paused like the host.
fn sync(core: &Core) {
    let (now, settling, host_pos, speed) = {
        let mut u = lock(&rt().jam);
        let Some(j) = u.jam.as_mut().filter(|j| !j.host) else { return };
        j.synced_at = Some(Instant::now());
        if !j.listen_here {
            return;
        }
        let Some(now) = j.state.now.clone() else { return };
        (now, j.settle_until.is_some_and(|t| Instant::now() < t), j.host_position().unwrap_or(0), j.speed)
    };
    let (cur, resolving, restored) = {
        let st = core.st();
        (st.current.clone(), st.resolving, st.restored_pos.is_some())
    };
    let here = cur.is_some_and(|c| same_song(&c, &now.track)) && !restored;
    if !here {
        // The host's song is about to end and this app already went on to
        // the next one: the host follows in a moment.
        let ending = now.track.duration_ms.is_some_and(|d| host_pos + 4000 >= d);
        // Also when something else was started here meanwhile.
        if !settling && !ending {
            if let Some(j) = lock(&rt().jam).jam.as_mut() {
                j.settle_until = Some(Instant::now() + SETTLE);
                j.speed = 1.0;
            }
            if let Some(arc) = core.arc() {
                arc.start_track(now.resolved.clone().unwrap_or_else(|| now.track.clone()));
            }
            // Load it paused when the host is paused.
            rt().hold.store(!now.playing, Ordering::Relaxed);
        }
        return;
    }
    if resolving || settling {
        return;
    }
    let playing = is_playing(core);
    if now.playing != playing && !remote_send(Cmd::Pause(!now.playing)) {
        if let Some(p) = &core.player {
            if now.playing {
                p.play();
            } else {
                p.pause();
            }
        }
    }
    if !now.playing {
        return;
    }
    let have = position(core).as_millis() as u64;
    let embed = local_state(core).is_some_and(|s| s.backend == Some(BackendKind::Embed));
    let can_nudge = !remote_chosen() && !embed;
    let (speed_to, seek) = match glyph_core::jam::step(have, host_pos, can_nudge) {
        Step::Keep => (1.0, None),
        Step::Nudge(r) => (r, None),
        Step::Seek(ms) => (1.0, Some(ms)),
    };
    if (speed_to - speed).abs() > 0.002 && can_nudge {
        if let Some(p) = &core.player {
            p.set_speed(speed_to);
        }
    }
    if let Some(j) = lock(&rt().jam).jam.as_mut() {
        j.speed = speed_to;
        if seek.is_some() {
            j.settle_until = Some(Instant::now() + SETTLE);
        }
    }
    if let Some(ms) = seek {
        let to = Duration::from_millis(ms);
        if !remote_send(Cmd::Seek(to.as_secs_f64())) {
            if let Some(p) = &core.player {
                p.seek(to);
            }
        }
    }
}

fn flush_outbox(core: &Arc<Core>) {
    let Some(b) = core.backend() else { return };
    let (ops, code) = {
        let mut u = lock(&rt().jam);
        let Some(j) = u.jam.as_mut() else { return };
        j.sending = true;
        (std::mem::take(&mut j.outbox), j.code.clone())
    };
    let (core, gen) = (core.clone(), cur_gen());
    crate::rt::spawn(async move {
        let mut err = None;
        for op in ops {
            if let Err(e) = call(b.jam_op(&code, op)).await {
                err = Some(e);
                break;
            }
        }
        if cur_gen() != gen {
            return;
        }
        if let Some(j) = lock(&rt().jam).jam.as_mut() {
            j.sending = false;
        }
        match err {
            Some((c, msg)) if gone(c.as_deref()) => lost(&core, problem(c.as_deref(), &msg)),
            Some((_, msg)) => toast(&core, &msg, Value::Null, None, "info"),
            None => {}
        }
    });
}

/// The host is gone: carry on as the host.
fn take_over_host(core: &Arc<Core>) {
    let Some(b) = core.backend() else { return };
    let code = {
        let mut u = lock(&rt().jam);
        let Some(j) = u.jam.as_mut() else { return };
        j.claiming = true;
        j.code.clone()
    };
    let (core, gen) = (core.clone(), cur_gen());
    crate::rt::spawn(async move {
        let r = call(b.jam_claim(&code)).await;
        if cur_gen() != gen {
            return;
        }
        match r {
            Ok(state) => become_host(&core, state),
            Err((c, msg)) if gone(c.as_deref()) => lost(&core, problem(c.as_deref(), &msg)),
            Err((_, msg)) => {
                // The host is back, or someone else was faster.
                tracing::info!("jam take over: {msg}");
                if let Some(j) = lock(&rt().jam).jam.as_mut() {
                    j.claiming = false;
                    j.host_gone = None;
                }
            }
        }
    });
}

/// Took over: the jam's queue (already this app's queue) and the song go on
/// from here.
fn become_host(core: &Arc<Core>, state: State) {
    let Some(old) = lock(&rt().jam).jam.take() else { return };
    let remote = !old.listen_here;
    let mut j = Jam::new(state.code.clone(), true, old.me.clone(), state.clone());
    j.clock = old.clock;
    j.guests = state.members.iter().filter(|m| !m.host && m.handle != j.me).map(|m| m.handle.clone()).collect();
    for item in &state.queue {
        j.by.insert(item.id.clone(), item.by.clone());
        if !item.votes.is_empty() {
            j.votes.insert(item.id.clone(), item.votes.iter().cloned().collect());
        }
    }
    let me = j.me.clone();
    j.event(&me, What::Host);
    if (old.speed - 1.0).abs() > f32::EPSILON {
        if let Some(p) = &core.player {
            p.set_speed(1.0);
        }
    }
    // Not listening here so far: start the song where the old host was.
    let mut start = None;
    if remote {
        if let Some(n) = state.now.clone() {
            j.resume_at = state.position_at(j.server_now());
            start = Some((n.resolved.unwrap_or(n.track), !n.playing));
        }
    }
    let code = j.code.clone();
    j.saved_queue = old.saved_queue;
    save_jam(&code, &me, true, j.saved_queue.as_ref());
    lock(&rt().jam).jam = Some(j);
    bump();
    if let Some((track, paused)) = start {
        clear_remote(core);
        core.start_track(track);
        rt().hold.store(paused, Ordering::Relaxed);
    }
    host_loop(core);
    emit_jam(core);
    toast(core, "jam.you_host", Value::Null, None, "ok");
}

#[cfg(test)]
mod saved_jam_tests {
    use super::SavedJam;

    #[test]
    fn saved_jam_roundtrip_and_default_listen_here() {
        let s = SavedJam { code: "K7M2QP".into(), handle: "alice".into(), listen_here: false, saved_queue: None };
        let raw = serde_json::to_string(&s).unwrap();
        let back: SavedJam = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, s);

        let minimal: SavedJam = serde_json::from_str(r#"{"code":"K7M2QP","handle":"alice"}"#).unwrap();
        assert!(minimal.listen_here);
        assert!(minimal.saved_queue.is_none());
    }
}
