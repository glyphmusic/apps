//! Keyboard shortcuts: every action a key can start, its default key and
//! where it works. The keymap is built from this list plus the keys the
//! user changed (`config.ui.keys`, Settings › About › Shortcuts).

use super::*;
use crate::i18n::t;
use glyph_core::config::UiConfig;
use gpui::Keystroke;

/// Where a shortcut works.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Not while typing in a text field.
    Main,
    /// Anywhere in the main window.
    Window,
    /// Everywhere, also while typing.
    Global,
}

/// Groups on the shortcuts page.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Playback,
    Navigation,
    App,
}

/// One action a key can start.
pub struct Shortcut {
    /// Id under which a changed key is saved (`config.ui.keys`).
    pub id: &'static str,
    /// Label (i18n key).
    pub label: &'static str,
    /// Default key in GPUI syntax ("secondary" = Ctrl, or Cmd on macOS);
    /// "" = none.
    pub default: &'static str,
    pub scope: Scope,
    pub group: Group,
}

const fn s(id: &'static str, label: &'static str, default: &'static str, scope: Scope, group: Group) -> Shortcut {
    Shortcut { id, label, default, scope, group }
}

/// Every action a key can start, in the order of the shortcuts page.
pub const SHORTCUTS: &[Shortcut] = &[
    s("play", "sp.keys.play", "space", Scope::Main, Group::Playback),
    s("seek_back", "sp.keys.seek_back", "left", Scope::Main, Group::Playback),
    s("seek_forward", "sp.keys.seek_forward", "right", Scope::Main, Group::Playback),
    s("prev", "sp.keys.prev", "secondary-left", Scope::Window, Group::Playback),
    s("next", "sp.keys.next", "secondary-right", Scope::Window, Group::Playback),
    s("volume_up", "sp.keys.volume_up", "secondary-up", Scope::Window, Group::Playback),
    s("volume_down", "sp.keys.volume_down", "secondary-down", Scope::Window, Group::Playback),
    s("mute", "sp.keys.mute", "m", Scope::Main, Group::Playback),
    s("like", "sp.keys.like", "secondary-shift-l", Scope::Window, Group::Playback),
    s("shuffle", "sp.keys.shuffle", "secondary-s", Scope::Window, Group::Playback),
    s("repeat", "sp.keys.repeat", "secondary-r", Scope::Window, Group::Playback),
    s("download", "sp.keys.download", "secondary-d", Scope::Main, Group::Playback),
    s("loop_start", "sp.keys.loop_start", "alt-1", Scope::Global, Group::Playback),
    s("loop_end", "sp.keys.loop_end", "alt-2", Scope::Global, Group::Playback),
    s("loop_clear", "sp.keys.loop_clear", "alt-0", Scope::Global, Group::Playback),
    s("search", "sp.keys.search", "secondary-k", Scope::Global, Group::Navigation),
    s("lyrics", "sp.keys.lyrics", "secondary-l", Scope::Global, Group::Navigation),
    s("zen", "sp.keys.zen", "z", Scope::Main, Group::Navigation),
    s("queue", "sp.keys.queue", "secondary-j", Scope::Window, Group::Navigation),
    s("settings", "sp.keys.settings", "secondary-0", Scope::Global, Group::Navigation),
    s("quit", "sp.keys.quit", "secondary-q", Scope::Global, Group::App),
];

/// Keys that stay fixed (lists, sidebar, closing): a shortcut cannot take them.
pub const FIXED: &[&str] = &[
    "up", "down", "enter", "delete", "backspace", "escape", "tab", "shift-tab",
    "secondary-1", "secondary-2", "secondary-3", "secondary-4", "secondary-5", "secondary-6", "secondary-7", "secondary-8", "secondary-9",
];

/// The shortcut with id `id`.
pub fn shortcut(id: &str) -> Option<&'static Shortcut> {
    SHORTCUTS.iter().find(|s| s.id == id)
}

/// The key `id` has now: the user's choice, else its default ("" = none).
pub fn key_of(ui: &UiConfig, id: &str) -> String {
    match ui.keys.get(id) {
        Some(k) => k.clone(),
        None => shortcut(id).map(|s| s.default.to_string()).unwrap_or_default(),
    }
}

/// One single keystroke GPUI can read (sequences are not offered).
pub fn valid(key: &str) -> bool {
    !key.is_empty() && !key.contains(char::is_whitespace) && Keystroke::parse(key).is_ok()
}

/// The same key, however it is written ("secondary-d" and "ctrl-d" on
/// Windows).
pub fn same_key(a: &str, b: &str) -> bool {
    match (Keystroke::parse(a), Keystroke::parse(b)) {
        (Ok(x), Ok(y)) => x.modifiers == y.modifiers && x.key == y.key,
        _ => false,
    }
}

/// A key without Ctrl / Alt / Cmd (a letter, Space, an arrow): it must not
/// fire while typing, whatever scope its action has by default.
fn bare(key: &str) -> bool {
    Keystroke::parse(key).is_ok_and(|k| {
        let m = k.modifiers;
        !(m.control || m.alt || m.platform || m.function) && !(k.key.starts_with('f') && k.key.len() > 1 && k.key[1..].chars().all(|c| c.is_ascii_digit()))
    })
}

