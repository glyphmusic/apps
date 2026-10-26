//! Offline downloads.
//!
//! Only songs whose source allows downloading are saved: Jamendo tracks with
//! `audiodownload_allowed`, Audius tracks the artist marked downloadable, and
//! Creative-Commons / public-domain items on the Internet Archive, and YouTube
//! audio through the configured local extractor or server/companion streaming path (see
//! [`crate::ytdlp`]). Everything else (Spotify, Deezer previews, SoundCloud
//! embeds) is never downloaded.
//!
//! Files go to `storage::downloads_dir()`, with an `index.json` next to them
//! mapping songs (normalised title + artist) to their file.
//!
//! A download runs as a [`Job`] (task-local): the code that fetches bytes
//! reports into its [`Progress`], and providers read its quality, without
//! threading both through the provider trait.

use crate::config::AudioQuality;
use crate::dedupe::track_key;
use crate::paths;
use anyhow::{bail, Result};
use futures::StreamExt;
use glyph_protocol::{ProviderId, Track};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Shared admission for all download batches. Existing transfers finish when
/// the limit falls; only admission of new transfers is delayed.
#[derive(Default)]
pub struct DownloadQueue {
    active: std::sync::atomic::AtomicUsize,
    maximum: std::sync::atomic::AtomicUsize,
    adaptive: std::sync::atomic::AtomicBool,
    changed: tokio::sync::Notify,
}

pub fn parallelism(maximum: usize, adaptive: bool, class: glyph_protocol::network::Class) -> usize {
    let maximum = maximum.clamp(1, 10);
    if !adaptive { return maximum; }
    maximum.min(match class {
        glyph_protocol::network::Class::Slow => 1,
        glyph_protocol::network::Class::Standard => 3,
        glyph_protocol::network::Class::Fast => 10,
    })
}

impl DownloadQueue {
    pub fn configure(&self, maximum: usize, adaptive: bool) {
        self.maximum.store(maximum.clamp(1, 10), Ordering::SeqCst);
        self.adaptive.store(adaptive, Ordering::SeqCst);
        self.changed.notify_waiters();
    }

    pub async fn acquire(self: &Arc<Self>) -> DownloadPermit {
        loop {
            let changed = self.changed.notified();
            let limit = parallelism(self.maximum.load(Ordering::SeqCst), self.adaptive.load(Ordering::SeqCst), glyph_protocol::network::class());
            if self.active.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < limit).then_some(n + 1)).is_ok() {
                return DownloadPermit(self.clone());
            }
            // Network estimates can change without a download completing.
            tokio::select! { _ = changed => {}, _ = tokio::time::sleep(Duration::from_millis(250)) => {} }
        }
    }
}

pub struct DownloadPermit(Arc<DownloadQueue>);
impl Drop for DownloadPermit {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

/// Bytes fetched so far of one download.
#[derive(Debug, Default)]
pub struct Progress {
    done: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    pub fn set_total(&self, bytes: u64) {
        self.total.store(bytes, Ordering::Relaxed);
    }

    pub fn add(&self, bytes: u64) {
        self.done.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Starts over (another file is fetched instead).
    pub fn reset(&self) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
    }

    /// Marks the download complete (a cached file needs no fetching).
    pub fn finish(&self) {
        let total = self.total.load(Ordering::Relaxed).max(1);
        self.total.store(total, Ordering::Relaxed);
        self.done.store(total, Ordering::Relaxed);
    }

    /// 0..=1, `None` while the size is unknown.
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| (self.done.load(Ordering::Relaxed) as f64 / total as f64).min(1.0) as f32)
    }
}

/// The download running in this task.
#[derive(Debug, Clone)]
pub struct Job {
    pub progress: Arc<Progress>,
    pub quality: AudioQuality,
}

tokio::task_local! {
    static JOB: Job;
}

/// The current download job, if this task runs one.
pub fn job() -> Option<Job> {
    JOB.try_with(|j| j.clone()).ok()
}

/// Quality wanted by the current download (`Standard` outside a download).
pub fn job_quality() -> AudioQuality {
    job().map(|j| j.quality.effective()).unwrap_or(AudioQuality::Standard)
}

/// Reports fetched bytes to the current job, if any.
pub fn report(f: impl FnOnce(&Progress)) {
    if let Some(j) = job() {
        f(&j.progress);
    }
}

/// Runs `fut` as download `job`.
pub async fn run_job<F: Future>(job: Job, fut: F) -> F::Output {
    JOB.scope(job, fut).await
}

/// Jamendo download URLs name their format in the path
/// (`.../download/track/<id>/mp32/`); other qualities are the same URL with
/// another format.
pub fn jamendo_format_url(url: &str, q: AudioQuality) -> Option<String> {
    let fmt = match q.effective() {
        AudioQuality::Low => "mp31",
        AudioQuality::Auto | AudioQuality::Standard => return None,
        AudioQuality::High => "flac",
    };
    url.contains("/download/track/").then(|| url.replacen("/mp32/", &format!("/{fmt}/"), 1)).filter(|u| u != url)
}

