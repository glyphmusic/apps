use super::*;

impl Inner {
    pub(super) async fn build(config: Config, old_cooldowns: Option<Arc<Cooldowns>>) -> Result<Inner> {
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
        crate::album_names::set_store(paths::data_dir().join("album-names.tsv"));
        let covers = CoverCache::new(storage::covers_dir(), http.clone(), providers.clone()).with_max_bytes(limit);
        let lyrics_cache = LyricsCache::new(paths::data_dir().join("lyrics"));

        library::set_playlist_track_limit(library::MAX_PLAYLIST_TRACKS);
        let mut mode = BackendMode::Direct;
        let mut server = None;
        let mut server_providers = None;
        let mut server_rates = false;
        let mut server_wrong_song = false;
        let url = server_client::normalize_base_url(config.effective_server_url());
        if !url.is_empty() {
            let key_url = url.clone();
            let token = tokio::task::spawn_blocking(move || server_access::load(&key_url)).await.ok().flatten();
            let client = ServerClient::new(&url, http.clone(), server_client::install_id()).with_bearer(token)?;
            match client.check_compat().await {
                Ok(Compat::Compatible(h)) => {
                    if crate::outage::is_official(&url) { crate::outage::clear(); }
                    crate::app_outdated::note_health(h.min_app_version.as_deref());
                    library::set_playlist_track_limit(h.playlist_max_tracks);
                    server_rates = h.supports(glyph_protocol::capability::RATINGS);
                    server_wrong_song = h.supports(glyph_protocol::capability::WRONG_SONG);
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
                    crate::outage::note(&url, &e).await;
                    server_providers = Some(vec![]);
                    mode = BackendMode::Server { url, server_version: "offline".into() };
                    server = Some(client);
                },
            }
        }
        let covers = covers.with_server(server.clone());
        Ok(Inner { config: Arc::new(config), mode, http, providers, cooldowns, resolver, server, server_providers, server_rates, server_wrong_song, covers, lyrics_cache })
    }

    /// The server, unless it is the official one and that is out (see
    /// [`crate::outage`]): then everything runs as in direct mode.
    pub(super) fn live_server(&self) -> Option<&ServerClient> {
        self.server.as_ref().filter(|s| !crate::outage::is_official(s.base_url()) || !crate::outage::switched())
    }

    /// YouTube is enabled and yt-dlp runs on this machine.
    pub(super) fn local_youtube_on(&self) -> bool {
        (self.config.providers.youtube_mode == config::YouTubeMode::Local || self.live_server().is_none())
            && self.providers.youtube.as_ref().is_some_and(|y| y.local().is_some())
    }

    /// YouTube through the local yt-dlp (home IP, no bot check): the song's
    /// own video id, a cached mapping, the id other users shared through the
    /// server (`hint`), else a search for "Artist - Title" — a sure search
    /// hit is shared back so the next user skips the search.
    pub(super) async fn local_youtube_match(&self, t: &Track, hint: Option<&str>) -> Option<(Track, f32)> {
        if !self.local_youtube_on() { return None; }
        self.local_youtube_match_here(t, hint, true).await
    }

