//! UI: root layout, actions and keybindings.
//!
//! Hard rule: gradients / glow live only in the background field
//! (`visuals::glow`). Everything here is flat: black 60–75 % panels,
//! 1 px white 10–15 % borders, song colours only as flat fills.

pub mod chrome;
pub mod input;
pub mod now_playing;
pub mod player_bar;
pub mod settings;
pub mod views;
pub mod widgets;

use crate::app::{Glyph, View};
use crate::theme;
use gpui::{
    actions, div, prelude::*, px, App, Context, Entity, KeyBinding, StyleRefinement, Window,
};

actions!(
    glyph,
    [
        TogglePlay,
        SeekBack,
        SeekForward,
        NextTrack,
        PrevTrack,
        FocusSearch,
        ToggleLyrics,
        ToggleQueue,
        Quit
    ]
);

const MAIN: &str = "Glyph && !TextInput";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("space", TogglePlay, Some(MAIN)),
        KeyBinding::new("left", SeekBack, Some(MAIN)),
        KeyBinding::new("right", SeekForward, Some(MAIN)),
        KeyBinding::new("secondary-left", PrevTrack, Some("Glyph")),
        KeyBinding::new("secondary-right", NextTrack, Some("Glyph")),
        KeyBinding::new("secondary-k", FocusSearch, None),
        KeyBinding::new("secondary-l", ToggleLyrics, None),
        KeyBinding::new("secondary-q", Quit, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

/// Window root: the glow field behind everything, the (cached) app on top,
/// so the 30 fps field never re-renders the UI tree.
pub struct Shell {
    pub glow: Entity<crate::visuals::glow::GlowView>,
    pub app: Entity<Glyph>,
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let decorations = window.window_decorations();
        let rounded = match decorations {
            gpui::Decorations::Client { tiling } => !tiling.is_tiled(),
            gpui::Decorations::Server => false,
        };
        div()
            .relative()
            .size_full()
            .bg(theme::bg())
            .font_family(theme::FONT)
            .text_size(theme::text_md())
            .text_color(theme::text())
            .when(rounded, |d| {
                d.rounded(px(theme::WINDOW_RADIUS))
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme::border_faint())
            })
            .child(div().absolute().inset_0().child(self.glow.clone()))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(self.app.clone().cached(StyleRefinement::default().size_full())),
            )
    }
}

impl Render for Glyph {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let titlebar = chrome::titlebar(self, window, cx).into_any_element();
        let sidebar = chrome::sidebar(self, window, cx).into_any_element();
        let main = match self.view {
            View::Home => views::home(self, window, cx).into_any_element(),
            View::Search => views::search(self, window, cx).into_any_element(),
            View::Library => views::library(self, window, cx).into_any_element(),
            View::NowPlaying => now_playing::now_playing(self, window, cx).into_any_element(),
            View::Settings => settings::settings(self, window, cx).into_any_element(),
        };
        // Now playing lets the glow fill the area: no panel behind it.
        let main_panel = if self.view == View::NowPlaying {
            div().flex_1().min_w_0().h_full().child(main)
        } else {
            widgets::panel().flex_1().min_w_0().h_full().overflow_hidden().child(main)
        };
        let queue = self
            .show_queue
            .then(|| views::queue_panel(self, window, cx).into_any_element());
        let bar = player_bar::player_bar(self, window, cx).into_any_element();
        let toast = self.toast.as_ref().map(|(m, _)| m.clone());

        div()
            .id("glyph-root")
            .key_context("Glyph")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &TogglePlay, _, cx| this.toggle_play(cx)))
            .on_action(cx.listener(|this, _: &SeekBack, _, cx| {
                this.seek_relative(-5.0);
                cx.notify()
            }))
            .on_action(cx.listener(|this, _: &SeekForward, _, cx| {
                this.seek_relative(5.0);
                cx.notify()
            }))
            .on_action(cx.listener(|this, _: &NextTrack, _, cx| this.next(false, cx)))
            .on_action(cx.listener(|this, _: &PrevTrack, _, cx| this.previous(cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, w, cx| this.focus_search(w, cx)))
            .on_action(cx.listener(|this, _: &ToggleLyrics, w, cx| {
                if this.view == View::NowPlaying {
                    this.show_lyrics = !this.show_lyrics;
                    cx.notify();
                } else {
                    this.show_lyrics = true;
                    this.navigate(View::NowPlaying, w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleQueue, _, cx| {
                this.show_queue = !this.show_queue;
                cx.notify()
            }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, w, cx| {
                    // Clicking outside an input returns focus to the app (shortcuts).
                    if !this.focus.is_focused(w) {
                        let f = this.focus.clone();
                        w.focus(&f, cx);
                    }
                }),
            )
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(titlebar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap(px(theme::GAP))
                    .px(px(theme::GAP))
                    .child(sidebar)
                    .child(main_panel)
                    .children(queue),
            )
            .child(div().p(px(theme::GAP)).child(bar))
            .when_some(toast, |d, m| {
                d.child(
                    div()
                        .absolute()
                        .right(px(18.0))
                        .bottom(px(theme::PLAYER_H + 26.0))
                        .px(px(12.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .bg(theme::panel_strong())
                        .border_1()
                        .border_color(theme::border())
                        .text_size(theme::text_xs())
                        .text_color(theme::text_dim())
                        .child(m),
                )
            })
    }
}