/// `true` for failures worth another try: timeouts, dropped connections and
/// gateway errors (a server that had to prepare the file for too long).
fn transient(e: &anyhow::Error) -> bool {
    if e.downcast_ref::<tokio::time::error::Elapsed>().is_some() { return true; }
    e.chain().any(|c| {
        if let Some(r) = c.downcast_ref::<reqwest::Error>() {
            return r.is_timeout() || r.is_connect() || r.is_body() || r.is_request() || r.status().is_some_and(|s| s.is_server_error() || s.as_u16() == 429);
        }
        let m = c.to_string().to_lowercase();
        m.contains("timed out") || m.contains("connection") || m.contains("reset")
    })
}

/// [`Downloads::fetch`] with up to three tries. A server download that first
/// converts to a smaller quality can hit the gateway's time limit on long
/// songs; the second try then takes the original stream, which starts at once.
pub async fn fetch_with_retry(
    dl: &Downloads,
    http: &reqwest::Client,
    t: &Track,
    provider: ProviderId,
    url: &str,
    server: Option<&crate::server_client::ServerClient>,
) -> Result<PathBuf> {
    let mut url = url.to_string();
    let mut last = None;
    for attempt in 0..3u32 {
        if attempt > 0 {
            report(|p| p.reset());
            tokio::time::sleep(Duration::from_millis(1500 * u64::from(attempt))).await;
        }
        match dl.fetch(http, t, provider, &url, server).await {
            Ok(p) => return Ok(p),
            Err(e) if transient(&e) => {
                tracing::info!(attempt, "download failed, trying again: {e:#}");
                // Converted server streams: fall back to the original quality.
                if attempt == 1 {
                    if let Ok(mut parsed) = reqwest::Url::parse(&url) {
                        if parsed.path().starts_with("/v1/download/") && parsed.query_pairs().any(|(k, v)| k == "quality" && v != "high") {
                            let mut pairs: Vec<(String, String)> = parsed.query_pairs().filter(|(k, _)| k != "quality").map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
                            pairs.push(("quality".into(), "high".into()));
                            parsed.query_pairs_mut().clear().extend_pairs(pairs);
                            url = parsed.into();
                        }
                    }
                }
                last = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("download failed")))
}

/// Larger files are refused (an album rip in one file, a broken server).
const MAX_BYTES: u64 = 500 * 1024 * 1024;
const INDEX: &str = "index.json";

/// Owns a reservation until publication, including when an async job is cancelled.
struct PendingFile {
    file: Option<std::io::BufWriter<std::fs::File>>,
    path: PathBuf,
}

impl PendingFile {
    fn new(file: std::fs::File, path: PathBuf) -> Self {
        Self { file: Some(std::io::BufWriter::with_capacity(64 * 1024, file)), path }
    }

    fn commit(mut self, destination: &Path) -> Result<()> {
        self.file.as_mut().unwrap().flush()?;
        drop(self.file.take());
        std::fs::rename(&self.path, destination)?;
        Ok(())
    }
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        // Windows requires closing the file before removing it.
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Revision {
    pub etag: Option<String>,
    pub modified: Option<String>,
    pub sha256: Option<String>,
    pub checked_at: u64,
    pub hashed_at: u64,
    pub quality: AudioQuality,
}

fn digest_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut hash = Sha256::new();
    let mut file = std::fs::File::open(path)?;
    let mut block = [0u8; 64 * 1024];
    loop { let n = file.read(&mut block)?; if n == 0 { break; } hash.update(&block[..n]); }
    Ok(format!("{:x}", hash.finalize()))
}

/// One downloaded song.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub track: Track,
    /// File name inside the downloads folder.
    pub file: String,
    pub provider: ProviderId,
    pub bytes: u64,
    /// Unix seconds.
    pub ts: u64,
    #[serde(default)]
    pub revision: Revision,
}

/// The downloads folder and its index.
pub struct Downloads {
    pub(crate) relocation: tokio::sync::RwLock<()>,
    /// Changes when the user picks another folder ([`Downloads::move_to`]).
    dir: std::sync::RwLock<PathBuf>,
    map: Mutex<HashMap<String, Entry>>,
    index_io: Mutex<()>,
    tracks_snapshot: Mutex<Option<Arc<Vec<Track>>>>,
}

impl Downloads {
    /// Called by maintenance, never by playback. Claiming also backs off errors.
    /// YouTube songs are never checked: their lookup alone fetches the whole
    /// audio again (r-dlp / yt-dlp, the server's conversion), and the audio
    /// of a video does not change.
    pub fn claim_refresh(&self, limit: usize) -> Vec<Entry> {
        let mut map = self.map.lock().unwrap();
        let mut due: Vec<_> = map.values_mut()
            .filter(|e| e.provider != ProviderId::YouTube && now().saturating_sub(e.revision.checked_at.max(e.ts)) >= 86400)
            .collect();
        due.sort_by_key(|e| e.revision.checked_at.max(e.ts));
        let entries: Vec<Entry> = due.into_iter().take(limit).map(|e| { e.revision.checked_at = now(); e.clone() }).collect();
        drop(map);
        if !entries.is_empty() { self.save(); }
        entries
    }

