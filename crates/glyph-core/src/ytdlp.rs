//! Local YouTube resolver: runs yt-dlp on this machine (the user's home IP)
//! to turn a video id into a direct audio URL that glyph-audio plays like any
//! other stream. Hosted servers get YouTube's bot check, a home connection
//! does not.
//!
//! * Binary: `providers.ytdlp_path` from the config, else `yt-dlp` on PATH,
//!   else the app-managed copy in `data_dir()/bin/yt-dlp` (see [`install`]).
//! * Several videos (the next songs of the queue) resolve in one run
//!   ([`YtDlp::resolve_many`]): yt-dlp's start-up is paid once.
//! * Only validated 11-character video ids and `ytsearchN:` queries reach the
//!   command line, always after `--`. No cookies, no login, no user config.
//! * YouTube needs a JavaScript runtime for its player challenges; yt-dlp uses
//!   deno by default. Without deno an installed node/bun is enabled instead.
//! * Resolved URLs are cached until shortly before they expire (`expire=` in
//!   the URL), so replays and preloads do not run yt-dlp again.

use crate::paths;
use crate::provider::{Candidate, ProviderError};
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Hard limit for one yt-dlp run.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(20);
/// Audio formats glyph-audio can decode (symphonia has no Opus): AAC in MP4
/// first, MP3 as a fallback. WebM/Opus-only videos fail and use the embed.
const AUDIO_FORMAT: &str = "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio[acodec=mp3]";
/// Data-saver downloads: the smallest AAC stream (often ~48 kbps), else the
/// normal one. AAC ~128 kbps is the best YouTube offers that symphonia plays.
const SMALL_AUDIO_FORMAT: &str =
    "worstaudio[ext=m4a]/worstaudio[acodec^=mp4a]/bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio[acodec=mp3]";
/// Results per search, like the API search.
pub const SEARCH_RESULTS: usize = 10;
/// Videos per batch run, and the extra time each one gets.
pub const MAX_BATCH: usize = 5;
const BATCH_EXTRA: Duration = Duration::from_secs(6);
/// A cached URL is used only while it stays valid at least this long
/// (a preloaded song can start minutes later).
const URL_MARGIN_S: u64 = 30 * 60;
/// Lifetime assumed when the URL carries no `expire=`.
const URL_DEFAULT_TTL_S: u64 = 60 * 60;
const RELEASE_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
/// The biggest official archive (macOS) is ~55 MB.
const MAX_BINARY_BYTES: u64 = 150 * 1024 * 1024;

// ------------------------------------------------------------- validation

/// `true` for a YouTube video id: exactly 11 of `[A-Za-z0-9_-]`.
pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Search text safe for `ytsearchN:` (no control characters, bounded length).
fn clean_query(q: &str) -> String {
    let s: String = q.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.chars().take(200).collect()
}

// ------------------------------------------------------------- arguments

fn base_args(js: Option<&JsRuntime>) -> Vec<String> {
    let mut v: Vec<String> = ["--ignore-config", "--no-warnings", "--no-progress", "--no-cookies", "--no-playlist"]
        .into_iter()
        .map(String::from)
        .collect();
    if let Some(rt) = js.filter(|r| r.name != "deno") {
        // deno is on by default; others have to be enabled.
        v.push("--js-runtimes".into());
        v.push(rt.name.into());
    }
    v
}

/// Arguments for resolving one video to its audio URL (`-J` = one JSON object).
pub fn resolve_args(video_id: &str, js: Option<&JsRuntime>) -> Result<Vec<String>> {
    resolve_args_with(video_id, js, AUDIO_FORMAT)
}

fn resolve_args_with(video_id: &str, js: Option<&JsRuntime>, format: &'static str) -> Result<Vec<String>> {
    if !valid_video_id(video_id) {
        bail!(ProviderError::NotFound);
    }
    let mut v = base_args(js);
    v.extend(["-J".into(), "-f".into(), format.into(), "--".into()]);
    v.push(format!("https://www.youtube.com/watch?v={video_id}"));
    Ok(v)
}

/// Arguments for resolving several videos in one run (`-j` = one JSON line
/// per video, `--ignore-errors` = a broken video does not stop the rest).
pub fn batch_args(video_ids: &[&str], js: Option<&JsRuntime>) -> Result<Vec<String>> {
    if video_ids.is_empty() || video_ids.iter().any(|id| !valid_video_id(id)) {
        bail!(ProviderError::NotFound);
    }
    let mut v = base_args(js);
    v.extend(["-j".into(), "--ignore-errors".into(), "-f".into(), AUDIO_FORMAT.into(), "--".into()]);
    v.extend(video_ids.iter().map(|id| format!("https://www.youtube.com/watch?v={id}")));
    Ok(v)
}

/// Arguments for a flat search (ids, titles, channels, durations; no
/// per-video extraction).
pub fn search_args(query: &str, n: usize, js: Option<&JsRuntime>) -> Result<Vec<String>> {
    let q = clean_query(query);
    if q.is_empty() {
        bail!(ProviderError::NotFound);
    }
    let mut v = base_args(js);
    v.extend(["--flat-playlist".into(), "--dump-single-json".into(), "--".into()]);
    v.push(format!("ytsearch{}:{q}", n.clamp(1, 25)));
    Ok(v)
}

// ---------------------------------------------------------------- parsing

/// A playable audio stream for one video.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalAudio {
    pub video_id: String,
    pub url: String,
    pub mime: Option<String>,
    pub duration_ms: Option<u64>,
    pub title: String,
    pub channel: String,
    /// Unix seconds after which the URL stops working.
    pub expires: u64,
    /// Headers yt-dlp says the URL needs (User-Agent, ...).
    pub headers: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct VideoJson {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    uploader: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    acodec: Option<String>,
    #[serde(default)]
    format_id: Option<String>,
    #[serde(default)]
    formats: Vec<FormatJson>,
    #[serde(default)]
    live_status: Option<String>,
    #[serde(default)]
    http_headers: HashMap<String, String>,
}

#[derive(Deserialize)]
struct FormatJson {
    #[serde(default)]
    format_id: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    acodec: Option<String>,
}

