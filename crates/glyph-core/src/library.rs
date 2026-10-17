//! Local library: liked songs, recently played (last 50), playlists, per-song
//! lyrics offsets. Persisted as JSON in the app data dir. Never leaves the device.

use crate::dedupe::{same_song, track_key};
use crate::paths;
use glyph_protocol::Track;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Max entries in "recently played".
pub const RECENT_LIMIT: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
    /// Where it was imported from (URL), so a re-import only adds new songs.
    pub origin: Option<String>,
    /// Picture chosen by the user (local file in the data dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
}

/// A song list the library can point at (sidebar "recent", saved albums).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ListKey {
    Liked,
    Recent,
    Playlist { id: String },
    Album { title: String, artist: String, tracks: Vec<Track> },
    ArtistLiked { artist: String },
}

impl ListKey {
    /// Same list (albums compare by title + artist, not by their songs).
    pub fn same(&self, other: &ListKey) -> bool {
        match (self, other) {
            (ListKey::Album { title: a, artist: b, .. }, ListKey::Album { title: c, artist: d, .. }) => {
                a.eq_ignore_ascii_case(c) && b.eq_ignore_ascii_case(d)
            }
            _ => self == other,
        }
    }
}

/// Max entries in "recently opened lists".
pub const RECENT_LISTS_LIMIT: usize = 8;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub liked: Vec<Track>,
    pub recent: Vec<Track>,
    pub playlists: Vec<Playlist>,
    /// Lyrics offsets in ms, keyed by normalised `title|artist`.
    #[serde(default)]
    pub lyrics_offsets: BTreeMap<String, i64>,
    /// Songs picked from search, newest first (the search bar's dropdown).
    #[serde(default)]
    pub search_history: Vec<Track>,
    /// Albums saved with the heart (shown in the library).
    #[serde(default)]
    pub saved_albums: Vec<ListKey>,
    /// Lists opened recently, newest first (sidebar).
    #[serde(default)]
    pub recent_lists: Vec<ListKey>,
}

/// Max entries in the search history.
pub const SEARCH_HISTORY_LIMIT: usize = 20;

impl Library {
    pub fn path() -> PathBuf {
        paths::data_dir().join("library.json")
    }

    /// Missing file = empty library. A corrupt file is kept as `library.json.bak`.
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(lib) => Ok(lib),
                Err(e) => {
                    let _ = std::fs::copy(path, path.with_extension("json.bak"));
                    Err(anyhow::anyhow!("library file is corrupt (backup kept): {e}"))
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        paths::write_json(path, self)
    }

    pub fn is_liked(&self, track: &Track) -> bool {
        self.liked.iter().any(|t| same_entry(t, track))
    }

    pub fn toggle_like(&mut self, track: &Track) {
        if let Some(i) = self.liked.iter().position(|t| same_entry(t, track)) {
            self.liked.remove(i);
        } else {
            self.liked.insert(0, track.clone());
        }
    }

    /// Push to front, dedupe, cap at 50.
    pub fn push_recent(&mut self, track: Track) {
        self.recent.retain(|t| !same_entry(t, &track));
        self.recent.insert(0, track);
        self.recent.truncate(RECENT_LIMIT);
    }

    /// Remember a song picked from search: front, deduped, capped.
    pub fn push_search(&mut self, track: Track) {
        self.search_history.retain(|t| !same_entry(t, &track));
        self.search_history.insert(0, track);
        self.search_history.truncate(SEARCH_HISTORY_LIMIT);
    }

    pub fn remove_search(&mut self, index: usize) {
        if index < self.search_history.len() {
            self.search_history.remove(index);
        }
    }

    pub fn is_saved(&self, key: &ListKey) -> bool {
        self.saved_albums.iter().any(|k| k.same(key))
    }

    /// Saves or unsaves an album. Returns true when it is now saved.
    pub fn toggle_saved(&mut self, key: ListKey) -> bool {
        if let Some(i) = self.saved_albums.iter().position(|k| k.same(&key)) {
            self.saved_albums.remove(i);
            false
        } else {
            self.saved_albums.insert(0, key);
            true
        }
    }

    /// Remembers an opened list: front, deduped, capped.
    pub fn push_recent_list(&mut self, key: ListKey) {
        self.recent_lists.retain(|k| !k.same(&key));
        self.recent_lists.insert(0, key);
        self.recent_lists.truncate(RECENT_LISTS_LIMIT);
    }

    pub fn create_playlist(&mut self, name: &str) -> &mut Playlist {
        self.playlists.push(Playlist {
            id: uuid::Uuid::new_v4().simple().to_string(),
            name: name.trim().to_string(),
            tracks: vec![],
            origin: None,
            cover: None,
        });
        self.playlists.last_mut().expect("just pushed")
    }

