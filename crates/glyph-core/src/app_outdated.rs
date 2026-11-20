//! "This Glyph is too old for the server."
//!
//! After a change that breaks older apps, the server names the oldest app
//! version that still works (`min_app_version` in `/v1/health`) and answers
//! the API calls of older apps with 426 ([`glyph_protocol::APP_OUTDATED`]).
//! Either way the app notes it here; the UI then covers everything with
//! "Update Glyph" until the user updates. A server that drops the minimum
//! again (or a new enough app) clears it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// This app's version.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outdated {
    /// Oldest version the server takes, when it said so.
    pub min: Option<String>,
}

static STATE: Mutex<Option<Outdated>> = Mutex::new(None);
/// Bumped on every change, so the app announces each one once.
static SEQ: AtomicU64 = AtomicU64::new(0);

fn set(v: Option<Outdated>) {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if *s != v {
        *s = v;
        SEQ.fetch_add(1, Ordering::Relaxed);
    }
}

/// What `/v1/health` said about the oldest app version.
pub fn note_health(min_app_version: Option<&str>) {
    note_health_for(APP_VERSION, min_app_version);
}

fn note_health_for(app: &str, min: Option<&str>) {
    match min.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) if glyph_protocol::app_older_than(app, m) => set(Some(Outdated { min: Some(m.to_string()) })),
        _ => set(None),
    }
}

/// A call came back with 426 `app_outdated`.
pub fn note_rejected() {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if s.is_none() {
        *s = Some(Outdated { min: None });
        SEQ.fetch_add(1, Ordering::Relaxed);
    }
}

/// Whether the server asked this app to update.
pub fn current() -> Option<Outdated> {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Changes so far (compare to announce each change once).
pub fn seq() -> u64 {
    SEQ.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_apps_are_asked_to_update_and_newer_ones_not() {
        let start = seq();
        note_health_for("0.12.11", Some("0.13.0"));
        assert_eq!(current(), Some(Outdated { min: Some("0.13.0".into()) }));
        assert!(seq() > start);
        let after = seq();
        // The same answer again changes nothing (one announcement).
        note_health_for("0.12.11", Some("0.13.0"));
        assert_eq!(seq(), after);
        // A 426 keeps the known minimum.
        note_rejected();
        assert_eq!(current().unwrap().min.as_deref(), Some("0.13.0"));
        // New enough, or the server lifted the minimum: cleared.
        note_health_for("0.13.0", Some("0.13.0"));
        assert_eq!(current(), None);
        note_health_for("0.12.11", None);
        assert_eq!(current(), None);
        note_rejected();
        assert_eq!(current(), Some(Outdated { min: None }));
        note_health_for("0.12.11", Some(""));
        assert_eq!(current(), None);
    }
}
