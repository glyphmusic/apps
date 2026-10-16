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
    "download", "grip-vertical", "heart", "house", "library", "link-2", "list-music", "list-plus", "list-start",
    "minimize-2", "minus", "music-2", "pause", "picture-in-picture-2", "play", "plus",
    "refresh-cw", "repeat-1", "repeat", "search", "settings", "shuffle", "skip-back",
    "skip-forward", "square", "timer", "trash-2", "volume-2", "volume-x", "x",
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
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
