//! Music links copied in another app: when the Glyph window gets the focus
//! and the clipboard holds one song or playlist link, the UI offers to add
//! it (`clipboard_link` event: a toast with "Add"). Each copied link is
//! offered once; links Glyph copied itself never are.
//!
//! Off with Settings -> System -> "Offer copied music links"
//! (`desktop.clipboard_links`).

use glyph_app::controller::Core;
use std::sync::{Arc, Mutex};
use tauri_plugin_clipboard_manager::ClipboardExt;

/// The clipboard text last looked at (offered or written by Glyph).
static SEEN: Mutex<String> = Mutex::new(String::new());

/// Glyph copied `text` itself ("Copy link"): never offer it back.
pub fn seen(text: &str) {
    if let Ok(mut s) = SEEN.lock() {
        *s = text.trim().to_string();
    }
}

/// The main window got the focus.
pub fn on_focus(app: &tauri::AppHandle, core: &Arc<Core>) {
    if !core.st().config.desktop.clipboard_links {
        return;
    }
    let Ok(text) = app.clipboard().read_text() else { return };
    let text = text.trim();
    {
        let Ok(mut s) = SEEN.lock() else { return };
        if *s == text {
            return;
        }
        *s = text.to_string();
    }
    if let Some(link) = glyph_app::links::music_link(text) {
        core.emit("clipboard_link", link);
    }
}
