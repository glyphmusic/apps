//! Parsing and handling incoming `glyph://` deep links and Android/OS share intents.

use crate::controller::Core;
use std::sync::Arc;

/// Marks text shared to Glyph (`MainActivity.takeLink`), not a URL.
pub const SHARED_PREFIX: &str = "glyph-shared:";

/// Normalized action derived from an incoming deep link or shared text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingAction {
    Jam(String),
    Plugin(String),
    Shared(String),
}

/// Maps a `glyph://` URL or `https://glyph.my/...` link to a request string
/// (`jam <code>`, `plugin <id>`, or `open <url>`).
pub fn request_for_url(url: &str) -> Option<String> {
    if url.starts_with(glyph_protocol::jam::APP_LINK_PREFIX) || glyph_protocol::jam::is_jam_text(url) {
        return glyph_protocol::jam::normalize_code(url).map(|code| format!("jam {code}"));
    }
    if let Some(id) = plugin_id(url) {
        return Some(format!("plugin {id}"));
    }
    glyph_core::share::link_id(url).map(|id| format!("open https://glyph.my/p/{id}"))
}

fn plugin_id(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("glyph://plugin/")
        .or_else(|| url.strip_prefix("https://glyph.my/plugins/"))?;
    let id = rest.split(['?', '#', '/']).next()?.to_ascii_lowercase();
    glyph_core::plugins::valid_id(&id).then_some(id)
}

/// Extracts a search/import query from OS share-sheet text (`SHARED_PREFIX` stripped).
pub fn shared_query(text: &str) -> Option<String> {
    let links: Vec<&str> = text
        .split_whitespace()
        .filter(|w| w.starts_with("https://") || w.starts_with("http://") || w.starts_with("spotify:"))
        .collect();
    let q = if links.is_empty() {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        links.join(" ")
    };
    let q: String = q.chars().take(2000).collect();
    (!q.is_empty()).then_some(q)
}

/// Converts a raw Android `gpuiTakeOpenUrl()` payload (either `glyph-shared:...`
/// or a deep link URL) into an [`IncomingAction`].
pub fn parse_incoming(raw: &str) -> Option<IncomingAction> {
    if let Some(text) = raw.strip_prefix(SHARED_PREFIX) {
        return shared_query(text).map(IncomingAction::Shared);
    }
    let req = request_for_url(raw)?;
    if let Some(code) = req.strip_prefix("jam ") {
        Some(IncomingAction::Jam(code.to_string()))
    } else if let Some(id) = req.strip_prefix("plugin ") {
        Some(IncomingAction::Plugin(id.to_string()))
    } else if let Some(text) = req.strip_prefix("open ").or_else(|| req.strip_prefix("shared ")) {
        Some(IncomingAction::Shared(text.to_string()))
    } else {
        None
    }
}

/// Applies an incoming link or share action to the running app controller.
pub fn apply(core: &Arc<Core>, raw: &str) -> bool {
    let Some(action) = parse_incoming(raw) else {
        return false;
    };
    match action {
        IncomingAction::Jam(code) => crate::jam::open_link(core, &code),
        IncomingAction::Plugin(id) => crate::plugins::open_plugin(core, &id),
        IncomingAction::Shared(text) => core.emit("shared", text),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_parse() {
        assert_eq!(request_for_url("glyph://jam/k7p2qx").as_deref(), Some("jam K7P2QX"));
        assert_eq!(request_for_url("https://glyph.my/jam/K7P2QX").as_deref(), Some("jam K7P2QX"));
        assert_eq!(request_for_url("glyph://jam/nope"), None);
        assert_eq!(request_for_url("glyph://p/AbC123xy").as_deref(), Some("open https://glyph.my/p/AbC123xy"));
        assert_eq!(request_for_url("https://example.com"), None);
        assert_eq!(request_for_url("glyph://plugin/sakura").as_deref(), Some("plugin sakura"));
        assert_eq!(request_for_url("https://glyph.my/plugins/sakura?ref=x").as_deref(), Some("plugin sakura"));
        assert_eq!(request_for_url("glyph://plugin/../x"), None);
        assert_eq!(
            parse_incoming("glyph-shared:listen https://open.spotify.com/track/123 now"),
            Some(IncomingAction::Shared("https://open.spotify.com/track/123".into()))
        );
    }
}
