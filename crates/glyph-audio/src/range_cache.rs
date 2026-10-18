//! Sparse local playback cache. Only confirmed byte intervals may be read;
//! receiving more ranges fills the same file, including after a restart.
use std::{fs::{self, OpenOptions}, io::{Read, Seek, SeekFrom, Write}, path::PathBuf, sync::{Mutex, RwLock}};
use sha2::{Digest, Sha256};

static DIRECTORY: RwLock<Option<PathBuf>> = RwLock::new(None);
static IO: Mutex<()> = Mutex::new(());
const MAX_TRACK: u64 = 32 * 1024 * 1024;
const MAX_CACHE: u64 = 512 * 1024 * 1024;

pub fn set_cache_dir(path: PathBuf) { *DIRECTORY.write().unwrap() = Some(path); }

#[derive(Clone)]
pub struct Entry { path: PathBuf, pub total: u64 }
type Manifest = (u64, Vec<(u64, u64)>);

impl Entry {
    pub fn open(url: &str, total: Option<u64>) -> Option<Self> {
        // Stable server URLs include the selected quality. Signed provider
        // URLs intentionally get separate entries when their identity changes.
        let dir = DIRECTORY.read().ok()?.clone()?;
        let hash = Sha256::digest(url.as_bytes());
        let key: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        Self::at(dir.join(format!("{key}.audio")), total)
    }

    fn at(path: PathBuf, total: Option<u64>) -> Option<Self> {
        let _guard = IO.lock().ok()?;
        let previous = Self::manifest(&path);
        let size = total.or_else(|| previous.as_ref().map(|m| m.0))?;
        if size == 0 || size > MAX_TRACK { return None; }
        if previous.as_ref().is_some_and(|m| m.0 == size)
            && fs::metadata(&path).is_ok_and(|meta| meta.len() == size) {
            if let Ok(file) = OpenOptions::new().write(true).open(&path) {
                let _ = file.set_modified(std::time::SystemTime::now());
            }
            return Some(Self { path, total: size });
        }
        total?;
        fs::create_dir_all(path.parent()?).ok()?;
        Self::prune(path.parent()?, size);
        let file = OpenOptions::new().create(true).truncate(true).write(true).open(&path).ok()?;
        file.set_len(size).ok()?;
        fs::write(path.with_extension("json"), serde_json::to_vec(&(size, Vec::<(u64,u64)>::new())).ok()?).ok()?;
        Some(Self { path, total: size })
    }

    fn manifest(path: &std::path::Path) -> Option<Manifest> {
        let manifest: Manifest = serde_json::from_slice(&fs::read(path.with_extension("json")).ok()?).ok()?;
        let (total, ranges) = &manifest;
        if *total == 0 || *total > MAX_TRACK || ranges.len() > 4096 { return None; }
        if ranges.iter().any(|(a,b)| a >= b || b > total)
            || ranges.windows(2).any(|w| w[0].1 >= w[1].0) { return None; }
        Some(manifest)
    }

    pub fn read(&self, at: u64, count: u64) -> Option<Vec<u8>> {
        let _guard = IO.lock().ok()?;
        let (total, ranges) = Self::manifest(&self.path)?;
        if total != self.total { return None; }
        let end = ranges.iter().find(|(a,b)| *a <= at && at < *b)?.1;
        let mut file = fs::File::open(&self.path).ok()?;
        if file.metadata().ok()?.len() != total { return None; }
        file.seek(SeekFrom::Start(at)).ok()?;
        let mut bytes = vec![0; count.min(end-at) as usize];
        file.read_exact(&mut bytes).ok()?;
        Some(bytes)
    }

    pub fn write(&self, at: u64, bytes: &[u8]) {
        let _ = self.store(at, bytes);
    }

    fn store(&self, at: u64, bytes: &[u8]) -> Option<()> {
        let end = at.checked_add(bytes.len() as u64)?;
        if bytes.is_empty() || end > self.total { return None; }
        let _guard = IO.lock().ok()?;
        let (total, mut ranges) = Self::manifest(&self.path)?;
        if total != self.total { return None; }
        let mut file = OpenOptions::new().write(true).open(&self.path).ok()?;
        if file.metadata().ok()?.len() != total { return None; }
        file.seek(SeekFrom::Start(at)).ok()?;
        file.write_all(bytes).ok()?;
        ranges.push((at, end));
        ranges.sort_unstable();
        let mut merged: Vec<(u64,u64)> = Vec::new();
        for (a,b) in ranges {
            if let Some(last) = merged.last_mut().filter(|last| last.1 >= a) { last.1 = last.1.max(b); }
            else { merged.push((a,b)); }
        }
        // Publishing intervals follows the writes: holes never count as audio.
        fs::write(self.path.with_extension("json"), serde_json::to_vec(&(total, merged)).ok()?).ok()?;
        Some(())
    }

    fn prune(dir: &std::path::Path, incoming: u64) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        let mut files: Vec<_> = entries.flatten().filter_map(|entry| {
            let path = entry.path();
            if path.extension()?.to_str()? != "audio" { return None; }
            let meta = entry.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), path))
        }).collect();
        files.sort_by_key(|v| v.0);
        let mut size = files.iter().map(|v| v.1).sum::<u64>() + incoming;
        for (_, len, path) in files {
            if size <= MAX_CACHE { break; }
            if fs::remove_file(&path).is_ok() {
                let _ = fs::remove_file(path.with_extension("json"));
                size = size.saturating_sub(len);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_merge_and_survive_reopen_without_exposing_holes() {
        let dir = std::env::temp_dir().join(format!("glyph-range-cache-{}", std::process::id()));
        let path = dir.join("test.audio");
        let entry = Entry::at(path.clone(), Some(10)).unwrap();
        entry.write(0, b"abc");
        entry.write(7, b"hij");
        assert_eq!(entry.read(0, 10).unwrap(), b"abc");
        assert!(entry.read(3, 4).is_none());
        let reopened = Entry::at(path.clone(), None).unwrap();
        reopened.write(3, b"defg");
        assert_eq!(reopened.read(0, 20).unwrap(), b"abcdefghij");
        assert_eq!(Entry::manifest(&path).unwrap().1, vec![(0,10)]);
        // A changed representation invalidates all previously received bytes.
        let changed = Entry::at(path.clone(), Some(11)).unwrap();
        assert!(changed.read(0, 10).is_none());
        fs::remove_file(&path).unwrap();
        fs::remove_file(path.with_extension("json")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