fn mime_for(ext: Option<&str>, acodec: Option<&str>) -> Option<String> {
    let m = match (ext.unwrap_or(""), acodec.unwrap_or("")) {
        ("m4a" | "mp4", _) => "audio/mp4",
        (_, a) if a.starts_with("mp4a") => "audio/mp4",
        ("mp3", _) | (_, "mp3") => "audio/mpeg",
        ("webm", _) => "audio/webm",
        _ => return None,
    };
    Some(m.into())
}

/// `expire=` (unix seconds) from a googlevideo URL.
pub fn url_expiry(url: &str) -> Option<u64> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|kv| kv.strip_prefix("expire=")).and_then(|v| v.parse().ok())
}

/// Parses `yt-dlp -J -f …` output for one video.
pub fn parse_resolve(json: &[u8], now: u64) -> Result<LocalAudio> {
    let v: VideoJson = serde_json::from_slice(json).context("yt-dlp output is not JSON")?;
    if !valid_video_id(&v.id) {
        bail!("yt-dlp returned no video id");
    }
    if matches!(v.live_status.as_deref(), Some("is_live" | "is_upcoming")) {
        bail!(ProviderError::Unsupported);
    }
    // With one selected format its fields sit at the top level; else look it
    // up by id among the formats.
    let (url, ext, acodec) = match v.url.clone() {
        Some(u) => (u, v.ext.clone(), v.acodec.clone()),
        None => {
            let fid = v.format_id.as_deref().unwrap_or("");
            let f = v.formats.iter().find(|f| f.format_id == fid).ok_or_else(|| anyhow!("yt-dlp selected no audio format"))?;
            (f.url.clone().ok_or_else(|| anyhow!("format has no URL"))?, f.ext.clone(), f.acodec.clone())
        }
    };
    if !url.starts_with("https://") {
        bail!("unexpected stream URL");
    }
    let expires = url_expiry(&url).unwrap_or(now + URL_DEFAULT_TTL_S);
    Ok(LocalAudio {
        video_id: v.id,
        mime: mime_for(ext.as_deref(), acodec.as_deref()),
        url,
        duration_ms: v.duration.and_then(crate::provider::secs_to_ms),
        title: v.title,
        channel: v.channel.or(v.uploader).unwrap_or_default(),
        expires,
        // Never forward credentials (there are none: no cookies, no login).
        headers: v
            .http_headers
            .into_iter()
            .filter(|(k, _)| !matches!(k.to_ascii_lowercase().as_str(), "cookie" | "authorization"))
            .collect(),
    })
}

/// Parses `-j` batch output (one JSON object per line); broken lines are
/// skipped.
pub fn parse_batch(out: &[u8], now: u64) -> Vec<LocalAudio> {
    out.split(|b| *b == b'\n')
        .filter(|l| l.iter().any(|b| !b.is_ascii_whitespace()))
        .filter_map(|l| parse_resolve(l, now).ok())
        .collect()
}

