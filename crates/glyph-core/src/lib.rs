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
pub mod provider;
pub mod providers;
pub mod queue;
pub mod resolver;
pub mod server_client;
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
    /// Recent search results (query -> when, songs): repeated searches do
    /// not hit the sources again for 10 minutes.
    search_cache: Arc<std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Vec<Track>)>>>,
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
            let client = ServerClient::new(&url, http.clone(), server_client::install_id());
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
        self.providers.youtube.as_ref().is_some_and(|y| y.local().is_some())
    }

    /// YouTube through the local yt-dlp (home IP, no bot check): the song's
    /// own video id, a cached mapping, the id other users shared through the
    /// server (`hint`), else a search for "Artist - Title" — a sure search
    /// hit is shared back so the next user skips the search.
    async fn local_youtube(&self, t: &Track, hint: Option<&str>) -> Option<Resolved> {
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
        if let Some(server) = &self.server {
            let local_yt = self.local_youtube_on();
            let priority = self.resolver.priority();
            let rank = |p: ProviderId| priority.iter().position(|x| *x == p).unwrap_or(usize::MAX);
            // YouTube first and the video already known: no server round trip.
            let known = provider::id_at(ProviderId::YouTube, t).is_some() || self.resolver.cached_at(t, ProviderId::YouTube).is_some();
            if local_yt && rank(ProviderId::YouTube) == 0 && known {
                if let Some(r) = self.local_youtube(t, None).await {
                    return Ok(Some(r));
                }
            }
            match server.resolve(t, &priority).await {
                Ok(resp) => {
                    let hint = resp.youtube_hint.as_deref();
                    if let Some(source) = resp.source.clone() {
                        let matched = resp.matched.unwrap_or_else(|| t.clone());
                        // The server cannot play YouTube (datacenter IPs get
                        // the bot check): its YouTube answers go to the local
                        // yt-dlp, and so does everything YouTube outranks in
                        // the priority. A 30 s preview never beats it.
                        if local_yt && source.provider() == ProviderId::YouTube {
                            if let Some(r) = self.local_youtube(&matched, hint).await {
                                return Ok(Some(r));
                            }
                        }
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
        Ok(Self {
            inner: std::sync::RwLock::new(Arc::new(inner)),
            downloads,
            search_cache: Default::default(),
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
        Ok(())
    }

    /// Unified search over all enabled providers, results arrive as they come
    /// in (already de-duplicated across providers). Ends with `SearchEvent::Done`.
    ///
    /// A song that was already sent by an earlier provider is sent again with
    /// the merged `external_ids` (same `source` + `id`): replace by that key.
    pub fn search(&self, query: &str) -> BoxStream<'static, SearchEvent> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let inner = self.cur();
        let q = query.trim().to_string();
        let key = q.to_lowercase();
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
                match server.search_into(&q, &tx).await {
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
            let mut events = inner.resolver.search_stream(&q);
            while let Some(ev) = events.recv().await {
                let done = ev == SearchEvent::Done;
                if tx.send(ev).is_err() || done {
                    break;
                }
            }
        });
        futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|v| (v, rx)) }).boxed()
    }

    /// Best legal full source (or a preview) for a track. Downloaded songs
    /// play from their local file.
    pub async fn resolve(&self, track: &Track) -> anyhow::Result<Option<Resolved>> {
        if let Some((path, provider)) = self.downloads.local(track) {
            let mime = Some(downloads::mime_of(&path).to_string());
            let source = PlaybackSource::Stream { provider, url: path.to_string_lossy().into_owned(), mime };
            return Ok(Some(Resolved { track: track.clone(), source, confidence: 1.0 }));
        }
        let (inner, track) = (self.cur(), track.clone());
        self.spawn(async move { inner.resolve(&track).await }).await?
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
            tracks.into_iter().filter(|t| self.downloads.local(t).is_none()).take(ytdlp::MAX_BATCH).collect();
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
        self.spawn(async move { ytdlp::status(configured.as_deref()).await }).await
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
        if let Some((p, _)) = self.downloads.local(track) {
            return Ok(Some(p));
        }
        let (inner, dl, track) = (self.cur(), self.downloads.clone(), track.clone());
        let job = downloads::Job { progress: Arc::default(), quality: inner.config.audio.download_quality };
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
                        match dl.fetch(&inner.http, &track, provider, &alt).await {
                            Ok(p) => return Ok(Some(p)),
                            Err(e) => tracing::info!("Jamendo {alt} failed, taking the MP3: {e:#}"),
                        }
                        downloads::report(|p| p.reset());
                    }
                }
                dl.fetch(&inner.http, &track, provider, &url).await.map(Some)
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

    /// Index lookup only (cheap enough for every rendered row).
    pub fn is_downloaded(&self, track: &Track) -> bool {
        self.downloads.contains(track)
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
