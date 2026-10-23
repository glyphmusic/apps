//! "Check for updates" + auto-update from GitHub releases.
//!
//! * `GET https://api.github.com/repos/{repo}/releases`: highest SemVer
//!   non-draft; channel "stable" also skips pre-releases. GitHub's "Latest"
//!   flag is not used, so a pre-release wrongly marked Latest stays out.
//! * SemVer compare with `env!("CARGO_PKG_VERSION")`.
//! * Asset naming: `glyph-{version}-{target}.zip` (Windows) / `.tar.gz`
//!   (macOS, Linux), `glyph-{version}-android.apk` (Android) plus
//!   `SHA256SUMS.txt` (legacy: `{asset}.sha256`). Verify before replacing.
//! * Android cannot replace its own binary: the verified APK goes to the
//!   system package installer (`set_apk_installer`), which asks the user.
//! * Package-manager installs (under /usr, Homebrew Cellar, WinGet) give
//!   `UpdateStatus::Available` with `can_self_install = false`.
//! * At most one automatic check per 24 h (timestamp in the data dir).
//!
//! No telemetry: the only request is the GitHub API call itself.

use crate::config::UpdatesConfig;
use crate::paths;
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// Automatic checks happen at most this often.
pub const CHECK_INTERVAL_SECS: u64 = 24 * 3600;
const MAX_DOWNLOAD_BYTES: u64 = 400 * 1024 * 1024;

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
    env!("GLYPH_TARGET")
}

/// Version of the running app.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn parse_version(s: &str) -> Option<semver::Version> {
    semver::Version::parse(s.trim().trim_start_matches(['v', 'V'])).ok()
}

/// `true` if `candidate` is a newer SemVer than `current` (leading `v` allowed).
pub fn is_newer(current: &str, candidate: &str) -> bool {
    match (parse_version(current), parse_version(candidate)) {
        (Some(c), Some(n)) => n > c,
        _ => false,
    }
}

// ------------------------------------------------------- package managers

/// Installs managed by a package manager must be updated by it.
pub fn is_package_managed_path(path: &Path) -> bool {
    let p = path.to_string_lossy().replace('\\', "/");
    let lower = p.to_lowercase();
    let markers = [
        "/cellar/",
        "/homebrew/",
        "/linuxbrew/",
        "/microsoft/winget/",
        "/windowsapps/",
        "/scoop/apps/",
        "/chocolatey/",
        "/nix/store/",
        "/snap/",
        "/flatpak/",
    ];
    if markers.iter().any(|m| lower.contains(m)) {
        return true;
    }
    // Distro packages live under /usr (but /usr/local is a manual install).
    lower.starts_with("/usr/") && !lower.starts_with("/usr/local/") || lower.starts_with("/app/")
}

/// Glyph's own .deb / .rpm from a GitHub release: the package manager owns
/// the files, but new versions come as a download, not from a distro repository.
pub fn is_release_package() -> bool {
    cfg!(target_os = "linux") && Path::new("/usr/share/glyph/release-package").exists()
}

fn exe_is_package_managed() -> bool {
    std::env::current_exe()
        .ok()
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
        .map(|p| is_package_managed_path(&p))
        .unwrap_or(true) // unknown location: do not touch it
}

// ------------------------------------------------------------ GitHub parse

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// Names accepted as the release asset for `version` + `target`.
pub fn asset_names(version: &str, target: &str) -> Vec<String> {
    let v = version.trim_start_matches(['v', 'V']);
    if target.ends_with("-android") {
        return vec![format!("glyph-{v}-android.apk")];
    }
    vec![format!("glyph-{v}-{target}.zip"), format!("glyph-{v}-{target}.tar.gz")]
}

type ApkInstaller = Box<dyn Fn(&Path) -> bool + Send + Sync>;
static APK_INSTALLER: std::sync::OnceLock<ApkInstaller> = std::sync::OnceLock::new();

/// Android: hands a downloaded, verified APK to the system installer
/// (`true` = the installer took it). Set once by the Android shell.
pub fn set_apk_installer(f: ApkInstaller) {
    let _ = APK_INSTALLER.set(f);
}

fn to_info(rel: &GhRelease, target: &str, can_self_install: bool) -> UpdateInfo {
    let version = rel.tag_name.trim_start_matches(['v', 'V']).to_string();
    let names = asset_names(&version, target);
    let asset = rel.assets.iter().find(|a| names.contains(&a.name));
    let sha = asset.and_then(|a| {
        let want = format!("{}.sha256", a.name);
        rel.assets.iter().find(|s| s.name == "SHA256SUMS.txt")
            .or_else(|| rel.assets.iter().find(|s| s.name == want))
    });
    UpdateInfo {
        version,
        notes: rel.body.clone().unwrap_or_default(),
        html_url: rel.html_url.clone(),
        asset_url: asset.map(|a| a.browser_download_url.clone()),
        sha256_url: sha.map(|a| a.browser_download_url.clone()),
        can_self_install: can_self_install && asset.is_some() && sha.is_some(),
    }
}

