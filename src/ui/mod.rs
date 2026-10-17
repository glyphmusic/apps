//! UI: root layout, actions and keybindings.
//!
//! Hard rule: gradients / glow live only in the background field
//! (`visuals::glow`). Everything here is flat: black 60–75 % panels,
//! 1 px white 10–15 % borders, song colours only as flat fills.

pub mod chrome;
pub mod input;
pub mod lists;
pub mod now_playing;
pub mod player_bar;
pub mod settings;
pub mod splash;
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
        CloseOverlay,
        Quit,
        PlayRecent0,
        PlayRecent1,
        PlayRecent2,
        PlayRecent3,
        PlayRecent4
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
        KeyBinding::new("escape", CloseOverlay, Some("Glyph")),
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
            // Window control areas (drag strip, window buttons) live here, outside
            // the cached app view: GPUI does not carry them over when it reuses a
            // cached view's paint, so inside it they vanish after the first frame
            // and Windows stops treating the titlebar as a titlebar.
            // The drag strip sits *behind* the app, so the search bar and other
            // occluding titlebar elements still take their own clicks.
            .child(
                div()
                    .id("drag-strip")
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(theme::TITLEBAR_H))
                    .window_control_area(gpui::WindowControlArea::Drag),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(self.app.clone().cached(StyleRefinement::default().size_full())),
            )
            .when(!cfg!(target_os = "macos"), |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px((theme::TITLEBAR_H - chrome::WINDOW_PILL_H) / 2.0))
                        .right(px(8.0))
                        .child(chrome::window_pill()),
                )
            })
    }
}

impl Render for Glyph {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::set_ink(self.ink);
        theme::set_zone(Some(0));
        let titlebar = chrome::titlebar(self, window, cx).into_any_element();
        theme::set_zone(Some(1));
        let sidebar = chrome::sidebar(self, window, cx).into_any_element();
        // Settings is built from cards: panel ink throughout.
        theme::set_zone((self.view != View::Settings).then_some(2));
        let main = match self.view {
            View::Home => views::home(self, window, cx).into_any_element(),
            View::Search => views::search(self, window, cx).into_any_element(),
            View::Library => lists::library(self, window, cx),
            View::List => lists::list_view(self, window, cx),
            View::Artist => lists::artist_view(self, window, cx),
            View::NowPlaying => now_playing::now_playing(self, window, cx).into_any_element(),
            // Settings is a modal over the current view (see below).
            View::Settings => views::home(self, window, cx).into_any_element(),
        };
        theme::set_zone(None);
        // Content floats directly on the background (Liner): no big panel.
        // Scroll frames fade in/out at their edges instead of a hard cut.
        let main = if self.view == View::NowPlaying {
            main
        } else {
            let bar = widgets::scrollbar(&self.main_scroll, !self.settings_open, cx);
            widgets::fade_edges(main, gpui::hsla(0.0, 0.0, 0.0, 0.3), 28.0, Some(&self.main_scroll)).size_full().child(bar).into_any_element()
        };
        let main_panel = div().flex_1().min_w_0().h_full().overflow_hidden().child(main);
        let queue = self
            .show_queue
            .then(|| views::queue_panel(self, window, cx).into_any_element());
        // Song + lyrics in Now Playing carries its own transport (Liner):
        // no second player bar there. Lyrics-only keeps the bar.
        let full_np = self.view == View::NowPlaying && self.current.is_some() && !(self.show_lyrics && self.lyrics_only);
        // The bar slides in when something starts playing and out when it is
        // not needed (nothing loaded, or Now Playing with its own transport).
        let want_bar = !full_np && self.current.is_some();
        let shown = self.bar_slide.update(want_bar);
        let bar = (shown > 0.0).then(|| {
            let h = theme::PLAYER_H + theme::GAP * 2.0;
            div()
                .flex_none()
                .relative()
                .overflow_hidden()
                .h(px(h * shown))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(0.0))
                        .p(px(theme::GAP))
                        .child(player_bar::player_bar(self, window, cx)),
                )
                .into_any_element()
        });
        let toast = self.toast.as_ref().map(|(m, at)| {
            let e = at.elapsed().as_secs_f32();
            let left = crate::app::TOAST_FOR.as_secs_f32() - e;
            (m.clone(), ((e / 0.2).min(1.0) * (left / 0.35).clamp(0.0, 1.0)).clamp(0.0, 1.0))
        });
        let modal = self.settings_open.then(|| settings::modal(self, window, cx));
        let menu = lists::ctx_menu(self, window, cx);
        if self.splash.as_ref().is_some_and(|s| !s.active()) {
            self.splash = None;
        }
        let splash = self.splash.as_ref().map(|s| {
            div()
                .id("splash")
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| {
                    if let Some(s) = this.splash.as_mut() {
                        s.skipped = true;
                    }
                    cx.notify();
                }))
                .child(s.element())
        });
        let dialog = self.name_dialog.is_some().then(|| lists::name_dialog(self, cx));
        let dim = self.settings_open;

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
            // Taskbar jump list / dock menu: recently played.
            .on_action(cx.listener(|this, _: &PlayRecent0, _, cx| this.play_recent(0, cx)))
            .on_action(cx.listener(|this, _: &PlayRecent1, _, cx| this.play_recent(1, cx)))
            .on_action(cx.listener(|this, _: &PlayRecent2, _, cx| this.play_recent(2, cx)))
            .on_action(cx.listener(|this, _: &PlayRecent3, _, cx| this.play_recent(3, cx)))
            .on_action(cx.listener(|this, _: &PlayRecent4, _, cx| this.play_recent(4, cx)))
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
            .on_action(cx.listener(|this, _: &CloseOverlay, w, cx| {
                if this.name_dialog.is_some() {
                    this.close_name_dialog(cx);
                } else if this.ctx_menu.is_some() {
                    this.close_ctx(cx);
                } else if this.open_select.is_some() {
                    this.open_select = None;
                    cx.notify();
                } else if this.settings_open {
                    this.close_settings(w, cx);
                } else {
                    // Closes the search dropdown (focus leaves the input).
                    let f = this.focus.clone();
                    w.focus(&f, cx);
                    cx.notify();
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
            // Behind the settings modal only the (already soft) glow stays
            // visible: GPUI has no backdrop blur, so the UI fades out instead.
            .child(div().flex_none().when(dim, |d| d.opacity(0.08)).child(titlebar))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap(px(theme::GAP))
                    .px(px(theme::GAP))
                    .when(dim, |d| d.opacity(0.08))
                    .child(sidebar)
                    .child(main_panel)
                    .children(queue),
            )
            .children(bar.map(|b| div().flex_none().when(dim, |d| d.opacity(0.08)).child(b)))
            .children(modal)
            .children(menu)
            .children(dialog)
            .children(splash)
            .when_some(toast, |d, (m, alpha)| {
                // Centred above the player bar; fades and lifts in, fades out.
                let bottom = if shown > 0.0 { (theme::PLAYER_H + theme::GAP * 2.0) * shown + 12.0 } else { 24.0 };
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(px(bottom + (1.0 - alpha) * 8.0))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .px(px(14.0))
                                .h(px(34.0))
                                .rounded(px(8.0))
                                .bg(gpui::hsla(0.0, 0.0, 0.1, 0.97))
                                .border_1()
                                .border_color(gpui::hsla(0.0, 0.0, 1.0, 0.08))
                                .opacity(alpha)
                                .text_size(px(13.0))
                                .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.92))
                                .child(widgets::icon("circle-check", 14.0, gpui::hsla(0.0, 0.0, 1.0, 0.7)))
                                .child(m),
                        ),
                )
            })
    }
}
