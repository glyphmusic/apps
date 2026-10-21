//! GPUI-free core of Glyph.
//!
//! CONTRACT (the UI depends on exactly these public items — extend, but do not
//! rename or change signatures without updating `docs/INTERFACES.md`):
//!
//! * [`Backend`] is the single entry point the UI talks to.
//! * Every `async fn` on `Backend` must be awaitable from a NON-tokio executor
//!   (GPUI's). `Backend` owns a tokio runtime and internally spawns work on it,
//!   handing results back through channels.
//! * `Backend::config()` returns an `Arc<Config>` snapshot; `Backend::set_config`
//!   swaps the config (and re-evaluates providers / server) without a restart.
//! * [`Queue`] and [`Library`] are plain synchronous data structures.

pub mod account;
pub mod config;
pub mod artist;
pub mod catalog;
pub mod cover;
pub mod dedupe;
pub mod downloads;
pub mod eq;
pub mod import;
pub mod jam;
pub mod library;
pub mod lyrics;
pub mod lyricsearch;
pub mod paths;
pub mod profile_images;
pub mod provider;
pub mod providers;
pub mod queue;
pub mod resolver;
pub mod search_ops;
pub mod server_client;
pub mod server_access;
pub mod settings_share;
pub mod settings_sync;
pub mod share;
pub mod sync;
pub mod stats;
pub mod storage;
pub mod taste;
pub mod updater;
pub mod ytdlp;
pub mod ytnative;

pub use config::Config;
pub use library::{FollowedArtist, Library, ListKey, Playlist, RecentItem, SearchItem};
pub use lyrics::Lyrics;
pub use queue::{Queue, RepeatMode};
pub use updater::{UpdateInfo, UpdateStatus};

use anyhow::{anyhow, Result};
use futures::stream::BoxStream;
use futures::StreamExt;
use glyph_protocol::{PlaybackSource, ProviderId, SearchEvent, Track};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use cover::CoverCache;
use lyrics::LyricsCache;
use provider::Cooldowns;
use providers::Providers;
use resolver::Resolver;
use server_client::{Compat, ServerClient};

mod inner;

/// 256 random bits as hex (tokens the user pastes into other apps).
pub fn random_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// Where metadata requests go.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendMode {
    /// Talking to a glyph-server instance.
    Server { url: String, server_version: String },
    /// No server configured: providers queried directly with local keys.
    Direct,
    /// Server answered with another protocol version; fell back to Direct.
    /// UI shows `[ SERVER v? · INKOMPATIBEL ]`.
    Incompatible { url: String, protocol_version: u32 },
}

type WarmCache = Arc<std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Option<Resolved>)>>>;

type StreamWarmer = Arc<dyn Fn(String) + Send + Sync>;
static STREAM_WARMER: std::sync::RwLock<Option<StreamWarmer>> = std::sync::RwLock::new(None);
pub fn set_stream_warmer(warmer: StreamWarmer) { *STREAM_WARMER.write().unwrap() = Some(warmer); }

/// Songs played lately, kept on disk (oldest dropped above the limit).
fn stream_cache_dir() -> PathBuf {
    paths::cache_dir().join("streams")
}
const STREAM_CACHE_MAX_BYTES: u64 = 1024 * 1024 * 1024;
/// Bigger files are not kept (long mixes).
const STREAM_CACHE_MAX_FILE: u64 = 80 * 1024 * 1024;

fn stream_cache_name(t: &Track) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    dedupe::track_key(t).hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Which local file of a song was dropped as broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokenFile {
    Download,
    CachedStream,
}

/// The cached file of a song played lately (touched so it stays).
fn stream_cached(t: &Track) -> Option<PathBuf> {
    let dir = stream_cache_dir();
    let name = stream_cache_name(t);
    let p = ["mp3", "m4a", "ogg", "flac", "opus", "wav", "aac"].iter().map(|e| dir.join(format!("{name}.{e}"))).find(|p| p.is_file())?;
    if let Ok(f) = std::fs::File::options().write(true).open(&p) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
    Some(p)
}

/// Saves a playing stream to the cache in the background (whole file, then
/// renamed into place; nothing is kept if it fails or is too big).
async fn cache_stream(http: reqwest::Client, t: Track, url: String, mime: Option<String>) {
    if stream_cached(&t).is_some() {
        return;
    }
    let ext_of = |m: &str| match m {
        "audio/mp4" | "audio/x-m4a" | "audio/aac" => "m4a",
        "audio/ogg" => "ogg",
        "audio/opus" => "opus",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/wav" | "audio/x-wav" => "wav",
        _ => "mp3",
    };
    let r = async {
        let mut resp = http.get(&url).timeout(std::time::Duration::from_secs(180)).send().await?.error_for_status()?;
        if resp.content_length().is_some_and(|n| n > STREAM_CACHE_MAX_FILE) {
            anyhow::bail!("too big");
        }
        let ctype = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(|s| s.split(';').next().unwrap_or("").trim().to_string());
        let ext = ext_of(mime.as_deref().or(ctype.as_deref()).unwrap_or(""));
        let dir = stream_cache_dir();
        std::fs::create_dir_all(&dir)?;
        let name = stream_cache_name(&t);
        let tmp = dir.join(format!("{name}.part"));
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            buf.extend_from_slice(&chunk);
            if buf.len() as u64 > STREAM_CACHE_MAX_FILE {
                anyhow::bail!("too big");
            }
        }
        std::fs::write(&tmp, &buf)?;
        std::fs::rename(&tmp, dir.join(format!("{name}.{ext}")))?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    match r {
        Ok(()) => {
            let _ = tokio::task::spawn_blocking(|| ytdlp::prune_audio_cache(&stream_cache_dir(), STREAM_CACHE_MAX_BYTES)).await;
        }
        Err(e) => tracing::debug!("stream cache skipped: {e:#}"),
    }
}

/// How long a source found ahead of time is trusted (stream URLs expire).
const WARM_TTL: std::time::Duration = std::time::Duration::from_secs(8 * 60);

