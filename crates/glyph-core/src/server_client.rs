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
        Err(ServerError {
            status: status.as_u16(),
            code: api.as_ref().map(|a| a.code.clone()).unwrap_or_else(|| status.to_string()),
            message: api.as_ref().map(|a| a.message.clone()).unwrap_or_default(),
            retry_after_s: api.and_then(|a| a.retry_after_s).or(retry_header),
        }
        .into())
    }

    /// `GET /v1/health`.
    pub async fn health(&self) -> Result<HealthResponse> {
        let resp = self.get("/v1/health").timeout(HEALTH_TIMEOUT).send().await?;
        let resp = Self::check(resp).await?;
        let health: HealthResponse = resp.json().await.context("invalid /v1/health response")?;
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
        let body = YouTubeHintRequest { track: track.clone(), video_id: video_id.to_string() };
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
        let resp = Self::check(self.member_request(self.post("/v1/download")).await.json(&body).send().await?).await?;
        let value: Option<(ProviderId, String)> = {
            let v: serde_json::Value = resp.json().await?;
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
        let body = ImportRequest { url: url.to_string() };
        let resp = Self::check(self.member_request(self.post("/v1/import/spotify")).await.json(&body).send().await?).await?;
        Ok(resp.json().await?)
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

    async fn json_of<T: serde::de::DeserializeOwned>(rb: reqwest::RequestBuilder) -> Result<T> {
        Ok(Self::check(rb.send().await?).await?.json().await?)
    }

    pub async fn login_start(&self, device_name: &str, platform: &str) -> Result<LoginStartResponse> {
        let body = LoginStartRequest { device_name: device_name.into(), platform: platform.into() };
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
        Self::json_of(self.authed(reqwest::Method::GET, &format!("/v1/profiles/{}/playlists/{id}", clean_handle(handle)), token)).await
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

#[cfg(test)]
mod account_tests {
    use super::*;

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
