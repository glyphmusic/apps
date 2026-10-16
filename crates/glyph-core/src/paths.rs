//! Filesystem locations and small file helpers shared by the core modules.

use std::path::{Path, PathBuf};

/// App data dir (`dirs::data_dir()/glyph`); `GLYPH_DATA_DIR` overrides (tests, portable installs).
pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("GLYPH_DATA_DIR") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("glyph")
}

/// Cache dir (`dirs::cache_dir()/glyph`) for covers.
pub fn cache_dir() -> PathBuf {
    if let Ok(p) = std::env::var("GLYPH_CACHE_DIR") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".")).join("glyph")
}

/// Writes via a temp file + rename so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        // Windows cannot rename over an open file in some cases; fall back to copy.
        let r = std::fs::copy(&tmp, path).map(|_| ());
        let _ = std::fs::remove_file(&tmp);
        r.map_err(|_| e)?;
    }
    Ok(())
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?)
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Stable short hex hash (SHA-256 prefix) for cache file names.
pub fn hash_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(s.as_bytes());
    d.iter().take(10).map(|b| format!("{b:02x}")).collect()
}
