//! Client for `glyph-server` (`/v1/health|search|resolve|lyrics|similar|import/spotify`).
//!
//! Only metadata goes through the server — audio and embeds are always fetched
//! directly from the platform. Every request carries an anonymous, random,
//! locally persisted install id (rate limiting only).

use crate::paths;
use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use glyph_protocol::*;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(4);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// `GET /v1/share/<id>`: a playlist behind a short link.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SharedPlaylist {
    pub code: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    /// Handle of the account that shared it (none for artist albums).
    #[serde(default)]
    pub author: Option<String>,
}

/// What a jam's push socket delivers.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveEvent {
    Msg(glyph_protocol::jam::Live),
    /// The socket closed (with the server's close code, if it sent one).
    Closed(Option<u16>),
}

/// Non-2xx answer of the server.
#[derive(Debug, thiserror::Error)]
#[error("server error {status}: {code} {message}")]
pub struct ServerError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub retry_after_s: Option<u64>,
}

/// Outcome of the start-up compatibility check.
#[derive(Debug, Clone, PartialEq)]
pub enum Compat {
    Compatible(HealthResponse),
    Incompatible(HealthResponse),
}

/// The open `/v1/devices/playback/live` line: what the server pushes, and
/// this app's own state reports going up. `events` ends (after a last
/// `Wake`) when the line closes; dropping either half closes it.
pub struct PlaybackLive {
    pub events: tokio::sync::mpsc::Receiver<glyph_protocol::playback::Live>,
    pub reports: tokio::sync::mpsc::Sender<glyph_protocol::playback::State>,
}

#[derive(Clone)]
pub struct ServerClient {
    base: String,
    http: reqwest::Client,
    install_id: String,
    bearer: Option<String>,
    auth_required: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Random anonymous install id, persisted in the data dir.
pub fn install_id() -> String {
    // Startup/account workers may initialize clients together. Serialize the
    // read-create-publish sequence so they cannot overwrite each other's ID.
    static ID_IO: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ID_IO.lock().unwrap_or_else(|e| e.into_inner());
    let path = paths::data_dir().join("install_id");
    if let Ok(s) = std::fs::read_to_string(&path) {
        let s = s.trim();
        if s.len() >= 16 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return s.to_string();
        }
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    if let Err(e) = paths::write_atomic(&path, id.as_bytes()) {
        tracing::warn!("could not persist install id: {e}");
    }
    id
}

pub fn normalize_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

impl ServerClient {
    /// Only for reads, including metadata POSTs. Never replay account mutations.
    async fn read_json_with_retry<T: serde::de::DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T> {
        for attempt in 0..3u32 {
            let cloned = request.try_clone().context("metadata request cannot be replayed")?;
            let result: Result<T> = async { Ok(Self::check(cloned.send().await?).await?.json().await?) }.await;
            match result {
                Ok(value) => return Ok(value),
                Err(error) => {
                    let Some(wait) = retry_delay(&error, attempt) else { return Err(error) };
                    tokio::time::sleep(wait).await;
                }
            }
        }
        unreachable!("last attempt returns its error")
    }

    pub fn new(url: &str, http: reqwest::Client, install_id: String) -> Self {
        Self { base: normalize_base_url(url), http, install_id, bearer: None, auth_required: Default::default() }
    }

    pub fn with_bearer(mut self, token: Option<String>) -> Result<Self> {
        if token.is_some() && !crate::server_access::secure_base(&self.base) { return Err(anyhow!("protected servers require HTTPS")); }
        // A custom credential header must never follow redirects to another host.
        self.http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(HEALTH_TIMEOUT).user_agent(concat!("Glyph/", env!("CARGO_PKG_VERSION"))).build()?;
        self.bearer = token;
        Ok(self)
    }

    pub fn auth_required(&self) -> bool { self.auth_required.load(std::sync::atomic::Ordering::Relaxed) }
    pub fn media_credentials(&self) -> Option<(String,String)> { self.bearer.as_ref().map(|t| (self.base.clone(),t.clone())) }
    pub fn authorize_url(&self, fallback: reqwest::RequestBuilder, url: &str) -> reqwest::RequestBuilder {
        if crate::server_access::same_server(&self.base,url) {
            let request = self.authorize(fallback);
            match crate::account::cached_access(&self.base) {
                Some(token) => request.bearer_auth(token),
                None => request,
            }
        } else { fallback }
    }
    fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        // Lets the server's operator ban this device (see `hardware`).
        let request = match crate::hardware::id() {
            Some(id) => request.header(glyph_protocol::HARDWARE_ID_HEADER, id),
            None => request,
        };
        // Account authorization is owned by member_request/authed. Appending
        // it here as well creates duplicate Authorization headers (HTTP 400).
        if let Some(token) = &self.bearer {
            let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).unwrap_or_else(|_| reqwest::header::HeaderValue::from_static("invalid"));
            value.set_sensitive(true);
            request.header(crate::server_access::HEADER,value)
        } else { request }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    async fn member_request(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = crate::account::optional_access(self).await {
            request.bearer_auth(token)
        } else { request }
    }

    /// Concrete per-track variant; never rewrite another provider's URL.
    pub fn audio_quality_url(&self, url: &str, quality: crate::config::AudioQuality) -> String {
        if !crate::server_access::same_server(&self.base, url) { return url.into(); }
        let Ok(mut parsed) = reqwest::Url::parse(url) else { return url.into(); };
        if !parsed.path().contains("/v1/audio/youtube/") { return url.into(); }
        let pairs: Vec<_> = parsed.query_pairs().filter(|(k, _)| k != "quality" && k != "adaptive").map(|(k,v)| (k.into_owned(),v.into_owned())).collect();
        parsed.set_query(None);
        parsed.query_pairs_mut().extend_pairs(pairs).append_pair("quality",quality.server_name());
        if quality == crate::config::AudioQuality::Auto { parsed.query_pairs_mut().append_pair("adaptive", "1"); }
        parsed.into()
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.authorize(self.http.get(self.url(path))).header(INSTALL_ID_HEADER, &self.install_id).timeout(REQUEST_TIMEOUT)
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.authorize(self.http.post(self.url(path))).header(INSTALL_ID_HEADER, &self.install_id).timeout(REQUEST_TIMEOUT)
    }

