//! Plugin code at work (the sandbox is `glyph_core::plugin_host`): starting
//! and stopping plugins, answering their calls, sending them events, and
//! the values of the settings rows they add.
//!
//! Calls (`{"fn": ..., "args": ...}`) arrive already checked against the
//! plugin's permissions. Player, queue, library, search, lyrics and stats
//! reuse the MCP tools (`crate::mcp_tools`), so plugins and AI assistants
//! see the same Glyph.

use super::*;
use glyph_core::plugin_host::{self, HostCall, Running};
use glyph_core::plugins::{self, Installed};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Running plugin code and the state of the rows plugins add to Settings.
pub struct PluginRuntime {
    /// Running plugins with the version and install time they started at
    /// (a change restarts them).
    pub running: HashMap<String, (Running, String, u64)>,
    tx: flume::Sender<HostCall>,
    rx: flume::Receiver<HostCall>,
    /// Values of the plugins' settings items (`values.json`), per plugin.
    pub values: HashMap<String, Map<String, Value>>,
    /// Changes plugins made to their rows (`ui.update`): label, desc,
    /// status, hidden, disabled, options; per plugin and item.
    pub patches: HashMap<String, HashMap<String, Map<String, Value>>>,
    /// Text fields of `text` items, per (plugin, item).
    pub inputs: HashMap<(String, String), (Entity<TextInput>, Subscription)>,
    /// Why a plugin stopped by itself (shown on its pages).
    pub stopped: HashMap<String, String>,
    /// What plugins were last told (events go out on changes only).
    last_track: Option<String>,
    last_playing: bool,
    pub missing_credentials: HashMap<String, Vec<String>>,
    pub configured_credentials: std::collections::HashSet<(String, String)>,
    server_url: String,
    service_budget: glyph_core::plugin_policy::CallBudget,
    plugin_lyrics: HashMap<String, (String, glyph_core::lyrics::Lyrics)>,
}

impl Default for PluginRuntime {
    fn default() -> Self {
        let (tx, rx) = flume::bounded(128);
        Self {
            running: HashMap::new(),
            tx,
            rx,
            values: HashMap::new(),
            patches: HashMap::new(),
            inputs: HashMap::new(),
            stopped: HashMap::new(),
            last_track: None,
            last_playing: false,
            missing_credentials: Default::default(),
            configured_credentials: Default::default(),
            server_url: String::new(),
            service_budget: Default::default(),
            plugin_lyrics: Default::default(),
        }
    }
}

/// Plugin functions answered by an MCP tool of the same meaning.
const MCP_FUNCS: &[(&str, &str)] = &[
    ("player.state", "now_playing"),
    ("player.play", "play"),
    ("player.pause", "pause"),
    ("player.toggle", "toggle_play"),
    ("player.next", "next"),
    ("player.previous", "previous"),
    ("player.seek", "seek"),
    ("player.volume", "set_volume"),
    ("player.shuffle", "set_shuffle"),
    ("player.repeat", "set_repeat"),
    ("player.play_song", "play_song"),
    ("player.play_artist", "play_artist"),
    ("queue.get", "get_queue"),
    ("queue.add", "queue_add"),
    ("search", "search"),
    ("library.playlists", "list_playlists"),
    ("library.playlist", "get_playlist"),
    ("library.play_playlist", "play_playlist"),
    ("library.create_playlist", "create_playlist"),
    ("library.rename_playlist", "rename_playlist"),
    ("library.delete_playlist", "delete_playlist"),
    ("library.add_to_playlist", "add_to_playlist"),
    ("library.remove_from_playlist", "remove_from_playlist"),
    ("library.like", "like"),
    ("library.unlike", "unlike"),
    ("lyrics.get", "get_lyrics"),
    ("stats.get", "listening_stats"),
];

fn ok(v: Value) -> Value {
    json!({ "ok": v })
}

fn err(msg: impl Into<String>) -> Value {
    json!({ "error": msg.into() })
}

/// An MCP tool result as a plugin answer.
fn from_mcp(r: Value) -> Value {
    let text = r["content"][0]["text"].as_str().unwrap_or("").to_string();
    if r["isError"].as_bool().unwrap_or(false) {
        return err(text);
    }
    match r.get("structuredContent") {
        Some(v) => ok(v.clone()),
        None => ok(Value::String(text)),
    }
}

