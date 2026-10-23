//! Starting Glyph with the computer (Settings -> System, desktop only).
//!
//! The entry starts `glyph --autostart`; whether such a start opens the
//! window or stays in the tray is read from the config at launch, so
//! switching that needs no new entry.
//! - Windows: a `Glyph` value under `HKCU\...\CurrentVersion\Run`.
//! - Linux: `~/.config/autostart/glyph.desktop`.
//! - macOS: a LaunchAgent `~/Library/LaunchAgents/com.glyphmusic.glyph.plist`.

use std::path::{Path, PathBuf};

/// Command-line flag of a start by the system.
pub const FLAG: &str = "--autostart";

/// This process was started by the autostart entry.
pub fn launched_by_system() -> bool {
    std::env::args().skip(1).any(|a| a == FLAG)
}

/// Whether the platform can start Glyph with the computer.
pub const SUPPORTED: bool = cfg!(any(windows, target_os = "linux", target_os = "freebsd", target_os = "macos"));

/// Adds or removes the entry for the running binary.
pub fn set(enabled: bool) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    if enabled {
        platform::add(&exe)
    } else {
        platform::remove()
    }
}

/// Points an existing entry at the current binary (it may have moved or been
/// updated since). Runs in the background; failures are only logged.
pub fn refresh(enabled: bool) {
    if !enabled {
        return;
    }
    let _ = std::thread::Builder::new().name("glyph-autostart".into()).spawn(|| {
        if let Err(e) = set(true) {
            tracing::warn!("autostart: {e:#}");
        }
    });
}

#[allow(dead_code)]
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from)
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::process::CommandExt;

    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    fn reg(args: &[&str]) -> anyhow::Result<bool> {
        let out = std::process::Command::new("reg.exe").args(args).creation_flags(CREATE_NO_WINDOW).output()?;
        Ok(out.status.success())
    }

    pub fn add(exe: &Path) -> anyhow::Result<()> {
        // Ordinary Win32 path, not canonicalize's extended prefix.
        let exe = exe.to_string_lossy().trim_start_matches(r"\\?\").to_string();
        let data = format!(r#""{exe}" {FLAG}"#);
        // Values are passed as arguments, never as shell code.
        anyhow::ensure!(reg(&["add", KEY, "/v", "Glyph", "/t", "REG_SZ", "/d", &data, "/f"])?, "could not add the autostart entry");
        Ok(())
    }

    pub fn remove() -> anyhow::Result<()> {
        // Fails when there is no entry, which is what we want anyway.
        let _ = reg(&["delete", KEY, "/v", "Glyph", "/f"])?;
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod platform {
    use super::*;

    fn entry() -> anyhow::Result<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".config")))
            .ok_or_else(|| anyhow::anyhow!("no HOME"))?;
        Ok(config.join("autostart/glyph.desktop"))
    }

    /// Desktop Entry `Exec` quoting (as in `integrate.rs`).
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

    pub fn add(exe: &Path) -> anyhow::Result<()> {
        // An AppImage runs from a temporary mount; the image file is what stays.
        let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).unwrap_or_else(|| exe.to_path_buf());
        let exec = format!("{} {FLAG}", quote(&exe.to_string_lossy()));
        let desktop = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Glyph\n\
             Exec={exec}\n\
             Icon=glyph\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n"
        );
        let path = entry()?;
        if std::fs::read_to_string(&path).is_ok_and(|s| s == desktop) {
            return Ok(());
        }
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        glyph_core::paths::write_atomic(&path, desktop.as_bytes())?;
        Ok(())
    }

    pub fn remove() -> anyhow::Result<()> {
        match std::fs::remove_file(entry()?) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    const LABEL: &str = "com.glyphmusic.glyph";

    fn agent() -> anyhow::Result<PathBuf> {
        Ok(home().ok_or_else(|| anyhow::anyhow!("no HOME"))?.join(format!("Library/LaunchAgents/{LABEL}.plist")))
    }

    fn xml(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn add(exe: &Path) -> anyhow::Result<()> {
        // Through the app bundle when there is one (the bundle `integrate.rs`
        // writes, or the one Glyph runs from), so it starts with name and icon.
        // `-g`: not in front; Glyph brings its window up itself unless it
        // starts in the background.
        let bundle = exe
            .ancestors()
            .find(|a| a.extension().is_some_and(|e| e == "app"))
            .map(Path::to_path_buf)
            .or_else(|| home().map(|h| h.join("Applications/Glyph.app")).filter(|b| b.exists()));
        let args: Vec<String> = match bundle {
            Some(b) => vec!["/usr/bin/open".into(), "-g".into(), "-a".into(), b.to_string_lossy().into_owned(), "--args".into(), FLAG.into()],
            None => vec![exe.to_string_lossy().into_owned(), FLAG.into()],
        };
        let args: String = args.iter().map(|a| format!("    <string>{}</string>\n", xml(a))).collect();
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{args}  </array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#
        );
        let path = agent()?;
        if std::fs::read_to_string(&path).is_ok_and(|s| s == plist) {
            return Ok(());
        }
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        glyph_core::paths::write_atomic(&path, plist.as_bytes())?;
        Ok(())
    }

    pub fn remove() -> anyhow::Result<()> {
        match std::fs::remove_file(agent()?) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "freebsd", target_os = "macos")))]
mod platform {
    use super::*;

    pub fn add(_exe: &Path) -> anyhow::Result<()> {
        anyhow::bail!("not supported on this platform")
    }

    pub fn remove() -> anyhow::Result<()> {
        Ok(())
    }
}
