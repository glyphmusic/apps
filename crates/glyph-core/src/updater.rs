//! "Check for updates" + auto-update from GitHub releases.
//!
//! * `GET https://api.github.com/repos/{repo}/releases/latest` (channel
//!   "prerelease": `/releases`, first non-draft).
//! * SemVer compare with `env!("CARGO_PKG_VERSION")`.
//! * Asset naming: `glyph-{version}-{target}.zip` (Windows) / `.tar.gz`
//!   (macOS, Linux) plus `{asset}.sha256`. Verify the hash before replacing.
//! * Package-manager installs (under /usr, Homebrew Cellar, WinGet) give
//!   `UpdateStatus::Available` with `can_self_install = false`.
//! * At most one automatic check per 24 h (timestamp in the data dir).

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    pub html_url: String,
    pub asset_url: Option<String>,
    pub sha256_url: Option<String>,
    pub can_self_install: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    UpToDate { current: String },
    Available(UpdateInfo),
    /// Automatic check skipped (checked less than 24 h ago).
    Skipped,
}

/// Rust target triple this binary was built for, used to pick the asset.
pub fn current_target() -> &'static str {
    todo!()
}

/// `true` if `candidate` is a newer SemVer than `current` (leading `v` allowed).
pub fn is_newer(current: &str, candidate: &str) -> bool {
    let _ = (current, candidate);
    todo!()
}
