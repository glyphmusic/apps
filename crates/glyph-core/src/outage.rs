//! Automatic local fallback while the public Glyph server is down.
//!
//! Only the official server ([`config::DEFAULT_SERVER_URL`]) is covered: a
//! self-hosted server failing stays an error the user sees and fixes. When a
//! call fails because the server is offline, or Cloudflare has used up the
//! free plan's daily requests (Workers error 1027, reset at 00:00 UTC),
//! search and playback run locally until the outage ends.
//!
//! A connection that fails without any answer can also mean this device has
//! no internet. A few well-known sites are asked then: none answering means
//! no internet ([`Kind::NoInternet`]), which is announced but switches
//! nothing (local search needs the internet just as much).

use crate::config;
use crate::server_client::ServerError;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Code `check` gives Cloudflare's "daily request limit reached" page.
pub const EDGE_DAILY_LIMIT: &str = "edge_daily_limit";

/// After this long offline the server is asked again.
const OFFLINE_RETRY: Duration = Duration::from_secs(120);
/// Without internet: how long before the next failure probes again.
const NO_INTERNET_RETRY: Duration = Duration::from_secs(15);
/// "No internet" again within this time is still the same outage (one
/// announcement while the device stays offline).
const NO_INTERNET_SAME: Duration = Duration::from_secs(300);
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
/// Independent operators, so one of them being down is no false alarm.
const PROBES: [&str; 3] = [
    "https://www.gstatic.com/generate_204",
    "https://www.msftconnecttest.com/connecttest.txt",
    "https://cloudflare.com/cdn-cgi/trace",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Not reachable (connection refused, timeout, 5xx from the edge) while
    /// the internet works.
    Offline,
    /// Cloudflare's daily limit is used up; resets at 00:00 UTC.
    DailyLimit,
    /// This device has no internet; the server may be fine.
    NoInternet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outage {
    pub kind: Kind,
    pub until: SystemTime,
    /// Counts outages, so the UI announces each one once.
    pub seq: u64,
}

/// Current outage, outage counter, and the last "no internet" one (kept past
/// its end so a device that stays offline is announced once).
static STATE: Mutex<(Option<Outage>, u64, Option<Outage>)> = Mutex::new((None, 0, None));

pub fn is_official(base: &str) -> bool {
    base.trim_end_matches('/').eq_ignore_ascii_case(config::DEFAULT_SERVER_URL)
}

/// The outage in effect right now, if any.
pub fn current() -> Option<Outage> {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if s.0.is_some_and(|o| SystemTime::now() >= o.until) {
        s.0 = None;
    }
    s.0
}

/// The official server is out and calls go local (not when only the
/// internet is gone).
pub fn switched() -> bool {
    current().is_some_and(|o| o.kind != Kind::NoInternet)
}

/// The official server answered again: back to normal.
pub fn clear() {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).0 = None;
}

/// Records `error` from a call to `base` if it means the official server is
/// out. Returns the outage kind when calls should go local now (never for
/// [`Kind::NoInternet`]).
pub async fn note(base: &str, error: &anyhow::Error) -> Option<Kind> {
    if !is_official(base) { return None; }
    let (mut kind, unanswered) = classify(error)?;
    if unanswered {
        // Already known: no new probe for every failing call.
        if let Some(o) = current() {
            return (o.kind != Kind::NoInternet).then_some(o.kind);
        }
        if !internet_up().await { kind = Kind::NoInternet; }
    }
    record(kind);
    (kind != Kind::NoInternet).then_some(kind)
}

fn record(kind: Kind) {
    let now = SystemTime::now();
    let until = match kind {
        Kind::Offline => now + OFFLINE_RETRY,
        Kind::DailyLimit => next_utc_midnight(now),
        Kind::NoInternet => now + NO_INTERNET_RETRY,
    };
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = &mut *guard;
    match &mut s.0 {
        // Still the same outage: a daily limit outranks "offline".
        Some(o) if o.kind == kind || (o.kind == Kind::DailyLimit && kind == Kind::Offline) => o.until = o.until.max(until),
        _ => {
            let still_offline = kind == Kind::NoInternet && s.2.is_some_and(|o| o.until + NO_INTERNET_SAME > now);
            if !still_offline { s.1 += 1; }
            let outage = Outage { kind, until, seq: s.1 };
            s.0 = Some(outage);
            if kind == Kind::NoInternet { s.2 = Some(outage); }
            tracing::warn!(?kind, "official server out");
        }
    }
}

