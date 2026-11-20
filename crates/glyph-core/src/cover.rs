//! Cover cache: size-normalised JPEGs on disk (LRU, 500 MB) with the fallback
//! chain non-YouTube cover -> Spotify/Deezer -> MusicBrainz Cover Art Archive,
//! falling back to YouTube thumbnails only when no other cover exists.
//! If everything fails the UI draws a generated ASCII pattern (`None`).

use crate::dedupe::{artist_similarity, normalize_artist, normalize_title, title_similarity, track_key};
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

/// `true` when `url` (or its `/v1/upstream?url=` target) points to a YouTube
/// image host (`ytimg.com`, `youtube.com`, `ggpht.com`).
pub fn is_youtube_cover(url: &str) -> bool {
    fn yt_host(host: &str) -> bool {
        const YT_HOSTS: &[&str] = &["ytimg.com", "youtube.com", "ggpht.com"];
        let h = host.to_ascii_lowercase();
        YT_HOSTS.iter().any(|s| h == *s || h.ends_with(&format!(".{s}")))
    }
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.path().ends_with("/v1/upstream") {
        if let Some((_, inner)) = parsed.query_pairs().find(|(k, _)| k == "url") {
            if let Ok(inner_url) = reqwest::Url::parse(&inner) {
                return inner_url.host_str().is_some_and(yt_host);
            }
        }
    }
    parsed.host_str().is_some_and(yt_host)
}

/// `true` when the cached file's header dimensions are within 10% of 1:1
/// (so 4:3 and 16:9 YouTube thumbnails are detected without decoding pixels).
fn is_roughly_square(path: &Path) -> bool {
    match image::image_dimensions(path) {
        Ok((w, h)) if w > 0 && h > 0 => w.min(h) * 100 >= w.max(h) * 90,
        _ => true,
    }
}

pub const MAX_CACHE_BYTES: u64 = 500 * 1024 * 1024;
/// Longest edge of a cached cover.
pub const COVER_PX: u32 = 512;
pub(crate) const MAX_DOWNLOAD: usize = 12 * 1024 * 1024;
/// Cap simultaneous full-res cover/artist image decodes to keep peak heap small.
const MAX_CONCURRENT_IMAGE_DECODES: usize = 4;
const PRUNE_INTERVAL: Duration = Duration::from_secs(60);

fn image_decode_semaphore() -> &'static tokio::sync::Semaphore {
    static SEM: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    SEM.get_or_init(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_IMAGE_DECODES))
}

#[derive(Debug, Clone)]
pub struct CoverResult {
    pub path: PathBuf,
    pub album: Option<String>,
    pub cover_url: Option<String>,
}

const SERVER_REQ_GAP: Duration = Duration::from_millis(180);
const SERVER_RATE_LIMIT_DEFAULT_SECS: u64 = 15;

pub struct CoverCache {
    server: Option<crate::server_client::ServerClient>,
    server_musicbrainz: Option<Arc<crate::providers::musicbrainz::MusicBrainz>>,
    dir: PathBuf,
    max_bytes: u64,
    http: reqwest::Client,
    providers: Arc<Providers>,
    /// Songs for which the whole chain failed (this session only).
    misses: Mutex<HashSet<String>>,
    /// Tracks whose album metadata was already checked cleanly this session.
    meta_checked: Mutex<HashSet<String>>,
    /// Non-square cached covers already checked for a non-YouTube upgrade this session.
    upgraded: Mutex<HashSet<String>>,
    last_prune: Mutex<Option<std::time::Instant>>,
    pruning: Arc<AtomicBool>,
    /// Bounds concurrent network cover lookups so large playlists load progressively.
    fetch_sem: tokio::sync::Semaphore,
    last_server_req: tokio::sync::Mutex<Option<std::time::Instant>>,
    server_cooldown_until: Mutex<Option<std::time::Instant>>,
}

impl CoverCache {
    pub fn new(dir: PathBuf, http: reqwest::Client, providers: Arc<Providers>) -> Self {
        Self {
            server: None,
            server_musicbrainz: None,
            dir,
            max_bytes: MAX_CACHE_BYTES,
            http,
            providers,
            misses: Mutex::new(HashSet::new()),
            meta_checked: Mutex::new(HashSet::new()),
            upgraded: Mutex::new(HashSet::new()),
            last_prune: Mutex::new(None),
            pruning: Arc::new(AtomicBool::new(false)),
            fetch_sem: tokio::sync::Semaphore::new(2),
            last_server_req: tokio::sync::Mutex::new(None),
            server_cooldown_until: Mutex::new(None),
        }
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

    fn try_claim_upgrade(&self, key: &str) -> bool {
        self.upgraded.lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_string())
    }

