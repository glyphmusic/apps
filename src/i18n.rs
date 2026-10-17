//! UI strings in English (default), German and Russian.
//!
//! Strings live in `assets/i18n/{en,de,ru}.toml` (flat `key = "text"`,
//! placeholders as `{name}`), embedded at compile time. Missing keys fall back
//! to English, then to the key itself.

use gpui::SharedString;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    De,
    Ru,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::En, Lang::De, Lang::Ru];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
            Lang::Ru => "ru",
        }
    }

    /// Name of the language in itself (for the switcher).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::De => "Deutsch",
            Lang::Ru => "Русский",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        let c = code.trim().to_ascii_lowercase();
        let c = c.split(['-', '_', '.']).next().unwrap_or_default();
        match c {
            "en" => Some(Lang::En),
            "de" => Some(Lang::De),
            "ru" => Some(Lang::Ru),
            _ => None,
        }
    }

    /// System language if en/de/ru, else English.
    pub fn system() -> Lang {
        sys_locale::get_locale()
            .and_then(|l| Lang::from_code(&l))
            .unwrap_or(Lang::En)
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);
static TABLES: OnceLock<[HashMap<String, String>; 3]> = OnceLock::new();

fn parse(src: &str, name: &str) -> HashMap<String, String> {
    match toml::from_str::<toml::Table>(src) {
        Ok(t) => t
            .into_iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
            .collect(),
        Err(e) => {
            tracing::error!("i18n {name}: {e}");
            HashMap::new()
        }
    }
}

fn tables() -> &'static [HashMap<String, String>; 3] {
    TABLES.get_or_init(|| {
        [
            parse(include_str!("../assets/i18n/en.toml"), "en"),
            parse(include_str!("../assets/i18n/de.toml"), "de"),
            parse(include_str!("../assets/i18n/ru.toml"), "ru"),
        ]
    })
}

pub fn set_lang(l: Lang) {
    CURRENT.store(l as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::De,
        2 => Lang::Ru,
        _ => Lang::En,
    }
}

fn lookup(key: &str) -> String {
    let t = tables();
    t[lang() as usize]
        .get(key)
        .or_else(|| t[0].get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// Translated string.
pub fn t(key: &str) -> SharedString {
    lookup(key).into()
}

/// Translated string with `{name}` placeholders.
pub fn tf(key: &str, args: &[(&str, &dyn std::fmt::Display)]) -> SharedString {
    let mut s = lookup(key);
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), &v.to_string());
    }
    s.into()
}

#[cfg(test)]
mod tests {
    /// A single bad line makes a whole language fall back to raw keys, so
    /// every file must parse and carry the same keys as English.
    #[test]
    fn language_files_parse_and_match_english() {
        let load = |src: &str| src.parse::<toml::Table>().expect("valid TOML");
        let en = load(include_str!("../assets/i18n/en.toml"));
        for (name, src) in [("de", include_str!("../assets/i18n/de.toml")), ("ru", include_str!("../assets/i18n/ru.toml"))] {
            let other = load(src);
            let missing: Vec<_> = en.keys().filter(|k| !other.contains_key(*k)).collect();
            assert!(missing.is_empty(), "{name} lacks {missing:?}");
        }
    }
}