#[derive(Deserialize)]
struct SearchJson {
    #[serde(default)]
    entries: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct EntryJson {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    uploader: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    live_status: Option<String>,
}

/// Parses `--flat-playlist --dump-single-json ytsearchN:` output into
/// resolver candidates. Channels, playlists, live streams and broken entries
/// are skipped.
pub fn parse_search(json: &[u8]) -> Result<Vec<Candidate>> {
    let s: SearchJson = serde_json::from_slice(json).context("yt-dlp output is not JSON")?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for raw in s.entries {
        let Ok(e) = serde_json::from_value::<EntryJson>(raw) else { continue };
        if !valid_video_id(&e.id) || e.title.is_empty() || !seen.insert(e.id.clone()) {
            continue;
        }
        if matches!(e.live_status.as_deref(), Some("is_live" | "is_upcoming" | "was_live")) {
            continue;
        }
        let channel = e.channel.or(e.uploader).unwrap_or_default();
        let dur = e.duration.and_then(crate::provider::secs_to_ms);
        let (track, bonus) = crate::providers::youtube::video_track(&e.id, &e.title, &channel, None, dur);
        out.push(Candidate { track, bonus });
    }
    Ok(out)
}

/// Shortens yt-dlp's stderr to its error line (for logs and toasts).
pub fn error_line(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .find(|l| l.contains("ERROR"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("yt-dlp failed")
        .trim();
    line.trim_start_matches("ERROR:").trim().chars().take(300).collect()
}

/// Maps a failed run to a provider error: a missing / private video is a
/// plain "not found" (no cooldown), everything else is a real failure.
fn classify(stderr: &str) -> anyhow::Error {
    let msg = error_line(stderr);
    let lower = msg.to_lowercase();
    if lower.contains("video unavailable") || lower.contains("private video") || lower.contains("has been removed") {
        return ProviderError::NotFound.into();
    }
    if lower.contains("requested format is not available") {
        return ProviderError::Unsupported.into();
    }
    ProviderError::Other(format!("yt-dlp: {msg}")).into()
}

// --------------------------------------------------------------- binaries

/// A JavaScript runtime yt-dlp can use for YouTube's player challenges.
#[derive(Debug, Clone, PartialEq)]
pub struct JsRuntime {
    /// `deno`, `node` or `bun` (yt-dlp's `--js-runtimes` names).
    pub name: &'static str,
    pub path: PathBuf,
}

fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

/// First `name` (with `.exe` on Windows) found on PATH.
pub fn which(name: &str) -> Option<PathBuf> {
    let file = exe_name(name);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(&file))
        .find(|p| p.is_file())
}

/// Folder of the app-managed copy (the unpacked "onedir" release: the
/// executable next to `_internal/`).
pub fn managed_dir() -> PathBuf {
    paths::data_dir().join("bin").join("yt-dlp")
}

/// The executable inside an unpacked release folder (`yt-dlp.exe`,
/// `yt-dlp_linux`, `yt-dlp_macos`, ...).
fn exe_in(dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.starts_with("yt-dlp") && if cfg!(windows) { name.ends_with(".exe") } else { !name.contains('.') }
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

/// The app-managed copy, if installed.
pub fn managed_path() -> Option<PathBuf> {
    exe_in(&managed_dir())
}

/// Which yt-dlp to run: configured path, PATH, app-managed copy.
pub fn locate(configured: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = configured.map(str::trim).filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
        tracing::warn!("ytdlp_path {} does not exist, looking elsewhere", p.display());
    }
    which("yt-dlp").or_else(managed_path)
}

/// deno first (yt-dlp's default), then node and bun.
pub fn find_js_runtime() -> Option<JsRuntime> {
    ["deno", "node", "bun"].into_iter().find_map(|name| which(name).map(|path| JsRuntime { name, path }))
}

fn command(bin: &Path) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(bin);
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    {
        // No console window flashing up from the GUI app.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        c.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    c
}

/// Runs `bin args…` with [`RUN_TIMEOUT`]; stdout on success.
async fn run(bin: &Path, args: &[String]) -> Result<Vec<u8>> {
    run_with(bin, args, RUN_TIMEOUT, false).await
}

/// `partial_ok`: a failed exit still returns stdout when it has output (a
/// batch where only some videos failed).
async fn run_with(bin: &Path, args: &[String], timeout: Duration, partial_ok: bool) -> Result<Vec<u8>> {
    static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let _permit = SLOTS.acquire().await.map_err(|_| anyhow!("yt-dlp scheduler stopped"))?;
    let child = command(bin).args(args).spawn().with_context(|| format!("cannot start {}", bin.display()))?;
    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| ProviderError::Timeout)??;
    if !out.status.success() && !(partial_ok && !out.stdout.is_empty()) {
        return Err(classify(&String::from_utf8_lossy(&out.stderr)));
    }
    Ok(out.stdout)
}

/// `yt-dlp --version`, e.g. `2026.09.30`. Generous timeout: the first start
/// of a freshly downloaded exe waits for the virus scanner on Windows.
pub async fn version(bin: &Path) -> Option<String> {
    let out = tokio::time::timeout(Duration::from_secs(60), command(bin).arg("--version").output()).await.ok()?.ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}

// ------------------------------------------------------------- the runner

static URL_CACHE: Mutex<Option<HashMap<String, LocalAudio>>> = Mutex::new(None);

/// Drops every cached URL (after a 403 the whole batch is suspect).
pub fn forget_urls() {
    *URL_CACHE.lock().unwrap() = None;
}

fn forget_url(id: &str) {
    if let Some(m) = URL_CACHE.lock().unwrap().as_mut() {
        m.remove(id);
    }
}

fn cached(id: &str, now: u64) -> Option<LocalAudio> {
    let g = URL_CACHE.lock().unwrap();
    g.as_ref()?.get(id).filter(|a| a.expires > now + URL_MARGIN_S).cloned()
}

fn store(a: &LocalAudio) {
    let mut g = URL_CACHE.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 500 {
        let now = paths::now_unix();
        m.retain(|_, v| v.expires > now + URL_MARGIN_S);
    }
    m.insert(a.video_id.clone(), a.clone());
}

/// Videos a run is resolving right now: a second request for the same id
/// (hover prefetch, then the click) waits for it instead of starting
/// another yt-dlp.
static INFLIGHT: Mutex<Option<HashMap<String, Arc<tokio::sync::Notify>>>> = Mutex::new(None);

/// Ids claimed by one run; released (and waiters woken) on drop.
struct Claim(Vec<(String, Arc<tokio::sync::Notify>)>);

impl Drop for Claim {
    fn drop(&mut self) {
        let mut g = INFLIGHT.lock().unwrap();
        if let Some(m) = g.as_mut() {
            for (id, _) in &self.0 {
                m.remove(id);
            }
        }
        drop(g);
        for (_, n) in &self.0 {
            n.notify_waiters();
        }
    }
}

/// Claims the ids nobody is resolving yet; returns the claim and the ids
/// that are already in flight elsewhere.
fn claim(ids: &[&str]) -> (Claim, Vec<(String, Arc<tokio::sync::Notify>)>) {
    let mut g = INFLIGHT.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    let (mut mine, mut busy) = (Vec::new(), Vec::new());
    for id in ids {
        match m.get(*id) {
            Some(n) => busy.push((id.to_string(), n.clone())),
            None => {
                let n = Arc::new(tokio::sync::Notify::new());
                m.insert(id.to_string(), n.clone());
                mine.push((id.to_string(), n));
            }
        }
    }
    (Claim(mine), busy)
}

fn in_flight(id: &str) -> bool {
    INFLIGHT.lock().unwrap().as_ref().is_some_and(|m| m.contains_key(id))
}

/// Waits (bounded) until another run finished `id`.
async fn wait_for(id: &str, n: &tokio::sync::Notify) {
    let done = n.notified();
    tokio::pin!(done);
    done.as_mut().enable();
    if in_flight(id) {
        let _ = tokio::time::timeout(RUN_TIMEOUT + BATCH_EXTRA * MAX_BATCH as u32, done).await;
    }
}

/// A located yt-dlp plus the JS runtime it will use.
#[derive(Debug, Clone)]
pub struct YtDlp {
    pub bin: PathBuf,
    pub js: Option<JsRuntime>,
}

impl YtDlp {
    /// `None` when no yt-dlp is installed.
    pub fn locate(configured: Option<&str>) -> Option<Self> {
        Some(Self { bin: locate(configured)?, js: find_js_runtime() })
    }

    /// Direct audio URL for a video (cached until shortly before it expires).
    pub async fn resolve(&self, video_id: &str) -> Result<LocalAudio> {
        let args = resolve_args(video_id, self.js.as_ref())?;
        let mut waited = false;
        let _claim = loop {
            if let Some(hit) = cached(video_id, paths::now_unix()) {
                return Ok(hit);
            }
            let (claim, busy) = claim(&[video_id]);
            match busy.into_iter().next() {
                // Someone else is on it: wait once, then do it ourselves.
                Some((id, n)) if !waited => {
                    waited = true;
                    wait_for(&id, &n).await;
                }
                Some(_) => break claim,
                None => break claim,
            }
        };
        let now = paths::now_unix();
        let out = run(&self.bin, &args).await;
        let audio = match out {
            Ok(o) => parse_resolve(&o, now)?,
            Err(e) => {
                tracing::info!(video_id, "yt-dlp resolve failed: {e:#}");
                return Err(e);
            }
        };
        store(&audio);
        Ok(audio)
    }

