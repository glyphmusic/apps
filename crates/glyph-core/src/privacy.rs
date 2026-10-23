//! Transparency: what the current server can see, and what this device
//! noticed about it.
//!
//! The report is built from this app's own behaviour, never from what a
//! server says about itself: a server cannot make an item look encrypted
//! here. Notices are things a server did that only make sense if it tried
//! to read data it should not (alarms), or that the user should know about
//! (for example the data key going to a new device).

use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// How readable something is for the server's operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Stays on this device.
    Local,
    /// End-to-end encrypted: the server stores what it cannot read.
    Encrypted,
    /// Encrypted, but the server holds the key.
    ServerCanDecrypt,
    /// The server needs it to do its work, so it sees it.
    ServerSees,
    /// Visible to other signed-in people.
    Public,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Local => "local",
            Level::Encrypted => "encrypted",
            Level::ServerCanDecrypt => "decryptable",
            Level::ServerSees => "visible",
            Level::Public => "public",
        }
    }
}

/// Encryption state of the signed-in account's library on this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Library {
    /// Not signed in: nothing syncs.
    Guest,
    /// Signed in, no keyring yet (the next sync round creates it).
    Starting,
    /// The server keeps the key because the account has no password.
    Escrow,
    /// End-to-end encrypted, and this device has the key.
    Encrypted,
    /// End-to-end encrypted, the key has not reached this device yet.
    Locked,
}

/// What the report depends on.
#[derive(Debug, Clone)]
pub struct Inputs {
    pub official: bool,
    pub library: Library,
    /// Playlists are shown on the profile.
    pub profile_playlists: bool,
    /// The MCP relay through the server is on.
    pub mcp_relay: bool,
}

/// One line of the report: `area` and `note` are text ids (`privacy.area.*`,
/// `privacy.note.*`).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub area: &'static str,
    pub level: Level,
    pub note: Option<&'static str>,
}

fn line(area: &'static str, level: Level, note: Option<&'static str>) -> Line {
    Line { area, level, note }
}

pub fn report(i: &Inputs) -> Vec<Line> {
    let signed_in = i.library != Library::Guest;
    let mut v = Vec::new();
    v.push(match i.library {
        Library::Guest => line("library", Level::Local, None),
        Library::Starting => line("library", Level::ServerSees, Some("starting")),
        Library::Escrow => line("library", Level::ServerCanDecrypt, Some("no_password")),
        Library::Encrypted => line("library", Level::Encrypted, None),
        Library::Locked => line("library", Level::Encrypted, Some("locked")),
    });
    v.push(match i.library {
        Library::Guest => line("api_keys", Level::Local, None),
        Library::Encrypted | Library::Locked => line("api_keys", Level::Encrypted, None),
        Library::Starting | Library::Escrow => line("api_keys", Level::Local, Some("keys_not_synced")),
    });
    v.push(line("password", Level::Local, Some(if i.official { "official_page" } else { "app_only" })));
    if signed_in {
        v.push(line("profile", Level::Public, i.profile_playlists.then_some("profile_playlists")));
    }
    v.push(line("requests", Level::ServerSees, Some(if signed_in { "linked" } else { "anonymous" })));
    if signed_in {
        v.push(line("devices", Level::ServerSees, None));
        v.push(line("jam", Level::ServerSees, None));
    }
    if i.mcp_relay {
        v.push(line("mcp", Level::ServerSees, None));
    }
    v.push(line("technical", Level::ServerSees, None));
    v
}

/// Something this device noticed about a server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Unix ms.
    pub at: u64,
    pub server: String,
    #[serde(default)]
    pub account: String,
    /// The server did something that only makes sense to read data.
    pub alarm: bool,
    /// Text id: `privacy.event.<code>`.
    pub code: String,
    /// Fills `{detail}` in the text (a device name, ...).
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub seen: bool,
}

const KEEP: usize = 100;

fn path() -> PathBuf {
    paths::data_dir().join("privacy-events.json")
}

fn lock() -> &'static Mutex<()> {
    static L: Mutex<()> = Mutex::new(());
    &L
}

fn same_server(a: &str, b: &str) -> bool {
    a.trim_end_matches('/').eq_ignore_ascii_case(b.trim_end_matches('/'))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Remembers an event. The same event within an hour is kept once.
pub fn record(server: &str, account: &str, alarm: bool, code: &str, detail: &str) {
    let _g = lock().lock().unwrap();
    let mut all: Vec<Event> = paths::read_json(&path()).unwrap_or_default();
    let now = now_ms();
    if all.iter().any(|e| same_server(&e.server, server) && e.code == code && e.detail == detail && now.saturating_sub(e.at) < 3_600_000) {
        return;
    }
    if alarm {
        tracing::warn!("privacy alarm on {server}: {code} {detail}");
    }
    all.push(Event { at: now, server: server.into(), account: account.into(), alarm, code: code.into(), detail: detail.into(), seen: false });
    let excess = all.len().saturating_sub(KEEP);
    all.drain(..excess);
    if let Err(e) = paths::write_json(&path(), &all) {
        tracing::warn!("privacy events: {e}");
    }
}

/// Events for `server`, newest first.
pub fn events(server: &str) -> Vec<Event> {
    let _g = lock().lock().unwrap();
    let mut v: Vec<Event> = paths::read_json::<Vec<Event>>(&path()).unwrap_or_default().into_iter().filter(|e| same_server(&e.server, server)).collect();
    v.reverse();
    v
}

/// Alarms for `server` not shown yet; marks them shown.
pub fn take_unseen_alarms(server: &str) -> Vec<Event> {
    let _g = lock().lock().unwrap();
    let mut all: Vec<Event> = paths::read_json(&path()).unwrap_or_default();
    let mut out = Vec::new();
    for e in all.iter_mut().filter(|e| e.alarm && !e.seen && same_server(&e.server, server)) {
        e.seen = true;
        out.push(e.clone());
    }
    if !out.is_empty() {
        let _ = paths::write_json(&path(), &all);
    }
    out
}

/// Forgets the events of `server`.
pub fn clear(server: &str) {
    let _g = lock().lock().unwrap();
    let all: Vec<Event> = paths::read_json::<Vec<Event>>(&path()).unwrap_or_default().into_iter().filter(|e| !same_server(&e.server, server)).collect();
    let _ = paths::write_json(&path(), &all);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_follows_the_encryption_state() {
        let base = Inputs { official: false, library: Library::Encrypted, profile_playlists: false, mcp_relay: false };
        let lib = |i: &Inputs| report(i).into_iter().find(|l| l.area == "library").unwrap();
        assert_eq!(lib(&base).level, Level::Encrypted);
        assert_eq!(lib(&Inputs { library: Library::Escrow, ..base.clone() }).level, Level::ServerCanDecrypt);
        assert_eq!(lib(&Inputs { library: Library::Guest, ..base.clone() }).level, Level::Local);
        let r = report(&base);
        assert!(r.iter().any(|l| l.area == "requests" && l.level == Level::ServerSees));
        assert!(!r.iter().any(|l| l.area == "mcp"));
        assert!(report(&Inputs { mcp_relay: true, ..base }).iter().any(|l| l.area == "mcp"));
    }
}
