//! Audio files opened with Glyph: "Open with" and "Send to" on the desktop,
//! "Open with" / share on Android. Each file becomes a song that plays from
//! the file (`LocalLibrary::open_file`).
//!
//! The desktop plays them at once, like any music player. The phone shows
//! the "add shared song" sheet (`pick_songs` event): play, Liked Songs,
//! playlists.

use crate::controller::{Core, PlayFrom};
use glyph_protocol::Track;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One "Open with" hands over at most this many files.
pub const MAX_FILES: usize = 200;

/// What to do with the songs once they are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Then {
    /// Play them now (desktop).
    Play,
    /// Ask what to do (the phone's sheet).
    Ask,
}

/// Files that arrived within [`GATHER`] of each other, per mode.
static GATHERED: Mutex<Vec<(Then, Vec<PathBuf>)>> = Mutex::new(Vec::new());
/// Windows starts Glyph once per selected file ("Open" on five songs is
/// five starts, each handing over one file): they are gathered this long
/// and handled as one list, so the first one does not stop the others.
const GATHER: Duration = Duration::from_millis(700);

/// Reads `paths` (off the UI thread, once the backend is up) and plays them
/// or asks what to do with them.
pub fn open_files(core: &Arc<Core>, paths: Vec<PathBuf>, then: Then) {
    if paths.is_empty() {
        return;
    }
    {
        let mut g = GATHERED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, list)) = g.iter_mut().find(|(t, _)| *t == then) {
            list.extend(paths);
            return;
        }
        g.push((then, paths));
    }
    let core = core.clone();
    crate::rt::spawn(async move {
        tokio::time::sleep(GATHER).await;
        let paths: Vec<PathBuf> = {
            let mut g = GATHERED.lock().unwrap_or_else(|e| e.into_inner());
            let i = g.iter().position(|(t, _)| *t == then);
            i.map(|i| g.remove(i).1).unwrap_or_default()
        };
        let mut seen = std::collections::HashSet::new();
        let paths: Vec<PathBuf> = paths.into_iter().filter(|p| seen.insert(p.clone())).take(MAX_FILES).collect();
        if paths.is_empty() {
            return;
        }
        let tracks = match read_files(&core, paths).await {
            Ok(t) => t,
            Err(key) => {
                core.toast_key(key, "info");
                return;
            }
        };
        match then {
            Then::Play => core.play_list_from(tracks, 0, None, PlayFrom::default()),
            Then::Ask => core.emit("pick_songs", json!({ "tracks": tracks, "file": true })),
        }
    });
}

/// Reads opened files into songs, once the backend is up (a file that
/// started Glyph arrives before it). The error is an i18n key.
pub async fn read_files(core: &Arc<Core>, paths: Vec<PathBuf>) -> Result<Vec<Track>, &'static str> {
    let mut backend = core.backend();
    for _ in 0..120 {
        if backend.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        backend = core.backend();
    }
    let b = backend.ok_or("status.backend_offline")?;
    let lib = b.local_library();
    let tracks: Vec<Track> = crate::rt::spawn_blocking(move || {
        paths
            .iter()
            .take(MAX_FILES)
            .filter_map(|p| match lib.open_file(p) {
                Ok(t) => Some(t),
                Err(e) => {
                    tracing::info!(error = %e, "opened file not readable");
                    None
                }
            })
            .collect()
    })
    .await
    .unwrap_or_default();
    if tracks.is_empty() {
        return Err("open.file_unreadable");
    }
    Ok(tracks)
}
