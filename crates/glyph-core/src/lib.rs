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
pub mod library;
pub mod lyrics;
pub mod paths;
pub mod profile_images;
pub mod provider;
pub mod providers;
pub mod queue;
pub mod resolver;
pub mod server_client;
pub mod server_access;
pub mod settings_share;
pub mod settings_sync;
pub mod share;
pub mod sync;
pub mod storage;
pub mod taste;
pub mod updater;
pub mod ytdlp;

pub use config::Config;
pub use library::{FollowedArtist, Library, ListKey, Playlist, SearchItem};
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
    rt: Option<tokio::runtime::Runtime>,
}

fn user_agent(config: &Config) -> String {
    format!("Glyph/{} (+https://github.com/{})", updater::current_version(), config.updates.repo)
}

impl Inner {
    async fn build(config: Config, old_cooldowns: Option<Arc<Cooldowns>>) -> Result<Inner> {
        let _ = std::fs::create_dir_all(paths::data_dir());
        let http = reqwest::Client::builder()
            .user_agent(user_agent(&config))
            .connect_timeout(std::time::Duration::from_secs(8))
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        let providers = Arc::new(Providers::build(&config, &http));
        let cooldowns = old_cooldowns.unwrap_or_else(|| Arc::new(Cooldowns::new()));
        let resolver = Arc::new(Resolver::new(
            providers.clone(),
            cooldowns.clone(),
            config.providers.priority.clone(),
            Some(paths::data_dir().join("resolver_cache.json")),
        ));
        let limit = match config.storage.cache_limit_mb {
            0 => u64::MAX,
            mb => mb * 1024 * 1024,
        };
        let covers = CoverCache::new(storage::covers_dir(), http.clone(), providers.clone()).with_max_bytes(limit);
        let lyrics_cache = LyricsCache::new(paths::data_dir().join("lyrics"));

        let mut mode = BackendMode::Direct;
        let mut server = None;
        let mut server_providers = None;
        let url = server_client::normalize_base_url(config.effective_server_url());
        if !url.is_empty() {
            let key_url = url.clone();
            let token = tokio::task::spawn_blocking(move || server_access::load(&key_url)).await.ok().flatten();
            let client = ServerClient::new(&url, http.clone(), server_client::install_id()).with_bearer(token)?;
            match client.check_compat().await {
                Ok(Compat::Compatible(h)) => {
                    server_providers = Some(h.providers);
                    mode = BackendMode::Server { url, server_version: h.server_version };
                    server = Some(client);
                }
                Ok(Compat::Incompatible(h)) => {
                    tracing::warn!(
                        server = h.protocol_version,
                        ours = glyph_protocol::PROTOCOL_VERSION,
                        "incompatible server"
                    );
                    return Err(anyhow!("incompatible server protocol {} (expected {})", h.protocol_version, glyph_protocol::PROTOCOL_VERSION));
                }
                Err(e) => {
                    tracing::warn!("server {url} unreachable: {e}");
                    server_providers = Some(vec![]);
                    mode = BackendMode::Server { url, server_version: "offline".into() };
                    server = Some(client);
                },
            }
        }
        let covers = covers.with_server(server.clone());
        Ok(Inner { config: Arc::new(config), mode, http, providers, cooldowns, resolver, server, server_providers, covers, lyrics_cache })
    }

    /// YouTube is enabled and yt-dlp runs on this machine.
    fn local_youtube_on(&self) -> bool {
        (self.config.providers.youtube_mode == config::YouTubeMode::Local || self.server.is_none())
            && self.providers.youtube.as_ref().is_some_and(|y| y.local().is_some())
    }

