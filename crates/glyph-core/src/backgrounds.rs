//! Background pictures the user added (Settings › Appearance › Background).
//! Each lives in `<data>/backgrounds/<hash>.<ext>`; the UI scales a picture
//! down before it sends it, this file only checks and stores the bytes.
//! Plugins bring their own pictures (`look.backgrounds`, see
//! [`crate::plugins::LookSpec`]); those stay in the plugin's folder.

use crate::paths;
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// One picture at most this big (the UI sends ~0.5-3 MB).
pub const MAX_BYTES: usize = 12 * 1024 * 1024;
/// Pictures kept at most.
pub const MAX_COUNT: usize = 40;

pub fn dir() -> PathBuf {
    paths::data_dir().join("backgrounds")
}

/// File type of picture bytes, by their first bytes.
pub fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// `<16 hex>.<png|jpg|webp>`: a name [`add`] made (nothing else is read).
pub fn valid_name(name: &str) -> bool {
    let Some((stem, ext)) = name.split_once('.') else { return false };
    stem.len() == 16 && stem.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) && matches!(ext, "png" | "jpg" | "webp")
}

/// The file of picture `name`, if it is one.
pub fn path(name: &str) -> Option<PathBuf> {
    valid_name(name).then(|| dir().join(name)).filter(|p| p.is_file())
}

/// Stored pictures, newest first.
pub fn list() -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut out: Vec<(std::time::SystemTime, String)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let meta = e.metadata().ok()?;
            (valid_name(&name) && meta.is_file()).then(|| (meta.modified().unwrap_or(std::time::UNIX_EPOCH), name))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    out.into_iter().map(|(_, n)| n).collect()
}

/// Stores a picture (PNG, JPEG or WebP) and returns its name. The same
/// picture twice keeps one file.
pub fn add(bytes: &[u8]) -> Result<String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        bail!("the picture must be smaller than {} MB", MAX_BYTES / (1024 * 1024));
    }
    let Some(ext) = image_ext(bytes) else { bail!("only PNG, JPEG and WebP pictures work") };
    let hash = Sha256::digest(bytes);
    let stem: String = hash.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let name = format!("{stem}.{ext}");
    let d = dir();
    let file = d.join(&name);
    if file.is_file() {
        return Ok(name);
    }
    if list().len() >= MAX_COUNT {
        bail!("at most {MAX_COUNT} pictures; remove one first");
    }
    std::fs::create_dir_all(&d)?;
    // Written beside, then moved: the UI never loads half a picture.
    let tmp = d.join(format!("{name}.part"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &file)?;
    Ok(name)
}

/// [`add`] for a picture sent as base64 (from the UI).
pub fn add_base64(data: &str) -> Result<String> {
    use base64::Engine;
    // Base64 is 4/3 the size: refuse oversize text before decoding it.
    if data.len() > MAX_BYTES / 3 * 4 + 8 {
        bail!("the picture must be smaller than {} MB", MAX_BYTES / (1024 * 1024));
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|_| anyhow::anyhow!("not a picture"))?;
    add(&bytes)
}

pub fn remove(name: &str) -> Result<()> {
    if !valid_name(name) {
        bail!("invalid picture name");
    }
    let file = dir().join(name);
    if file.exists() {
        std::fs::remove_file(file)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_types() {
        assert!(valid_name("0123456789abcdef.webp"));
        assert!(!valid_name("0123456789ABCDEF.webp"));
        assert!(!valid_name("../0123456789abcdef.png"));
        assert!(!valid_name("0123456789abcdef.gif"));
        assert!(!valid_name("0123456789abcde.png"));
        assert_eq!(image_ext(b"\x89PNG\r\n\x1a\nxxxx"), Some("png"));
        assert_eq!(image_ext(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
        assert_eq!(image_ext(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(image_ext(b"GIF89a"), None);
        assert!(add(b"<svg/>").is_err());
    }
}