/// A source found ahead of time for `key` (taken out: used once). While its
/// lookup still runs, waits for it up to 4 s instead of asking again.
async fn take_warm(warm: &WarmCache, key: &str) -> Option<Resolved> {
    for _ in 0..40 {
        let pending = {
            let mut w = warm.lock().unwrap_or_else(|e| e.into_inner());
            match w.get(key) {
                None => return None,
                Some((at, _)) if at.elapsed() > WARM_TTL => {
                    w.remove(key);
                    return None;
                }
                Some((_, Some(_))) => return w.remove(key).and_then(|(_, r)| r),
                Some((_, None)) => true,
            }
        };
        if pending {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    None
}

/// Result of the resolver for one track.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub track: Track,
    pub source: PlaybackSource,
    pub confidence: f32,
}

/// One line of import progress, e.g. `IMPORT spotify/playlist ██████░░ 98/142`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportProgress {
    pub label: String,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportInput {
    /// Playlist URL pasted into the search bar.
    Url(String),
    /// CSV / Spotify JSON export / M3U / M3U8 / XSPF dropped on the window.
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportMatch {
    pub original: Track,
    pub resolved: Option<Resolved>,
}

impl ImportMatch {
    /// The song to store in a playlist: the original metadata plus the id of
    /// the matched source in `external_ids` (so playback needs no new search).
    pub fn to_track(&self) -> Track {
        let mut t = self.original.clone();
        if let Some(r) = &self.resolved {
            if matches!(r.source, PlaybackSource::Preview { .. }) {
                return t;
            }
            let m = &r.track;
            if m.source != t.source {
                t.external_ids.insert(m.source, m.id.clone());
            }
            for (p, id) in &m.external_ids {
                if *p != t.source {
                    t.external_ids.entry(*p).or_insert_with(|| id.clone());
                }
            }
            if t.cover_url.is_none() {
                t.cover_url = m.cover_url.clone();
            }
            if t.duration_ms.is_none() {
                t.duration_ms = m.duration_ms;
            }
        }
        t
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportResult {
    pub name: String,
    /// Link the list came from (None for files); pass to `Library::save_import`
    /// so a re-import only adds new songs.
    pub origin: Option<String>,
    /// confidence >= 0.85
    pub sure: Vec<ImportMatch>,
    /// 0.5 ..< 0.85
    pub unsure: Vec<ImportMatch>,
    pub not_found: Vec<ImportMatch>,
    /// Every song in the list's own order: found ones carry the id of their
    /// source, the rest stay as metadata (found later, when played).
    pub ordered: Vec<Track>,
}

impl ImportResult {
    /// Tracks to store for the given matches, in order.
    pub fn tracks_of<'a>(matches: impl IntoIterator<Item = &'a ImportMatch>) -> Vec<Track> {
        matches.into_iter().map(ImportMatch::to_track).collect()
    }
}

struct Inner {
    config: Arc<Config>,
    mode: BackendMode,
    http: reqwest::Client,
    providers: Arc<Providers>,
    cooldowns: Arc<Cooldowns>,
    resolver: Arc<Resolver>,
    /// Only set in `BackendMode::Server`.
    server: Option<ServerClient>,
    server_providers: Option<Vec<ProviderId>>,
    covers: CoverCache,
    lyrics_cache: LyricsCache,
}

pub struct Backend {
    /// Current setup; replaced as a whole by `set_config`.
    inner: std::sync::RwLock<Arc<Inner>>,
    downloads: Arc<downloads::Downloads>,
    /// Audio files dropped onto playlists (same index format as downloads).
    imports: Arc<downloads::Downloads>,
    /// Recent search results (query -> when, songs): repeated searches do
    /// not hit the sources again for 10 minutes.
    search_cache: Arc<std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Vec<Track>)>>>,
    /// Sources found ahead of time (mouse resting on a song, top search
    /// hits): song key -> when, result. A click then starts without waiting
    /// for the resolve round trip. `None` while it is still being looked up.
    warm: WarmCache,
    /// Running downloads by song key, for the progress ring.
    dl_progress: Arc<std::sync::Mutex<std::collections::HashMap<String, Arc<downloads::Progress>>>>,
    download_queue: Arc<downloads::DownloadQueue>,
    rt: Option<tokio::runtime::Runtime>,
}

fn user_agent(config: &Config) -> String {
    format!("Glyph/{} (+https://github.com/{})", updater::current_version(), config.updates.repo)
}

impl Backend {
    /// Loads config side effects (cache dirs), checks `/v1/health` if a server
    /// is configured and decides the [`BackendMode`].
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("glyph-core")
            .enable_all()
            .build()?;
        let inner = rt.spawn(Inner::build(config, None)).await.map_err(|e| anyhow!("backend start failed: {e}"))??;
        let downloads = Arc::new(downloads::Downloads::load(storage::downloads_dir()));
        let imports = Arc::new(downloads::Downloads::load(storage::imports_dir()));
        let download_queue = Arc::new(downloads::DownloadQueue::default());
        download_queue.configure(if inner.config.storage.data_saver { 1 } else { inner.config.audio.download_parallelism }, inner.config.audio.download_adaptive);
        Ok(Self {
            download_queue,
            inner: std::sync::RwLock::new(Arc::new(inner)),
            downloads,
            imports,
            search_cache: Default::default(),
            warm: Default::default(),
            dl_progress: Default::default(),
            rt: Some(rt),
        })
    }

    /// Runs `fut` on the backend runtime; the returned future can be awaited
    /// from any executor (tokio's `JoinHandle` is executor-agnostic).
    fn spawn<T, F>(&self, fut: F) -> impl Future<Output = Result<T>> + 'static
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let handle = self.rt.as_ref().expect("runtime alive").spawn(fut);
        async move { handle.await.map_err(|e| anyhow!("backend task failed: {e}")) }
    }

    pub fn server_auth_required(&self) -> bool { self.cur().server.as_ref().is_some_and(|s| s.auth_required()) }
    pub fn server_media_credentials(&self) -> Option<(String,String)> { self.cur().server.as_ref().and_then(|s| s.media_credentials()) }

    pub fn mode(&self) -> BackendMode {
        self.cur().mode.clone()
    }

    /// Current config (after the latest [`Backend::set_config`]).
    pub fn config(&self) -> Arc<Config> {
        self.cur().config.clone()
    }

    fn cur(&self) -> Arc<Inner> {
        self.inner.read().unwrap().clone()
    }

    /// Applies a changed config without restarting: provider set, priority and
    /// server URL are re-evaluated (health check included); cooldowns and caches
    /// are kept. In-flight calls finish on the old setup.
    pub async fn set_config(&self, config: Config) -> anyhow::Result<()> {
        let old = self.cur();
        old.resolver.cache().flush();
        let cooldowns = old.cooldowns.clone();
        let inner = self.spawn(Inner::build(config, Some(cooldowns))).await??;
        self.download_queue.configure(if inner.config.storage.data_saver { 1 } else { inner.config.audio.download_parallelism }, inner.config.audio.download_adaptive);
        *self.inner.write().unwrap() = Arc::new(inner);
        self.warm.lock().unwrap_or_else(|e| e.into_inner()).clear();
        Ok(())
    }

    /// Unified search over all enabled providers, results arrive as they come
    /// in (already de-duplicated across providers). Ends with `SearchEvent::Done`.
    ///
    /// A song that was already sent by an earlier provider is sent again with
    /// the merged `external_ids` (same `source` + `id`): replace by that key.
    pub fn search(&self, query: &str) -> BoxStream<'static, SearchEvent> {
        self.search_page(query, 0)
    }

    /// [`Self::search`] for result page `page` (0 = first): further results
    /// while the user scrolls. Only sources that can page answer later pages.
    pub fn search_page(&self, query: &str, page: usize) -> BoxStream<'static, SearchEvent> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let inner = self.cur();
        let q = query.trim().to_string();
        let route = if inner.config.providers.local_search || inner.server.is_none() { "local" } else { "server" };
        let key = format!("{route}:{} #p{page}", q.to_lowercase());
        let cache = self.search_cache.clone();
        let hit = cache
            .lock()
            .unwrap()
            .get(&key)
            .filter(|(at, v)| at.elapsed() < std::time::Duration::from_secs(600) && !v.is_empty())
            .map(|(_, v)| v.clone());
        if let Some(tracks) = hit {
            let provider = tracks.first().map(|t| t.source).unwrap_or(ProviderId::Deezer);
            let _ = tx.send(SearchEvent::Results { provider, tracks });
            let _ = tx.send(SearchEvent::Done);
            return futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|v| (v, rx)) }).boxed();
        }
        // Collect what streams by, for the cache.
        let (tap_tx, mut tap_rx) = tokio::sync::mpsc::unbounded_channel::<SearchEvent>();
        self.rt.as_ref().expect("runtime alive").spawn(async move {
            let mut all: Vec<Track> = Vec::new();
            while let Some(ev) = tap_rx.recv().await {
                match ev {
                    SearchEvent::Results { tracks, .. } => {
                        for t in tracks {
                            match all.iter_mut().find(|x| x.source == t.source && x.id == t.id) {
                                Some(x) => *x = t,
                                None => all.push(t),
                            }
                        }
                    }
                    SearchEvent::Done => {
                        let mut c = cache.lock().unwrap();
                        if c.len() > 200 {
                            c.retain(|_, (at, _)| at.elapsed() < std::time::Duration::from_secs(600));
                        }
                        c.insert(key, (std::time::Instant::now(), all));
                        break;
                    }
                    _ => {}
                }
            }
        });
        let tx = {
            let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<SearchEvent>();
            let fwd = tx.clone();
            self.rt.as_ref().expect("runtime alive").spawn(async move {
                loop {
                    let ev = tokio::select! {
                        biased;
                        _ = fwd.closed() => break,
                        ev = out_rx.recv() => match ev { Some(ev) => ev, None => break },
                    };
                    let _ = tap_tx.send(ev.clone());
                    if fwd.send(ev).is_err() {
                        break;
                    }
                }
            });
            out_tx
        };
        self.rt.as_ref().expect("runtime alive").spawn(async move {
            let work = async {
                if let Some(server) = inner.server.as_ref().filter(|_| !inner.config.providers.local_search) {
                    match server.search_into(&q, page, &tx).await {
                        Ok(true) => return,
                        Ok(false) => {
                            let _ = tx.send(SearchEvent::Done);
                            return;
                        }
                        Err(e) => {
                            tracing::warn!("server search failed: {e}");
                            let _ = tx.send(SearchEvent::ProviderError { provider: ProviderId::Deezer, message: format!("server search failed: {e}") });
                            let _ = tx.send(SearchEvent::Done);
                            return;
                        },
                    }
                }
                let mut events = inner.resolver.search_stream(&q, page);
                while let Some(ev) = events.recv().await {
                    let done = ev == SearchEvent::Done;
                    if tx.send(ev).is_err() || done {
                        break;
                    }
                }
            };
            // Dropping the UI search stream aborts pending server requests too.
            tokio::select! { biased; _ = tx.closed() => {}, _ = work => {} }
        });
        futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|v| (v, rx)) }).boxed()
    }

    /// Best legal full source (or a preview) for a track. Downloaded and
    /// imported songs play from their local file.
    pub async fn resolve(&self, track: &Track) -> anyhow::Result<Option<Resolved>> {
        if let Some((path, provider)) = self.local_file(track) {
            let mime = Some(downloads::mime_of(&path).to_string());
            let source = PlaybackSource::Stream { provider, url: path.to_string_lossy().into_owned(), mime };
            return Ok(Some(Resolved { track: track.clone(), source, confidence: 1.0 }));
        }
        // Played lately: straight from the disk cache.
        if let Some(path) = stream_cached(track) {
            let mime = Some(downloads::mime_of(&path).to_string());
            let source = PlaybackSource::Stream { provider: track.source, url: path.to_string_lossy().into_owned(), mime };
            return Ok(Some(Resolved { track: track.clone(), source, confidence: 1.0 }));
        }
        let (inner, track, warm) = (self.cur(), track.clone(), self.warm.clone());
        self.spawn(async move {
            // An in-flight prefetch from before a mode switch must not send
            // a locally selected YouTube track back through the server.
            let mut r = if inner.config.providers.youtube_mode == config::YouTubeMode::Local {
                inner.resolve(&track).await?
            } else { match take_warm(&warm, &dedupe::track_key(&track)).await {
                Some(r) => Some(r),
                None => inner.resolve(&track).await?,
            }};
            if let (Some(server), Some(resolved)) = (&inner.server, r.as_mut()) {
                if let PlaybackSource::Stream { url, .. } | PlaybackSource::Preview { url, .. } = &mut resolved.source {
                    *url = server.audio_quality_url(url, inner.config.audio.quality);
                }
            }
            // Keep a copy of what plays now for replays (YouTube has its own cache).
            if let Some(Resolved { source: PlaybackSource::Stream { url, mime, .. }, .. }) = &r {
                if url.starts_with("http") && !url.contains("/v1/audio/")
                    && inner.config.audio.quality != config::AudioQuality::Auto
                    && !inner.config.storage.data_saver
                    && glyph_protocol::network::class() == glyph_protocol::network::Class::Fast {
                    tokio::spawn(cache_stream(inner.http.clone(), track.clone(), url.clone(), mime.clone()));
                }
            }
            Ok::<_, anyhow::Error>(r)
        })
        .await?
    }

    /// Looks up the source of `tracks` in the background so that playing
    /// them starts right away (fire and forget; at most 3 at once).
    pub fn warm(&self, tracks: Vec<Track>) {
        let Some(rt) = self.rt.as_ref() else { return };
        for t in tracks.into_iter().filter(|t| self.local_file(t).is_none()).take(3) {
            let key = dedupe::track_key(&t);
            {
                let mut w = self.warm.lock().unwrap_or_else(|e| e.into_inner());
                if w.get(&key).is_some_and(|(at, _)| at.elapsed() < WARM_TTL) {
                    continue;
                }
                w.retain(|_, (at, _)| at.elapsed() < WARM_TTL);
                w.insert(key.clone(), (std::time::Instant::now(), None));
            }
            let (inner, cache) = (self.cur(), self.warm.clone());
            rt.spawn(async move {
                let r = inner.resolve(&t).await.ok().flatten();
                if !inner.config.storage.data_saver {
                    if let Some(Resolved { source: PlaybackSource::Stream { url, .. }, .. }) = &r {
                        if url.starts_with("https://") || url.starts_with("http://") {
                            let url = inner.server.as_ref().map(|s| s.audio_quality_url(url, inner.config.audio.quality)).unwrap_or_else(|| url.clone());
                            let warmer = STREAM_WARMER.read().unwrap().clone();
                            if let Some(warmer) = warmer { warmer(url); }
                        }
                    }
                }
                let mut w = cache.lock().unwrap_or_else(|e| e.into_inner());
                match r {
                    // Previews carry signed URLs that die fast: not kept.
                    Some(r) if !matches!(r.source, PlaybackSource::Preview { .. }) => {
                        w.insert(key, (std::time::Instant::now(), Some(r)));
                    }
                    _ => {
                        w.remove(&key);
                    }
                }
            });
        }
    }

    /// A fresh 30 s preview straight from Deezer (signed URLs expire after
    /// minutes; a cached one from a server can already be dead).
    pub async fn refresh_preview(&self, track: &Track) -> anyhow::Result<Option<Resolved>> {
        let (inner, track) = (self.cur(), track.clone());
        self.spawn(async move { Ok(inner.resolver.resolve_preview(&track).await) }).await?
    }

    /// Resolves again with fresh stream URLs (after an HTTP 403 from an
    /// expired YouTube URL).
    pub async fn refresh_source(&self, track: &Track) -> anyhow::Result<Option<Resolved>> {
        ytdlp::forget_urls();
        self.resolve(track).await
    }

    /// YouTube plays through a local yt-dlp (prefetching is worth it).
    pub fn local_youtube(&self) -> bool {
        self.cur().local_youtube_on()
    }

    /// Resolves the next songs in the background so they start without
    /// waiting for yt-dlp (fire and forget; at most [`ytdlp::MAX_BATCH`]).
    /// Songs already downloaded are skipped.
    pub fn prefetch(&self, tracks: Vec<Track>) {
        let inner = self.cur();
        if !inner.local_youtube_on() {
            return;
        }
        let tracks: Vec<Track> =
            tracks.into_iter().filter(|t| self.local_file(t).is_none()).take(ytdlp::MAX_BATCH).collect();
        if tracks.is_empty() {
            return;
        }
        if let Some(rt) = self.rt.as_ref() {
            rt.spawn(async move { inner.prefetch(tracks).await });
        }
    }

    /// yt-dlp found / version / JS runtime, for the settings page.
    pub async fn ytdlp_status(&self) -> anyhow::Result<ytdlp::YtDlpStatus> {
        let configured = self.cur().config.providers.ytdlp_path.clone();
        let http = self.cur().http.clone();
        self.spawn(async move {
            tokio::time::timeout(std::time::Duration::from_secs(15), ytdlp::status_with_updates(configured.as_deref(), &http))
                .await.map_err(|_| anyhow!("yt-dlp did not respond within 15 seconds. Try again or reinstall yt-dlp."))
        }).await?
    }

    /// Downloads the official yt-dlp release for this OS into the data dir
    /// (SHA-256 checked) and re-evaluates the providers so YouTube picks it up.
    pub async fn install_ytdlp(&self) -> anyhow::Result<ytdlp::YtDlpStatus> {
        let inner = self.cur();
        self.spawn(async move { ytdlp::install(&inner.http).await }).await??;
        self.set_config(Config::clone(&self.config())).await?;
        self.ytdlp_status().await
    }

    /// Fans + verified profile of an artist.
    pub async fn artist_stats(&self, name: &str) -> anyhow::Result<Option<catalog::ArtistStats>> {
        let (inner, name) = (self.cur(), name.to_string());
        self.spawn(async move { catalog::artist_stats(&inner.http, &name).await }).await?
    }

    /// An artist's whole discography (Deezer catalog), newest first.
    pub async fn artist_releases(&self, name: &str) -> anyhow::Result<Vec<providers::deezer::Release>> {
        let (inner, name) = (self.cur(), name.to_string());
        self.spawn(async move {
            let d = match &inner.providers.deezer {
                Some(d) => d.clone(),
                None => Arc::new(providers::deezer::Deezer::new(inner.http.clone())),
            };
            d.artist_releases(&name).await
        })
        .await?
    }

    /// Releases of `artists` that came out in the last `days` days, newest
    /// first (Deezer catalog). Artists are asked one after another with a
    /// short pause, so a long follow list stays under the catalog's rate limit.
    pub async fn new_releases(&self, artists: Vec<String>, days: u32) -> anyhow::Result<Vec<providers::deezer::Release>> {
        let inner = self.cur();
        self.spawn(async move {
            let d = match &inner.providers.deezer {
                Some(d) => d.clone(),
                None => Arc::new(providers::deezer::Deezer::new(inner.http.clone())),
            };
            let (since, today) = (catalog::date_days_ago(days), catalog::date_days_ago(0));
            let mut out: Vec<providers::deezer::Release> = Vec::new();
            for name in artists {
                if let Ok(list) = d.artist_releases(&name).await {
                    out.extend(list.into_iter().filter(|r| r.released.as_deref().is_some_and(|day| day >= since.as_str() && day <= today.as_str())));
                }
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            }
            out.sort_by(|a, b| b.released.cmp(&a.released));
            out.dedup_by(|a, b| a.id == b.id);
            Ok(out)
        })
        .await?
    }

    /// The songs of one release.
    pub async fn release_tracks(&self, release: &providers::deezer::Release) -> anyhow::Result<Vec<Track>> {
        let (inner, release) = (self.cur(), release.clone());
        self.spawn(async move {
            let d = match &inner.providers.deezer {
                Some(d) => d.clone(),
                None => Arc::new(providers::deezer::Deezer::new(inner.http.clone())),
            };
            d.album_tracks(&release).await
        })
        .await?
    }

    /// Release year + type (album, EP, single) of an album.
    pub async fn album_meta(&self, title: &str, artist: &str) -> anyhow::Result<Option<catalog::AlbumMeta>> {
        let (inner, title, artist) = (self.cur(), title.to_string(), artist.to_string());
        self.spawn(async move { catalog::album_meta(&inner.http, &title, &artist).await }).await?
    }

    /// Songs popular right now (public Deezer chart, cached by the caller).
    pub async fn trending(&self, limit: usize) -> anyhow::Result<Vec<Track>> {
        let inner = self.cur();
        self.spawn(async move {
            let d = match &inner.providers.deezer {
                Some(d) => d.clone(),
                None => Arc::new(providers::deezer::Deezer::new(inner.http.clone())),
            };
            d.chart(limit).await
        })
        .await?
    }

    /// Saves `track` for offline use if a source allows it. `Ok(None)`: no
    /// source offers a download for this song.
    ///
    /// Uses `audio.download_quality`; [`Backend::download_progress`] follows it.
    pub async fn download(&self, track: &Track) -> anyhow::Result<Option<PathBuf>> {
        if let Some((p, _)) = self.local_file(track) {
            return Ok(Some(p));
        }
        let (inner, dl, track) = (self.cur(), self.downloads.clone(), track.clone());
        let job = downloads::Job { progress: Arc::default(), quality: inner.config.audio.download_quality.effective() };
        let key = dedupe::track_key(&track);
        self.dl_progress.lock().unwrap().insert(key.clone(), job.progress.clone());
        let queue = self.download_queue.clone();
        let res = self
            .spawn(downloads::run_job(job, async move {
                let _permit = queue.acquire().await;
                let Some((provider, url)) = inner.download_url(&track).await? else { return Ok(None) };
                // YouTube songs already sit in the audio cache: copy the file.
                let local = std::path::Path::new(&url);
                if provider == ProviderId::YouTube && local.is_absolute() && local.is_file() {
                    let stored = dl.store_copy(&track, provider, local);
                    // Data-saver files are only fetched for this download.
                    if local.starts_with(ytdlp::small_audio_dir()) {
                        let _ = std::fs::remove_file(local);
                    }
                    return stored.map(Some);
                }
                if provider == ProviderId::Jamendo {
                    if let Some(alt) = downloads::jamendo_format_url(&url, downloads::job_quality()) {
                        match dl.fetch(&inner.http, &track, provider, &alt, inner.server.as_ref()).await {
                            Ok(p) => return Ok(Some(p)),
                            Err(e) => tracing::info!("Jamendo {alt} failed, taking the MP3: {e:#}"),
                        }
                        downloads::report(|p| p.reset());
                    }
                }
                downloads::fetch_with_retry(&dl, &inner.http, &track, provider, &url, inner.server.as_ref()).await.map(Some)
            }))
            .await;
        self.dl_progress.lock().unwrap().remove(&key);
        res?
    }

    /// `None` when no download of `track` runs; else 0..=1, or `Some(None)`
    /// while its size is unknown (yt-dlp still resolving).
    pub fn download_progress(&self, track: &Track) -> Option<Option<f32>> {
        let map = self.dl_progress.lock().unwrap();
        if map.is_empty() {
            return None;
        }
        map.get(&dedupe::track_key(track)).map(|p| p.fraction())
    }

    /// Index lookup only (cheap enough for every rendered row). Imported
    /// files count as downloaded.
    pub fn is_downloaded(&self, track: &Track) -> bool {
        self.downloads.contains(track) || self.imports.contains(track)
    }

    /// Slow maintenance stays entirely separate from the local playback path.
    pub fn refresh_downloads(&self) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static RUNNING: AtomicBool = AtomicBool::new(false);
        let Some(rt) = &self.rt else { return };
        let inner = self.cur();
        if inner.config.storage.data_saver || RUNNING.swap(true, Ordering::SeqCst) { return; }
        let downloads = self.downloads.clone();
        rt.spawn(async move {
            struct Running;
            impl Drop for Running { fn drop(&mut self) { RUNNING.store(false, Ordering::SeqCst); } }
            let _running = Running;
            for entry in downloads.claim_refresh(2) {
                let job = downloads::Job { progress: Arc::default(), quality: entry.revision.quality };
                downloads::run_job(job, async {
                    if let Ok(Some((provider, url))) = inner.download_url(&entry.track).await {
                        if provider == entry.provider && url.starts_with("https://") {
                            if let Err(e) = downloads.refresh(&inner.http, entry, &url, inner.server.as_ref()).await {
                                tracing::debug!("download freshness check deferred: {e}");
                            }
                        }
                    }
                }).await;
            }
        });
    }

    /// The downloaded or imported file of `track`.
    fn local_file(&self, track: &Track) -> Option<(PathBuf, ProviderId)> {
        self.downloads.local(track).or_else(|| self.imports.local(track))
    }

    /// Imports dropped audio files / folders (tags, embedded cover, copy
    /// into the imports folder). The songs come back in drop order.
    pub async fn import_local(&self, paths: Vec<PathBuf>) -> anyhow::Result<import::local::LocalImport> {
        let imports = self.imports.clone();
        self.spawn(async move {
            tokio::task::spawn_blocking(move || import::local::import_files(&imports, &paths)).await
        })
        .await?
        .map_err(|e| anyhow!("import failed: {e}"))
    }

    /// Downloaded songs, newest first.
    pub fn downloaded_tracks(&self) -> Vec<Track> {
        self.downloads.tracks()
    }

    pub fn downloaded_bytes(&self, tracks: &[Track]) -> u64 { self.downloads.saved_bytes(tracks) }

    pub fn remove_download(&self, track: &Track) -> bool {
        self.downloads.remove(track)
    }

    /// Drops the local file a song plays from after it failed to decode
    /// (truncated or damaged): its download, else its copy in the cache of
    /// songs played lately. Imported files are the user's own and stay.
    /// `None`: the song has no such file.
    pub fn forget_broken_file(&self, track: &Track) -> Option<BrokenFile> {
        if self.downloads.local(track).is_some() {
            return self.downloads.remove(track).then_some(BrokenFile::Download);
        }
        if self.imports.local(track).is_some() {
            return None;
        }
        let p = stream_cached(track)?;
        std::fs::remove_file(&p).ok().map(|_| BrokenFile::CachedStream)
    }

    /// Re-reads the download index (after the folder was cleared).
    pub fn reload_downloads(&self) {
        self.downloads.reload();
    }

    pub async fn lyrics(&self, track: &Track) -> anyhow::Result<Option<Lyrics>> {
        let (inner, track) = (self.cur(), track.clone());
        self.spawn(async move { inner.lyrics(&track).await }).await?
    }

    /// Songs whose lyrics contain `query` (a remembered line), with the line.
    pub async fn search_lyrics(&self, query: &str) -> anyhow::Result<Vec<lyricsearch::LyricHit>> {
        let (inner, q) = (self.cur(), query.trim().to_string());
        self.spawn(async move {
            let d = match &inner.providers.deezer {
                Some(d) => d.clone(),
                None => Arc::new(providers::deezer::Deezer::new(inner.http.clone())),
            };
            lyricsearch::search(&inner.http, &d, &q, 5).await
        })
        .await?
    }

    /// Other versions of a song's lyrics (LRCLIB), best match first, for when
    /// the shown ones are wrong.
    pub async fn lyrics_candidates(&self, track: &Track) -> anyhow::Result<Vec<Lyrics>> {
        let (inner, track) = (self.cur(), track.clone());
        self.spawn(async move {
            let l = match &inner.providers.lrclib {
                Some(l) => l.clone(),
                None => Arc::new(providers::lrclib::LrcLib::new(inner.http.clone())),
            };
            l.candidates(&track.title, &track.artist, track.duration_ms).await
        })
        .await?
    }

    /// The user picked these lyrics for `track`: they stay its lyrics.
    pub fn pin_lyrics(&self, track: &Track, lyrics: &Lyrics) {
        self.cur().lyrics_cache.put(&lyrics::cache_key(track), Some(lyrics));
    }

    /// Bio + photo for the artist page (Wikipedia in `lang`, then English).
    pub async fn artist_info(&self, name: &str, lang: &str) -> anyhow::Result<Option<artist::ArtistInfo>> {
        let (inner, name, lang) = (self.cur(), name.to_string(), lang.to_string());
        self.spawn(async move { artist::lookup_via(&inner.http, &name, &lang, inner.server.as_ref()).await }).await?
    }

    pub async fn artist_photo(&self, name: &str, lang: &str) -> anyhow::Result<Option<PathBuf>> {
        let (inner, name, lang) = (self.cur(), name.to_string(), lang.to_string());
        self.spawn(async move {
            static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
            let _permit = SLOTS.acquire().await?;
            let catalog = artist::portrait(&inner.http, &name).await;
            if matches!(&catalog, Ok(Some(_))) { return catalog; }
            let fallback = artist::lookup_via(&inner.http, &name, &lang, inner.server.as_ref()).await?.and_then(|i| i.image);
            if fallback.is_some() { return Ok(fallback); }
            catalog
        }).await?
    }

    /// Whether radio can actually deliver something. With a server the single
    /// source of truth is its `/v1/similar` (a wrong or missing Last.fm key
    /// leaves it empty, and the UI greys the switch out); without one the app's
    /// own providers answer.
    pub async fn radio_available(&self) -> anyhow::Result<bool> {
        let inner = self.cur();
        if inner.server.is_none() {
            // Own Last.fm key or the Deezer related-artist fallback.
            return Ok(true);
        }
        self.spawn(async move {
            let Some(server) = &inner.server else { return true };
            // Canonical, always-known track: an empty answer means the server
            // side of radio is not configured.
            matches!(server.similar("Daft Punk", "Get Lucky").await, Ok(r) if !r.tracks.is_empty())
        })
        .await
    }

    /// Radio: similar tracks (Last.fm track.getSimilar), not yet resolved.
    pub async fn similar(&self, track: &Track) -> anyhow::Result<Vec<Track>> {
        let (inner, track) = (self.cur(), track.clone());
        self.spawn(async move {
            // Server (Last.fm there) -> own Last.fm key -> Deezer related
            // artists (needs no key), so radio always has something.
            if let Some(server) = &inner.server {
                match server.similar(&track.artist, &track.title).await {
                    Ok(r) if !r.tracks.is_empty() => return Ok(r.tracks),
                    Ok(_) => {}
                    Err(e) => tracing::info!("server similar unavailable, falling back: {e:#}"),
                }
            }
            if let Some(lf) = &inner.providers.lastfm {
                match lf.similar(&track.artist, &track.title).await {
                    Ok(v) if !v.is_empty() => return Ok(v),
                    Ok(_) => {}
                    Err(e) => tracing::info!("Last.fm similar failed, falling back: {e:#}"),
                }
            }
            let d = match &inner.providers.deezer {
                Some(d) => d.clone(),
                None => Arc::new(providers::deezer::Deezer::new(inner.http.clone())),
            };
            d.related_tracks(&track).await
        })
        .await?
    }

    /// Artists similar to `artist`: the app's own Last.fm key first, then
    /// Deezer's related artists (needs no key). Empty when nothing is known.
    pub async fn similar_artists(&self, artist: &str) -> anyhow::Result<Vec<String>> {
        let (inner, artist) = (self.cur(), artist.to_string());
        self.spawn(async move {
            if let Some(lf) = &inner.providers.lastfm {
                match lf.similar_artists(&artist).await {
                    Ok(v) if !v.is_empty() => return Ok(v),
                    Ok(_) => {}
                    Err(e) => tracing::info!("Last.fm similar artists failed, falling back: {e:#}"),
                }
            }
            let d = match &inner.providers.deezer {
                Some(d) => d.clone(),
                None => Arc::new(providers::deezer::Deezer::new(inner.http.clone())),
            };
            d.related_artists(&artist).await
        })
        .await?
    }

    /// Local path of the (cached, size-normalised) cover, following the
    /// fallback chain provider → Spotify/Deezer → Cover Art Archive.
    /// `None` = UI draws a generated ASCII pattern.
    pub async fn cover(&self, track: &Track) -> anyhow::Result<Option<PathBuf>> {
        if self.imports.contains(track) {
            let p = import::local::cover_path(&self.imports, track);
            if p.is_file() {
                return Ok(Some(p));
            }
        }
        let (inner, track) = (self.cur(), track.clone());
        self.spawn(async move { inner.covers.get(&track).await }).await?
    }

    pub async fn import(
        &self,
        input: ImportInput,
        progress: flume_like::Sender<ImportProgress>,
    ) -> anyhow::Result<ImportResult> {
        let inner = self.cur();
        self.spawn(async move {
            if let Some(server) = &inner.server {
                if let Ok(health) = server.health().await {
                    library::set_playlist_track_limit(health.playlist_max_tracks);
                }
            }
            let (list, origin) = import::fetch_list_with_progress(&input, &inner.providers, inner.server.as_ref(), &inner.http, &inner.config.keys, Some(&progress)).await?;
            if list.tracks.len() > library::playlist_track_limit() {
                anyhow::bail!("Playlists may contain at most {} songs", library::playlist_track_limit());
            }
            let worker = inner.clone();
            let result = import::match_all(list, origin, progress, move |t| {
                let inner = worker.clone();
                async move { inner.resolve(&t).await.ok().flatten() }
            })
            .await;
            inner.resolver.cache().flush();
            Ok(result)
        })
        .await?
    }

    /// The library of a connected service (`keys`: the app's current keys,
    /// which may be newer than the backend's).
    pub fn library_source(
        &self,
        service: import::accounts::Service,
        login: import::accounts::Login,
        keys: config::ApiKeys,
    ) -> anyhow::Result<Arc<dyn import::accounts::LibrarySource>> {
        let inner = self.cur();
        let ctx = import::accounts::SourceCtx {
            http: inner.http.clone(),
            keys,
            lastfm: inner.providers.lastfm.clone(),
            deezer: inner.providers.deezer.clone(),
            reader: Arc::new(InnerReader(inner.clone())),
        };
        import::accounts::source(service, login, ctx)
    }

    /// Song lists in pasted text (many links copied in Spotify & co.):
    /// (single songs, lists). Used to tell a pasted link list from a search.
    pub fn pasted_links(text: &str) -> (usize, usize) {
        let p = import::keyless::parse_pasted(text);
        (p.songs.len(), p.lists.len())
    }

    /// What the connected account holds (counts, playlists).
    pub async fn library_overview(&self, src: Arc<dyn import::accounts::LibrarySource>) -> anyhow::Result<import::accounts::Overview> {
        self.spawn(async move { src.overview().await }).await?
    }

    /// Moves the picked collections one after the other: reads each, matches
    /// its songs (4 at a time, like a link import) and reports it as
    /// `MigrationEvent::Done` for the app to save. `cancel` stops after the
    /// current song; a cancelled collection is not reported as done.
    pub async fn migrate(
        &self,
        src: Arc<dyn import::accounts::LibrarySource>,
        collections: Vec<import::accounts::Collection>,
        events: flume_like::Sender<import::accounts::MigrationEvent>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    ) -> anyhow::Result<()> {
        use import::accounts::{Fetched, Kind, MigrationEvent as Ev, Migrated};
        use std::sync::atomic::Ordering;
        let inner = self.cur();
        self.spawn(async move {
            if let Some(server) = &inner.server {
                if let Ok(health) = server.health().await {
                    library::set_playlist_track_limit(health.playlist_max_tracks);
                }
            }
            let total = collections.len();
            let stopped = || cancel.load(Ordering::Relaxed);
            for (index, c) in collections.into_iter().enumerate() {
                if stopped() {
                    let _ = events.send(Ev::Finished { cancelled: true });
                    return;
                }
                let _ = events.send(Ev::Collection { index, total, collection: c.clone() });
                let result = match src.fetch(&c).await {
                    Err(e) => {
                        let _ = events.send(Ev::Failed { collection: c, error: format!("{e:#}") });
                        continue;
                    }
                    Ok(fetched @ (Fetched::Tracks(_) | Fetched::Named(..))) => {
                        let (name, tracks) = match fetched {
                            Fetched::Named(name, tracks) if c.name.trim().is_empty() => (name, tracks),
                            Fetched::Named(_, tracks) | Fetched::Tracks(tracks) => (c.name.clone(), tracks),
                            _ => unreachable!(),
                        };
                        if c.kind != Kind::Liked && tracks.len() > library::playlist_track_limit() {
                            let _ = events.send(Ev::Failed { collection: c, error: format!("Playlists may contain at most {} songs", library::playlist_track_limit()) });
                            continue;
                        }
                        let (ptx, mut prx) = flume_like::channel::<ImportProgress>();
                        let fwd_events = events.clone();
                        let fwd = tokio::spawn(async move {
                            while let Some(p) = prx.recv().await {
                                let _ = fwd_events.send(Ev::Progress { done: p.done, total: p.total });
                            }
                        });
                        let kind = match c.kind {
                            Kind::Liked => "liked",
                            _ => "playlist",
                        };
                        let list = import::files::ParsedList { name, label: format!("{}/{kind}", src.service().key()), tracks };
                        let (worker, flag) = (inner.clone(), cancel.clone());
                        let res = import::match_all(list, c.origin.clone(), ptx, move |t| {
                            let (inner, flag) = (worker.clone(), flag.clone());
                            async move {
                                if flag.load(Ordering::Relaxed) {
                                    return None;
                                }
                                inner.resolve(&t).await.ok().flatten()
                            }
                        })
                        .await;
                        let _ = fwd.await;
                        inner.resolver.cache().flush();
                        if stopped() {
                            let _ = events.send(Ev::Finished { cancelled: true });
                            return;
                        }
                        Migrated::Tracks(res)
                    }
                    Ok(Fetched::Albums(a)) => Migrated::Albums(a),
                    Ok(Fetched::Artists(a)) => Migrated::Artists(a),
                };
                let _ = events.send(Ev::Done { collection: c, result });
            }
            let _ = events.send(Ev::Finished { cancelled: false });
        })
        .await
    }

    /// Provider is currently skipped (error / exhausted quota, 10 min).
    pub fn provider_cooling_down(&self, provider: ProviderId) -> bool {
        self.cur().cooldowns.is_cooling(provider)
    }

    /// Providers that are active (enabled, keys present).
    pub fn active_providers(&self) -> Vec<ProviderId> {
        let inner = self.cur();
        inner.server_providers.clone().unwrap_or_else(|| inner.providers.ids())
    }

    /// Changes the full-source priority for the running session.
    pub fn set_priority(&self, priority: Vec<ProviderId>) {
        self.cur().resolver.set_priority(priority);
    }

    /// Connection test for the settings page (`GET /v1/health` of any URL).
    pub async fn test_server(&self, url: &str) -> anyhow::Result<Compat> {
        let client = ServerClient::new(url, self.cur().http.clone(), server_client::install_id());
        self.spawn(async move { client.check_compat().await }).await?
    }

    /// Checks GitHub releases (see [`updater`]) right now: the manual "Check
    /// for updates" action, never returns `Skipped`.
    pub async fn check_for_update(&self) -> anyhow::Result<UpdateStatus> {
        let inner = self.cur();
        self.spawn(async move { updater::check(&inner.http, &inner.config.updates, false).await }).await?
    }

    /// Start-up check: honours `check_on_start` and the 24 h throttle (`Skipped`).
    pub async fn check_for_update_auto(&self) -> anyhow::Result<UpdateStatus> {
        let inner = self.cur();
        self.spawn(async move { updater::check(&inner.http, &inner.config.updates, true).await }).await?
    }

    /// Downloads, verifies (SHA-256) and installs the update. Restart is up to the UI.
    pub async fn install_update(
        &self,
        info: UpdateInfo,
        progress: flume_like::Sender<(u64, u64)>,
    ) -> anyhow::Result<()> {
        let inner = self.cur();
        self.spawn(async move { updater::install(&inner.http, &info, progress).await }).await?
    }

    /// Persists caches (also done on drop).
    pub fn flush(&self) {
        self.cur().resolver.cache().flush();
    }

    /// Clears in-memory catalog caches after `StorageKind::Catalog` was cleared on disk.
    pub fn clear_catalog_caches(&self) {
        self.cur().resolver.cache().clear();
        self.search_cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.warm.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// Reads links and files for the library move like a pasted link import.
struct InnerReader(Arc<Inner>);

#[async_trait::async_trait]
impl import::accounts::LinkReader for InnerReader {
    async fn list(&self, input: ImportInput) -> anyhow::Result<import::files::ParsedList> {
        let i = &self.0;
        Ok(import::fetch_list(&input, &i.providers, i.server.as_ref(), &i.http, &i.config.keys).await?.0)
    }

    async fn songs(&self, songs: Vec<import::keyless::SongLink>) -> Vec<Track> {
        let i = &self.0;
        let dz = i.providers.deezer.clone().unwrap_or_else(|| Arc::new(providers::deezer::Deezer::new(i.http.clone())));
        import::keyless::read_songs(&songs, i.providers.spotify.as_ref(), &dz, &i.http).await
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.cur().resolver.cache().flush();
        if let Some(rt) = self.rt.take() {
            // Safe even if dropped inside an async context.
            rt.shutdown_background();
        }
    }
}

/// Progress channel type used across the API (re-export so the UI does not
/// need to pick a channel crate). Implemented with `tokio::sync::mpsc::UnboundedSender`.
pub mod flume_like {
    pub type Sender<T> = tokio::sync::mpsc::UnboundedSender<T>;
    pub type Receiver<T> = tokio::sync::mpsc::UnboundedReceiver<T>;
    pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
        tokio::sync::mpsc::unbounded_channel()
    }
}

#[cfg(test)]
mod tests;