    pub fn playlist(&self, id: &str) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    pub fn playlist_mut(&mut self, id: &str) -> Option<&mut Playlist> {
        self.playlists.iter_mut().find(|p| p.id == id)
    }

    pub fn rename_playlist(&mut self, id: &str, name: &str) {
        if let Some(p) = self.playlist_mut(id) {
            p.name = name.trim().to_string();
        }
    }

    pub fn set_playlist_cover(&mut self, id: &str, cover: Option<String>) {
        if let Some(p) = self.playlist_mut(id) {
            p.cover = cover;
        }
    }

    pub fn delete_playlist(&mut self, id: &str) {
        self.playlists.retain(|p| p.id != id);
        self.recent_lists.retain(|k| k != &ListKey::Playlist { id: id.to_string() });
    }

    /// Appends a song unless the playlist already has it. Returns `true` if added.
    pub fn add_to_playlist(&mut self, id: &str, track: Track) -> bool {
        let Some(p) = self.playlist_mut(id) else { return false };
        if p.tracks.iter().any(|t| same_entry(t, &track)) {
            return false;
        }
        p.tracks.push(track);
        true
    }

    pub fn remove_from_playlist(&mut self, id: &str, index: usize) {
        if let Some(p) = self.playlist_mut(id) {
            if index < p.tracks.len() {
                p.tracks.remove(index);
            }
        }
    }

    pub fn move_in_playlist(&mut self, id: &str, from: usize, to: usize) {
        if let Some(p) = self.playlist_mut(id) {
            if from < p.tracks.len() && to < p.tracks.len() {
                let t = p.tracks.remove(from);
                p.tracks.insert(to, t);
            }
        }
    }

    /// Saves imported tracks as a playlist. If a playlist with the same
    /// `origin` already exists, only songs that are not in it yet are added
    /// (re-import). Returns the playlist and how many songs were added.
    pub fn save_import(&mut self, name: &str, origin: Option<&str>, tracks: Vec<Track>) -> (&mut Playlist, usize) {
        let existing = origin.and_then(|o| self.playlists.iter().position(|p| p.origin.as_deref() == Some(o)));
        let idx = match existing {
            Some(i) => i,
            None => {
                let p = self.create_playlist(name);
                p.origin = origin.map(str::to_string);
                self.playlists.len() - 1
            }
        };
        let p = &mut self.playlists[idx];
        let mut added = 0;
        for t in tracks {
            if !p.tracks.iter().any(|x| same_entry(x, &t)) {
                p.tracks.push(t);
                added += 1;
            }
        }
        (&mut self.playlists[idx], added)
    }

    /// Adds imported tracks to Liked Songs (no duplicates). Returns how many were added.
    pub fn like_all(&mut self, tracks: Vec<Track>) -> usize {
        let mut added = 0;
        for t in tracks {
            if !self.is_liked(&t) {
                self.liked.push(t);
                added += 1;
            }
        }
        added
    }

    /// Lyrics offset in ms for a track (default 0), stepped by ±100 in the UI.
    pub fn lyrics_offset(&self, track: &Track) -> i64 {
        self.lyrics_offsets.get(&track_key(track)).copied().unwrap_or(0)
    }

    pub fn set_lyrics_offset(&mut self, track: &Track, offset_ms: i64) {
        let key = track_key(track);
        if offset_ms == 0 {
            self.lyrics_offsets.remove(&key);
        } else {
            self.lyrics_offsets.insert(key, offset_ms);
        }
    }
}

