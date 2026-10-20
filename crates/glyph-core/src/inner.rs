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
    pub(super) fn local_youtube_on(&self) -> bool {
        (self.config.providers.youtube_mode == config::YouTubeMode::Local || self.server.is_none())
            && self.providers.youtube.as_ref().is_some_and(|y| y.local().is_some())
    }

    /// YouTube through the local yt-dlp (home IP, no bot check): the song's
    /// own video id, a cached mapping, the id other users shared through the
    /// server (`hint`), else a search for "Artist - Title" — a sure search
    /// hit is shared back so the next user skips the search.
    pub(super) async fn local_youtube_match(&self, t: &Track, hint: Option<&str>) -> Option<(Track, f32)> {
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
            Some((m, c)) if c >= resolver::SURE_CONFIDENCE => m.id,
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

    pub(super) async fn local_youtube(&self, t: &Track, hint: Option<&str>) -> Option<Resolved> {
        let (matched, confidence) = self.local_youtube_match(t, hint).await?;
        let yt = self.providers.youtube.as_ref()?;
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
    pub(super) async fn prefetch(&self, tracks: Vec<Track>) {
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

    pub(super) async fn resolve(&self, t: &Track) -> Result<Option<Resolved>> {
        if self.config.providers.youtube_mode == config::YouTubeMode::Local
            && self.config.providers.enabled.contains(&ProviderId::YouTube)
            && (self.config.providers.youtube_only || t.source == ProviderId::YouTube || self.resolver.priority().first() == Some(&ProviderId::YouTube))
        {
            return self.local_youtube(t, None).await.map(Some)
                .ok_or_else(|| anyhow!("Local YouTube playback failed. Check yt-dlp in Settings > Playback > Sources."));
        }
        if let Some(server) = &self.server {
            let local_yt = self.local_youtube_on();
            let priority = self.resolver.priority();
            let rank = |p: ProviderId| priority.iter().position(|x| *x == p).unwrap_or(usize::MAX);
            // The server would search YouTube for this song first (seconds on
            // a cold server); a search from here takes a few hundred ms. A sure
            // match travels along as the song's YouTube id.
            let hinted;
            let t = if self.config.providers.youtube_only || priority.first() == Some(&ProviderId::YouTube) {
                hinted = self.youtube_id_hint(t).await;
                hinted.as_ref().unwrap_or(t)
            } else {
                t
            };
            match server.resolve_with_mode(t, &priority, self.config.providers.youtube_only).await {
                Ok(resp) => {
                    let hint = resp.youtube_hint.as_deref();
                    if let Some(source) = resp.source.clone() {
                        if self.config.providers.youtube_only && (source.provider() != ProviderId::YouTube || matches!(source, PlaybackSource::Preview { .. })) {
                            return Err(anyhow!("Song not found on YouTube."));
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
                        if !matches!(source, PlaybackSource::Preview { .. }) {
                            self.resolver.remember(t, &matched, resp.confidence);
                        }
                        return Ok(Some(Resolved { track: matched, source, confidence: resp.confidence }));
                    }
                    if local_yt {
                        return Ok(self.local_youtube(t, hint).await);
                    }
                    if self.config.providers.youtube_only { return Err(anyhow!("Song not found on YouTube.")); }
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
        if self.config.providers.youtube_only {
            return self.local_youtube(t, None).await.map(Some).ok_or_else(|| anyhow!("Song not found on YouTube."));
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
    pub(super) async fn download_url(&self, t: &Track) -> Result<Option<(ProviderId, String)>> {
        if self.config.providers.youtube_mode == config::YouTubeMode::Local
            && (self.config.providers.youtube_only || t.source == ProviderId::YouTube || self.resolver.priority().first() == Some(&ProviderId::YouTube)) {
            if let Some((matched, _)) = self.local_youtube_match(t, None).await {
                return Ok(self.download_at(ProviderId::YouTube, &matched).await.map(|url| (ProviderId::YouTube, url)));
            }
            return Err(anyhow!("Local YouTube download failed. Check yt-dlp in Settings > Playback > Sources."));
        }
        if let Some(server) = &self.server {
            let lookup = match server.download_url_with_priority(t, &self.resolver.priority(), self.config.providers.youtube_only).await {
                Ok(hit) => hit,
                Err(e) if self.config.providers.youtube_mode == config::YouTubeMode::Server => return Err(e),
                Err(e) => { tracing::debug!("server download lookup failed: {e}"); None }
            };
            if let Some(hit) = lookup {
                if self.config.providers.youtube_only && hit.0 != ProviderId::YouTube {
                    return Err(anyhow!("Song not found on YouTube."));
                }
                // The server picks the normal Archive file; another quality
                // needs the item's file list.
                if hit.0 == ProviderId::Archive && downloads::job_quality() != config::AudioQuality::Standard {
                    if let Some(u) = self.download_at(ProviderId::Archive, t).await {
                        return Ok(Some((ProviderId::Archive, u)));
                    }
                }
                if hit.0 == ProviderId::YouTube {
                    let quality = match downloads::job_quality() { config::AudioQuality::Low => "low", config::AudioQuality::High => "high", _ => "standard" };
                    return Ok(Some((hit.0, format!("{}?quality={quality}", hit.1))));
                }
                return Ok(Some(hit));
            }
            if self.config.providers.youtube_only { return Err(anyhow!("Song not found on YouTube.")); }
            // Same path as playback: YouTube through the local yt-dlp.
            if let Some(Resolved { source: PlaybackSource::Stream { provider, url, .. }, .. }) = self.local_youtube(t, None).await {
                return Ok(Some((provider, url)));
            }
            return Ok(None);
        }
        if self.config.providers.youtube_only { return Err(anyhow!("Song not found on YouTube.")); }
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

    pub(super) async fn download_at(&self, p: ProviderId, t: &Track) -> Option<String> {
        match self.providers.get(p)?.download_url(t).await {
            Ok(u) => u,
            Err(e) => {
                tracing::info!(provider = ?p, "download lookup failed: {e}");
                None
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
        if let Some(server) = &self.server {
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
}
