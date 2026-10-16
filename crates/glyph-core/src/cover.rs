//! Cover cache: size-normalised JPEGs on disk (LRU, 500 MB) with the fallback
//! chain provider cover -> Spotify/Deezer -> MusicBrainz Cover Art Archive.
//! If everything fails the UI draws a generated ASCII pattern (`None`).

use crate::dedupe::{artist_similarity, title_similarity, track_key};
use crate::paths;
use crate::provider::MusicProvider;
use crate::providers::Providers;
use anyhow::{anyhow, Result};
use glyph_protocol::{ProviderId, Track};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

pub const MAX_CACHE_BYTES: u64 = 500 * 1024 * 1024;
/// Longest edge of a cached cover.
pub const COVER_PX: u32 = 512;
const MAX_DOWNLOAD: usize = 12 * 1024 * 1024;

pub struct CoverCache {
    dir: PathBuf,
    max_bytes: u64,
    http: reqwest::Client,
    providers: Arc<Providers>,
    /// Songs for which the whole chain failed (this session only).
    misses: Mutex<HashSet<String>>,
}

impl CoverCache {
    pub fn new(dir: PathBuf, http: reqwest::Client, providers: Arc<Providers>) -> Self {
        Self { dir, max_bytes: MAX_CACHE_BYTES, http, providers, misses: Mutex::new(HashSet::new()) }
    }

    pub fn with_max_bytes(mut self, max: u64) -> Self {
        self.max_bytes = max;
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.jpg"))
    }

    /// Cached cover for the song, fetching it through the fallback chain if needed.
    pub async fn get(&self, track: &Track) -> Result<Option<PathBuf>> {
        let key = paths::hash_hex(&track_key(track));
        let path = self.path_for(&key);
        if path.is_file() {
            touch(&path);
            return Ok(Some(path));
        }
        if self.misses.lock().unwrap().contains(&key) {
            return Ok(None);
        }
        tokio::fs::create_dir_all(&self.dir).await?;

        let mut tried: Vec<String> = Vec::new();
        // 1. provider cover
        if let Some(u) = &track.cover_url {
            if self.try_url(u, &path, &mut tried).await {
                return Ok(Some(self.finish(path)));
            }
        }
        // 2. Spotify / Deezer
        for u in self.metadata_cover_urls(track).await {
            if self.try_url(&u, &path, &mut tried).await {
                return Ok(Some(self.finish(path)));
            }
        }
        // 3. Cover Art Archive via MusicBrainz
        if let Some(mb) = &self.providers.musicbrainz {
            match mb.cover_urls(&track.artist, &track.title).await {
                Ok(urls) => {
                    for u in urls {
                        if self.try_url(&u, &path, &mut tried).await {
                            return Ok(Some(self.finish(path)));
                        }
                    }
                }
                Err(e) => tracing::debug!("musicbrainz cover lookup failed: {e}"),
            }
        }
        self.misses.lock().unwrap().insert(key);
        Ok(None)
    }

    fn finish(&self, path: PathBuf) -> PathBuf {
        let dir = self.dir.clone();
        let max = self.max_bytes;
        std::thread::spawn(move || {
            let _ = prune(&dir, max);
        });
        path
    }

    /// Cover URLs from Spotify / Deezer (known ids first, then a text/ISRC match).
    async fn metadata_cover_urls(&self, track: &Track) -> Vec<String> {
        let mut urls = Vec::new();
        let wants: [(ProviderId, Option<Arc<dyn MusicProvider>>); 2] = [
            (ProviderId::Deezer, self.providers.get(ProviderId::Deezer)),
            (ProviderId::Spotify, self.providers.get(ProviderId::Spotify)),
        ];
        for (id, provider) in wants {
            let Some(p) = provider else { continue };
            if track.source == id {
                continue; // that cover was step 1
            }
            let found: Option<Track> = async {
                if let Some(known) = track.external_ids.get(&id) {
                    if let Ok(t) = p.track(known).await {
                        return Some(t);
                    }
                }
                if let Some(isrc) = &track.isrc {
                    if let Ok(Some(t)) = p.find_by_isrc(isrc).await {
                        return Some(t);
                    }
                }
                let q = format!("{} {}", track.artist, track.title);
                let hits = p.search(&q).await.ok()?;
                hits.into_iter().find(|h| {
                    title_similarity(&track.title, &h.title) >= 0.85 && artist_similarity(&track.artist, &h.artist) >= 0.8
                })
            }
            .await;
            if let Some(u) = found.and_then(|t| t.cover_url) {
                urls.push(u);
            }
        }
        urls
    }