fn binding(id: &str, key: &str, scope: Option<&str>) -> Option<KeyBinding> {
    Some(match id {
        "play" => KeyBinding::new(key, TogglePlay, scope),
        "seek_back" => KeyBinding::new(key, SeekBack, scope),
        "seek_forward" => KeyBinding::new(key, SeekForward, scope),
        "prev" => KeyBinding::new(key, PrevTrack, scope),
        "next" => KeyBinding::new(key, NextTrack, scope),
        "volume_up" => KeyBinding::new(key, VolumeUp, scope),
        "volume_down" => KeyBinding::new(key, VolumeDown, scope),
        "mute" => KeyBinding::new(key, ToggleMute, scope),
        "like" => KeyBinding::new(key, LikeCurrent, scope),
        "shuffle" => KeyBinding::new(key, ToggleShuffle, scope),
        "repeat" => KeyBinding::new(key, CycleRepeat, scope),
        "download" => KeyBinding::new(key, DownloadCurrent, scope),
        "loop_start" => KeyBinding::new(key, LoopStart, scope),
        "loop_end" => KeyBinding::new(key, LoopEnd, scope),
        "loop_clear" => KeyBinding::new(key, LoopClear, scope),
        "search" => KeyBinding::new(key, FocusSearch, scope),
        "lyrics" => KeyBinding::new(key, ToggleLyrics, scope),
        "zen" => KeyBinding::new(key, ToggleZen, scope),
        "queue" => KeyBinding::new(key, ToggleQueue, scope),
        "settings" => KeyBinding::new(key, Nav0, scope),
        "quit" => KeyBinding::new(key, Quit, scope),
        _ => return None,
    })
}

/// Key bindings of every shortcut as `ui` has them.
pub fn bindings(ui: &UiConfig) -> Vec<KeyBinding> {
    SHORTCUTS
        .iter()
        .filter_map(|s| {
            let key = key_of(ui, s.id);
            if !valid(&key) {
                return None;
            }
            let scope = match s.scope {
                _ if bare(&key) => Some(MAIN),
                Scope::Main => Some(MAIN),
                Scope::Window => Some("Glyph"),
                Scope::Global => None,
            };
            binding(s.id, &key, scope)
        })
        .collect()
}

/// How a key reads ("Ctrl+Shift+Z", "F11", "Space", "←").
pub fn key_label(key: &str) -> String {
    let Ok(k) = Keystroke::parse(key) else { return String::new() };
    let mac = cfg!(target_os = "macos");
    let mut parts: Vec<String> = Vec::new();
    let m = k.modifiers;
    if m.control {
        parts.push(if mac { "⌃".into() } else { "Ctrl".into() });
    }
    if m.alt {
        parts.push(if mac { "⌥".into() } else { "Alt".into() });
    }
    if m.platform {
        parts.push(if mac { "⌘".into() } else { "Win".into() });
    }
    if m.shift {
        parts.push(if mac { "⇧".into() } else { "Shift".into() });
    }
    if m.function {
        parts.push("Fn".into());
    }
    parts.push(match k.key.as_str() {
        "left" => "←".into(),
        "right" => "→".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "space" => t("sp.keys.space").to_string(),
        "enter" => "Enter".into(),
        "escape" => "Esc".into(),
        "backspace" => "⌫".into(),
        "delete" => t("sp.keys.del").to_string(),
        "tab" => "Tab".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PgUp".into(),
        "pagedown" => "PgDn".into(),
        other => other.to_uppercase(),
    });
    parts.join("+")
}

/// The key a shortcut has now, as it reads ("" = none).
pub fn label_of(ui: &UiConfig, id: &str) -> String {
    let key = key_of(ui, id);
    if valid(&key) { key_label(&key) } else { String::new() }
}

/// A key press as GPUI writes keys ("ctrl-shift-z"); `None` for modifier
/// keys on their own.
pub fn from_keystroke(k: &Keystroke) -> Option<String> {
    if matches!(k.key.as_str(), "shift" | "control" | "alt" | "platform" | "function" | "") {
        return None;
    }
    let key = k.unparse();
    valid(&key).then_some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_unique() {
        let ui = UiConfig::default();
        for s in SHORTCUTS {
            assert!(s.default.is_empty() || valid(s.default), "{}", s.id);
            assert!(binding(s.id, "f12", None).is_some(), "{} has no action", s.id);
        }
        for (i, a) in SHORTCUTS.iter().enumerate() {
            for b in &SHORTCUTS[i + 1..] {
                assert!(!same_key(a.default, b.default), "{} and {} share a key", a.id, b.id);
            }
            assert!(!FIXED.iter().any(|f| same_key(f, a.default)), "{} takes a fixed key", a.id);
        }
        assert_eq!(bindings(&ui).len(), SHORTCUTS.iter().filter(|s| !s.default.is_empty()).count());
    }

    #[test]
    fn changed_keys_win_and_empty_means_none() {
        let mut ui = UiConfig::default();
        ui.keys.insert("zen".into(), "f11".into());
        ui.keys.insert("download".into(), String::new());
        assert_eq!(key_of(&ui, "zen"), "f11");
        assert_eq!(label_of(&ui, "download"), "");
        assert!(same_key("ctrl-d", if cfg!(target_os = "macos") { "ctrl-d" } else { "secondary-d" }));
        assert!(!valid("ctrl-k ctrl-l"));
        assert!(bare("z") && bare("space") && !bare("ctrl-z") && !bare("f11"));
    }
}
