//! Album names found for songs, remembered on this device.
//!
//! Songs saved without an album (old imports, YouTube finds) get their album
//! looked up when a list shows them ([`crate::cover::CoverCache::resolve_meta`]).
//! Every answer is written to `album-names.tsv` in the data folder, so a list
//! shows the album right away the next time, also after a restart or after
//! the library came back from sync without it. "Nothing found" is remembered
//! too and asked again only after [`RETRY_MISSING_DAYS`].
//!
//! The file only grows by appended lines (`key \t album \t unix seconds`, an
//! empty album means nothing was found); the last line of a song wins. It is
//! written fresh when it holds far more lines than songs.

use crate::dedupe::track_key;
use crate::library::Library;
use crate::paths;
use glyph_protocol::Track;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// A song with no album found is looked up again after this many days.
pub const RETRY_MISSING_DAYS: u64 = 14;
/// Longest album name kept (a broken answer must not fill the file).
const MAX_ALBUM_CHARS: usize = 300;

/// What is known about a song's album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Known {
    /// Found before.
    Album(String),
    /// Looked up recently, nothing found.
    Missing,
    /// Never looked up (or long ago).
    Unknown,
}

struct Entry {
    album: Option<String>,
    at: u64,
}

struct Store {
    path: Option<PathBuf>,
    map: HashMap<String, Entry>,
    lines: usize,
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store { path: None, map: HashMap::new(), lines: 0 }));

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn key_of(track: &Track) -> String {
    paths::hash_hex(&track_key(track))
}

fn clean(album: &str) -> Option<String> {
    let a: String = album.trim().replace(['\t', '\n', '\r'], " ").chars().take(MAX_ALBUM_CHARS).collect();
    (!a.is_empty()).then_some(a)
}

/// Where album names are remembered; loads what is there. Calling it again
/// with the same path keeps what is loaded.
pub fn set_store(path: PathBuf) {
    let mut s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    if s.path.as_ref() == Some(&path) {
        return;
    }
    s.map.clear();
    s.lines = 0;
    if let Ok(text) = std::fs::read_to_string(&path) {
        for line in text.lines() {
            let mut parts = line.splitn(3, '\t');
            let (Some(key), Some(album), Some(at)) = (parts.next(), parts.next(), parts.next()) else { continue };
            if key.is_empty() {
                continue;
            }
            s.lines += 1;
            s.map.insert(key.to_string(), Entry { album: clean(album), at: at.trim().parse().unwrap_or(0) });
        }
    }
    s.path = Some(path);
    if s.lines > 1000 && s.lines > s.map.len() * 2 {
        rewrite(&mut s);
    }
}

fn rewrite(s: &mut Store) {
    let Some(path) = s.path.clone() else { return };
    let mut out = String::with_capacity(s.map.len() * 48);
    for (k, e) in &s.map {
        out.push_str(k);
        out.push('\t');
        out.push_str(e.album.as_deref().unwrap_or(""));
        out.push('\t');
        out.push_str(&e.at.to_string());
        out.push('\n');
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tsv.tmp");
    if std::fs::write(&tmp, out).is_ok() && std::fs::rename(&tmp, &path).is_ok() {
        s.lines = s.map.len();
    }
}

/// What is known about `track`'s album.
pub fn lookup(track: &Track) -> Known {
    let s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    match s.map.get(&key_of(track)) {
        Some(Entry { album: Some(a), .. }) => Known::Album(a.clone()),
        Some(Entry { album: None, at }) if now().saturating_sub(*at) < RETRY_MISSING_DAYS * 86_400 => Known::Missing,
        _ => Known::Unknown,
    }
}

/// Remembers what a lookup for `track` found (`None`: nothing).
pub fn remember(track: &Track, album: Option<&str>) {
    let album = album.and_then(clean);
    let key = key_of(track);
    let mut s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(old) = s.map.get(&key) {
        // A found album is never replaced by "nothing found".
        if album.is_none() && old.album.is_some() {
            return;
        }
        if old.album == album && album.is_some() {
            return;
        }
    }
    let at = now();
    if let Some(path) = s.path.clone() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let line = format!("{key}\t{}\t{at}\n", album.as_deref().unwrap_or(""));
        let wrote = std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(e) = wrote {
            tracing::debug!("album names: {e}");
        } else {
            s.lines += 1;
        }
    }
    s.map.insert(key, Entry { album, at });
}