/// Same library entry: same provider id, or the same song by title/artist/duration.
fn same_entry(a: &Track, b: &Track) -> bool {
    (a.source == b.source && a.id == b.id)
        || a.external_ids.get(&b.source).map(|id| id == &b.id).unwrap_or(false)
        || b.external_ids.get(&a.source).map(|id| id == &a.id).unwrap_or(false)
        || same_song(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedupe::test_track;
    use glyph_protocol::ProviderId;

    fn tr(id: &str, title: &str) -> Track {
        test_track(ProviderId::Audius, id, title, "Band", Some(180_000))
    }

    #[test]
    fn saved_albums_and_recent_lists() {
        let mut lib = Library::default();
        let a = ListKey::Album { title: "Discovery".into(), artist: "Daft Punk".into(), tracks: vec![tr("1", "One")] };
        let same = ListKey::Album { title: "discovery".into(), artist: "daft punk".into(), tracks: vec![] };
        assert!(lib.toggle_saved(a.clone()));
        assert!(lib.is_saved(&same));
        assert!(!lib.toggle_saved(same));
        assert!(lib.saved_albums.is_empty());
        for i in 0..10 {
            lib.push_recent_list(ListKey::Playlist { id: i.to_string() });
        }
        lib.push_recent_list(ListKey::Playlist { id: "5".into() });
        assert_eq!(lib.recent_lists.len(), RECENT_LISTS_LIMIT);
        assert_eq!(lib.recent_lists[0], ListKey::Playlist { id: "5".into() });
        lib.playlists.push(Playlist { id: "5".into(), name: "x".into(), tracks: vec![], origin: None, cover: None });
        lib.delete_playlist("5");
        assert!(!lib.recent_lists.contains(&ListKey::Playlist { id: "5".into() }));
    }

    #[test]
    fn search_history_front_dedupe_cap_remove() {
        let mut lib = Library::default();
        for i in 0..25 {
            lib.push_search(tr(&i.to_string(), &format!("Song {i}")));
        }
        assert_eq!(lib.search_history.len(), SEARCH_HISTORY_LIMIT);
        assert_eq!(lib.search_history[0].title, "Song 24");
        lib.push_search(tr("20", "Song 20"));
        assert_eq!(lib.search_history[0].title, "Song 20");
        assert_eq!(lib.search_history.iter().filter(|t| t.title == "Song 20").count(), 1);
        lib.remove_search(0);
        assert_eq!(lib.search_history[0].title, "Song 24");
        lib.remove_search(999);
    }

    #[test]
    fn like_toggle_and_recent_cap() {
        let mut lib = Library::default();
        let t = tr("1", "Song");
        assert!(!lib.is_liked(&t));
        lib.toggle_like(&t);
        assert!(lib.is_liked(&t));
        // same song from another provider counts as liked
        let other = test_track(ProviderId::Deezer, "d9", "Song (Remastered)", "Band", Some(181_000));
        assert!(lib.is_liked(&other));
        lib.toggle_like(&t);
        assert!(!lib.is_liked(&t));

        for i in 0..60 {
            lib.push_recent(tr(&i.to_string(), &format!("Song {i}")));
        }
        assert_eq!(lib.recent.len(), 50);
        assert_eq!(lib.recent[0].id, "59");
        lib.push_recent(tr("30", "Song 30"));
        assert_eq!(lib.recent.len(), 50);
        assert_eq!(lib.recent[0].id, "30");
        assert_eq!(lib.recent.iter().filter(|t| t.id == "30").count(), 1);
    }

    #[test]
    fn playlists_crud_and_reimport() {
        let mut lib = Library::default();
        let id = lib.create_playlist("  Mix ").id.clone();
        assert_eq!(lib.playlist(&id).unwrap().name, "Mix");
        lib.rename_playlist(&id, "Better");
        assert!(lib.add_to_playlist(&id, tr("1", "A")));
        assert!(!lib.add_to_playlist(&id, tr("1", "A")));
        assert!(lib.add_to_playlist(&id, tr("2", "B")));
        lib.move_in_playlist(&id, 1, 0);
        assert_eq!(lib.playlist(&id).unwrap().tracks[0].id, "2");
        lib.remove_from_playlist(&id, 0);
        assert_eq!(lib.playlist(&id).unwrap().tracks.len(), 1);
        lib.delete_playlist(&id);
        assert!(lib.playlists.is_empty());

        let (p, added) = lib.save_import("Imp", Some("https://x/pl"), vec![tr("1", "A"), tr("2", "B")]);
        assert_eq!((p.tracks.len(), added), (2, 2));
        let (p, added) = lib.save_import("Imp", Some("https://x/pl"), vec![tr("2", "B"), tr("3", "C")]);
        assert_eq!((p.tracks.len(), added), (3, 1));
        assert_eq!(lib.playlists.len(), 1);
    }

    #[test]
    fn offsets_and_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.json");
        assert_eq!(Library::load_from(&path).unwrap(), Library::default());

        let mut lib = Library::default();
        let t = tr("1", "Song");
        lib.set_lyrics_offset(&t, -300);
        assert_eq!(lib.lyrics_offset(&t), -300);
        lib.toggle_like(&t);
        lib.save_to(&path).unwrap();
        let back = Library::load_from(&path).unwrap();
        assert_eq!(back, lib);
        let mut back = back;
        back.set_lyrics_offset(&t, 0);
        assert_eq!(back.lyrics_offset(&t), 0);

        std::fs::write(&path, b"{ nope").unwrap();
        assert!(Library::load_from(&path).is_err());
        assert!(path.with_extension("json.bak").exists());
    }
}
