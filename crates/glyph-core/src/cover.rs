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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// Data saver on: covers are fetched in a small size.
static DATA_SAVER: AtomicBool = AtomicBool::new(false);

pub fn set_data_saver(on: bool) {
    DATA_SAVER.store(on, Ordering::Relaxed);
}

pub fn data_saver() -> bool {
    DATA_SAVER.load(Ordering::Relaxed)
}

/// The same picture in a small size where the host has one in the URL
/// (Deezer, YouTube, Audius, SoundCloud, Cover Art Archive).
pub fn small_cover_url(url: &str) -> String {
    let mut u = url.to_string();
    for big in ["/1000x1000-", "/500x500-"] {
        u = u.replace(big, "/250x250-");
    }
    for big in ["maxresdefault", "sddefault", "hqdefault"] {
        u = u.replace(big, "mqdefault");
    }
    for big in ["1000x1000.jpg", "480x480.jpg"] {
        u = u.replace(big, "150x150.jpg");
    }
    for big in ["-t500x500", "-original"] {
        u = u.replace(big, "-t200x200");
    }
    if u.contains("coverartarchive.org") && u.ends_with("/front") {
        u.push_str("-250");
    }
    u
}

pub const MAX_CACHE_BYTES: u64 = 500 * 1024 * 1024;
/// Longest edge of a cached cover.
pub const COVER_PX: u32 = 512;
const MAX_DOWNLOAD: usize = 12 * 1024 * 1024;

pub struct CoverCache {
    server: Option<crate::server_client::ServerClient>,
    server_musicbrainz: Option<Arc<crate::providers::musicbrainz::MusicBrainz>>,
    dir: PathBuf,
    max_bytes: u64,
    http: reqwest::Client,
    providers: Arc<Providers>,
    /// Songs for which the whole chain failed (this session only).
    misses: Mutex<HashSet<String>>,
}

impl CoverCache {
    pub fn new(dir: PathBuf, http: reqwest::Client, providers: Arc<Providers>) -> Self {
        Self { server: None, server_musicbrainz: None, dir, max_bytes: MAX_CACHE_BYTES, http, providers, misses: Mutex::new(HashSet::new()) }
    }

    pub fn with_server(mut self, server: Option<crate::server_client::ServerClient>) -> Self {
        self.server_musicbrainz = server.as_ref().filter(|_| self.providers.musicbrainz.is_some())
            .map(|s| Arc::new(crate::providers::musicbrainz::MusicBrainz::new(self.http.clone()).with_server(s.clone())));
        self.server = server;
        self
    }

    pub fn with_max_bytes(mut self, max: u64) -> Self {
        self.max_bytes = max;
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, key: &str) -> PathBuf {
        // Older decoders re-encoded incomplete JPEGs as valid, half-grey files.
        // Those cannot be recognized by an EOF check: refetch under a new key.
        self.dir.join(format!("validated-v2-{key}.jpg"))
    }

    /// Cached cover for the song, fetching it through the fallback chain if needed.
    pub async fn get(&self, track: &Track) -> Result<Option<PathBuf>> {
        let key = paths::hash_hex(&track_key(track));
        let path = self.path_for(&key);
        if path.is_file() {
            // Covers cached before cut-off downloads were rejected may be
            // half grey: fetch those again.
            if cached_truncated(&path) {
                let _ = std::fs::remove_file(&path);
            } else {
                touch(&path);
                return Ok(Some(path));
            }
        }
        if self.misses.lock().unwrap().contains(&key) {
            return Ok(None);
        }
        tokio::fs::create_dir_all(&self.dir).await?;
        if let Some(server) = &self.server {
            let mut candidate = track.clone();
            if data_saver() {
                candidate.cover_url = candidate.cover_url.map(|u| small_cover_url(&u));
            }
            for _ in 0..2 {
                // An older server has no /v1/cover: fall through to the direct chain.
                let url = match server.cover_url(&candidate).await {
                    Ok(u) => u,
                    Err(e) => {
                        tracing::debug!("server cover failed, going direct: {e}");
                        break;
                    }
                };
                if let Some(url) = url {
                    if self.download_and_store(&url, &path).await.is_ok() {
                        return Ok(Some(self.finish(path)));
                    }
                }
                if candidate.cover_url.take().is_none() { break; }
            }
            if let Some(mb) = &self.server_musicbrainz {
                for url in mb.cover_urls(&track.artist, &track.title).await.unwrap_or_default() {
                    if self.download_and_store(&server.upstream_url(&url), &path).await.is_ok() {
                        return Ok(Some(self.finish(path)));
                    }
                }
            }
        }

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
        let small;
        let url = if data_saver() {
            small = small_cover_url(url);
            small.as_str()
        } else {
            url
        };
        let img = download_image(&self.http, url, self.server.as_ref()).await?;
        let dest = dest.to_path_buf();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let jpeg = encode_cover(img)?;
            paths::write_atomic(&dest, &jpeg)
        })
        .await??;
        Ok(())
    }
}