/// Pure decision logic: picks the release for the channel and compares versions.
fn evaluate(releases: Vec<GhRelease>, channel: &str, current: &str, target: &str, managed: bool) -> UpdateStatus {
    let prerelease_channel = channel.eq_ignore_ascii_case("prerelease");
    let best = releases
        .into_iter()
        .filter(|r| !r.draft && (prerelease_channel || !r.prerelease))
        .filter(|r| parse_version(&r.tag_name).is_some())
        .max_by(|a, b| parse_version(&a.tag_name).cmp(&parse_version(&b.tag_name)));
    match best {
        Some(rel) if is_newer(current, &rel.tag_name) => UpdateStatus::Available(to_info(&rel, target, !managed)),
        _ => UpdateStatus::UpToDate { current: current.to_string() },
    }
}

fn last_check_path() -> PathBuf {
    paths::data_dir().join("update_check")
}

fn checked_recently() -> bool {
    std::fs::read_to_string(last_check_path())
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|t| paths::now_unix().saturating_sub(t) < CHECK_INTERVAL_SECS)
        .unwrap_or(false)
}

fn record_check() {
    let _ = paths::write_atomic(&last_check_path(), paths::now_unix().to_string().as_bytes());
}

/// Checks GitHub. With `automatic = true` the 24 h throttle and
/// `check_on_start` apply (`Skipped`); manual checks always hit GitHub.
pub async fn check(http: &reqwest::Client, cfg: &UpdatesConfig, automatic: bool) -> Result<UpdateStatus> {
    if automatic && (!cfg.check_on_start || checked_recently()) {
        return Ok(UpdateStatus::Skipped);
    }
    let repo = cfg.repo.trim();
    if repo.is_empty() || repo.starts_with("OWNER/") || repo.split('/').count() != 2 {
        bail!("updates.repo is not configured (expected `owner/repo` in config.toml)");
    }
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=15");
    let resp = http
        .get(url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;
    record_check();
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        // No release published yet.
        return Ok(UpdateStatus::UpToDate { current: current_version().into() });
    }
    if !resp.status().is_success() {
        bail!("GitHub answered HTTP {}", resp.status());
    }
    let releases: Vec<GhRelease> = resp.json().await?;
    Ok(evaluate(releases, &cfg.channel, current_version(), current_target(), exe_is_package_managed()))
}

// ---------------------------------------------------------------- install

/// Parses a `.sha256` file: `<hex>` or `<hex>  <filename>`.
pub fn parse_sha256_file(text: &str) -> Option<String> {
    let h = text.split_whitespace().next()?.to_lowercase();
    (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())).then_some(h)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn exe_file_name() -> &'static str {
    if cfg!(windows) {
        "glyph.exe"
    } else {
        "glyph"
    }
}

