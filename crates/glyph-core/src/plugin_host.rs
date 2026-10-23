//! Runs plugin code: one WebAssembly module per plugin (see
//! [`crate::plugins`]), each on its own thread, in an interpreter
//! ([`wasmi`]) with no access to anything but the functions below.
//!
//! # ABI (module `glyph`)
//!
//! The plugin imports:
//! - `call(ptr: i32, len: i32) -> i32`: a request as JSON
//!   (`{"fn": "player.next", "args": {...}}`) at `ptr..ptr+len`; returns the
//!   length of the answer (`{"ok": ...}` or `{"error": "..."}`).
//! - `read(ptr: i32) -> i32`: copies the pending message (the last answer,
//!   or the event being delivered) to `ptr`; returns its length.
//! - `log(level: i32, ptr: i32, len: i32)`: a line for the log (0 debug,
//!   1 info, 2 warn, 3 error).
//!
//! The plugin exports (both optional) `glyph_start()`, called once, and
//! `glyph_event(len: i32)`, called with each event (JSON, fetched with
//! `read`). Its memory is exported as `memory`.
//!
//! # Limits
//!
//! Each `glyph_start` / `glyph_event` call gets a fuel budget (a plugin
//! stuck in a loop is stopped, not the app), memory is capped at 64 MiB, a
//! plugin that traps three times in a row is stopped. Storage, timers and
//! network requests (to the manifest's hosts only, https, no redirects) are
//! handled on the plugin's thread; every other call goes to the app as a
//! [`HostCall`], checked against the plugin's permissions first.