    pub async fn refresh(self: &std::sync::Arc<Self>, http: &reqwest::Client, old: Entry, url: &str, server: Option<&crate::server_client::ServerClient>) -> Result<()> {
        let request = http.head(url);
        let mut request = server.map(|s| s.authorize_url(http.head(url), url)).unwrap_or(request);
        if let Some(etag) = &old.revision.etag { request = request.header("if-none-match", etag); }
        else if let Some(modified) = &old.revision.modified { request = request.header("if-modified-since", modified); }
        let response = request.timeout(Duration::from_secs(15)).send().await?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED { return Ok(()); }
        let supported = response.status().is_success();
        if !supported && !matches!(response.status().as_u16(), 405 | 501) { response.error_for_status()?; return Ok(()); }
        let etag = response.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
        let modified = response.headers().get("last-modified").and_then(|v| v.to_str().ok()).map(str::to_string);
        let size = response.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
        let changed = supported && (size.is_some_and(|n| n != old.bytes)
            || old.revision.etag.as_ref().zip(etag.as_ref()).is_some_and(|(a,b)| a != b)
            || old.revision.modified.as_ref().zip(modified.as_ref()).is_some_and(|(a,b)| a != b));
        let matched = supported && (old.revision.etag.as_ref().zip(etag.as_ref()).is_some_and(|(a,b)| a == b)
            || old.revision.modified.as_ref().zip(modified.as_ref()).is_some_and(|(a,b)| a == b));
        let new_validator = old.revision.etag.is_none() && etag.is_some() || old.revision.modified.is_none() && modified.is_some();
        // Without a validator there is nothing cheap to compare: an unchanged
        // size counts as unchanged instead of fetching the whole file again.
        if !changed && (matched || !new_validator) { return Ok(()); }
        drop(response);
        // A disposable download cannot replace the index or the playing file.
        let temp = tempfile::tempdir_in(self.path())?;
        let staged = Downloads::load(temp.path().to_path_buf());
        let path = staged.fetch(http, &old.track, old.provider, url, server).await?;
        let fresh = staged.map.lock().unwrap().get(&track_key(&old.track)).cloned().unwrap();
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let _temp = temp;
            this.install_refresh(&old, fresh, &path)
        }).await??;
        Ok(())
    }

    fn install_refresh(&self, old: &Entry, mut fresh: Entry, source: &Path) -> Result<()> {
        let old_path = self.path().join(&old.file);
        let old_hash = match &old.revision.sha256 { Some(hash) => hash.clone(), None => digest_file(&old_path)? };
        let changed = fresh.revision.sha256.as_ref() != Some(&old_hash);
        let replacement = if changed {
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
            let name = format!("{}-{nonce}.{}", paths::hash_hex(&track_key(&old.track)), extension("", &source.to_string_lossy()));
            let path = self.path().join(&name);
            std::fs::copy(source, &path)?;
            fresh.file = name;
            Some(path)
        } else { fresh.file = old.file.clone(); fresh.ts = old.ts; None };
        let _index_io = self.index_io.lock().unwrap();
        let mut map = self.map.lock().unwrap();
        if !map.get(&track_key(&old.track)).is_some_and(|e| e.file == old.file) {
            if let Some(path) = replacement { let _ = std::fs::remove_file(path); }
            return Ok(()); // Removed or manually replaced during the check.
        }
        let mut snapshot = map.clone();
        snapshot.insert(track_key(&old.track), fresh);
        if let Err(e) = paths::write_json(&self.path().join(INDEX), &snapshot) {
            if let Some(path) = replacement { let _ = std::fs::remove_file(path); }
            return Err(e);
        }
        *map = snapshot;
        drop(map);
        // On Windows an active decoder may still hold this file. Keep it if
        // deletion is refused; the new index already points at the new version.
        if changed { let _ = std::fs::remove_file(old_path); }
        Ok(())

    }
    pub fn load(dir: PathBuf) -> Self {
        let d = Self { relocation: Default::default(), dir: std::sync::RwLock::new(dir), map: Mutex::new(HashMap::new()), index_io: Mutex::new(()), tracks_snapshot: Mutex::new(None) };
        d.reload();
        d
    }

    /// Re-reads the index (after the folder was cleared in settings). Entries
    /// whose file is gone are dropped.
    pub fn reload(&self) {
        let dir = self.path();
        let mut map: HashMap<String, Entry> = paths::read_json(&dir.join(INDEX)).unwrap_or_default();
        map.retain(|_, e| dir.join(&e.file).is_file());
        *self.map.lock().unwrap() = map;
        *self.tracks_snapshot.lock().unwrap() = None;
    }

    /// Moves every downloaded file and the index into `new_dir` and saves
    /// there from now on. Files already in `new_dir` are never overwritten
    /// (a moved file gets a free name); songs `new_dir` already has stay as
    /// they are there. On failure the originals and their index stay intact.
    /// Blocking: run off the UI thread while holding `relocation` exclusively.
    pub fn move_to(&self, new_dir: PathBuf) -> Result<MoveReport> {
        let _index_io = self.index_io.lock().unwrap();
        let old_dir = self.path();
        let mut report = MoveReport::default();
        if same_dir(&old_dir, &new_dir) {
            return Ok(report);
        }
        std::fs::create_dir_all(&new_dir)?;
        let mut map = self.map.lock().unwrap();
        let index = new_dir.join(INDEX);
        let mut merged: HashMap<String, Entry> = match std::fs::read(&index) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e.into()),
        };
        merged.retain(|_, e| new_dir.join(&e.file).is_file());
        let mut copied = Vec::new();
        for (key, mut e) in map.clone() {
            if merged.contains_key(&key) {
                continue;
            }
            let from = old_dir.join(&e.file);
            let (stem, ext) = match e.file.rsplit_once('.') {
                Some((a, b)) => (a.to_string(), format!(".{b}")),
                None => (e.file.clone(), String::new()),
            };
            let mut name = e.file.clone();
            let mut i = 2;
            while new_dir.join(&name).exists() {
                name = format!("{stem} ({i}){ext}");
                i += 1;
            }
            let to = new_dir.join(&name);
            // Keep the originals until the complete destination index is durable.
            // create_new also protects a file created after the name check.
            let moved = (|| -> std::io::Result<()> {
                let mut dest = std::fs::OpenOptions::new().write(true).create_new(true).open(&to)?;
                copied.push((from.clone(), to.clone()));
                let mut source = std::fs::File::open(&from)?;
                std::io::copy(&mut source, &mut dest)?;
                dest.sync_all()
            })();
            if moved.is_ok() {
                e.file = name;
                merged.insert(key, e);
                report.moved += 1;
            } else {
                for (_, path) in &copied { let _ = std::fs::remove_file(path); }
                return Err(moved.unwrap_err().into());
            }
        }
        if let Err(e) = paths::write_json(&new_dir.join(INDEX), &merged) {
            for (_, path) in &copied { let _ = std::fs::remove_file(path); }
            return Err(e);
        }
        *map = merged;
        drop(map);
        *self.dir.write().unwrap() = new_dir;
        *self.tracks_snapshot.lock().unwrap() = None;
        for (from, _) in copied { let _ = std::fs::remove_file(from); }
        // The old index points at files that are gone now.
        let _ = std::fs::remove_file(old_dir.join(INDEX));
        Ok(report)
    }

    fn save(&self) {
        *self.tracks_snapshot.lock().unwrap() = None;
        let _index_io = self.index_io.lock().unwrap();
        let snapshot = self.map.lock().unwrap().clone();
        let _ = std::fs::create_dir_all(self.path());
        if let Err(e) = paths::write_json(&self.path().join(INDEX), &snapshot) {
            tracing::warn!("could not save download index: {e}");
        }
    }

    pub fn dir(&self) -> PathBuf {
        self.path()
    }

    fn path(&self) -> PathBuf {
        self.dir.read().unwrap().clone()
    }

    pub fn saved_bytes(&self, tracks: &[Track]) -> u64 {
        let map = self.map.lock().unwrap();
        let keys: std::collections::HashSet<_> = tracks.iter().map(track_key).collect();
        keys.iter().filter_map(|k| map.get(k)).map(|e| e.bytes).sum()
    }

    /// Cheap check for the UI (index only, no disk access).
    pub fn contains(&self, t: &Track) -> bool {
        self.map.lock().unwrap().contains_key(&track_key(t))
    }

    /// The local file of `t` (checked on disk) and the source it came from.
    pub fn local(&self, t: &Track) -> Option<(PathBuf, ProviderId)> {
        let e = self.map.lock().unwrap().get(&track_key(t)).cloned()?;
        let p = self.path().join(&e.file);
        p.is_file().then_some((p, e.provider))
    }

    /// Downloaded songs, newest first.
    pub fn tracks(&self) -> Vec<Track> {
        (*self.tracks_snapshot()).clone()
    }

    /// Sorting and cloning are paid once per index change, not per UI frame.
    pub fn tracks_snapshot(&self) -> Arc<Vec<Track>> {
        let mut cached = self.tracks_snapshot.lock().unwrap();
        if let Some(tracks) = cached.as_ref() { return tracks.clone(); }
        let mut v: Vec<Entry> = self.map.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| b.ts.cmp(&a.ts).then_with(|| track_key(&a.track).cmp(&track_key(&b.track))));
        let tracks: Arc<Vec<Track>> = Arc::new(v.into_iter().map(|e| e.track).collect());
        *cached = Some(tracks.clone());
        tracks
    }

    /// Deletes the file of `t`. `true` if there was one.
    pub fn remove(&self, t: &Track) -> bool {
        let gone = self.map.lock().unwrap().remove(&track_key(t));
        match gone {
            Some(e) => {
                let _ = std::fs::remove_file(self.path().join(&e.file));
                self.save();
                true
            }
            None => false,
        }
    }

    /// Copies an already downloaded file (YouTube audio cache) into the
    /// folder and records it for `t`.
    pub fn store_copy(&self, t: &Track, provider: ProviderId, src: &Path) -> Result<PathBuf> {
        self.store_file(t, provider, src, extension("", &src.to_string_lossy()))
    }

    /// Copies `src` into the folder as `<name>.<ext>` and records it for `t`.
    pub fn store_file(&self, t: &Track, provider: ProviderId, src: &Path, ext: &str) -> Result<PathBuf> {
        let len = std::fs::metadata(src)?.len();
        if len == 0 || len > MAX_BYTES {
            bail!("unusable source file");
        }
        std::fs::create_dir_all(self.path())?;
        let (name, part, reservation) = self.reserve_name(t, ext)?;
        drop(reservation);
        if let Err(e) = std::fs::copy(src, &part) {
            let _ = std::fs::remove_file(&part);
            return Err(e.into());
        }
        let path = self.path().join(&name);
        std::fs::rename(&part, &path)?;
        let entry = Entry { track: t.clone(), file: name, provider, bytes: len, ts: now(),
            revision: Revision { sha256: Some(digest_file(&path)?), checked_at: now(), hashed_at: now(), quality: job_quality(), ..Default::default() } };
        self.map.lock().unwrap().insert(track_key(t), entry);
        self.save();
        Ok(path)
    }

    /// Fetches `url` into the folder and records it for `t`.
    pub async fn fetch(&self, http: &reqwest::Client, t: &Track, provider: ProviderId, url: &str, server: Option<&crate::server_client::ServerClient>) -> Result<PathBuf> {
        if !url.starts_with("https://") {
            bail!("refusing non-HTTPS download URL");
        }
        std::fs::create_dir_all(self.path())?;
        let request = server.map(|s| s.authorize_url(http.get(url),url)).unwrap_or_else(|| http.get(url));
        let resp = request.timeout(Duration::from_secs(15 * 60)).send().await?.error_for_status()?;
        self.save_response(resp, t, provider).await
    }

    async fn save_response(&self, resp: reqwest::Response, t: &Track, provider: ProviderId) -> Result<PathBuf> {
        use sha2::{Digest, Sha256};
        if resp.status() != reqwest::StatusCode::OK {
            bail!("expected a complete download, got HTTP {}", resp.status());
        }
        let etag = resp.headers().get("etag").and_then(|h| h.to_str().ok()).map(str::to_string);
        let modified = resp.headers().get("last-modified").and_then(|h| h.to_str().ok()).map(str::to_string);
        if resp.content_length().is_some_and(|n| n > MAX_BYTES) {
            bail!("file too large");
        }
        let expected = resp.content_length();
        if let Some(n) = expected {
            report(|p| p.set_total(n));
        }
        let quality = if resp.url().path().starts_with("/v1/download/") && resp.url().query_pairs().any(|(k, v)| k == "quality" && v == "high") {
            AudioQuality::High
        } else { job_quality() };
        let mime = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        if mime.starts_with("text/") || mime.contains("json") {
            bail!("server returned a document instead of audio");
        }
        let ext = extension(&mime, resp.url().path());
        let (name, part, file) = self.reserve_name(t, ext)?;
        let mut out = PendingFile::new(file, part);
        let mut digest = Sha256::new();
        let mut bytes = 0u64;
        let started = std::time::Instant::now();
        let transfer = glyph_protocol::network::Transfer::start();
        let mut body = resp.bytes_stream();
        while let Some(chunk) = tokio::time::timeout(Duration::from_secs(30), body.next()).await? {
            let chunk = chunk?;
            transfer.progress();
            bytes += chunk.len() as u64;
            if bytes > MAX_BYTES {
                bail!("file too large");
            }
            out.file.as_mut().unwrap().write_all(&chunk)?;
            digest.update(&chunk);
            report(|p| p.add(chunk.len() as u64));
        }
        if expected.is_some_and(|n| n != bytes) { bail!("download cut off"); }
        if bytes == 0 {
            bail!("empty file");
        }
        glyph_protocol::network::observe(bytes, started.elapsed());
        let path = self.path().join(&name);
        out.commit(&path)?;
        let entry = Entry { track: t.clone(), file: name, provider, bytes, ts: now(),
            revision: Revision { etag, modified, sha256: Some(format!("{:x}", digest.finalize())), checked_at: now(), hashed_at: now(), quality } };
        self.map.lock().unwrap().insert(track_key(t), entry);
        self.save();
        Ok(path)
    }

    fn reserve_name(&self, t: &Track, ext: &str) -> Result<(String, PathBuf, std::fs::File)> {
        let _lock = self.index_io.lock().unwrap();
        loop {
            let name = self.free_name(t, ext);
            let part = self.path().join(format!("{name}.part"));
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&part) {
                Ok(file) => return Ok((name, part, file)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// "Artist - Title.ext", numbered if taken by another song.
    fn free_name(&self, t: &Track, ext: &str) -> String {
        let base = sanitize(&format!("{} - {}", t.artist, t.title));
        let dir = self.path();
        let taken = |n: &str| dir.join(n).exists() || dir.join(format!("{n}.part")).exists();
        let mut name = format!("{base}.{ext}");
        let mut i = 2;
        while taken(&name) {
            name = format!("{base} ({i}).{ext}");
            i += 1;
        }
        name
    }
}

/// What [`Downloads::move_to`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MoveReport {
    pub moved: usize,
    pub failed: usize,
}

/// The same folder (also when spelled differently).
fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// File extension from the content type, else from the URL path.
fn extension(mime: &str, path: &str) -> &'static str {
    let m = mime.to_ascii_lowercase();
    let p = path.to_ascii_lowercase();
    if m.contains("ogg") || p.ends_with(".ogg") {
        "ogg"
    } else if m.contains("flac") || p.ends_with(".flac") {
        "flac"
    } else if m.contains("wav") || p.ends_with(".wav") {
        "wav"
    } else if m.contains("webm") || p.ends_with(".webm") {
        "webm"
    } else if m.contains("aac") || p.ends_with(".aac") {
        "aac"
    } else if m.contains("mp4") || p.ends_with(".m4a") {
        "m4a"
    } else {
        "mp3"
    }
}

