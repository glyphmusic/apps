//! Local playback session persistence: remembers the current song, position,
//! queue and context across clean exits and crashes so playback can resume
//! right where the user left off. Never synced or uploaded.

use crate::dedupe::track_key;
use crate::library::ListKey;
use crate::paths;
use crate::queue::QueueSnapshot;
use glyph_protocol::Track;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Full snapshot of the local playback state, written when the song, queue or
/// context changes and on quit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackSession {
    /// Monotonic sequence number matching [`PlaybackPosition::seq`].
    pub seq: u64,
    pub current: Option<Track>,
    pub position_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub activity_recorded: bool,
    pub queue: QueueSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playing_from: Option<ListKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_artist_ctx: Option<String>,
    pub saved_at: u64,
}

/// Lightweight position sidecar written periodically while a song plays so
/// crashes lose at most a few seconds without re-serializing large queues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackPosition {
    pub seq: u64,
    pub track_key: String,
    pub position_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub activity_recorded: bool,
}

impl PlaybackSession {
    pub fn path_for(library_path: &Path) -> PathBuf {
        library_path.with_extension("playback.json")
    }

    pub fn pos_path_for(library_path: &Path) -> PathBuf {
        library_path.with_extension("playback-pos.json")
    }

    pub fn is_empty(&self) -> bool {
        self.current.is_none() && self.queue.is_empty()
    }

    pub fn load_for(library_path: &Path) -> Option<Self> {
        let bytes = std::fs::read(Self::path_for(library_path)).ok()?;
        let mut session: Self = serde_json::from_slice(&bytes).ok()?;
        if session.is_empty() {
            return None;
        }
        if let Ok(pos_bytes) = std::fs::read(Self::pos_path_for(library_path)) {
            if let Ok(pos) = serde_json::from_slice::<PlaybackPosition>(&pos_bytes) {
                let same_track = session
                    .current
                    .as_ref()
                    .map(track_key)
                    .is_some_and(|k| k == pos.track_key);
                if pos.seq == session.seq && same_track {
                    session.position_ms = pos.position_ms;
                    if pos.duration_ms.is_some() {
                        session.duration_ms = pos.duration_ms;
                    }
                    session.activity_recorded |= pos.activity_recorded;
                }
            }
        }
        Some(session)
    }

    pub fn save_to(&self, library_path: &Path) -> anyhow::Result<()> {
        let path = Self::path_for(library_path);
        let pos_path = Self::pos_path_for(library_path);
        if self.is_empty() {
            remove_if_exists(&path)?;
            remove_if_exists(&pos_path)?;
            return Ok(());
        }
        let bytes = serde_json::to_vec(self)?;
        paths::write_atomic(&path, &bytes)?;
        let _ = remove_if_exists(&pos_path);
        Ok(())
    }
}

impl PlaybackPosition {
    pub fn save_to(&self, library_path: &Path) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(self)?;
        paths::write_atomic(&PlaybackSession::pos_path_for(library_path), &bytes)
    }
}

fn remove_if_exists(path: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedupe::test_track;
    use crate::queue::{Queue, RepeatMode, ShuffleMode};
    use glyph_protocol::ProviderId;

    fn temp_lib(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "glyph-playback-test-{}-{}-{}",
            name,
            std::process::id(),
            paths::now_unix()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("library.json")
    }

    #[test]
    fn round_trip_and_position_sidecar() {
        let lib = temp_lib("roundtrip");
        let t1 = test_track(ProviderId::Audius, "1", "First", "Artist A", Some(180_000));
        let t2 = test_track(ProviderId::Audius, "2", "Second", "Artist B", Some(200_000));
        let mut q = Queue::new();
        q.set_shuffle_mode(ShuffleMode::Random);
        q.set_repeat(RepeatMode::All);
        q.set(vec![t1.clone(), t2.clone()], 0);
        q.push(t2.clone());

        let sess = PlaybackSession {
            seq: 7,
            current: Some(t1.clone()),
            position_ms: 12_000,
            duration_ms: Some(180_000),
            activity_recorded: false,
            queue: q.snapshot(),
            queue_from: Some("Test Playlist".into()),
            playing_from: Some(ListKey::Liked),
            recent_artist_ctx: None,
            saved_at: 1000,
        };
        sess.save_to(&lib).unwrap();

        let pos = PlaybackPosition {
            seq: 7,
            track_key: track_key(&t1),
            position_ms: 94_500,
            duration_ms: Some(181_000),
            activity_recorded: true,
        };
        pos.save_to(&lib).unwrap();

        let loaded = PlaybackSession::load_for(&lib).unwrap();
        assert_eq!(loaded.seq, 7);
        assert_eq!(loaded.position_ms, 94_500);
        assert_eq!(loaded.duration_ms, Some(181_000));
        assert!(loaded.activity_recorded);
        assert_eq!(loaded.queue_from.as_deref(), Some("Test Playlist"));
        assert_eq!(loaded.playing_from, Some(ListKey::Liked));

        let mut restored_q = Queue::new();
        restored_q.restore(loaded.queue);
        assert_eq!(restored_q.current().unwrap().id, "1");
        assert_eq!(restored_q.upcoming().len(), 2);
        assert_eq!(restored_q.repeat(), RepeatMode::All);
        assert_eq!(restored_q.shuffle_mode(), ShuffleMode::Random);

        // Saving a mismatched position sidecar must be ignored.
        PlaybackPosition {
            seq: 6,
            track_key: track_key(&t1),
            position_ms: 5_000,
            duration_ms: None,
            activity_recorded: false,
        }
        .save_to(&lib)
        .unwrap();
        let loaded2 = PlaybackSession::load_for(&lib).unwrap();
        assert_eq!(loaded2.position_ms, 12_000);

        // Saving an empty session clears both files.
        PlaybackSession::default().save_to(&lib).unwrap();
        assert!(PlaybackSession::load_for(&lib).is_none());
        assert!(!PlaybackSession::path_for(&lib).exists());
        assert!(!PlaybackSession::pos_path_for(&lib).exists());

        let _ = std::fs::remove_dir_all(lib.parent().unwrap());
    }

    #[test]
    fn corrupt_queue_indices_are_sanitized() {
        let mut q = Queue::new();
        let t1 = test_track(ProviderId::Audius, "1", "First", "Artist A", None);
        let t2 = test_track(ProviderId::Audius, "2", "Second", "Artist B", None);
        q.restore(QueueSnapshot {
            base: vec![t1, t2],
            order: vec![0, 99],
            pos: Some(5),
            next_round: Some(vec![42]),
            ..Default::default()
        });
        assert_eq!(q.current().unwrap().id, "1");
        assert_eq!(q.upcoming().len(), 1);
    }
}