/// Shallow merge: `patch`'s fields replace `base`'s (both objects), else
/// `patch` wins.
fn merge(base: &Value, patch: &Value) -> Value {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            let mut out = b.clone();
            for (k, v) in p {
                out.insert(k.clone(), v.clone());
            }
            Value::Object(out)
        }
        _ => patch.clone(),
    }
}

fn plugin_lyrics(id: &str, name: &str, args: &Value) -> Result<glyph_core::lyrics::Lyrics, String> {
    let plain = args["plain"].as_str().map(str::to_string);
    let lrc = args["lrc"].as_str().unwrap_or("");
    if plain.as_ref().map_or(0, |s| s.len()) + lrc.len() > 128 * 1024 { return Err("lyrics exceed 128 KiB".into()); }
    // LRC permits several timestamps per line; bound expansion before parsing.
    if lrc.matches('[').count() > 2000 || lrc.lines().any(|l| l.len() > 4000) { return Err("too many timestamps or an oversized lyrics line".into()); }
    let synced = glyph_core::lyrics::Lyrics::parse_lrc(lrc);
    if synced.len() > 2000 || synced.iter().any(|l| l.time_ms > 24 * 3600 * 1000) { return Err("invalid lyrics timing or too many lines".into()); }
    if plain.as_ref().is_none_or(|s| s.trim().is_empty()) && synced.is_empty() { return Err("lyrics need plain text or timed LRC".into()); }
    Ok(glyph_core::lyrics::Lyrics { provider: format!("Plugin: {name}"), match_score: 1.0, synced, plain,
        instrumental: false, source_id: format!("plugin:{id}") })
}

impl Glyph {
    /// Code plugins may run: enabled, and from the checked index or put
    /// into the plugins folder by hand.
    fn may_run(&self, p: &Installed) -> bool {
        p.manifest.main.is_some() && self.plugin_enabled(&p.manifest.id) && (p.origin.checked || p.origin.is_local())
    }

    pub fn plugin_enabled(&self, id: &str) -> bool {
        !self.config.plugins.disabled.iter().any(|d| d == id)
    }

