//! Registers Glyph with the desktop so it shows up as "Glyph" (with its icon)
//! in the Start menu / Windows search, the Linux app launcher and macOS
//! Launchpad / Spotlight, instead of as a bare `glyph.exe`.
//!
//! The releases are plain archives without an installer, so the app does it
//! itself on start, once per location of the binary:
//! - Windows: a `Glyph.lnk` shortcut in the user's Start menu programs folder.
//! - Linux: `glyph.desktop` plus icons under `~/.local/share`.
//! - macOS: a small `~/Applications/Glyph.app` bundle around the binary.
//!
//! Skipped for debug builds and for installs a package manager owns (those
//! bring their own entry).

use std::path::{Path, PathBuf};

/// Runs in the background; failures are only logged.
pub fn register() {
    if cfg!(debug_assertions) {
        return;
    }
    let _ = std::thread::Builder::new().name("glyph-integrate".into()).spawn(|| {
        let Some(exe) = std::env::current_exe().ok().map(|p| std::fs::canonicalize(&p).unwrap_or(p)) else { return };
        if glyph_core::updater::is_package_managed_path(&exe) {
            return;
        }
        if let Err(e) = platform::register(&exe) {
            tracing::warn!("desktop integration: {e:#}");
        }
    });
}

/// What was registered last (binary path + version), so a start where
/// nothing changed does not touch the system.
fn marker() -> PathBuf {
    glyph_core::paths::data_dir().join("integration.txt")
}

fn stamp(exe: &Path) -> String {
    format!("{}\n{}", exe.display(), env!("CARGO_PKG_VERSION"))
}

fn up_to_date(exe: &Path, entry: &Path) -> bool {
    entry.exists() && std::fs::read_to_string(marker()).is_ok_and(|s| s == stamp(exe))
}

