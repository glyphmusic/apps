//! MCP tools for AI apps (from ../../src/mcp_tools.rs, without GPUI).
//!
//! `crate::mcp` is the transport (local HTTP server, JSON-RPC); calls come
//! here through a channel and are answered from the tools below, which act
//! through `Core`. Plugin code can use the same tools with [`call`].
//!
//! Songs travel between tools as a `ref` (the song key); every song an
//! answer mentions is remembered so a later call can use its ref.

use crate::controller::Core;
use crate::mcp::McpRequest;
use futures::StreamExt;
use glyph_core::config::{McpMode, ShuffleStyle};
use glyph_core::queue::ShuffleMode;
use glyph_core::RepeatMode;
use glyph_protocol::mcp::text_result;
use glyph_protocol::{SearchEvent, Track};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// MCP state of the app.
pub struct McpState {
    pub tx: flume::Sender<McpRequest>,
    rx: flume::Receiver<McpRequest>,
    local: Option<crate::mcp::LocalServer>,
    /// Token the local server was started with.
    local_token: String,
    /// Songs mentioned in answers, by ref.
    refs: HashMap<String, Track>,
    /// Songs of the last search the UI showed (see [`remember_search`]).
    search: Vec<Track>,
    /// Bumped to stop a running relay loop.
    relay_gen: u64,
    pub relay_running: bool,
    /// Last problem (port taken, server refused), for the settings page.
    pub error: Option<String>,
    /// MCP token for the server endpoint, as last created (shown once).
    pub server_token: Option<glyph_protocol::mcp::TokenResponse>,
    pub token_loading: bool,
    /// Assistants asking for access (oldest first).
    pub approvals: Vec<glyph_protocol::mcp::Approval>,
}

impl Default for McpState {
    fn default() -> Self {
        let (tx, rx) = flume::unbounded();
        Self {
            tx,
            rx,
            local: None,
            local_token: String::new(),
            refs: Default::default(),
            search: Vec::new(),
            relay_gen: 0,
            relay_running: false,
            error: None,
            server_token: None,
            token_loading: false,
            approvals: Vec::new(),
        }
    }
}

pub fn state() -> MutexGuard<'static, McpState> {
    static S: OnceLock<Mutex<McpState>> = OnceLock::new();
    S.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

fn ok(v: Value) -> Value {
    text_result(&v, false)
}

fn fail(msg: impl Into<String>) -> Value {
    text_result(&Value::String(msg.into()), true)
}

fn signed_in(core: &Core) -> bool {
    core.backend().is_some_and(|b| b.account().is_some())
}

