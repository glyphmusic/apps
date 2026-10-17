//! Offline downloads.
//!
//! Only songs whose source allows downloading are saved: Jamendo tracks with
//! `audiodownload_allowed`, Audius tracks the artist marked downloadable, and
//! Creative-Commons / public-domain items on the Internet Archive, and YouTube
//! audio when the local yt-dlp is set up (same URL as playback, see
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
    job().map(|j| j.quality).unwrap_or(AudioQuality::Standard)
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
    let fmt = match q {
        AudioQuality::Low => "mp31",
        AudioQuality::Standard => return None,
        AudioQuality::High => "flac",
    };
    url.contains("/download/track/").then(|| url.replacen("/mp32/", &format!("/{fmt}/"), 1)).filter(|u| u != url)
}

/// Larger files are refused (an album rip in one file, a broken server).
const MAX_BYTES: u64 = 500 * 1024 * 1024;
const INDEX: &str = "index.json";

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
}

/// The downloads folder and its index.
pub struct Downloads {
    dir: PathBuf,
    map: Mutex<HashMap<String, Entry>>,
}

impl Downloads {
    pub fn load(dir: PathBuf) -> Self {
        let d = Self { dir, map: Mutex::new(HashMap::new()) };
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
        let name = self.free_name(t, ext);
        let path = self.dir.join(&name);
        std::fs::copy(src, &path)?;
        let entry = Entry { track: t.clone(), file: name, provider, bytes: len, ts: now() };
        self.map.lock().unwrap().insert(track_key(t), entry);
        self.save();
        Ok(path)
    }

    /// Fetches `url` into the folder and records it for `t`.
    pub async fn fetch(&self, http: &reqwest::Client, t: &Track, provider: ProviderId, url: &str) -> Result<PathBuf> {
        if !url.starts_with("https://") {
            bail!("refusing non-HTTPS download URL");
        }
        std::fs::create_dir_all(&self.dir)?;
        let resp = http.get(url).timeout(Duration::from_secs(15 * 60)).send().await?.error_for_status()?;
        if resp.content_length().is_some_and(|n| n > MAX_BYTES) {
            bail!("file too large");
        }
        if let Some(n) = resp.content_length() {
            report(|p| p.set_total(n));
        }
        let mime = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let ext = extension(&mime, resp.url().path());
        let name = self.free_name(t, ext);
        let part = self.dir.join(format!("{name}.part"));
        let mut out = std::fs::File::create(&part)?;
        let mut bytes = 0u64;
        let mut body = resp.bytes_stream();
        let res: Result<()> = async {
            while let Some(chunk) = body.next().await {
                let chunk = chunk?;
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
        let path = self.dir.join(&name);
        std::fs::rename(&part, &path)?;
        let entry = Entry { track: t.clone(), file: name, provider, bytes, ts: now() };
        self.map.lock().unwrap().insert(track_key(t), entry);
        self.save();
        Ok(path)
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
    fn index_round_trip_and_remove() {
        let dir = std::env::temp_dir().join(format!("glyph-dl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = Downloads::load(dir.clone());
        let t = track("Song");
        let name = d.free_name(&t, "mp3");
        assert_eq!(name, "Band - Song.mp3");
        std::fs::write(dir.join(&name), b"x").unwrap();
        d.map.lock().unwrap().insert(track_key(&t), Entry { track: t.clone(), file: name.clone(), provider: ProviderId::Archive, bytes: 1, ts: 1 });
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