    /// [`Self::local_youtube_match`] without asking whether local YouTube is
    /// the chosen mode (see [`Self::rescue_on_this_computer`]). `share_hint`
    /// is disabled during bulk imports/transfers so matching never fires a
    /// burst of `POST /v1/youtube/hint` calls at the server.
    async fn local_youtube_match_here(&self, t: &Track, hint: Option<&str>, share_hint: bool) -> Option<(Track, f32)> {
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
            if share_hint && confidence >= resolver::SURE_CONFIDENCE {
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
        Some((matched, confidence))
    }

    /// `t` with a YouTube video id attached when a quick search here finds a
    /// sure match (`None`: it has one already, or nothing sure was found).
    pub(super) async fn youtube_id_hint(&self, t: &Track) -> Option<Track> {
        // Songs that already have a full source (or a YouTube id) need no search.
        let full = [ProviderId::YouTube, ProviderId::Audius, ProviderId::Jamendo, ProviderId::Archive, ProviderId::SoundCloud];
        if full.contains(&t.source) || t.external_ids.contains_key(&ProviderId::YouTube) {
            return None;
        }
        let id = match self.resolver.cached_at(t, ProviderId::YouTube) {
            Some((m, c)) if c >= 0.7 && ytdlp::valid_video_id(&m.id) => m.id,
            _ => {
                let q = format!("{} - {}", t.artist, t.title);
                let cands = tokio::time::timeout(std::time::Duration::from_millis(2500), ytnative::search(&self.http, &q, ytdlp::SEARCH_RESULTS))
                    .await
                    .ok()?
                    .ok()?;
                let (best, confidence) = resolver::best_candidate(t, cands)?;
                if confidence < resolver::SURE_CONFIDENCE {
                    return None;
                }
                self.resolver.remember(t, &best.track, confidence);
                best.track.id
            }
        };
        let mut out = t.clone();
        out.external_ids.insert(ProviderId::YouTube, id);
        Some(out)
    }

    /// Other good enough videos for `t` (best first), searched with the
    /// wording of `round`: 0 = the normal "Artist - Title" (cached, so free
    /// after the first search), later rounds word it differently.
    async fn local_youtube_alternatives(&self, t: &Track, round: usize) -> Vec<(Track, f32)> {
        let Some(yt) = self.providers.youtube.as_ref().filter(|y| y.local().is_some()) else { return Vec::new() };
        let title = t.title.split(" (").next().unwrap_or(&t.title).trim();
        let query = match round {
            0 => format!("{} - {}", t.artist, t.title),
            1 => format!("{} {}", t.artist, t.title),
            _ => format!("{} {title} audio", crate::dedupe::primary_artist(&t.artist)),
        };
        match provider::MusicProvider::search_candidates(yt.as_ref(), &query).await {
            Ok(cands) => resolver::ranked_candidates(t, cands).into_iter().map(|(c, s)| (c.track, s)).collect(),
            Err(e) => {
                tracing::info!(round, "local YouTube search failed: {e:#}");
                Vec::new()
            }
        }
    }

    /// The best match first; when its video will not play (removed,
    /// blocked, a hiccup), the next good matches. Patient search
    /// (Settings > Playback) asks again with other wordings and waits a
    /// little between the tries.
    pub(super) async fn local_youtube(&self, t: &Track, hint: Option<&str>) -> Option<Resolved> {
        let first = self.local_youtube_match(t, hint).await;
        self.local_youtube_from(t, first).await
    }

    /// Server mode, and the server answered without full YouTube audio for
    /// a song that should come from YouTube (YouTube blocks or bot-checks
    /// the server, or its resolver is broken): this computer tries itself
    /// before the song counts as unplayable. r-dlp is built in, so this
    /// works without installing anything. Not when the server is down:
    /// server mode sends nothing directly then.
    async fn rescue_on_this_computer(&self, t: &Track, hint: Option<&str>) -> Option<Resolved> {
        // Local mode already tried here; YouTube switched off stays off.
        if self.local_youtube_on() || !self.config.providers.enabled.contains(&ProviderId::YouTube) {
            return None;
        }
        self.providers.youtube.as_ref().filter(|y| y.local().is_some())?;
        tracing::info!("server found no YouTube audio, trying this computer");
        let first = self.local_youtube_match_here(t, hint, true).await;
        self.local_youtube_from(t, first).await
    }

    async fn local_youtube_from(&self, t: &Track, first: Option<(Track, f32)>) -> Option<Resolved> {
        let yt = self.providers.youtube.as_ref()?;
        let patient = self.config.playback.patient_search;
        let rounds = if patient { 3 } else { 1 };
        let per_round = if patient { 5 } else { 3 };
        let mut tried: Vec<String> = Vec::new();
        for round in 0..rounds {
            if round > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(1500 * round as u64)).await;
                tracing::info!(round, "patient search: trying again");
            }
            let mut list: Vec<(Track, f32)> = Vec::new();
            if round == 0 {
                list.extend(first.clone());
            }
            let mut alternatives_loaded = false;
            let mut i = 0;
            loop {
                if i >= list.len() {
                    if alternatives_loaded {
                        break;
                    }
                    alternatives_loaded = true;
                    list.extend(self.local_youtube_alternatives(t, round).await);
                    if i >= list.len() {
                        break;
                    }
                }
                let (matched, confidence) = list[i].clone();
                i += 1;
                if tried.contains(&matched.id) {
                    continue;
                }
                if tried.len() >= per_round * (round + 1) {
                    break;
                }
                tried.push(matched.id.clone());
                match yt.local_source(&matched.id).await {
                    Ok(source) => {
                        if tried.len() > 1 {
                            // A fallback played: remember it for next time.
                            self.resolver.remember(t, &matched, confidence);
                        }
                        // Keep the wanted song's metadata when it was a direct id hit.
                        let track = if confidence >= 1.0 && t.source != ProviderId::YouTube { t.clone() } else { matched };
                        return Some(Resolved { track, source, confidence });
                    }
                    Err(e) => tracing::info!(video_id = %matched.id, "local YouTube playback failed: {e:#}"),
                }
            }
        }
        None
    }

    /// Warms the yt-dlp URL cache for songs that will play soon: songs whose
    /// YouTube video is already known resolve together in one yt-dlp run,
    /// the others go through the normal resolve (which caches what it finds).
    pub(super) async fn prefetch(&self, tracks: Vec<Track>) {
        if self.live_server().is_some() {
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
}

/// Bounds and paces server `/v1/resolve` fallback calls during a single import
/// or multi-playlist transfer so bulk matching never trips the server's rate
/// limit or 15-minute abuse block (even for guests).
pub(super) struct ImportMatchGuard {
    server_calls: std::sync::atomic::AtomicUsize,
    server_blocked: std::sync::atomic::AtomicBool,
    last_server_call: tokio::sync::Mutex<Option<std::time::Instant>>,
}

impl ImportMatchGuard {
    /// At most this many server `/v1/resolve` fallback calls per transfer;
    /// any remaining tracks keep their metadata and resolve on playback.
    const MAX_SERVER_RESOLVES: usize = 25;
    /// Minimum gap between consecutive server `/v1/resolve` calls (~200/min ceiling).
    const SERVER_GAP: std::time::Duration = std::time::Duration::from_millis(300);

    pub(super) fn new() -> Self {
        Self {
            server_calls: std::sync::atomic::AtomicUsize::new(0),
            server_blocked: std::sync::atomic::AtomicBool::new(false),
            last_server_call: tokio::sync::Mutex::new(None),
        }
    }

    async fn claim_server_slot(&self) -> bool {
        use std::sync::atomic::Ordering;
        if self.server_blocked.load(Ordering::Relaxed) {
            return false;
        }
        let claimed = self
            .server_calls
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < Self::MAX_SERVER_RESOLVES).then_some(n + 1))
            .is_ok();
        if !claimed {
            return false;
        }
        let mut last = self.last_server_call.lock().await;
        if self.server_blocked.load(Ordering::Relaxed) {
            return false;
        }
        if let Some(at) = *last {
            let elapsed = at.elapsed();
            if elapsed < Self::SERVER_GAP {
                tokio::time::sleep(Self::SERVER_GAP - elapsed).await;
            }
        }
        if self.server_blocked.load(Ordering::Relaxed) {
            return false;
        }
        *last = Some(std::time::Instant::now());
        true
    }

    fn note_server_error(&self, err: &anyhow::Error) {
        use std::sync::atomic::Ordering;
        if matches!(server_client::server_error(err), Some((429, _))) {
            // Stop all further server fallback calls immediately on the very
            // first 429 so we never accumulate abuse-ban strikes.
            self.server_blocked.store(true, Ordering::Relaxed);
        }
    }
}

