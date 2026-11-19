//! Pages the app loaded lately (artist pages, album track lists), kept in
//! memory and on disk so they show at once, also after a restart. A page
//! older than its "fresh" time is still shown first and refreshed behind it.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct PageCache {
    name: &'static str,
    /// In memory at most this many; on disk at most `max * 8`.
    max: usize,
    mem: Mutex<HashMap<String, (u64, Value)>>,
}

impl PageCache {
    pub fn new(name: &'static str, max: usize) -> Self {
        Self { name, max, mem: Mutex::new(HashMap::new()) }
    }

    fn dir(&self) -> PathBuf {
        glyph_core::paths::cache_dir().join("pages").join(self.name)
    }

    fn file(&self, key: &str) -> PathBuf {
        self.dir().join(format!("{}.json", glyph_core::paths::hash_hex(key)))
    }

    /// The page and its age in seconds.
    pub fn get(&self, key: &str) -> Option<(u64, Value)> {
        let now = glyph_core::paths::now_unix();
        if let Some((at, v)) = self.mem.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
            return Some((now.saturating_sub(*at), v.clone()));
        }
        let saved: Value = glyph_core::paths::read_json(&self.file(key))?;
        // Another key with the same hash: not ours.
        if saved["key"].as_str() != Some(key) {
            return None;
        }
        let at = saved["at"].as_u64()?;
        let v = saved.get("page")?.clone();
        self.remember(key, at, v.clone());
        Some((now.saturating_sub(at), v))
    }

    pub fn put(&self, key: &str, page: &Value) {
        let at = glyph_core::paths::now_unix();
        self.remember(key, at, page.clone());
        let path = self.file(key);
        let body = json!({ "key": key, "at": at, "page": page });
        let dir = self.dir();
        let max_files = self.max * 8;
        // Off the caller's thread: writing must never hold up a page.
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&dir);
            let _ = glyph_core::paths::write_json(&path, &body);
            prune_dir(&dir, max_files);
        });
    }

    fn remember(&self, key: &str, at: u64, v: Value) {
        let mut m = self.mem.lock().unwrap_or_else(|e| e.into_inner());
        while m.len() >= self.max && !m.contains_key(key) {
            let Some(oldest) = m.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone()) else { break };
            m.remove(&oldest);
        }
        m.insert(key.to_string(), (at, v));
    }

    /// Memory pressure: forget what is in memory (the disk copies stay).
    pub fn trim(&self) {
        let mut m = self.mem.lock().unwrap_or_else(|e| e.into_inner());
        m.clear();
        m.shrink_to_fit();
    }
}

/// Keeps the newest `max` files of `dir`.
fn prune_dir(dir: &std::path::Path, max: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= max {
        return;
    }
    files.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    for (_, p) in files.into_iter().skip(max) {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_at_most_max_in_memory() {
        let c = PageCache::new("test-mem", 2);
        c.remember("a", 1, json!(1));
        c.remember("b", 2, json!(2));
        c.remember("c", 3, json!(3));
        let m = c.mem.lock().unwrap();
        assert_eq!(m.len(), 2);
        assert!(!m.contains_key("a"));
    }
}