    /// Resolves several videos in ONE yt-dlp run (the start-up cost is paid
    /// once) and fills the URL cache. Already cached / invalid ids are
    /// skipped; videos that fail are left out. Returns how many were added.
    pub async fn resolve_many(&self, video_ids: &[String]) -> Result<usize> {
        let now = paths::now_unix();
        let mut todo: Vec<&str> = Vec::new();
        for id in video_ids {
            if valid_video_id(id) && cached(id, now).is_none() && !todo.contains(&id.as_str()) {
                todo.push(id);
            }
        }
        todo.truncate(MAX_BATCH);
        match todo.len() {
            0 => return Ok(0),
            1 => return self.resolve(todo[0]).await.map(|_| 1),
            _ => {}
        }
        // Ids another run is already resolving are left to it.
        let (claim, _busy) = claim(&todo);
        let todo: Vec<&str> = claim.0.iter().map(|(id, _)| id.as_str()).collect();
        if todo.len() < 2 {
            let single = todo.first().map(|id| id.to_string());
            drop(claim);
            return match single {
                Some(id) => self.resolve(&id).await.map(|_| 1),
                None => Ok(0),
            };
        }
        let args = batch_args(&todo, self.js.as_ref())?;
        let timeout = RUN_TIMEOUT + BATCH_EXTRA * (todo.len() as u32 - 1);
        // With --ignore-errors a failing video only drops its own line.
        let out = run_with(&self.bin, &args, timeout, true).await?;
        let found = parse_batch(&out, now);
        for a in &found {
            store(a);
        }
        Ok(found.len())
    }

    pub async fn search(&self, query: &str, n: usize) -> Result<Vec<Candidate>> {
        let out = run(&self.bin, &search_args(query, n, self.js.as_ref())?).await?;
        parse_search(&out)
    }
}

// ------------------------------------------------------------- audio cache
//
// googlevideo URLs stop answering (HTTP 403) once a stream is read at
// playback pace for about a minute; read in one go they work. So a song is
// downloaded completely right away (parallel ranges, a few seconds) and
// played from this cache. Replays need neither yt-dlp nor the network.

/// Range size and parallel requests for the download.
const PART: u64 = 1024 * 1024;
const PARALLEL: usize = 4;
/// Songs larger than this are refused (a 10 h "mix").
const MAX_AUDIO_BYTES: u64 = 80 * 1024 * 1024;
/// Total size of the cache before the oldest songs go.
pub const AUDIO_CACHE_MAX_BYTES: u64 = 400 * 1024 * 1024;

pub fn audio_cache_dir() -> PathBuf {
    paths::cache_dir().join("youtube")
}

fn ext_for(mime: Option<&str>) -> &'static str {
    match mime {
        Some("audio/mpeg") => "mp3",
        Some("audio/webm") => "webm",
        _ => "m4a",
    }
}

pub fn mime_for_path(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()) {
        Some("mp3") => "audio/mpeg",
        Some("webm") => "audio/webm",
        _ => "audio/mp4",
    }
}

/// The complete cached file of a video (refreshes its age for pruning).
pub fn cached_audio(video_id: &str) -> Option<PathBuf> {
    if !valid_video_id(video_id) {
        return None;
    }
    let dir = audio_cache_dir();
    let p = ["m4a", "mp3", "webm"].iter().map(|e| dir.join(format!("{video_id}.{e}"))).find(|p| p.is_file())?;
    if let Ok(f) = std::fs::File::options().write(true).open(&p) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
    Some(p)
}

/// Deletes the oldest songs until the cache fits `max_bytes`.
pub fn prune_audio_cache(dir: &Path, max_bytes: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// `true` for an HTTP 403/401 (the URL is no longer good).
fn is_forbidden(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<reqwest::Error>()
            .and_then(|r| r.status())
            .is_some_and(|s| s.as_u16() == 403 || s.as_u16() == 401)
            || matches!(c.downcast_ref::<ProviderError>(), Some(ProviderError::Http(403 | 401)))
    })
}

async fn get_range(http: &reqwest::Client, a: &LocalAudio, from: u64, to: u64) -> Result<(Option<u64>, Vec<u8>)> {
    let mut req = http.get(&a.url).header(reqwest::header::RANGE, format!("bytes={from}-{to}")).timeout(Duration::from_secs(30));
    for (k, v) in &a.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await?;
    let status = resp.status().as_u16();
    if status != 206 && status != 200 {
        bail!(ProviderError::Http(status));
    }
    let total = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let body = resp.bytes().await?.to_vec();
    if status == 200 && from > 0 {
        bail!("server ignored the Range request");
    }
    Ok((if status == 200 { Some(body.len() as u64) } else { total }, body))
}

/// Downloads the whole stream of `a` into `dir` (parallel ranges) and
/// returns the file.
pub async fn download_audio(http: &reqwest::Client, a: &LocalAudio, dir: &Path) -> Result<PathBuf> {
    let (total, first) = get_range(http, a, 0, PART - 1).await?;
    let total = total.ok_or_else(|| anyhow!("stream size unknown"))?;
    if total > MAX_AUDIO_BYTES {
        bail!("audio stream too large");
    }
    let mut data = vec![0u8; total as usize];
    let n = first.len().min(data.len());
    data[..n].copy_from_slice(&first[..n]);
    crate::downloads::report(|p| {
        p.set_total(total);
        p.add(n as u64);
    });
    let ranges: Vec<(u64, u64)> = (n as u64..total).step_by(PART as usize).map(|s| (s, (s + PART).min(total) - 1)).collect();
    let mut parts = futures::stream::iter(ranges)
        .map(|(from, to)| async move { get_range(http, a, from, to).await.map(|(_, b)| (from, to, b)) })
        .buffer_unordered(PARALLEL);
    while let Some(part) = parts.next().await {
        let (from, to, body) = part?;
        if body.len() as u64 != to - from + 1 {
            bail!("short read from the stream");
        }
        data[from as usize..=to as usize].copy_from_slice(&body);
        crate::downloads::report(|p| p.add(body.len() as u64));
    }
    let path = dir.join(format!("{}.{}", a.video_id, ext_for(a.mime.as_deref())));
    let dest = path.clone();
    tokio::task::spawn_blocking(move || paths::write_atomic(&dest, &data)).await??;
    Ok(path)
}

