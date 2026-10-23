//! Android input to GPUI: raw touches (GPUI's portable recognizers turn them
//! into taps, scrolls and long presses), hardware keys, and the soft keyboard
//! through GameActivity's text input state.
//!
//! Touches are copied out of GameActivity's buffer and handed over once per
//! run-loop pass, with the moves in between merged into the newest one.
//! GPUI's recognizers time every touch sample on arrival: the moves that
//! piled up while a frame was drawn, delivered back to back, looked like a
//! finger crossing the screen in no time, and a short swipe flung the list
//! at full speed.
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
    GestureTuning, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, PlatformGestures, PlatformInput, Pixels,
    Point, ScrollPhysics, TouchEvent, TouchId, TouchPhase, px,
};
use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

/// Android's feel for GPUI's recognizers (`ViewConfiguration` defaults, in
/// dp, which are GPUI's logical pixels here): `OverScroller` flings instead
/// of iOS's long glide, the stock long-press and double-tap timeouts.
pub(crate) struct AndroidGestures;

impl PlatformGestures for AndroidGestures {
    fn tuning(&self) -> GestureTuning {
        GestureTuning {
            touch_slop: px(8.),
            multi_tap_interval: Duration::from_millis(300),
            multi_tap_slop: px(100.),
            long_press_duration: Duration::from_millis(400),
            scroll_physics: ScrollPhysics::android(),
            min_fling_velocity: 50.,
        }
    }
}

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

    /// Touches still open when a new gesture starts: their release got lost.
    fn take_stale(&mut self) -> Vec<TouchId> {
        let mut ids: Vec<TouchId> = self.live.drain().map(|(_, id)| TouchId(id)).collect();
        ids.sort_by_key(|id| id.0);
        ids
    }
}

/// What a motion event did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    /// First finger down (a new gesture).
    Down,
    /// Another finger down.
    PointerDown,
    Move,
    /// A finger up (the last one or another one).
    Up,
    Cancel,
}

#[derive(Clone, Copy, Debug)]
struct Pointer {
    id: i32,
    position: Point<Pixels>,
    force: Option<f32>,
}

/// One motion event, copied out of GameActivity's input buffer.
#[derive(Clone, Debug)]
pub(crate) struct Motion {
    act: Act,
    /// The pointer that went down or up.
    index: usize,
    pointers: Vec<Pointer>,
}

impl Motion {
    fn read(event: &MotionEvent, scale: f32) -> Option<Self> {
        let act = match event.action() {
            MotionAction::Down => Act::Down,
            MotionAction::PointerDown => Act::PointerDown,
            MotionAction::Move => Act::Move,
            MotionAction::Up | MotionAction::PointerUp => Act::Up,
            MotionAction::Cancel => Act::Cancel,
            _ => return None,
        };
        let pointers = (0..event.pointer_count())
            .map(|i| {
                let p = event.pointer_at_index(i);
                let pressure = p.pressure();
                Pointer {
                    id: p.pointer_id(),
                    position: Point::new(px(p.x() / scale), px(p.y() / scale)),
                    force: (pressure > 0.0).then(|| pressure.clamp(0.0, 1.0)),
                }
            })
            .collect();
        Some(Self { act, index: event.pointer_index(), pointers })
    }
}

/// Drops every move that a later move of the same run supersedes (a move
/// carries all fingers' positions). Downs, ups and cancels all stay.
fn coalesce(motions: Vec<Motion>) -> Vec<Motion> {
    let mut out: Vec<Motion> = Vec::with_capacity(motions.len());
    for m in motions {
        if m.act == Act::Move && out.last().is_some_and(|l| l.act == Act::Move) {
            out.pop();
        }
        out.push(m);
    }
    out
}

/// Motions copied during one run-loop pass, until [`flush_motions`].
#[derive(Default)]
pub(crate) struct PendingMotions(Vec<Motion>);

/// What GameTextInput was last told, so its echo of our own state is not
/// applied a second time.
#[derive(Default)]
pub(crate) struct ImeSync {
    /// True while an IME edit is being applied to the GPUI input handler
    /// (its change notifications must not be pushed back).
    pub applying: bool,
    /// Focus moved to an input during a draw: its text is pushed (and the
    /// keyboard shown) once the frame is done, when the input can be read.
    pub pending: bool,
    last_text: Option<String>,
}