    /// Starts plugins that should run and stops the others; an updated
    /// plugin restarts. Also loads every plugin's settings values.
    pub(super) fn sync_plugin_code(&mut self, cx: &mut Context<Self>) {
        let server_url = self.config.effective_server_url().to_string();
        if self.plugins.rt.server_url != server_url {
            self.plugins.rt.running.clear();
            self.plugins.rt.server_url = server_url.clone();
        }
        self.plugins.rt.missing_credentials.clear();
        self.plugins.rt.configured_credentials.clear();
        // Remove legacy plaintext keys; require re-entry with the displayed destination binding.
        for p in &self.plugins.installed {
            let mut values = plugins::load_values(&p.manifest.id);
            let mut changed = false;
            let mut missing = Vec::new();
            for item in p.manifest.settings.iter().flat_map(|s| &s.items).filter(|i| i.secret) {
                if values.remove(&item.id).is_some() {
                    // Old plaintext has no trusted destination binding. Remove it;
                    // never silently authorize it for a newly declared service.
                    changed = true;
                }
                if glyph_core::plugin_credentials::load(&p.manifest.id, item).is_some() {
                    self.plugins.rt.configured_credentials.insert((p.manifest.id.clone(), item.id.clone()));
                } else if item.required && !missing.contains(&item.id) { missing.push(item.id.clone()); }
            }
            if changed && plugins::save_values(&p.manifest.id, &values).is_err() {
                // Never run with credentials that could not be removed from plaintext.
                missing.push("secure-storage".into());
            }
            if !missing.is_empty() { self.plugins.rt.missing_credentials.insert(p.manifest.id.clone(), missing); }
        }
        let want: Vec<Installed> = self.plugins.installed.iter().filter(|p| self.may_run(p) && !self.plugins.rt.missing_credentials.contains_key(&p.manifest.id)).cloned().collect();
        let rt = &mut self.plugins.rt;
        rt.running.retain(|id, (_, version, at)| want.iter().any(|p| &p.manifest.id == id && &p.manifest.version == version && p.origin.installed_at == *at));
        rt.values.retain(|id, _| self.plugins.installed.iter().any(|p| &p.manifest.id == id));
        rt.inputs.retain(|(id, _), _| self.plugins.installed.iter().any(|p| &p.manifest.id == id));
        for p in &self.plugins.installed {
            let mut values = plugins::load_values(&p.manifest.id);
            values.retain(|id, _| p.manifest.item(id).is_some_and(|i| !i.secret));
            rt.values.insert(p.manifest.id.clone(), values);
        }
        let running_ids: HashSet<String> = rt.running.keys().cloned().collect();
        rt.plugin_lyrics.retain(|id, _| running_ids.contains(id));
        let mut started = Vec::new();
        for p in want {
            let id = p.manifest.id.clone();
            if rt.running.contains_key(&id) {
                continue;
            }
            if rt.running.len() >= 16 {
                rt.stopped.insert(id, "at most 16 code plugins may run at once".into());
                continue;
            }
            match plugin_host::start(&p, rt.tx.clone(), &server_url) {
                Ok(r) => {
                    rt.stopped.remove(&id);
                    rt.patches.remove(&id);
                    rt.running.insert(id.clone(), (r, p.manifest.version.clone(), p.origin.installed_at));
                    started.push(id);
                }
                Err(e) => {
                    tracing::warn!(plugin = %id, "plugin did not start: {e:#}");
                    rt.stopped.insert(id, format!("{e:#}"));
                }
            }
        }
        // First event: the plugin's settings and (with `playback`) what
        // plays right now.
        for id in started {
            let values = self.plugins.rt.values.get(&id).cloned().unwrap_or_default();
            let sees = self.plugin_sees_playback(&id);
            let song = self.current.clone().filter(|_| sees).map(|t| self.mcp_track(&t));
            let playing = sees && self.is_playing();
            self.plugin_event(&id, json!({ "type": "ready", "settings": values, "song": song, "playing": playing, "lang": crate::i18n::lang().code() }));
        }
        self.refresh_plugin_lyrics();
        cx.notify();
    }

    /// Stops every plugin that runs from the plugins folder (for Reload: its
    /// code may have changed).
    pub(super) fn stop_local_plugins(&mut self) {
        let local: Vec<String> = self.plugins.installed.iter().filter(|p| p.origin.is_local()).map(|p| p.manifest.id.clone()).collect();
        self.plugins.rt.running.retain(|id, _| !local.contains(id));
    }

    /// Settings › Plugins: a plugin's switch.
    pub fn set_plugin_enabled(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        self.config.plugins.disabled.retain(|d| d != id);
        if !on {
            self.config.plugins.disabled.push(id.to_string());
        }
        self.save_config(cx);
        self.load_plugins();
        self.sync_plugin_code(cx);
        if on && self.plugins.rt.missing_credentials.contains_key(id) { self.open_plugin_page(id, cx); }
        cx.notify();
    }

    /// Sends `event` to one running plugin.
    pub fn plugin_event(&mut self, id: &str, event: Value) {
        if let Some((r, _, _)) = self.plugins.rt.running.get(id) {
            r.event(event);
        }
    }

    /// The plugin may see what plays (`playback` permission).
    fn plugin_sees_playback(&self, id: &str) -> bool {
        self.plugins.installed.iter().any(|p| p.manifest.id == id && p.manifest.grants("playback"))
    }

    /// Sends a playback event to the plugins allowed to see it.
    fn broadcast_playback(&mut self, event: Value) {
        for (id, (r, _, _)) in &self.plugins.rt.running {
            if self.plugin_sees_playback(id) {
                r.event(event.clone());
            }
        }
    }