use crate::plugins::{self, Installed, Manifest};
use anyhow::{anyhow, bail, Context as _, Result};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use wasmi::{Caller, Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

/// Fuel (roughly instructions) for `glyph_start` and for each event.
#[cfg(not(test))]
const START_FUEL: u64 = 4_000_000_000;
#[cfg(not(test))]
const EVENT_FUEL: u64 = 1_000_000_000;
#[cfg(test)]
const START_FUEL: u64 = 20_000_000;
#[cfg(test)]
const EVENT_FUEL: u64 = 5_000_000;
const MAX_MEMORY: usize = 64 * 1024 * 1024;
/// Requests, answers and events bigger than this are refused.
const MAX_MESSAGE: usize = 4 * 1024 * 1024;
const MAX_STORAGE: usize = 1024 * 1024;
const MAX_TIMERS: usize = 32;
const MAX_FETCH: usize = 4 * 1024 * 1024;
/// How long a plugin waits for the app to answer a call (searches take up
/// to 10 s).
const CALL_TIMEOUT: Duration = Duration::from_secs(20);
/// Traps in a row after which the plugin is stopped.
const MAX_STRIKES: u32 = 3;
const LOG_LINES: usize = 100;

/// A call the app answers (player, library, settings, UI ...). The answer
/// is `{"ok": ...}` or `{"error": "..."}`.
pub struct HostCall {
    pub plugin: String,
    pub generation: u64,
    pub func: String,
    pub args: Value,
    pub reply: flume::Sender<Value>,
}

/// Function the app gets when a plugin stopped by itself (`args.error`
/// says why); nobody waits for its answer.
pub const STOPPED: &str = "host.stopped";

enum Msg {
    Event(Value),
    Stop,
}

/// A running plugin; dropping it stops it.
pub struct Running {
    pub id: String,
    pub generation: u64,
    active: Arc<AtomicBool>,
    tx: flume::Sender<Msg>,
    /// Recent log lines (newest last), for the plugin's detail view.
    pub logs: Arc<Mutex<VecDeque<String>>>,
}

impl Running {
    /// Hands the plugin an event (`{"type": "...", ...}`); `false` once it stopped.
    pub fn event(&self, event: Value) -> bool {
        self.active.load(Ordering::Acquire) && self.tx.try_send(Msg::Event(event)).is_ok()
    }

    pub fn log_lines(&self) -> Vec<String> {
        self.logs.lock().map(|l| l.iter().cloned().collect()).unwrap_or_default()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        let _ = self.tx.try_send(Msg::Stop);
    }
}

/// Starts the code of `plugin`; calls for the app arrive on `calls`.
pub fn start(plugin: &Installed, calls: flume::Sender<HostCall>, server_url: &str) -> Result<Running> {
    static GENERATION: AtomicU64 = AtomicU64::new(1);
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    let active = Arc::new(AtomicBool::new(true));
    let thread_active = active.clone();
    let blocked_hosts = [crate::config::DEFAULT_SERVER_URL, "https://glyph-server.glyphmusic.workers.dev", server_url]
        .iter().filter_map(|s| reqwest::Url::parse(s).ok()?.host_str().map(str::to_string)).collect::<Vec<_>>();
    let path = plugin.code_path().ok_or_else(|| anyhow!("{} has no code", plugin.manifest.id))?;
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    plugins::check_code(&bytes)?;
    let (tx, rx) = flume::bounded(64);
    let logs = Arc::new(Mutex::new(VecDeque::new()));
    let manifest = plugin.manifest.clone();
    let dir = plugin.dir();
    let logs2 = logs.clone();
    std::thread::Builder::new().name(format!("glyph-plugin-{}", manifest.id)).spawn(move || {
        let id = manifest.id.clone();
        // Compiled on the plugin's thread: a big module never stalls the UI.
        let engine = engine();
        let started = Module::new(&engine, &bytes[..])
            .map_err(|e| anyhow!("not a valid WebAssembly module: {e}"))
            .and_then(|module| run(engine, module, manifest, dir, rx, calls.clone(), logs2.clone(), generation, thread_active.clone(), blocked_hosts));
        thread_active.store(false, Ordering::Release);
        if let Err(e) = started {
            tracing::warn!(plugin = %id, "plugin stopped: {e:#}");
            push_log(&logs2, format!("[stopped] {e:#}"));
            let (reply, _) = flume::bounded(1);
            let _ = calls.try_send(HostCall { plugin: id, generation, func: STOPPED.into(), args: json!({ "error": format!("{e:#}") }), reply });
        }
    })?;
    Ok(Running { id: plugin.manifest.id.clone(), generation, active, tx, logs })
}

fn engine() -> Engine {
    let mut config = Config::default();
    config.consume_fuel(true);
    Engine::new(&config)
}

fn push_log(logs: &Mutex<VecDeque<String>>, line: String) {
    if let Ok(mut l) = logs.lock() {
        if l.len() >= LOG_LINES {
            l.pop_front();
        }
        l.push_back(line);
    }
}

/// What the host functions see.
struct State {
    id: String,
    generation: u64,
    active: Arc<AtomicBool>,
    blocked_hosts: Vec<String>,
    budget: crate::plugin_policy::CallBudget,
    manifest: Manifest,
    calls: flume::Sender<HostCall>,
    /// The pending message for `read`.
    pending: Vec<u8>,
    storage: Map<String, Value>,
    storage_path: PathBuf,
    timers: Vec<Timer>,
    logs: Arc<Mutex<VecDeque<String>>>,
    limits: StoreLimits,
}

struct Timer {
    id: String,
    due: Instant,
    every: Option<Duration>,
}

fn run(
    engine: Engine,
    module: Module,
    manifest: Manifest,
    dir: PathBuf,
    rx: flume::Receiver<Msg>,
    calls: flume::Sender<HostCall>,
    logs: Arc<Mutex<VecDeque<String>>>,
    generation: u64,
    active: Arc<AtomicBool>,
    blocked_hosts: Vec<String>,
) -> Result<()> {
    let storage_path = dir.join("storage.json");
    let state = State {
        id: manifest.id.clone(),
        generation, active, blocked_hosts, budget: Default::default(),
        storage: crate::paths::read_json(&storage_path).unwrap_or_default(),
        storage_path,
        manifest,
        calls,
        pending: Vec::new(),
        timers: Vec::new(),
        logs,
        limits: StoreLimitsBuilder::new().memory_size(MAX_MEMORY).instances(1).memories(1).tables(4).build(),
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|s| &mut s.limits);
    let mut linker = Linker::<State>::new(&engine);
    linker
        .func_wrap("glyph", "call", |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let req = read_memory(&caller, ptr, len)?;
            let answer = handle(caller.data_mut(), &req);
            let mut bytes = serde_json::to_vec(&answer).unwrap_or_default();
            if bytes.len() > MAX_MESSAGE { bytes = br#"{"error":"plugin response exceeds 4 MiB"}"#.to_vec(); }
            let n = bytes.len() as i32;
            caller.data_mut().pending = bytes;
            Ok(n)
        })
        .map_err(|e| anyhow!("{e}"))?;
    linker
        .func_wrap("glyph", "read", |mut caller: Caller<'_, State>, ptr: i32| -> Result<i32, wasmi::Error> {
            let bytes = caller.data().pending.clone();
            let memory = caller.get_export("memory").and_then(|e| e.into_memory()).ok_or_else(|| wasmi::Error::new("the plugin exports no memory"))?;
            let at = usize::try_from(ptr).map_err(|_| wasmi::Error::new("negative pointer"))?;
            memory.write(&mut caller, at, &bytes).map_err(|_| wasmi::Error::new("read: pointer out of bounds"))?;
            Ok(bytes.len() as i32)
        })
        .map_err(|e| anyhow!("{e}"))?;
    linker
        .func_wrap("glyph", "log", |caller: Caller<'_, State>, level: i32, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let bytes = read_memory(&caller, ptr, len.min(4000))?;
            let line: String = String::from_utf8_lossy(&bytes).chars().take(2000).collect();
            let s = caller.data();
            match level {
                0 => tracing::debug!(plugin = %s.id, "{line}"),
                1 => tracing::info!(plugin = %s.id, "{line}"),
                2 => tracing::warn!(plugin = %s.id, "{line}"),
                _ => tracing::error!(plugin = %s.id, "{line}"),
            }
            let tag = ["debug", "info", "warn", "error"][level.clamp(0, 3) as usize];
            push_log(&s.logs, format!("[{tag}] {line}"));
            Ok(0)
        })
        .map_err(|e| anyhow!("{e}"))?;
    store.set_fuel(START_FUEL).map_err(|e| anyhow!("{e}"))?;
    let instance = linker.instantiate_and_start(&mut store, &module).map_err(|e| anyhow!("could not start: {e}"))?;
    let start = instance.get_typed_func::<(), ()>(&store, "glyph_start").ok();
    let on_event = instance.get_typed_func::<i32, ()>(&store, "glyph_event").ok();
    if let Some(f) = start {
        store.set_fuel(START_FUEL).map_err(|e| anyhow!("{e}"))?;
        f.call(&mut store, ()).map_err(|e| anyhow!("glyph_start failed: {}", trap_text(&e)))?;
    }
    tracing::info!(plugin = %store.data().id, "plugin started");
    let mut strikes = 0u32;
    loop {
        if !store.data().active.load(Ordering::Acquire) { break; }
        let next = store.data().timers.iter().map(|t| t.due).min();
        let msg = match next {
            Some(due) => rx.recv_deadline(due),
            None => rx.recv().map_err(|_| flume::RecvTimeoutError::Disconnected),
        };
        let events: Vec<Value> = match msg {
            Ok(Msg::Stop) | Err(flume::RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Event(v)) => vec![v],
            Err(flume::RecvTimeoutError::Timeout) => due_timers(store.data_mut()),
        };
        let Some(f) = on_event else { continue };
        for ev in events {
            if !store.data().active.load(Ordering::Acquire) { break; }
            let bytes = serde_json::to_vec(&ev).unwrap_or_default();
            if bytes.len() > MAX_MESSAGE {
                continue;
            }
            let n = bytes.len() as i32;
            store.data_mut().pending = bytes;
            store.set_fuel(EVENT_FUEL).map_err(|e| anyhow!("{e}"))?;
            match f.call(&mut store, n) {
                Ok(()) => strikes = 0,
                Err(e) => {
                    strikes += 1;
                    let why = trap_text(&e);
                    tracing::warn!(plugin = %store.data().id, "event failed: {why}");
                    push_log(&store.data().logs, format!("[error] event {}: {why}", ev["type"].as_str().unwrap_or("?")));
                    if strikes >= MAX_STRIKES {
                        bail!("failed {MAX_STRIKES} times in a row (last: {why})");
                    }
                }
            }
        }
    }
    tracing::info!(plugin = %store.data().id, "plugin stopped");
    Ok(())
}