/// Starts the dispatcher of tool calls and the watcher that starts / stops
/// the local server and the relay as the settings say. Safe to call twice.
pub fn start(core: &Arc<Core>) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Plugins reach every tool through the same code (with their own permissions).
        crate::plugins::set_mcp_handler(plugin_handler);
        let rx = state().rx.clone();
        let c = core.clone();
        let _ = std::thread::Builder::new().name("glyph-mcp-calls".into()).spawn(move || {
            while let Ok(r) = rx.recv() {
                dispatch(&c, r);
            }
        });
        let c = core.clone();
        crate::rt::spawn(async move {
            let mut last = String::new();
            loop {
                let fp = {
                    let m = c.st().config.mcp.clone();
                    format!("{}|{:?}|{}|{}|{}|{}", m.enabled, m.mode, m.port, m.token, signed_in(&c), c.backend().is_some())
                };
                if fp != last {
                    last = fp;
                    sync(&c);
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    });
}

/// The songs of the search the UI shows now; an assistant can name them by `ref`
/// (call it where the "search" event is emitted).
pub fn remember_search(tracks: &[Track]) {
    state().search = tracks.iter().take(300).cloned().collect();
}

/// For plugins: runs a tool and waits (on the caller's worker thread).
fn plugin_handler(core: &Arc<Core>, tool: &str, args: &Value) -> Option<Value> {
    Some(crate::rt::block_on(call(core, tool, args)))
}

/// Assistants asking for access that nobody answered yet (the UI asks on start).
pub fn mcp_pending() -> Vec<glyph_protocol::mcp::Approval> {
    state().approvals.clone()
}

/// One call from an assistant: the access settings decide, then the tool runs.
fn dispatch(core: &Arc<Core>, r: McpRequest) {
    let access = core.st().config.mcp.access;
    if r.name == glyph_protocol::mcp::LIST_TOOLS {
        let _ = r.reply.send(json!({ "tools": access.tools() }));
        return;
    }
    if !access.allows(&r.name) {
        tracing::info!(tool = %r.name, "MCP tool call refused by the AI assistant settings");
        let _ = r.reply.send(fail(format!(
            "The user does not allow AI assistants to use `{}` (Glyph: Settings › Integrations › AI assistants).",
            r.name
        )));
        return;
    }
    tracing::info!(tool = %r.name, "MCP tool call");
    let core = core.clone();
    crate::rt::spawn(async move {
        let v = call(&core, &r.name, &r.args).await;
        let _ = r.reply.send(v);
    });
}

/// Starts / stops the local server and the server relay as the settings say.
pub fn sync(core: &Arc<Core>) {
    let cfg = core.st().config.mcp.clone();
    let want_local = cfg.enabled && cfg.mode == McpMode::Local;
    let want_relay = cfg.enabled && cfg.mode == McpMode::Server && signed_in(core);
    if want_local {
        let mut token = cfg.token.clone();
        if token.is_empty() {
            token = crate::mcp::new_token();
            core.st().config.mcp.token = token.clone();
            core.config_changed();
        }
        let mut s = state();
        if s.local.as_ref().map(|l| l.port) != Some(cfg.port) || s.local_token != token {
            s.local = None;
            match crate::mcp::start_local(cfg.port, token.clone(), s.tx.clone()) {
                Ok(l) => {
                    s.local = Some(l);
                    s.local_token = token;
                    s.error = None;
                }
                Err(e) => {
                    tracing::warn!("MCP server could not start: {e}");
                    s.error = Some(e.to_string());
                }
            }
        }
    } else {
        state().local = None;
    }
    let running = state().relay_running;
    if want_relay && !running {
        start_relay(core);
    } else if !want_relay && running {
        let mut s = state();
        s.relay_gen += 1;
        s.relay_running = false;
    }
}

/// What Settings > Integrations shows (`mcp_info`).
pub fn info(core: &Core) -> Value {
    let (cfg, base) = {
        let st = core.st();
        (st.config.mcp.clone(), st.config.effective_server_url().trim_end_matches('/').to_string())
    };
    let signed = signed_in(core);
    let s = state();
    let (url, token) = match cfg.mode {
        McpMode::Local => (Some(format!("http://127.0.0.1:{}/mcp", cfg.port)), Some(cfg.token.clone()).filter(|t| !t.is_empty())),
        // The assistant only needs the address (OAuth); a token is the
        // fallback for clients without it.
        McpMode::Server if signed => (
            Some(s.server_token.as_ref().map(|t| t.url.clone()).unwrap_or_else(|| format!("{base}/mcp"))),
            s.server_token.as_ref().map(|t| t.token.clone()),
        ),
        McpMode::Server => (None, None),
    };
    let config = url.as_ref().zip(token.as_ref()).map(|(u, t)| crate::mcp::client_config(u, t));
    json!({
        "url": url, "token": token, "config": config, "error": s.error,
        "relay_running": s.relay_running, "token_loading": s.token_loading,
    })
}

/// New local token (the old one stops working at once).
pub fn rotate_token(core: &Arc<Core>) {
    core.st().config.mcp.token = crate::mcp::new_token();
    state().local = None;
    core.config_changed();
    sync(core);
    core.toast_key("mcp.token_rotated", "info");
}

/// Asks the server for a token for its MCP endpoint.
pub async fn request_server_token(core: &Arc<Core>, name: Option<String>) {
    let Some(b) = core.backend() else { return };
    {
        let mut s = state();
        if s.token_loading {
            return;
        }
        s.token_loading = true;
    }
    let r = b.mcp_token(name.as_deref()).await;
    let mut s = state();
    s.token_loading = false;
    match r {
        Ok(t) => {
            s.server_token = Some(t);
            s.error = None;
        }
        Err(e) => s.error = Some(format!("{e:#}")),
    }
}

pub async fn sessions(core: &Arc<Core>) -> Result<Vec<glyph_protocol::mcp::Session>, String> {
    let b = core.backend_or_err()?;
    b.mcp_sessions().await.map_err(|e| format!("{e:#}"))
}

pub async fn rename_session(core: &Arc<Core>, id: String, name: String) -> Result<glyph_protocol::mcp::Session, String> {
    let b = core.backend_or_err()?;
    b.mcp_rename_session(&id, &name).await.map_err(|e| format!("{e:#}"))
}

pub async fn revoke_session(core: &Arc<Core>, id: String) -> Result<(), String> {
    let b = core.backend_or_err()?;
    b.mcp_revoke_session(&id).await.map_err(|e| format!("{e:#}"))
}

pub async fn claim_oauth(core: &Arc<Core>, id: String) -> Result<glyph_protocol::mcp::Approval, String> {
    let b = core.backend_or_err()?;
    b.mcp_claim_oauth(&id).await.map_err(|e| format!("{e:#}"))
}

/// The user answers an assistant's access request (relay mode).
pub async fn mcp_approve(core: &Arc<Core>, id: String, allow: bool) -> Result<(), String> {
    state().approvals.retain(|a| a.id != id);
    core.emit("mcp_approvals", state().approvals.clone());
    let b = core.backend_or_err()?;
    b.mcp_approve(glyph_protocol::mcp::Decision { id, allow }).await.map_err(|e| format!("{e:#}"))?;
    core.toast_key(if allow { "mcp.allowed" } else { "mcp.denied" }, if allow { "ok" } else { "info" });
    Ok(())
}

/// Fetches tool calls queued on the server (long poll), runs them and sends
/// the results back, until MCP is turned off or the mode changes.
fn start_relay(core: &Arc<Core>) {
    let Some(b) = core.backend() else { return };
    let (gen, calls) = {
        let mut s = state();
        s.relay_gen += 1;
        s.relay_running = true;
        (s.relay_gen, s.tx.clone())
    };
    let core = core.clone();
    crate::rt::spawn(async move {
        let mut failures = 0u32;
        loop {
            if state().relay_gen != gen {
                break;
            }
            match b.mcp_pull().await {
                Ok(pull) => {
                    failures = 0;
                    // A poll may end after MCP was turned off or the mode changed.
                    {
                        let mut s = state();
                        if s.relay_gen != gen {
                            break;
                        }
                        s.error = None;
                        let mut new = false;
                        for a in pull.approvals {
                            if !s.approvals.iter().any(|x| x.id == a.id) {
                                s.approvals.push(a);
                                new = true;
                            }
                        }
                        if new {
                            core.emit("mcp_approvals", s.approvals.clone());
                            // Like GPUI's `cx.activate`: bring the window up so the question is seen.
                            core.host.focus_main_window();
                        }
                    }
                    for call in pull.calls {
                        let (b, calls) = (b.clone(), calls.clone());
                        // `crate::mcp::call` waits for the answer: its own thread.
                        crate::rt::spawn(async move {
                            let result = crate::rt::spawn_blocking(move || crate::mcp::call(call.name, call.arguments, &calls)).await;
                            if let Ok(result) = result {
                                let _ = b.mcp_result(glyph_protocol::mcp::ResultRequest { id: call.id, result }).await;
                            }
                        });
                    }
                }
                Err(e) => {
                    failures += 1;
                    tracing::info!("MCP relay: {e}");
                    state().error = Some(format!("{e:#}"));
                    // Back off: 2, 4, 8 ... 60 s.
                    tokio::time::sleep(Duration::from_secs((1u64 << failures.min(6)).min(60))).await;
                }
            }
        }
    });
}

// ------------------------------------------------------------- tools

fn key(t: &Track) -> String {
    glyph_core::dedupe::track_key(t)
}

/// The song as an answer shows it; remembers its ref.
fn track_json(core: &Core, t: &Track) -> Value {
    let r = key(t);
    let liked = core.st().library.is_liked(t);
    {
        let mut s = state();
        if s.refs.len() > 3000 {
            s.refs.clear();
        }
        s.refs.insert(r.clone(), t.clone());
    }
    json!({
        "ref": r, "title": t.title, "artist": t.artist, "album": t.album,
        "duration_s": t.duration_ms.map(|ms| ms / 1000), "explicit": t.explicit, "liked": liked,
    })
}

fn tracks_json(core: &Core, list: &[Track]) -> Vec<Value> {
    list.iter().map(|t| track_json(core, t)).collect()
}

/// The song a call names by `ref` (any song an answer mentioned, the queue,
/// the library).
fn find_ref(core: &Core, r: &str) -> Option<Track> {
    if let Some(t) = state().refs.get(r) {
        return Some(t.clone());
    }
    let st = core.st();
    let hit = |t: &&Track| key(t) == r;
    st.current.iter().find(hit).cloned()
        .or_else(|| st.queue.upcoming().into_iter().find(|t| key(t) == r).cloned())
        .or_else(|| st.library.liked.iter().find(hit).cloned())
        .or_else(|| st.library.playlists.iter().flat_map(|p| p.tracks.iter()).find(hit).cloned())
        .or_else(|| state().search.iter().find(hit).cloned())
}

fn refs_of(core: &Core, args: &Value) -> Vec<Track> {
    args["refs"].as_array().map(|a| a.iter().filter_map(|r| r.as_str()).filter_map(|r| find_ref(core, r)).collect()).unwrap_or_default()
}

/// A list by id: "liked", "recent" or a playlist; its name and songs.
fn list_of(core: &Core, id: &str) -> Option<(String, Vec<Track>)> {
    let st = core.st();
    match id {
        "liked" => Some(("Liked Songs".into(), st.library.liked.clone())),
        "recent" => Some(("Recently played".into(), st.library.recent_tracks().cloned().collect())),
        id => st.library.playlist(id).map(|p| (p.name.clone(), p.tracks.clone())),
    }
}

/// Searches and returns the songs, best first.
async fn search(core: &Core, query: &str, limit: usize) -> Vec<Track> {
    let Some(b) = core.backend() else { return Vec::new() };
    let mut found: Vec<Track> = Vec::new();
    let mut stream = b.search(query);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, stream.next()).await {
            Ok(Some(SearchEvent::Results { tracks, .. })) => {
                for t in tracks {
                    if !found.iter().any(|x| x.source == t.source && x.id == t.id) {
                        found.push(t);
                    }
                }
            }
            Ok(Some(SearchEvent::Done)) | Ok(None) | Err(_) => break,
            _ => {}
        }
    }
    let (plain, _) = glyph_core::taste::parse_query(query);
    let st = core.st();
    let hide = st.config.audio.hide_explicit;
    let taste = glyph_core::taste::Taste::from_library(&st.library);
    let hidden = &st.library.hidden_artists;
    found.retain(|t| !(hide && t.maybe_explicit()) && !hidden.iter().any(|h| glyph_core::dedupe::by_artist(h, &t.artist)));
    found.sort_by(|a, b| {
        glyph_core::taste::score(b, &plain, &taste).partial_cmp(&glyph_core::taste::score(a, &plain, &taste)).unwrap_or(std::cmp::Ordering::Equal)
    });
    found.truncate(limit.max(1));
    found
}

/// The song of a call: its `ref`, else the best hit for `query`.
async fn song(core: &Core, args: &Value) -> Option<Track> {
    if let Some(t) = args["ref"].as_str().and_then(|r| find_ref(core, r)) {
        return Some(t);
    }
    let q = args["query"].as_str().map(str::trim).filter(|q| !q.is_empty())?;
    search(core, q, 1).await.into_iter().next()
}

fn is_playing(core: &Core) -> bool {
    core.player_state().is_some_and(|s| matches!(s.status, glyph_audio::Status::Playing))
}

/// Plays `tracks` from the top, shuffled when asked.
fn play_all(core: &Arc<Core>, tracks: Vec<Track>, shuffle: bool, context: Option<String>) {
    if shuffle {
        core.set_shuffle(true);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let start = nanos as usize % tracks.len().max(1);
        core.play_list(tracks, start, context);
    } else {
        core.play_list(tracks, 0, context);
    }
}

fn shuffle_tracks(tracks: &mut [Track]) {
    let mut s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64).max(1);
    for i in (1..tracks.len()).rev() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let j = (s as usize) % (i + 1);
        tracks.swap(i, j);
    }
}

enum ResolvedDevice {
    Local,
    Account(glyph_protocol::playback::Output),
    Network { id: String, name: String, protocol: String },
}

fn device_entry_json(core: &Core, o: &glyph_protocol::playback::Output, selected: Option<&str>) -> Value {
    json!({
        "id": o.device.id,
        "name": o.device.name,
        "platform": o.device.platform,
        "current": o.device.current,
        "online": o.online,
        "available": o.online && o.state.available,
        "selected": selected == Some(o.device.id.as_str()),
        "playing": o.online && o.state.playing,
        "volume": (o.state.volume * 100.0).round(),
        "song": o.state.track.as_ref().map(|t| track_json(core, t)),
        "last_seen": o.device.last_seen,
        "location": o.device.approximate_location,
    })
}

