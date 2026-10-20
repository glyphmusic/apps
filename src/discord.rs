//! Discord Rich Presence: shows the song Glyph is playing in the user's
//! Discord profile.
//!
//! Discord does not take presence over the network; every Discord client runs
//! a local IPC pipe for the logged-in user (`\\.\pipe\discord-ipc-0` .. `-9` on
//! Windows, a Unix socket under the runtime / temp directory elsewhere). This
//! module speaks that protocol directly: one handshake, then `SET_ACTIVITY`
//! frames. The worker thread owns the connection and reconnects when Discord
//! starts or restarts; the UI only sends "what is playing" through a channel
//! and never waits for the pipe.

use glyph_protocol::Track;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Discord application whose name and assets the presence shows (created in
/// the Discord Developer Portal as "Glyph"). Users can override it under
/// Settings > Integrations, with `GLYPH_DISCORD_CLIENT_ID`, or via
/// `[integrations] discord_client_id` in the config.
pub const APP_ID: &str = "1561796441783935017";
/// How often a disconnected worker retries while a song is loaded (Discord
/// may start after Glyph).
const RETRY: Duration = Duration::from_secs(10);
/// Discord cuts "details" and "state" to 128 characters each.
const MAX_TEXT: usize = 128;

/// The client ID to use: `GLYPH_DISCORD_CLIENT_ID`, else the settings, else
/// the ID built into this build. `None`: the presence cannot be shown.
pub fn client_id(config: &glyph_core::Config) -> Option<String> {
    if let Ok(v) = std::env::var("GLYPH_DISCORD_CLIENT_ID") {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    let from_config = config.integrations.discord_client_id.trim();
    if !from_config.is_empty() {
        return Some(from_config.to_string());
    }
    (!APP_ID.is_empty()).then(|| APP_ID.to_string())
}

enum Msg {
    /// What the player is doing now.
    Now { track: Option<Track>, playing: bool, position: Duration },
    Quit,
}

/// Cheap handle; the worker thread owns the connection and the reconnect
/// logic. Dropping it clears the presence and ends the thread.
pub struct DiscordPresence {
    tx: flume::Sender<Msg>,
}

impl DiscordPresence {
    pub fn new(client_id: String) -> Self {
        let (tx, rx) = flume::unbounded();
        let spawned = std::thread::Builder::new()
            .name("glyph-discord".into())
            .spawn(move || worker(client_id, rx));
        if let Err(e) = spawned {
            tracing::warn!("discord presence: worker did not start: {e}");
        }
        Self { tx }
    }

    /// Track, play state and position as of now.
    pub fn set(&self, track: Option<&Track>, playing: bool, position: Duration) {
        let _ = self.tx.send(Msg::Now { track: track.cloned(), playing, position });
    }
}

impl Drop for DiscordPresence {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
    }
}

struct Now {
    track: Option<Track>,
    playing: bool,
    position: Duration,
    /// When the position was sampled, so a late reconnect (Discord started
    /// minutes after Glyph) still reports the right progress.
    captured: Instant,
}

impl Now {
    /// The activity to send; `None` clears the presence.
    fn activity(&self) -> Option<Value> {
        let track = self.track.as_ref()?;
        let mut act = json!({
            "details": clip(&track.title),
            "state": clip(&self.state_text(track)),
            // 2 = "Listening to", the fitting line for a music player.
            "type": 2,
        });
        if self.playing {
            // Discord counts on from the start timestamp itself.
            let position = self.position + self.captured.elapsed();
            let position = match track.duration_ms {
                Some(ms) if ms > 0 => position.min(Duration::from_millis(ms)),
                _ => position,
            };
            let start = unix_now().saturating_sub(position.as_secs());
            let mut ts = json!({ "start": start });
            if let Some(ms) = track.duration_ms.filter(|ms| *ms > 0) {
                ts["end"] = json!(start + ms / 1000);
            }
            act["timestamps"] = ts;
        }
        // Discord proxies https cover URLs; local cache paths never leave
        // this computer.
        if let Some(url) = track.cover_url.as_deref().filter(|u| u.starts_with("https://")) {
            let label = track.album.clone().unwrap_or_else(|| track.artist.clone());
            act["assets"] = json!({ "large_image": url, "large_text": clip(&label) });
        }
        Some(act)
    }

    fn state_text(&self, track: &Track) -> String {
        match track.album.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            Some(album) => format!("{} · {}", track.artist, album),
            None => track.artist.clone(),
        }
    }
}

fn worker(client_id: String, rx: flume::Receiver<Msg>) {
    let mut conn: Option<Conn> = None;
    let mut now: Option<Now> = None;
    loop {
        let mut dirty = false;
        match rx.recv_timeout(RETRY) {
            Ok(Msg::Quit) => break,
            Ok(Msg::Now { track, playing, position }) => {
                now = Some(Now { track, playing, position, captured: Instant::now() });
                dirty = true;
            }
            Err(flume::RecvTimeoutError::Disconnected) => break,
            // Discord may have started while the song kept playing.
            Err(flume::RecvTimeoutError::Timeout) => {
                dirty = conn.is_none() && now.is_some();
            }
        }
        if !dirty {
            continue;
        }
        let Some(now) = &now else { continue };
        // Nothing to show: no need to open a connection at all.
        if conn.is_none() && now.track.is_none() {
            continue;
        }
        if conn.is_none() {
            conn = Conn::open(&client_id);
        }
        // Send through a temporary so the `conn` borrow ends before a failed
        // write drops the connection.
        let send_failed = match conn.as_mut() {
            Some(c) => c.set_activity(now.activity()).is_err(),
            None => false,
        };
        if send_failed {
            conn = None;
        }
    }
    if let Some(c) = conn.as_mut() {
        c.close();
    }
}