fn trap_text(e: &wasmi::Error) -> String {
    if e.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) {
        "took too long (stopped after its time budget)".into()
    } else {
        e.to_string()
    }
}

fn read_memory(caller: &Caller<'_, State>, ptr: i32, len: i32) -> Result<Vec<u8>, wasmi::Error> {
    let (at, n) = match (usize::try_from(ptr), usize::try_from(len)) {
        (Ok(a), Ok(n)) if n <= MAX_MESSAGE => (a, n),
        _ => return Err(wasmi::Error::new("bad pointer or length")),
    };
    let memory = caller.get_export("memory").and_then(|e| e.into_memory()).ok_or_else(|| wasmi::Error::new("the plugin exports no memory"))?;
    let mut buf = vec![0u8; n];
    memory.read(caller, at, &mut buf).map_err(|_| wasmi::Error::new("pointer out of bounds"))?;
    Ok(buf)
}

fn due_timers(s: &mut State) -> Vec<Value> {
    let now = Instant::now();
    let mut out = Vec::new();
    s.timers.retain_mut(|t| {
        if t.due > now {
            return true;
        }
        out.push(json!({ "type": "timer", "id": t.id }));
        match t.every {
            Some(every) => {
                t.due = now + every;
                true
            }
            None => false,
        }
    });
    out
}

