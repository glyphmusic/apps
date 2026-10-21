//! Server-scoped bearer credentials. Never included in settings, sync or exports.
use anyhow::{anyhow, Result};
use reqwest::Url;

pub const HEADER: &str = "x-glyph-server-authorization";
pub fn secure_base(base: &str) -> bool {
    Url::parse(base).is_ok_and(|u| u.username().is_empty() && u.password().is_none() && (u.scheme() == "https" || u.scheme() == "http" && matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))))
}
pub fn same_server(base: &str, url: &str) -> bool {
    match (Url::parse(base), Url::parse(url)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin() && (b.path() == a.path().trim_end_matches('/') || b.path().starts_with(&format!("{}/", a.path().trim_end_matches('/')))),
        _ => false,
    }
}
#[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
fn entry(base: &str) -> Result<keyring::Entry> {
    keyring::Entry::new("glyph-server-access", &crate::paths::hash_hex(base.trim_end_matches('/'))).map_err(|_| anyhow!("server credential store unavailable"))
}
/// No keychain crate for Android: the app's private data folder (sandboxed per app).
#[cfg(not(any(windows, target_os = "macos", target_os = "ios", target_os = "linux")))]
fn fallback_path(base: &str) -> std::path::PathBuf {
    crate::paths::data_dir().join(format!("server_access_{}", crate::paths::hash_hex(base.trim_end_matches('/'))))
}
pub fn load(base: &str) -> Option<String> {
    if !secure_base(base) { return None; }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    return entry(base).ok()?.get_password().ok();
    #[cfg(not(any(windows, target_os = "macos", target_os = "ios", target_os = "linux")))]
    return std::fs::read_to_string(fallback_path(base)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
}
pub fn store(base: &str, token: &str) -> Result<()> {
    if !secure_base(base) { return Err(anyhow!("use HTTPS for a protected server (HTTP only on localhost)")); }
    if !token.is_empty() && (!(32..=256).contains(&token.len()) || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b))) { return Err(anyhow!("use a bearer with 32-256 ASCII characters")); }
    #[cfg(any(windows, target_os = "macos", target_os = "ios", target_os = "linux"))]
    {
        let e = entry(base)?;
        if token.is_empty() {
            match e.delete_credential() { Ok(()) | Err(keyring::Error::NoEntry) => Ok(()), Err(_) => Err(anyhow!("could not remove server credential")) }
        } else {
            e.set_password(token).map_err(|_| anyhow!("could not save server credential in OS keychain"))
        }
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "ios", target_os = "linux")))]
    {
        let path = fallback_path(base);
        if token.is_empty() {
            match std::fs::remove_file(&path) { Ok(()) => Ok(()), Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()), Err(_) => Err(anyhow!("could not remove server credential")) }
        } else {
            crate::paths::write_atomic(&path, token.as_bytes()).map_err(|_| anyhow!("could not save server credential"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn scope_and_transport() {
        assert!(secure_base("https://mine.example"));
        assert!(secure_base("http://127.0.0.1:9000"));
        assert!(!secure_base("http://mine.example"));
        assert!(!secure_base("https://user:pass@mine.example"));
        assert!(same_server("https://mine.example/api", "https://mine.example/api/v1/audio/x"));
        for url in ["https://mine.example.evil/api/x", "http://mine.example/api/x", "https://mine.example:444/api/x", "https://mine.example/api2/x"] { assert!(!same_server("https://mine.example/api",url)); }
    }
}

/// Small health-only probe; callers run this on a background executor.
pub fn probe(base: &str) -> Result<bool> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async {
        let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build()?;
        Ok(crate::server_client::ServerClient::new(base,http,String::new()).health().await?.server_auth_required)
    })
}