struct Conn {
    pipe: Pipe,
    seq: u64,
}

impl Conn {
    /// Opens the pipe and sends the handshake. `None`: Discord is not running.
    fn open(client_id: &str) -> Option<Self> {
        let mut pipe = Pipe::connect()?;
        let hello = frame(0, &json!({ "v": 1, "client_id": client_id }));
        pipe.send(&hello).ok()?;
        Some(Self { pipe, seq: 0 })
    }

    /// `None` clears the presence.
    fn set_activity(&mut self, activity: Option<Value>) -> std::io::Result<()> {
        self.seq += 1;
        let mut args = serde_json::Map::new();
        args.insert("pid".into(), json!(std::process::id()));
        if let Some(a) = activity {
            args.insert("activity".into(), a);
        }
        let payload = json!({
            "cmd": "SET_ACTIVITY",
            "args": Value::Object(args),
            "nonce": format!("{}-{}", std::process::id(), self.seq),
        });
        self.pipe.send(&frame(1, &payload))
    }

    /// Closing the pipe clears the presence on Discord's side.
    fn close(&mut self) {
        let _ = self.pipe.send(&frame(2, &json!({})));
    }
}

/// One Discord IPC frame: little-endian opcode and length, then JSON.
fn frame(opcode: u32, payload: &Value) -> Vec<u8> {
    let json = payload.to_string();
    let mut buf = Vec::with_capacity(8 + json.len());
    buf.extend_from_slice(&opcode.to_le_bytes());
    buf.extend_from_slice(&(json.len() as u32).to_le_bytes());
    buf.extend_from_slice(json.as_bytes());
    buf
}

/// One connected Discord pipe (a named pipe file / a Unix socket, both read
/// and written like a file).
#[cfg(windows)]
struct Pipe(std::fs::File);
#[cfg(unix)]
struct Pipe(std::os::unix::net::UnixStream);

impl Pipe {
    fn connect() -> Option<Self> {
        candidates().into_iter().find_map(|path| Self::open(&path).ok())
    }

    #[cfg(windows)]
    fn open(path: &Path) -> std::io::Result<Self> {
        std::fs::OpenOptions::new().read(true).write(true).open(path).map(Self)
    }

    #[cfg(unix)]
    fn open(path: &Path) -> std::io::Result<Self> {
        std::os::unix::net::UnixStream::connect(path).map(Self)
    }

    fn send(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.0.write_all(buf)?;
        self.0.flush()
    }
}

/// Every pipe path Discord uses for the current user, most likely first.
#[cfg(windows)]
fn candidates() -> Vec<PathBuf> {
    (0..10).map(|i| PathBuf::from(format!(r"\\.\pipe\discord-ipc-{i}"))).collect()
}

#[cfg(unix)]
fn candidates() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        let dir = PathBuf::from(dir);
        // Native, Flatpak and Snap installations get their own folders.
        roots.push(dir.clone());
        roots.push(dir.join("app/com.discordapp.Discord"));
        roots.push(dir.join("snap.discord"));
    }
    if let Some(dir) = std::env::var_os("TMPDIR").filter(|v| !v.is_empty()) {
        roots.push(PathBuf::from(dir));
    }
    roots.push(PathBuf::from("/tmp"));
    roots
        .into_iter()
        .flat_map(|root| (0..10).map(move |i| root.join(format!("discord-ipc-{i}"))))
        .collect()
}

/// Discord shows at most [`MAX_TEXT`] characters per line.
fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_TEXT {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX_TEXT - 1).collect();
    out.push('…');
    out
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_has_opcode_and_length_prefix() {
        let f = frame(1, &json!({ "a": 1 }));
        assert_eq!(&f[..4], &1u32.to_le_bytes());
        assert_eq!(&f[4..8], &(f.len() as u32 - 8).to_le_bytes());
        assert_eq!(&f[8..], br#"{"a":1}"#);
    }

    #[test]
    fn activity_clips_and_reports_progress() {
        let track = Track {
            id: "1".into(),
            source: glyph_protocol::ProviderId::Audius,
            title: "T".repeat(200),
            artist: "A".into(),
            album: Some("Album".into()),
            duration_ms: Some(180_000),
            cover_url: Some("https://covers.example/x.jpg".into()),
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false,
        };
        let now = Now { track: Some(track), playing: true, position: Duration::from_secs(30), captured: Instant::now() };
        let a = now.activity().unwrap();
        let details = a["details"].as_str().unwrap();
        assert_eq!(details.chars().count(), 128);
        assert!(details.ends_with('…'));
        assert_eq!(a["state"].as_str(), Some("A · Album"));
        assert_eq!(a["type"].as_u64(), Some(2));
        let start = a["timestamps"]["start"].as_u64().unwrap();
        assert_eq!(a["timestamps"]["end"].as_u64().unwrap() - start, 180);
        assert_eq!(a["assets"]["large_image"].as_str(), Some("https://covers.example/x.jpg"));
        // Paused: a position would be a lie, so no timestamps.
        let paused = Now { playing: false, ..now };
        assert!(paused.activity().unwrap().get("timestamps").is_none());
    }

    #[test]
    fn no_track_clears() {
        let now = Now { track: None, playing: false, position: Duration::ZERO, captured: Instant::now() };
        assert!(now.activity().is_none());
    }
}