/// Permission a function needs (`None`: every plugin may use it).
pub fn permission_for(func: &str) -> Option<&'static str> {
    if matches!(func, "ui.search" | "ui.open_artist") { return Some("search"); }
    if func == "lyrics.publish" { return Some("lyrics_provider"); }
    let ns = func.split('.').next().unwrap_or("");
    match ns {
        "player" | "queue" => Some("playback"),
        "library" | "lyrics" | "stats" => Some("library"),
        "search" => Some("search"),
        "settings" => Some("settings"),
        "net" => Some("network"),
        "audio" => Some("audio"),
        _ => None,
    }
}

fn ok(v: Value) -> Value {
    json!({ "ok": v })
}

fn err(msg: impl Into<String>) -> Value {
    json!({ "error": msg.into() })
}

/// One request from the plugin.
fn handle(s: &mut State, req: &[u8]) -> Value {
    let Ok(req) = serde_json::from_slice::<Value>(req) else { return err("the request is not JSON") };
    let Some(func) = req["fn"].as_str().map(str::to_string) else { return err("the request has no \"fn\"") };
    let args = match req.get("args") {
        Some(Value::Object(_)) => req["args"].clone(),
        None | Some(Value::Null) => json!({}),
        Some(_) => return err("\"args\" must be an object"),
    };
    if !s.active.load(Ordering::Acquire) { return err("the plugin was stopped"); }
    if !crate::plugin_policy::known_function(&func) { return err("unknown or unavailable plugin function"); }
    if !s.budget.admit(&func) { return err("plugin call limit reached; try again later"); }
    if let Some(p) = permission_for(&func) {
        if !s.manifest.grants(p) {
            return err(format!("{func} needs the \"{p}\" permission (add it to \"permissions\" in glyph-plugin.json)"));
        }
    }
    match func.as_str() {
        "storage.get" => ok(args["key"].as_str().and_then(|k| s.storage.get(k).cloned()).unwrap_or(Value::Null)),
        "storage.keys" => ok(json!(s.storage.keys().collect::<Vec<_>>())),
        "storage.set" => {
            let Some(key) = args["key"].as_str().filter(|k| !k.is_empty() && k.len() <= 200) else { return err("storage.set needs a key (1-200 characters)") };
            let old = s.storage.insert(key.to_string(), args["value"].clone());
            if serde_json::to_vec(&s.storage).map_or(true, |b| b.len() > MAX_STORAGE) {
                match old {
                    Some(v) => s.storage.insert(key.to_string(), v),
                    None => s.storage.remove(key),
                };
                return err("storage is full (1 MiB per plugin)");
            }
            save_storage(s)
        }
        "storage.delete" => {
            if let Some(k) = args["key"].as_str() {
                s.storage.remove(k);
            }
            save_storage(s)
        }
        "storage.clear" => {
            s.storage.clear();
            save_storage(s)
        }
        "timer.set" => {
            let Some(id) = args["id"].as_str().filter(|k| !k.is_empty() && k.len() <= 60).map(str::to_string) else { return err("timer.set needs an id") };
            let ms = args["ms"].as_u64().unwrap_or(1000).clamp(50, 24 * 3600 * 1000);
            let every = args["repeat"].as_bool().unwrap_or(false).then(|| Duration::from_millis(ms.max(250)));
            s.timers.retain(|t| t.id != id);
            if s.timers.len() >= MAX_TIMERS {
                return err("too many timers (32)");
            }
            s.timers.push(Timer { id, due: Instant::now() + Duration::from_millis(ms), every });
            ok(Value::Null)
        }
        "timer.clear" => {
            let id = args["id"].as_str().unwrap_or("");
            s.timers.retain(|t| t.id != id);
            ok(Value::Null)
        }
        // WebAssembly without WASI has no clock of its own.
        "app.now" => ok(json!(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64))),
        "net.fetch" => match fetch(&s.manifest, &args, &s.blocked_hosts, Some(&s.active)) {
            Ok(v) => ok(v),
            Err(e) => err(format!("{e:#}")),
        },
        _ => forward(s, func, args),
    }
}

