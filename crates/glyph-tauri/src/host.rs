use glyph_app::controller::Core;
use glyph_app::host::{BoxFuture, Host};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;

pub struct TauriHost {
    pub app: AppHandle,
}

impl TauriHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

/// URL the webview uses for a file on disk (Tauri's asset protocol), as
/// `convertFileSrc` in the UI makes it.
pub fn asset_url(path: &Path) -> String {
    let p = path.to_string_lossy();
    let mut enc = String::with_capacity(p.len() * 3);
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            enc.push(b as char);
        } else {
            enc.push_str(&format!("%{b:02X}"));
        }
    }
    if cfg!(any(target_os = "windows", target_os = "android")) {
        format!("http://asset.localhost/{enc}")
    } else {
        format!("asset://localhost/{enc}")
    }
}

impl Host for TauriHost {
    fn emit(&self, event: &str, payload: Value) {
        if let Err(e) = self.app.emit(event, payload) {
            tracing::warn!(event, error = %e, "emit failed");
        }
    }

    fn asset_url(&self, path: &Path) -> String {
        asset_url(path)
    }

    fn run_on_main_thread(&self, f: Box<dyn FnOnce() + Send>) {
        let _ = self.app.run_on_main_thread(f);
    }

    fn on_tick(&self, core: &Arc<Core>) {
        crate::desktop::tick(core);
    }

    fn focus_main_window(&self) {
        if let Some(w) = self.app.get_webview_window("main") {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
    }

    fn open_path(&self, path: &Path) -> Result<(), String> {
        tauri_plugin_opener::open_path(path.to_string_lossy().into_owned(), None::<&str>).map_err(|e| e.to_string())
    }

    fn reveal_in_dir(&self, path: &Path) -> Result<(), String> {
        tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| e.to_string())
    }

    fn write_clipboard(&self, text: &str) -> Result<(), String> {
        crate::clip::seen(text);
        self.app.clipboard().write_text(text.to_string()).map_err(|e| e.to_string())
    }

    fn pick_file(
        &self,
        title: Option<&str>,
        filter_name: &str,
        extensions: &'static [&'static str],
    ) -> BoxFuture<'static, Result<Option<PathBuf>, String>> {
        let app = self.app.clone();
        let title = title.map(str::to_string);
        let filter_name = filter_name.to_string();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let mut d = app.dialog().file().add_filter(&filter_name, extensions);
            if let Some(t) = title {
                d = d.set_title(t);
            }
            d.pick_file(move |f| {
                let _ = tx.send(f);
            });
            let Some(file) = rx.await.ok().flatten() else {
                return Ok(None);
            };
            file.into_path().map(Some).map_err(|e| e.to_string())
        })
    }

    fn pick_folders(
        &self,
        prompt: Option<String>,
        multiple: bool,
    ) -> BoxFuture<'static, Result<Option<Vec<PathBuf>>, String>> {
        let app = self.app.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut d = app.dialog().file();
                if let Some(p) = prompt {
                    d = d.set_title(p);
                }
                if multiple {
                    Ok(d.blocking_pick_folders()
                        .map(|v| v.into_iter().filter_map(|p| p.into_path().ok()).collect()))
                } else {
                    Ok(d.blocking_pick_folder()
                        .and_then(|p| p.into_path().ok())
                        .map(|p| vec![p]))
                }
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }

    fn autostart_supported(&self) -> bool {
        crate::autostart::SUPPORTED
    }

    fn set_autostart(&self, on: bool) -> Result<(), String> {
        crate::autostart::set(on).map_err(|e| format!("{e:#}"))
    }

    fn restart(&self) {
        self.app.restart();
    }

    fn discord_builtin(&self) -> bool {
        crate::discord::client_id(&glyph_core::Config::default()).is_some()
    }

    fn set_theme_icon(&self, _theme_id: &str, accent: u32, icon: Option<&[u8]>) {
        let icon = icon.map(<[u8]>::to_vec);
        let _ = self.app.run_on_main_thread(move || crate::desktop::set_theme_icon(accent, icon));
    }
}