/// Decodes any supported image and returns a JPEG with the longest edge <= [`COVER_PX`].
pub fn normalize_image(bytes: &[u8]) -> Result<Vec<u8>> {
    // The JPEG decoder fills a cut-off file with grey instead of failing;
    // such a cover would be cached half grey for good.
    if is_truncated_jpeg(bytes) {
        return Err(anyhow!("truncated JPEG"));
    }
    encode_cover(decode_image(bytes)?)
}

fn encode_cover(img: image::DynamicImage) -> Result<Vec<u8>> {
    use image::codecs::jpeg::JpegEncoder;
    let img = if img.width().max(img.height()) > COVER_PX { img.thumbnail(COVER_PX, COVER_PX) } else { img };
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 85).encode_image(&rgb)?;
    Ok(out)
}

/// The default image decoder tolerates broken JPEG entropy and paints grey.
/// libjpeg warnings are errors here: tolerant decoders silently fill missing rows.
pub(crate) fn decode_image(bytes: &[u8]) -> Result<image::DynamicImage> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        if is_truncated_jpeg(bytes) { return Err(anyhow!("truncated JPEG")); }
        return decode_jpeg(bytes);
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode()?)
}

std::thread_local! { static JPEG_WARNING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

extern "C-unwind" fn jpeg_fatal(_: &mut mozjpeg_sys::jpeg_common_struct) {
    // Matches mozjpeg's unwinding error manager; caught at the decode boundary.
    std::panic::resume_unwind(Box::new("invalid JPEG"));
}

extern "C-unwind" fn jpeg_message(_: &mut mozjpeg_sys::jpeg_common_struct, level: std::ffi::c_int) {
    if level < 0 { JPEG_WARNING.with(|warning| warning.set(true)); }
}

fn decode_jpeg(bytes: &[u8]) -> Result<image::DynamicImage> {
    JPEG_WARNING.with(|warning| warning.set(false));
    std::panic::catch_unwind(|| -> Result<image::DynamicImage> {
        // SAFETY: jpeg_error_mgr contains nullable function pointers, pointers
        // and integers. Zero initialization is valid; jpeg_std_error then
        // installs its defaults before the owned manager reaches the decoder.
        let mut errors = unsafe {
            let mut errors = std::mem::zeroed();
            mozjpeg_sys::jpeg_std_error(&mut errors);
            errors
        };
        errors.error_exit = Some(jpeg_fatal);
        errors.emit_message = Some(jpeg_message);
        let decoder = mozjpeg::Decompress::with_err(errors).from_mem(bytes)?;
        let (width, height) = (decoder.width(), decoder.height());
        if width > 8192 || height > 8192 || width.checked_mul(height).and_then(|n| n.checked_mul(3)).is_none_or(|n| n > 64 * 1024 * 1024) {
            return Err(anyhow!("image too large"));
        }
        let mut decoder = decoder.rgb()?;
        let pixels = decoder.read_scanlines::<u8>()?;
        decoder.finish()?;
        if JPEG_WARNING.with(|warning| warning.get()) { return Err(anyhow!("incomplete JPEG scan")); }
        let rgb = image::RgbImage::from_raw(width as u32, height as u32, pixels).ok_or_else(|| anyhow!("invalid JPEG pixels"))?;
        Ok(image::DynamicImage::ImageRgb8(rgb))
    }).map_err(|_| anyhow!("invalid JPEG"))?
}

/// Fetch completely and validate before publishing; retry cut-off transfers.
pub(crate) async fn download_image(http: &reqwest::Client, url: &str, server: Option<&crate::server_client::ServerClient>) -> Result<image::DynamicImage> {
    use futures::StreamExt;
    let mut last = None;
    for attempt in 0..2 {
        if attempt > 0 { tokio::time::sleep(Duration::from_millis(300)).await; }
        let result: Result<image::DynamicImage> = async {
            let request = server.map(|s| s.authorize_url(http.get(url), url)).unwrap_or_else(|| http.get(url));
            let response = request.timeout(Duration::from_secs(15)).send().await?.error_for_status()?;
            if response.status() != reqwest::StatusCode::OK { return Err(anyhow!("incomplete image response")); }
            let expected = response.content_length();
            if expected.is_some_and(|n| n > MAX_DOWNLOAD as u64) { return Err(anyhow!("image too large")); }
            let mut bytes = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                if bytes.len() + chunk.len() > MAX_DOWNLOAD { return Err(anyhow!("image too large")); }
                bytes.extend_from_slice(&chunk);
            }
            if expected.is_some_and(|n| n != bytes.len() as u64) { return Err(anyhow!("image download cut off")); }
            tokio::task::spawn_blocking(move || decode_image(&bytes)).await?
        }.await;
        match result { Ok(image) => return Ok(image), Err(error) => last = Some(error) }
    }
    Err(last.unwrap_or_else(|| anyhow!("image unavailable")))
}

