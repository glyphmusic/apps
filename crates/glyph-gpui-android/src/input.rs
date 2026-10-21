//! Android input to GPUI: raw touches (GPUI's portable recognizers turn them
//! into taps, scrolls and long presses), hardware keys, and the soft keyboard
//! through GameActivity's text input state.
//!
//! GameTextInput keeps its own copy of the edited text. We mirror the focused
//! GPUI input into it (`push_ime_state`) and apply its edits back as the
//! smallest replacement (`apply_text_event`). GameTextInput counts in UTF-8
//! bytes, GPUI's input handler in UTF-16 units.

use crate::window::AndroidWindowState;
use android_activity::input::{
    InputEvent, KeyAction, KeyEvent, KeyMapChar, Keycode, MetaState, MotionAction, MotionEvent,
    TextInputAction, TextInputState, TextSpan,
};
use gpui::{
    KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, PlatformInput, Point, TouchEvent, TouchId,
    TouchPhase, px,
};
use std::collections::HashMap;
use std::ops::Range;

/// Android reuses pointer ids for later touches; GPUI must never see a
/// reused `TouchId`.
#[derive(Default)]
pub(crate) struct TouchIds {
    live: HashMap<i32, u64>,
    next: u64,
}

impl TouchIds {
    fn start(&mut self, pointer: i32) -> TouchId {
        self.next += 1;
        self.live.insert(pointer, self.next);
        TouchId(self.next)
    }

    fn get(&mut self, pointer: i32) -> TouchId {
        match self.live.get(&pointer) {
            Some(id) => TouchId(*id),
            // A move without a down (e.g. after a surface change): new touch.
            None => self.start(pointer),
        }
    }

    fn end(&mut self, pointer: i32) -> TouchId {
        let id = self.get(pointer);
        self.live.remove(&pointer);
        id
    }
}

/// What GameTextInput was last told, so its echo of our own state is not
/// applied a second time.
#[derive(Default)]
pub(crate) struct ImeSync {
    /// True while an IME edit is being applied to the GPUI input handler
    /// (its change notifications must not be pushed back).
    pub applying: bool,
    last_text: Option<String>,
}

impl ImeSync {
    pub fn reset(&mut self) {
        self.applying = false;
        self.last_text = None;
    }
}

/// Handles one event; `true` when GPUI used it.
pub(crate) fn handle(state: &AndroidWindowState, event: &InputEvent) -> bool {
    match event {
        InputEvent::MotionEvent(m) => motion(state, m),
        InputEvent::KeyEvent(k) => key(state, k),
        InputEvent::TextEvent(t) => {
            apply_text_event(state, t.clone());
            true
        }
        InputEvent::TextAction(a) => text_action(state, *a),
        _ => false,
    }
}

fn motion(state: &AndroidWindowState, event: &MotionEvent) -> bool {
    let scale = state.scale();
    let position = |i: usize| {
        let p = event.pointer_at_index(i);
        Point::new(px(p.x() / scale), px(p.y() / scale))
    };
    let force = |i: usize| {
        let p = event.pointer_at_index(i).pressure();
        (p > 0.0).then(|| p.clamp(0.0, 1.0))
    };
    let send = |id: TouchId, phase: TouchPhase, i: usize| {
        state.dispatch(PlatformInput::Touch(TouchEvent {
            id,
            phase,
            position: position(i),
            predicted_position: None,
            force: force(i),
        }))
    };
    match event.action() {
        MotionAction::Down | MotionAction::PointerDown => {
            let i = event.pointer_index();
            let id = state.touches.borrow_mut().start(event.pointer_at_index(i).pointer_id());
            send(id, TouchPhase::Started, i)
        }
        MotionAction::Move => {
            let mut handled = false;
            for i in 0..event.pointer_count() {
                let id = state.touches.borrow_mut().get(event.pointer_at_index(i).pointer_id());
                handled |= send(id, TouchPhase::Moved, i);
            }
            handled
        }
        MotionAction::Up | MotionAction::PointerUp => {
            let i = event.pointer_index();
            let id = state.touches.borrow_mut().end(event.pointer_at_index(i).pointer_id());
            send(id, TouchPhase::Ended, i)
        }
        MotionAction::Cancel => {
            let mut handled = false;
            for i in 0..event.pointer_count() {
                let id = state.touches.borrow_mut().end(event.pointer_at_index(i).pointer_id());
                handled |= send(id, TouchPhase::Cancelled, i);
            }
            handled
        }
        _ => false,
    }
}