/// MIME type for a downloaded file (playback hint).
pub fn mime_of(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "oga" => "audio/ogg",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "mka" => "audio/x-matroska",
        "webm" => "audio/webm",
        _ => "audio/mpeg",
    }
}

/// Safe file name on every platform.
fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    let short: String = trimmed.chars().take(120).collect();
    if short.is_empty() {
        "track".into()
    } else {
        short
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn download_parallelism_respects_config_and_network() {
        use glyph_protocol::network::Class::*;
        assert_eq!(super::parallelism(5, true, Slow), 1);
        assert_eq!(super::parallelism(5, true, Standard), 3);
        assert_eq!(super::parallelism(5, true, Fast), 5);
        assert_eq!(super::parallelism(5, false, Slow), 5);
        assert_eq!(super::parallelism(999, false, Fast), 10);
        assert_eq!(super::parallelism(0, false, Fast), 1);
    }

    #[tokio::test]
    async fn download_queue_is_shared_and_reconfiguration_does_not_cancel_transfers() {
        let queue = std::sync::Arc::new(super::DownloadQueue::default());
        queue.configure(2, false);
        let first = queue.acquire().await;
        let second = queue.acquire().await;
        let mut waiting = tokio::spawn({ let queue = queue.clone(); async move { queue.acquire().await } });
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut waiting).await.is_err());
        queue.configure(1, false);
        drop(first);
        assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut waiting).await.is_err());
        drop(second);
        let permit = tokio::time::timeout(std::time::Duration::from_secs(1), waiting).await.unwrap().unwrap();
        drop(permit);
    }

    #[test]
    fn download_snapshot_is_reused_and_invalidated_after_store_remove_and_move() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let downloads = super::Downloads::load(dir.path().to_path_buf());
        let empty = downloads.tracks_snapshot();
        assert!(Arc::ptr_eq(&empty, &downloads.tracks_snapshot()));
        let src = dir.path().join("src.mp3");
        std::fs::write(&src, b"abc").unwrap();
        let song = crate::dedupe::test_track(glyph_protocol::ProviderId::Archive, "1", "Song", "Band", Some(200));
        downloads.store_file(&song, glyph_protocol::ProviderId::Archive, &src, "mp3").unwrap();
        let saved = downloads.tracks_snapshot();
        assert_eq!(saved.len(), 1);
        assert!(Arc::ptr_eq(&saved, &downloads.tracks_snapshot()));
        downloads.move_to(dest.path().to_path_buf()).unwrap();
        assert_eq!(downloads.tracks_snapshot().len(), 1);
        assert!(downloads.local(&song).is_some());
        assert!(downloads.remove(&song));
        assert!(downloads.tracks_snapshot().is_empty());
        // A previously handed-out snapshot remains immutable.
        assert_eq!(saved.len(), 1);
    }

    #[test]
    fn failed_folder_move_keeps_original_index_and_files() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let downloads = super::Downloads::load(a.path().to_path_buf());
        let src = a.path().join("src.mp3");
        std::fs::write(&src, b"abc").unwrap();
        let song = crate::dedupe::test_track(glyph_protocol::ProviderId::Archive, "1", "Song", "Band", Some(200));
        let stored = downloads.store_file(&song, glyph_protocol::ProviderId::Archive, &src, "mp3").unwrap();
        std::fs::create_dir(b.path().join(super::INDEX)).unwrap();
        assert!(downloads.move_to(b.path().to_path_buf()).is_err());
        assert_eq!(downloads.dir(), a.path());
        assert_eq!(std::fs::read(stored).unwrap(), b"abc");
        assert!(super::Downloads::load(a.path().to_path_buf()).contains(&song));
    }

    #[test]
    fn moving_the_folder_keeps_songs_and_never_overwrites() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let downloads = super::Downloads::load(a.path().to_path_buf());
        let src = a.path().join("src.mp3");
        std::fs::write(&src, b"abc").unwrap();
        let song = crate::dedupe::test_track(glyph_protocol::ProviderId::Archive, "1", "Song", "Band", Some(200));
        let stored = downloads.store_file(&song, glyph_protocol::ProviderId::Archive, &src, "mp3").unwrap();
        let name = stored.file_name().unwrap().to_owned();
        // Someone else's file with the same name already lives in the new folder.
        std::fs::write(b.path().join(&name), b"mine").unwrap();
        let report = downloads.move_to(b.path().to_path_buf()).unwrap();
        assert_eq!(report, super::MoveReport { moved: 1, failed: 0 });
        assert_eq!(std::fs::read(b.path().join(&name)).unwrap(), b"mine");
        let (path, _) = downloads.local(&song).unwrap();
        assert!(path.starts_with(b.path()));
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
        // A fresh load of the new folder finds the song again.
        assert!(super::Downloads::load(b.path().to_path_buf()).contains(&song));
    }

    #[test]
    fn simultaneous_file_reservations_do_not_share_a_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let downloads = super::Downloads::load(dir.path().to_path_buf());
        let track = crate::dedupe::test_track(glyph_protocol::ProviderId::YouTube, "test", "Song", "Band", Some(200));
        let first = downloads.reserve_name(&track, "m4a").unwrap();
        let second = downloads.reserve_name(&track, "m4a").unwrap();
        assert_ne!(first.0, second.0);
        assert_ne!(first.1, second.1);
    }

    use super::*;

    fn track(title: &str) -> Track {
        Track {
            id: "1".into(),
            source: ProviderId::Archive,
            title: title.into(),
            artist: "Band".into(),
            album: None,
            duration_ms: None,
            cover_url: None,
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false,
        }
    }

    #[tokio::test]
    async fn job_reports_progress_and_quality() {
        assert!(job().is_none());
        assert_eq!(job_quality(), AudioQuality::Standard);
        let job = Job { progress: Arc::default(), quality: AudioQuality::Low };
        let p = job.progress.clone();
        assert_eq!(p.fraction(), None);
        run_job(job, async {
            assert_eq!(job_quality(), AudioQuality::Low);
            report(|p| p.set_total(200));
            report(|p| p.add(50));
        })
        .await;
        assert_eq!(p.fraction(), Some(0.25));
        p.finish();
        assert_eq!(p.fraction(), Some(1.0));
        p.reset();
        assert_eq!(p.fraction(), None);
    }

    #[test]
    fn jamendo_quality_urls() {
        let u = "https://prod-1.storage.jamendo.com/download/track/168/mp32/";
        assert_eq!(jamendo_format_url(u, AudioQuality::High).as_deref(), Some("https://prod-1.storage.jamendo.com/download/track/168/flac/"));
        assert_eq!(jamendo_format_url(u, AudioQuality::Low).as_deref(), Some("https://prod-1.storage.jamendo.com/download/track/168/mp31/"));
        assert_eq!(jamendo_format_url(u, AudioQuality::Standard), None);
        assert_eq!(jamendo_format_url("https://x.test/a.mp3", AudioQuality::High), None);
    }

    #[test]
    fn names_are_safe() {
        assert_eq!(sanitize("AC/DC - What? <Live>"), "AC_DC - What_ _Live_");
        assert_eq!(sanitize("  ..  "), "track");
        assert_eq!(extension("audio/ogg", "/x"), "ogg");
        assert_eq!(extension("", "/a/b.flac"), "flac");
        assert_eq!(extension("application/octet-stream", "/download"), "mp3");
        assert_eq!(extension("audio/webm", "/download"), "webm");
        assert_eq!(extension("audio/aac", "/download"), "aac");
    }

    #[tokio::test]
    async fn complete_downloads_are_hashed_and_broken_transfers_leave_no_file() {
        use std::io::{Read, Write};
        let dir = tempfile::tempdir().unwrap();
        let d = Downloads::load(dir.path().to_path_buf());
        for (i, response) in [
            "HTTP/1.1 200 OK\r\nContent-Type: audio/webm\r\nContent-Length: 4\r\nConnection: close\r\n\r\nsong",
            "HTTP/1.1 200 OK\r\nContent-Type: audio/mp4\r\nContent-Length: 20\r\nConnection: close\r\n\r\nshort",
            "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/mp4\r\nContent-Length: 4\r\nConnection: close\r\n\r\npart",
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 4\r\nConnection: close\r\n\r\noops",
        ].into_iter().enumerate() {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request);
                stream.write_all(response.as_bytes()).unwrap();
            });
            let response = reqwest::Client::new().get(format!("http://{address}")).send().await.unwrap();
            let song = track(&format!("Download {i}"));
            let result = d.save_response(response, &song, ProviderId::YouTube).await;
            server.join().unwrap();
            if i == 0 {
                let path = result.unwrap();
                assert_eq!(path.extension().unwrap(), "webm");
                assert_eq!(std::fs::read(&path).unwrap(), b"song");
                assert_eq!(d.map.lock().unwrap()[&track_key(&song)].revision.sha256.as_ref(), Some(&digest_file(&path).unwrap()));
            } else {
                assert!(result.is_err());
                assert!(!d.contains(&song));
            }
            assert!(!std::fs::read_dir(dir.path()).unwrap().any(|e| e.unwrap().path().extension().is_some_and(|x| x == "part")));
        }
    }

    #[test]
    fn cancelling_a_pending_file_removes_its_reservation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.part");
        let mut pending = PendingFile::new(std::fs::File::create(&path).unwrap(), path.clone());
        pending.file.as_mut().unwrap().write_all(b"unfinished").unwrap();
        drop(pending);
        assert!(!path.exists());
    }

    #[test]
    fn refresh_installs_complete_changes_and_keeps_identical_files() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mp3");
        std::fs::write(&source, b"old").unwrap();
        let d = Downloads::load(dir.path().join("downloads"));
        let t = track("Refresh");
        d.store_copy(&t, ProviderId::Archive, &source).unwrap();
        let old = d.map.lock().unwrap()[&track_key(&t)].clone();
        let held = std::fs::File::open(d.dir().join(&old.file)).unwrap();
        std::fs::write(&source, b"new complete audio").unwrap();
        let mut fresh = old.clone();
        fresh.bytes = 18;
        fresh.revision.sha256 = Some(digest_file(&source).unwrap());
        d.install_refresh(&old, fresh, &source).unwrap();
        let changed = d.map.lock().unwrap()[&track_key(&t)].clone();
        assert_ne!(changed.file, old.file);
        assert_eq!(std::fs::read(d.local(&t).unwrap().0).unwrap(), b"new complete audio");
        d.install_refresh(&changed, changed.clone(), &source).unwrap();
        assert_eq!(d.map.lock().unwrap()[&track_key(&t)].file, changed.file);
        drop(held);
    }

    #[test]
    fn refresh_never_claims_youtube_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.m4a");
        std::fs::write(&source, b"audio").unwrap();
        let d = Downloads::load(dir.path().join("downloads"));
        let (yt, archive) = (track("Video"), track("Archived"));
        d.store_copy(&yt, ProviderId::YouTube, &source).unwrap();
        d.store_copy(&archive, ProviderId::Archive, &source).unwrap();
        for e in d.map.lock().unwrap().values_mut() {
            e.ts = 0;
            e.revision.checked_at = 0;
        }
        let claimed = d.claim_refresh(10);
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].provider, ProviderId::Archive);
    }

    #[test]
    fn refresh_cannot_resurrect_removed_downloads_or_replace_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mp3");
        std::fs::write(&source, b"old").unwrap();
        let d = Downloads::load(dir.path().join("downloads"));
        let t = track("Cancelled");
        let path = d.store_copy(&t, ProviderId::Archive, &source).unwrap();
        let old = d.map.lock().unwrap()[&track_key(&t)].clone();
        let mut fresh = old.clone();
        fresh.revision.sha256 = Some("different".into());
        assert!(d.install_refresh(&old, fresh.clone(), &dir.path().join("missing.mp3")).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"old");
        assert!(d.remove(&t));
        d.install_refresh(&old, fresh, &source).unwrap();
        assert!(!d.contains(&t));
        assert!(d.tracks().is_empty());
    }

    #[test]
    fn index_round_trip_and_remove() {
        let dir = std::env::temp_dir().join(format!("glyph-dl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = Downloads::load(dir.clone());
        let t = track("Song");
        let name = d.free_name(&t, "mp3");
        assert_eq!(name, "Band - Song.mp3");
        std::fs::write(dir.join(&name), b"x").unwrap();
        d.map.lock().unwrap().insert(track_key(&t), Entry { track: t.clone(), file: name.clone(), provider: ProviderId::Archive, bytes: 1, ts: 1, revision: Revision::default() });
        d.save();
        // Same song from another source counts as downloaded.
        let mut other = t.clone();
        other.source = ProviderId::Spotify;
        let d2 = Downloads::load(dir.clone());
        assert!(d2.contains(&other));
        assert_eq!(d2.free_name(&t, "mp3"), "Band - Song (2).mp3");
        assert!(d2.remove(&t));
        assert!(!dir.join(&name).exists());
        assert!(d2.tracks().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