impl ImeSync {
    pub fn reset(&mut self) {
        self.applying = false;
        self.pending = false;
        self.last_text = None;
    }
}

/// Handles one event; `true` when GPUI used it. Motions are only copied
/// (see [`flush_motions`]); anything else first delivers the motions before
/// it, so the order stays.
pub(crate) fn handle(state: &AndroidWindowState, event: &InputEvent) -> bool {
    if let InputEvent::MotionEvent(m) = event {
        if let Some(m) = Motion::read(m, state.scale()) {
            state.motions.borrow_mut().0.push(m);
        }
        return true;
    }
    flush_motions(state);
    match event {
        InputEvent::KeyEvent(k) => key(state, k),
        InputEvent::TextEvent(t) => {
            apply_text_event(state, t.clone());
            true
        }
        InputEvent::TextAction(a) => text_action(state, *a),
        _ => false,
    }
}

/// Delivers the copied motions, the moves merged (see the module docs).
pub(crate) fn flush_motions(state: &AndroidWindowState) {
    let motions = std::mem::take(&mut state.motions.borrow_mut().0);
    for m in coalesce(motions) {
        motion(state, &m);
    }
}

fn motion(state: &AndroidWindowState, m: &Motion) {
    let send = |id: TouchId, phase: TouchPhase, p: &Pointer| {
        state.dispatch(PlatformInput::Touch(TouchEvent {
            id,
            phase,
            position: p.position,
            predicted_position: None,
            force: p.force,
        }));
    };
    let Some(at) = m.pointers.get(m.index).or(m.pointers.first()) else { return };
    match m.act {
        Act::Down | Act::PointerDown => {
            if m.act == Act::Down {
                // A new gesture while touches are still open: their up got
                // lost (e.g. a surface change). GPUI ignores new touches
                // while one is being recognized, so close them first, or
                // every tap and scroll after it would go nowhere.
                let stale = state.touches.borrow_mut().take_stale();
                for id in stale {
                    send(id, TouchPhase::Cancelled, at);
                }
            }
            let id = state.touches.borrow_mut().start(at.id);
            send(id, TouchPhase::Started, at);
        }
        Act::Move => {
            for p in &m.pointers {
                let id = state.touches.borrow_mut().get(p.id);
                send(id, TouchPhase::Moved, p);
            }
        }
        Act::Up => {
            let id = state.touches.borrow_mut().end(at.id);
            send(id, TouchPhase::Ended, at);
        }
        Act::Cancel => {
            for p in &m.pointers {
                let id = state.touches.borrow_mut().end(p.id);
                send(id, TouchPhase::Cancelled, p);
            }
        }
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
/// `None` while the input cannot be read (e.g. during a draw): an empty text
/// in its place would make the keyboard's next edit wipe the field.
fn read_handler(state: &AndroidWindowState) -> Option<(String, Range<usize>, Option<Range<usize>>)> {
    state
        .with_input_handler(|h| {
            let mut actual = None;
            let text = h.text_for_range(0..usize::MAX / 2, &mut actual)?;
            let selection = h.selected_text_range(false).map(|s| s.range).unwrap_or(0..0);
            Some((text, selection, h.marked_text_range()))
        })
        .flatten()
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

    #[test]
    fn lost_releases_are_found() {
        let mut ids = TouchIds::default();
        let a = ids.start(0);
        let b = ids.start(1);
        assert_eq!(ids.take_stale(), vec![a, b]);
        assert!(ids.take_stale().is_empty());
    }

    fn mo(act: Act, y: f32) -> Motion {
        Motion { act, index: 0, pointers: vec![Pointer { id: 0, position: Point::new(px(0.), px(y)), force: None }] }
    }

    #[test]
    fn piled_up_moves_merge_into_the_newest() {
        let out = coalesce(vec![
            mo(Act::Down, 0.),
            mo(Act::Move, 10.),
            mo(Act::Move, 20.),
            mo(Act::Move, 30.),
            mo(Act::Up, 30.),
            mo(Act::Down, 50.),
            mo(Act::Move, 60.),
        ]);
        let got: Vec<(Act, f32)> = out.iter().map(|m| (m.act, f32::from(m.pointers[0].position.y))).collect();
        assert_eq!(got, vec![(Act::Down, 0.), (Act::Move, 30.), (Act::Up, 30.), (Act::Down, 50.), (Act::Move, 60.)]);
    }
}