/// Extracts the `glyph` binary from a `.zip` / `.tar.gz` into `dest`. Only
/// that single entry is written (no path traversal possible).
pub fn extract_binary(archive: &Path, dest: &Path) -> Result<()> {
    let want = exe_file_name();
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
        let mut best: Option<(usize, usize)> = None; // (depth, index)
        for i in 0..zip.len() {
            let f = zip.by_index(i)?;
            if f.is_dir() {
                continue;
            }
            let Ok(n) = f.name() else { continue };
            let path = n.replace('\\', "/");
            if path.rsplit('/').next() == Some(want) {
                let depth = path.matches('/').count();
                if best.map(|b| depth < b.0).unwrap_or(true) {
                    best = Some((depth, i));
                }
            }
        }
        let (_, idx) = best.ok_or_else(|| anyhow!("archive does not contain `{want}`"))?;
        let mut f = zip.by_index(idx)?;
        let mut out = std::fs::File::create(dest)?;
        std::io::copy(&mut f, &mut out)?;
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        // Two passes: find the shallowest match, then extract it.
        let open = || -> Result<tar::Archive<flate2::read::GzDecoder<std::fs::File>>> {
            Ok(tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(archive)?)))
        };
        let mut best: Option<(usize, PathBuf)> = None;
        for entry in open()?.entries()? {
            let entry = entry?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let p = entry.path()?.into_owned();
            if p.file_name().and_then(|n| n.to_str()) == Some(want) {
                let depth = p.components().count();
                if best.as_ref().map(|b| depth < b.0).unwrap_or(true) {
                    best = Some((depth, p));
                }
            }
        }
        let (_, wanted) = best.ok_or_else(|| anyhow!("archive does not contain `{want}`"))?;
        for entry in open()?.entries()? {
            let mut entry = entry?;
            if entry.path()?.as_ref() == wanted.as_path() {
                let mut out = std::fs::File::create(dest)?;
                std::io::copy(&mut entry, &mut out)?;
                break;
            }
        }
    } else {
        bail!("unsupported archive type: {name}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Downloads (with progress), verifies SHA-256, extracts and replaces the
/// running binary. Restart is up to the caller.
pub async fn install(
    http: &reqwest::Client,
    info: &UpdateInfo,
    progress: crate::flume_like::Sender<(u64, u64)>,
) -> Result<()> {
    if !info.can_self_install {
        bail!("this installation is managed by a package manager (or the release has no matching asset); update it there");
    }
    let asset_url = info.asset_url.as_deref().ok_or_else(|| anyhow!("no asset for {}", current_target()))?;
    let sha_url = info.sha256_url.as_deref().ok_or_else(|| anyhow!("release has no checksum manifest; refusing to install"))?;
    for u in [asset_url, sha_url] {
        if !u.starts_with("https://") {
            bail!("refusing non-HTTPS download URL");
        }
    }

    let checksum_text = http.get(sha_url).send().await?.error_for_status()?.text().await?;
    let filename = asset_url.rsplit('/').next().ok_or_else(|| anyhow!("invalid asset URL"))?;
    let expected = glyph_protocol::checksum_for_asset(&checksum_text, filename)
        .ok_or_else(|| anyhow!("checksum manifest has no valid, unique entry for {filename}"))?;

    let work = paths::data_dir().join("update").join(&info.version);
    let _ = tokio::fs::remove_dir_all(&work).await;
    tokio::fs::create_dir_all(&work).await?;
    let file_name = asset_url.rsplit('/').next().unwrap_or("update.bin");
    let archive = work.join(file_name);

    let result = async {
        let resp = http.get(asset_url).send().await?.error_for_status()?;
        let total = resp.content_length().unwrap_or(0);
        if total > MAX_DOWNLOAD_BYTES {
            bail!("update is unreasonably large");
        }
        let mut hasher = Sha256::new();
        let mut file = tokio::fs::File::create(&archive).await?;
        let mut stream = resp.bytes_stream();
        let mut done = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            done += chunk.len() as u64;
            if done > MAX_DOWNLOAD_BYTES {
                bail!("update is unreasonably large");
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            let _ = progress.send((done, total.max(done)));
        }
        file.flush().await?;
        drop(file);
        let actual = hex(&hasher.finalize());
        if actual != expected {
            bail!("SHA-256 mismatch (expected {expected}, got {actual}); update discarded");
        }
        if archive.extension().is_some_and(|e| e == "apk") {
            let install = APK_INSTALLER.get().ok_or_else(|| anyhow!("no package installer on this platform"))?;
            // The installer reads the file after this returns; it deletes it.
            if !install(&archive) {
                bail!("the package installer could not be opened");
            }
            return Ok(true);
        }
        let new_bin = work.join(format!("new-{}", exe_file_name()));
        let (a, n) = (archive.clone(), new_bin.clone());
        tokio::task::spawn_blocking(move || -> Result<()> {
            extract_binary(&a, &n)?;
            self_replace::self_replace(&n).context("replacing the running binary")?;
            Ok(())
        })
        .await??;
        Ok(false)
    }
    .await;
    if !matches!(result, Ok(true)) {
        let _ = tokio::fs::remove_dir_all(&work).await;
    }
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn semver_compare() {
        assert!(is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.1.0", "v0.2.0"));
        assert!(is_newer("v1.9.9", "v1.10.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.2.0", "1.1.9"));
        assert!(is_newer("1.0.0-beta.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(is_newer("1.0.0-beta.2", "1.0.0-beta.10"));
        assert!(!is_newer("1.0.0", "garbage"));
        assert!(!is_newer("garbage", "1.0.0"));
    }

    #[test]
    fn target_is_set() {
        assert!(current_target().contains('-'), "{}", current_target());
    }

    #[test]
    fn package_manager_detection() {
        for p in [
            "/usr/bin/glyph",
            "/opt/homebrew/Cellar/glyph/0.1.0/bin/glyph",
            "/usr/local/Cellar/glyph/0.1.0/bin/glyph",
            "/home/linuxbrew/.linuxbrew/bin/glyph",
            "C:\\Users\\me\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Glyph\\glyph.exe",
            "C:\\Users\\me\\scoop\\apps\\glyph\\current\\glyph.exe",
            "/nix/store/abc-glyph/bin/glyph",
            "/snap/glyph/current/bin/glyph",
        ] {
            assert!(is_package_managed_path(Path::new(p)), "{p}");
        }
        for p in [
            "/home/me/.local/bin/glyph",
            "/usr/local/bin/glyph",
            "/Applications/Glyph.app/Contents/MacOS/glyph",
            "C:\\Users\\me\\AppData\\Local\\Glyph\\glyph.exe",
        ] {
            assert!(!is_package_managed_path(Path::new(p)), "{p}");
        }
    }

    fn release(tag: &str, pre: bool, draft: bool, assets: &[&str]) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            body: Some("notes".into()),
            html_url: format!("https://github.com/o/glyph/releases/tag/{tag}"),
            draft,
            prerelease: pre,
            assets: assets
                .iter()
                .map(|n| GhAsset { name: n.to_string(), browser_download_url: format!("https://dl/{n}") })
                .collect(),
        }
    }

    #[test]
    fn android_picks_the_apk() {
        let rel = release("v0.6.0", false, false, &["glyph-0.6.0-aarch64-linux-android.tar.gz", "glyph-0.6.0-android.apk", "SHA256SUMS.txt"]);
        match evaluate(vec![rel], "stable", "0.5.0", "aarch64-linux-android", false) {
            UpdateStatus::Available(i) => {
                assert_eq!(i.asset_url.as_deref(), Some("https://dl/glyph-0.6.0-android.apk"));
                assert!(i.can_self_install);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn evaluates_channels_and_assets() {
        let t = "x86_64-unknown-linux-gnu";
        let rels = || {
            vec![
                release("v0.3.0-rc.1", true, false, &[]),
                release("v0.2.0", false, false, &["glyph-0.2.0-x86_64-unknown-linux-gnu.tar.gz", "glyph-0.2.0-x86_64-unknown-linux-gnu.tar.gz.sha256"]),
                release("v0.9.0", false, true, &[]), // draft: ignored
            ]
        };
        match evaluate(rels(), "stable", "0.1.0", t, false) {
            UpdateStatus::Available(i) => {
                assert_eq!(i.version, "0.2.0");
                assert!(i.can_self_install);
                assert_eq!(i.sha256_url.as_deref(), Some("https://dl/glyph-0.2.0-x86_64-unknown-linux-gnu.tar.gz.sha256"));
            }
            other => panic!("{other:?}"),
        }
        match evaluate(rels(), "prerelease", "0.1.0", t, false) {
            UpdateStatus::Available(i) => {
                assert_eq!(i.version, "0.3.0-rc.1");
                assert!(!i.can_self_install, "no assets");
            }
            other => panic!("{other:?}"),
        }
        match evaluate(rels(), "stable", "0.1.0", t, true) {
            UpdateStatus::Available(i) => assert!(!i.can_self_install, "package managed"),
            other => panic!("{other:?}"),
        }
        assert_eq!(evaluate(rels(), "stable", "0.2.0", t, false), UpdateStatus::UpToDate { current: "0.2.0".into() });
    }

    #[test]
    fn sha_file_parsing() {
        let h = "a".repeat(64);
        assert_eq!(parse_sha256_file(&format!("{h}  glyph.zip\n")), Some(h.clone()));
        assert_eq!(parse_sha256_file(&h.to_uppercase()), Some(h));
        assert_eq!(parse_sha256_file("short"), None);
    }

    #[test]
    fn shared_manifest_enables_updates_without_sidecars() {
        let target = "x86_64-unknown-linux-gnu";
        let rel = release("v0.2.2", false, false, &["glyph-0.2.2-x86_64-unknown-linux-gnu.tar.gz", "SHA256SUMS.txt"]);
        let info = to_info(&rel, target, true);
        assert!(info.can_self_install);
        assert_eq!(info.sha256_url.as_deref(), Some("https://dl/SHA256SUMS.txt"));
    }

    #[test]
    fn extracts_zip_and_targz() {
        let dir = tempfile::tempdir().unwrap();
        let exe = exe_file_name();
        // zip
        let zpath = dir.path().join("a.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&zpath).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("other.txt", opts).unwrap();
            z.write_all(b"no").unwrap();
            z.start_file(format!("glyph-0.2.0/{exe}"), opts).unwrap();
            z.write_all(b"BINARY").unwrap();
            z.finish().unwrap();
        }
        let out = dir.path().join("out1");
        extract_binary(&zpath, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"BINARY");

        // tar.gz
        let tpath = dir.path().join("a.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(std::fs::File::create(&tpath).unwrap(), flate2::Compression::fast());
            let mut tb = tar::Builder::new(gz);
            let data = b"TARBIN";
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            tb.append_data(&mut h, format!("glyph/{exe}"), &data[..]).unwrap();
            tb.into_inner().unwrap().finish().unwrap();
        }
        let out2 = dir.path().join("out2");
        extract_binary(&tpath, &out2).unwrap();
        assert_eq!(std::fs::read(&out2).unwrap(), b"TARBIN");

        assert!(extract_binary(&dir.path().join("x.rar"), &out2).is_err());
    }
}