    fn server_cooling_down(&self) -> bool {
        let now = std::time::Instant::now();
        let mut g = self.server_cooldown_until.lock().unwrap_or_else(|e| e.into_inner());
        match *g {
            Some(until) if until > now => true,
            Some(_) => { *g = None; false }
            None => false,
        }
    }

    fn note_rate_limit(&self, retry_after_s: Option<u64>) {
        let secs = retry_after_s.unwrap_or(SERVER_RATE_LIMIT_DEFAULT_SECS).clamp(5, 120);
        let until = std::time::Instant::now() + Duration::from_secs(secs);
        let mut g = self.server_cooldown_until.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none_or(|cur| until > cur) {
            *g = Some(until);
        }
    }

    fn note_server_error(&self, err: &anyhow::Error) -> bool {
        if let Some(se) = err.downcast_ref::<crate::server_client::ServerError>() {
            if se.status == 429 {
                self.note_rate_limit(se.retry_after_s);
                return true;
            }
            return se.status >= 500 || se.status == 408;
        }
        if let Some(re) = err.downcast_ref::<reqwest::Error>() {
            if re.status() == Some(reqwest::StatusCode::TOO_MANY_REQUESTS) {
                self.note_rate_limit(None);
                return true;
            }
            return re.is_timeout() || re.is_connect() || re.status().is_some_and(|s| s.is_server_error());
        }
        false
    }

    async fn pace_server(&self) {
        let mut last = self.last_server_req.lock().await;
        if let Some(at) = *last {
            let elapsed = at.elapsed();
            if elapsed < SERVER_REQ_GAP {
                tokio::time::sleep(SERVER_REQ_GAP - elapsed).await;
            }
        }
        *last = Some(std::time::Instant::now());
    }

    /// A fetched cover; an album found on the way is remembered for the song.
    fn found(&self, track: &Track, path: PathBuf, album: Option<String>, cover_url: Option<String>) -> CoverResult {
        if album.as_deref().is_some_and(|a| !a.trim().is_empty()) {
            crate::album_names::remember(track, album.as_deref());
        }
        CoverResult { path: self.finish(path), album, cover_url }
    }

    /// Cached cover for the song, fetching it through the fallback chain if needed.
    pub async fn get(&self, track: &Track) -> Result<Option<PathBuf>> {
        Ok(self.get_with_meta(track).await?.map(|r| r.path))
    }

    /// Cached cover plus any freshly discovered album / cover URL metadata.
    pub async fn get_with_meta(&self, track: &Track) -> Result<Option<CoverResult>> {
        let key = paths::hash_hex(&track_key(track));
        let path = self.path_for(&key);
        if path.is_file() {
            // Covers cached before cut-off downloads were rejected may be
            // half grey: fetch those again.
            if cached_truncated(&path) {
                let _ = std::fs::remove_file(&path);
            } else if !is_roughly_square(&path) && !self.server_cooling_down() && self.try_claim_upgrade(&key) {
                let _permit = self.fetch_sem.acquire().await?;
                let (ok, _, album, cover_url) = self.fetch_non_youtube_meta(track, &path).await;
                if ok {
                    return Ok(Some(self.found(track, path, album, cover_url)));
                }
                touch(&path);
                return Ok(Some(CoverResult { path, album: None, cover_url: None }));
            } else {
                touch(&path);
                return Ok(Some(CoverResult { path, album: None, cover_url: None }));
            }
        }
        if self.misses.lock().unwrap().contains(&key) {
            return Ok(None);
        }
        tokio::fs::create_dir_all(&self.dir).await?;
        let _permit = self.fetch_sem.acquire().await?;
        if path.is_file() && !cached_truncated(&path) {
            touch(&path);
            return Ok(Some(CoverResult { path, album: None, cover_url: None }));
        }
        let (ok, mut transient, album, cover_url) = self.fetch_non_youtube_meta(track, &path).await;
        if ok {
            return Ok(Some(self.found(track, path, album, cover_url)));
        }
        // Last resort: YouTube thumbnail only when no square/non-YouTube cover exists.
        if let Some(u) = track.cover_url.as_deref().filter(|u| is_youtube_cover(u)) {
            if let Some(server) = &self.server {
                if self.server_cooling_down() {
                    transient = true;
                } else {
                    self.pace_server().await;
                    match self.download_and_store(&server.upstream_url(u), &path).await {
                        Ok(()) => return Ok(Some(CoverResult { path: self.finish(path), album: None, cover_url: None })),
                        Err(e) => { transient |= self.note_server_error(&e); }
                    }
                }
            }
            let mut tried = Vec::new();
            if self.try_url(u, &path, &mut tried).await {
                return Ok(Some(CoverResult { path: self.finish(path), album: None, cover_url: None }));
            }
        }
        if !transient {
            self.misses.lock().unwrap().insert(key);
        }
        Ok(None)
    }

