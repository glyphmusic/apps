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
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

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
        Ok(resp.json().await?)
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
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GLYPH_DATA_DIR", dir.path());
        let a = install_id();
        let b = install_id();
        std::env::remove_var("GLYPH_DATA_DIR");
        assert_eq!(a, b);
        assert!(a.len() >= 16);
    }
}