    /// Downloads + normalises one URL into `dest`. Returns `true` on success.
    async fn try_url(&self, url: &str, dest: &Path, tried: &mut Vec<String>) -> bool {
        if tried.iter().any(|t| t == url) || !(url.starts_with("https://") || url.starts_with("http://")) {
            return false;
        }
        tried.push(url.to_string());
        match self.download_and_store(url, dest).await {
            Ok(()) => true,
            Err(e) => {
                tracing::debug!(%url, "cover download failed: {e}");
                false
            }
        }
    }

    async fn download_and_store(&self, url: &str, dest: &Path) -> Result<()> {
        let resp = self.http.get(url).timeout(Duration::from_secs(10)).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("HTTP {}", resp.status()));
        }
        if resp.content_length().map(|l| l as usize > MAX_DOWNLOAD).unwrap_or(false) {
            return Err(anyhow!("cover too large"));
        }
        let bytes = resp.bytes().await?;
        if bytes.len() > MAX_DOWNLOAD {
            return Err(anyhow!("cover too large"));
        }
        let dest = dest.to_path_buf();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let jpeg = normalize_image(&bytes)?;
            paths::write_atomic(&dest, &jpeg)
        })
        .await??;
        Ok(())
    }
}

/// Decodes any supported image and returns a JPEG with the longest edge <= [`COVER_PX`].
pub fn normalize_image(bytes: &[u8]) -> Result<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    let img = image::load_from_memory(bytes)?;
    let img = if img.width().max(img.height()) > COVER_PX { img.thumbnail(COVER_PX, COVER_PX) } else { img };
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 85).encode_image(&rgb)?;
    Ok(out)
}

fn touch(path: &Path) {
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Deletes least-recently-used files until the directory is below 90 % of `max_bytes`.
/// Returns the number of removed files.
pub fn prune(dir: &Path, max_bytes: u64) -> std::io::Result<usize> {
    let mut files: Vec<(PathBuf, u64, SystemTime)> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let md = e.metadata().ok()?;
            md.is_file().then(|| (e.path(), md.len(), md.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max_bytes {
        return Ok(0);
    }
    files.sort_by_key(|f| f.2); // oldest first
    let target = max_bytes / 10 * 9;
    let mut removed = 0;
    for (path, size, _) in files {
        if total <= target {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_to_max_edge_jpeg() {
        let img = image::RgbImage::from_fn(1200, 800, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, 90]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let jpeg = normalize_image(&png).unwrap();
        let back = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(back.width(), COVER_PX);
        assert_eq!(back.height(), 341);
        assert!(normalize_image(b"not an image").is_err());
    }

    #[test]
    fn prune_removes_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        for (i, age_s) in [(0, 300u64), (1, 200), (2, 100), (3, 10)] {
            let p = dir.path().join(format!("{i}.jpg"));
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
            f.set_modified(now - Duration::from_secs(age_s)).unwrap();
        }
        // limit 250 B -> target 225 B: delete the two oldest (400 -> 200)
        assert_eq!(prune(dir.path(), 250).unwrap(), 2);
        assert!(!dir.path().join("0.jpg").exists());
        assert!(!dir.path().join("1.jpg").exists());
        assert!(dir.path().join("2.jpg").exists());
        assert!(dir.path().join("3.jpg").exists());
        assert_eq!(prune(dir.path(), 10_000).unwrap(), 0);
    }
}