fn resolve_device(core: &Core, query: &str) -> Result<ResolvedDevice, String> {
    let q = query.trim();
    if q.is_empty() {
        return Err("device is empty".into());
    }
    let ql = q.to_lowercase();
    if matches!(ql.as_str(), "local" | "this" | "this_computer" | "here") {
        return Ok(ResolvedDevice::Local);
    }
    let (outputs, _) = crate::account::outputs_snapshot(core);
    if let Some(o) = outputs.iter().find(|o| o.device.id == q) {
        return Ok(if o.device.current { ResolvedDevice::Local } else { ResolvedDevice::Account(o.clone()) });
    }
    let all_devs = crate::jam::devices_json(core);
    let net: Vec<(String, String, String)> = all_devs
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| !matches!(d["kind"].as_str(), Some("this") | Some("account")))
        .filter_map(|d| Some((d["id"].as_str()?.to_string(), d["name"].as_str()?.to_string(), d["protocol"].as_str().unwrap_or("").to_string())))
        .collect();
    if let Some((id, name, protocol)) = net.iter().find(|(id, _, _)| id == q).cloned() {
        return Ok(ResolvedDevice::Network { id, name, protocol });
    }
    let mut matches: Vec<ResolvedDevice> = outputs
        .iter()
        .filter(|o| o.device.name.eq_ignore_ascii_case(q))
        .map(|o| if o.device.current { ResolvedDevice::Local } else { ResolvedDevice::Account(o.clone()) })
        .collect();
    matches.extend(net.iter().filter(|(_, n, _)| n.eq_ignore_ascii_case(q)).cloned().map(|(id, name, protocol)| ResolvedDevice::Network { id, name, protocol }));
    if matches.is_empty() {
        matches = outputs
            .iter()
            .filter(|o| o.device.name.to_lowercase().contains(&ql))
            .map(|o| if o.device.current { ResolvedDevice::Local } else { ResolvedDevice::Account(o.clone()) })
            .collect();
        matches.extend(
            net.iter()
                .filter(|(_, n, _)| n.to_lowercase().contains(&ql))
                .cloned()
                .map(|(id, name, protocol)| ResolvedDevice::Network { id, name, protocol }),
        );
    }
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap()),
        0 => {
            let names: Vec<String> = outputs.iter().map(|o| o.device.name.clone()).chain(net.iter().map(|(_, n, _)| n.clone())).collect();
            if names.is_empty() {
                Err(format!("No device \"{q}\" found (no signed-in account devices or network devices available)."))
            } else {
                Err(format!("No device \"{q}\" found. Available: {}.", names.join(", ")))
            }
        }
        _ => {
            let desc: Vec<String> = matches
                .iter()
                .map(|d| match d {
                    ResolvedDevice::Local => "This device (local)".into(),
                    ResolvedDevice::Account(o) => format!("{} ({})", o.device.name, o.device.id),
                    ResolvedDevice::Network { id, name, .. } => format!("{name} ({id})"),
                })
                .collect();
            Err(format!("Multiple devices match \"{q}\": {}. Use the device id.", desc.join(", ")))
        }
    }
}

async fn resolve_play_tracks(core: &Arc<Core>, args: &Value) -> Result<(Vec<Track>, Option<String>), String> {
    if let Some(id) = args["playlist_id"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        let Some((title, tracks)) = list_of(core, id) else {
            return Err(format!("No playlist {id}."));
        };
        if tracks.is_empty() {
            return Err("The playlist is empty.".into());
        }
        return Ok((tracks, Some(title)));
    }
    if let Some(artist) = args["artist"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        let songs: Vec<Track> = search(core, artist, 40)
            .await
            .into_iter()
            .filter(|t| glyph_core::dedupe::by_artist(&t.artist, artist))
            .collect();
        if songs.is_empty() {
            return Err(format!("No songs by {artist} found."));
        }
        return Ok((songs, Some(artist.to_string())));
    }
    match song(core, args).await {
        Some(t) => Ok((vec![t], None)),
        None => Err("Give ref, query, artist, or playlist_id.".into()),
    }
}

async fn send_remote_transfer(core: &Arc<Core>, target: &glyph_protocol::playback::Output, tracks: Vec<Track>) -> Result<(), String> {
    use glyph_protocol::playback::{Command, Send as PlaySend};
    let Some(b) = core.backend() else {
        return Err("Glyph is still starting; try again in a moment.".into());
    };
    let send = PlaySend {
        request_id: None,
        target: target.device.id.clone(),
        command: Command::Transfer { tracks, index: 0, position_ms: 0, playing: true },
    };
    if target.state.acknowledgements {
        match tokio::time::timeout(Duration::from_secs(18), b.playback_command(send)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err(format!("{} did not confirm playback in time.", target.device.name)),
        }
    } else {
        let rx = b.playback_send(send);
        match tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(10))).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(e))) => Err(e.to_string()),
            _ => Err(format!("Could not send playback command to {}.", target.device.name)),
        }
    }
}

