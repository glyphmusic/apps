//! Embedded assets served to GPUI (`svg().path("icons/ui/play.svg")`).
//! Icons: Lucide (ISC, see assets/icons/ui/LICENSE-lucide.txt).

use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[
            $((concat!("icons/ui/", $name, ".svg"), include_bytes!(concat!("../assets/icons/ui/", $name, ".svg")))),*
        ];
    };
}

icons!(
    "arrow-down", "arrow-up", "chevron-down", "chevron-left", "chevron-right", "copy", "disc-3",
    "download", "grip-vertical", "heart", "heart-fill", "layout-split", "layout-split-fill", "list-music-fill", "picture-in-picture-2-fill", "text-center", "house", "library", "link-2", "list-music", "list-plus", "list-start",
    "minimize-2", "minus", "music-2", "pause", "picture-in-picture-2", "play", "plus",
    "refresh-cw", "repeat-1", "repeat", "search", "settings", "shuffle", "skip-back",
    "skip-forward", "square", "timer", "trash-2", "volume-2", "volume-x", "x",
    "info", "check", "database", "server", "headphones", "book-open", "life-buoy", "github", "external-link", "palette", "folder-open", "circle-check",
    "ellipsis-vertical", "arrow-left", "history", "user-round", "circle-minus", "pencil", "eye", "eye-off",
    "arrow-down-to-line", "pin", "pin-fill", "circle-plus", "image-plus", "image", "maximize-2", "grip-horizontal",
    "panel-bottom", "rotate-ccw", "rotate-cw", "speaker", "circle", "audio-lines", "orbit", "arrow-right",
    "chart-column", "sparkles", "mic-vocal", "languages", "bot", "activity", "file-text", "globe", "radio", "cast",
    "monitor-speaker", "clock", "trophy", "calendar", "flame", "text-search", "badge-check", "ellipsis", "sliders-horizontal",
    "arrow-right-left", "airplay", "tv", "monitor", "users", "thumbs-up", "thumbs-up-fill",
    "arrow-up-down",
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if path == "icons/app/icon-256.png" {
            return Ok(Some(Cow::Borrowed(include_bytes!("../assets/icons/icon-256.png"))));
        }
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}