    async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let retry_header = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
        let body = resp.text().await.unwrap_or_default();
        let api: Option<ApiError> = serde_json::from_str(&body).ok();
        // Cloudflare's own page (not JSON) when the Worker's daily requests are used up.
        let edge = api.is_none() && crate::outage::is_daily_limit_page(status.as_u16(), &body);
        Err(ServerError {
            status: status.as_u16(),
            code: api.as_ref().map(|a| a.code.clone()).unwrap_or_else(|| if edge { crate::outage::EDGE_DAILY_LIMIT.into() } else { status.to_string() }),
            message: api.as_ref().map(|a| a.message.clone()).unwrap_or_default(),
            retry_after_s: api.and_then(|a| a.retry_after_s).or(retry_header),
        }
        .into())
    }

    /// `GET /v1/health`.
    pub async fn health(&self) -> Result<HealthResponse> {
        let health: HealthResponse = Self::read_json_with_retry(self.get("/v1/health").timeout(HEALTH_TIMEOUT)).await.context("invalid /v1/health response")?;
        self.auth_required.store(health.server_auth_required, std::sync::atomic::Ordering::Relaxed);
        Ok(health)
    }

    /// Health + protocol-version check.
    pub async fn check_compat(&self) -> Result<Compat> {
        let h = self.health().await?;
        if h.server_auth_required { Self::check(self.get("/v1/access").timeout(HEALTH_TIMEOUT).send().await?).await?; }
        Ok(if h.protocol_version == PROTOCOL_VERSION { Compat::Compatible(h) } else { Compat::Incompatible(h) })
    }

    /// `GET /v1/search?q=` (NDJSON). Forwards events into `tx` as they arrive.
    /// Returns `Ok(true)` if the stream ended with `Done`. Fails early (before
    /// anything was forwarded) if the connection or status is bad, so the
    /// caller can fall back to direct mode.
    pub async fn search_into(&self, query: &str, page: usize, tx: &UnboundedSender<SearchEvent>) -> Result<bool> {
        let req = self.member_request(self.get("/v1/search")).await.query(&[("q", query)]);
        // Older servers ignore `page`; only send it for later pages.
        let req = if page > 0 { req.query(&[("page", page.to_string())]) } else { req };
        let resp = req.send().await?;
        let resp = Self::check(resp).await?;
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut done = false;
        let mut sent = false;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                // Mid-stream failure after results were forwarded: keep them, no fallback.
                Err(_) if sent => return Ok(done),
                Err(e) => return Err(e.into()),
            };
            buf.extend_from_slice(&chunk);
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                match parse_ndjson_line(&line) {
                    Ok(Some(ev)) => {
                        done |= ev == SearchEvent::Done;
                        sent = true;
                        if tx.send(ev).is_err() {
                            return Ok(done);
                        }
                    }
                    Ok(None) => {}
                    Err(_) if sent => return Ok(done),
                    Err(e) => return Err(e),
                }
            }
        }
        if let Ok(Some(ev)) = parse_ndjson_line(&buf) {
            done |= ev == SearchEvent::Done;
            let _ = tx.send(ev);
        }
        Ok(done)
    }

    /// `POST /v1/resolve`.
    pub async fn resolve(&self, track: &Track, priority: &[ProviderId]) -> Result<ResolveResponse> {
        self.resolve_with_mode(track, priority, false).await
    }

    pub async fn resolve_with_mode(&self, track: &Track, priority: &[ProviderId], youtube_only: bool) -> Result<ResolveResponse> {
        let body = ResolveRequest { youtube_only, track: track.clone(), priority: priority.to_vec() };
        let resp = Self::check(self.member_request(self.post("/v1/resolve")).await.json(&body).send().await?).await?;
        let mut resolved: ResolveResponse = resp.json().await?;
        if let Some(PlaybackSource::Stream { url, .. } | PlaybackSource::Preview { url, .. }) = resolved.source.as_mut() {
            if url.starts_with("/v1/audio/") { *url = self.url(url); }
        }
        Ok(resolved)
    }

    /// `POST /v1/youtube/hint`: share "song -> video id" found by the local
    /// yt-dlp so other clients skip their search (ids only, never audio).
    pub async fn report_youtube_hint(&self, track: &Track, video_id: &str) -> Result<YouTubeHintResponse> {
        let video_id = VideoId::parse(video_id).ok_or_else(|| anyhow!("not a YouTube video id"))?;
        let body = YouTubeHintRequest { track: track.clone(), video_id };
        let resp = Self::check(self.member_request(self.post("/v1/youtube/hint")).await.timeout(Duration::from_secs(15)).json(&body).send().await?).await?;
        Ok(resp.json().await?)
    }

    /// `GET /v1/share/<id>`: the playlist behind a short link.
    pub async fn shared_playlist(&self, id: &str) -> Result<SharedPlaylist> {
        let resp = Self::check(self.member_request(self.get(&format!("/v1/share/{id}"))).await.timeout(Duration::from_secs(10)).send().await?).await?;
        Ok(resp.json().await?)
    }

    /// Route public media and metadata through the Worker allowlist.
    pub fn upstream_url(&self, upstream: &str) -> String {
        reqwest::Url::parse_with_params(&self.url("/v1/upstream"), &[("url", upstream)])
            .map(String::from).unwrap_or_else(|_| self.url("/v1/upstream"))
    }

    pub async fn cover_url(&self, track: &Track) -> Result<Option<String>> {
        let resp = Self::check(self.member_request(self.post("/v1/cover")).await.json(track).send().await?).await?;
        let value: serde_json::Value = resp.json().await?;
        Ok(value["url"].as_str().map(|u| self.upstream_url(u)))
    }

    pub async fn download_url(&self, track: &Track) -> Result<Option<(ProviderId, String)>> {
        self.download_url_with_priority(track, &[], false).await
    }

    pub async fn download_url_with_priority(&self, track: &Track, priority: &[ProviderId], youtube_only: bool) -> Result<Option<(ProviderId, String)>> {
        let body = ResolveRequest { youtube_only, track: track.clone(), priority: priority.to_vec() };
        // A cold server first has to find the song (search + companion), which
        // can take longer than the client's normal 30 s; one retry covers a
        // dropped connection or a busy moment.
        let req = self.member_request(self.post("/v1/download")).await.json(&body).timeout(Duration::from_secs(90));
        let v: serde_json::Value = Self::read_json_with_retry(req).await?;
        let value: Option<(ProviderId, String)> = {
            if v.is_null() { None } else { Some((serde_json::from_value(v["provider"].clone())?, v["url"].as_str().context("missing download URL")?.into())) }
        };
        Ok(value.map(|(p,u)| (p, if u.starts_with("/v1/download/") { self.url(&u) } else { u })))
    }

    /// `GET /v1/lyrics`. 404 = no lyrics.
    pub async fn lyrics(
        &self,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_s: Option<u64>,
    ) -> Result<Option<LyricsResponse>> {
        let mut q: Vec<(&str, String)> = vec![("title", title.into()), ("artist", artist.into())];
        if let Some(a) = album {
            q.push(("album", a.into()));
        }
        if let Some(d) = duration_s {
            q.push(("duration", d.to_string()));
        }
        let resp = self.member_request(self.get("/v1/lyrics")).await.query(&q).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(Self::check(resp).await?.json().await?))
    }

    /// `GET /v1/similar`.
    pub async fn similar(&self, artist: &str, title: &str) -> Result<SimilarResponse> {
        let resp = self.member_request(self.get("/v1/similar")).await.query(&[("artist", artist), ("title", title)]).send().await?;
        Ok(Self::check(resp).await?.json().await?)
    }

    /// `POST /v1/import/spotify` (public Spotify / Deezer / YouTube playlist links).
    pub async fn import(&self, url: &str) -> Result<ImportResponse> {
        self.import_with_progress(url, None).await
    }

    pub async fn import_with_progress(&self, url: &str, progress: Option<&crate::flume_like::Sender<crate::ImportProgress>>) -> Result<ImportResponse> {
        #[derive(serde::Deserialize)]
        struct Page {
            #[serde(flatten)]
            list: ImportResponse,
            #[serde(default)]
            next_offset: Option<usize>,
        }
        let max_tracks = self.health().await?.playlist_max_tracks.clamp(1, 50_000);
        crate::library::set_playlist_track_limit(max_tracks);
        let mut offset = 0usize;
        let mut result: Option<ImportResponse> = None;
        loop {
            let request = self.member_request(self.post("/v1/import/spotify")).await
                .timeout(Duration::from_secs(60)).json(&serde_json::json!({"url": url, "offset": offset}));
            let value: serde_json::Value = Self::read_json_with_retry(request).await?;
            let paginated = value.get("next_offset").is_some();
            let page: Page = serde_json::from_value(value)?;
            if !paginated && page.list.tracks.len() >= 100 {
                return Err(anyhow!("Update the Glyph server and Companion, or import locally, to avoid an incomplete playlist"));
            }
            let count = page.list.tracks.len();
            if count > max_tracks || result.as_ref().map_or(0, |r| r.tracks.len()) + count > max_tracks {
                return Err(anyhow!("Playlists may contain at most {max_tracks} songs"));
            }
            match result.as_mut() {
                Some(r) => r.tracks.extend(page.list.tracks),
                None => result = Some(page.list),
            }
            if let Some(progress) = progress {
                let done = result.as_ref().map_or(0, |r| r.tracks.len());
                let total = if page.next_offset.is_none() { done } else { 0 };
                let _ = progress.send(crate::ImportProgress { label: "IMPORT reading playlist".into(), done, total });
            }
            match page.next_offset {
                Some(next) if count <= 100 && next == offset + 100 && next < max_tracks => offset = next,
                Some(_) => return Err(anyhow!("invalid import pagination; no playlist was saved")),
                None => return result.ok_or_else(|| anyhow!("empty import response")),
            }
        }
    }
}