/// Runs one tool and returns its answer (an MCP `CallToolResult`). No access
/// check: the caller (assistant dispatcher, plugin host) does its own.
pub async fn call(core: &Arc<Core>, name: &str, args: &Value) -> Value {
    match name {
        "now_playing" => {
            let cur = core.st().current.clone();
            let song = cur.map(|t| track_json(core, &t));
            let p = core.player_dto();
            let (queue_length, from) = {
                let st = core.st();
                (st.queue.upcoming().len(), st.context.clone())
            };
            ok(json!({
                "song": song,
                "playing": is_playing(core),
                "loading": p.status == "loading",
                "position_s": p.position as u64,
                "duration_s": p.duration.map(|d| d as u64),
                "volume": (p.volume * 100.0).round(),
                "shuffle": p.shuffle,
                "repeat": p.repeat,
                "queue_length": queue_length,
                "playing_from": from,
            }))
        }
        "play" => {
            if !is_playing(core) {
                core.toggle_play();
            }
            ok(json!("Playing."))
        }
        "pause" => {
            if is_playing(core) {
                core.toggle_play();
            }
            ok(json!("Paused."))
        }
        "toggle_play" => {
            core.toggle_play();
            ok(json!("Toggled."))
        }
        "next" => {
            core.next(false);
            ok(json!("Skipped."))
        }
        "previous" => {
            core.previous();
            ok(json!("Back to the previous song."))
        }
        "seek" => {
            let s = args["seconds"].as_f64().unwrap_or(0.0).max(0.0);
            core.seek(s);
            ok(json!(format!("At {s:.0} s.")))
        }
        "set_volume" => {
            let v = args["volume"].as_f64().unwrap_or(50.0).clamp(0.0, 100.0);
            core.set_volume((v / 100.0) as f32);
            ok(json!(format!("Volume {v:.0} %.")))
        }
        "set_shuffle" => {
            let on = args["on"].as_bool().unwrap_or(true);
            let style = match args["style"].as_str() {
                Some("random") => Some(ShuffleStyle::Random),
                Some("smart") => Some(ShuffleStyle::Smart),
                _ => None,
            };
            if let Some(style) = style {
                {
                    let mut st = core.st();
                    st.config.playback.shuffle = style;
                    let recent: Vec<String> = st.library.recent_tracks().map(key).collect();
                    st.queue.set_recent(recent);
                    st.queue.set_shuffle_mode(match style {
                        ShuffleStyle::Smart => ShuffleMode::Smart,
                        ShuffleStyle::Random => ShuffleMode::Random,
                    });
                }
                core.config_changed();
                core.emit_queue();
            }
            core.set_shuffle(on);
            ok(json!(if on { "Shuffle on." } else { "Shuffle off." }))
        }
        "set_repeat" => {
            let mode = match args["mode"].as_str() {
                Some("all") => RepeatMode::All,
                Some("one") => RepeatMode::One,
                _ => RepeatMode::Off,
            };
            core.set_repeat(crate::dto::repeat_name(mode));
            ok(json!("Repeat set."))
        }
        "search" => {
            let q = args["query"].as_str().unwrap_or("").trim().to_string();
            if q.is_empty() {
                return fail("query is empty");
            }
            let limit = args["limit"].as_u64().unwrap_or(10).clamp(1, 50) as usize;
            let found = search(core, &q, limit).await;
            ok(json!({ "songs": tracks_json(core, &found) }))
        }
        "play_song" => match song(core, args).await {
            Some(t) => {
                let v = track_json(core, &t);
                core.play_list(vec![t], 0, None);
                ok(json!({ "playing": v }))
            }
            None => fail("No such song (give a ref from another answer, or a query)."),
        },
        "play_artist" => {
            let name = args["name"].as_str().unwrap_or("").trim().to_string();
            if name.is_empty() {
                return fail("name is empty");
            }
            let shuffle = args["shuffle"].as_bool().unwrap_or(false);
            let songs: Vec<Track> = search(core, &name, 40).await.into_iter().filter(|t| glyph_core::dedupe::by_artist(&t.artist, &name)).collect();
            if songs.is_empty() {
                return fail(format!("No songs by {name} found."));
            }
            let n = songs.len();
            play_all(core, songs, shuffle, Some(name.clone()));
            ok(json!(format!("Playing {n} songs by {name}.")))
        }
        "queue_add" => {
            let next = args["next"].as_bool().unwrap_or(false);
            match song(core, args).await {
                Some(t) => {
                    let v = track_json(core, &t);
                    core.queue_add(vec![t], next);
                    ok(json!({ "queued": v, "next": next }))
                }
                None => fail("No such song (give a ref from another answer, or a query)."),
            }
        }
        "get_queue" => {
            let (up, manual, from) = {
                let st = core.st();
                (st.queue.upcoming().into_iter().cloned().collect::<Vec<Track>>(), st.queue.manual_len(), st.context.clone())
            };
            let m = manual.min(up.len());
            let first = tracks_json(core, &up[..m]);
            let rest = tracks_json(core, &up[m..up.len().min(m + 50)]);
            ok(json!({ "up_next": first, "next_from": from, "later": rest, "total": up.len() }))
        }
        "queue_remove" => {
            let i = args["index"].as_u64().unwrap_or(u64::MAX) as usize;
            let (first, len) = {
                let st = core.st();
                (st.queue.history().len() + usize::from(st.queue.current().is_some()), st.queue.upcoming().len())
            };
            if i >= len {
                return fail(format!("Queue index {i} out of range ({len} upcoming songs)."));
            }
            core.queue_remove(first + i);
            ok(json!({ "removed_index": i }))
        }
        "queue_move" => {
            let from = args["from"].as_u64().unwrap_or(u64::MAX) as usize;
            let to = args["to"].as_u64().unwrap_or(u64::MAX) as usize;
            let (first, len) = {
                let st = core.st();
                (st.queue.history().len() + usize::from(st.queue.current().is_some()), st.queue.upcoming().len())
            };
            if from >= len || to >= len {
                return fail(format!("Queue index out of range ({len} upcoming songs)."));
            }
            core.queue_move(first + from, first + to);
            ok(json!({ "from": from, "to": to }))
        }
        "queue_jump" => {
            let i = args["index"].as_u64().unwrap_or(u64::MAX) as usize;
            let (first, track, len) = {
                let st = core.st();
                let up = st.queue.upcoming();
                let len = up.len();
                let track = up.get(i).map(|t| (*t).clone());
                (st.queue.history().len() + usize::from(st.queue.current().is_some()), track, len)
            };
            let Some(track) = track else {
                return fail(format!("Queue index {i} out of range ({len} upcoming songs)."));
            };
            core.queue_jump(first + i);
            ok(json!({ "playing": track_json(core, &track), "index": i }))
        }
        "queue_clear" => {
            let cleared = core.st().queue.upcoming().len();
            core.queue_clear();
            ok(json!({ "cleared": cleared }))
        }
        "start_radio" => {
            if let Some(id) = args["playlist_id"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                if id == "recent" {
                    let (title, seeds) = list_of(core, id).unwrap_or_else(|| ("Recent".into(), Vec::new()));
                    if seeds.is_empty() {
                        return fail("No recently played songs to base radio on.");
                    }
                    crate::discover::start_radio_tracks(core, seeds, title.clone());
                    return ok(json!({ "started": true, "from": title }));
                }
                let list = if id == "liked" {
                    glyph_core::ListKey::Liked
                } else {
                    if core.st().library.playlist(id).is_none() {
                        return fail(format!("No playlist {id}."));
                    }
                    glyph_core::ListKey::Playlist { id: id.to_string() }
                };
                let title = list_of(core, id).map_or_else(|| id.to_string(), |(t, _)| t);
                crate::discover::start_list_radio(core, &list);
                ok(json!({ "started": true, "from": title }))
            } else if let Some(artist) = args["artist"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                let mut seeds: Vec<Track> = {
                    let st = core.st();
                    st.library
                        .liked
                        .iter()
                        .chain(st.library.recent_tracks())
                        .filter(|t| glyph_core::dedupe::by_artist(&t.artist, artist))
                        .take(5)
                        .cloned()
                        .collect()
                };
                if seeds.is_empty() {
                    seeds = search(core, artist, 10)
                        .await
                        .into_iter()
                        .filter(|t| glyph_core::dedupe::by_artist(&t.artist, artist))
                        .take(5)
                        .collect();
                }
                if seeds.is_empty() {
                    return fail(format!("No songs by {artist} found to start radio."));
                }
                crate::discover::start_radio_tracks(core, seeds, artist.to_string());
                ok(json!({ "started": true, "from": artist }))
            } else if args["ref"].as_str().is_some() || args["query"].as_str().is_some() {
                match song(core, args).await {
                    Some(t) => {
                        let v = track_json(core, &t);
                        let title = t.title.clone();
                        crate::discover::start_radio_tracks(core, vec![t], title);
                        ok(json!({ "started": true, "seed": v }))
                    }
                    None => fail("No such song (give a ref from another answer, or a query)."),
                }
            } else if let Some(cur) = core.st().current.clone() {
                let v = track_json(core, &cur);
                let title = cur.title.clone();
                crate::discover::start_radio_tracks(core, vec![cur], title);
                ok(json!({ "started": true, "seed": v }))
            } else {
                fail("Nothing is playing and no ref, query, artist, or playlist_id was given.")
            }
        }
        "sleep_timer" => {
            let off = args["off"].as_bool() == Some(true) || args["minutes"].as_u64() == Some(0);
            if off {
                crate::discover::set_sleep_timer(core, None, false);
            } else if args["end_of_song"].as_bool() == Some(true) {
                crate::discover::set_sleep_timer(core, None, true);
            } else if let Some(mins) = args["minutes"].as_u64() {
                crate::discover::set_sleep_timer(core, Some((mins as u32).clamp(1, 480)), false);
            }
            ok(crate::discover::sleep_status(core))
        }
        "recent_songs" => {
            let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 50) as usize;
            let songs: Vec<Track> = core.st().library.recent_tracks().take(limit).cloned().collect();
            ok(json!({ "songs": tracks_json(core, &songs) }))
        }
        "list_devices" => {
            let (outputs, selected) = crate::account::outputs_snapshot(core);
            let all = crate::jam::devices_json(core);
            let account_devices: Vec<Value> = outputs.iter().map(|o| device_entry_json(core, o, selected.as_deref())).collect();
            let network_devices: Vec<Value> = all
                .as_array()
                .into_iter()
                .flatten()
                .filter(|d| !matches!(d["kind"].as_str(), Some("this") | Some("account")))
                .cloned()
                .collect();
            let active_output = all
                .as_array()
                .and_then(|arr| arr.iter().find(|d| d["active"].as_bool() == Some(true)).cloned())
                .unwrap_or_else(|| json!({ "id": "this", "name": "This computer", "kind": "this", "active": true }));
            let signed_in = core.backend().is_some_and(|b| b.account().is_some());
            ok(json!({
                "signed_in": signed_in,
                "active_output": active_output,
                "account_devices": account_devices,
                "network_devices": network_devices,
            }))
        }
        "play_on_device" => {
            let target_q = args["device"].as_str().unwrap_or("");
            let target = match resolve_device(core, target_q) {
                Ok(t) => t,
                Err(e) => return fail(e),
            };
            if let ResolvedDevice::Account(ref o) = target {
                if !o.online {
                    return fail(format!("{} is currently offline.", o.device.name));
                }
                if !o.state.available {
                    return fail(format!("{} is currently busy or in a Jam.", o.device.name));
                }
            }
            let (mut tracks, context) = match resolve_play_tracks(core, args).await {
                Ok(r) => r,
                Err(e) => return fail(e),
            };
            let shuffle = args["shuffle"].as_bool().unwrap_or(false);
            let control = args["control"].as_bool().unwrap_or(false);
            match target {
                ResolvedDevice::Local => {
                    crate::account::release_output(core);
                    crate::jam::use_local_output(core);
                    let first = track_json(core, &tracks[0]);
                    let count = tracks.len();
                    play_all(core, tracks, shuffle, context);
                    ok(json!({ "device": { "id": "this", "name": "This computer", "kind": "local" }, "playing": first, "count": count }))
                }
                ResolvedDevice::Network { id, name, protocol } => {
                    if let Err(e) = crate::jam::select_output_id(core, id.clone()) {
                        return fail(format!("Could not select {name}: {e}"));
                    }
                    let first = track_json(core, &tracks[0]);
                    let count = tracks.len();
                    play_all(core, tracks, shuffle, context);
                    ok(json!({ "device": { "id": id, "name": name, "kind": "network", "protocol": protocol }, "playing": first, "count": count }))
                }
                ResolvedDevice::Account(o) => {
                    if shuffle && tracks.len() > 1 {
                        shuffle_tracks(&mut tracks);
                    }
                    tracks.truncate(100);
                    let first = track_json(core, &tracks[0]);
                    let count = tracks.len();
                    if let Err(e) = send_remote_transfer(core, &o, tracks).await {
                        return fail(e);
                    }
                    if control {
                        crate::account::select_output_now(core, o.device.id.clone());
                    }
                    ok(json!({
                        "device": { "id": o.device.id, "name": o.device.name, "platform": o.device.platform, "kind": "account" },
                        "playing": first,
                        "count": count,
                        "controlling": control,
                    }))
                }
            }
        }
        "select_device" => {
            let target_q = args["device"].as_str().unwrap_or("");
            let target = match resolve_device(core, target_q) {
                Ok(t) => t,
                Err(e) => return fail(e),
            };
            match target {
                ResolvedDevice::Local => match crate::jam::use_local_output_now(core) {
                    Ok(_) => ok(json!({ "device": { "id": "this", "name": "This computer", "kind": "local" } })),
                    Err(e) => fail(e),
                },
                ResolvedDevice::Account(o) => {
                    if !o.online {
                        return fail(format!("{} is currently offline.", o.device.name));
                    }
                    if !o.state.available {
                        return fail(format!("{} is currently busy or in a Jam.", o.device.name));
                    }
                    match crate::jam::select_output_id(core, o.device.id.clone()) {
                        Ok(_) => ok(json!({ "device": { "id": o.device.id, "name": o.device.name, "platform": o.device.platform, "kind": "account" } })),
                        Err(e) => fail(e),
                    }
                }
                ResolvedDevice::Network { id, name, protocol } => match crate::jam::select_output_id(core, id.clone()) {
                    Ok(_) => ok(json!({ "device": { "id": id, "name": name, "protocol": protocol, "kind": "network" } })),
                    Err(e) => fail(e),
                },
            }
        }
        "list_followed" => {
            let artists: Vec<String> = core.st().library.followed.iter().map(|f| f.name.clone()).collect();
            ok(json!({ "artists": artists }))
        }
        "follow_artist" => {
            let name = args["name"].as_str().unwrap_or("").trim().to_string();
            if name.is_empty() {
                return fail("name is empty");
            }
            let want = args["follow"].as_bool().unwrap_or(true);
            crate::commands::set_follow(core, name.clone(), want);
            ok(json!({ "artist": name, "following": want }))
        }
        "list_playlists" => {
            let st = core.st();
            let mut v = vec![json!({ "id": "liked", "name": crate::i18n::tr(core, "library.liked"), "songs": st.library.liked.len() })];
            v.extend(st.library.playlists.iter().map(|p| json!({ "id": p.id, "name": p.name, "songs": p.tracks.len() })));
            ok(json!({ "playlists": v }))
        }
        "get_playlist" | "play_playlist" => {
            let id = args["id"].as_str().unwrap_or("");
            let Some((title, tracks)) = list_of(core, id) else { return fail(format!("No playlist {id}.")) };
            if name == "get_playlist" {
                let songs = tracks_json(core, &tracks[..tracks.len().min(500)]);
                ok(json!({ "id": id, "name": title, "songs": songs, "total": tracks.len() }))
            } else if tracks.is_empty() {
                fail("The playlist is empty.")
            } else {
                play_all(core, tracks, args["shuffle"].as_bool().unwrap_or(false), Some(title.clone()));
                ok(json!(format!("Playing {title}.")))
            }
        }
        "create_playlist" => {
            let name = glyph_core::library::clean_playlist_name(args["name"].as_str().unwrap_or(""));
            if name.is_empty() {
                return fail("name is empty");
            }
            let songs = refs_of(core, args);
            let id = {
                let mut st = core.st();
                let p = st.library.create_playlist(&name);
                p.tracks = songs.clone();
                p.id.clone()
            };
            core.library_changed();
            ok(json!({ "id": id, "name": name, "songs": songs.len() }))
        }
        "rename_playlist" => {
            let id = args["id"].as_str().unwrap_or("").to_string();
            let name = glyph_core::library::clean_playlist_name(args["name"].as_str().unwrap_or(""));
            if core.st().library.playlist(&id).is_none() || name.is_empty() {
                return fail("No such playlist or empty name.");
            }
            core.st().library.rename_playlist(&id, &name);
            core.library_changed();
            ok(json!({ "id": id, "name": name }))
        }
        "delete_playlist" => {
            let id = args["id"].as_str().unwrap_or("").to_string();
            if core.st().library.playlist(&id).is_none() {
                return fail("No such playlist.");
            }
            core.st().library.delete_playlist(&id);
            core.library_changed();
            ok(json!("Deleted."))
        }
        "add_to_playlist" => {
            let id = args["id"].as_str().unwrap_or("").to_string();
            if core.st().library.playlist(&id).is_none() {
                return fail("No such playlist.");
            }
            let songs = refs_of(core, args);
            if songs.is_empty() {
                return fail("No known song refs given.");
            }
            let n = {
                let mut st = core.st();
                songs.into_iter().filter(|t| st.library.add_to_playlist(&id, t.clone())).count()
            };
            core.library_changed();
            ok(json!(format!("Added {n} songs.")))
        }
        "remove_from_playlist" => {
            let id = args["id"].as_str().unwrap_or("").to_string();
            let i = args["index"].as_u64().unwrap_or(u64::MAX) as usize;
            if core.st().library.playlist(&id).is_none_or(|p| i >= p.tracks.len()) {
                return fail("No such playlist or index.");
            }
            core.st().library.remove_from_playlist(&id, i);
            core.library_changed();
            ok(json!("Removed."))
        }
        "like" | "unlike" => {
            let want = name == "like";
            let t = match args["ref"].as_str() {
                Some(r) => find_ref(core, r),
                None => core.st().current.clone(),
            };
            let Some(t) = t else { return fail("No such song.") };
            {
                let mut st = core.st();
                if st.library.is_liked(&t) != want {
                    st.library.toggle_like(&t);
                }
            }
            core.library_changed();
            ok(json!({ "song": track_json(core, &t), "liked": want }))
        }
        "get_lyrics" => {
            let Some(t) = core.st().current.clone() else { return fail("Nothing is playing.") };
            let Some(b) = core.backend() else { return fail("Glyph is still starting; try again in a moment.") };
            match b.lyrics(&t).await {
                Ok(Some(l)) => {
                    let text = if l.synced.is_empty() {
                        l.plain.clone().unwrap_or_default()
                    } else {
                        l.synced.iter().map(|x| x.text.clone()).collect::<Vec<_>>().join("\n")
                    };
                    ok(json!({ "title": t.title, "artist": t.artist, "source": l.provider, "lyrics": text }))
                }
                _ => match crate::plugins::first_lyrics(core) {
                    Some(l) => {
                        let text = if l.synced.is_empty() { l.plain.clone().unwrap_or_default() } else { l.synced.iter().map(|x| x.text.clone()).collect::<Vec<_>>().join("\n") };
                        ok(json!({ "title": t.title, "artist": t.artist, "source": l.provider, "lyrics": text }))
                    }
                    None => fail("No lyrics found for this song."),
                },
            }
        }
        "listening_stats" => {
            use glyph_core::stats::{self, Range};
            if !core.st().config.stats.enabled {
                return fail("Listening statistics are turned off (Settings › Playback › Content).");
            }
            let range = match args["range"].as_str() {
                Some("last_month") => Range::LastMonth,
                Some("year") => Range::Year,
                Some("all") => Range::All,
                _ => Range::ThisMonth,
            };
            let s = core.st().library.stats.summary(range, stats::When::now());
            let artists: Vec<Value> = stats::top_artists(&s, 10).into_iter().map(|(n, a)| json!({ "artist": n, "minutes": a.ms / 60_000, "plays": a.plays })).collect();
            let songs: Vec<Value> = stats::top_tracks(&s, 10).into_iter().map(|x| json!({ "title": x.track.title, "artist": x.track.artist, "minutes": x.ms / 60_000, "plays": x.plays })).collect();
            ok(json!({ "minutes": s.ms / 60_000, "plays": s.plays, "skips": s.skips, "top_artists": artists, "top_songs": songs }))
        }
        "get_settings" => {
            let installed = core.plugins().as_deref().map(crate::plugins::installed_theme_ids).unwrap_or_default();
            let cfg = core.st().config.clone();
            match settings_snapshot_from(&cfg, &installed, args["section"].as_str()) {
                Ok(v) => ok(v),
                Err(e) => fail(e),
            }
        }
        "set_settings" => {
            let installed = core.plugins().as_deref().map(crate::plugins::installed_theme_ids).unwrap_or_default();
            let (mut cfg, prev_shuffle, prev_repeat) = {
                let st = core.st();
                (st.config.clone(), st.config.player.shuffle, st.config.player.repeat.clone())
            };
            let updated = match apply_settings_patch_with(&mut cfg, &installed, args) {
                Ok(u) => u,
                Err(e) => return fail(e),
            };
            let new_shuffle = cfg.player.shuffle;
            let new_repeat = cfg.player.repeat.clone();
            if let Err(e) = crate::commands::commit_config(core, cfg.clone(), false).await {
                return fail(e);
            }
            if new_shuffle != prev_shuffle {
                crate::commands::set_shuffle(core, new_shuffle);
            }
            if new_repeat != prev_repeat {
                crate::commands::set_repeat(core, new_repeat);
            }
            let final_cfg = core.st().config.clone();
            match settings_snapshot_from(&final_cfg, &installed, None) {
                Ok(mut v) => {
                    v["updated"] = json!(updated);
                    ok(v)
                }
                Err(e) => fail(e),
            }
        }
        // The docs: answered by their own MCP server (docs.glyph.my).
        "docs_search" | "docs_read" => {
            let Some(b) = core.backend() else { return fail("Glyph is still starting; try again in a moment.") };
            let tool = if name == "docs_search" { "search_docs" } else { "read_doc" };
            match b.docs_tool(tool, args.clone()).await {
                Ok(v) => v,
                Err(e) => fail(format!("The Glyph docs could not be reached ({e}). They are also at https://docs.glyph.my")),
            }
        }
        other => fail(format!("unknown tool {other}")),
    }
}