/// A JPEG without its end-of-image marker (FF D9) near the end.
fn is_truncated_jpeg(bytes: &[u8]) -> bool {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return false;
    }
    // Some encoders leave a few bytes of padding after the marker.
    let tail = &bytes[bytes.len().saturating_sub(64)..];
    !tail.windows(2).any(|w| w == [0xFF, 0xD9])
}

fn cached_truncated(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return true };
    let mut head = [0u8; 2];
    if f.read_exact(&mut head).is_err() || head != [0xFF, 0xD8] {
        return true;
    }
    let mut tail = Vec::new();
    if f.seek(SeekFrom::End(-64)).is_err() || f.read_to_end(&mut tail).is_err() {
        return true;
    }
    !tail.windows(2).any(|w| w == [0xFF, 0xD9])
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
    fn truncated_jpeg_is_rejected() {
        let img = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 30, 30]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg).encode_image(&img).unwrap();
        assert!(!is_truncated_jpeg(&jpeg));
        assert!(normalize_image(&jpeg).is_ok());
        let cut = &jpeg[..jpeg.len() * 6 / 10];
        assert!(is_truncated_jpeg(cut));
        assert!(normalize_image(cut).is_err());
        // Not a JPEG: left to the decoder.
        assert!(!is_truncated_jpeg(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn truncated_jpeg_with_a_forged_end_marker_is_rejected() {
        let img = image::RgbImage::from_fn(256, 256, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, ((x * y) % 255) as u8]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg).encode_image(&img).unwrap();
        for length in [jpeg.len() * 2 / 3, jpeg.len() * 9 / 10, jpeg.len() - 24] {
            let mut broken = jpeg[..length].to_vec();
            broken.extend_from_slice(&[0xff, 0xd9]);
            assert!(!is_truncated_jpeg(&broken));
            assert!(normalize_image(&broken).is_err(), "accepted a {length}-byte partial scan");
        }
        assert!(normalize_image(&[0xff, 0xd8, 0xff, 0xd9]).is_err());
    }

    #[test]
    fn complete_progressive_photos_and_real_grey_backgrounds_are_accepted() {
        let img = image::RgbImage::from_fn(64, 64, |_, y| if y > 32 { image::Rgb([128, 128, 128]) } else { image::Rgb([20, 180, 50]) });
        let mut baseline = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut baseline).encode_image(&img).unwrap();
        assert!(normalize_image(&baseline).is_ok());
        let mut encoder = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        encoder.set_size(64, 64);
        encoder.set_progressive_mode();
        let mut encoder = encoder.start_compress(Vec::new()).unwrap();
        encoder.write_scanlines(img.as_raw()).unwrap();
        let progressive = encoder.finish().unwrap();
        assert!(progressive.windows(2).any(|b| b == [0xff, 0xc2]));
        assert!(normalize_image(&progressive).is_ok());
    }

    #[tokio::test]
    async fn retries_incomplete_image_before_accepting_the_complete_one() {
        use std::io::{Read, Write};
        let image = image::RgbImage::from_pixel(16, 16, image::Rgb([220, 30, 20]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg).encode_image(&image).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            for complete in [false, true] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request);
                let body = if complete { &jpeg[..] } else { &jpeg[..jpeg.len() / 2] };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        let result = download_image(&reqwest::Client::new(), &format!("http://{addr}"), None).await.unwrap();
        thread.join().unwrap();
        assert_eq!((result.width(), result.height()), (16, 16));
    }

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
