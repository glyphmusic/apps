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
        Self { base: normalize_base_url(url), http, install_id }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.get(self.url(path)).header(INSTALL_ID_HEADER, &self.install_id).timeout(REQUEST_TIMEOUT)
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http.post(self.url(path)).header(INSTALL_ID_HEADER, &self.install_id).timeout(REQUEST_TIMEOUT)
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
        Ok(resp.json().await.context("invalid /v1/health response")?)
    }

    /// Health + protocol-version check.
    pub async fn check_compat(&self) -> Result<Compat> {
        let h = self.health().await?;
        Ok(if h.protocol_version == PROTOCOL_VERSION { Compat::Compatible(h) } else { Compat::Incompatible(h) })
    }

    /// `GET /v1/search?q=` (NDJSON). Forwards events into `tx` as they arrive.
    /// Returns `Ok(true)` if the stream ended with `Done`. Fails early (before
    /// anything was forwarded) if the connection or status is bad, so the
    /// caller can fall back to direct mode.
    pub async fn search_into(&self, query: &str, tx: &UnboundedSender<SearchEvent>) -> Result<bool> {
        let resp = self.get("/v1/search").query(&[("q", query)]).send().await?;
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
        let body = ResolveRequest { track: track.clone(), priority: priority.to_vec() };
        let resp = Self::check(self.post("/v1/resolve").json(&body).send().await?).await?;
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
        let resp = Self::check(self.post("/v1/youtube/hint").timeout(Duration::from_secs(15)).json(&body).send().await?).await?;
        Ok(resp.json().await?)
    }

    /// `GET /v1/share/<id>`: the playlist behind a short link.
    pub async fn shared_playlist(&self, id: &str) -> Result<SharedPlaylist> {
        let resp = Self::check(self.get(&format!("/v1/share/{id}")).timeout(Duration::from_secs(10)).send().await?).await?;
        Ok(resp.json().await?)
    }

    /// Route public media and metadata through the Worker allowlist.
    pub fn upstream_url(&self, upstream: &str) -> String {
        reqwest::Url::parse_with_params(&self.url("/v1/upstream"), &[("url", upstream)])
            .map(String::from).unwrap_or_else(|_| self.url("/v1/upstream"))
    }

    pub async fn cover_url(&self, track: &Track) -> Result<Option<String>> {
        let resp = Self::check(self.post("/v1/cover").json(track).send().await?).await?;
        let value: serde_json::Value = resp.json().await?;
        Ok(value["url"].as_str().map(|u| self.upstream_url(u)))
    }

    pub async fn download_url(&self, track: &Track) -> Result<Option<(ProviderId, String)>> {
        let resp = Self::check(self.post("/v1/download").json(track).send().await?).await?;
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
        let resp = self.get("/v1/lyrics").query(&q).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(Self::check(resp).await?.json().await?))
    }

    /// `GET /v1/similar`.
    pub async fn similar(&self, artist: &str, title: &str) -> Result<SimilarResponse> {
        let resp = self.get("/v1/similar").query(&[("artist", artist), ("title", title)]).send().await?;
        Ok(Self::check(resp).await?.json().await?)
    }

    /// `POST /v1/import/spotify` (public Spotify / Deezer / YouTube playlist links).
    pub async fn import(&self, url: &str) -> Result<ImportResponse> {
        let body = ImportRequest { url: url.to_string() };
        let resp = Self::check(self.post("/v1/import/spotify").json(&body).send().await?).await?;
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
        self.http
            .request(method, self.url(path))
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
        Self::json_of(self.post("/v1/account/token/refresh").json(&RefreshRequest { refresh_token: refresh_token.into() })).await
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
