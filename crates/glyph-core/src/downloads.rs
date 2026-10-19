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

/// Larger files are refused (an album rip in one file, a broken server).
const MAX_BYTES: u64 = 500 * 1024 * 1024;
const INDEX: &str = "index.json";

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
    dir: PathBuf,
    map: Mutex<HashMap<String, Entry>>,
    index_io: Mutex<()>,
}

impl Downloads {
    /// Called by maintenance, never by playback. Claiming also backs off errors.
    pub fn claim_refresh(&self, limit: usize) -> Vec<Entry> {
        let mut map = self.map.lock().unwrap();
        let mut due: Vec<_> = map.values_mut().filter(|e| now().saturating_sub(e.revision.checked_at.max(e.ts)) >= 86400).collect();
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
        if !changed && (matched || !new_validator && now().saturating_sub(old.revision.hashed_at) < 7 * 86400) { return Ok(()); }
        drop(response);
        // A disposable download cannot replace the index or the playing file.
        let temp = tempfile::tempdir_in(&self.dir)?;
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
        let old_path = self.dir.join(&old.file);
        let old_hash = match &old.revision.sha256 { Some(hash) => hash.clone(), None => digest_file(&old_path)? };
        let changed = fresh.revision.sha256.as_ref() != Some(&old_hash);
        let replacement = if changed {
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
            let name = format!("{}-{nonce}.{}", paths::hash_hex(&track_key(&old.track)), extension("", &source.to_string_lossy()));
            let path = self.dir.join(&name);
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
        if let Err(e) = paths::write_json(&self.dir.join(INDEX), &snapshot) {
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
        let d = Self { dir, map: Mutex::new(HashMap::new()), index_io: Mutex::new(()) };
        d.reload();
        d
    }

    /// Re-reads the index (after the folder was cleared in settings). Entries
    /// whose file is gone are dropped.
    pub fn reload(&self) {
        let mut map: HashMap<String, Entry> = paths::read_json(&self.dir.join(INDEX)).unwrap_or_default();
        map.retain(|_, e| self.dir.join(&e.file).is_file());
        *self.map.lock().unwrap() = map;
    }

    fn save(&self) {
        let _index_io = self.index_io.lock().unwrap();
        let snapshot = self.map.lock().unwrap().clone();
        let _ = std::fs::create_dir_all(&self.dir);
        if let Err(e) = paths::write_json(&self.dir.join(INDEX), &snapshot) {
            tracing::warn!("could not save download index: {e}");
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Cheap check for the UI (index only, no disk access).
    pub fn contains(&self, t: &Track) -> bool {
        self.map.lock().unwrap().contains_key(&track_key(t))
    }

    /// The local file of `t` (checked on disk) and the source it came from.
    pub fn local(&self, t: &Track) -> Option<(PathBuf, ProviderId)> {
        let e = self.map.lock().unwrap().get(&track_key(t)).cloned()?;
        let p = self.dir.join(&e.file);
        p.is_file().then_some((p, e.provider))
    }

    /// Downloaded songs, newest first.
    pub fn tracks(&self) -> Vec<Track> {
        let mut v: Vec<Entry> = self.map.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| b.ts.cmp(&a.ts));
        v.into_iter().map(|e| e.track).collect()
    }

    /// Deletes the file of `t`. `true` if there was one.
    pub fn remove(&self, t: &Track) -> bool {
        let gone = self.map.lock().unwrap().remove(&track_key(t));
        match gone {
            Some(e) => {
                let _ = std::fs::remove_file(self.dir.join(&e.file));
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
        std::fs::create_dir_all(&self.dir)?;
        let (name, part, reservation) = self.reserve_name(t, ext)?;
        drop(reservation);
        if let Err(e) = std::fs::copy(src, &part) {
            let _ = std::fs::remove_file(&part);
            return Err(e.into());
        }
        let path = self.dir.join(&name);
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
        std::fs::create_dir_all(&self.dir)?;
        let request = server.map(|s| s.authorize_url(http.get(url),url)).unwrap_or_else(|| http.get(url));
        let resp = request.timeout(Duration::from_secs(15 * 60)).send().await?.error_for_status()?;
        let etag = resp.headers().get("etag").and_then(|h| h.to_str().ok()).map(str::to_string);
        let modified = resp.headers().get("last-modified").and_then(|h| h.to_str().ok()).map(str::to_string);
        if resp.content_length().is_some_and(|n| n > MAX_BYTES) {
            bail!("file too large");
        }
        if let Some(n) = resp.content_length() {
            report(|p| p.set_total(n));
        }
        let mime = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let ext = extension(&mime, resp.url().path());
        let (name, part, mut out) = self.reserve_name(t, ext)?;
        let mut bytes = 0u64;
        let started = std::time::Instant::now();
        let transfer = glyph_protocol::network::Transfer::start();
        let mut body = resp.bytes_stream();
        let res: Result<()> = async {
            while let Some(chunk) = body.next().await {
                let chunk = chunk?;
                transfer.progress();
                bytes += chunk.len() as u64;
                if bytes > MAX_BYTES {
                    bail!("file too large");
                }
                out.write_all(&chunk)?;
                report(|p| p.add(chunk.len() as u64));
            }
            out.flush()?;
            Ok(())
        }
        .await;
        drop(out);
        if let Err(e) = res {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        if bytes == 0 {
            let _ = std::fs::remove_file(&part);
            bail!("empty file");
        }
        glyph_protocol::network::observe(bytes, started.elapsed());
        let path = self.dir.join(&name);
        std::fs::rename(&part, &path)?;
        let entry = Entry { track: t.clone(), file: name, provider, bytes, ts: now(),
            revision: Revision { etag, modified, sha256: Some(digest_file(&path)?), checked_at: now(), hashed_at: now(), quality: job_quality() } };
        self.map.lock().unwrap().insert(track_key(t), entry);
        self.save();
        Ok(path)
    }

    fn reserve_name(&self, t: &Track, ext: &str) -> Result<(String, PathBuf, std::fs::File)> {
        let _lock = self.index_io.lock().unwrap();
        loop {
            let name = self.free_name(t, ext);
            let part = self.dir.join(format!("{name}.part"));
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
        let taken = |n: &str| self.dir.join(n).exists() || self.dir.join(format!("{n}.part")).exists();
        let mut name = format!("{base}.{ext}");
        let mut i = 2;
        while taken(&name) {
            name = format!("{base} ({i}).{ext}");
            i += 1;
        }
        name
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
    } else if m.contains("mp4") || m.contains("aac") || p.ends_with(".m4a") {
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
        let held = std::fs::File::open(d.dir.join(&old.file)).unwrap();
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
