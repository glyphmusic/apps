//! Music folders on this computer ("Local files"). Audio files are read in
//! place (tags and embedded cover, see [`crate::import::local::read_track`])
//! and never copied or uploaded. The result of a scan is kept in
//! `<data>/local_library.json`, so the next scan only reads new or changed
//! files.
//!
//! Songs from here carry the id `file:<hash of the path>` and are
//! metadata-only tracks (`ProviderId::MusicBrainz`): on another device, or
//! once the file is gone, they resolve by title and artist like any song.

use crate::dedupe::track_key;
use crate::import::local::{is_audio, read_track};
use crate::paths;
use glyph_protocol::Track;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::RwLock;

/// Folders are walked this deep.
const MAX_DEPTH: usize = 12;
/// At most this many files (a whole disk picked by mistake stays bounded).
pub const MAX_FILES: usize = 50_000;
/// Id prefix of songs from the music folders.
pub const ID_PREFIX: &str = "file:";

/// One audio file in a music folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalFile {
    pub path: PathBuf,
    pub track: Track,
    /// Modification time (unix seconds) and size when the tags were read.
    pub mtime: u64,
    pub size: u64,
    /// An embedded cover was saved (see [`cover_path`]).
    #[serde(default)]
    pub cover: bool,
}

/// What one scan found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanReport {
    /// Audio files in the folders.
    pub files: usize,
    /// Files whose tags were read this time (new or changed).
    pub read: usize,
    /// Files that could not be read.
    pub failed: usize,
}

/// Subfolders and songs of one folder (the folder browser).
#[derive(Debug, Default, Clone)]
pub struct FolderView {
    /// Subfolder and how many songs it holds (all levels below).
    pub dirs: Vec<(PathBuf, usize)>,
    /// Songs directly in the folder, in file name order.
    pub tracks: Vec<Track>,
}

/// One album of the music folders.
#[derive(Debug, Clone)]
pub struct LocalAlbum {
    pub title: String,
    pub artist: String,
    pub tracks: Vec<Track>,
}

#[derive(Default)]
pub struct LocalLibrary {
    files: RwLock<Vec<LocalFile>>,
    by_id: RwLock<HashMap<String, usize>>,
    by_key: RwLock<HashMap<String, usize>>,
    /// Counts scans that changed something (views cache by it).
    generation: AtomicU64,
    scanning: AtomicBool,
    /// Files read so far / found in the running scan.
    progress: (AtomicUsize, AtomicUsize),
}

fn index_path() -> PathBuf {
    paths::data_dir().join("local_library.json")
}

/// Embedded cover of a song from the music folders.
pub fn cover_path(t: &Track) -> PathBuf {
    paths::cache_dir().join("local-covers").join(format!("{}.jpg", paths::hash_hex(&t.id)))
}