fn modifiers(meta: MetaState) -> Modifiers {
    Modifiers {
        control: meta.ctrl_on(),
        alt: meta.alt_on(),
        shift: meta.shift_on(),
        platform: meta.meta_on(),
        ..Default::default()
    }
}

/// GPUI key name for keys that do not type a character.
fn named_key(code: Keycode) -> Option<&'static str> {
    Some(match code {
        Keycode::Enter | Keycode::NumpadEnter => "enter",
        Keycode::Del => "backspace",
        Keycode::ForwardDel => "delete",
        Keycode::Escape => "escape",
        Keycode::Tab => "tab",
        Keycode::Space => "space",
        Keycode::DpadLeft => "left",
        Keycode::DpadRight => "right",
        Keycode::DpadUp => "up",
        Keycode::DpadDown => "down",
        Keycode::MoveHome => "home",
        Keycode::MoveEnd => "end",
        Keycode::PageUp => "pageup",
        Keycode::PageDown => "pagedown",
        _ => return None,
    })
}

fn key(state: &AndroidWindowState, event: &KeyEvent) -> bool {
    let code = event.key_code();
    // System back (button or gesture): GPUI's back handler when the app
    // wants it, otherwise the activity's default (see `MainActivity`).
    if code == Keycode::Back {
        // Delivered as `escape`: apps close overlays / go back with it. When
        // nothing takes it, the activity's default runs (see `MainActivity`).
        let keystroke = Keystroke { modifiers: Modifiers::default(), key: "escape".into(), key_char: None };
        return match event.action() {
            KeyAction::Down => {
                let handled = state.back_wanted()
                    || state.dispatch(PlatformInput::KeyDown(KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }));
                state.back_taken.set(handled);
                handled
            }
            KeyAction::Up => {
                state.dispatch(PlatformInput::KeyUp(KeyUpEvent { keystroke }));
                if state.back_wanted() {
                    state.back();
                }
                state.back_taken.replace(false)
            }
            _ => false,
        };
    }
    // Volume and media keys stay with the system.
    if matches!(code, Keycode::VolumeUp | Keycode::VolumeDown | Keycode::VolumeMute)
        || matches!(code, Keycode::MediaPlayPause | Keycode::MediaNext | Keycode::MediaPrevious)
    {
        return false;
    }
    let meta = event.meta_state();
    let mods = modifiers(meta);
    let keystroke = match named_key(code) {
        Some(name) => Keystroke {
            modifiers: mods,
            key: name.into(),
            key_char: (name == "space").then(|| " ".to_string()),
        },
        None => {
            let ch = state
                .app()
                .device_key_character_map(event.device_id())
                .ok()
                .and_then(|map| map.get(code, meta).ok())
                .and_then(|c| match c {
                    KeyMapChar::Unicode(c) => Some(c),
                    _ => None,
                });
            let Some(ch) = ch else { return false };
            let base = ch.to_lowercase().to_string();
            Keystroke { modifiers: mods, key: base, key_char: Some(ch.to_string()) }
        }
    };
    match event.action() {
        KeyAction::Down => state.dispatch(PlatformInput::KeyDown(KeyDownEvent {
            keystroke,
            is_held: event.repeat_count() > 0,
            prefer_character_input: false,
        })),
        KeyAction::Up => state.dispatch(PlatformInput::KeyUp(KeyUpEvent { keystroke })),
        _ => false,
    }
}

/// Enter/Search/Go on the soft keyboard: an `enter` key press.
fn text_action(state: &AndroidWindowState, _action: TextInputAction) -> bool {
    let keystroke = Keystroke { modifiers: Modifiers::default(), key: "enter".into(), key_char: None };
    let down = state.dispatch(PlatformInput::KeyDown(KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    }));
    state.dispatch(PlatformInput::KeyUp(KeyUpEvent { keystroke }));
    down
}

// ------------------------------------------------------------------ IME

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// UTF-8 byte offset -> UTF-16 offset (clamped, rounded down to a char).
fn byte_to_utf16(s: &str, byte: usize) -> usize {
    let byte = byte.min(s.len());
    s.char_indices().take_while(|(i, _)| *i < byte).map(|(_, c)| c.len_utf16()).sum()
}

/// UTF-16 offset -> UTF-8 byte offset (clamped).
fn utf16_to_byte(s: &str, utf16: usize) -> usize {
    let mut count = 0;
    for (i, c) in s.char_indices() {
        if count >= utf16 {
            return i;
        }
        count += c.len_utf16();
    }
    s.len()
}

fn floor_char(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while !s.is_char_boundary(b) {
        b -= 1;
    }
    b
}