/// Parses one NDJSON line (blank lines ignored).
pub fn parse_ndjson_line(line: &[u8]) -> Result<Option<SearchEvent>> {
    let s = std::str::from_utf8(line).map_err(|e| anyhow!("invalid utf-8 in NDJSON: {e}"))?.trim();
    if s.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(s).with_context(|| format!("bad NDJSON line: {s}"))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndjson_lines() {
        assert_eq!(parse_ndjson_line(b"\n").unwrap(), None);
        assert_eq!(parse_ndjson_line(b"{\"event\":\"done\"}\n").unwrap(), Some(SearchEvent::Done));
        let ev = parse_ndjson_line(br#"{"event":"provider_error","provider":"youtube","message":"quota"}"#).unwrap();
        assert_eq!(
            ev,
            Some(SearchEvent::ProviderError { provider: ProviderId::YouTube, message: "quota".into() })
        );
        assert!(parse_ndjson_line(b"not json").is_err());
    }

    #[test]
    fn base_url_is_normalised() {
        assert_eq!(normalize_base_url(" https://glyph.example/ "), "https://glyph.example");
        let c = ServerClient::new("https://a.b/", reqwest::Client::new(), "id".into());
        assert_eq!(c.url("/v1/health"), "https://a.b/v1/health");
    }

    #[test]
    fn install_id_is_stable() {
        let _g = crate::paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GLYPH_DATA_DIR", dir.path());
        let a = install_id();
        let b = install_id();
        std::env::remove_var("GLYPH_DATA_DIR");
        assert_eq!(a, b);
        assert!(a.len() >= 16);
    }

    #[test]
    fn concurrent_first_clients_share_one_installation_id() {
        let _guard = crate::paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("GLYPH_DATA_DIR");
        std::env::set_var("GLYPH_DATA_DIR", dir.path());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let ids = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8).map(|_| {
                let barrier = barrier.clone();
                scope.spawn(move || { barrier.wait(); install_id() })
            }).collect();
            threads.into_iter().map(|t| t.join().unwrap()).collect::<Vec<_>>()
        });
        let stored = std::fs::read_to_string(dir.path().join("install_id")).unwrap();
        match previous { Some(value) => std::env::set_var("GLYPH_DATA_DIR", value), None => std::env::remove_var("GLYPH_DATA_DIR") }
        assert!(ids.iter().all(|id| id == &stored));
    }
}

