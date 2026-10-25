//! The MCP tools run inside the app (see `crate::mcp` for the transport):
//! playback, queue, search, playlists, likes, lyrics and statistics. Plugin
//! code uses the same tools (see `app::plugin_api`).
//!
//! Songs travel between tools as a `ref` (the app's song key); every song an
//! answer mentions is remembered so a later call can use its ref.

use crate::app::{track_key, Glyph, ListRef, View};
use crate::guard::{guard, guard_async};
use crate::mcp::McpRequest;
use glyph_core::config::McpMode;
use glyph_core::RepeatMode;
use glyph_protocol::mcp::text_result;
use glyph_protocol::{SearchEvent, Track};
use gpui::Context;
use serde_json::{json, Value};
use std::time::Duration;

/// MCP state of the app.
pub struct McpState {
    pub tx: flume::Sender<McpRequest>,
    rx: flume::Receiver<McpRequest>,
    local: Option<crate::mcp::LocalServer>,
    /// Songs mentioned in answers, by ref.
    refs: std::collections::HashMap<String, Track>,
    /// Bumped to stop a running relay loop.
    relay_gen: u64,
    pub relay_running: bool,
    /// Last problem (port taken, server refused), for the settings page.
    pub error: Option<String>,
    /// MCP token for the server endpoint, as last created (shown once).
    pub server_token: Option<glyph_protocol::mcp::TokenResponse>,
    pub token_loading: bool,
    /// Assistants asking for access (oldest first); the app asks the user.
    pub approvals: Vec<glyph_protocol::mcp::Approval>,
}

impl Default for McpState {
    fn default() -> Self {
        let (tx, rx) = flume::unbounded();
        Self { tx, rx, local: None, refs: Default::default(), relay_gen: 0, relay_running: false, error: None, server_token: None, token_loading: false, approvals: Vec::new() }
    }
}

fn ok(v: Value) -> Value {
    text_result(&v, false)
}

fn fail(msg: impl Into<String>) -> Value {
    text_result(&Value::String(msg.into()), true)
}

impl Glyph {
    /// Starts / stops the local server and the server relay as the settings say.
    pub fn sync_mcp(&mut self, cx: &mut Context<Self>) {
        let cfg = self.config.mcp.clone();
        let want_local = cfg.enabled && cfg.mode == McpMode::Local;
        let want_relay = cfg.enabled && cfg.mode == McpMode::Server && self.signed_in().is_some();
        if want_local {
            if self.config.mcp.token.is_empty() {
                self.config.mcp.token = crate::mcp::new_token();
                self.save_config(cx);
            }
            if self.mcp.local.as_ref().map(|s| s.port) != Some(cfg.port) {
                self.mcp.local = None;
                match crate::mcp::start_local(cfg.port, self.config.mcp.token.clone(), self.mcp.tx.clone()) {
                    Ok(s) => {
                        self.mcp.local = Some(s);
                        self.mcp.error = None;
                    }
                    Err(e) => {
                        tracing::warn!("MCP server could not start: {e}");
                        self.mcp.error = Some(e.to_string());
                    }
                }
            }
        } else {
            self.mcp.local = None;
        }
        if want_relay && !self.mcp.relay_running {
            self.start_mcp_relay(cx);
        } else if !want_relay && self.mcp.relay_running {
            self.mcp.relay_gen += 1;
            self.mcp.relay_running = false;
        }
        cx.notify();
    }

    /// Settings > Integrations: MCP on / off.
    pub fn set_mcp_enabled(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.mcp.enabled = on;
        self.save_config(cx);
        self.sync_mcp(cx);
    }

    /// Settings › Integrations › AI assistants: none / read / write per area.
    pub fn set_mcp_access(&mut self, area: glyph_protocol::mcp::Area, access: glyph_protocol::mcp::Access, cx: &mut Context<Self>) {
        self.config.mcp.access.set(area, access);
        self.save_config(cx);
        cx.notify();
    }

    pub fn set_mcp_mode(&mut self, mode: McpMode, cx: &mut Context<Self>) {
        self.config.mcp.mode = mode;
        self.save_config(cx);
        self.sync_mcp(cx);
    }

