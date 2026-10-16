//! Local library: liked songs, recently played (last 50), playlists, per-song
//! lyrics offsets. Persisted as JSON in the app data dir. Never leaves the device.

use glyph_protocol::Track;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
    /// Where it was imported from (URL), so a re-import only adds new songs.
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub liked: Vec<Track>,
    pub recent: Vec<Track>,
    pub playlists: Vec<Playlist>,
}

impl Library {
    pub fn load() -> anyhow::Result<Self> {
        todo!()
    }
    pub fn save(&self) -> anyhow::Result<()> {
        todo!()
    }
    pub fn is_liked(&self, track: &Track) -> bool {
        let _ = track;
        todo!()
    }
    pub fn toggle_like(&mut self, track: &Track) {
        let _ = track;
        todo!()
    }
    /// Push to front, dedupe, cap at 50.
    pub fn push_recent(&mut self, track: Track) {
        let _ = track;
        todo!()
    }
    pub fn create_playlist(&mut self, name: &str) -> &mut Playlist {
        let _ = name;
        todo!()
    }
    pub fn rename_playlist(&mut self, id: &str, name: &str) {
        let _ = (id, name);
        todo!()
    }
    pub fn delete_playlist(&mut self, id: &str) {
        let _ = id;
        todo!()
    }
    /// Lyrics offset in ms for a track (default 0), stepped by ±100 in the UI.
    pub fn lyrics_offset(&self, track: &Track) -> i64 {
        let _ = track;
        todo!()
    }
    pub fn set_lyrics_offset(&mut self, track: &Track, offset_ms: i64) {
        let _ = (track, offset_ms);
        todo!()
    }
}