// ---------------------------------------------------------------- accounts & sync
//
// Optional server feature (`HealthResponse::accounts`). Calls that need a
// signed-in device take its access token; token refresh and the local session
// live in `crate::account`.

impl ServerClient {
    fn authed(&self, method: reqwest::Method, path: &str, token: &str) -> reqwest::RequestBuilder {
        self.authorize(self.http
            .request(method, self.url(path)))
            .header(INSTALL_ID_HEADER, &self.install_id)
            .bearer_auth(token)
            .timeout(REQUEST_TIMEOUT)
    }

    /// `POST /v1/mcp/token`: a new MCP token for AI assistants (replaces the old one).
    pub async fn mcp_token(&self, token: &str) -> Result<glyph_protocol::mcp::TokenResponse> {
        Self::json_of(self.authed(reqwest::Method::POST, "/v1/mcp/token", token)).await
    }

    /// `GET /v1/mcp/pull`: tool calls waiting for this account (long poll).
    pub async fn mcp_pull(&self, token: &str) -> Result<glyph_protocol::mcp::PullResponse> {
        let wait = Duration::from_millis(glyph_protocol::mcp::PULL_WAIT_MS + 15_000);
        Self::json_of(self.authed(reqwest::Method::GET, "/v1/mcp/pull", token).timeout(wait)).await
    }

    /// `POST /v1/mcp/approve`: the user's answer to an assistant asking for access.
    pub async fn mcp_approve(&self, token: &str, d: &glyph_protocol::mcp::Decision) -> Result<()> {
        Self::check(self.authed(reqwest::Method::POST, "/v1/mcp/approve", token).json(d).send().await?).await?;
        Ok(())
    }

    /// `POST /v1/mcp/result`: the answer to one tool call.
    pub async fn mcp_result(&self, token: &str, r: &glyph_protocol::mcp::ResultRequest) -> Result<()> {
        Self::check(self.authed(reqwest::Method::POST, "/v1/mcp/result", token).json(r).send().await?).await?;
        Ok(())
    }

    /// `POST /v1/jam`: starts a jam hosted by this account.
    pub async fn jam_start(&self, token: &str) -> Result<glyph_protocol::jam::State> {
        Self::json_of(self.authed(reqwest::Method::POST, "/v1/jam", token)).await
    }

    /// `GET /v1/jam/{code}?rev=`: the jam's state once it differs from `rev`
    /// (long poll).
    pub async fn jam_state(&self, token: &str, code: &str, rev: u64) -> Result<glyph_protocol::jam::State> {
        let wait = Duration::from_millis(glyph_protocol::jam::WAIT_MS + 15_000);
        let mut state: glyph_protocol::jam::State = Self::json_of(self.authed(reqwest::Method::GET, &format!("/v1/jam/{code}?rev={rev}"), token).timeout(wait)).await?;
        state.sanitize();
        Ok(state)
    }

    /// In (the state), or waiting for the host to let us in.
    pub async fn jam_join(&self, token: &str, code: &str) -> Result<glyph_protocol::jam::JoinResponse> {
        let mut joined: glyph_protocol::jam::JoinResponse = Self::json_of(self.authed(reqwest::Method::POST, &format!("/v1/jam/{code}/join"), token)).await?;
        if let Some(state) = joined.state.as_mut() {
            state.sanitize();
        }
        Ok(joined)
    }

    /// `GET /v1/jam/{code}/time`: the server clock.
    pub async fn jam_time(&self, token: &str, code: &str) -> Result<glyph_protocol::jam::TimeResponse> {
        Self::json_of(self.authed(reqwest::Method::GET, &format!("/v1/jam/{code}/time"), token).timeout(Duration::from_secs(10))).await
    }

    /// The host lets someone in (or not).
    pub async fn jam_admit(&self, token: &str, code: &str, handle: &str, allow: bool) -> Result<()> {
        let body = glyph_protocol::jam::AdmitRequest { handle: handle.to_string(), allow };
        Self::check(self.authed(reqwest::Method::POST, &format!("/v1/jam/{code}/admit"), token).json(&body).send().await?).await?;
        Ok(())
    }

    /// Takes over a jam whose host is gone; answers the state.
    pub async fn jam_claim(&self, token: &str, code: &str) -> Result<glyph_protocol::jam::State> {
        let mut state: glyph_protocol::jam::State = Self::json_of(self.authed(reqwest::Method::POST, &format!("/v1/jam/{code}/claim"), token)).await?;
        state.sanitize();
        Ok(state)
    }

    /// Leaves the jam; the host ends it.
    pub async fn jam_leave(&self, token: &str, code: &str) -> Result<()> {
        Self::check(self.authed(reqwest::Method::POST, &format!("/v1/jam/{code}/leave"), token).send().await?).await?;
        Ok(())
    }

    pub async fn jam_op(&self, token: &str, code: &str, op: &glyph_protocol::jam::Op) -> Result<()> {
        Self::check(self.authed(reqwest::Method::POST, &format!("/v1/jam/{code}/op"), token).json(op).send().await?).await?;
        Ok(())
    }

    /// `GET /v1/jam/{code}/ops`: the members' requests (host, long poll;
    /// `now`: only what is there).
    pub async fn jam_ops(&self, token: &str, code: &str, now: bool) -> Result<glyph_protocol::jam::OpsResponse> {
        let mut ops: glyph_protocol::jam::OpsResponse = if now {
            Self::json_of(self.authed(reqwest::Method::GET, &format!("/v1/jam/{code}/ops?wait=0"), token)).await?
        } else {
            let wait = Duration::from_millis(glyph_protocol::jam::OPS_WAIT_MS + 15_000);
            Self::json_of(self.authed(reqwest::Method::GET, &format!("/v1/jam/{code}/ops"), token).timeout(wait)).await?
        };
        ops.sanitize();
        Ok(ops)
    }