    /// YouTube through the local yt-dlp (home IP, no bot check): the song's
    /// own video id, a cached mapping, the id other users shared through the
    /// server (`hint`), else a search for "Artist - Title" — a sure search
    /// hit is shared back so the next user skips the search.
    async fn local_youtube(&self, t: &Track, hint: Option<&str>) -> Option<Resolved> {
        if !self.local_youtube_on() { return None; }
        let yt = self.providers.youtube.as_ref().filter(|y| y.local().is_some())?;
        let as_video = |id: &str| {
            let mut view = t.clone();
            view.source = ProviderId::YouTube;
            view.id = id.to_string();
            view
        };
        let (matched, confidence) = if let Some(id) = provider::id_at(ProviderId::YouTube, t) {
            (as_video(id), 1.0)
        } else if let Some(hit) = self.resolver.cached_at(t, ProviderId::YouTube) {
            hit
        } else if let Some(id) = hint.filter(|id| ytdlp::valid_video_id(id)) {
            (as_video(id), resolver::SURE_CONFIDENCE)
        } else {
            let query = format!("{} - {}", t.artist, t.title);
            let cands = match provider::MusicProvider::search_candidates(yt.as_ref(), &query).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::info!("local YouTube search failed: {e:#}");
                    return None;
                }
            };
            let (best, confidence) = resolver::best_candidate(t, cands)?;
            self.resolver.remember(t, &best.track, confidence);
            if confidence >= resolver::SURE_CONFIDENCE {
                if let Some(server) = self.server.clone() {
                    let (want, id) = (t.clone(), best.track.id.clone());
                    tokio::spawn(async move {
                        if let Err(e) = server.report_youtube_hint(&want, &id).await {
                            tracing::debug!("sharing the YouTube id failed: {e:#}");
                        }
                    });
                }
            }
            (best.track, confidence)
        };
        match yt.local_source(&matched.id).await {
            Ok(source) => {
                // Keep the wanted song's metadata when it was a direct id hit.
                let track = if confidence >= 1.0 && t.source != ProviderId::YouTube { t.clone() } else { matched };
                Some(Resolved { track, source, confidence })
            }
            Err(e) => {
                tracing::info!("local YouTube playback failed: {e:#}");
                None
            }
        }
    }

    /// Warms the yt-dlp URL cache for songs that will play soon: songs whose
    /// YouTube video is already known resolve together in one yt-dlp run,
    /// the others go through the normal resolve (which caches what it finds).
    async fn prefetch(&self, tracks: Vec<Track>) {
        if self.server.is_some() {
            for t in tracks { let _ = self.resolve(&t).await; }
            return;
        }
        let Some(yt) = self.providers.youtube.as_ref().and_then(|y| y.local()) else { return };
        let mut ids = Vec::new();
        let mut rest = Vec::new();
        for t in tracks {
            // Only when YouTube is surely what plays (its own songs or an
            // earlier YouTube match); anything else asks the resolver.
            let known = if t.source == ProviderId::YouTube {
                Some(t.id.clone())
            } else {
                self.resolver.cached_at(&t, ProviderId::YouTube).map(|(m, _)| m.id)
            };
            match known {
                Some(id) => ids.push(id),
                None => rest.push(t),
            }
        }
        if let Err(e) = yt.resolve_many(&ids).await {
            tracing::debug!("prefetch batch failed: {e:#}");
        }
        // The URLs are cached now; fetch the audio so the songs start at once.
        for id in &ids {
            if let Err(e) = yt.audio_file(&self.http, id).await {
                tracing::debug!(video_id = %id, "prefetch download failed: {e:#}");
            }
        }
        for t in rest {
            let _ = self.resolve(&t).await;
        }
    }

    async fn resolve(&self, t: &Track) -> Result<Option<Resolved>> {
        if self.config.providers.youtube_mode == config::YouTubeMode::Local
            && self.config.providers.enabled.contains(&ProviderId::YouTube)
            && (t.source == ProviderId::YouTube || self.resolver.priority().first() == Some(&ProviderId::YouTube))
        {
            return self.local_youtube(t, None).await.map(Some)
                .ok_or_else(|| anyhow!("Local YouTube playback failed. Check yt-dlp in Settings > Playback > Sources."));
        }
        if let Some(server) = &self.server {
            let local_yt = self.local_youtube_on();
            let priority = self.resolver.priority();
            let rank = |p: ProviderId| priority.iter().position(|x| *x == p).unwrap_or(usize::MAX);
            match server.resolve(t, &priority).await {
                Ok(resp) => {
                    let hint = resp.youtube_hint.as_deref();
                    if let Some(source) = resp.source.clone() {
                        let matched = resp.matched.unwrap_or_else(|| t.clone());
                        if local_yt && source.provider() == ProviderId::YouTube {
                            return Ok(self.local_youtube(&matched, hint).await);
                        }
                        // A full local source can outrank a short preview.
                        let preview = matches!(source, PlaybackSource::Preview { .. });
                        if local_yt && (preview || rank(ProviderId::YouTube) < rank(source.provider())) {
                            if let Some(r) = self.local_youtube(t, hint).await {
                                return Ok(Some(r));
                            }
                        }
                        if !matches!(source, PlaybackSource::Preview { .. }) {
                            self.resolver.remember(t, &matched, resp.confidence);
                        }
                        return Ok(Some(Resolved { track: matched, source, confidence: resp.confidence }));
                    }
                    if local_yt {
                        return Ok(self.local_youtube(t, hint).await);
                    }
                    return Ok(None);
                }
                Err(e) => {
                    // Server down: YouTube on this machine still works.
                    if local_yt {
                        if let Some(r) = self.local_youtube(t, None).await {
                            return Ok(Some(r));
                        }
                    }
                    return Err(e);
                }
            }
        }
        if let Some(r) = self.resolver.resolve_known(t).await {
            return Ok(Some(r));
        }
        if let Some(r) = self.resolver.resolve_cached(t).await {
            return Ok(Some(r));
        }
        if let Some(r) = self.resolver.resolve_by_search(t).await {
            return Ok(Some(r));
        }
        Ok(self.resolver.resolve_preview(t).await)
    }

    /// First source that offers a download: the song's own ids, then the
    /// full source the resolver matched it to.
    async fn download_url(&self, t: &Track) -> Result<Option<(ProviderId, String)>> {
        if self.config.providers.youtube_mode == config::YouTubeMode::Local && t.source == ProviderId::YouTube {
            return Ok(self.download_at(ProviderId::YouTube, t).await.map(|url| (ProviderId::YouTube, url)));
        }
        if let Some(server) = &self.server {
            if let Some(hit) = server.download_url(t).await.unwrap_or_else(|e| {
                tracing::debug!("server download lookup failed, going direct: {e}");
                None
            }) {
                // The server picks the normal Archive file; another quality
                // needs the item's file list.
                if hit.0 == ProviderId::Archive && downloads::job_quality() != config::AudioQuality::Standard {
                    if let Some(u) = self.download_at(ProviderId::Archive, t).await {
                        return Ok(Some((ProviderId::Archive, u)));
                    }
                }
                return Ok(Some(hit));
            }
            // Same path as playback: YouTube through the local yt-dlp.
            if let Some(Resolved { source: PlaybackSource::Stream { provider, url, .. }, .. }) = self.local_youtube(t, None).await {
                return Ok(Some((provider, url)));
            }
            return Ok(None);
        }
        const DL: [ProviderId; 4] = [ProviderId::Jamendo, ProviderId::Audius, ProviderId::Archive, ProviderId::YouTube];
        for p in DL {
            if provider::id_at(p, t).is_some() {
                if let Some(u) = self.download_at(p, t).await {
                    return Ok(Some((p, u)));
                }
            }
        }
        if let Some(r) = self.resolve(t).await? {
            let p = r.source.provider();
            if let PlaybackSource::Stream { provider: ProviderId::YouTube, url, .. } = &r.source {
                return Ok(Some((ProviderId::YouTube, url.clone())));
            }
            if matches!(r.source, PlaybackSource::Stream { .. }) && DL.contains(&p) {
                if let Some(u) = self.download_at(p, &r.track).await {
                    return Ok(Some((p, u)));
                }
            }
        }
        Ok(None)
    }

    async fn download_at(&self, p: ProviderId, t: &Track) -> Option<String> {
        match self.providers.get(p)?.download_url(t).await {
            Ok(u) => u,
            Err(e) => {
                tracing::info!(provider = ?p, "download lookup failed: {e}");
                None
            }
        }
    }

    async fn lyrics(&self, t: &Track) -> Result<Option<Lyrics>> {
        let key = lyrics::cache_key(t);
        if let Some(hit) = self.lyrics_cache.get(&key) {
            return Ok(hit);
        }
        let mut found: Option<Lyrics> = None;
        let mut server_answered = false;
        if let Some(server) = &self.server {
            match server.lyrics(&t.title, &t.artist, t.album.as_deref(), t.duration_ms.map(|d| d / 1000)).await {
                Ok(r) => {
                    server_answered = true;
                    // The wire type carries no match score.
                    found = r.map(|r| Lyrics::from_response(&r, 1.0));
                }
                Err(e) => return Err(e),
            }
        }
        if !server_answered {
            if let Some(l) = &self.providers.lrclib {
                found = l.lookup(&t.title, &t.artist, t.album.as_deref(), t.duration_ms).await?;
            }
        }
        self.lyrics_cache.put(&key, found.as_ref());
        Ok(found)
    }
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
        Ok(Self {
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
        let key = if page == 0 { q.to_lowercase() } else { format!("{} #p{page}", q.to_lowercase()) };
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
                while let Some(ev) = out_rx.recv().await {
                    let _ = tap_tx.send(ev.clone());
                    if fwd.send(ev).is_err() {
                        break;
                    }
                }
            });
            out_tx
        };
        self.rt.as_ref().expect("runtime alive").spawn(async move {
            if let Some(server) = &inner.server {
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
        let res = self
            .spawn(downloads::run_job(job, async move {
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
                dl.fetch(&inner.http, &track, provider, &url, inner.server.as_ref()).await.map(Some)
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

    pub fn remove_download(&self, track: &Track) -> bool {
        self.downloads.remove(track)
    }

    /// Re-reads the download index (after the folder was cleared).
    pub fn reload_downloads(&self) {
        self.downloads.reload();
    }

    pub async fn lyrics(&self, track: &Track) -> anyhow::Result<Option<Lyrics>> {
        let (inner, track) = (self.cur(), track.clone());
        self.spawn(async move { inner.lyrics(&track).await }).await?
    }

    /// Bio + photo for the artist page (Wikipedia in `lang`, then English).
    pub async fn artist_info(&self, name: &str, lang: &str) -> anyhow::Result<Option<artist::ArtistInfo>> {
        let (inner, name, lang) = (self.cur(), name.to_string(), lang.to_string());
        self.spawn(async move { artist::lookup_via(&inner.http, &name, &lang, inner.server.as_ref()).await }).await?
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
            let (list, origin) = import::fetch_list(&input, &inner.providers, inner.server.as_ref()).await?;
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
mod tests {
    use super::*;

    #[tokio::test]
    async fn configured_server_handles_known_tracks_and_errors_without_direct_fallback() {
        use std::io::{Read, Write};
        let _g = paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GLYPH_DATA_DIR", dir.path().join("data"));
        std::env::set_var("GLYPH_CACHE_DIR", dir.path().join("cache"));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let track = Track {
            id: "known".into(), source: ProviderId::Audius, title: "Song".into(), artist: "Artist".into(),
            album: None, duration_ms: None, cover_url: None, isrc: None, permalink: None, external_ids: Default::default(), explicit: false,
        };
        let resolved = serde_json::json!({"matched":track,"source":{"kind":"stream","provider":"audius","url":"/v1/audio/audius/known"},"confidence":1.0}).to_string();
        let fixture = std::thread::spawn(move || {
            let mut targets = Vec::new();
            let replies = [
                (200, serde_json::json!({"server_version":"test","protocol_version":1,"providers":["audius","youtube"]}).to_string()),
                (200, resolved),
                (503, serde_json::json!({"code":"unavailable","message":"test outage"}).to_string()),
            ];
            for (status,body) in replies {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                let mut socket = loop {
                    match listener.accept() {
                        Ok((s,_)) => break s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(5)),
                        Err(e) => panic!("missing server request: {e}"),
                    }
                };
                // The listener is non-blocking; accepted sockets inherit that on Windows.
                socket.set_nonblocking(false).unwrap();
                socket.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0;4096];
                    let n = socket.read(&mut chunk).unwrap();
                    if n == 0 { break; }
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let len = headers.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0);
                        if request.len() >= end + 4 + len { break; }
                    }
                }
                targets.push(String::from_utf8_lossy(&request).lines().next().unwrap().to_string());
                write!(socket, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            targets
        });
        let mut config = Config::default();
        config.server_url = base.clone();
        let backend = Backend::new(config).await.unwrap();
        assert!(backend.active_providers().contains(&ProviderId::YouTube));
        let result = backend.resolve(&track).await.unwrap().unwrap();
        assert!(matches!(result.source, PlaybackSource::Stream { url, .. } if url == format!("{base}/v1/audio/audius/known")));
        assert!(backend.resolve(&track).await.is_err(), "server errors must not use cached direct URLs");
        assert_eq!(fixture.join().unwrap(), ["GET /v1/health HTTP/1.1", "POST /v1/resolve HTTP/1.1", "POST /v1/resolve HTTP/1.1"]);
        // Even with a configured (now offline) server, saved audio wins before
        // warm-cache lookups, resolver requests or network authentication.
        let source = dir.path().join("saved.mp3");
        std::fs::write(&source, b"saved audio").unwrap();
        let saved = backend.downloads.store_copy(&track, ProviderId::Audius, &source).unwrap();
        backend.warm(vec![track.clone()]);
        for _ in 0..2 {
            let result = backend.resolve(&track).await.unwrap().unwrap();
            assert!(matches!(result.source, PlaybackSource::Stream { url, .. } if url == saved.to_string_lossy()));
        }
        std::env::remove_var("GLYPH_DATA_DIR");
        std::env::remove_var("GLYPH_CACHE_DIR");
    }

    #[tokio::test]
    async fn set_config_reevaluates_providers_priority_and_server() {
        let _g = paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GLYPH_DATA_DIR", dir.path().join("data"));
        std::env::set_var("GLYPH_CACHE_DIR", dir.path().join("cache"));

        let mut direct = Config::default();
        direct.server_url = "off".into();
        let b = Backend::new(direct.clone()).await.unwrap();
        assert_eq!(b.mode(), BackendMode::Direct);
        assert_eq!(b.active_providers().contains(&ProviderId::YouTube), ytdlp::locate(None).is_some());
        assert!(!b.active_providers().contains(&ProviderId::Jamendo));

        let mut cfg = direct;
        cfg.keys.youtube = Some("k".into());
        cfg.keys.jamendo_client_id = Some("j".into());
        cfg.providers.enabled.push(ProviderId::YouTube);
        cfg.providers.priority = vec![ProviderId::YouTube, ProviderId::Audius];
        cfg.language = "de".into();
        b.provider_cooling_down(ProviderId::Audius);
        b.set_config(cfg).await.unwrap();
        let active = b.active_providers();
        assert!(active.contains(&ProviderId::YouTube) && active.contains(&ProviderId::Jamendo));
        assert_eq!(b.config().resolved_language(), "de");
        assert_eq!(b.config().providers.priority[0], ProviderId::YouTube);

        // Unreachable configured server stays in server mode; no silent direct traffic.
        let mut cfg2 = Config::clone(&b.config());
        cfg2.server_url = "http://127.0.0.1:9".into();
        b.set_config(cfg2).await.unwrap();
        assert!(matches!(b.mode(), BackendMode::Server { .. }));

        std::env::remove_var("GLYPH_DATA_DIR");
        std::env::remove_var("GLYPH_CACHE_DIR");
    }
}