impl Inner {
    /// Fast metadata-only matching for playlist imports and library transfers.
    ///
    /// `ImportMatch::to_track()` only keeps the matched source's ID and metadata
    /// (audio stream URLs expire and are discarded), so running `yt-dlp`/`r-dlp`
    /// stream extraction or fetching stream URLs during import wastes seconds per
    /// song. Instead:
    /// 1. 0 ms if the track already has an enabled full-source ID (e.g. YouTube/SoundCloud).
    /// 2. 0 ms if already in the local persistent mapping cache.
    /// 3. Lightweight metadata search (native YouTube search or enabled local providers)
    ///    without extracting audio streams or firing `POST /v1/youtube/hint` bursts.
    /// 4. Paced, bounded server fallback (at most 25 calls per transfer, halted
    ///    immediately on 429) only if local providers couldn't match and server mode is active.
    pub(super) async fn match_for_import_guarded(&self, t: &Track, guard: &ImportMatchGuard) -> Option<Resolved> {
        let enabled = &self.config.providers.enabled;
        let yt_only = self.config.providers.youtube_only;
        let wrap = |(matched, confidence): (Track, f32)| Resolved {
            source: PlaybackSource::Stream {
                provider: matched.source,
                url: String::new(),
                mime: None,
            },
            track: matched,
            confidence,
        };
        if let Some(hit) = self.resolver.known_match(t, enabled, yt_only) {
            return Some(wrap(hit));
        }
        if let Some(hit) = self.resolver.cached_match(t, enabled, yt_only) {
            return Some(wrap(hit));
        }
        let yt_enabled = enabled.contains(&ProviderId::YouTube);
        let yt_first = yt_enabled
            && (yt_only
                || self.config.providers.youtube_mode == config::YouTubeMode::Local
                || self.resolver.priority().first() == Some(&ProviderId::YouTube));
        if yt_first {
            if let Some(hit) = self.local_youtube_match_here(t, None, false).await {
                if hit.1 >= resolver::SURE_CONFIDENCE || yt_only {
                    return Some(wrap(hit));
                }
                // Try other local providers too in case one has a higher-confidence match.
                if let Some(other) = self.resolver.match_by_search(t, enabled, false).await {
                    if other.1 > hit.1 {
                        return Some(wrap(other));
                    }
                }
                return Some(wrap(hit));
            }
            if yt_only {
                return None;
            }
        }
        if let Some(hit) = self.resolver.match_by_search(t, enabled, yt_only).await {
            return Some(wrap(hit));
        }
        // Bounded, paced server fallback only when local providers could not match.
        if self.live_server().is_some() && !self.local_youtube_on() && guard.claim_server_slot().await {
            match self.resolve(t).await {
                Ok(r) => return r,
                Err(e) => guard.note_server_error(&e),
            }
        }
        None
    }
}