    /// `GET /v1/jam/{code}/live`: the server's pushes while the WebSocket
    /// is open. Ends with [`LiveEvent::Closed`]; dropping the receiver
    /// closes the socket. Servers without pushes answer `jam_push_off`.
    pub async fn jam_live(&self, token: &str, code: &str) -> Result<tokio::sync::mpsc::Receiver<LiveEvent>> {
        use futures::SinkExt;
        use glyph_protocol::jam::{Live, LIVE_PING, LIVE_PING_MS};
        use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Error as WsError, Message};
        let url = self.url(&format!("/v1/jam/{code}/live"));
        let url = match url.strip_prefix("https://") {
            Some(rest) => format!("wss://{rest}"),
            None => format!("ws://{}", url.trim_start_matches("http://")),
        };
        let mut req = url.into_client_request()?;
        let h = req.headers_mut();
        h.insert(reqwest::header::AUTHORIZATION, format!("Bearer {token}").parse()?);
        h.insert(INSTALL_ID_HEADER, self.install_id.parse()?);
        if let Some(id) = crate::hardware::id() {
            h.insert(glyph_protocol::HARDWARE_ID_HEADER, id.parse()?);
        }
        if let Some(b) = &self.bearer {
            let mut v: reqwest::header::HeaderValue = format!("Bearer {b}").parse()?;
            v.set_sensitive(true);
            h.insert(crate::server_access::HEADER, v);
        }
        let ws = match tokio::time::timeout(Duration::from_secs(20), tokio_tungstenite::connect_async(req)).await {
            Err(_) => return Err(anyhow!("jam live: connecting timed out")),
            Ok(Ok((ws, _))) => ws,
            // Refused before the upgrade: the server's answer, like any call.
            Ok(Err(WsError::Http(resp))) => {
                let body = resp.body().as_deref().unwrap_or_default();
                let api: Option<ApiError> = serde_json::from_slice(body).ok();
                return Err(ServerError {
                    status: resp.status().as_u16(),
                    code: api.as_ref().map(|a| a.code.clone()).unwrap_or_else(|| resp.status().to_string()),
                    message: api.as_ref().map(|a| a.message.clone()).unwrap_or_default(),
                    retry_after_s: api.and_then(|a| a.retry_after_s),
                }
                .into());
            }
            Ok(Err(e)) => return Err(anyhow!("jam live: {e}")),
        };
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        tokio::spawn(async move {
            let (mut sink, mut stream) = ws.split();
            let every = Duration::from_millis(LIVE_PING_MS);
            let mut ping = tokio::time::interval(every);
            ping.tick().await;
            let mut heard = tokio::time::Instant::now();
            let closed = loop {
                tokio::select! {
                    _ = tx.closed() => {
                        let _ = sink.send(Message::Close(None)).await;
                        return;
                    }
                    _ = ping.tick() => {
                        // No answer to the last keepalives: the line is dead.
                        if heard.elapsed() > every * 2 + Duration::from_secs(10) {
                            break None;
                        }
                        if sink.send(Message::Text(LIVE_PING.into())).await.is_err() {
                            break None;
                        }
                    }
                    m = stream.next() => {
                        heard = tokio::time::Instant::now();
                        match m {
                            Some(Ok(Message::Text(t))) => {
                                if let Ok(mut live) = serde_json::from_str::<Live>(t.as_str()) {
                                    match &mut live {
                                        Live::State { state } => state.sanitize(),
                                        Live::Now { now, .. } => now.sanitize(),
                                        _ => {}
                                    }
                                    if tx.send(LiveEvent::Msg(live)).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            Some(Ok(Message::Close(frame))) => break frame.map(|f| u16::from(f.code)),
                            Some(Ok(_)) => {}
                            Some(Err(_)) | None => break None,
                        }
                    }
                }
            };
            let _ = tx.send(LiveEvent::Closed(closed)).await;
        });
        Ok(rx)
    }

    pub async fn playback_live(&self, token: &str) -> Result<PlaybackLive> {
        use futures::SinkExt;
        use glyph_protocol::jam::{LIVE_PING, LIVE_PING_MS};
        use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Error as WsError, Message};
        let url = self.url("/v1/devices/playback/live");
        let url = match url.strip_prefix("https://") {
            Some(rest) => format!("wss://{rest}"),
            None => format!("ws://{}", url.trim_start_matches("http://")),
        };
        let mut req = url.into_client_request()?;
        let h = req.headers_mut();
        h.insert(reqwest::header::AUTHORIZATION, format!("Bearer {token}").parse()?);
        h.insert(INSTALL_ID_HEADER, self.install_id.parse()?);
        if let Some(id) = crate::hardware::id() {
            h.insert(glyph_protocol::HARDWARE_ID_HEADER, id.parse()?);
        }
        if let Some(b) = &self.bearer {
            let mut v: reqwest::header::HeaderValue = format!("Bearer {b}").parse()?;
            v.set_sensitive(true);
            h.insert(crate::server_access::HEADER, v);
        }
        let ws = match tokio::time::timeout(Duration::from_secs(20), tokio_tungstenite::connect_async(req)).await {
            Err(_) => return Err(anyhow!("playback live: connecting timed out")),
            Ok(Ok((ws, _))) => ws,
            // Refused before the upgrade: the server's answer, like any call.
            Ok(Err(WsError::Http(resp))) => {
                let body = resp.body().as_deref().unwrap_or_default();
                let api: Option<ApiError> = serde_json::from_slice(body).ok();
                return Err(ServerError {
                    status: resp.status().as_u16(),
                    code: api.as_ref().map(|a| a.code.clone()).unwrap_or_else(|| resp.status().to_string()),
                    message: api.as_ref().map(|a| a.message.clone()).unwrap_or_default(),
                    retry_after_s: api.and_then(|a| a.retry_after_s),
                }
                .into());
            }
            Ok(Err(e)) => return Err(anyhow!("playback live: {e}")),
        };
        use glyph_protocol::playback::Live;
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        let (report_tx, mut reports) = tokio::sync::mpsc::channel::<glyph_protocol::playback::State>(4);
        tokio::spawn(async move {
            let (mut sink, mut stream) = ws.split();
            let every = Duration::from_millis(LIVE_PING_MS);
            let mut ping = tokio::time::interval(every);
            ping.tick().await;
            let mut heard = tokio::time::Instant::now();
            let _closed = loop {
                tokio::select! {
                    _ = tx.closed() => {
                        let _ = sink.send(Message::Close(None)).await;
                        return;
                    }
                    report = reports.recv() => {
                        let Some(state) = report else {
                            let _ = sink.send(Message::Close(None)).await;
                            return;
                        };
                        let text = serde_json::to_string(&Live::Report { state }).unwrap_or_default();
                        if sink.send(Message::Text(text.into())).await.is_err() {
                            break None;
                        }
                    }
                    _ = ping.tick() => {
                        // No answer to the last keepalives: the line is dead.
                        if heard.elapsed() > every * 2 + Duration::from_secs(10) {
                            break None;
                        }
                        if sink.send(Message::Text(LIVE_PING.into())).await.is_err() {
                            break None;
                        }
                    }
                    m = stream.next() => {
                        heard = tokio::time::Instant::now();
                        match m {
                            Some(Ok(Message::Text(t))) => {
                                // Older servers only send `wake`; unknown messages are skipped.
                                if let Ok(live) = serde_json::from_str::<Live>(t.as_str()) {
                                    if tx.send(live).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            Some(Ok(Message::Close(frame))) => break frame.map(|f| u16::from(f.code)),
                            Some(Ok(_)) => {}
                            Some(Err(_)) | None => break None,
                        }
                    }
                }
            };
            // Whatever waited while the line was down: ask once more.
            let _ = tx.send(Live::Wake).await;
        });
        Ok(PlaybackLive { events: rx, reports: report_tx })
    }

    /// `POST /v1/jam/{code}/state`: publishes the host's state.
    pub async fn jam_publish(&self, token: &str, state: &glyph_protocol::jam::State) -> Result<()> {
        Self::check(self.authed(reqwest::Method::POST, &format!("/v1/jam/{}/state", state.code), token).json(state).send().await?).await?;
        Ok(())
    }

    /// `POST /v1/jam/{code}/now`: only the position moved (a few hundred
    /// bytes instead of the whole state). Older servers answer `not_found`.
    pub async fn jam_now(&self, token: &str, code: &str, now: &glyph_protocol::jam::Now) -> Result<()> {
        Self::check(self.authed(reqwest::Method::POST, &format!("/v1/jam/{code}/now"), token).json(now).send().await?).await?;
        Ok(())
    }

    pub async fn jam_kick(&self, token: &str, code: &str, handle: &str) -> Result<()> {
        let body = glyph_protocol::jam::KickRequest { handle: handle.to_string() };
        Self::check(self.authed(reqwest::Method::POST, &format!("/v1/jam/{code}/kick"), token).json(&body).send().await?).await?;
        Ok(())
    }

    async fn json_of<T: serde::de::DeserializeOwned>(rb: reqwest::RequestBuilder) -> Result<T> {
        Ok(Self::check(rb.send().await?).await?.json().await?)
    }

    pub async fn login_start(&self, device_name: &str, platform: &str, pake: Option<String>) -> Result<LoginStartResponse> {
        let body = LoginStartRequest { device_name: device_name.into(), platform: platform.into(), pake };
        Self::json_of(self.post("/v1/account/login/start").json(&body)).await
    }

    pub async fn login_poll(&self, login_id: &str, poll_secret: &str) -> Result<LoginPollResponse> {
        let body = LoginPollRequest { login_id: login_id.into(), poll_secret: poll_secret.into() };
        Self::json_of(self.post("/v1/account/login/poll").json(&body)).await
    }

    pub async fn password_salt(&self, handle: &str) -> Result<PasswordSalt> {
        let h: String = handle.trim().trim_start_matches('@').chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        Self::json_of(self.get(&format!("/v1/account/password/salt/{h}"))).await
    }

    pub async fn password_register(&self, body: &PasswordRegisterRequest) -> Result<SignupResponse> {
        Self::json_of(self.post("/v1/account/password/register").json(body)).await
    }

    pub async fn password_login(&self, body: &PasswordLoginRequest) -> Result<Session> {
        Self::json_of(self.post("/v1/account/password/login").json(body)).await
    }

    /// `POST /v1/share` (signed in): stores a playlist code under the
    /// account and returns its short link.
    pub async fn share_playlist(&self, token: &str, code: &str, name: &str, songs: usize, kind: &str) -> Result<String> {
        let body = serde_json::json!({ "code": code, "name": name, "songs": songs, "kind": kind });
        let value: serde_json::Value = Self::json_of(self.authed(reqwest::Method::POST, "/v1/share", token).json(&body)).await?;
        value["url"].as_str().map(str::to_string).ok_or_else(|| anyhow!("server sent no link"))
    }

    pub async fn set_password(&self, token: &str, body: &PasswordSetRequest) -> Result<Account> {
        Self::json_of(self.authed(reqwest::Method::PUT, "/v1/account/password", token).json(body)).await
    }

    pub async fn password_check(&self, token: &str, key: &str) -> Result<()> {
        let body = glyph_protocol::PasswordCheckRequest { key: key.into() };
        Self::json_of::<serde_json::Value>(self.authed(reqwest::Method::POST, "/v1/account/password/check", token).json(&body)).await.map(|_| ())
    }

    pub async fn passkey_link(&self, token: &str) -> Result<LinkStartResponse> {
        Self::json_of(self.authed(reqwest::Method::POST, "/v1/account/passkey/link", token)).await
    }

    pub async fn refresh_session(&self, refresh_token: &str) -> Result<Session> {
        Self::json_of(self.post("/v1/account/token/refresh").timeout(Duration::from_secs(10)).json(&RefreshRequest { refresh_token: refresh_token.into() })).await
    }

    pub async fn handle_check(&self, handle: &str) -> Result<HandleCheck> {
        let h: String = handle.trim().trim_start_matches('@').chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        Self::json_of(self.get(&format!("/v1/account/handle/{h}"))).await
    }

    pub async fn account_get(&self, token: &str) -> Result<Account> {
        Self::json_of(self.authed(reqwest::Method::GET, "/v1/account", token)).await
    }

    pub async fn account_patch(&self, token: &str, patch: &AccountPatch) -> Result<Account> {
        Self::json_of(self.authed(reqwest::Method::PATCH, "/v1/account", token).json(patch)).await
    }

    pub async fn account_delete(&self, token: &str, confirm_handle: &str) -> Result<()> {
        let body = DeleteAccountRequest { confirm_handle: confirm_handle.into() };
        Self::check(self.authed(reqwest::Method::DELETE, "/v1/account", token).json(&body).send().await?).await?;
        Ok(())
    }

    pub async fn new_recovery_code(&self, token: &str) -> Result<String> {
        let v: serde_json::Value = Self::json_of(self.authed(reqwest::Method::POST, "/v1/account/recovery_code", token)).await?;
        v["recovery_code"].as_str().map(str::to_string).context("missing recovery_code")
    }

    pub async fn approve_login(&self, token: &str, user_code: &str) -> Result<ApproveResponse> {
        let body = ApproveRequest { user_code: user_code.into() };
        Self::json_of(self.authed(reqwest::Method::POST, "/v1/account/login/approve", token).json(&body)).await
    }

    pub async fn devices(&self, token: &str) -> Result<DevicesResponse> {
        Self::json_of(self.authed(reqwest::Method::GET, "/v1/devices", token)).await
    }

    pub async fn playback_poll(&self, token: &str, state: &glyph_protocol::playback::State) -> Result<glyph_protocol::playback::Poll> {
        Self::json_of(self.authed(reqwest::Method::POST, "/v1/devices/playback/poll", token).json(state)).await
    }

    pub async fn playback_command(&self, token: &str, send: &glyph_protocol::playback::Send) -> Result<()> {
        let response: serde_json::Value = Self::json_of(self.authed(reqwest::Method::POST, "/v1/devices/playback/command?confirm=1", token).json(send)).await?;
        let id = response["id"].as_str().ok_or_else(|| anyhow!("Target did not confirm support for acknowledgements"))?.to_string();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        loop {
            let receipt: glyph_protocol::playback::Receipt = Self::json_of(self.authed(reqwest::Method::POST, "/v1/devices/playback/receipt", token).json(&glyph_protocol::playback::Receipt { id: id.clone(), ..Default::default() })).await?;
            if receipt.done { return receipt.error.map_or(Ok(()), |e| Err(anyhow!(e))); }
            if tokio::time::Instant::now() >= deadline { return Err(anyhow!("Device did not confirm the command in time")); }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    pub async fn playback_ack(&self, token: &str, receipt: &glyph_protocol::playback::Receipt) -> Result<()> {
        Self::check(self.authed(reqwest::Method::POST, "/v1/devices/playback/ack", token).json(receipt).send().await?).await?;
        Ok(())
    }

    pub async fn remove_device(&self, token: &str, id: &str) -> Result<()> {
        let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        Self::check(self.authed(reqwest::Method::DELETE, &format!("/v1/devices/{id}"), token).send().await?).await?;
        Ok(())
    }

    pub async fn sign_out_device(&self, token: &str) -> Result<()> {
        Self::check(self.authed(reqwest::Method::POST, "/v1/devices/self/signout", token).send().await?).await?;
        Ok(())
    }

    pub async fn sync_push(&self, token: &str, items: Vec<SyncItem>) -> Result<SyncPushResponse> {
        Self::json_of(self.authed(reqwest::Method::POST, "/v1/sync/push", token).json(&SyncPushRequest { items })).await
    }

    pub async fn sync_pull(&self, token: &str, since: u64) -> Result<SyncPullResponse> {
        Self::json_of(self.authed(reqwest::Method::GET, "/v1/sync/pull", token).query(&[("since", since)])).await
    }

    pub async fn upload_cover(&self, token: &str, sha256: &str, bytes: Vec<u8>) -> Result<()> {
        Self::check(self.authed(reqwest::Method::PUT, &format!("/v1/sync/covers/{sha256}"), token).body(bytes).send().await?).await?;
        Ok(())
    }

    pub async fn download_cover(&self, token: &str, sha256: &str) -> Result<Vec<u8>> {
        let resp = Self::check(self.authed(reqwest::Method::GET, &format!("/v1/sync/covers/{sha256}"), token).send().await?).await?;
        Ok(resp.bytes().await?.to_vec())
    }

    // ---------------------------------------------------------- profiles

    pub async fn profile_settings(&self, token: &str) -> Result<ProfileSettings> {
        Self::json_of(self.authed(reqwest::Method::GET, "/v1/profile", token)).await
    }

    pub async fn profile_patch(&self, token: &str, patch: &ProfilePatch) -> Result<ProfileSettings> {
        Self::json_of(self.authed(reqwest::Method::PATCH, "/v1/profile", token).json(patch)).await
    }

    /// `banner`: the banner instead of the picture; `None` removes it.
    pub async fn profile_image(&self, token: &str, banner: bool, bytes: Option<Vec<u8>>) -> Result<ProfileSettings> {
        let path = if banner { "/v1/profile/banner" } else { "/v1/profile/avatar" };
        let req = match bytes {
            Some(b) => self.authed(reqwest::Method::PUT, path, token).body(b),
            None => self.authed(reqwest::Method::DELETE, path, token),
        };
        Self::json_of(req).await
    }

    pub async fn profile_recent(&self, token: &str, tracks: Vec<Track>) -> Result<()> {
        Self::check(self.authed(reqwest::Method::PUT, "/v1/profile/recent", token).json(&ProfileRecent { tracks }).send().await?).await?;
        Ok(())
    }

    pub async fn profile(&self, token: &str, handle: &str) -> Result<PublicProfile> {
        Self::json_of(self.authed(reqwest::Method::GET, &format!("/v1/profiles/{}", clean_handle(handle)), token)).await
    }

    pub async fn profile_playlist(&self, token: &str, handle: &str, id: &str) -> Result<ProfilePlaylistSongs> {
        let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        let mut list: ProfilePlaylistSongs = Self::json_of(self.authed(reqwest::Method::GET, &format!("/v1/profiles/{}/playlists/{id}", clean_handle(handle)), token)).await?;
        // Another account's playlist: its tracks are claims.
        list.tracks.iter_mut().for_each(Track::sanitize);
        Ok(list)
    }

    pub async fn profile_image_bytes(&self, token: &str, handle: &str, sha256: &str) -> Result<Vec<u8>> {
        let sha: String = sha256.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        let resp = Self::check(self.authed(reqwest::Method::GET, &format!("/v1/profiles/{}/images/{sha}", clean_handle(handle)), token).send().await?).await?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// The server's cached copy of a cover image (allow-listed hosts only).
    pub fn cached_cover_url(&self, url: &str) -> Option<String> {
        use sha2::Digest;
        if !cover_cache_allowed(url) {
            return None;
        }
        let name: String = sha2::Sha256::digest(url.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        reqwest::Url::parse_with_params(&self.url(&format!("/v1/covers/{name}")), &[("u", url)]).ok().map(String::from)
    }
}

/// `@Name` -> `name`, only the characters a handle can have.
fn clean_handle(h: &str) -> String {
    h.trim().trim_start_matches('@').chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>().to_ascii_lowercase()
}

/// HTTP status and API code of a failed server call, if it was one.
pub fn server_error(e: &anyhow::Error) -> Option<(u16, &str)> {
    e.downcast_ref::<ServerError>().map(|s| (s.status, s.code.as_str()))
}

fn retry_delay(error: &anyhow::Error, attempt: u32) -> Option<Duration> {
    if attempt >= 2 { return None; }
    let fallback = 1 << attempt;
    if let Some(error) = error.downcast_ref::<ServerError>() {
        if error.status != 429 && !(500..=599).contains(&error.status) { return None; }
        return Some(Duration::from_secs(error.retry_after_s.unwrap_or(if error.status == 429 { 5 * fallback } else { fallback }).clamp(1, 120)));
    }
    error.downcast_ref::<reqwest::Error>()
        .filter(|e| e.is_timeout() || e.is_connect() || e.is_body() || e.is_decode())
        .map(|_| Duration::from_secs(fallback))
}

#[cfg(test)]
mod account_tests {
    use super::*;

    #[test]
    fn metadata_retries_only_transient_errors_and_honours_retry_after() {
        let error = |status, retry_after_s| anyhow::Error::new(ServerError { status, retry_after_s, code: String::new(), message: String::new() });
        for status in [400, 401, 403, 404, 409] { assert_eq!(retry_delay(&error(status, None), 0), None); }
        assert_eq!(retry_delay(&error(503, None), 0), Some(Duration::from_secs(1)));
        assert_eq!(retry_delay(&error(503, None), 1), Some(Duration::from_secs(2)));
        assert_eq!(retry_delay(&error(429, Some(12)), 0), Some(Duration::from_secs(12)));
        assert_eq!(retry_delay(&error(429, Some(u64::MAX)), 0), Some(Duration::from_secs(120)));
        assert_eq!(retry_delay(&error(503, None), 2), None);
    }

    #[tokio::test]
    async fn metadata_read_retries_a_cut_off_json_body() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            for response in [
                "HTTP/1.1 200 OK\r\nContent-Length: 99\r\nConnection: close\r\n\r\n{",
                "HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request);
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let request = reqwest::Client::new().post(format!("http://{addr}")).json(&serde_json::json!({"offset":100})).timeout(Duration::from_secs(3));
        let result: serde_json::Value = ServerClient::read_json_with_retry(request).await.unwrap();
        thread.join().unwrap();
        assert_eq!(result["ok"], true);
    }

    #[test]
    fn cover_cache_urls() {
        let c = ServerClient::new("https://srv.example", reqwest::Client::new(), "id".into());
        let u = c.cached_cover_url("https://i.ytimg.com/vi/x/hq.jpg").unwrap();
        assert!(u.starts_with("https://srv.example/v1/covers/"));
        assert!(u.contains("u=https%3A%2F%2Fi.ytimg.com"));
        assert_eq!(c.cached_cover_url("https://example.com/x.jpg"), None);
    }
}

#[cfg(test)]
mod access_tests {
    use super::*;
    #[test]
    fn signed_in_requests_have_exactly_one_authorization_header() {
        let _env = crate::paths::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let base = "https://header-regression.invalid";
        crate::account::with_test_access(base, "test-token", || {
            let duplicate = reqwest::Client::new().get(base).bearer_auth("test-token")
                .bearer_auth("test-token").build().unwrap();
            assert_eq!(duplicate.headers().get_all(reqwest::header::AUTHORIZATION).iter().count(), 2);
            let c = ServerClient::new(base, reqwest::Client::new(), "install".into());
            let account = c.authed(reqwest::Method::GET, "/v1/devices", "test-token").build().unwrap();
            let search = futures::executor::block_on(c.member_request(c.get("/v1/search"))).build().unwrap();
            let media_url = format!("{base}/v1/audio/youtube/test");
            let media = c.authorize_url(reqwest::Client::new().get(&media_url), &media_url).build().unwrap();
            let head = c.authorize_url(reqwest::Client::new().head(&media_url).header("if-none-match", "revision-1"), &media_url).build().unwrap();
            assert_eq!(head.method(), reqwest::Method::HEAD);
            assert_eq!(head.headers()["if-none-match"], "revision-1");
            for request in [account, search, media] {
                assert_eq!(request.headers().get_all(reqwest::header::AUTHORIZATION).iter().count(), 1);
                assert_eq!(request.headers()[reqwest::header::AUTHORIZATION], "Bearer test-token");
            }
        });
    }
    #[test]
    fn credentials_are_scoped_independent_and_sensitive() {
        let c = ServerClient::new("https://private.example",reqwest::Client::new(),"install".into()).with_bearer(Some("secret-server-bearer-with-at-least-32-characters".into())).unwrap();
        let request = c.authed(reqwest::Method::GET,"/v1/profile","account-access").build().unwrap();
        assert_eq!(request.headers()[reqwest::header::AUTHORIZATION],"Bearer account-access");
        let value = &request.headers()[crate::server_access::HEADER];
        assert!(value.is_sensitive());
        assert!(!format!("{request:?}").contains("secret-server-bearer"));
        let outside = "https://private.example.evil/v1/audio/x";
        let request = c.authorize_url(reqwest::Client::new().get(outside),outside).build().unwrap();
        assert!(request.headers().get(crate::server_access::HEADER).is_none());
        assert!(ServerClient::new("http://private.example",reqwest::Client::new(),String::new()).with_bearer(Some("secret".into())).is_err());
    }
}
