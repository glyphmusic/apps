//! Local storage: where Glyph keeps covers, downloads, lyrics and its
//! catalogue caches, how much each takes, and clearing them.
//!
//! All functions touch the file system synchronously; call them off the UI
//! thread.

use crate::paths;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageKind {
    /// Cover art cache (LRU, limited by `storage.cache_limit_mb`).
    Covers,
    /// Songs saved for offline playback (never removed automatically).
    Downloads,
    /// Lyrics cache.
    Lyrics,
    /// Resolver mappings, search and quota caches.
    Catalog,
    /// HTTP range chunks and local stream cache (`audio-ranges`, `streams`).
    AudioRanges,
}

impl StorageKind {
    pub const ALL: [StorageKind; 5] = [
        Self::Covers,
        Self::Downloads,
        Self::Lyrics,
        Self::Catalog,
        Self::AudioRanges,
    ];
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StorageStats {
    pub covers: u64,
    pub downloads: u64,
    pub lyrics: u64,
    pub catalog: u64,
    pub audio_ranges: u64,
}

impl StorageStats {
    pub fn get(&self, kind: StorageKind) -> u64 {
        match kind {
            StorageKind::Covers => self.covers,
            StorageKind::Downloads => self.downloads,
            StorageKind::Lyrics => self.lyrics,
            StorageKind::Catalog => self.catalog,
            StorageKind::AudioRanges => self.audio_ranges,
        }
    }
    pub fn total(&self) -> u64 {
        self.covers + self.downloads + self.lyrics + self.catalog + self.audio_ranges
    }
}

pub fn covers_dir() -> PathBuf {
    paths::cache_dir().join("covers")
}

/// Folder the user picked for downloads (Settings > Storage).
static DOWNLOADS_OVERRIDE: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Sets the folder for downloads; `None` = [`default_downloads_dir`].
pub fn set_downloads_dir(dir: Option<PathBuf>) {
    *DOWNLOADS_OVERRIDE.write().unwrap() = dir.filter(|d| !d.as_os_str().is_empty());
}

/// Glyph's own downloads folder (in its data folder).
pub fn default_downloads_dir() -> PathBuf {
    paths::data_dir().join("downloads")
}

/// The downloads folder in use.
pub fn downloads_dir() -> PathBuf {
    custom_downloads_dir().unwrap_or_else(default_downloads_dir)
}

/// The folder the user picked, if any. It may hold their own files too, so
/// only what the download index names is ever counted or deleted there.
fn custom_downloads_dir() -> Option<PathBuf> {
    DOWNLOADS_OVERRIDE.read().unwrap().clone()
}

/// Files the download index of `dir` names (and the index itself).
fn indexed_files(dir: &Path) -> Vec<PathBuf> {
    let index: std::collections::HashMap<String, serde_json::Value> = paths::read_json(&dir.join("index.json")).unwrap_or_default();
    let mut files: Vec<PathBuf> = index
        .values()
        .filter_map(|e| e.get("file").and_then(|f| f.as_str()))
        // Only plain names inside the folder.
        .filter(|f| !f.contains(['/', '\\']) && *f != ".." && *f != ".")
        .map(|f| dir.join(f))
        .collect();
    files.push(dir.join("index.json"));
    files
}

fn downloads_size() -> u64 {
    match custom_downloads_dir() {
        Some(dir) => indexed_files(&dir).iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum(),
        None => dir_size(&default_downloads_dir()),
    }
}

/// Audio files the user imported (dropped onto a playlist). Not part of
/// the downloads, so clearing those keeps them.
pub fn imports_dir() -> PathBuf {
    paths::data_dir().join("imports")
}

pub fn lyrics_dir() -> PathBuf {
    paths::data_dir().join("lyrics")
}

pub fn audio_ranges_dir() -> PathBuf {
    paths::cache_dir().join("audio-ranges")
}

pub fn streams_dir() -> PathBuf {
    paths::cache_dir().join("streams")
}

/// Catalogue cache files (resolver mappings, YouTube quota bookkeeping).
fn catalog_files() -> Vec<PathBuf> {
    vec![paths::data_dir().join("resolver_cache.json"), paths::data_dir().join("youtube_quota.json")]
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

pub fn stats() -> StorageStats {
    StorageStats {
        covers: dir_size(&covers_dir()),
        downloads: downloads_size(),
        lyrics: dir_size(&lyrics_dir()),
        catalog: catalog_files().iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum(),
        audio_ranges: dir_size(&audio_ranges_dir()) + dir_size(&streams_dir()),
    }
}

/// Deletes one kind of stored data. Downloads are only removed when asked
/// for explicitly.
pub fn clear(kind: StorageKind) -> anyhow::Result<()> {
    let remove_dir = |d: PathBuf| -> anyhow::Result<()> {
        match std::fs::remove_dir_all(&d) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    };
    match kind {
        StorageKind::Covers => remove_dir(covers_dir()),
        StorageKind::Downloads => match custom_downloads_dir() {
            // The user's own folder: only Glyph's downloads go.
            Some(dir) => {
                for f in indexed_files(&dir) {
                    let _ = std::fs::remove_file(f);
                }
                Ok(())
            }
            None => remove_dir(default_downloads_dir()),
        },
        StorageKind::Lyrics => remove_dir(lyrics_dir()),
        StorageKind::Catalog => {
            for f in catalog_files() {
                let _ = std::fs::remove_file(f);
            }
            Ok(())
        }
        StorageKind::AudioRanges => {
            remove_dir(audio_ranges_dir())?;
            remove_dir(streams_dir())
        }
    }
}

/// "52 B", "3.4 MB", "1.2 GB".
pub fn format_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else if v < 10.0 {
        format!("{v:.1} {}", UNITS[i])
    } else {
        format!("{v:.0} {}", UNITS[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_names_only_files_inside_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.json"),
            r#"{"a":{"file":"A - B.mp3"},"b":{"file":"../evil.mp3"},"c":{"file":"sub/x.mp3"}}"#,
        )
        .unwrap();
        let files = indexed_files(dir.path());
        assert_eq!(files, vec![dir.path().join("A - B.mp3"), dir.path().join("index.json")]);
    }

    #[test]
    fn formats_bytes() {
        assert_eq!(format_bytes(52), "52 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
        assert_eq!(format_bytes(250 * 1024 * 1024), "250 MB");
    }

    #[test]
    fn dir_size_counts_nested_files() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a"), [0u8; 10]).unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub").join("b"), [0u8; 5]).unwrap();
        assert_eq!(dir_size(d.path()), 15);
        assert_eq!(dir_size(&d.path().join("missing")), 0);
    }
}
