use crate::controller::Core;
use serde_json::Value;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Platform-specific host capabilities implemented by `glyph-tauri` (desktop)
/// and `glyph-android` (Android WebView shell).
pub trait Host: Send + Sync + 'static {
    /// Emits `event` (full name, e.g. `"glyph://player"`) with JSON payload.
    fn emit(&self, event: &str, payload: Value);

    /// URL the webview uses to load a local file on disk.
    fn asset_url(&self, path: &Path) -> String;

    /// Schedules `f` to run on the platform's main / UI thread.
    fn run_on_main_thread(&self, f: Box<dyn FnOnce() + Send>);

    /// Platform per-tick hook (~100 ms), executed on the main thread.
    fn on_tick(&self, _core: &Arc<Core>) {}

    /// Shows and focuses the main application window when an assistant needs approval.
    fn focus_main_window(&self) {}

    /// Opens an `http(s)://` or `mailto:` URL in the external browser.
    fn open_url(&self, url: &str) -> Result<(), String>;

    /// Opens a directory in the system file manager.
    fn open_path(&self, path: &Path) -> Result<(), String> {
        let _ = path;
        Err("not supported on this platform".into())
    }

    /// Reveals a file or directory in the system file manager.
    fn reveal_in_dir(&self, path: &Path) -> Result<(), String> {
        let _ = path;
        Err("not supported on this platform".into())
    }

    /// Writes `text` to the system clipboard.
    fn write_clipboard(&self, text: &str) -> Result<(), String>;

    /// Shows a phone notification (`crate::notify`); nothing on desktop.
    fn notify(&self, notice: &crate::notify::Notice) {
        let _ = notice;
    }

    /// Whether [`Host::notify`] shows anything (phones).
    fn can_notify(&self) -> bool {
        false
    }

    /// Opens the system share sheet with `text` (Android); not on desktop.
    fn share_text(&self, text: &str, title: Option<&str>) -> Result<(), String> {
        let _ = (text, title);
        Err("not supported on this platform".into())
    }

    /// Opens a native file chooser. Returns `Ok(None)` when cancelled or unsupported.
    fn pick_file(
        &self,
        title: Option<&str>,
        filter_name: &str,
        extensions: &'static [&'static str],
    ) -> BoxFuture<'static, Result<Option<PathBuf>, String>> {
        let _ = (title, filter_name, extensions);
        Box::pin(async { Ok(None) })
    }

    /// Opens a native folder chooser. Returns `Ok(None)` when cancelled or unsupported.
    fn pick_folders(
        &self,
        prompt: Option<String>,
        multiple: bool,
    ) -> BoxFuture<'static, Result<Option<Vec<PathBuf>>, String>> {
        let _ = (prompt, multiple);
        Box::pin(async { Ok(None) })
    }

    /// Whether OS autostart is supported.
    fn autostart_supported(&self) -> bool {
        false
    }

    /// Enables or disables starting Glyph with the OS.
    fn set_autostart(&self, _on: bool) -> Result<(), String> {
        Err("autostart is desktop-only".into())
    }

    /// Restarts the application after an update.
    fn restart(&self) {}

    /// Whether a built-in Discord client ID is configured for Rich Presence.
    fn discord_builtin(&self) -> bool {
        false
    }

    /// Updates the platform's app/window/tray/launcher icons to match the
    /// configured theme accent (`0xrrggbb`), or to a plugin look's app icon
    /// (`icon`, checked .png bytes) while one is in use.
    fn set_theme_icon(&self, _theme_id: &str, _accent: u32, _icon: Option<&[u8]>) {}
}

static HOST: OnceLock<Arc<dyn Host>> = OnceLock::new();

pub fn set(host: Arc<dyn Host>) {
    let _ = HOST.set(host);
}

pub fn try_get() -> Option<&'static Arc<dyn Host>> {
    HOST.get()
}

pub fn get() -> &'static Arc<dyn Host> {
    HOST.get().expect("glyph-app host not initialized")
}

pub fn asset_url(path: &Path) -> String {
    HOST.get().map(|h| h.asset_url(path)).unwrap_or_default()
}