/// The remembered album of `track`, if one was found before.
pub fn album_of(track: &Track) -> Option<String> {
    match lookup(track) {
        Known::Album(a) => Some(a),
        _ => None,
    }
}

/// Gives every song in the library that has no album the one remembered for
/// it. Returns whether anything changed.
pub fn fill_library(lib: &mut Library) -> bool {
    let s = STORE.lock().unwrap_or_else(|e| e.into_inner());
    if s.map.is_empty() {
        return false;
    }
    let mut changed = false;
    let mut fill = |t: &mut Track| {
        if t.album.as_deref().is_some_and(|a| !a.trim().is_empty()) || t.title.trim().is_empty() {
            return;
        }
        if let Some(Entry { album: Some(a), .. }) = s.map.get(&key_of(t)) {
            t.album = Some(a.clone());
            changed = true;
        }
    };
    for t in &mut lib.liked {
        fill(t);
    }
    for pl in &mut lib.playlists {
        for t in &mut pl.tracks {
            fill(t);
        }
    }
    for r in &mut lib.recent {
        if let crate::library::RecentItem::Song { track, .. } = r {
            fill(track);
        }
    }
    changed
}

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str) -> Track {
        Track {
            id: title.into(),
            source: glyph_protocol::ProviderId::Audius,
            title: title.into(),
            artist: "Someone".into(),
            album: None,
            duration_ms: None,
            cover_url: None,
            isrc: None,
            permalink: None,
            external_ids: Default::default(),
            explicit: false, explicit_rating: None,
        }
    }

    #[test]
    fn found_albums_survive_a_restart_and_fill_the_library() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("album-names.tsv");
        set_store(path.clone());
        let (a, b, c) = (song("One"), song("Two"), song("Three"));
        assert_eq!(lookup(&a), Known::Unknown);
        remember(&a, Some("First Album"));
        remember(&b, None);
        // "Nothing found" never replaces a found album.
        remember(&a, None);
        assert_eq!(lookup(&a), Known::Album("First Album".into()));
        assert_eq!(lookup(&b), Known::Missing);

        // A restart: read back from the file.
        set_store(dir.path().join("other.tsv"));
        assert_eq!(lookup(&a), Known::Unknown);
        set_store(path.clone());
        assert_eq!(lookup(&a), Known::Album("First Album".into()));
        assert_eq!(lookup(&b), Known::Missing);
        assert_eq!(lookup(&c), Known::Unknown);

        let mut lib = Library::default();
        lib.liked = vec![a.clone(), b.clone(), Track { album: Some("Kept".into()), ..a.clone() }];
        assert!(fill_library(&mut lib));
        assert_eq!(lib.liked[0].album.as_deref(), Some("First Album"));
        assert_eq!(lib.liked[1].album, None);
        assert_eq!(lib.liked[2].album.as_deref(), Some("Kept"));
        assert!(!fill_library(&mut lib));
        set_store(dir.path().join("unused.tsv"));
    }

    #[test]
    fn old_misses_are_asked_again_and_names_are_cleaned() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("album-names.tsv");
        let old = now() - (RETRY_MISSING_DAYS + 1) * 86_400;
        let t = song("Old miss");
        std::fs::write(&path, format!("{}\t\t{old}\nbroken line\n", key_of(&t))).unwrap();
        set_store(path.clone());
        assert_eq!(lookup(&t), Known::Unknown);
        let u = song("Tabs");
        remember(&u, Some("  A\tB\n "));
        assert_eq!(lookup(&u), Known::Album("A B".into()));
        set_store(dir.path().join("unused.tsv"));
    }
}