/// Kind of failure and whether nothing answered at all (then it could be
/// this device's internet instead of the server).
fn classify(error: &anyhow::Error) -> Option<(Kind, bool)> {
    if let Some(e) = error.downcast_ref::<ServerError>() {
        // An HTTP answer came back, from the server or Cloudflare's edge:
        // the internet works.
        if e.code == EDGE_DAILY_LIMIT { return Some((Kind::DailyLimit, false)); }
        // D1 / KV free-plan limits also reset at 00:00 UTC.
        if e.status >= 500 && e.message.to_ascii_lowercase().contains("limit exceeded") { return Some((Kind::DailyLimit, false)); }
        return matches!(e.status, 502..=504 | 520..=527 | 530).then_some((Kind::Offline, false));
    }
    error.chain()
        .filter_map(|c| c.downcast_ref::<reqwest::Error>())
        .any(|e| e.is_connect() || e.is_timeout())
        .then_some((Kind::Offline, true))
}

/// Any of [`PROBES`] answering (any status) means the internet works.
pub(crate) async fn internet_up() -> bool {
    let Ok(http) = reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() else { return true };
    let mut probes: futures::stream::FuturesUnordered<_> = PROBES.iter().map(|url| http.head(*url).send()).collect();
    use futures::StreamExt;
    while let Some(r) = probes.next().await {
        if r.is_ok() { return true; }
    }
    false
}

fn next_utc_midnight(now: SystemTime) -> SystemTime {
    let secs = now.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    UNIX_EPOCH + Duration::from_secs((secs / 86_400 + 1) * 86_400)
}

/// Whether a Cloudflare error page is the daily request limit (error 1027).
pub fn is_daily_limit_page(status: u16, body: &str) -> bool {
    status == 429 && (body.contains("1027") || body.to_ascii_lowercase().contains("temporarily rate limited"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_error(status: u16, code: &str, message: &str) -> anyhow::Error {
        anyhow::Error::new(ServerError { status, code: code.into(), message: message.into(), retry_after_s: None })
    }

    #[test]
    fn classifies_edge_failures_only() {
        // An HTTP answer means the internet works: no probe needed.
        assert_eq!(classify(&server_error(429, EDGE_DAILY_LIMIT, "")), Some((Kind::DailyLimit, false)));
        assert_eq!(classify(&server_error(500, "internal", "D1 rows read limit exceeded")), Some((Kind::DailyLimit, false)));
        assert_eq!(classify(&server_error(503, "503", "")), Some((Kind::Offline, false)));
        assert_eq!(classify(&server_error(522, "522", "")), Some((Kind::Offline, false)));
        // The server's own per-minute limit and client errors are no outage.
        assert_eq!(classify(&server_error(429, "rate_limited", "")), None);
        assert_eq!(classify(&server_error(404, "not_found", "")), None);
        assert_eq!(classify(&anyhow::anyhow!("bad json")), None);
    }

    #[tokio::test]
    async fn only_the_official_server_switches() {
        assert_eq!(note("https://mine.example", &server_error(503, "503", "")).await, None);
        assert!(is_official("https://API.glyph.my/"));
    }

    #[test]
    fn daily_limit_lasts_until_utc_midnight() {
        let t = UNIX_EPOCH + Duration::from_secs(20_000 * 86_400 + 3_600);
        assert_eq!(next_utc_midnight(t), UNIX_EPOCH + Duration::from_secs(20_001 * 86_400));
        assert!(is_daily_limit_page(429, "<title>Error 1027</title>"));
        assert!(!is_daily_limit_page(503, "error code: 1027"));
    }
}
