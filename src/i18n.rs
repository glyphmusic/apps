//! UI strings in English (default), German, Russian, Spanish, French,
//! Portuguese, Italian, Polish, Turkish, Ukrainian and Afrikaans.
//!
//! Strings live in `assets/i18n/<code>.toml` (flat `key = "text"`,
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
    Es,
    Fr,
    Pt,
    It,
    Pl,
    Tr,
    Uk,
    Af,
}

impl Lang {
    pub const ALL: [Lang; 11] = [Lang::En, Lang::Af, Lang::De, Lang::Es, Lang::Fr, Lang::It, Lang::Pl, Lang::Pt, Lang::Ru, Lang::Tr, Lang::Uk];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
            Lang::Ru => "ru",
            Lang::Es => "es",
            Lang::Fr => "fr",
            Lang::Pt => "pt",
            Lang::It => "it",
            Lang::Pl => "pl",
            Lang::Tr => "tr",
            Lang::Uk => "uk",
            Lang::Af => "af",
        }
    }

    /// Name of the language in itself (for the switcher).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::De => "Deutsch",
            Lang::Ru => "Русский",
            Lang::Es => "Español",
            Lang::Fr => "Français",
            Lang::Pt => "Português",
            Lang::It => "Italiano",
            Lang::Pl => "Polski",
            Lang::Tr => "Türkçe",
            Lang::Uk => "Українська",
            Lang::Af => "Afrikaans",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        let c = code.trim().to_ascii_lowercase();
        let c = c.split(['-', '_', '.']).next().unwrap_or_default();
        Lang::ALL.into_iter().find(|l| l.code() == c)
    }

    /// System language if supported, else English.
    pub fn system() -> Lang {
        sys_locale::get_locale()
            .and_then(|l| Lang::from_code(&l))
            .unwrap_or(Lang::En)
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);
static TABLES: [OnceLock<HashMap<String, String>>; 11] = [const { OnceLock::new() }; 11];

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

fn table(lang: Lang) -> &'static HashMap<String, String> {
    TABLES[lang as usize].get_or_init(|| {
        match lang {
            Lang::En => parse(include_str!("../assets/i18n/en.toml"), "en"),
            Lang::De => parse(include_str!("../assets/i18n/de.toml"), "de"),
            Lang::Ru => parse(include_str!("../assets/i18n/ru.toml"), "ru"),
            Lang::Es => parse(include_str!("../assets/i18n/es.toml"), "es"),
            Lang::Fr => parse(include_str!("../assets/i18n/fr.toml"), "fr"),
            Lang::Pt => parse(include_str!("../assets/i18n/pt.toml"), "pt"),
            Lang::It => parse(include_str!("../assets/i18n/it.toml"), "it"),
            Lang::Pl => parse(include_str!("../assets/i18n/pl.toml"), "pl"),
            Lang::Tr => parse(include_str!("../assets/i18n/tr.toml"), "tr"),
            Lang::Uk => parse(include_str!("../assets/i18n/uk.toml"), "uk"),
            Lang::Af => parse(include_str!("../assets/i18n/af.toml"), "af"),
        }
    })
}

pub fn set_lang(l: Lang) {
    CURRENT.store(l as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    let n = CURRENT.load(Ordering::Relaxed);
    Lang::ALL
        .into_iter()
        .find(|l| *l as u8 == n)
        .unwrap_or(Lang::En)
}

fn lookup(key: &str) -> String {
    table(lang())
        .get(key)
        .or_else(|| table(Lang::En).get(key))
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
        for (name, src) in [
            ("de", include_str!("../assets/i18n/de.toml")),
            ("ru", include_str!("../assets/i18n/ru.toml")),
            ("es", include_str!("../assets/i18n/es.toml")),
            ("fr", include_str!("../assets/i18n/fr.toml")),
            ("pt", include_str!("../assets/i18n/pt.toml")),
            ("it", include_str!("../assets/i18n/it.toml")),
            ("pl", include_str!("../assets/i18n/pl.toml")),
            ("tr", include_str!("../assets/i18n/tr.toml")),
            ("uk", include_str!("../assets/i18n/uk.toml")),
            ("af", include_str!("../assets/i18n/af.toml")),
        ] {
            let other = load(src);
            let missing: Vec<_> = en.keys().filter(|k| !other.contains_key(*k)).collect();
            assert!(missing.is_empty(), "{name} lacks {missing:?}");
        }
    }
}