    /// Resolves missing album (and upgraded cover URL) progressively when a track's
    /// cover is already cached locally or has no album tag stored.
    pub async fn resolve_meta(&self, track: &Track) -> Result<Option<(Option<String>, Option<String>)>> {
        if track.album.as_deref().is_some_and(|a| !a.trim().is_empty()) {
            return Ok(None);
        }
        if track.title.trim().is_empty() || track.artist.trim().is_empty() {
            return Ok(None);
        }
        // Found (or not found) before, also in an earlier run: no network.
        match crate::album_names::lookup(track) {
            crate::album_names::Known::Album(a) => return Ok(Some((None, Some(a)))),
            crate::album_names::Known::Missing => return Ok(None),
            crate::album_names::Known::Unknown => {}
        }
        let key = paths::hash_hex(&track_key(track));
        if self.meta_checked.lock().unwrap_or_else(|e| e.into_inner()).contains(&key) {
            return Ok(None);
        }
        if self.server.is_some() && self.server_cooling_down() {
            return Ok(None);
        }
        let _permit = self.fetch_sem.acquire().await?;
        let mut transient = false;
        let mut found_album: Option<String> = None;
        let mut found_cover: Option<String> = None;
        if let Some(server) = &self.server {
            if self.server_cooling_down() {
                return Ok(None);
            }
            self.pace_server().await;
            match server.cover_meta(track).await {
                Ok(meta) => {
                    found_album = meta.album;
                    found_cover = meta.raw_url.filter(|u| !is_youtube_cover(u));
                }
                Err(e) => {
                    transient = self.note_server_error(&e);
                    tracing::debug!("server meta backfill failed: {e}");
                }
            }
        }
        if found_album.is_none() && !transient {
            for (cover, album) in self.metadata_cover_hits(track).await {
                if found_cover.is_none() && !is_youtube_cover(&cover) {
                    found_cover = Some(cover);
                }
                if found_album.is_none() {
                    found_album = album.filter(|a| !a.trim().is_empty());
                }
                if found_album.is_some() {
                    break;
                }
            }
        }
        if !transient {
            self.meta_checked.lock().unwrap_or_else(|e| e.into_inner()).insert(key);
            crate::album_names::remember(track, found_album.as_deref());
        }
        if found_album.is_some() || found_cover.is_some() {
            Ok(Some((found_cover, found_album)))
        } else {
            Ok(None)
        }
    }