fn save_storage(s: &State) -> Value {
    match crate::paths::write_json(&s.storage_path, &s.storage) {
        Ok(()) => ok(Value::Null),
        Err(e) => err(format!("could not save: {e:#}")),
    }
}

/// Hands a call to the app and waits for its answer.
fn forward(s: &State, func: String, args: Value) -> Value {
    let (reply, answer) = flume::bounded(1);
    if s.calls.try_send(HostCall { plugin: s.id.clone(), generation: s.generation, func, args, reply }).is_err() {
        return err("Glyph is closing");
    }
    match answer.recv_timeout(CALL_TIMEOUT) {
        Ok(v) => v,
        Err(flume::RecvTimeoutError::Timeout) => err("Glyph did not answer in time"),
        Err(flume::RecvTimeoutError::Disconnected) => err("Glyph stopped the plugin"),
    }
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("glyph-plugin-net")
            .enable_all()
            .build()
            .expect("plugin network runtime")
    })
}

fn checked_url(manifest: &Manifest, args: &Value, blocked_hosts: &[String]) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(args["url"].as_str().unwrap_or("")).map_err(|_| anyhow!("net.fetch needs a valid URL"))?;
    let host = url.host_str().unwrap_or("");
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some()
        || url.port().is_some_and(|p| p != 443) || url.fragment().is_some() {
        bail!("net.fetch requires HTTPS on port 443 without user information or fragments");
    }
    if !plugins::host_allowed(&manifest.network, host) { bail!("host is not in the plugin's network list"); }
    if host.ends_with('.') || blocked_hosts.iter().any(|h| h.eq_ignore_ascii_case(host)) {
        bail!("plugins cannot contact Glyph servers directly");
    }
    Ok(url)
}

/// Credentials are injected by Glyph; never returned to Wasm or inherited from app config.
fn inject_credentials(manifest: &Manifest, args: &Value, url: &mut reqwest::Url) -> Result<(Vec<(String, String)>, Vec<String>)> {
    let mut headers = Vec::new();
    let mut secrets = Vec::new();
    let Some(creds) = args.get("credentials") else { return Ok((headers, secrets)) };
    let creds = creds.as_object().ok_or_else(|| anyhow!("credentials must be an object"))?;
    if creds.keys().any(|k| !matches!(k.as_str(), "query" | "headers")) { bail!("credentials supports query and headers only"); }
    for (kind, fields) in creds {
        let fields = fields.as_object().ok_or_else(|| anyhow!("credential fields must be an object"))?;
        if fields.len() > 10 { bail!("too many credentials"); }
        for (name, id) in fields {
            if name.is_empty() || name.len() > 100 { bail!("invalid credential field name"); }
            let item = id.as_str().and_then(|id| manifest.item(id)).filter(|i| i.secret && i.kind == "text")
                .ok_or_else(|| anyhow!("credential must name this plugin's secret text item"))?;
            let scope = item.credential.as_ref().ok_or_else(|| anyhow!("credential destination is not configured"))?;
            if url.host_str() != Some(scope.host.as_str()) || url.path() != scope.path || kind != &scope.location || name != &scope.name {
                bail!("credential is bound to a different destination or request field");
            }
            let value = crate::plugin_credentials::load(&manifest.id, item).ok_or_else(|| anyhow!("required plugin credential is not configured"))?;
            secrets.push(value.clone());
            if kind == "query" { url.query_pairs_mut().append_pair(name, &value); }
            else { headers.push((name.clone(), value)); }
        }
    }
    Ok((headers, secrets))
}

fn redact_credentials(mut value: String, secrets: &[String]) -> String {
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        value = value.replace(secret, "[redacted]");
        let escaped = serde_json::to_string(secret).unwrap_or_default();
        if escaped.len() >= 2 { value = value.replace(&escaped[1..escaped.len()-1], "[redacted]"); }
        let encoded: String = secret.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            (b as char).to_string()
        } else { format!("%{b:02X}") }).collect();
        value = value.replace(&encoded, "[redacted]");
    }
    value
}

fn allowed_header(name: &str) -> bool {
    !matches!(name.to_ascii_lowercase().as_str(), "host" | "cookie" | "proxy-authorization" | "proxy-connection" |
        "connection" | "transfer-encoding" | "content-length" | "forwarded" | "origin" | "referer")
        && !name.to_ascii_lowercase().starts_with("x-glyph-")
        && !name.to_ascii_lowercase().starts_with("x-forwarded-")
}