impl YtDlp {
    /// The song as a local file: from the cache, else resolved (yt-dlp) and
    /// downloaded at once. An expired URL (403) is resolved once more.
    ///
    /// Inside a data-saver download job the smaller stream is fetched into
    /// [`small_audio_dir`] instead (the caller copies and deletes it).
    pub async fn audio_file(&self, http: &reqwest::Client, video_id: &str) -> Result<PathBuf> {
        if crate::downloads::job_quality() == crate::config::AudioQuality::Low {
            return self.small_audio_file(http, video_id).await;
        }
        if let Some(p) = cached_audio(video_id) {
            crate::downloads::report(|p| p.finish());
            return Ok(p);
        }
        let key = format!("file:{video_id}");
        let (claim, busy) = claim(&[key.as_str()]);
        if let Some((k, n)) = busy.into_iter().next() {
            drop(claim);
            wait_for(&k, &n).await;
            if let Some(p) = cached_audio(video_id) {
                return Ok(p);
            }
        }
        let dir = audio_cache_dir();
        let a = self.resolve(video_id).await?;
        let path = match download_audio(http, &a, &dir).await {
            Ok(p) => p,
            Err(e) if is_forbidden(&e) => {
                tracing::info!(video_id, "stream URL refused, resolving again");
                forget_url(video_id);
                let a = self.resolve(video_id).await?;
                download_audio(http, &a, &dir).await?
            }
            Err(e) => return Err(e),
        };
        tokio::task::spawn_blocking(move || prune_audio_cache(&dir, AUDIO_CACHE_MAX_BYTES));
        Ok(path)
    }

    /// The data-saver stream of a video, downloaded into [`small_audio_dir`]
    /// (fresh yt-dlp run, the URL cache holds the normal stream).
    async fn small_audio_file(&self, http: &reqwest::Client, video_id: &str) -> Result<PathBuf> {
        let args = resolve_args_with(video_id, self.js.as_ref(), SMALL_AUDIO_FORMAT)?;
        let a = parse_resolve(&run(&self.bin, &args).await?, paths::now_unix())?;
        download_audio(http, &a, &small_audio_dir()).await
    }
}

/// Where data-saver downloads land before they are copied to the downloads.
pub fn small_audio_dir() -> PathBuf {
    audio_cache_dir().join("small")
}

// ------------------------------------------------------------ status/install

/// What the settings page shows about yt-dlp.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct YtDlpStatus {
    pub latest_version: Option<String>,
    pub update_available: Option<bool>,
    pub update_error: Option<String>,
    pub path: Option<PathBuf>,
    /// `None` with a path: the binary did not answer `--version`.
    pub version: Option<String>,
    /// The path is the app-managed copy (the install button updates it).
    pub managed: bool,
    /// `deno` / `node` / `bun`; `None` = YouTube will mostly fail.
    pub js_runtime: Option<String>,
    /// An official release binary exists for this OS / CPU.
    pub can_install: bool,
}

pub async fn status(configured: Option<&str>) -> YtDlpStatus {
    let path = locate(configured);
    let version = match &path {
        Some(p) => version(p).await,
        None => None,
    };
    YtDlpStatus {
        latest_version: None,
        update_available: None,
        update_error: None,
        managed: path.is_some() && path == managed_path(),
        path,
        version,
        js_runtime: find_js_runtime().map(|r| r.name.to_string()),
        can_install: release_asset().is_some(),
    }
}

fn release_version(value: &str) -> Option<[u32;3]> {
    let mut parts=value.trim().trim_start_matches('v').split('.');
    Some([parts.next()?.parse().ok()?,parts.next()?.parse().ok()?,parts.next()?.parse().ok()?])
}

pub async fn status_with_updates(configured: Option<&str>, http: &reqwest::Client) -> YtDlpStatus {
    let latest=async {
        let response=http.get("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest").timeout(Duration::from_secs(8)).send().await?.error_for_status()?;
        let json:serde_json::Value=response.json().await?;
        let tag=json["tag_name"].as_str().filter(|s| release_version(s).is_some()).ok_or_else(|| anyhow!("invalid yt-dlp release version"))?;
        Ok::<_,anyhow::Error>(tag.trim_start_matches('v').to_string())
    };
    let (mut status,latest)=tokio::join!(status(configured),latest);
    match latest {
        Ok(latest) => {
            status.update_available=status.version.as_deref().and_then(release_version).zip(release_version(&latest)).map(|(installed,new)| installed<new);
            status.latest_version=Some(latest);
            if status.version.is_some() && status.update_available.is_none() { status.update_error=Some("unrecognized installed version".into()); }
        }
        Err(_) => status.update_error=Some("release check unavailable".into()),
    }
    status
}

/// The official release archive for this OS / CPU. The "onedir" ZIPs are
/// used instead of the single-file builds: those unpack themselves on every
/// start (~4 s on Windows), the unpacked folder starts in under a second.
pub fn release_asset() -> Option<&'static str> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    Some(match (os, arch) {
        ("windows", "x86_64") => "yt-dlp_win.zip",
        ("windows", "x86") => "yt-dlp_win_x86.zip",
        ("windows", "aarch64") => "yt-dlp_win_arm64.zip",
        ("macos", _) => "yt-dlp_macos.zip",
        ("linux", "x86_64") => "yt-dlp_linux.zip",
        ("linux", "aarch64") => "yt-dlp_linux_aarch64.zip",
        ("linux", "arm") => "yt-dlp_linux_armv7l.zip",
        _ => return None,
    })
}