fn remember(exe: &Path) {
    let _ = glyph_core::paths::write_atomic(&marker(), stamp(exe).as_bytes());
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::core::{Interface, HSTRING};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    pub fn register(exe: &Path) -> anyhow::Result<()> {
        let appdata = std::env::var_os("APPDATA").ok_or_else(|| anyhow::anyhow!("APPDATA not set"))?;
        let dir = PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs");
        let lnk = dir.join("Glyph.lnk");
        if up_to_date(exe, &lnk) {
            return Ok(());
        }
        std::fs::create_dir_all(&dir)?;
        // The shortcut's file name is the name Windows search and the Start
        // menu show.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(exe.as_os_str()))?;
            if let Some(parent) = exe.parent() {
                link.SetWorkingDirectory(&HSTRING::from(parent.as_os_str()))?;
            }
            link.SetDescription(&HSTRING::from("Glyph"))?;
            link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
            let file: IPersistFile = link.cast()?;
            file.Save(&HSTRING::from(lnk.as_os_str()), true)?;
        }
        remember(exe);
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod platform {
    use super::*;

    const ICONS: &[(u32, &[u8])] = &[
        (16, include_bytes!("../assets/icons/icon-16.png")),
        (32, include_bytes!("../assets/icons/icon-32.png")),
        (48, include_bytes!("../assets/icons/icon-48.png")),
        (64, include_bytes!("../assets/icons/icon-64.png")),
        (128, include_bytes!("../assets/icons/icon-128.png")),
        (256, include_bytes!("../assets/icons/icon-256.png")),
        (512, include_bytes!("../assets/icons/icon-512.png")),
    ];

    fn data_home() -> Option<PathBuf> {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    }

    /// Desktop Entry `Exec` quoting: the path in double quotes, with `"`,
    /// `` ` ``, `$` and `\` escaped.
    fn quote(path: &str) -> String {
        let mut s = String::from("\"");
        for c in path.chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                s.push('\\');
            }
            s.push(c);
        }
        s.push('"');
        s
    }

    pub fn register(exe: &Path) -> anyhow::Result<()> {
        // Sandboxed or system-wide installs have their own entry.
        if std::env::var_os("FLATPAK_ID").is_some() || std::env::var_os("SNAP").is_some() {
            return Ok(());
        }
        if Path::new("/usr/share/applications/glyph.desktop").exists() {
            return Ok(());
        }
        // An AppImage runs from a temporary mount; the image file is what stays.
        let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).unwrap_or_else(|| exe.to_path_buf());
        let share = data_home().ok_or_else(|| anyhow::anyhow!("no HOME"))?;
        // Named after the Wayland app id / X11 class "glyph", so the running
        // window is matched to this entry (name and icon in docks).
        let entry = share.join("applications/glyph.desktop");
        if up_to_date(&exe, &entry) {
            return Ok(());
        }
        for (size, bytes) in ICONS {
            let dir = share.join(format!("icons/hicolor/{size}x{size}/apps"));
            std::fs::create_dir_all(&dir)?;
            glyph_core::paths::write_atomic(&dir.join("glyph.png"), bytes)?;
        }
        let desktop = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Glyph\n\
             GenericName=Music Player\n\
             Comment=Search everywhere, play from free sources\n\
             Exec={}\n\
             Icon=glyph\n\
             Terminal=false\n\
             Categories=AudioVideo;Audio;Player;\n\
             Keywords=music;player;audio;songs;playlist;\n\
             StartupWMClass=glyph\n",
            quote(&exe.to_string_lossy())
        );
        std::fs::create_dir_all(entry.parent().expect("has parent"))?;
        glyph_core::paths::write_atomic(&entry, desktop.as_bytes())?;
        // Lets launchers pick up the new icons right away (best effort).
        let _ = std::process::Command::new("gtk-update-icon-cache")
            .args(["-f", "-t"])
            .arg(share.join("icons/hicolor"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        remember(&exe);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    const ICNS: &[u8] = include_bytes!("../assets/icons/glyph.icns");

    pub fn register(exe: &Path) -> anyhow::Result<()> {
        // Already started from an app bundle: nothing to do.
        if exe.ancestors().any(|a| a.extension().is_some_and(|e| e == "app")) {
            return Ok(());
        }
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("no HOME"))?;
        let app = PathBuf::from(home).join("Applications/Glyph.app");
        let bin = app.join("Contents/MacOS/glyph");
        if up_to_date(exe, &bin) {
            return Ok(());
        }
        let contents = app.join("Contents");
        std::fs::create_dir_all(contents.join("MacOS"))?;
        std::fs::create_dir_all(contents.join("Resources"))?;
        let version = env!("CARGO_PKG_VERSION");
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Glyph</string>
  <key>CFBundleDisplayName</key><string>Glyph</string>
  <key>CFBundleIdentifier</key><string>com.glyphmusic.glyph</string>
  <key>CFBundleExecutable</key><string>glyph</string>
  <key>CFBundleIconFile</key><string>glyph.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#
        );
        glyph_core::paths::write_atomic(&contents.join("Info.plist"), plist.as_bytes())?;
        glyph_core::paths::write_atomic(&contents.join("Resources/glyph.icns"), ICNS)?;
        // The bundle holds its own copy of the binary (a hard link when on
        // the same disk), so the Dock shows it with name and icon.
        let tmp = bin.with_extension("new");
        let _ = std::fs::remove_file(&tmp);
        if std::fs::hard_link(exe, &tmp).is_err() {
            std::fs::copy(exe, &tmp)?;
        }
        std::fs::rename(&tmp, &bin)?;
        // The binary already ran from where it was downloaded; the bundle
        // around it should not be blocked as a new unknown download.
        let _ = std::process::Command::new("xattr").args(["-dr", "com.apple.quarantine"]).arg(&app).status();
        // Tell Launchpad / Spotlight about it (best effort).
        let _ = std::process::Command::new(
            "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
        )
        .args(["-f"])
        .arg(&app)
        .status();
        remember(exe);
        Ok(())
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "freebsd", target_os = "macos")))]
mod platform {
    use super::*;

    pub fn register(_exe: &Path) -> anyhow::Result<()> {
        Ok(())
    }
}