fn fetch(manifest: &Manifest, args: &Value, blocked_hosts: &[String], active: Option<&AtomicBool>) -> Result<Value> {
    let mut url = checked_url(manifest, args, blocked_hosts)?;
    let host = url.host_str().unwrap_or("").to_string();
    let method = match args["method"].as_str().unwrap_or("GET").to_ascii_uppercase().as_str() {
        "GET" => reqwest::Method::GET, "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT, "PATCH" => reqwest::Method::PATCH,
        "DELETE" => reqwest::Method::DELETE, "HEAD" => reqwest::Method::HEAD,
        _ => bail!("unsupported HTTP method"),
    };
    let mut headers = Vec::new();
    if let Some(h) = args.get("headers") {
        let h = h.as_object().ok_or_else(|| anyhow!("headers must be an object"))?;
        if h.len() > 30 { bail!("too many headers"); }
        for (name, value) in h {
            headers.push((name.clone(), value.as_str().ok_or_else(|| anyhow!("header value must be text"))?.to_string()));
        }
    }
    let (credential_headers, secrets) = inject_credentials(manifest, args, &mut url)?;
    headers.extend(credential_headers);
    let mut checked_headers = reqwest::header::HeaderMap::new();
    for (name, value) in headers {
        if !allowed_header(&name) { bail!("reserved HTTP header"); }
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| anyhow!("invalid header name"))?;
        let mut value = reqwest::header::HeaderValue::from_str(&value).map_err(|_| anyhow!("invalid header value"))?;
        value.set_sensitive(true);
        checked_headers.insert(name, value);
    }
    runtime().block_on(async move {
        tokio::select! {
            result = async {
        use futures::StreamExt;
        // Reject the whole lookup if any answer is private; pin this exact set
        // into reqwest so a second DNS lookup cannot rebind the destination.
        let addresses: Vec<_> = tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host.as_str(), 443)))
            .await.map_err(|_| anyhow!("DNS lookup timed out"))?.map_err(|_| anyhow!("DNS lookup failed"))?.take(32).collect();
        if addresses.is_empty() || addresses.iter().any(|a| !crate::plugin_policy::public_ip(a.ip())) { bail!("private or reserved network destination"); }
        // DNS aliases must not reach a configured Glyph server either.
        for blocked in blocked_hosts {
            if let Ok(Ok(ips)) = tokio::time::timeout(Duration::from_secs(2), tokio::net::lookup_host((blocked.as_str(), 443))).await {
                if ips.into_iter().take(32).any(|b| addresses.iter().any(|a| a.ip() == b.ip())) { bail!("destination belongs to a Glyph server"); }
            }
        }
        let http = reqwest::Client::builder().no_proxy()
            .user_agent(concat!("Glyph/", env!("CARGO_PKG_VERSION"), " (plugin)"))
            .redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(20))
            .resolve_to_addrs(&host, &addresses).build().map_err(|_| anyhow!("could not create plugin HTTP client"))?;
        let mut req = http.request(method, url).headers(checked_headers);
        if let Some(j) = args.get("json").filter(|v| !v.is_null()) { req = req.json(j); }
        else if let Some(b) = args["body"].as_str() { req = req.body(b.to_string()); }
        // Do not include URLs (which may contain injected credentials) in errors.
        let r = req.send().await.map_err(|_| anyhow!("plugin network request failed"))?;
        let status = r.status().as_u16();
        let headers: Map<String, Value> = r.headers().iter()
            .filter(|(k, _)| !matches!(k.as_str(), "set-cookie" | "location"))
            .filter_map(|(k, v)| Some((k.as_str().to_string(), Value::String(redact_credentials(v.to_str().ok()?.to_string(), &secrets))))).collect();
        let mut body = Vec::new();
        let mut stream = r.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| anyhow!("could not read plugin network response"))?;
            if body.len() + chunk.len() > MAX_FETCH { bail!("answer larger than 4 MiB"); }
            body.extend_from_slice(&chunk);
        }
        Ok(json!({ "status": status, "headers": headers, "body": redact_credentials(String::from_utf8_lossy(&body).into_owned(), &secrets) }))
            } => result,
            _ = async {
                loop {
                    if active.is_some_and(|a| !a.load(Ordering::Acquire)) { break; }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            } => Err(anyhow!("plugin was stopped")),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        serde_json::from_value(json!({
            "schema": 2, "id": "probe", "name": "Probe", "version": "1.0.0", "kind": "extension",
            "main": "probe.wasm", "permissions": ["playback", "network"], "network": ["api.example.com"]
        }))
        .unwrap()
    }

    fn state(dir: &std::path::Path) -> (State, flume::Receiver<HostCall>) {
        let (tx, rx) = flume::unbounded();
        let s = State {
            id: "probe".into(),
            generation: 1, active: Arc::new(AtomicBool::new(true)), blocked_hosts: Vec::new(), budget: Default::default(),
            manifest: manifest(),
            calls: tx,
            pending: Vec::new(),
            storage: Map::new(),
            storage_path: dir.join("storage.json"),
            timers: Vec::new(),
            logs: Default::default(),
            limits: StoreLimitsBuilder::new().build(),
        };
        (s, rx)
    }

    #[test]
    fn permissions_are_checked_before_anything_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, rx) = state(dir.path());
        let r = handle(&mut s, br#"{"fn":"library.playlists"}"#);
        assert!(r["error"].as_str().unwrap().contains("\"library\" permission"));
        assert!(rx.is_empty());
        assert_eq!(permission_for("player.next"), Some("playback"));
        assert_eq!(permission_for("ui.toast"), None);
        assert_eq!(permission_for("net.fetch"), Some("network"));
        assert!(handle(&mut s, b"nope")["error"].is_string());
        assert!(handle(&mut s, br#"{"fn":"ui.toast","args":[1]}"#)["error"].is_string());
    }

    #[test]
    fn calls_go_to_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, rx) = state(dir.path());
        std::thread::spawn(move || {
            let c = rx.recv().unwrap();
            assert_eq!(c.plugin, "probe");
            let _ = c.reply.send(json!({ "ok": c.func }));
        });
        assert_eq!(handle(&mut s, br#"{"fn":"player.next"}"#), json!({ "ok": "player.next" }));
    }

    #[test]
    fn storage_persists_and_has_a_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, _rx) = state(dir.path());
        assert_eq!(handle(&mut s, br#"{"fn":"storage.set","args":{"key":"a","value":[1,2]}}"#), json!({ "ok": null }));
        assert_eq!(handle(&mut s, br#"{"fn":"storage.get","args":{"key":"a"}}"#), json!({ "ok": [1, 2] }));
        let saved: Map<String, Value> = crate::paths::read_json(&dir.path().join("storage.json")).unwrap();
        assert_eq!(saved["a"], json!([1, 2]));
        let big = format!(r#"{{"fn":"storage.set","args":{{"key":"b","value":"{}"}}}}"#, "x".repeat(MAX_STORAGE));
        assert!(handle(&mut s, big.as_bytes())["error"].is_string());
        assert!(s.storage.get("b").is_none());
    }

    #[test]
    fn timers_fire_and_repeat() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, _rx) = state(dir.path());
        handle(&mut s, br#"{"fn":"timer.set","args":{"id":"once","ms":50}}"#);
        handle(&mut s, br#"{"fn":"timer.set","args":{"id":"tick","ms":50,"repeat":true}}"#);
        for t in &mut s.timers {
            t.due = Instant::now() - Duration::from_millis(1);
        }
        let fired = due_timers(&mut s);
        assert_eq!(fired.len(), 2);
        assert_eq!(s.timers.len(), 1);
        assert_eq!(s.timers[0].id, "tick");
    }

    #[test]
    fn fetch_stays_on_allowed_hosts() {
        let m = manifest();
        assert!(fetch(&m, &json!({ "url": "http://api.example.com/x" }), &[], None).is_err());
        assert!(fetch(&m, &json!({ "url": "https://evil.com/x" }), &[], None).is_err());
        assert!(fetch(&m, &json!({ "url": "https://api.example.com:8443/x" }), &[], None).is_err());
        assert!(fetch(&m, &json!({ "url": "https://api.example.com/x", "method": "TRACE" }), &[], None).is_err());
    }

    #[test]
    fn server_destinations_and_identity_headers_are_not_plugin_capabilities() {
        let mut m = manifest();
        m.network = vec!["*.example.com".into()];
        for url in ["https://api.example.com/v1/config", "https://user:pass@api.example.com/", "https://api.example.com/#secret"] {
            assert!(checked_url(&m, &json!({"url": url}), &["api.example.com".into()]).is_err());
        }
        for header in ["Host", "Cookie", "X-Glyph-Server-Authorization", "X-Forwarded-For", "Content-Length"] {
            assert!(!allowed_header(header));
        }
        assert!(allowed_header("Authorization")); // Only explicitly supplied, plugin-owned credentials.
        assert!(inject_credentials(&m, &json!({"credentials":{"query":{"key":"account:token"}}}), &mut reqwest::Url::parse("https://api.example.com/").unwrap()).is_err());
    }

    #[test]
    fn stopped_and_unknown_calls_cannot_reach_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, rx) = state(dir.path());
        assert!(handle(&mut s, br#"{"fn":"host.stopped"}"#)["error"].is_string());
        assert!(rx.is_empty());
        s.active.store(false, Ordering::Release);
        assert!(handle(&mut s, br#"{"fn":"player.next"}"#)["error"].is_string());
        assert!(rx.is_empty());
    }

    #[test]
    fn credential_injection_rejects_destination_and_field_changes_before_loading_keys() {
        let mut m = manifest();
        m.settings = serde_json::from_value(json!([{"id":"account","items":[{
            "id":"api_key", "type":"text", "label":"Key", "secret":true,
            "credential":{"host":"api.example.com","path":"/v1/","location":"query","name":"api_key"}
        }]}])).unwrap();
        for (url, args) in [
            ("https://evil.example.com/v1/", json!({"credentials":{"query":{"api_key":"api_key"}}})),
            ("https://api.example.com/echo", json!({"credentials":{"query":{"api_key":"api_key"}}})),
            ("https://api.example.com/v1/", json!({"credentials":{"query":{"callback":"api_key"}}})),
            ("https://api.example.com/v1/", json!({"credentials":{"headers":{"api_key":"api_key"}}})),
        ] {
            let error = inject_credentials(&m, &args, &mut reqwest::Url::parse(url).unwrap()).unwrap_err();
            assert!(error.to_string().contains("bound to a different"));
        }
    }

    #[test]
    fn direct_credential_echoes_are_redacted() {
        let secrets = vec!["private-key".into(), "a/b".into()];
        assert_eq!(redact_credentials("private-key a%2Fb a/b".into(), &secrets), "[redacted] [redacted] [redacted]");
    }

    /// A hand-written module: on each event it reads the event, calls
    /// `player.next` and logs; `glyph_start` sets a timer.
    #[test]
    fn runs_a_module_end_to_end() {
        let wat = r#"
          (module
            (import "glyph" "call" (func $call (param i32 i32) (result i32)))
            (import "glyph" "read" (func $read (param i32) (result i32)))
            (import "glyph" "log" (func $log (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "{\"fn\":\"player.next\"}")
            (data (i32.const 64) "hello")
            (func (export "glyph_start")
              (drop (call $log (i32.const 1) (i32.const 64) (i32.const 5))))
            (func (export "glyph_event") (param $len i32)
              (drop (call $read (i32.const 1024)))
              (drop (call $call (i32.const 0) (i32.const 20)))))
        "#;
        let _env = crate::paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GLYPH_DATA_DIR", dir.path());
        let pdir = plugins::plugins_dir().join("probe");
        std::fs::create_dir_all(&pdir).unwrap();
        std::fs::write(pdir.join("probe.wasm"), wat::parse_str(wat).unwrap()).unwrap();
        let inst = Installed { manifest: manifest(), origin: serde_json::from_value(json!({ "repo": "local", "rev": "local", "installed_at": 0 })).unwrap() };
        let (tx, rx) = flume::unbounded();
        let running = start(&inst, tx, "https://api.glyph.my").unwrap();
        assert!(running.event(json!({ "type": "track" })));
        let call = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(call.func, "player.next");
        let _ = call.reply.send(json!({ "ok": null }));
        assert!(running.log_lines().iter().any(|l| l.contains("hello")));
        drop(running);
        std::env::remove_var("GLYPH_DATA_DIR");
    }

    /// An endless loop is stopped by its fuel budget, the app keeps going.
    #[test]
    fn endless_loops_run_out_of_fuel() {
        let wat = r#"
          (module
            (memory (export "memory") 1)
            (func (export "glyph_event") (param i32) (loop $l (br $l))))
        "#;
        let engine = engine();
        let module = Module::new(&engine, &wat::parse_str(wat).unwrap()[..]).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let (tx, rx) = flume::unbounded();
        let (calls, _calls_rx) = flume::unbounded();
        let h = std::thread::spawn(move || run(engine, module, manifest(), path, rx, calls, Default::default(), 1, Arc::new(AtomicBool::new(true)), Vec::new()));
        for _ in 0..MAX_STRIKES {
            tx.send(Msg::Event(json!({ "type": "x" }))).unwrap();
        }
        let r = h.join().unwrap();
        assert!(r.unwrap_err().to_string().contains("too long"));
    }
}