/// Unpacks a release ZIP into `dest` (created fresh) and returns the
/// executable inside it.
pub fn unpack(zip_path: &Path, dest: &Path) -> Result<PathBuf> {
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    std::fs::create_dir_all(dest)?;
    // `extract` refuses entries that would leave `dest` and restores the
    // unix permissions stored in the archive.
    zip::ZipArchive::new(std::fs::File::open(zip_path)?)?.extract(dest)?;
    let exe = exe_in(dest).ok_or_else(|| anyhow!("the archive contains no yt-dlp executable"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(exe)
}

/// The hash of `asset` in a `SHA2-256SUMS` file (`<hex>  <name>` lines).
pub fn parse_sums(text: &str, asset: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        let (hash, name) = (parts.next()?, parts.next()?);
        let name = name.trim_start_matches('*');
        (name == asset && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

/// Downloads the latest official yt-dlp release archive for this OS,
/// verifies it against the release's `SHA2-256SUMS` and unpacks it into
/// [`managed_dir`].
/// Returns the installed version.
pub async fn install(http: &reqwest::Client) -> Result<String> {
    let asset = release_asset().ok_or_else(|| anyhow!("no official yt-dlp build for this system"))?;
    let long = Duration::from_secs(10 * 60);
    let sums = http
        .get(format!("{RELEASE_BASE}/SHA2-256SUMS"))
        .timeout(Duration::from_secs(60))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let expected = parse_sums(&sums, asset).ok_or_else(|| anyhow!("{asset} is missing from SHA2-256SUMS"))?;

    let dest = managed_dir();
    let dir = dest.parent().expect("managed dir has a parent").to_path_buf();
    std::fs::create_dir_all(&dir)?;
    let part = dir.join("yt-dlp.zip.part");
    let resp = http.get(format!("{RELEASE_BASE}/{asset}")).timeout(long).send().await?.error_for_status()?;
    if resp.content_length().is_some_and(|n| n > MAX_BINARY_BYTES) {
        bail!("yt-dlp download is unexpectedly large");
    }
    let mut hasher = Sha256::new();
    let mut file = std::fs::File::create(&part)?;
    let mut bytes = 0u64;
    let mut body = resp.bytes_stream();
    let res: Result<()> = async {
        use std::io::Write;
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            bytes += chunk.len() as u64;
            if bytes > MAX_BINARY_BYTES {
                bail!("yt-dlp download is unexpectedly large");
            }
            hasher.update(&chunk);
            file.write_all(&chunk)?;
        }
        file.flush()?;
        Ok(())
    }
    .await;
    drop(file);
    let got: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    if got != expected {
        let _ = std::fs::remove_file(&part);
        bail!("yt-dlp checksum mismatch (expected {expected}, got {got})");
    }
    // Unpack next to the live copy, then swap folders.
    let fresh = dir.join("yt-dlp.new");
    let unpacked = {
        let (part, fresh) = (part.clone(), fresh.clone());
        tokio::task::spawn_blocking(move || unpack(&part, &fresh)).await.map_err(|e| anyhow!("unpack failed: {e}"))?
    };
    let _ = std::fs::remove_file(&part);
    unpacked?;
    let old = dir.join("yt-dlp.old");
    let _ = std::fs::remove_dir_all(&old);
    // Single-file copies of earlier versions ("bin/yt-dlp(.exe)").
    if dest.is_file() {
        std::fs::remove_file(&dest)?;
    }
    let _ = std::fs::remove_file(dir.join(exe_name("yt-dlp")));
    if dest.exists() {
        std::fs::rename(&dest, &old).context("yt-dlp is in use; try again when nothing is playing")?;
    }
    std::fs::rename(&fresh, &dest)?;
    let _ = std::fs::remove_dir_all(&old);
    forget_urls();
    let exe = managed_path().ok_or_else(|| anyhow!("installed yt-dlp is missing"))?;
    version(&exe).await.ok_or_else(|| anyhow!("installed yt-dlp does not start"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESOLVE_FIXTURE: &str = r#"{
        "id": "dQw4w9WgXcQ", "title": "Rick Astley - Never Gonna Give You Up (Official Video)",
        "channel": "Rick Astley", "uploader": "RickAstleyVEVO", "duration": 213.0, "live_status": "not_live",
        "format_id": "140", "ext": "m4a", "acodec": "mp4a.40.2",
        "url": "https://rr3---sn-x.googlevideo.com/videoplayback?expire=1760700000&ei=abc&itag=140&mime=audio%2Fmp4",
        "formats": [{"format_id": "251", "ext": "webm", "acodec": "opus", "url": "https://rr3---sn-x.googlevideo.com/videoplayback?itag=251"}]
    }"#;

    const SEARCH_FIXTURE: &str = r#"{
        "_type": "playlist", "id": "believer", "title": "believer",
        "entries": [
            {"_type": "url", "ie_key": "Youtube", "id": "7wtfhZwyrcc", "url": "https://www.youtube.com/watch?v=7wtfhZwyrcc",
             "title": "Imagine Dragons - Believer (Official Music Video)", "channel": "ImagineDragonsVEVO", "duration": 217.0, "live_status": null},
            {"_type": "url", "ie_key": "Youtube", "id": "IhP3J0j9JmY", "title": "Believer", "channel": "Imagine Dragons - Topic", "duration": 204},
            {"_type": "url", "ie_key": "YoutubeTab", "id": "UCT9zcQNlyht7fRlcjmflRSA", "title": "Imagine Dragons"},
            {"_type": "url", "ie_key": "Youtube", "id": "liveliveliv", "title": "24/7 radio", "channel": "x", "live_status": "is_live"},
            {"_type": "url", "ie_key": "Youtube", "id": "7wtfhZwyrcc", "title": "duplicate", "channel": "x"},
            {"broken": true}
        ]
    }"#;

    #[test]
    fn video_ids_are_strict() {
        assert!(valid_video_id("dQw4w9WgXcQ"));
        assert!(valid_video_id("a-b_c-d_e-f"));
        for bad in ["", "dQw4w9WgXc", "dQw4w9WgXcQQ", "--exec=cal", "dQw4w9WgX/Q", "https://x.y", "dQw4w9WgXc ", "dQw4w9WgXcä"] {
            assert!(!valid_video_id(bad), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn resolve_args_are_fixed_and_end_with_the_url() {
        let a = resolve_args("dQw4w9WgXcQ", None).unwrap();
        assert_eq!(a.last().unwrap(), "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(a[a.len() - 2], "--");
        for must in ["--ignore-config", "--no-cookies", "--no-playlist", "-J"] {
            assert!(a.iter().any(|x| x == must), "missing {must}");
        }
        assert!(!a.iter().any(|x| x.contains("cookies-from-browser") || x == "--username" || x == "--js-runtimes"));
        let f = a.iter().position(|x| x == "-f").unwrap();
        assert!(a[f + 1].starts_with("bestaudio[ext=m4a]"));
        assert!(resolve_args("--exec=rm -rf", None).is_err());
        assert!(resolve_args("https://evil.example/x", None).is_err());

        let node = JsRuntime { name: "node", path: "node".into() };
        let a = resolve_args("dQw4w9WgXcQ", Some(&node)).unwrap();
        let i = a.iter().position(|x| x == "--js-runtimes").unwrap();
        assert_eq!(a[i + 1], "node");
        let deno = JsRuntime { name: "deno", path: "deno".into() };
        assert!(!resolve_args("dQw4w9WgXcQ", Some(&deno)).unwrap().iter().any(|x| x == "--js-runtimes"));
    }

    #[test]
    fn search_args_keep_the_query_one_argument() {
        let a = search_args("  --exec evil\n\tsong  ", 10, None).unwrap();
        assert_eq!(a.last().unwrap(), "ytsearch10:--exec evil song");
        assert_eq!(a[a.len() - 2], "--");
        assert!(a.iter().any(|x| x == "--flat-playlist") && a.iter().any(|x| x == "--dump-single-json"));
        assert!(search_args("x", 999, None).unwrap().last().unwrap().starts_with("ytsearch25:"));
        assert!(search_args(" \n ", 5, None).is_err());
        let long = "a".repeat(500);
        assert_eq!(search_args(&long, 5, None).unwrap().last().unwrap().len(), "ytsearch5:".len() + 200);
    }

    #[test]
    fn parses_resolve_output() {
        let a = parse_resolve(RESOLVE_FIXTURE.as_bytes(), 1_760_000_000).unwrap();
        assert_eq!(a.video_id, "dQw4w9WgXcQ");
        assert!(a.url.contains("itag=140"));
        assert_eq!(a.mime.as_deref(), Some("audio/mp4"));
        assert_eq!(a.duration_ms, Some(213_000));
        assert_eq!(a.channel, "Rick Astley");
        assert_eq!(a.expires, 1_760_700_000);

        // Format only in the list, no expire param.
        let j = r#"{"id":"dQw4w9WgXcQ","title":"t","format_id":"140","formats":[{"format_id":"140","ext":"m4a","acodec":"mp4a.40.2","url":"https://g/v?itag=140"}]}"#;
        let a = parse_resolve(j.as_bytes(), 100).unwrap();
        assert_eq!(a.url, "https://g/v?itag=140");
        assert_eq!(a.expires, 100 + URL_DEFAULT_TTL_S);

        assert!(parse_resolve(b"not json", 0).is_err());
        assert!(parse_resolve(br#"{"id":"dQw4w9WgXcQ","title":"t","url":"http://plain"}"#, 0).is_err());
        assert!(parse_resolve(br#"{"id":"bad","url":"https://x"}"#, 0).is_err());
        assert!(parse_resolve(br#"{"id":"dQw4w9WgXcQ","url":"https://x","live_status":"is_live"}"#, 0).is_err());
    }

    #[test]
    fn parses_search_output() {
        let c = parse_search(SEARCH_FIXTURE.as_bytes()).unwrap();
        let ids: Vec<&str> = c.iter().map(|c| c.track.id.as_str()).collect();
        assert_eq!(ids, ["7wtfhZwyrcc", "IhP3J0j9JmY"]);
        assert_eq!(c[0].track.artist, "Imagine Dragons");
        assert_eq!(c[0].track.title, "Believer");
        assert_eq!(c[0].track.duration_ms, Some(217_000));
        assert_eq!(c[0].track.source, glyph_protocol::ProviderId::YouTube);
        // Topic channel: clean metadata and a bonus
        assert_eq!(c[1].track.artist, "Imagine Dragons");
        assert!(c[1].bonus > c[0].bonus);
        assert!(parse_search(br#"{"entries":[]}"#).unwrap().is_empty());
        assert!(parse_search(b"ERROR: no json").is_err());
    }

    #[test]
    fn errors_are_classified() {
        let e = classify("[youtube] x: Downloading webpage\nERROR: [youtube] dQw4w9WgXcQ: Video unavailable\n");
        assert!(matches!(e.downcast_ref::<ProviderError>(), Some(ProviderError::NotFound)));
        let e = classify("ERROR: [youtube] x: Sign in to confirm you're not a bot.");
        assert!(crate::provider::should_cool_down(&e));
        assert!(e.to_string().contains("Sign in to confirm"));
        assert_eq!(error_line(""), "yt-dlp failed");
    }

    #[test]
    fn sums_and_assets() {
        let h = "a".repeat(64);
        let text = format!("{}  yt-dlp\n{h}  yt-dlp.exe\n{}  yt-dlp_macos\n", "b".repeat(64), "c".repeat(64));
        assert_eq!(parse_sums(&text, "yt-dlp.exe"), Some(h));
        assert_eq!(parse_sums(&text, "yt-dlp_linux"), None);
        assert_eq!(parse_sums("short  yt-dlp.exe", "yt-dlp.exe"), None);
        #[cfg(all(windows, target_arch = "x86_64"))]
        assert_eq!(release_asset(), Some("yt-dlp_win.zip"));
        #[cfg(target_os = "macos")]
        assert_eq!(release_asset(), Some("yt-dlp_macos.zip"));
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(release_asset(), Some("yt-dlp_linux.zip"));
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            if name.ends_with('/') {
                w.add_directory(*name, opts).unwrap();
            } else {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap();
    }

    #[test]
    fn unpacks_onedir_release() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("r.zip");
        let exe = exe_name(if cfg!(windows) { "yt-dlp" } else { "yt-dlp_linux" });
        write_zip(&zip_path, &[("_internal/", b""), ("_internal/base_library.zip", b"x"), (exe.as_str(), b"bin")]);
        let dest = dir.path().join("out");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("stale"), b"old").unwrap();
        let got = unpack(&zip_path, &dest).unwrap();
        assert_eq!(got, dest.join(&exe));
        assert!(dest.join("_internal").join("base_library.zip").is_file());
        assert!(!dest.join("stale").exists(), "old files are removed");

        // No executable inside: refused.
        write_zip(&zip_path, &[("_internal/x", b"x")]);
        assert!(unpack(&zip_path, &dir.path().join("out2")).is_err());
        // Entries escaping the folder never land outside it.
        write_zip(&zip_path, &[("../escaped", b"x"), (exe.as_str(), b"bin")]);
        let _ = unpack(&zip_path, &dir.path().join("out3"));
        assert!(!dir.path().join("escaped").exists());
    }

    #[test]
    fn batch_args_and_output() {
        let a = batch_args(&["dQw4w9WgXcQ", "7wtfhZwyrcc"], None).unwrap();
        assert!(a.iter().any(|x| x == "-j") && a.iter().any(|x| x == "--ignore-errors"));
        let dash = a.iter().position(|x| x == "--").unwrap();
        assert_eq!(&a[dash + 1..], ["https://www.youtube.com/watch?v=dQw4w9WgXcQ", "https://www.youtube.com/watch?v=7wtfhZwyrcc"]);
        assert!(batch_args(&["dQw4w9WgXcQ", "--exec=x"], None).is_err());
        assert!(batch_args(&[], None).is_err());

        let one_line = RESOLVE_FIXTURE.replace('\n', " ");
        let second = one_line.replace("dQw4w9WgXcQ", "7wtfhZwyrcc");
        let out = format!("{one_line}\nnot json\n\n{second}\n");
        let got = parse_batch(out.as_bytes(), 0);
        assert_eq!(got.iter().map(|a| a.video_id.as_str()).collect::<Vec<_>>(), ["dQw4w9WgXcQ", "7wtfhZwyrcc"]);
        assert!(parse_batch(b"", 0).is_empty());
    }

    #[test]
    fn url_cache_respects_expiry() {
        let mut a = parse_resolve(RESOLVE_FIXTURE.as_bytes(), 0).unwrap();
        a.video_id = "cacheTest01".into();
        a.expires = 10_000;
        store(&a);
        assert!(cached("cacheTest01", 10_000 - URL_MARGIN_S - 1).is_some());
        assert!(cached("cacheTest01", 10_000 - URL_MARGIN_S).is_none());
        assert_eq!(url_expiry("https://x/v?a=1&expire=42&b=2"), Some(42));
        assert_eq!(url_expiry("https://x/v"), None);
    }

    /// Network + real binary: `GLYPH_DATA_DIR=<scratch> cargo test -p glyph-core live_ -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_install_resolve_and_fetch() {
        let http = reqwest::Client::builder().user_agent("glyph-test").build().unwrap();
        if locate(None).is_none() {
            println!("installed {}", install(&http).await.unwrap());
        }
        let st = status(None).await;
        println!("{st:?}");
        let yt = YtDlp::locate(None).unwrap();
        let t = std::time::Instant::now();
        let _ = version(&yt.bin).await;
        println!("start-up: {:?}", t.elapsed());
        let hits = yt.search("Imagine Dragons - Believer", 5).await.unwrap();
        println!("search: {:?}", hits.iter().map(|c| (&c.track.id, &c.track.artist, &c.track.title)).collect::<Vec<_>>());
        assert!(!hits.is_empty());
        let a = yt.resolve(&hits[0].track.id).await.unwrap();
        println!("mime {:?} dur {:?} expires {}", a.mime, a.duration_ms, a.expires);
        let r = http.get(&a.url).header("Range", "bytes=0-65535").send().await.unwrap();
        println!("fetch: {} {:?}", r.status(), r.headers().get("content-type"));
        assert_eq!(r.status().as_u16(), 206);
        // Batch: the other hits in one run.
        forget_urls();
        let ids: Vec<String> = hits.iter().skip(1).take(3).map(|c| c.track.id.clone()).collect();
        let t = std::time::Instant::now();
        let n = yt.resolve_many(&ids).await.unwrap();
        println!("batch: {n}/{} in {:?}", ids.len(), t.elapsed());
        assert!(n >= 2);
        // Whole song into the audio cache (what playback uses).
        let t = std::time::Instant::now();
        let file = yt.audio_file(&http, &ids[0]).await.unwrap();
        let len = std::fs::metadata(&file).unwrap().len();
        println!("audio file: {} ({len} bytes) in {:?}", file.display(), t.elapsed());
        assert!(len > 500_000);
        let t = std::time::Instant::now();
        assert_eq!(yt.audio_file(&http, &ids[0]).await.unwrap(), file);
        println!("cached replay: {:?}", t.elapsed());
    }

    #[test]
    fn audio_cache_and_headers() {
        let a = parse_resolve(RESOLVE_FIXTURE.as_bytes(), 0).unwrap();
        assert!(a.headers.is_empty());
        let j = r#"{"id":"dQw4w9WgXcQ","title":"t","url":"https://g/v","ext":"m4a","http_headers":{"User-Agent":"UA","Cookie":"x","Accept":"*/*"}}"#;
        let a = parse_resolve(j.as_bytes(), 0).unwrap();
        let mut h: Vec<&str> = a.headers.iter().map(|(k, _)| k.as_str()).collect();
        h.sort();
        assert_eq!(h, ["Accept", "User-Agent"], "cookies are never forwarded");

        assert_eq!(ext_for(Some("audio/mp4")), "m4a");
        assert_eq!(ext_for(Some("audio/mpeg")), "mp3");
        assert_eq!(mime_for_path(Path::new("x/abc.m4a")), "audio/mp4");

        let dir = tempfile::tempdir().unwrap();
        for (i, name) in ["old.m4a", "mid.m4a", "new.m4a"].iter().enumerate() {
            let p = dir.path().join(name);
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let t = std::time::SystemTime::now() - Duration::from_secs(300 - i as u64 * 100);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        prune_audio_cache(dir.path(), 200);
        assert!(!dir.path().join("old.m4a").exists());
        assert!(dir.path().join("mid.m4a").exists() && dir.path().join("new.m4a").exists());
        assert!(cached_audio("not-an-id").is_none());
    }

    #[test]
    fn forbidden_is_detected() {
        assert!(is_forbidden(&anyhow::Error::from(ProviderError::Http(403))));
        assert!(!is_forbidden(&anyhow::Error::from(ProviderError::Http(500))));
        assert!(!is_forbidden(&anyhow!("other")));
    }

    #[test]
    fn locate_prefers_configured_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join(exe_name("my-ytdlp"));
        std::fs::write(&f, b"").unwrap();
        assert_eq!(locate(Some(f.to_str().unwrap())), Some(f));
    }
}

#[cfg(test)]
mod update_version_tests {
    use super::*;
    #[test] fn release_dates_are_compared_numerically() {
        assert!(release_version("2026.9.1")<release_version("2026.10.1"));
        assert_eq!(release_version("v2026.09.01"),release_version("2026.9.1"));
        assert!(release_version("custom build").is_none());
    }
}