    /// Fetch a non-YouTube cover into `path`: non-YouTube provider URL,
    /// Spotify/Deezer (via server or directly), then MusicBrainz Cover Art Archive.
    async fn fetch_non_youtube_meta(&self, track: &Track, path: &Path) -> (bool, bool, Option<String>, Option<String>) {
        let mut server_mb_checked = false;
        let mut transient = false;
        if let Some(u) = track.cover_url.as_deref().filter(|u| !is_youtube_cover(u)) {
            let scaled = if data_saver() { small_cover_url(u) } else { u.to_string() };
            if let Some(server) = &self.server {
                if self.server_cooling_down() {
                    transient = true;
                } else {
                    self.pace_server().await;
                    match self.download_and_store(&server.upstream_url(&scaled), path).await {
                        Ok(()) => return (true, false, None, None),
                        Err(e) => { transient |= self.note_server_error(&e); }
                    }
                }
            } else if self.download_and_store(&scaled, path).await.is_ok() {
                return (true, false, None, None);
            }
        }
        if let Some(server) = &self.server {
            if self.server_cooling_down() {
                transient = true;
            } else {
                let mut candidate = track.clone();
                // Own non-YouTube URL was already tried above; ask server to search metadata if missing/failed.
                candidate.cover_url = None;
                self.pace_server().await;
                let mut server_ok = true;
                match server.cover_meta(&candidate).await {
                    Ok(meta) => {
                        if let Some(url) = meta.url.filter(|u| !is_youtube_cover(u)) {
                            self.pace_server().await;
                            match self.download_and_store(&url, path).await {
                                Ok(()) => {
                                    let clean_cover = meta.raw_url.filter(|u| !is_youtube_cover(u));
                                    return (true, false, meta.album, clean_cover);
                                }
                                Err(e) => { transient |= self.note_server_error(&e); }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("server cover failed, going direct: {e}");
                        server_ok = false;
                        transient |= self.note_server_error(&e);
                    }
                }
                if server_ok && !self.server_cooling_down() {
                    if let Some(mb) = &self.server_musicbrainz {
                        server_mb_checked = true;
                        for url in Self::mb_cover_urls(mb, track).await {
                            self.pace_server().await;
                            match self.download_and_store(&server.upstream_url(&url), path).await {
                                Ok(()) => return (true, false, None, None),
                                Err(e) => {
                                    if self.note_server_error(&e) {
                                        transient = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut tried: Vec<String> = Vec::new();
        // 1. Non-YouTube provider cover
        if let Some(u) = track.cover_url.as_deref().filter(|u| !is_youtube_cover(u)) {
            if self.try_url(u, path, &mut tried).await {
                return (true, false, None, None);
            }
        }
        // 2. Spotify / Deezer
        for (u, album) in self.metadata_cover_hits(track).await {
            if !is_youtube_cover(&u) && self.try_url(&u, path, &mut tried).await {
                return (true, false, album, Some(u));
            }
        }
        // 3. Cover Art Archive via MusicBrainz
        if !server_mb_checked {
            if let Some(mb) = &self.providers.musicbrainz {
                for u in Self::mb_cover_urls(mb, track).await {
                    if self.try_url(&u, path, &mut tried).await {
                        return (true, false, None, None);
                    }
                }
            }
        }
        (false, transient, None, None)
    }

    async fn mb_cover_urls(mb: &crate::providers::musicbrainz::MusicBrainz, track: &Track) -> Vec<String> {
        match mb.cover_urls(&track.artist, &track.title).await {
            Ok(urls) if !urls.is_empty() => return urls,
            Ok(_) => {}
            Err(e) => tracing::debug!("musicbrainz cover lookup failed: {e}"),
        }
        let (na, nt) = (normalize_artist(&track.artist), normalize_title(&track.title));
        if !na.is_empty() && !nt.is_empty() && (!na.eq_ignore_ascii_case(&track.artist) || !nt.eq_ignore_ascii_case(&track.title)) {
            if let Ok(urls) = mb.cover_urls(&na, &nt).await {
                return urls;
            }
        }
        Vec::new()
    }

    fn try_claim_prune(&self) -> bool {
        let now = std::time::Instant::now();
        {
            let guard = self.last_prune.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_some_and(|t| now.saturating_duration_since(t) < PRUNE_INTERVAL) {
                return false;
            }
        }
        if self.pruning.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return false;
        }
        *self.last_prune.lock().unwrap_or_else(|e| e.into_inner()) = Some(now);
        true
    }

    fn finish(&self, path: PathBuf) -> PathBuf {
        if self.try_claim_prune() {
            let dir = self.dir.clone();
            let max = self.max_bytes;
            let pruning = self.pruning.clone();
            tokio::task::spawn_blocking(move || {
                let _ = prune(&dir, max);
                pruning.store(false, Ordering::Release);
            });
        }
        path
    }

    /// Cover URLs and album names from Spotify / Deezer (known ids first, then a text/ISRC match).
    async fn metadata_cover_hits(&self, track: &Track) -> Vec<(String, Option<String>)> {
        let mut hits_out = Vec::new();
        let wants: [(ProviderId, Option<Arc<dyn MusicProvider>>); 2] = [
            (ProviderId::Deezer, self.providers.get(ProviderId::Deezer)),
            (ProviderId::Spotify, self.providers.get(ProviderId::Spotify)),
        ];
        let matches_track = |h: &Track| {
            h.cover_url.as_deref().is_some_and(|u| !is_youtube_cover(u))
                && title_similarity(&track.title, &h.title) >= 0.85
                && artist_similarity(&track.artist, &h.artist) >= 0.8
        };
        for (id, provider) in wants {
            let Some(p) = provider else { continue };
            if track.source == id && track.cover_url.as_deref().is_some_and(|u| !is_youtube_cover(u)) {
                continue; // that cover was step 1
            }
            let found: Option<Track> = async {
                if track.source == id {
                    if let Ok(t) = p.track(&track.id).await {
                        if t.cover_url.as_deref().is_some_and(|u| !is_youtube_cover(u)) {
                            return Some(t);
                        }
                    }
                }
                if let Some(known) = track.external_ids.get(&id) {
                    if let Ok(t) = p.track(known).await {
                        if t.cover_url.as_deref().is_some_and(|u| !is_youtube_cover(u)) {
                            return Some(t);
                        }
                    }
                }
                if let Some(isrc) = &track.isrc {
                    if let Ok(Some(t)) = p.find_by_isrc(isrc).await {
                        if t.cover_url.as_deref().is_some_and(|u| !is_youtube_cover(u)) {
                            return Some(t);
                        }
                    }
                }
                let q = format!("{} {}", track.artist, track.title);
                if let Ok(hits) = p.search(&q).await {
                    if let Some(hit) = hits.into_iter().find(matches_track) {
                        return Some(hit);
                    }
                }
                let (na, nt) = (normalize_artist(&track.artist), normalize_title(&track.title));
                let clean_q = format!("{na} {nt}");
                if !na.is_empty() && !nt.is_empty() && !clean_q.eq_ignore_ascii_case(&q) {
                    if let Ok(hits) = p.search(&clean_q).await {
                        if let Some(hit) = hits.into_iter().find(matches_track) {
                            return Some(hit);
                        }
                    }
                }
                None
            }
            .await;
            if let Some(t) = found {
                if let Some(u) = t.cover_url.filter(|u| !is_youtube_cover(u)) {
                    hits_out.push((u, t.album.filter(|a| !a.trim().is_empty())));
                }
            }
        }
        hits_out
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
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(48 * 1024 * 1024);
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
        if width > 4096 || height > 4096 || width.checked_mul(height).and_then(|n| n.checked_mul(3)).is_none_or(|n| n > 48 * 1024 * 1024) {
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
            let _permit = image_decode_semaphore().acquire().await?;
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

    #[tokio::test]
    async fn downloads_image_concurrently_beyond_decode_slots() {
        use std::io::{Read, Write};
        const N: usize = MAX_CONCURRENT_IMAGE_DECODES + 2;
        let image = image::RgbImage::from_pixel(16, 16, image::Rgb([30, 140, 220]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg).encode_image(&image).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut streams = Vec::with_capacity(N);
            for _ in 0..N {
                let (mut stream, _) = listener.accept().unwrap();
                let mut req = [0; 4096];
                let _ = stream.read(&mut req);
                streams.push(stream);
            }
            for mut stream in streams {
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    jpeg.len()
                )
                .unwrap();
                stream.write_all(&jpeg).unwrap();
            }
        });
        let client = reqwest::Client::builder().timeout(Duration::from_secs(3)).build().unwrap();
        let url = format!("http://{addr}");
        let handles: Vec<_> = (0..N)
            .map(|_| {
                let c = client.clone();
                let u = url.clone();
                tokio::spawn(async move { download_image(&c, &u, None).await })
            })
            .collect();
        for h in handles {
            let img = h.await.unwrap().unwrap();
            assert_eq!((img.width(), img.height()), (16, 16));
        }
        server.join().unwrap();
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

    #[test]
    fn prune_claim_is_coalesced() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CoverCache::new(dir.path().to_path_buf(), reqwest::Client::new(), Arc::new(Providers::default()));
        assert!(cache.try_claim_prune());
        assert!(!cache.try_claim_prune(), "second concurrent/immediate prune must coalesce");
        cache.pruning.store(false, Ordering::Release);
        assert!(!cache.try_claim_prune(), "cooldown must skip immediate repeat prune");
    }

    #[test]
    fn detects_youtube_cover_urls() {
        assert!(is_youtube_cover("https://i.ytimg.com/vi/abc/hqdefault.jpg"));
        assert!(is_youtube_cover("https://i1.ytimg.com/vi/abc/maxresdefault.jpg"));
        assert!(is_youtube_cover("https://img.youtube.com/vi/abc/0.jpg"));
        assert!(is_youtube_cover("https://yt3.ggpht.com/abc=s900"));
        assert!(is_youtube_cover("https://glyph.my/v1/upstream?url=https%3A%2F%2Fi.ytimg.com%2Fvi%2Fabc%2Fhqdefault.jpg"));
        assert!(!is_youtube_cover("https://e-cdns-images.dzcdn.net/images/cover/x/500x500.jpg"));
        assert!(!is_youtube_cover("https://i.scdn.co/image/ab67616d0000b273"));
        assert!(!is_youtube_cover("https://coverartarchive.org/release/123/front-500"));
        assert!(!is_youtube_cover("https://evil-ytimg.com/vi/abc/hqdefault.jpg"));
        assert!(!is_youtube_cover("https://glyph.my/v1/upstream?url=https%3A%2F%2Fe-cdns-images.dzcdn.net%2Fx.jpg"));
    }

    #[tokio::test]
    async fn prefers_metadata_cover_over_youtube_and_upgrades_non_square_cache() {
        use async_trait::async_trait;
        use glyph_protocol::{Capabilities, PlaybackSource};
        use std::io::{Read, Write};

        struct MockDeezer {
            cover: String,
        }
        #[async_trait]
        impl MusicProvider for MockDeezer {
            fn id(&self) -> ProviderId { ProviderId::Deezer }
            fn capabilities(&self) -> Capabilities { Capabilities::MetadataOnly }
            async fn search(&self, _q: &str) -> Result<Vec<Track>> {
                Ok(vec![Track {
                    id: "dz1".into(),
                    source: ProviderId::Deezer,
                    title: "Get Lucky".into(),
                    artist: "Daft Punk".into(),
                    album: Some("RAM".into()),
                    duration_ms: Some(248_000),
                    cover_url: Some(self.cover.clone()),
                    isrc: None,
                    permalink: None,
                    external_ids: Default::default(),
                    explicit: false,
                }])
            }
            async fn track(&self, _id: &str) -> Result<Track> { Err(anyhow!("unused")) }
            async fn playback_source(&self, _t: &Track) -> Result<PlaybackSource> { Err(anyhow!("unused")) }
        }

        let square = image::RgbImage::from_pixel(32, 32, image::Rgb([40, 180, 90]));
        let mut square_jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut square_jpeg).encode_image(&square).unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut req = [0; 4096];
                let _ = stream.read(&mut req);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    square_jpeg.len()
                ).unwrap();
                stream.write_all(&square_jpeg).unwrap();
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let mut providers = Providers::default();
        providers.insert(Arc::new(MockDeezer { cover: format!("http://{addr}/cover.jpg") }));
        let cache = CoverCache::new(dir.path().to_path_buf(), reqwest::Client::new(), Arc::new(providers));

        let yt_track = Track {
            id: "yt1".into(),
            source: ProviderId::YouTube,
            title: "Get Lucky (Official Video)".into(),
            artist: "Daft Punk - Topic".into(),
            album: None,
            duration_ms: Some(248_000),
            cover_url: Some("https://i.ytimg.com/vi/yt1/hqdefault.jpg".into()),
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false,
        };

        // 1. Fresh lookup skips YouTube and caches Deezer's 32x32 square cover.
        let path = cache.get(&yt_track).await.unwrap().unwrap();
        assert_eq!(image::image_dimensions(&path).unwrap(), (32, 32));

        // 2. Previously cached 4:3 (64x48) thumbnail on disk is upgraded to 32x32 square.
        let rect = image::RgbImage::from_pixel(64, 48, image::Rgb([10, 10, 10]));
        let mut rect_jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut rect_jpeg).encode_image(&rect).unwrap();
        std::fs::write(&path, &rect_jpeg).unwrap();
        assert_eq!(image::image_dimensions(&path).unwrap(), (64, 48));

        let upgraded = cache.get(&yt_track).await.unwrap().unwrap();
        assert_eq!(image::image_dimensions(&upgraded).unwrap(), (32, 32));
        server.join().unwrap();
    }
}