const SUPPORTED_LANGS: &[&str] = &["en", "af", "de", "es", "fr", "it", "pl", "pt", "ru", "tr", "uk"];
const BUILTIN_THEMES: &[&str] = &["glyph", "ember", "ocean", "forest", "violet", "crimson", "mono"];
const GLOW_SHAPES: &[&str] = &["auto", "strahl", "band", "sonnenaufgang", "horizont", "solid"];

fn settings_snapshot_from(cfg: &glyph_core::config::Config, installed_themes: &[String], section: Option<&str>) -> Result<Value, String> {
    let pick = section.unwrap_or("all").trim().to_ascii_lowercase();
    let sections = serde_json::Map::from_iter([
        ("general".to_string(), json!({ "language": if cfg.language.is_empty() { "system" } else { cfg.language.as_str() } })),
        (
            "appearance".to_string(),
            json!({
                "theme": cfg.visuals.theme,
                "glow_shape": if cfg.visuals.glow_shape.is_empty() { "auto" } else { cfg.visuals.glow_shape.as_str() },
                "intensity": cfg.visuals.intensity,
                "music_reactivity": cfg.visuals.music_reactivity,
                "ascii_covers": cfg.visuals.ascii_covers,
                "ascii_effects": cfg.visuals.ascii_effects,
                "startup_animation": cfg.visuals.startup_animation,
                "throttle_unfocused": cfg.visuals.throttle_unfocused,
            }),
        ),
        (
            "audio".to_string(),
            json!({
                "quality": cfg.audio.quality,
                "download_quality": cfg.audio.download_quality,
                "crossfade_s": cfg.audio.crossfade_s,
                "level": cfg.audio.level,
                "normalize": cfg.audio.normalize,
                "hide_explicit": cfg.audio.hide_explicit,
                "eq_enabled": cfg.audio.eq_enabled,
                "eq_preset": cfg.audio.eq_preset,
                "eq_gains": glyph_core::eq::sanitize(&cfg.audio.eq_gains),
                "eq_bass_db": cfg.audio.eq_bass_db,
                "eq_headroom_db": cfg.audio.eq_headroom_db,
                "eq_user_presets": cfg.audio.eq_user_presets.iter().map(|p| &p.name).collect::<Vec<_>>(),
            }),
        ),
        (
            "playback".to_string(),
            json!({
                "shuffle_style": cfg.playback.shuffle,
                "skip_unplayable": cfg.playback.skip_unplayable,
                "patient_search": cfg.playback.patient_search,
            }),
        ),
        (
            "player".to_string(),
            json!({
                "shuffle": cfg.player.shuffle,
                "repeat": cfg.player.repeat,
                "show_queue": cfg.player.show_queue,
                "mini_large": cfg.player.mini_large,
            }),
        ),
        ("stats".to_string(), json!({ "enabled": cfg.stats.enabled })),
        (
            "sharing".to_string(),
            json!({
                "glyph_links": cfg.sharing.glyph_links,
                "profile_sections": cfg.sharing.profile_sections,
            }),
        ),
        (
            "integrations".to_string(),
            json!({
                "discord_presence": cfg.integrations.discord_presence,
                "discord_client_id": cfg.integrations.discord_client_id,
            }),
        ),
        (
            "providers".to_string(),
            json!({
                "enabled": cfg.providers.enabled,
                "priority": cfg.providers.priority,
                "youtube_daily_searches": cfg.providers.youtube_daily_searches,
            }),
        ),
        (
            "updates".to_string(),
            json!({
                "check_on_start": cfg.updates.check_on_start,
                "auto_install": cfg.updates.auto_install,
                "channel": cfg.updates.channel,
            }),
        ),
        ("lists".to_string(), json!({ "sort": cfg.lists.sort })),
    ]);

    let selected = if pick.is_empty() || pick == "all" {
        Value::Object(sections)
    } else if let Some(v) = sections.get(&pick) {
        json!({ pick: v })
    } else {
        return Err(format!("Unknown settings section \"{}\".", section.unwrap_or("")));
    };

    let mut themes: Vec<String> = BUILTIN_THEMES.iter().map(|s| (*s).to_string()).collect();
    themes.extend(installed_themes.iter().cloned());
    let mut eq_presets: Vec<String> = glyph_core::eq::BUILTIN_PRESETS.iter().map(|&(k, _)| k.to_string()).collect();
    eq_presets.extend(cfg.audio.eq_user_presets.iter().map(|p| format!("user:{}", p.name)));
    let mut langs = vec!["system".to_string()];
    langs.extend(SUPPORTED_LANGS.iter().map(|s| (*s).to_string()));

    Ok(json!({
        "settings": selected,
        "available": {
            "languages": langs,
            "themes": themes,
            "glow_shapes": GLOW_SHAPES,
            "qualities": ["auto", "low", "standard", "high"],
            "levels": ["loud", "normal", "quiet"],
            "eq_presets": eq_presets,
            "shuffle_styles": ["smart", "random"],
            "repeats": ["off", "all", "one"],
            "profile_sections": glyph_protocol::PROFILE_SECTIONS,
            "providers": glyph_protocol::ProviderId::ALL,
            "channels": ["stable", "prerelease"],
            "list_sorts": ["custom", "title", "artist", "album", "duration"],
        }
    }))
}

