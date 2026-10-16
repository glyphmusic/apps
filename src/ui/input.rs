//! Minimal single-line text field (search bar, server URL, API keys).
//! Keyboard only: typing, Backspace, Ctrl/Cmd+Backspace (clear), Enter,
//! Escape, Ctrl/Cmd+V paste.

use crate::theme;
use gpui::{
    div, prelude::*, px, App, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable,
    KeyDownEvent, MouseButton, SharedString, Window,
};

#[derive(Debug, Clone)]
pub enum InputEvent {
    Changed(String),
    Submit(String),
    Cancel,
}

pub struct TextInput {
    pub focus: FocusHandle,
    pub value: String,
    pub placeholder: SharedString,
    pub masked: bool,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            value: String::new(),
            placeholder: placeholder.into(),
            masked: false,
        }
    }

    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    pub fn set_value(&mut self, v: impl Into<String>, cx: &mut Context<Self>) {
        self.value = v.into();
        cx.notify();
    }

    fn key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let cmd = ks.modifiers.control || ks.modifiers.platform;
        let mut changed = false;
        match ks.key.as_str() {
            "backspace" => {
                if cmd {
                    self.value.clear();
                } else {
                    self.value.pop();
                }
                changed = true;
            }
            "enter" => {
                cx.emit(InputEvent::Submit(self.value.clone()));
            }
            "escape" => {
                cx.emit(InputEvent::Cancel);
                window.blur(cx);
            }
            "v" if cmd => {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    self.value.push_str(text.lines().next().unwrap_or_default());
                    changed = true;
                }
            }
            "c" if cmd && !self.masked => {
                cx.write_to_clipboard(ClipboardItem::new_string(self.value.clone()));
            }
            _ => {
                if !cmd {
                    if let Some(ch) = ks.key_char.as_ref() {
                        if !ch.chars().any(|c| c.is_control()) {
                            self.value.push_str(ch);
                            changed = true;
                        }
                    }
                }
            }
        }
        cx.stop_propagation();
        if changed {
            cx.emit(InputEvent::Changed(self.value.clone()));
            cx.notify();
        }
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let shown: SharedString = if self.value.is_empty() {
            crate::i18n::t(&self.placeholder)
        } else if self.masked {
            "•".repeat(self.value.chars().count().min(32)).into()
        } else {
            self.value.clone().into()
        };
        let empty = self.value.is_empty();
        div()
            .id("text-input")
            .key_context("TextInput")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus, cx);
                    cx.stop_propagation();
                }),
            )
            .flex()
            .items_center()
            .min_w_0()
            .overflow_hidden()
            .size_full()
            // Inputs always sit on their own flat surface: fixed light ink,
            // never the adaptive one (this entity renders outside any scope).
            .text_color(if empty { theme::on_panel(theme::text_faint) } else { theme::on_panel(theme::text) })
            .child(div().flex_none().whitespace_nowrap().child(shown))
            .when(focused, |d| {
                d.child(
                    div()
                        .ml(px(1.0))
                        .w(px(6.0))
                        .h(px(13.0))
                        .bg(theme::on_panel(theme::text).opacity(0.75)),
                )
            })
    }
}
