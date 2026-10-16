//! Bridges glyph-core's `Config.language` to the UI's `Lang`.

use crate::i18n::Lang;
use glyph_core::Config;

/// Configured language, else the system language (en/de/ru), else English.
pub fn language(config: &Config) -> Lang {
    Lang::from_code(config.resolved_language()).unwrap_or(Lang::En)
}

pub fn set_language(config: &mut Config, lang: Lang) {
    config.language = lang.code().to_string();
}
