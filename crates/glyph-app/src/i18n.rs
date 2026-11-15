//! Texts Rust shows itself (tray, jump list, toasts with words, errors), in
//! the app's language, from the same `assets/i18n/*.toml` files as the UI.
//! Toasts the UI translates send a key instead (`Core::toast_key`).

use crate::controller::Core;

fn file(lang: &str) -> &'static str {
    match lang {
        "de" => include_str!("../../../assets/i18n/de.toml"),
        "ru" => include_str!("../../../assets/i18n/ru.toml"),
        "es" => include_str!("../../../assets/i18n/es.toml"),
        "fr" => include_str!("../../../assets/i18n/fr.toml"),
        "pt" => include_str!("../../../assets/i18n/pt.toml"),
        "it" => include_str!("../../../assets/i18n/it.toml"),
        "pl" => include_str!("../../../assets/i18n/pl.toml"),
        "tr" => include_str!("../../../assets/i18n/tr.toml"),
        "uk" => include_str!("../../../assets/i18n/uk.toml"),
        "af" => include_str!("../../../assets/i18n/af.toml"),
        _ => include_str!("../../../assets/i18n/en.toml"),
    }
}

fn find(text: &str, key: &str) -> Option<String> {
    let want = format!("\"{key}\"");
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == want).then(|| v.trim().trim_matches('"').replace("\\\"", "\"").replace("\\n", "\n"))
    })
}

/// The text for `key` in `lang` ("de", "en", ...), English when missing.
pub fn tr_in(lang: &str, key: &str) -> String {
    find(file(lang), key).or_else(|| find(file("en"), key)).unwrap_or_else(|| key.to_string())
}

/// The text for `key` in the app's language.
pub fn tr(core: &Core, key: &str) -> String {
    let lang = core.st().config.resolved_language();
    tr_in(lang, key)
}

/// `tr` with `{name}` placeholders filled in.
pub fn trf(core: &Core, key: &str, vars: &[(&str, &str)]) -> String {
    let mut s = tr(core, key);
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}
