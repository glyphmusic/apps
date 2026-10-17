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

pub mod config;
pub mod artist;
pub mod cover;
pub mod dedupe;
pub mod import;
pub mod library;
pub mod lyrics;
pub mod paths;
pub mod provider;
pub mod providers;
pub mod queue;
pub mod resolver;
pub mod server_client;
pub mod storage;
pub mod updater;

pub use config::Config;
pub use library::{Library, Playlist};
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
    covers: CoverCache,
    lyrics_cache: LyricsCache,
}

pub struct Backend {
    /// Current setup; replaced as a whole by `set_config`.
    inner: std::sync::RwLock<Arc<Inner>>,
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
        let url = server_client::normalize_base_url(config.effective_server_url());
        if !url.is_empty() {
            let client = ServerClient::new(&url, http.clone(), server_client::install_id());
            match client.check_compat().await {
                Ok(Compat::Compatible(h)) => {
                    mode = BackendMode::Server { url, server_version: h.server_version };
                    server = Some(client);
                }
                Ok(Compat::Incompatible(h)) => {
                    tracing::warn!(
                        server = h.protocol_version,
                        ours = glyph_protocol::PROTOCOL_VERSION,
                        "incompatible server, using direct mode"
                    );
                    mode = BackendMode::Incompatible { url, protocol_version: h.protocol_version };
                }
                Err(e) => tracing::warn!("server {url} unreachable, using direct mode: {e}"),
            }
        }
        Ok(Inner { config: Arc::new(config), mode, http, providers, cooldowns, resolver, server, covers, lyrics_cache })
    }

    async fn resolve(&self, t: &Track) -> Result<Option<Resolved>> {
        if let Some(r) = self.resolver.resolve_known(t).await {
            return Ok(Some(r));
        }
        if let Some(r) = self.resolver.resolve_cached(t).await {
            return Ok(Some(r));
        }
        if let Some(server) = &self.server {
            match server.resolve(t, &self.resolver.priority()).await {
                Ok(resp) => {
                    if let Some(source) = resp.source {
                        let matched = resp.matched.unwrap_or_else(|| t.clone());
                        if !matches!(source, PlaybackSource::Preview { .. }) {
                            self.resolver.remember(t, &matched, resp.confidence);
                        }
                        return Ok(Some(Resolved { track: matched, source, confidence: resp.confidence }));
                    }
                }
                Err(e) => tracing::warn!("server resolve failed, resolving locally: {e}"),
            }
        }
        if let Some(r) = self.resolver.resolve_by_search(t).await {
            return Ok(Some(r));
        }
        Ok(self.resolver.resolve_preview(t).await)
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
                Err(e) => tracing::warn!("server lyrics failed, going direct: {e}"),
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
        Ok(Self { inner: std::sync::RwLock::new(Arc::new(inner)), rt: Some(rt) })
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
        self.rt.as_ref().expect("runtime alive").spawn(async move {
            if let Some(server) = &inner.server {
                match server.search_into(&q, &tx).await {
                    Ok(true) => return,
                    Ok(false) => {
                        let _ = tx.send(SearchEvent::Done);
                        return;
                    }
                    Err(e) => tracing::warn!("server search failed, searching directly: {e}"),
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

    /// Best legal full source (or a preview) for a track.
    pub async fn resolve(&self, track: &Track) -> anyhow::Result<Option<Resolved>> {
        let (inner, track) = (self.cur(), track.clone());
        self.spawn(async move { inner.resolve(&track).await }).await?
    }

    pub async fn lyrics(&self, track: &Track) -> anyhow::Result<Option<Lyrics>> {
        let (inner, track) = (self.cur(), track.clone());
        self.spawn(async move { inner.lyrics(&track).await }).await?
    }

    /// Bio + photo for the artist page (Wikipedia in `lang`, then English).
    pub async fn artist_info(&self, name: &str, lang: &str) -> anyhow::Result<Option<artist::ArtistInfo>> {
        let (inner, name, lang) = (self.cur(), name.to_string(), lang.to_string());
        self.spawn(async move { artist::lookup(&inner.http, &name, &lang).await }).await?
    }

    /// Radio: similar tracks (Last.fm track.getSimilar), not yet resolved.
    pub async fn similar(&self, track: &Track) -> anyhow::Result<Vec<Track>> {
        let (inner, track) = (self.cur(), track.clone());
        self.spawn(async move {
            if let Some(server) = &inner.server {
                match server.similar(&track.artist, &track.title).await {
                    Ok(r) => return Ok(r.tracks),
                    Err(e) => tracing::warn!("server similar failed, going direct: {e}"),
                }
            }
            match &inner.providers.lastfm {
                Some(lf) => lf.similar(&track.artist, &track.title).await,
                None => Ok(vec![]), // no Last.fm key: radio unavailable
            }
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
        self.cur().providers.ids()
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
    async fn set_config_reevaluates_providers_priority_and_server() {
        let _g = paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GLYPH_DATA_DIR", dir.path().join("data"));
        std::env::set_var("GLYPH_CACHE_DIR", dir.path().join("cache"));

        let mut direct = Config::default();
        direct.server_url = "off".into();
        let b = Backend::new(direct.clone()).await.unwrap();
        assert_eq!(b.mode(), BackendMode::Direct);
        assert!(!b.active_providers().contains(&ProviderId::YouTube));
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

        // unreachable server -> still direct, no error
        let mut cfg2 = Config::clone(&b.config());
        cfg2.server_url = "http://127.0.0.1:9".into();
        b.set_config(cfg2).await.unwrap();
        assert_eq!(b.mode(), BackendMode::Direct);

        std::env::remove_var("GLYPH_DATA_DIR");
        std::env::remove_var("GLYPH_CACHE_DIR");
    }
}