    /// Answers calls that arrived and tells plugins what changed.
    pub(super) fn poll_plugins(&mut self, cx: &mut Context<Self>) -> bool {
        if self.plugins.rt.server_url != self.config.effective_server_url() { self.sync_plugin_code(cx); }
        let calls: Vec<HostCall> = self.plugins.rt.rx.try_iter().take(32).collect();
        let any = !calls.is_empty();
        for c in calls {
            self.plugin_call(c, cx);
        }
        if self.plugins.rt.running.is_empty() {
            return any;
        }
        let key = self.current.as_ref().map(track_key);
        if key != self.plugins.rt.last_track {
            self.plugins.rt.last_track = key;
            let current_key = self.plugins.rt.last_track.clone();
            self.plugins.rt.plugin_lyrics.retain(|_, (song, _)| Some(&*song) == current_key.as_ref());
            let song = self.current.clone().map(|t| self.mcp_track(&t));
            self.broadcast_playback(json!({ "type": "track", "song": song }));
        }
        let playing = self.is_playing();
        if playing != self.plugins.rt.last_playing {
            self.plugins.rt.last_playing = playing;
            self.broadcast_playback(json!({ "type": "playback", "playing": playing }));
        }
        any
    }

    fn plugin_call(&mut self, c: HostCall, cx: &mut Context<Self>) {
        let HostCall { plugin: id, generation, func, args, reply } = c;
        if !self.plugins.rt.running.get(&id).is_some_and(|(r, _, _)| r.generation == generation) {
            let _ = reply.send(err("the plugin invocation is no longer active"));
            return;
        }
        let Some(p) = self.plugins.installed.iter().find(|p| p.manifest.id == id).cloned() else {
            let _ = reply.send(err("the plugin is not installed"));
            return;
        };
        if func == plugin_host::STOPPED {
            self.plugins.rt.running.remove(&id);
            let why = args["error"].as_str().unwrap_or("").to_string();
            self.plugins.rt.stopped.insert(id, why);
            self.toast_info(tf("plugins.stopped_toast", &[("name", &p.manifest.name)]));
            cx.notify();
            return;
        }
        if !self.may_run(&p) || !glyph_core::plugin_policy::known_function(&func) {
            let _ = reply.send(err("plugin function is disabled or unavailable"));
            return;
        }
        // A permission check happened in the plugin's thread; this one guards
        // against a plugin whose manifest changed underneath.
        if let Some(perm) = plugin_host::permission_for(&func) {
            if !p.manifest.grants(perm) {
                let _ = reply.send(err(format!("{func} needs the \"{perm}\" permission")));
                return;
            }
        }
        if self.jam.jam.is_some() && (func.starts_with("player.") && func != "player.state"
            || func.starts_with("queue.") && func != "queue.get" || func == "library.play_playlist") {
            let _ = reply.send(err("plugin playback control is unavailable during a Jam"));
            return;
        }
        if !self.plugins.rt.service_budget.admit(&func) {
            let _ = reply.send(err("combined plugin call limit reached; try again later"));
            return;
        }
        if let Some((_, tool)) = MCP_FUNCS.iter().find(|(f, _)| *f == func) {
            let (tx, rx) = flume::bounded(1);
            if let Some(v) = self.mcp_call(tool, &args, tx, cx) {
                let _ = reply.send(from_mcp(v));
            } else {
                cx.background_executor()
                    .spawn(async move {
                        if let Ok(v) = rx.recv_async().await {
                            let _ = reply.send(from_mcp(v));
                        }
                    })
                    .detach();
            }
            return;
        }
        let answer = match func.as_str() {
            "app.info" => ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "platform": std::env::consts::OS,
                "lang": crate::i18n::lang().code(),
                "theme": self.config.visuals.theme,
                "plugin": { "id": p.manifest.id, "version": p.manifest.version },
            })),
            "app.text" => match args["key"].as_str().filter(|k| !k.is_empty() && k.len() <= 100) {
                Some(k) => ok(Value::String(t(k).to_string())),
                None => err("app.text needs a key"),
            },
            "ui.toast" => {
                let text: String = args["text"].as_str().unwrap_or("").trim().chars().take(200).collect();
                if text.is_empty() {
                    err("ui.toast needs a text")
                } else {
                    // Named, so nobody mistakes a plugin's message for Glyph's.
                    let msg = format!("{}: {text}", p.manifest.name);
                    if args["error"].as_bool().unwrap_or(false) { self.toast_info(msg) } else { self.toast(msg) }
                    ok(Value::Null)
                }
            }
            "ui.get" => match args["id"].as_str().and_then(|i| p.manifest.item(i)) {
                Some(item) if item.secret => ok(json!({"configured": self.plugins.rt.configured_credentials.contains(&(id.clone(), item.id.clone()))})),
                Some(item) => ok(plugins::item_value(item, self.plugins.rt.values.get(&id).unwrap_or(&Map::new()))),
                None => err("no settings item with this id"),
            },
            "ui.values" => {
                let values = self.plugins.rt.values.get(&id).cloned().unwrap_or_default();
                let all: Map<String, Value> = p.manifest.settings.iter().flat_map(|s| s.items.iter()).filter(|i| !i.secret && !matches!(i.kind.as_str(), "button" | "note" | "link" | "progress")).map(|i| (i.id.clone(), plugins::item_value(i, &values))).collect();
                ok(Value::Object(all))
            }
            "ui.set" => match args["id"].as_str().and_then(|i| p.manifest.item(i)).filter(|i| !i.secret).and_then(|item| plugins::coerce_value(item, &args["value"]).map(|v| (item.id.clone(), v))) {
                Some((item, v)) => {
                    self.store_plugin_value(&id, &item, v.clone(), cx);
                    if let Some((input, _)) = self.plugins.rt.inputs.get(&(id.clone(), item)) {
                        let s = v.as_str().unwrap_or("").to_string();
                        input.update(cx, |i, cx| i.set_value(s, cx));
                    }
                    ok(Value::Null)
                }
                None => err("no settings item with this id, or the value does not fit it"),
            },
            "ui.update" => match args["id"].as_str().filter(|i| p.manifest.item(i).is_some()) {
                Some(item) => {
                    if p.manifest.item(item).is_some_and(|i| i.secret) {
                        let _ = reply.send(err("credential controls are managed by Glyph"));
                        return;
                    }
                    let mut patch = Map::new();
                    if let Some(v) = args["value"].as_f64().filter(|v| v.is_finite()) {
                        patch.insert("value".into(), json!(v.clamp(0.0, 1.0)));
                    }
                    for k in ["label", "desc", "status"] {
                        if let Some(v) = args.get(k) {
                            patch.insert(k.into(), Value::String(v.as_str().unwrap_or("").chars().take(300).collect()));
                        }
                    }
                    for k in ["hidden", "disabled"] {
                        if let Some(v) = args[k].as_bool() {
                            patch.insert(k.into(), Value::Bool(v));
                        }
                    }
                    if let Some(opts) = args["options"].as_array() {
                        let opts: Vec<Value> = opts
                            .iter()
                            .filter_map(|o| Some(json!({ "value": o["value"].as_str()?.chars().take(60).collect::<String>(), "label": o["label"].as_str().unwrap_or(o["value"].as_str()?).chars().take(60).collect::<String>() })))
                            .take(50)
                            .collect();
                        patch.insert("options".into(), Value::Array(opts));
                    }
                    let entry = self.plugins.rt.patches.entry(id.clone()).or_default().entry(item.to_string()).or_default();
                    for (k, v) in patch {
                        entry.insert(k, v);
                    }
                    cx.notify();
                    ok(Value::Null)
                }
                None => err("no settings item with this id"),
            },
            "ui.open" => {
                if p.manifest.sections_on("own").next().is_some() {
                    self.open_plugin_page(&id, cx);
                    ok(Value::Null)
                } else {
                    err("the plugin has no settings page")
                }
            }
            "ui.search" => match args["query"].as_str().map(str::trim).filter(|q| !q.is_empty() && q.len() <= 200) {
                Some(q) => {
                    self.search_for(q.to_string(), cx);
                    ok(Value::Null)
                }
                None => err("ui.search needs a query"),
            },
            "ui.open_artist" => match args["name"].as_str().map(str::trim).filter(|n| !n.is_empty() && n.len() <= 200) {
                Some(n) => {
                    self.open_artist(n, cx);
                    ok(Value::Null)
                }
                None => err("ui.open_artist needs a name"),
            },
            "audio.spectrum" => {
                let bands = self.player.as_ref().and_then(|p| guard(|| p.spectrum())).unwrap_or([0.0; glyph_audio::SPECTRUM_BANDS]);
                ok(json!({ "bands": bands.to_vec(), "playing": self.is_playing() }))
            }
            "audio.equalizer" => ok(json!({ "enabled": self.config.audio.eq_enabled, "gains": self.config.audio.eq_gains })),
            "audio.set_equalizer" => {
                let gains = args["gains"].as_array().filter(|v| v.len() == 10)
                    .and_then(|v| v.iter().map(|x| x.as_f64().filter(|x| x.is_finite() && (-12.0..=12.0).contains(x)).map(|x| x as f32)).collect::<Option<Vec<_>>>())
                    .and_then(|v| <[f32; 10]>::try_from(v).ok());
                match (gains, args["enabled"].as_bool()) {
                    (Some(gains), Some(enabled)) => { self.set_eq(enabled, "custom", gains, true, cx); ok(Value::Null) }
                    _ => err("expected enabled and ten finite gains between -12 and 12 dB"),
                }
            }
            "queue.remove" | "queue.move" => {
                // Plugins edit only this device's queue, never a Jam session.
                if self.jam.jam.is_some() { err("plugins cannot edit a Jam queue") }
                else {
                    let n = guard(|| self.queue.upcoming().len()).unwrap_or(0);
                    match (args["index"].as_u64(), args["from"].as_u64(), args["to"].as_u64()) {
                        (Some(i), _, _) if func == "queue.remove" && i < n as u64 => { self.queue_remove(i as usize, cx); ok(Value::Null) }
                        (_, Some(a), Some(b)) if func == "queue.move" && a < n as u64 && b < n as u64 => { self.queue_move(a as usize, b as usize, cx); ok(Value::Null) }
                        _ => err("queue index is out of bounds"),
                    }
                }
            }
            "lyrics.publish" => {
                let current = self.current.as_ref().map(track_key);
                if args["ref"].as_str().is_none_or(|r| current.as_deref() != Some(r)) {
                    err("lyrics belong to a different song")
                } else {
                    match plugin_lyrics(&id, &p.manifest.name, &args) {
                        Ok(lyrics) => {
                            let key = track_key(self.current.as_ref().unwrap());
                            self.plugins.rt.plugin_lyrics.insert(id.clone(), (key, lyrics.clone()));
                            self.lyrics.alts.retain(|l| l.source_id != lyrics.source_id);
                            self.lyrics.alts.push(lyrics.clone());
                            if self.lyrics.lyrics.is_none() || self.lyrics.lyrics.as_ref().is_some_and(|l| l.source_id == lyrics.source_id) {
                                self.lyrics.lyrics = Some(lyrics);
                                self.lyrics.error = None;
                                self.lyrics.last_line = None;
                            }
                            cx.notify();
                            ok(Value::Null)
                        }
                        Err(e) => err(e),
                    }
                }
            }
            "settings.get" => {
                let all = glyph_core::settings_share::sections(&self.config);
                match args["section"].as_str() {
                    Some(s) => all.get(s).cloned().map(ok).unwrap_or_else(|| err(format!("no settings section {s}"))),
                    None => ok(Value::Object(all)),
                }
            }
            "settings.set" => {
                let (Some(section), Some(value)) = (args["section"].as_str(), args.get("value")) else {
                    let _ = reply.send(err("settings.set needs a section and a value"));
                    return;
                };
                if !matches!(section, "general" | "visuals" | "audio" | "playback" | "player" | "lists" | "eq_presets") {
                    let _ = reply.send(err("plugins cannot change server, provider, integration, privacy or update policy"));
                    return;
                }
                let all = glyph_core::settings_share::sections(&self.config);
                let Some(cur) = all.get(section) else {
                    let _ = reply.send(err(format!("no settings section {section}")));
                    return;
                };
                let mut merged = merge(cur, value);
                // Where updates come from stays Glyph's: another repository
                // would hand a plugin the app itself.
                if let Some(m) = merged.as_object_mut().filter(|_| section == "updates") {
                    m.insert("repo".into(), cur["repo"].clone());
                }
                let mut cfg = self.config.clone();
                let mut one = Map::new();
                one.insert(section.to_string(), merged);
                let r = glyph_core::settings_share::apply_sections(&mut cfg, one);
                if r.changed {
                    self.adopt_synced_config(cfg, r.providers_changed, cx);
                    self.schedule_sync(cx);
                }
                ok(json!({ "changed": r.changed }))
            }
            other => err(format!("unknown function {other}")),
        };
        let _ = reply.send(answer);
    }

    /// Merge live, song-scoped plugin candidates without pinning them to disk.
    pub(super) fn refresh_plugin_lyrics(&mut self) {
        let key = self.current.as_ref().map(track_key);
        let candidates: Vec<_> = self.plugins.rt.plugin_lyrics.iter()
            .filter(|(id, (song, _))| self.plugins.rt.running.contains_key(*id) && key.as_ref() == Some(song))
            .map(|(_, (_, lyrics))| lyrics.clone()).collect();
        self.lyrics.alts.retain(|l| !l.source_id.starts_with("plugin:"));
        self.lyrics.alts.extend(candidates.clone());
        if self.lyrics.lyrics.as_ref().is_some_and(|l| l.source_id.starts_with("plugin:")
            && !candidates.iter().any(|c| c.source_id == l.source_id)) {
            self.lyrics.lyrics = self.lyrics.alts.iter().find(|l| !l.source_id.starts_with("plugin:")).cloned();
            self.lyrics.last_line = None;
        }
        if self.lyrics.lyrics.is_none() { self.lyrics.lyrics = candidates.into_iter().next(); }
    }

    /// Keeps a settings value of a plugin and saves it.
    fn store_plugin_value(&mut self, id: &str, item: &str, value: Value, cx: &mut Context<Self>) {
        let values = self.plugins.rt.values.entry(id.to_string()).or_default();
        values.insert(item.to_string(), value);
        let (values, id) = (values.clone(), id.to_string());
        cx.background_executor()
            .spawn(async move {
                if let Err(e) = plugins::save_values(&id, &values) {
                    tracing::warn!(plugin = %id, "saving plugin settings: {e:#}");
                }
            })
            .detach();
        cx.notify();
    }

    /// The user changed a plugin's settings row.
    pub fn set_plugin_value(&mut self, id: &str, item: &str, value: Value, cx: &mut Context<Self>) {
        let Some(p) = self.plugins.installed.iter().find(|p| p.manifest.id == id) else { return };
        let Some(setting) = p.manifest.item(item) else { return };
        if setting.secret {
            match glyph_core::plugin_credentials::store(id, setting, value.as_str().unwrap_or("")) {
                Ok(()) => {
                    if let Some((input, _)) = self.plugins.rt.inputs.get(&(id.to_string(), item.to_string())) {
                        input.update(cx, |i, cx| i.set_value(String::new(), cx));
                    }
                    self.sync_plugin_code(cx);
                    self.plugin_event(id, json!({ "type": "credential", "id": item, "configured": !value.as_str().unwrap_or("").is_empty() }));
                }
                Err(_) => self.toast_info(t("plugins.setup.save_failed")),
            }
            return;
        }
        let Some(v) = plugins::coerce_value(setting, &value) else { return };
        self.store_plugin_value(id, item, v.clone(), cx);
        self.plugin_event(id, json!({ "type": "setting", "id": item, "value": v }));
    }

    /// Current value of a plugin's settings item.
    pub fn plugin_value(&self, id: &str, item: &plugins::SettingItem) -> Value {
        if item.secret { return Value::String(String::new()); }
        plugins::item_value(item, self.plugins.rt.values.get(id).unwrap_or(&Map::new()))
    }

    /// A plugin's button was pressed.
    pub fn plugin_action(&mut self, id: &str, item: &str) {
        self.plugin_event(id, json!({ "type": "action", "id": item }));
    }

    /// A plugin's song menu entry was picked.
    pub fn plugin_menu(&mut self, id: &str, entry: &str, track: &Track) {
        let song = self.mcp_track(track);
        self.plugin_event(id, json!({ "type": "menu", "id": entry, "song": song }));
    }

    /// Song menu entries of running plugins: (plugin, entry, label).
    pub fn plugin_menu_entries(&self) -> Vec<(String, String, SharedString)> {
        let lang = crate::i18n::lang().code();
        self.plugins
            .installed
            .iter()
            .filter(|p| self.plugins.rt.running.contains_key(&p.manifest.id))
            .flat_map(|p| p.manifest.menus.iter().filter(|m| m.menu == "song").map(move |m| (p.manifest.id.clone(), m.id.clone(), SharedString::from(m.label.get(lang).to_string()))))
            .collect()
    }

    /// The text field of a plugin's `text` item (made on first use).
    pub fn plugin_input(&mut self, id: &str, item: &plugins::SettingItem, cx: &mut Context<Self>) -> Entity<TextInput> {
        let key = (id.to_string(), item.id.clone());
        if let Some((input, _)) = self.plugins.rt.inputs.get(&key) {
            return input.clone();
        }
        let lang = crate::i18n::lang().code();
        let placeholder = item.placeholder.get(lang).to_string();
        let secret = item.secret;
        let value = self.plugin_value(id, item).as_str().unwrap_or("").to_string();
        let input = cx.new(|cx| {
            let mut i = TextInput::new(placeholder, cx).max_chars(if secret { 2000 } else { 500 });
            if secret {
                i = i.masked();
            }
            i.set_value(value, cx);
            i
        });
        let (pid, iid) = key.clone();
        let sub = cx.subscribe(&input, move |this, _, ev: &InputEvent, cx| match ev {
            // Saved and sent to the plugin as typed.
            InputEvent::Changed(v) if !secret => this.set_plugin_value(&pid, &iid, Value::String(v.clone()), cx),
            InputEvent::Submit(v) => this.set_plugin_value(&pid, &iid, Value::String(v.clone()), cx),
            _ => {}
        });
        self.plugins.rt.inputs.insert(key, (input.clone(), sub));
        input
    }

    /// Settings › Plugins › <plugin>: the plugin's own page.
    pub fn open_plugin_page(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(i) = self.plugins.pages.iter().position(|p| p == id) else { return };
        self.open_settings(Some(SettingsPage::Plugins), cx);
        self.set_settings_tab(SettingsPage::Plugins, (i + 2) as u8, cx);
    }

    /// Recent log lines of a running plugin (newest last).
    pub fn plugin_logs(&self, id: &str) -> Vec<String> {
        self.plugins.rt.running.get(id).map(|(r, _, _)| r.log_lines()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_results_become_plugin_answers() {
        let r = glyph_protocol::mcp::text_result(&json!({ "playing": true }), false);
        assert_eq!(from_mcp(r), json!({ "ok": { "playing": true } }));
        let r = glyph_protocol::mcp::text_result(&json!("Skipped."), false);
        assert_eq!(from_mcp(r), json!({ "ok": "Skipped." }));
        let r = glyph_protocol::mcp::text_result(&json!("No such song."), true);
        assert_eq!(from_mcp(r), json!({ "error": "No such song." }));
    }

    #[test]
    fn every_mapped_function_has_a_tool_and_a_permission() {
        for (f, tool) in MCP_FUNCS {
            assert!(glyph_protocol::mcp::tools().iter().any(|t| t["name"] == *tool), "{tool}");
            assert!(plugin_host::permission_for(f).is_some(), "{f}");
        }
    }

    #[test]
    fn merge_is_shallow() {
        assert_eq!(merge(&json!({ "a": 1, "b": 2 }), &json!({ "b": 3 })), json!({ "a": 1, "b": 3 }));
        assert_eq!(merge(&json!(1), &json!({ "b": 3 })), json!({ "b": 3 }));
    }

    #[test]
    fn provider_lyrics_are_bounded_and_attributed_by_the_host() {
        let lyrics = plugin_lyrics("demo", "Demo", &json!({"plain":"Words", "provider":"Glyph"})).unwrap();
        assert_eq!(lyrics.provider, "Plugin: Demo");
        assert_eq!(lyrics.source_id, "plugin:demo");
        assert!(plugin_lyrics("demo", "Demo", &json!({"plain":" "})).is_err());
        assert!(plugin_lyrics("demo", "Demo", &json!({"plain":"x".repeat(128 * 1024 + 1)})).is_err());
    }
}
