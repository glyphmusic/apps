//! Local audio files dropped onto a playlist: tags (title, artist, album,
//! ISRC, duration) and the embedded cover are read with symphonia, the file
//! is copied into the imports folder (`storage::imports_dir()`, same index
//! format as the downloads) and plays from there.
//!
//! Imported songs are metadata-only tracks (`ProviderId::MusicBrainz`), so on
//! another device, where the file is missing, they resolve like any other
//! song by title and artist.

use crate::dedupe::track_key;
use crate::downloads::Downloads;
use crate::paths;
use anyhow::{anyhow, Result};
use glyph_protocol::{ProviderId, Track};
use std::path::{Path, PathBuf};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey, StandardVisualKey};
use symphonia::core::probe::Hint;

/// Formats the native player decodes.
pub const AUDIO_EXTS: [&str; 9] = ["mp3", "flac", "ogg", "oga", "m4a", "mp4", "aac", "wav", "mka"];

/// Dropped folders are walked this deep.
const MAX_DEPTH: usize = 4;
/// One drop imports at most this many files.
pub const MAX_FILES: usize = 2000;

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Audio files among the dropped paths (folders are walked), sorted by path
/// inside each folder so albums keep their order.
pub fn collect(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(p: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if out.len() >= MAX_FILES {
            return;
        }
        if p.is_dir() {
            if depth >= MAX_DEPTH {
                return;
            }
            let Ok(rd) = std::fs::read_dir(p) else { return };
            let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            entries.sort();
            for e in entries {
                walk(&e, depth + 1, out);
            }
        } else if p.is_file() && is_audio(p) {
            out.push(p.to_path_buf());
        }
    }
    let mut out = Vec::new();
    for p in paths {
        walk(p, 0, &mut out);
    }
    out.truncate(MAX_FILES);
    out
}

/// What the tags of one file say.
#[derive(Debug, Default)]
struct Tags {
    title: Option<String>,
    artist: Option<String>,
    album_artist: Option<String>,
    album: Option<String>,
    isrc: Option<String>,
    cover: Option<Vec<u8>>,
}

impl Tags {
    fn absorb(&mut self, rev: &MetadataRevision) {
        for tag in rev.tags() {
            let v = tag.value.to_string().trim().trim_matches('\0').to_string();
            if v.is_empty() {
                continue;
            }
            let slot = match tag.std_key {
                Some(StandardTagKey::TrackTitle) => &mut self.title,
                Some(StandardTagKey::Artist) => &mut self.artist,
                Some(StandardTagKey::AlbumArtist) => &mut self.album_artist,
                Some(StandardTagKey::Album) => &mut self.album,
                Some(StandardTagKey::IdentIsrc) => &mut self.isrc,
                _ => continue,
            };
            slot.get_or_insert(v);
        }
        // Prefer the front cover, else any picture.
        let front = rev.visuals().iter().find(|v| v.usage == Some(StandardVisualKey::FrontCover));
        if let Some(v) = front.or_else(|| rev.visuals().first()) {
            if self.cover.is_none() || front.is_some() {
                self.cover = Some(v.data.to_vec());
            }
        }
    }
}

/// Reads the tags of `path` into a track; missing tags come from the file
/// name ("Artist - Title.mp3", "03 Title.flac").
pub fn read_track(path: &Path) -> Result<(Track, Option<Vec<u8>>)> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(&ext);
    let mut probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| anyhow!("unsupported audio file: {e}"))?;

    let mut tags = Tags::default();
    // Container tags (Vorbis comments, MP4 atoms) first, then ID3 in front.
    if let Some(rev) = probed.format.metadata().current() {
        tags.absorb(rev);
    }
    if let Some(mut m) = probed.metadata.get() {
        if let Some(rev) = m.skip_to_latest() {
            tags.absorb(rev);
        }
    }
    let duration_ms = probed.format.default_track().and_then(|t| {
        let p = &t.codec_params;
        match (p.n_frames, p.sample_rate) {
            (Some(n), Some(rate)) if rate > 0 => Some(n * 1000 / rate as u64),
            _ => None,
        }
    });

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let (file_artist, file_title) = super::files::split_artist_title(&clean_stem(&stem));
    let title = tags.title.unwrap_or(file_title);
    let artist = tags.artist.or(tags.album_artist).filter(|a| !a.is_empty()).unwrap_or(if file_artist.is_empty() {
        "Unknown".to_string()
    } else {
        file_artist
    });
    if title.trim().is_empty() {
        return Err(anyhow!("no title"));
    }
    let mut track = Track {
        id: String::new(),
        source: ProviderId::MusicBrainz,
        title,
        artist,
        album: tags.album,
        duration_ms: duration_ms.filter(|d| *d > 0),
        cover_url: None,
        isrc: tags.isrc,
        permalink: None,
        external_ids: Default::default(),
        explicit: false,
    };
    track.id = format!("local:{}", paths::hash_hex(&track_key(&track)));
    Ok((track, tags.cover))
}

/// "03 - Title" / "03. Title" -> "Title".
fn clean_stem(stem: &str) -> String {
    let trimmed = stem.trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '_' | ' '));
    if trimmed.is_empty() { stem } else { trimmed }.replace('_', " ").trim().to_string()
}

/// Cover file of an imported song (inside the imports folder).
pub fn cover_path(store: &Downloads, t: &Track) -> PathBuf {
    store.dir().join("covers").join(format!("{}.jpg", paths::hash_hex(&track_key(t))))
}

/// Result of one drop.
#[derive(Debug, Default)]
pub struct LocalImport {
    /// Imported songs, in drop order (already imported ones included).
    pub tracks: Vec<Track>,
    /// Files that could not be read or copied.
    pub failed: usize,
}

/// Imports `paths` (files or folders) into `store`. Blocking: run off the UI
/// thread.
pub fn import_files(store: &Downloads, paths: &[PathBuf]) -> LocalImport {
    let mut out = LocalImport::default();
    for path in collect(paths) {
        let res = read_track(&path).and_then(|(track, cover)| {
            if store.local(&track).is_none() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("mp3").to_ascii_lowercase();
                store.store_file(&track, ProviderId::MusicBrainz, &path, &ext)?;
            }
            if let Some(bytes) = cover {
                let dest = cover_path(store, &track);
                if !dest.is_file() {
                    match crate::cover::normalize_image(&bytes) {
                        Ok(jpeg) => {
                            if let Some(dir) = dest.parent() {
                                let _ = std::fs::create_dir_all(dir);
                            }
                            let _ = paths::write_atomic(&dest, &jpeg);
                        }
                        Err(e) => tracing::debug!("embedded cover of {} unreadable: {e}", path.display()),
                    }
                }
            }
            Ok(track)
        });
        match res {
            Ok(t) => {
                if !out.tracks.iter().any(|x| x.id == t.id) {
                    out.tracks.push(t);
                }
            }
            Err(e) => {
                tracing::info!("could not import {}: {e:#}", path.display());
                out.failed += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_extensions_and_stems() {
        assert!(is_audio(Path::new("/m/a.MP3")));
        assert!(is_audio(Path::new("x.flac")));
        assert!(!is_audio(Path::new("cover.jpg")));
        assert!(!is_audio(Path::new("noext")));
        assert_eq!(clean_stem("03 - Artist - Song_Name"), "Artist - Song Name");
        assert_eq!(clean_stem("1999"), "1999");
    }
}