impl Inner {

    pub(super) async fn resolve(&self, t: &Track) -> Result<Option<Resolved>> {
        // User explicitly selected a replacement via "Not the right one":
        // honour their choice locally immediately without waiting for server consensus.
        if let Some(pinned) = self.resolver.pinned_override(t) {
            if let Some(r) = self.resolve_exact(&pinned).await? {
                return Ok(Some(r));
            }
        }
        if self.config.providers.youtube_mode == config::YouTubeMode::Local
            && self.config.providers.enabled.contains(&ProviderId::YouTube)
            && (self.config.providers.youtube_only || t.source == ProviderId::YouTube || self.resolver.priority().first() == Some(&ProviderId::YouTube))
        {
            return self.local_youtube(t, None).await.map(Some)
                .ok_or_else(|| anyhow!(crate::ERR_YOUTUBE_FAILED));
        }
        if let Some(server) = self.live_server() {
            let local_yt = self.local_youtube_on();
            let enabled = &self.config.providers.enabled;
            let yt_enabled = enabled.contains(&ProviderId::YouTube);
            let priority: Vec<ProviderId> = self.resolver.priority().into_iter().filter(|p| enabled.contains(p)).collect();
            let rank = |p: ProviderId| priority.iter().position(|x| *x == p).unwrap_or(usize::MAX);
            // The server would search YouTube for this song first (seconds on
            // a cold server); a search from here takes a few hundred ms. A sure
            // match travels along as the song's YouTube id.
            // The song should come from YouTube: when the server answers
            // without YouTube audio, this computer may try (see below).
            let youtube_wanted = yt_enabled && (self.config.providers.youtube_only || t.source == ProviderId::YouTube || priority.first() == Some(&ProviderId::YouTube));
            let hinted;
            let t = if youtube_wanted && (self.config.providers.youtube_only || priority.first() == Some(&ProviderId::YouTube)) {
                hinted = self.youtube_id_hint(t).await;
                hinted.as_ref().unwrap_or(t)
            } else {
                t
            };
            match server.resolve_with_mode(t, &priority, self.config.providers.youtube_only).await {
                Ok(resp) => {
                    let hint = resp.youtube_hint.as_deref();
                    if let Some(source) = resp.source.clone().filter(|s| enabled.contains(&s.provider())) {
                        if self.config.providers.youtube_only && (source.provider() != ProviderId::YouTube || matches!(source, PlaybackSource::Preview { .. })) {
                            if let Some(r) = self.rescue_on_this_computer(t, hint).await {
                                return Ok(Some(r));
                            }
                            return Err(anyhow!(crate::ERR_NOT_ON_YOUTUBE));
                        }
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
                        // Only a 30-second preview from the server: the whole
                        // song from here beats it.
                        if preview && youtube_wanted {
                            if let Some(r) = self.rescue_on_this_computer(t, hint).await {
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
                    if youtube_wanted {
                        if let Some(r) = self.rescue_on_this_computer(t, hint).await {
                            return Ok(Some(r));
                        }
                    }
                    if self.config.providers.youtube_only { return Err(anyhow!(crate::ERR_NOT_ON_YOUTUBE)); }
                    return Ok(None);
                }
                Err(e) => {
                    // The official server is out: resolve locally below.
                    if crate::outage::note(server.base_url(), &e).await.is_none() {
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
        }
        if self.config.providers.youtube_only {
            return self.local_youtube(t, None).await.map(Some).ok_or_else(|| anyhow!(crate::ERR_NOT_ON_YOUTUBE));
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

    /// First source that offers a download: a user-pinned replacement first,
    /// then the song's own ids, then the full source the resolver matched it to.
    pub(super) async fn download_url(&self, t: &Track) -> Result<Option<(ProviderId, String)>> {
        if let Some(pinned) = self.resolver.pinned_override(t) {
            if let Some(hit) = self.download_url_exact(&pinned).await? {
                return Ok(Some(hit));
            }
        }
        if self.config.providers.youtube_mode == config::YouTubeMode::Local
            && (self.config.providers.youtube_only || t.source == ProviderId::YouTube || self.resolver.priority().first() == Some(&ProviderId::YouTube)) {
            if let Some((matched, _)) = self.local_youtube_match(t, None).await {
                return Ok(self.download_at(ProviderId::YouTube, &matched).await.map(|url| (ProviderId::YouTube, url)));
            }
            return Err(anyhow!(crate::ERR_YOUTUBE_FAILED));
        }
        // The official server out: the direct path below (`break 'server`).
        'server: { if let Some(server) = self.live_server() {
            let enabled = &self.config.providers.enabled;
            let priority: Vec<ProviderId> = self.resolver.priority().into_iter().filter(|p| enabled.contains(p)).collect();
            let yt_enabled = enabled.contains(&ProviderId::YouTube);
            let youtube_wanted = yt_enabled
                && (self.config.providers.youtube_only
                    || t.source == ProviderId::YouTube
                    || priority.first() == Some(&ProviderId::YouTube));
            let server_has_youtube = self.server_providers.as_ref().is_some_and(|ps| ps.contains(&ProviderId::YouTube));
            let quality = match downloads::job_quality() {
                config::AudioQuality::Low => "low",
                config::AudioQuality::High => "high",
                _ => "standard",
            };
            // Fast path: when YouTube is wanted and the server serves YouTube,
            // reuse an already-known/cached video id or resolve a sure match
            // locally in a few hundred ms — skipping the slow `POST /v1/download`
            // Worker + companion search round-trip altogether.
            let mut hinted: Option<Track> = None;
            if youtube_wanted {
                let known_id = provider::id_at(ProviderId::YouTube, t)
                    .filter(|id| ytdlp::valid_video_id(id))
                    .map(str::to_string)
                    .or_else(|| {
                        self.resolver
                            .cached_at(t, ProviderId::YouTube)
                            .filter(|(m, c)| *c >= 0.7 && ytdlp::valid_video_id(&m.id))
                            .map(|(m, _)| m.id)
                    });
                let id = match known_id {
                    Some(id) => Some(id),
                    None if self.config.providers.youtube_only || priority.first() == Some(&ProviderId::YouTube) => {
                        hinted = self.youtube_id_hint(t).await;
                        hinted
                            .as_ref()
                            .and_then(|h| provider::id_at(ProviderId::YouTube, h))
                            .filter(|id| ytdlp::valid_video_id(id))
                            .map(str::to_string)
                    }
                    None => None,
                };
                if let Some(id) = id.filter(|_| server_has_youtube) {
                    return Ok(Some((ProviderId::YouTube, server.download_youtube_url(&id, quality))));
                }
            }
            let lookup_track = hinted.as_ref().unwrap_or(t);
            let lookup = match server.download_url_with_priority(lookup_track, &priority, self.config.providers.youtube_only).await {
                Ok(hit) => hit,
                Err(e) => {
                    if crate::outage::note(server.base_url(), &e).await.is_some() { tracing::info!("official server out, downloading locally"); break 'server; }
                    if self.config.providers.youtube_mode == config::YouTubeMode::Server { return Err(e); }
                    tracing::debug!("server download lookup failed: {e}");
                    None
                }
            };
            if let Some(hit) = lookup.filter(|(p, _)| enabled.contains(p)) {
                if self.config.providers.youtube_only && hit.0 != ProviderId::YouTube {
                    return Err(anyhow!(crate::ERR_NOT_ON_YOUTUBE));
                }
                // The server picks the normal Archive file; another quality
                // needs the item's file list.
                if hit.0 == ProviderId::Archive && downloads::job_quality() != config::AudioQuality::Standard {
                    if let Some(u) = self.download_at(ProviderId::Archive, t).await {
                        return Ok(Some((ProviderId::Archive, u)));
                    }
                }
                if hit.0 == ProviderId::YouTube {
                    if let Some(id) = hit.1.rsplit("/v1/download/youtube/").next().map(|s| s.split('?').next().unwrap_or(s)).filter(|id| ytdlp::valid_video_id(id)) {
                        let mut matched = t.clone();
                        matched.source = ProviderId::YouTube;
                        matched.id = id.to_string();
                        self.resolver.remember(t, &matched, resolver::SURE_CONFIDENCE);
                    }
                    return Ok(Some((hit.0, format!("{}?quality={quality}", hit.1))));
                }
                return Ok(Some(hit));
            }
            if self.config.providers.youtube_only { return Err(anyhow!(crate::ERR_NOT_ON_YOUTUBE)); }
            // Same path as playback: YouTube through the local yt-dlp.
            if let Some(Resolved { source: PlaybackSource::Stream { provider, url, .. }, .. }) = self.local_youtube(t, None).await {
                return Ok(Some((provider, url)));
            }
            return Ok(None);
        } }
        if self.config.providers.youtube_only { return Err(anyhow!(crate::ERR_NOT_ON_YOUTUBE)); }
        const DL: [ProviderId; 4] = [ProviderId::Jamendo, ProviderId::Audius, ProviderId::Archive, ProviderId::YouTube];
        for p in DL {
            if self.config.providers.enabled.contains(&p) && provider::id_at(p, t).is_some() {
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

    pub(super) async fn download_at(&self, p: ProviderId, t: &Track) -> Option<String> {
        match self.providers.get(p)?.download_url(t).await {
            Ok(u) => u,
            Err(e) => {
                tracing::info!(provider = ?p, "download lookup failed: {e}");
                None
            }
        }
    }

    /// Resolves one exact candidate `(source, id)` directly (for 30-second
    /// previews and user-pinned replacements).
    pub(super) async fn resolve_exact(&self, candidate: &Track) -> Result<Option<Resolved>> {
        let p = candidate.source;
        if p == ProviderId::YouTube {
            if self.config.providers.youtube_mode == config::YouTubeMode::Local || self.local_youtube_on() {
                if let Some(r) = self.local_youtube(candidate, Some(&candidate.id)).await {
                    return Ok(Some(r));
                }
            }
            if let Some(server) = self.live_server() {
                if self.server_providers.as_ref().is_some_and(|ps| ps.contains(&ProviderId::YouTube))
                    && ytdlp::valid_video_id(&candidate.id)
                {
                    let url = server.audio_provider_url(ProviderId::YouTube, &candidate.id);
                    return Ok(Some(Resolved {
                        track: candidate.clone(),
                        source: PlaybackSource::Stream { provider: ProviderId::YouTube, url, mime: None },
                        confidence: 1.0,
                    }));
                }
            }
            if let Some(r) = self.rescue_on_this_computer(candidate, Some(&candidate.id)).await {
                return Ok(Some(r));
            }
            return Ok(self.resolver.resolve_exact(candidate).await);
        }
        if let Some(r) = self.resolver.resolve_exact(candidate).await {
            return Ok(Some(r));
        }
        if let Some(server) = self.live_server() {
            if self.server_providers.as_ref().is_some_and(|ps| ps.contains(&p)) {
                let url = server.audio_provider_url(p, &candidate.id);
                return Ok(Some(Resolved {
                    track: candidate.clone(),
                    source: PlaybackSource::Stream { provider: p, url, mime: None },
                    confidence: 1.0,
                }));
            }
        }
        Ok(None)
    }

    /// Download URL for one exact candidate `(source, id)` (user-pinned replacement).
    async fn download_url_exact(&self, candidate: &Track) -> Result<Option<(ProviderId, String)>> {
        let p = candidate.source;
        let quality = match downloads::job_quality() {
            config::AudioQuality::Low => "low",
            config::AudioQuality::High => "high",
            _ => "standard",
        };
        if p == ProviderId::YouTube {
            if self.config.providers.youtube_mode == config::YouTubeMode::Local {
                if let Some(u) = self.download_at(ProviderId::YouTube, candidate).await {
                    return Ok(Some((ProviderId::YouTube, u)));
                }
                if let Some(Resolved { source: PlaybackSource::Stream { url, .. }, .. }) =
                    self.local_youtube(candidate, Some(&candidate.id)).await
                {
                    return Ok(Some((ProviderId::YouTube, url)));
                }
                return Ok(None);
            }
            if let Some(server) = self.live_server() {
                if self.server_providers.as_ref().is_some_and(|ps| ps.contains(&ProviderId::YouTube))
                    && ytdlp::valid_video_id(&candidate.id)
                {
                    return Ok(Some((ProviderId::YouTube, server.download_youtube_url(&candidate.id, quality))));
                }
            }
            if let Some(u) = self.download_at(ProviderId::YouTube, candidate).await {
                return Ok(Some((ProviderId::YouTube, u)));
            }
            if let Some(Resolved { source: PlaybackSource::Stream { url, .. }, .. }) =
                self.local_youtube(candidate, Some(&candidate.id)).await
            {
                return Ok(Some((ProviderId::YouTube, url)));
            }
            return Ok(None);
        }
        if let Some(u) = self.download_at(p, candidate).await {
            return Ok(Some((p, u)));
        }
        if let Some(server) = self.live_server() {
            if self.server_providers.as_ref().is_some_and(|ps| ps.contains(&p)) {
                return Ok(Some((p, server.download_provider_url(p, &candidate.id, quality))));
            }
        }
        Ok(None)
    }

    /// Fetches replacement candidates from the server (when supported) and/or local
    /// providers + native YouTube search.
    pub(super) async fn match_alternatives(
        &self,
        track: &Track,
        query: Option<String>,
    ) -> Result<glyph_protocol::MatchAlternativesResponse> {
        let enabled = &self.config.providers.enabled;
        let priority: Vec<ProviderId> = self.resolver.priority().into_iter().filter(|p| enabled.contains(p)).collect();
        let mut resp = if let Some(server) = self.live_server().filter(|_| self.server_wrong_song) {
            match server.match_alternatives(track, query.clone(), &priority).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::info!("server match_alternatives failed, falling back locally: {e:#}");
                    glyph_protocol::MatchAlternativesResponse {
                        current_source: None,
                        current_id: None,
                        alternatives: Vec::new(),
                    }
                }
            }
        } else {
            glyph_protocol::MatchAlternativesResponse {
                current_source: None,
                current_id: None,
                alternatives: Vec::new(),
            }
        };

        // Ensure current local pinned/cached match is reflected as current.
        if let Some(cur) = self.resolver.current_mapping(track) {
            resp.current_source = Some(cur.source);
            resp.current_id = Some(cur.id);
        }

        // Fill/augment with local providers & native YouTube search when the server
        // isn't used or returned few candidates.
        if resp.alternatives.len() < 6 {
            let mut seen: std::collections::HashSet<(ProviderId, String)> = resp
                .alternatives
                .iter()
                .map(|a| (a.track.source, a.track.id.clone()))
                .collect();
            let local_alts = self.resolver.search_alternatives(track, query.as_deref()).await;
            for alt in local_alts {
                if seen.insert((alt.track.source, alt.track.id.clone())) {
                    resp.alternatives.push(alt);
                }
            }
            if enabled.contains(&ProviderId::YouTube) && resp.alternatives.len() < 8 {
                let q = query
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{} {}", track.artist, track.title));
                if let Ok(Ok(yt_cands)) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), ytnative::search(&self.http, &q, 10)).await
                {
                    let cur_pair = resp.current_source.zip(resp.current_id.as_deref());
                    for c in yt_cands {
                        if !seen.insert((ProviderId::YouTube, c.track.id.clone())) {
                            continue;
                        }
                        let conf = resolver::score_candidate(track, &c);
                        let tag = if cur_pair == Some((ProviderId::YouTube, c.track.id.as_str())) {
                            Some("current".into())
                        } else if c.bonus >= 0.14 {
                            Some("topic".into())
                        } else if c.bonus > 0.0 || c.track.title.to_lowercase().contains("official audio") {
                            Some("official_audio".into())
                        } else {
                            None
                        };
                        let preview_url = self.live_server().and_then(|s| {
                            self.server_providers
                                .as_ref()
                                .is_some_and(|ps| ps.contains(&ProviderId::YouTube))
                                .then(|| s.audio_provider_url(ProviderId::YouTube, &c.track.id))
                        });
                        resp.alternatives.push(glyph_protocol::MatchAlternative {
                            track: c.track,
                            confidence: conf,
                            preview_url,
                            tag,
                        });
                    }
                }
            }
            resp.alternatives.truncate(15);
        }
        Ok(resp)
    }

    /// Sends a wrong-song selection to the server (if connected and supported).
    pub(super) async fn report_wrong_match(
        &self,
        req: &glyph_protocol::WrongMatchReportRequest,
    ) -> Result<Option<glyph_protocol::WrongMatchReportResponse>> {
        let Some(server) = self.live_server().filter(|_| self.server_wrong_song) else {
            return Ok(None);
        };
        match server.report_wrong_match(req).await {
            Ok(r) => Ok(Some(r)),
            Err(e) => {
                tracing::info!("wrong-match report to server failed: {e:#}");
                Ok(None)
            }
        }
    }

    pub(super) async fn lyrics(&self, t: &Track) -> Result<Option<Lyrics>> {
        let key = lyrics::cache_key(t);
        if let Some(hit) = self.lyrics_cache.get(&key) {
            return Ok(hit);
        }
        let mut found: Option<Lyrics> = None;
        let mut server_answered = false;
        if let Some(server) = self.live_server() {
            match server.lyrics(&t.title, &t.artist, t.album.as_deref(), t.duration_ms.map(|d| d / 1000)).await {
                Ok(r) => {
                    server_answered = true;
                    // The wire type carries no match score.
                    found = r.map(|r| Lyrics::from_response(&r, 1.0));
                }
                // A missing/busy lyrics provider must not break the song view:
                // the local LRCLIB lookup below still has a chance.
                Err(e) => tracing::info!("server lyrics unavailable, falling back: {e:#}"),
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

    pub(super) async fn video_search(
        &self,
        track: &Track,
        query: Option<String>,
    ) -> Result<glyph_protocol::VideoSearchResponse> {
        let explicit = query.as_deref().map(str::trim).filter(|q| !q.is_empty());
        if let Some(server) = self.live_server() {
            if let Ok(resp) = server.video_search(explicit, &track.artist, &track.title).await {
                if !resp.results.is_empty() {
                    return Ok(resp);
                }
            }
        }
        let q = explicit
            .map(str::to_string)
            .unwrap_or_else(|| crate::video::default_query(&track.artist, &track.title));
        crate::video::search_native(&self.http, &q, &track.artist, &track.title, 12).await
    }

    pub(super) async fn video_resolve(&self, id: &str) -> Result<glyph_protocol::VideoResolveResponse> {
        // Prefer direct home-IP resolution when local YouTube is enabled (zero
        // server bandwidth + direct 720p/1080p HD), with server proxy fallback.
        if self.local_youtube_on() || self.live_server().is_none() {
            if let Ok(resp) = crate::video::resolve_native(&self.http, id).await {
                return Ok(resp);
            }
        }
        if let Some(server) = self.live_server() {
            if let Ok(resp) = server.video_resolve(id).await {
                return Ok(resp);
            }
        }
        crate::video::resolve_native(&self.http, id).await
    }
}