fn parse_bool(v: &Value, field: &str) -> Result<bool, String> {
    v.as_bool().ok_or_else(|| format!("{field} must be a boolean"))
}

fn parse_clamped_f32(v: &Value, field: &str, min: f32, max: f32) -> Result<f32, String> {
    let n = v.as_f64().ok_or_else(|| format!("{field} must be a number"))? as f32;
    if !n.is_finite() {
        return Err(format!("{field} must be finite"));
    }
    Ok(n.clamp(min, max))
}

fn parse_audio_quality(v: &Value, field: &str) -> Result<glyph_core::config::AudioQuality, String> {
    serde_json::from_value(json!(v.as_str().ok_or_else(|| format!("{field} must be a string"))?.trim().to_ascii_lowercase()))
        .map_err(|_| format!("Invalid {field} (expected auto, low, standard, or high)."))
}

fn parse_provider_list(v: &Value, field: &str) -> Result<Vec<glyph_protocol::ProviderId>, String> {
    let arr = v.as_array().ok_or_else(|| format!("{field} must be an array"))?;
    let mut out = Vec::new();
    for item in arr {
        let s = item.as_str().ok_or_else(|| format!("{field} items must be strings"))?.trim().to_ascii_lowercase();
        let id: glyph_protocol::ProviderId = serde_json::from_value(json!(s)).map_err(|_| format!("Unknown provider \"{s}\"."))?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

fn apply_settings_patch_with(cfg: &mut glyph_core::config::Config, installed_themes: &[String], args: &Value) -> Result<Vec<String>, String> {
    let mut updated = Vec::new();

    if let Some(v) = args.get("general").and_then(|g| g.get("language")).or_else(|| args.get("language")) {
        let code = v.as_str().ok_or("general.language must be a string")?.trim().to_ascii_lowercase();
        if code.is_empty() || code == "system" {
            cfg.language.clear();
        } else if SUPPORTED_LANGS.contains(&code.as_str()) {
            cfg.language = code;
        } else {
            return Err(format!("Unsupported language \"{code}\"."));
        }
        updated.push("general.language".into());
    }

    let app = args.get("appearance").or_else(|| args.get("visuals"));
    if let Some(v) = app.and_then(|a| a.get("theme")).or_else(|| args.get("theme")) {
        let t = v.as_str().ok_or("appearance.theme must be a string")?.trim();
        let ok_builtin = BUILTIN_THEMES.iter().any(|b| b.eq_ignore_ascii_case(t));
        let ok_plugin = installed_themes.iter().any(|p| p == t);
        if ok_builtin {
            cfg.visuals.theme = t.to_ascii_lowercase();
        } else if ok_plugin || t.starts_with("plugin:") && installed_themes.is_empty() {
            cfg.visuals.theme = t.to_string();
        } else {
            return Err(format!("Unknown theme \"{t}\"."));
        }
        updated.push("appearance.theme".into());
    }
    if let Some(v) = app.and_then(|a| a.get("glow_shape")).or_else(|| args.get("glow_shape")) {
        let g = v.as_str().ok_or("appearance.glow_shape must be a string")?.trim().to_ascii_lowercase();
        if g.is_empty() || g == "auto" {
            cfg.visuals.glow_shape.clear();
        } else if GLOW_SHAPES.contains(&g.as_str()) {
            cfg.visuals.glow_shape = g;
        } else {
            return Err(format!("Unknown glow_shape \"{g}\"."));
        }
        updated.push("appearance.glow_shape".into());
    }
    if let Some(a) = app.and_then(Value::as_object) {
        if let Some(v) = a.get("intensity") {
            cfg.visuals.intensity = parse_clamped_f32(v, "appearance.intensity", 0.0, 1.0)?;
            updated.push("appearance.intensity".into());
        }
        if let Some(v) = a.get("music_reactivity") {
            cfg.visuals.music_reactivity = parse_clamped_f32(v, "appearance.music_reactivity", 0.0, 1.0)?;
            updated.push("appearance.music_reactivity".into());
        }
        if let Some(v) = a.get("ascii_covers") {
            cfg.visuals.ascii_covers = parse_bool(v, "appearance.ascii_covers")?;
            updated.push("appearance.ascii_covers".into());
        }
        if let Some(v) = a.get("ascii_effects") {
            cfg.visuals.ascii_effects = parse_bool(v, "appearance.ascii_effects")?;
            updated.push("appearance.ascii_effects".into());
        }
        if let Some(v) = a.get("startup_animation") {
            cfg.visuals.startup_animation = parse_bool(v, "appearance.startup_animation")?;
            updated.push("appearance.startup_animation".into());
        }
        if let Some(v) = a.get("throttle_unfocused") {
            cfg.visuals.throttle_unfocused = parse_bool(v, "appearance.throttle_unfocused")?;
            updated.push("appearance.throttle_unfocused".into());
        }
    }

    if let Some(a) = args.get("audio").and_then(Value::as_object) {
        if let Some(v) = a.get("quality") {
            cfg.audio.quality = parse_audio_quality(v, "audio.quality")?;
            updated.push("audio.quality".into());
        }
        if let Some(v) = a.get("download_quality") {
            cfg.audio.download_quality = parse_audio_quality(v, "audio.download_quality")?;
            updated.push("audio.download_quality".into());
        }
        if let Some(v) = a.get("crossfade_s") {
            cfg.audio.crossfade_s = parse_clamped_f32(v, "audio.crossfade_s", 0.0, 20.0)?.round() as u32;
            updated.push("audio.crossfade_s".into());
        }
        if let Some(v) = a.get("level") {
            let s = v.as_str().ok_or("audio.level must be a string")?.trim().to_ascii_lowercase();
            cfg.audio.level = serde_json::from_value(json!(s)).map_err(|_| format!("Invalid audio.level \"{s}\"."))?;
            updated.push("audio.level".into());
        }
        if let Some(v) = a.get("normalize") {
            cfg.audio.normalize = parse_bool(v, "audio.normalize")?;
            updated.push("audio.normalize".into());
        }
        if let Some(v) = a.get("hide_explicit") {
            cfg.audio.hide_explicit = parse_bool(v, "audio.hide_explicit")?;
            updated.push("audio.hide_explicit".into());
        }
        if let Some(v) = a.get("eq_gains") {
            let arr = v.as_array().ok_or("audio.eq_gains must be an array of 10 numbers")?;
            if arr.len() != glyph_core::eq::BANDS {
                return Err(format!("audio.eq_gains must have exactly {} numbers", glyph_core::eq::BANDS));
            }
            let mut raw = Vec::with_capacity(glyph_core::eq::BANDS);
            for (i, item) in arr.iter().enumerate() {
                raw.push(parse_clamped_f32(item, &format!("audio.eq_gains[{i}]"), -glyph_core::eq::MAX_DB, glyph_core::eq::MAX_DB)?);
            }
            cfg.audio.eq_gains = glyph_core::eq::sanitize(&raw).to_vec();
            if !a.contains_key("eq_enabled") {
                cfg.audio.eq_enabled = true;
            }
            if !a.contains_key("eq_preset") {
                cfg.audio.eq_preset = "custom".into();
            }
            updated.push("audio.eq_gains".into());
        }
        if let Some(v) = a.get("eq_preset") {
            let key = v.as_str().ok_or("audio.eq_preset must be a string")?.trim();
            if key.eq_ignore_ascii_case("custom") {
                cfg.audio.eq_preset = "custom".into();
            } else if let Some((canon, gains)) = glyph_core::eq::resolve_preset(key, &cfg.audio.eq_user_presets) {
                cfg.audio.eq_preset = canon;
                if !a.contains_key("eq_gains") {
                    cfg.audio.eq_gains = gains.to_vec();
                }
                if !a.contains_key("eq_enabled") {
                    cfg.audio.eq_enabled = true;
                }
            } else {
                return Err(format!("Unknown EQ preset \"{key}\"."));
            }
            updated.push("audio.eq_preset".into());
        }
        if let Some(v) = a.get("eq_enabled") {
            cfg.audio.eq_enabled = parse_bool(v, "audio.eq_enabled")?;
            updated.push("audio.eq_enabled".into());
        }
        if let Some(v) = a.get("eq_bass_db") {
            cfg.audio.eq_bass_db = (parse_clamped_f32(v, "audio.eq_bass_db", -12.0, 12.0)? * 2.0).round() / 2.0;
            updated.push("audio.eq_bass_db".into());
        }
        if let Some(v) = a.get("eq_headroom_db") {
            cfg.audio.eq_headroom_db = (parse_clamped_f32(v, "audio.eq_headroom_db", 0.0, 12.0)? * 2.0).round() / 2.0;
            updated.push("audio.eq_headroom_db".into());
        }
    }

    if let Some(pb) = args.get("playback").and_then(Value::as_object) {
        if let Some(v) = pb.get("shuffle_style").or_else(|| pb.get("shuffle")) {
            let s = v.as_str().ok_or("playback.shuffle_style must be a string")?.trim().to_ascii_lowercase();
            cfg.playback.shuffle = serde_json::from_value(json!(s)).map_err(|_| format!("Invalid playback.shuffle_style \"{s}\"."))?;
            updated.push("playback.shuffle_style".into());
        }
        if let Some(v) = pb.get("skip_unplayable") {
            cfg.playback.skip_unplayable = parse_bool(v, "playback.skip_unplayable")?;
            updated.push("playback.skip_unplayable".into());
        }
        if let Some(v) = pb.get("patient_search") {
            cfg.playback.patient_search = parse_bool(v, "playback.patient_search")?;
            updated.push("playback.patient_search".into());
        }
    }

    if let Some(pl) = args.get("player").and_then(Value::as_object) {
        if let Some(v) = pl.get("shuffle") {
            cfg.player.shuffle = parse_bool(v, "player.shuffle")?;
            updated.push("player.shuffle".into());
        }
        if let Some(v) = pl.get("repeat") {
            let r = v.as_str().ok_or("player.repeat must be a string")?.trim().to_ascii_lowercase();
            if !matches!(r.as_str(), "off" | "all" | "one") {
                return Err(format!("Invalid player.repeat \"{r}\"."));
            }
            cfg.player.repeat = r;
            updated.push("player.repeat".into());
        }
        if let Some(v) = pl.get("show_queue") {
            cfg.player.show_queue = parse_bool(v, "player.show_queue")?;
            updated.push("player.show_queue".into());
        }
        if let Some(v) = pl.get("mini_large") {
            cfg.player.mini_large = parse_bool(v, "player.mini_large")?;
            updated.push("player.mini_large".into());
        }
    }

    if let Some(v) = args.get("stats").and_then(|s| s.get("enabled")) {
        cfg.stats.enabled = parse_bool(v, "stats.enabled")?;
        updated.push("stats.enabled".into());
    }

    if let Some(sh) = args.get("sharing").and_then(Value::as_object) {
        if let Some(v) = sh.get("glyph_links") {
            cfg.sharing.glyph_links = parse_bool(v, "sharing.glyph_links")?;
            updated.push("sharing.glyph_links".into());
        }
        if let Some(v) = sh.get("profile_sections") {
            let arr = v.as_array().ok_or("sharing.profile_sections must be an array")?;
            let mut order = Vec::new();
            for item in arr {
                let s = item.as_str().ok_or("sharing.profile_sections items must be strings")?.trim().to_ascii_lowercase();
                if !glyph_protocol::PROFILE_SECTIONS.contains(&s.as_str()) {
                    return Err(format!("Unknown profile_section \"{s}\"."));
                }
                order.push(s);
            }
            cfg.sharing.profile_sections = glyph_core::config::normalize_profile_sections(&order);
            updated.push("sharing.profile_sections".into());
        }
    }

    if let Some(intg) = args.get("integrations").and_then(Value::as_object) {
        if let Some(v) = intg.get("discord_presence") {
            cfg.integrations.discord_presence = parse_bool(v, "integrations.discord_presence")?;
            updated.push("integrations.discord_presence".into());
        }
        if let Some(v) = intg.get("discord_client_id") {
            cfg.integrations.discord_client_id = v.as_str().ok_or("integrations.discord_client_id must be a string")?.trim().to_string();
            updated.push("integrations.discord_client_id".into());
        }
    }

    if let Some(prov) = args.get("providers").and_then(Value::as_object) {
        if let Some(v) = prov.get("enabled") {
            cfg.providers.enabled = parse_provider_list(v, "providers.enabled")?;
            updated.push("providers.enabled".into());
        }
        if let Some(v) = prov.get("priority") {
            let mut p = parse_provider_list(v, "providers.priority")?;
            for id in &cfg.providers.priority {
                if !p.contains(id) {
                    p.push(*id);
                }
            }
            cfg.providers.priority = p;
            updated.push("providers.priority".into());
        }
        if let Some(v) = prov.get("youtube_daily_searches") {
            cfg.providers.youtube_daily_searches = parse_clamped_f32(v, "providers.youtube_daily_searches", 0.0, 100_000.0)?.round() as u32;
            updated.push("providers.youtube_daily_searches".into());
        }
    }

    if let Some(upd) = args.get("updates").and_then(Value::as_object) {
        if let Some(v) = upd.get("check_on_start") {
            cfg.updates.check_on_start = parse_bool(v, "updates.check_on_start")?;
            updated.push("updates.check_on_start".into());
        }
        if let Some(v) = upd.get("auto_install") {
            cfg.updates.auto_install = parse_bool(v, "updates.auto_install")?;
            updated.push("updates.auto_install".into());
        }
        if let Some(v) = upd.get("channel") {
            let ch = v.as_str().ok_or("updates.channel must be a string")?.trim().to_ascii_lowercase();
            if !matches!(ch.as_str(), "stable" | "prerelease") {
                return Err(format!("Invalid updates.channel \"{ch}\"."));
            }
            cfg.updates.channel = ch;
            updated.push("updates.channel".into());
        }
    }

    if let Some(map) = args.get("lists").and_then(|l| l.get("sort")).and_then(Value::as_object) {
        for (k, v) in map {
            let s = v.as_str().ok_or("lists.sort values must be strings")?.trim().to_ascii_lowercase();
            let mode: glyph_core::config::ListSort =
                serde_json::from_value(json!(s)).map_err(|_| format!("Invalid lists.sort value \"{s}\" for \"{k}\"."))?;
            cfg.lists.sort.insert(k.clone(), mode);
        }
        updated.push("lists.sort".into());
    }

    if updated.is_empty() {
        return Err("Provide at least one setting section or field to update.".into());
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glyph_core::config::{AudioQuality, Config, ListSort, ShuffleStyle};

    #[test]
    fn settings_permissions_enforce_none_read_write() {
        use glyph_protocol::mcp::{Access, Permissions};
        let mut p = Permissions::default();

        p.settings = Access::None;
        assert!(!p.allows("get_settings"));
        assert!(!p.allows("set_settings"));

        p.settings = Access::Read;
        assert!(p.allows("get_settings"));
        assert!(!p.allows("set_settings"));

        p.settings = Access::Write;
        assert!(p.allows("get_settings"));
        assert!(p.allows("set_settings"));
    }

    #[test]
    fn settings_snapshot_filters_and_exposes_available() {
        let cfg = Config::default();
        let installed = vec!["plugin:midnight".to_string()];
        let all = settings_snapshot_from(&cfg, &installed, None).unwrap();
        assert!(all["settings"]["appearance"].is_object());
        assert!(all["settings"]["audio"].is_object());
        assert!(all["available"]["themes"].as_array().unwrap().iter().any(|t| t == "plugin:midnight"));

        let only = settings_snapshot_from(&cfg, &installed, Some("appearance")).unwrap();
        assert_eq!(only["settings"].as_object().unwrap().len(), 1);
        assert!(only["settings"]["appearance"].is_object());
        assert!(settings_snapshot_from(&cfg, &installed, Some("nope")).is_err());
    }

    #[test]
    fn settings_patch_updates_cross_device_fields_and_preserves_local() {
        let mut cfg = Config::default();
        cfg.audio.output_device = Some("Desk DAC".into());
        cfg.player.volume = 0.42;
        cfg.storage.download_dir = Some("C:/Music".into());
        let installed = vec!["plugin:midnight".to_string()];

        let updated = apply_settings_patch_with(
            &mut cfg,
            &installed,
            &json!({
                "general": { "language": "de" },
                "appearance": {
                    "theme": "ocean",
                    "glow_shape": "solid",
                    "intensity": 1.5,
                    "ascii_covers": false
                },
                "audio": {
                    "quality": "high",
                    "eq_preset": "rock",
                    "crossfade_s": 5
                },
                "playback": { "shuffle_style": "random", "patient_search": true },
                "sharing": { "profile_sections": ["recent"] },
                "lists": { "sort": { "liked": "title" } }
            }),
        )
        .unwrap();

        assert!(updated.contains(&"appearance.theme".to_string()));
        assert_eq!(cfg.language, "de");
        assert_eq!(cfg.visuals.theme, "ocean");
        assert_eq!(cfg.visuals.glow_shape, "solid");
        assert_eq!(cfg.visuals.intensity, 1.0);
        assert!(!cfg.visuals.ascii_covers);
        assert_eq!(cfg.audio.quality, AudioQuality::High);
        assert!(cfg.audio.eq_enabled);
        assert_eq!(cfg.audio.eq_preset, "rock");
        assert_eq!(cfg.audio.eq_gains[0], 4.0);
        assert_eq!(cfg.audio.crossfade_s, 5);
        assert_eq!(cfg.playback.shuffle, ShuffleStyle::Random);
        assert!(cfg.playback.patient_search);
        assert_eq!(cfg.sharing.profile_sections, vec!["recent".to_string(), "playlists".to_string()]);
        assert_eq!(cfg.lists.sort.get("liked"), Some(&ListSort::Title));

        // Device-local settings must never be modified.
        assert_eq!(cfg.audio.output_device.as_deref(), Some("Desk DAC"));
        assert!((cfg.player.volume - 0.42).abs() < 1e-6);
        assert_eq!(cfg.storage.download_dir.as_deref(), Some("C:/Music"));
    }

    #[test]
    fn settings_patch_rejects_invalid_values() {
        let mut cfg = Config::default();
        assert!(apply_settings_patch_with(&mut cfg, &[], &json!({})).is_err());
        assert!(apply_settings_patch_with(&mut cfg, &[], &json!({ "appearance": { "theme": "nonexistent" } })).is_err());
        assert!(apply_settings_patch_with(&mut cfg, &[], &json!({ "audio": { "eq_gains": [1.0, 2.0] } })).is_err());
    }
}