fn span(s: &str, span: TextSpan) -> Range<usize> {
    let (a, b) = (floor_char(s, span.start), floor_char(s, span.end));
    a.min(b)..a.max(b)
}

/// The focused input's whole text, selection and marked range (UTF-16).
fn read_handler(state: &AndroidWindowState) -> Option<(String, Range<usize>, Option<Range<usize>>)> {
    state.with_input_handler(|h| {
        let mut actual = None;
        let text = h.text_for_range(0..usize::MAX / 2, &mut actual).unwrap_or_default();
        let selection = h.selected_text_range(false).map(|s| s.range).unwrap_or(0..0);
        (text, selection, h.marked_text_range())
    })
}

/// Tells GameTextInput what the focused GPUI input contains now.
pub(crate) fn push_ime_state(state: &AndroidWindowState) {
    let Some((text, selection, marked)) = read_handler(state) else { return };
    let to_byte = |r: Range<usize>| TextSpan { start: utf16_to_byte(&text, r.start), end: utf16_to_byte(&text, r.end) };
    let ime = TextInputState {
        selection: to_byte(selection),
        compose_region: marked.map(to_byte),
        text: text.clone(),
    };
    state.ime.borrow_mut().last_text = Some(text);
    state.app().set_text_input_state(ime);
}

/// Applies a GameTextInput edit to the focused GPUI input: the changed middle
/// of the text is replaced, a composing region becomes GPUI's marked text.
pub(crate) fn apply_text_event(state: &AndroidWindowState, new: TextInputState) {
    let Some((old, _, old_marked)) = read_handler(state) else { return };
    let compose = new.compose_region.map(|c| span(&new.text, c)).filter(|c| !c.is_empty());
    {
        let mut ime = state.ime.borrow_mut();
        if ime.last_text.as_deref() == Some(new.text.as_str()) && compose.is_none() && old_marked.is_none() && old == new.text {
            // Only our own state echoed back (or a pure selection move,
            // which GPUI's input handler cannot take from outside).
            return;
        }
        ime.applying = true;
        ime.last_text = Some(new.text.clone());
    }
    state.with_input_handler(|h| {
        if old_marked.is_some() {
            h.unmark_text();
        }
    });
    // Everything except the composing part goes in as committed text.
    let (committed, composing, compose_at) = match &compose {
        Some(c) => (
            format!("{}{}", &new.text[..c.start], &new.text[c.end..]),
            Some(new.text[c.clone()].to_string()),
            c.start,
        ),
        None => (new.text.clone(), None, 0),
    };
    let old_now = read_handler(state).map(|(t, _, _)| t).unwrap_or(old);
    if old_now != committed {
        let prefix = old_now.bytes().zip(committed.bytes()).take_while(|(a, b)| a == b).count();
        let prefix = floor_char(&old_now, floor_char(&committed, prefix));
        let max_suffix = (old_now.len() - prefix).min(committed.len() - prefix);
        let mut suffix = old_now.bytes().rev().zip(committed.bytes().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
        while !old_now.is_char_boundary(old_now.len() - suffix) || !committed.is_char_boundary(committed.len() - suffix) {
            suffix -= 1;
        }
        let range = byte_to_utf16(&old_now, prefix)..byte_to_utf16(&old_now, old_now.len() - suffix);
        let insert = committed[prefix..committed.len() - suffix].to_string();
        state.with_input_handler(|h| h.replace_text_in_range(Some(range), &insert));
    }
    if let Some(composing) = composing {
        let at = byte_to_utf16(&committed, compose_at);
        let caret = utf16_len(&composing);
        state.with_input_handler(|h| h.replace_and_mark_text_in_range(Some(at..at), &composing, Some(caret..caret)));
    }
    state.ime.borrow_mut().applying = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_byte_round_trip() {
        let s = "aä😀b";
        assert_eq!(utf16_len(s), 5);
        assert_eq!(byte_to_utf16(s, 3), 2);
        assert_eq!(byte_to_utf16(s, 7), 4);
        assert_eq!(utf16_to_byte(s, 4), 7);
        assert_eq!(utf16_to_byte(s, 99), s.len());
    }

    #[test]
    fn spans_snap_to_chars() {
        let s = "ä😀";
        assert_eq!(span(s, TextSpan { start: 5, end: 1 }), 0..2);
    }

    #[test]
    fn touch_ids_are_never_reused() {
        let mut ids = TouchIds::default();
        let a = ids.start(0);
        assert_eq!(ids.end(0), a);
        assert_ne!(ids.start(0), a);
    }
}