    /// New local token (the old one stops working at once).
    pub fn rotate_mcp_token(&mut self, cx: &mut Context<Self>) {
        self.config.mcp.token = crate::mcp::new_token();
        self.mcp.local = None;
        self.save_config(cx);
        self.sync_mcp(cx);
        self.toast_info(crate::i18n::t("mcp.token_rotated"));
    }

    /// Asks the server for a token for its MCP endpoint.
    pub fn request_mcp_server_token(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        if self.mcp.token_loading {
            return;
        }
        self.mcp.token_loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.mcp_token()).await;
            this.update(cx, |s, cx| {
                s.mcp.token_loading = false;
                match r {
                    Ok(t) => {
                        s.mcp.server_token = Some(t);
                        s.mcp.error = None;
                    }
                    Err(e) => s.mcp.error = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The user answered an assistant's access request.
    pub fn answer_mcp_approval(&mut self, id: &str, allow: bool, cx: &mut Context<Self>) {
        self.mcp.approvals.retain(|a| a.id != id);
        let Some(b) = self.backend.clone() else { return };
        let d = glyph_protocol::mcp::Decision { id: id.to_string(), allow };
        cx.spawn(async move |this, cx| {
            let r = guard_async(b.mcp_approve(d)).await;
            this.update(cx, |s, cx| {
                match r {
                    Ok(()) if allow => s.toast(crate::i18n::t("mcp.allowed")),
                    Ok(()) => s.toast_info(crate::i18n::t("mcp.denied")),
                    Err(e) => s.toast_info(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Fetches tool calls queued on the server (long poll), runs them and
    /// sends the results back, until MCP is turned off or the mode changes.
    fn start_mcp_relay(&mut self, cx: &mut Context<Self>) {
        let Some(b) = self.backend.clone() else { return };
        self.mcp.relay_gen += 1;
        self.mcp.relay_running = true;
        let gen = self.mcp.relay_gen;
        let calls = self.mcp.tx.clone();
        cx.spawn(async move |this, cx| {
            let mut failures = 0u32;
            loop {
                let alive = this.update(cx, |s, _| s.mcp.relay_gen == gen).unwrap_or(false);
                if !alive {
                    break;
                }
                match guard_async(b.mcp_pull()).await {
                    Ok(pull) => {
                        failures = 0;
                        // A poll may complete after the user disabled MCP or switched modes.
                        let alive = this.update(cx, |s, cx| {
                            if s.mcp.relay_gen != gen {
                                return false;
                            }
                            if s.mcp.error.take().is_some() {
                                cx.notify();
                            }
                            true
                        }).unwrap_or(false);
                        if !alive {
                            break;
                        }
                        if !pull.approvals.is_empty() {
                            let _ = this.update(cx, |s, cx| {
                                for a in pull.approvals.clone() {
                                    if !s.mcp.approvals.iter().any(|x| x.id == a.id) {
                                        s.mcp.approvals.push(a);
                                    }
                                }
                                cx.activate(true);
                                cx.notify();
                            });
                        }
                        for call in pull.calls {
                            let (b, calls) = (b.clone(), calls.clone());
                            // Run on a worker thread: `crate::mcp::call` waits for the UI thread.
                            cx.background_executor()
                                .spawn(async move {
                                    let result = crate::mcp::call(call.name, call.arguments, &calls);
                                    let _ = guard_async(b.mcp_result(glyph_protocol::mcp::ResultRequest { id: call.id, result })).await;
                                })
                                .detach();
                        }
                    }
                    Err(e) => {
                        failures += 1;
                        tracing::info!("MCP relay: {e}");
                        let _ = this.update(cx, |s, cx| {
                            s.mcp.error = Some(e.clone());
                            cx.notify();
                        });
                        // Back off: 2, 4, 8 … 60 s.
                        let wait = (1u64 << failures.min(6)).min(60);
                        cx.background_executor().timer(Duration::from_secs(wait)).await;
                    }
                }
            }
        })
        .detach();
    }

    /// Runs the tool calls that arrived since the last frame.
    pub fn poll_mcp(&mut self, cx: &mut Context<Self>) -> bool {
        let reqs: Vec<McpRequest> = self.mcp.rx.try_iter().collect();
        let any = !reqs.is_empty();
        for r in reqs {
            // Only assistants come through here (plugins call `mcp_call`
            // directly under their own permissions).
            let access = self.config.mcp.access;
            if r.name == glyph_protocol::mcp::LIST_TOOLS {
                let _ = r.reply.send(json!({ "tools": access.tools() }));
                continue;
            }
            if !access.allows(&r.name) {
                tracing::info!(tool = %r.name, "MCP tool call refused by the AI assistant settings");
                let _ = r.reply.send(fail(format!(
                    "The user does not allow AI assistants to use `{}` (Glyph: Settings › Integrations › AI assistants).",
                    r.name
                )));
                continue;
            }
            tracing::info!(tool = %r.name, "MCP tool call");
            let reply = r.reply.clone();
            if let Some(v) = self.mcp_call(&r.name, &r.args, r.reply, cx) {
                let _ = reply.send(v);
            }
        }
        any
    }

    pub(crate) fn mcp_track(&mut self, t: &Track) -> Value {
        let r = track_key(t);
        if self.mcp.refs.len() > 3000 {
            self.mcp.refs.clear();
        }
        self.mcp.refs.insert(r.clone(), t.clone());
        json!({
            "ref": r,
            "title": t.title,
            "artist": t.artist,
            "album": t.album,
            "duration_s": t.duration_ms.map(|ms| ms / 1000),
            "explicit": t.explicit,
            "liked": self.is_liked(t),
        })
    }

    fn mcp_tracks(&mut self, list: &[Track]) -> Vec<Value> {
        list.iter().map(|t| self.mcp_track(t)).collect()
    }

    /// The song a call names by `ref` (any song an answer mentioned, the
    /// queue, the library).
    fn mcp_ref(&self, r: &str) -> Option<Track> {
        if let Some(t) = self.mcp.refs.get(r) {
            return Some(t.clone());
        }
        let hit = |t: &&Track| track_key(t) == r;
        self.current.iter().find(hit).cloned()
            .or_else(|| self.upcoming().iter().find(hit).cloned())
            .or_else(|| self.library.liked.iter().find(hit).cloned())
            .or_else(|| self.library.playlists.iter().flat_map(|p| p.tracks.iter()).find(hit).cloned())
            .or_else(|| self.search.results.iter().find(hit).cloned())
    }

    fn mcp_list(&self, id: &str) -> Option<ListRef> {
        match id {
            "liked" => Some(ListRef::Liked),
            "recent" => Some(ListRef::Recent),
            id => self.library.playlist(id).map(|_| ListRef::User(id.to_string())),
        }
    }

    /// Searches and hands the songs (best first) to `then`; the call is
    /// answered from there.
    fn mcp_search(&mut self, query: String, limit: usize, cx: &mut Context<Self>, then: impl FnOnce(&mut Glyph, Vec<Track>, &mut Context<Glyph>) + 'static) {
        let Some(b) = self.backend.clone() else {
            then(self, Vec::new(), cx);
            return;
        };
        cx.spawn(async move |this, cx| {
            let mut found: Vec<Track> = Vec::new();
            if let Some(mut stream) = guard(|| b.search(&query)) {
                use futures::StreamExt;
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                loop {
                    let left = deadline.saturating_duration_since(std::time::Instant::now());
                    let next = futures::FutureExt::fuse(stream.next());
                    let timer = futures::FutureExt::fuse(cx.background_executor().timer(left));
                    futures::pin_mut!(next, timer);
                    let ev = futures::select! {
                        ev = next => ev,
                        _ = timer => None,
                    };
                    match ev {
                        Some(SearchEvent::Results { tracks, .. }) => {
                            for t in tracks {
                                if !found.iter().any(|x| x.source == t.source && x.id == t.id) {
                                    found.push(t);
                                }
                            }
                        }
                        Some(SearchEvent::Done) | None => break,
                        _ => {}
                    }
                }
            }
            this.update(cx, |s, cx| {
                let (plain, _) = glyph_core::taste::parse_query(&query);
                let taste = s.taste();
                found.retain(|t| s.allowed(t) && !s.is_hidden_artist(&t.artist));
                found.sort_by(|a, b| glyph_core::taste::score(b, &plain, &taste).partial_cmp(&glyph_core::taste::score(a, &plain, &taste)).unwrap_or(std::cmp::Ordering::Equal));
                found.truncate(limit.max(1));
                then(s, found, cx);
            })
            .ok();
        })
        .detach();
    }

    /// The song of a call: its `ref`, else the best hit for `query`.
    fn mcp_song(&mut self, args: &Value, cx: &mut Context<Self>, then: impl FnOnce(&mut Glyph, Option<Track>, &mut Context<Glyph>) + 'static) {
        if let Some(t) = args["ref"].as_str().and_then(|r| self.mcp_ref(r)) {
            then(self, Some(t), cx);
            return;
        }
        match args["query"].as_str().map(str::trim).filter(|q| !q.is_empty()) {
            Some(q) => self.mcp_search(q.to_string(), 1, cx, move |s, found, cx| then(s, found.into_iter().next(), cx)),
            None => then(self, None, cx),
        }
    }

    /// One tool call. `Some` answers now; `None`: the answer goes through
    /// `reply` later (searches).
    pub(crate) fn mcp_call(&mut self, name: &str, args: &Value, reply: flume::Sender<Value>, cx: &mut Context<Self>) -> Option<Value> {
        let send = move |v: Value| {
            let _ = reply.send(v);
        };
        let out = match name {
            "now_playing" => {
                let cur = self.current.clone().map(|t| self.mcp_track(&t));
                ok(json!({
                    "song": cur,
                    "playing": self.is_playing(),
                    "loading": self.is_loading(),
                    "position_s": self.position().as_secs(),
                    "duration_s": self.duration().map(|d| d.as_secs()),
                    "volume": (self.volume() * 100.0).round(),
                    "shuffle": self.shuffle(),
                    "repeat": match self.repeat() { RepeatMode::Off => "off", RepeatMode::All => "all", RepeatMode::One => "one" },
                    "queue_length": self.upcoming().len(),
                    "playing_from": self.queue_from,
                }))
            }
            "play" => {
                if self.current.is_none() {
                    self.toggle_play(cx);
                } else if !self.is_playing() {
                    self.toggle_play(cx);
                }
                ok(json!("Playing."))
            }
            "pause" => {
                if self.is_playing() {
                    self.toggle_play(cx);
                }
                ok(json!("Paused."))
            }
            "toggle_play" => {
                self.toggle_play(cx);
                ok(json!("Toggled."))
            }
            "next" => {
                self.next(false, cx);
                ok(json!("Skipped."))
            }
            "previous" => {
                self.previous_song(cx);
                ok(json!("Back to the previous song."))
            }
            "seek" => {
                let s = args["seconds"].as_f64().unwrap_or(0.0).max(0.0);
                self.seek_to(Duration::from_secs_f64(s));
                ok(json!(format!("At {s:.0} s.")))
            }
            "set_volume" => {
                let v = args["volume"].as_f64().unwrap_or(50.0).clamp(0.0, 100.0);
                self.set_volume((v / 100.0) as f32);
                ok(json!(format!("Volume {v:.0} %.")))
            }
            "set_shuffle" => {
                let on = args["on"].as_bool().unwrap_or(true);
                match args["style"].as_str() {
                    Some("random") => self.set_shuffle_style(glyph_core::config::ShuffleStyle::Random, cx),
                    Some("smart") => self.set_shuffle_style(glyph_core::config::ShuffleStyle::Smart, cx),
                    _ => {}
                }
                if self.shuffle() != on {
                    self.toggle_shuffle(cx);
                }
                ok(json!(if on { "Shuffle on." } else { "Shuffle off." }))
            }
            "set_repeat" => {
                let mode = match args["mode"].as_str() {
                    Some("all") => RepeatMode::All,
                    Some("one") => RepeatMode::One,
                    _ => RepeatMode::Off,
                };
                self.set_repeat_mode(mode, cx);
                ok(json!("Repeat set."))
            }
            "search" => {
                let q = args["query"].as_str().unwrap_or("").trim().to_string();
                if q.is_empty() {
                    return Some(fail("query is empty"));
                }
                let limit = args["limit"].as_u64().unwrap_or(10).clamp(1, 50) as usize;
                self.mcp_search(q, limit, cx, move |s, found, _| send(ok(json!({ "songs": s.mcp_tracks(&found) }))));
                return None;
            }
            "play_song" => {
                self.mcp_song(args, cx, move |s, t, cx| match t {
                    Some(t) => {
                        let v = s.mcp_track(&t);
                        s.play_list(vec![t], 0, cx);
                        send(ok(json!({ "playing": v })));
                    }
                    None => send(fail("No such song (give a ref from another answer, or a query).")),
                });
                return None;
            }
            "play_artist" => {
                let name = args["name"].as_str().unwrap_or("").trim().to_string();
                if name.is_empty() {
                    return Some(fail("name is empty"));
                }
                let shuffle = args["shuffle"].as_bool().unwrap_or(false);
                let n2 = name.clone();
                self.mcp_search(name, 40, cx, move |s, found, cx| {
                    let songs: Vec<Track> = found.into_iter().filter(|t| glyph_core::dedupe::by_artist(&t.artist, &n2)).collect();
                    if songs.is_empty() {
                        send(fail(format!("No songs by {n2} found.")));
                        return;
                    }
                    let n = songs.len();
                    if shuffle { s.play_shuffled(songs, cx) } else { s.play_list(songs, 0, cx) }
                    s.queue_from = Some(n2.clone());
                    send(ok(json!(format!("Playing {n} songs by {n2}."))));
                });
                return None;
            }
            "queue_add" => {
                let next = args["next"].as_bool().unwrap_or(false);
                self.mcp_song(args, cx, move |s, t, cx| match t {
                    Some(t) => {
                        let v = s.mcp_track(&t);
                        s.queue_add(t, next, cx);
                        send(ok(json!({ "queued": v, "next": next })));
                    }
                    None => send(fail("No such song (give a ref from another answer, or a query).")),
                });
                return None;
            }
            "get_queue" => {
                let up = self.upcoming();
                let m = self.manual_queue_len().min(up.len());
                let manual = self.mcp_tracks(&up[..m]);
                let rest = self.mcp_tracks(&up[m..up.len().min(m + 50)]);
                ok(json!({ "up_next": manual, "next_from": self.queue_from, "later": rest, "total": up.len() }))
            }
            "list_playlists" => {
                let mut v = vec![json!({ "id": "liked", "name": crate::i18n::t("library.liked").to_string(), "songs": self.library.liked.len() })];
                v.extend(self.library.playlists.iter().map(|p| json!({ "id": p.id, "name": p.name, "songs": p.tracks.len() })));
                ok(json!({ "playlists": v }))
            }
            "get_playlist" | "play_playlist" => {
                let id = args["id"].as_str().unwrap_or("");
                let Some(l) = self.mcp_list(id) else { return Some(fail(format!("No playlist {id}."))) };
                let tracks = self.list_tracks(&l);
                if name == "get_playlist" {
                    let songs = self.mcp_tracks(&tracks[..tracks.len().min(500)]);
                    ok(json!({ "id": id, "name": self.list_title(&l), "songs": songs, "total": tracks.len() }))
                } else if tracks.is_empty() {
                    fail("The playlist is empty.")
                } else {
                    let title = self.list_title(&l);
                    if args["shuffle"].as_bool().unwrap_or(false) { self.play_shuffled(tracks, cx) } else { self.play_list(tracks, 0, cx) }
                    self.playing_from = Some(l);
                    self.queue_from = Some(title.clone());
                    ok(json!(format!("Playing {title}.")))
                }
            }
            "create_playlist" => {
                let name = glyph_core::library::clean_playlist_name(args["name"].as_str().unwrap_or(""));
                if name.is_empty() {
                    return Some(fail("name is empty"));
                }
                if !self.library_ok {
                    return Some(fail("The library is not loaded."));
                }
                let songs: Vec<Track> = args["refs"].as_array().map(|a| a.iter().filter_map(|r| r.as_str()).filter_map(|r| self.mcp_ref(r)).collect()).unwrap_or_default();
                let id = {
                    let p = self.library.create_playlist(&name);
                    p.tracks = songs.clone();
                    p.id.clone()
                };
                self.save_library(cx);
                ok(json!({ "id": id, "name": name, "songs": songs.len() }))
            }
            "rename_playlist" => {
                let id = args["id"].as_str().unwrap_or("").to_string();
                let name = glyph_core::library::clean_playlist_name(args["name"].as_str().unwrap_or(""));
                if self.library.playlist(&id).is_none() || name.is_empty() {
                    return Some(fail("No such playlist or empty name."));
                }
                self.library.rename_playlist(&id, &name);
                self.save_library(cx);
                ok(json!({ "id": id, "name": name }))
            }
            "delete_playlist" => {
                let id = args["id"].as_str().unwrap_or("").to_string();
                if self.library.playlist(&id).is_none() {
                    return Some(fail("No such playlist."));
                }
                self.delete_playlist(&id, cx);
                ok(json!("Deleted."))
            }
            "add_to_playlist" => {
                let id = args["id"].as_str().unwrap_or("").to_string();
                if self.library.playlist(&id).is_none() {
                    return Some(fail("No such playlist."));
                }
                let songs: Vec<Track> = args["refs"].as_array().map(|a| a.iter().filter_map(|r| r.as_str()).filter_map(|r| self.mcp_ref(r)).collect()).unwrap_or_default();
                if songs.is_empty() {
                    return Some(fail("No known song refs given."));
                }
                let n = songs.len();
                self.add_many_to_playlist(&id, songs, cx);
                ok(json!(format!("Added {n} songs.")))
            }
            "remove_from_playlist" => {
                let id = args["id"].as_str().unwrap_or("").to_string();
                let i = args["index"].as_u64().unwrap_or(u64::MAX) as usize;
                if self.library.playlist(&id).is_none_or(|p| i >= p.tracks.len()) {
                    return Some(fail("No such playlist or index."));
                }
                self.remove_from_list(&ListRef::User(id), i, cx);
                ok(json!("Removed."))
            }
            "like" | "unlike" => {
                let want = name == "like";
                let t = match args["ref"].as_str() {
                    Some(r) => self.mcp_ref(r),
                    None => self.current.clone(),
                };
                let Some(t) = t else { return Some(fail("No such song.")) };
                if self.is_liked(&t) != want {
                    self.toggle_like(&t, cx);
                }
                let v = self.mcp_track(&t);
                ok(json!({ "song": v, "liked": want }))
            }
            "get_lyrics" => match (&self.current, &self.lyrics.lyrics) {
                (Some(t), Some(l)) => {
                    let text = if l.synced.is_empty() { l.plain.clone().unwrap_or_default() } else { l.synced.iter().map(|x| x.text.clone()).collect::<Vec<_>>().join("\n") };
                    ok(json!({ "title": t.title, "artist": t.artist, "source": l.provider, "lyrics": text }))
                }
                (Some(_), None) if self.lyrics.loading => fail("Lyrics are still loading."),
                (Some(_), None) => fail("No lyrics found for this song."),
                (None, _) => fail("Nothing is playing."),
            },
            "listening_stats" => {
                if !self.config.stats.enabled {
                    return Some(fail("Listening statistics are turned off (Settings › Playback › Content)."));
                }
                use glyph_core::stats::{self, Range};
                let range = match args["range"].as_str() {
                    Some("last_month") => Range::LastMonth,
                    Some("year") => Range::Year,
                    Some("all") => Range::All,
                    _ => Range::ThisMonth,
                };
                let s = self.library.stats.summary(range, stats::When::now());
                let artists: Vec<Value> = stats::top_artists(&s, 10).into_iter().map(|(n, a)| json!({ "artist": n, "minutes": a.ms / 60_000, "plays": a.plays })).collect();
                let songs: Vec<Value> = stats::top_tracks(&s, 10).into_iter().map(|x| json!({ "title": x.track.title, "artist": x.track.artist, "minutes": x.ms / 60_000, "plays": x.plays })).collect();
                ok(json!({ "minutes": s.ms / 60_000, "plays": s.plays, "skips": s.skips, "top_artists": artists, "top_songs": songs }))
            }
            // The docs: answered by their own MCP server (docs.glyph.my).
            "docs_search" | "docs_read" => {
                let Some(b) = self.backend.clone() else {
                    return Some(fail("Glyph is still starting; try again in a moment."));
                };
                let tool = if name == "docs_search" { "search_docs" } else { "read_doc" };
                let args = args.clone();
                cx.background_executor()
                    .spawn(async move {
                        let r = guard_async(b.docs_tool(tool, args)).await;
                        send(r.unwrap_or_else(|e| fail(format!("The Glyph docs could not be reached ({e}). They are also at https://docs.glyph.my"))));
                    })
                    .detach();
                return None;
            }
            other => fail(format!("unknown tool {other}")),
        };
        // Changes made by an assistant show right away.
        if self.view == View::List {
            cx.notify();
        }
        Some(out)
    }
}