fn mtime_of(m: &std::fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// Audio files below `roots`, sorted by path. Hidden folders and links to
/// folders are skipped (no loops).
pub fn walk(roots: &[PathBuf]) -> Vec<PathBuf> {
    fn go(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if out.len() >= MAX_FILES || depth > MAX_DEPTH {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<(PathBuf, std::fs::FileType)> = rd.flatten().filter_map(|e| Some((e.path(), e.file_type().ok()?))).collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (p, ty) in entries {
            if out.len() >= MAX_FILES {
                return;
            }
            let hidden = p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
            if ty.is_dir() && !hidden {
                go(&p, depth + 1, out);
            } else if ty.is_file() && is_audio(&p) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for r in roots {
        go(r, 0, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn sort_key(s: &str) -> String {
    s.to_lowercase()
}

impl LocalLibrary {
    /// The library as the last scan left it (nothing is read from the
    /// folders here).
    pub fn load() -> Self {
        let lib = Self::default();
        let files: Vec<LocalFile> = paths::read_json(&index_path()).unwrap_or_default();
        lib.install(files);
        lib
    }

    fn install(&self, files: Vec<LocalFile>) {
        let mut by_id = HashMap::with_capacity(files.len());
        let mut by_key = HashMap::with_capacity(files.len());
        for (i, f) in files.iter().enumerate() {
            by_id.entry(f.track.id.clone()).or_insert(i);
            by_key.entry(track_key(&f.track)).or_insert(i);
        }
        *self.files.write().unwrap() = files;
        *self.by_id.write().unwrap() = by_id;
        *self.by_key.write().unwrap() = by_key;
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Changes whenever the songs change (views cache by it).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.files.read().unwrap().is_empty()
    }

    pub fn len(&self) -> usize {
        self.files.read().unwrap().len()
    }

    /// `(read, found)` while a scan runs.
    pub fn scan_progress(&self) -> Option<(usize, usize)> {
        self.scanning.load(Ordering::Relaxed).then(|| (self.progress.0.load(Ordering::Relaxed), self.progress.1.load(Ordering::Relaxed)))
    }

    /// Index lookup only (cheap enough for every rendered row).
    pub fn contains(&self, t: &Track) -> bool {
        t.id.starts_with(ID_PREFIX) && self.by_id.read().unwrap().contains_key(&t.id)
    }

    /// The file a song from the music folders plays from. Only `file:`
    /// songs: a streamed song never turns into a local file by accident.
    pub fn local(&self, t: &Track) -> Option<PathBuf> {
        if !t.id.starts_with(ID_PREFIX) {
            return None;
        }
        // A scan can replace the files between reading an index and looking
        // up its row. Resolve against one snapshot so playback cannot pick
        // another song at the old row number.
        let files = self.files.read().unwrap();
        let file = files.iter().find(|f| f.track.id == t.id)
            .or_else(|| files.iter().find(|f| track_key(&f.track) == track_key(t)))?;
        let p = file.path.clone();
        p.is_file().then_some(p)
    }

    /// The saved embedded cover of a song from the music folders.
    pub fn cover(&self, t: &Track) -> Option<PathBuf> {
        if !t.id.starts_with(ID_PREFIX) {
            return None;
        }
        let p = cover_path(t);
        p.is_file().then_some(p)
    }

    /// Reads the folders: new and changed files get their tags read, files
    /// that are gone leave. Blocking: run off the UI thread. A second scan
    /// while one runs returns at once.
    pub fn scan(&self, roots: &[PathBuf]) -> ScanReport {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return ScanReport::default();
        }
        struct Done<'a>(&'a AtomicBool);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done(&self.scanning);
        let found = walk(roots);
        self.progress.0.store(0, Ordering::Relaxed);
        self.progress.1.store(found.len(), Ordering::Relaxed);
        let known: HashMap<PathBuf, LocalFile> = self.files.read().unwrap().iter().map(|f| (f.path.clone(), f.clone())).collect();
        let mut report = ScanReport { files: found.len(), ..Default::default() };
        let mut out = Vec::with_capacity(found.len());
        let mut changed = known.len() != found.len();
        for path in found {
            self.progress.0.fetch_add(1, Ordering::Relaxed);
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            let (mtime, size) = (mtime_of(&meta), meta.len());
            if let Some(f) = known.get(&path).filter(|f| f.mtime == mtime && f.size == size) {
                out.push(f.clone());
                continue;
            }
            changed = true;
            match read_track(&path) {
                Ok((mut track, cover)) => {
                    track.id = format!("{ID_PREFIX}{}", paths::hash_hex(&path.to_string_lossy()));
                    let mut has_cover = false;
                    if let Some(bytes) = cover {
                        let dest = cover_path(&track);
                        has_cover = dest.is_file()
                            || crate::cover::normalize_image(&bytes)
                                .ok()
                                .and_then(|jpeg| {
                                    let _ = std::fs::create_dir_all(dest.parent()?);
                                    paths::write_atomic(&dest, &jpeg).ok()
                                })
                                .is_some();
                    }
                    report.read += 1;
                    out.push(LocalFile { path, track, mtime, size, cover: has_cover });
                }
                Err(e) => {
                    tracing::debug!("local file {} unreadable: {e:#}", path.display());
                    report.failed += 1;
                }
            }
        }
        if changed {
            if let Err(e) = paths::write_json(&index_path(), &out) {
                tracing::warn!("could not save the local library: {e}");
            }
            self.install(out);
        }
        report
    }

    /// Forgets every song (all folders were removed).
    pub fn clear(&self) {
        let _ = std::fs::remove_file(index_path());
        self.install(Vec::new());
    }

    /// Every song, by artist, album, then file name.
    pub fn tracks(&self) -> Vec<Track> {
        let files = self.files.read().unwrap();
        let mut v: Vec<&LocalFile> = files.iter().collect();
        v.sort_by_cached_key(|f| (sort_key(&f.track.artist), sort_key(f.track.album.as_deref().unwrap_or("")), f.path.clone()));
        v.into_iter().map(|f| f.track.clone()).collect()
    }

    /// Subfolders and songs of `dir`.
    pub fn folder(&self, dir: &Path) -> FolderView {
        let files = self.files.read().unwrap();
        let mut dirs: HashMap<PathBuf, usize> = HashMap::new();
        let mut tracks: Vec<&LocalFile> = Vec::new();
        for f in files.iter() {
            let Ok(rel) = f.path.strip_prefix(dir) else { continue };
            let mut parts = rel.components();
            let Some(first) = parts.next() else { continue };
            if parts.next().is_none() {
                tracks.push(f);
            } else {
                *dirs.entry(dir.join(first)).or_default() += 1;
            }
        }
        tracks.sort_by(|a, b| a.path.cmp(&b.path));
        let mut dirs: Vec<(PathBuf, usize)> = dirs.into_iter().collect();
        dirs.sort_by_cached_key(|(p, _)| sort_key(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        FolderView { dirs, tracks: tracks.into_iter().map(|f| f.track.clone()).collect() }
    }

    /// Every song in `dir` and the folders below it, in path order.
    pub fn tracks_below(&self, dir: &Path) -> Vec<Track> {
        let files = self.files.read().unwrap();
        let mut v: Vec<&LocalFile> = files.iter().filter(|f| f.path.starts_with(dir)).collect();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        v.into_iter().map(|f| f.track.clone()).collect()
    }

    /// Songs in `root` and every folder below it (a music folder's count).
    pub fn count_in(&self, root: &Path) -> usize {
        self.files.read().unwrap().iter().filter(|f| f.path.starts_with(root)).count()
    }

    /// Artists with their number of songs, by name.
    pub fn artists(&self) -> Vec<(String, usize)> {
        let files = self.files.read().unwrap();
        let mut m: HashMap<String, (String, usize)> = HashMap::new();
        for f in files.iter() {
            let a = crate::dedupe::primary_artist(&f.track.artist);
            m.entry(sort_key(&a)).or_insert_with(|| (a.clone(), 0)).1 += 1;
        }
        let mut v: Vec<(String, (String, usize))> = m.into_iter().collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v.into_iter().map(|(_, x)| x).collect()
    }

    /// Songs whose (first) artist is `name`, by album and file name.
    pub fn by_artist(&self, name: &str) -> Vec<Track> {
        let key = sort_key(name);
        self.tracks().into_iter().filter(|t| sort_key(&crate::dedupe::primary_artist(&t.artist)) == key).collect()
    }

    /// Albums (songs with an album tag), by artist and title; songs in file
    /// name order.
    pub fn albums(&self) -> Vec<LocalAlbum> {
        let files = self.files.read().unwrap();
        let mut m: HashMap<(String, String), LocalAlbum> = HashMap::new();
        let mut sorted: Vec<&LocalFile> = files.iter().filter(|f| f.track.album.as_deref().is_some_and(|a| !a.trim().is_empty())).collect();
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        for f in sorted {
            let title = f.track.album.clone().unwrap_or_default();
            let artist = crate::dedupe::primary_artist(&f.track.artist);
            m.entry((sort_key(&artist), sort_key(&title)))
                .or_insert_with(|| LocalAlbum { title, artist, tracks: Vec::new() })
                .tracks
                .push(f.track.clone());
        }
        let mut v: Vec<((String, String), LocalAlbum)> = m.into_iter().collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v.into_iter().map(|(_, a)| a).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, artist: &str, album: Option<&str>) -> LocalFile {
        let mut track = crate::dedupe::test_track(glyph_protocol::ProviderId::MusicBrainz, "", path, artist, None);
        track.id = format!("{ID_PREFIX}{}", paths::hash_hex(path));
        track.album = album.map(str::to_string);
        LocalFile { path: PathBuf::from(path), track, mtime: 0, size: 0, cover: false }
    }

    #[test]
    fn folders_artists_and_albums() {
        let lib = LocalLibrary::default();
        let root = Path::new("/music");
        lib.install(vec![
            file("/music/a.mp3", "Band", None),
            file("/music/Band/Album/01.mp3", "Band feat. X", Some("Album")),
            file("/music/Band/Album/02.mp3", "Band", Some("Album")),
            file("/music/Other/x.flac", "Other", Some("Live")),
        ]);
        let v = lib.folder(root);
        assert_eq!(v.tracks.len(), 1);
        assert_eq!(v.dirs, vec![(PathBuf::from("/music/Band"), 2), (PathBuf::from("/music/Other"), 1)]);
        assert_eq!(lib.count_in(root), 4);
        assert_eq!(lib.artists(), vec![("Band".to_string(), 3), ("Other".to_string(), 1)]);
        assert_eq!(lib.by_artist("band").len(), 3);
        let albums = lib.albums();
        assert_eq!(albums.len(), 2);
        assert_eq!((albums[0].title.as_str(), albums[0].tracks.len()), ("Album", 2));
    }

    #[test]
    fn only_file_songs_play_from_the_folders() {
        let lib = LocalLibrary::default();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("song.mp3");
        std::fs::write(&p, b"x").unwrap();
        let mut f = file(&p.to_string_lossy(), "Band", None);
        f.path = p.clone();
        let t = f.track.clone();
        lib.install(vec![f]);
        assert_eq!(lib.local(&t), Some(p));
        // The same song from a streaming source stays a stream.
        let mut streamed = t.clone();
        streamed.id = "yt:abc".into();
        assert_eq!(lib.local(&streamed), None);
    }
}
