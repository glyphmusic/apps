//! UI: root layout, actions and keybindings.
//!
//! Hard rule: gradients / glow live only in the background field
//! (`visuals::glow`). Everything here is flat: black 60–75 % panels,
//! 1 px white 10–15 % borders, song colours only as flat fills.

pub mod account;
pub mod chrome;
pub mod compact;
pub mod connect;
pub mod controls;
pub mod input;
pub mod jam;
pub mod keys;
pub mod lists;
pub mod local;
pub mod migrate;
pub mod mini;
pub mod now_playing;
pub mod player_bar;
pub mod profile;
pub mod settings;
pub mod splash;
pub mod stats;
pub mod toasts;
pub mod transfer;
pub mod views;
pub mod widgets;
pub mod ytdlp;

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
        ListUp,
        ListDown,
        ListActivate,
        ListRemove,
        Nav1,
        Nav2,
        Nav3,
        Nav4,
        Nav5,
        Nav6,
        Nav7,
        Nav8,
        Nav9,
        Nav0,
        LoopStart,
        LoopEnd,
        LoopClear,
        CloseOverlay,
        Quit,
        PlayRecent0,
        PlayRecent1,
        PlayRecent2,
        PlayRecent3,
        PlayRecent4,
        ToggleZen,
        DownloadCurrent,
        VolumeUp,
        VolumeDown,
        ToggleMute,
        LikeCurrent,
        ToggleShuffle,
        CycleRepeat
    ]
);

const MAIN: &str = "Glyph && !TextInput";

/// All key bindings: the text fields', the fixed ones (lists, sidebar,
/// closing) and every shortcut with the key `ui` gives it (see
/// [`keys::SHORTCUTS`]). Changing a shortcut clears the keymap and calls
/// this again.
pub fn bind_keys(cx: &mut App, ui: &glyph_core::config::UiConfig) {
    input::bind_keys(cx);
    cx.bind_keys(keys::bindings(ui));
    cx.bind_keys([
        KeyBinding::new("up", ListUp, Some(MAIN)),
        KeyBinding::new("down", ListDown, Some(MAIN)),
        KeyBinding::new("enter", ListActivate, Some(MAIN)),
        KeyBinding::new("delete", ListRemove, Some(MAIN)),
        KeyBinding::new("backspace", ListRemove, Some(MAIN)),
        KeyBinding::new("escape", CloseOverlay, Some("Glyph")),
        // Ctrl+1..9 pick sidebar entries top to bottom (Settings is a shortcut).
        KeyBinding::new("secondary-1", Nav1, None),
        KeyBinding::new("secondary-2", Nav2, None),
        KeyBinding::new("secondary-3", Nav3, None),
        KeyBinding::new("secondary-4", Nav4, None),
        KeyBinding::new("secondary-5", Nav5, None),
        KeyBinding::new("secondary-6", Nav6, None),
        KeyBinding::new("secondary-7", Nav7, None),
        KeyBinding::new("secondary-8", Nav8, None),
        KeyBinding::new("secondary-9", Nav9, None),
    ]);
}

/// Global actions (once at start-up).
pub fn register_actions(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
}

pub use keys::key_label;

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
            // Phones: the glow fills the whole small screen; dim it so text
            // on top stays readable (Spotify-dark).
            .when(compact::TOUCH, |d| d.child(div().absolute().inset_0().bg(gpui::hsla(0.0, 0.0, 0.0, 0.5))))
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
            .when(!cfg!(any(target_os = "macos", target_os = "android", target_os = "ios")), |d| {
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
        for im in self.stale_heroes.drain(..) {
            let _ = window.drop_image(im);
        }
        widgets::begin_window_frame(window);
        // The window title follows the song from `Glyph::poll` (also while
        // the window is in the background and nothing renders).
        if widgets::step_smooth_scroll() {
            window.request_animation_frame();
        }
        theme::set_ink(self.ink);
        widgets::set_ascii_effects(self.config.visuals.ascii_effects);
        theme::set_theme(&self.config.visuals.theme);
        theme::set_zone(Some(0));
        // Recording a shortcut's key belongs to the shortcuts page; leaving
        // it gives the keymap back.
        if self.key_capture.is_some() && !(self.settings_open && self.settings_page == crate::app::SettingsPage::About && self.settings_sub == 2) {
            self.cancel_key_capture(cx);
        }
        // Zen mode belongs to Now Playing with a song.
        if self.zen && (self.view != View::NowPlaying || self.current.is_none() || self.settings_open) {
            self.zen = false;
        }
        // Phones: Spotify-like layout (see `compact`); the desktop is unchanged.
        let compact = compact::is_compact(window);
        let zen = self.zen && !compact;
        // Phones: Now Playing is a sheet over the page (see
        // `compact::now_playing`); while it covers the screen the page is not
        // built at all.
        if compact {
            compact::now_playing::adopt_view(self, cx);
        }
        let np_top = if compact { compact::now_playing::sheet_top(self, window) } else { None };
        let np_covers = np_top.is_some_and(|t| t < 0.5);
        let titlebar = if np_covers {
            div().into_any_element()
        } else if compact {
            match self.view {
                View::Home if !self.settings_open => compact::header(self, Some(crate::i18n::t("home.greeting")), Vec::new(), cx).into_any_element(),
                View::Search => compact::search_bar(self, window, cx),
                View::Library => {
                    let actions = compact::library_actions(cx);
                    compact::header(self, Some(crate::i18n::t("nav.library")), actions, cx).into_any_element()
                }
                View::Home | View::Settings => div().into_any_element(),
                _ if compact::floating_back(self) => div().into_any_element(),
                _ => compact::back_bar(self, window, cx),
            }
        } else if zen {
            // Only the window's own buttons stay (they live in `Shell`).
            div().h(px(theme::TITLEBAR_H)).into_any_element()
        } else {
            chrome::titlebar(self, window, cx).into_any_element()
        };
        theme::set_zone(Some(1));
        let sidebar = (!zen).then(|| chrome::sidebar(self, window, cx).into_any_element());
        // Settings is built from cards: panel ink throughout.
        theme::set_zone((self.view != View::Settings).then_some(2));
        let main = if np_covers {
            div().into_any_element()
        } else {
            match self.view {
                View::Home => views::home(self, window, cx).into_any_element(),
                View::Search => views::search(self, window, cx).into_any_element(),
                View::Library if compact => compact::library(self, window, cx),
                View::Library => lists::library(self, window, cx),
                View::List if compact => compact::list_view(self, window, cx),
                View::List => lists::list_view(self, window, cx),
                View::Artist => lists::artist_view(self, window, cx),
                View::Profile => profile::view(self, window, cx),
                View::Section => views::section_view(self, window, cx).into_any_element(),
                View::Releases => lists::releases_view(self, window, cx).into_any_element(),
                View::Similar => lists::similar_view(self, window, cx).into_any_element(),
                View::Stats => stats::view(self, window, cx),
                View::Local => local::view(self, window, cx),
                // Phones: never the view (`adopt_view` above), always the sheet.
                View::NowPlaying if compact => div().into_any_element(),
                View::NowPlaying => now_playing::now_playing(self, window, cx).into_any_element(),
                // Settings is a modal over the current view (see below).
                View::Settings => views::home(self, window, cx).into_any_element(),
            }
        };
        theme::set_zone(None);
        // Every page switch (and its tabs) fades the new page in.
        let page_key = widgets::fade_key((
            format!("{:?}", self.view),
            format!("{:?}", self.open_list),
            &self.open_artist,
            &self.open_profile,
            self.section.as_ref().map(|s| &s.0),
            format!("{:?}", self.search.filter),
            self.lib_filter,
            self.view == View::NowPlaying && self.show_lyrics,
            self.view == View::NowPlaying && self.lyrics_only,
            zen,
        ));
        let main = if np_covers { main } else { widgets::fade_in("page-fade", page_key, true, main) };
        // Content floats directly on the background (Liner): no big panel.
        // Scroll frames fade in/out at their edges instead of a hard cut.
        let main = if self.view == View::NowPlaying {
            main
        } else {
            // The bottom edge is deeper, so lists sink away instead of being cut off.
            widgets::fade_edges_split(main, (gpui::hsla(0.0, 0.0, 0.0, 0.3), 28.0), (gpui::hsla(0.0, 0.0, 0.0, 0.5), 52.0), Some(&self.main_scroll))
                .size_full()
                .into_any_element()
        };
        let main_bar = (self.view != View::NowPlaying && !compact).then(|| widgets::scrollbar(&self.main_scroll, !self.settings_open, cx));
        if zen {
            self.show_queue = false;
            self.show_devices = false;
        }
        if self.show_queue {
            self.show_devices = false;
            if let Some(a) = self.add_songs.as_mut() {
                a.open = false;
            }
        }
        // "Add songs" belongs to its playlist: leaving the page closes it.
        let on_list = |id: &str| self.view == View::List && self.open_list.as_ref().is_some_and(|l| *l == crate::app::ListRef::User(id.to_string()));
        if self.add_songs.as_ref().is_some_and(|a| a.open && !on_list(&a.id)) {
            if let Some(a) = self.add_songs.as_mut() {
                a.open = false;
            }
        }
        let add_open = self.add_songs.as_ref().is_some_and(|a| a.open);
        if add_open {
            self.show_devices = false;
        }
        // Phones: the queue and Connect are bottom sheets, not side panels.
        let right_open = if compact { add_open } else { self.show_queue || add_open || self.show_devices };
        // The page reaches into the window margin on the right so its
        // scrollbar sits there, next to the content instead of over it.
        let main_panel = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .when(!compact, |d| d.mr(px(-theme::GAP)))
            // Next to the queue the page runs right up to it (no gap), like it
            // runs under the player bar's margin.
            .child(div().size_full().pr(px(if right_open || compact { 0.0 } else { theme::GAP })).child(div().relative().size_full().overflow_hidden().child(main)))
            .when_some(main_bar, |d, bar| {
                d.child(div().absolute().top_0().bottom_0().right(px(if right_open { 0.0 } else { -5.0 })).w(px(theme::GAP + 2.0)).child(bar))
            });
        // The queue slides in from the right edge and out again.
        // Same slot for "Add songs": opening one closes the other.
        let qk = self.queue_slide.update(right_open);
        if qk <= 0.001 && !add_open {
            self.add_songs = None;
        }
        let show_add = add_open || (!self.show_queue && !self.show_devices && self.add_songs.is_some());
        let queue = (qk > 0.001).then(|| {
            let inner = if show_add {
                views::add_songs_panel(self, window, cx).into_any_element()
            } else if self.show_devices {
                connect::panel(self, window, cx).into_any_element()
            } else {
                views::queue_panel(self, window, cx).into_any_element()
            };
            // Queue / Connect / "Add songs" swap with a fade, not a cut.
            let inner = widgets::fade_in("side-fade", widgets::fade_key((show_add, self.show_devices)), true, inner);
            div()
                .flex_none()
                .h_full()
                .w(px(views::QUEUE_W * qk))
                .when(!show_add, |d| d.pt(px(theme::GAP)))
                .ml(px(-theme::GAP * (1.0 - qk)))
                .overflow_hidden()
                .child(div().w(px(views::QUEUE_W)).h_full().child(inner))
                .into_any_element()
        });
        // Song + lyrics in Now Playing carries its own transport (Liner):
        // no second player bar there. Lyrics-only keeps the bar.
        let full_np = self.view == View::NowPlaying && self.current.is_some() && !(self.show_lyrics && self.lyrics_only) && !zen;
        // The bar slides in when something starts playing and out when it is
        // not needed (nothing loaded, or Now Playing with its own transport).
        let want_bar = !full_np && self.current.is_some();
        let shown = self.bar_slide.update(want_bar);
        // Phones: bottom tab bar instead of the sidebar, mini player instead
        // of the player bar, all inside the safe area (see `compact`).
        let safe = window.insets().safe_area;
        // Phone Now Playing is full screen: no mini player, no tabs.
        let want_bar = want_bar && !(compact && self.view == View::NowPlaying);
        let bar = (shown > 0.0).then(|| {
            // "Playing on …" / "Connect to …" under the bar (see `connect::status_strip`).
            let strip = connect::status_strip(self, cx);
            let strip_h = if strip.is_some() { connect::STRIP_H + 6.0 } else { 0.0 };
            if compact {
                let inner = compact::mini_bar(self, window, cx).map(|b| b.into_any_element());
                return div()
                    .flex_none()
                    .relative()
                    .overflow_hidden()
                    .h(px((compact::MINI_H + theme::GAP + strip_h) * shown))
                    .children(inner.map(|b| div().absolute().left_0().right_0().top(px(0.0)).px(px(theme::GAP)).pt(px(theme::GAP)).flex().flex_col().gap(px(6.0)).child(b).children(strip)))
                    .into_any_element();
            }
            let h = theme::PLAYER_H + theme::GAP * 2.0 + strip_h;
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
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(player_bar::player_bar(self, window, cx))
                        .children(strip),
                )
                .into_any_element()
        });
        let tabs = (compact && self.view != View::NowPlaying).then(|| {
            div()
                .flex_none()
                .px(px(theme::GAP))
                .pt(px(theme::GAP))
                .pb(px(theme::GAP) + safe.bottom)
                // Dark under the tabs so the labels stay readable over the glow.
                .bg(gpui::linear_gradient(
                    180.0,
                    gpui::linear_color_stop(gpui::hsla(0.0, 0.0, 0.0, 0.75), 0.0),
                    gpui::linear_color_stop(gpui::hsla(0.0, 0.0, 0.0, 0.97), 0.5),
                ))
                .child(compact::tab_bar(self, cx))
                .into_any_element()
        });
        let toasts = toasts::layer(self, window, cx);
        let sk = self.settings_k();
        let modal = if compact {
            self.settings_open.then(|| div().absolute().inset_0().child(compact::settings(self, window, cx)))
        } else {
            (sk > 0.0).then(|| div().absolute().inset_0().opacity(sk).child(settings::modal(self, window, cx)))
        };
        let drawer = if compact { compact::drawer(self, window, cx) } else { None };
        let menu = lists::ctx_menu(self, window, cx);
        let control_menu = controls::control_menu(self, window, cx);
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
        let approval = controls::mcp_approval(self, cx);
        let alpha = compact::alpha_notice(window, cx);
        let crop = self.profile.crop.is_some().then(|| crate::ui::profile::crop_dialog(self, cx));
        let dim = sk > 0.0 && !compact;
        let dim_a = 1.0 - 0.92 * sk;

        let page = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .when(compact && !compact::floating_back(self), |d| d.pt(safe.top))
            // Behind the settings modal only the (already soft) glow stays
            // visible: GPUI has no backdrop blur, so the UI fades out instead.
            .child(div().flex_none().when(dim, |d| d.opacity(dim_a)).child(titlebar))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap(px(theme::GAP))
                    // Phones: pages run edge to edge and pad themselves.
                    .when(!compact, |d| d.px(px(theme::GAP)))
                    // The page runs on under the player bar's top margin
                    // instead of stopping at a hard edge above it.
                    .mb(px(-theme::GAP * shown))
                    .when(dim, |d| d.opacity(dim_a))
                    // Side panels keep their gap to the bar.
                    .when_some(sidebar.filter(|_| !compact), |d, sidebar| d.child(div().flex_none().h_full().pb(px(theme::GAP * shown)).child(sidebar)))
                    .child(main_panel)
                    // Always one gap above the bottom: the player bar's when it
                    // is there, the window edge's when it is not (Now Playing).
                    .children(queue.map(|q| div().flex_none().h_full().pb(px(theme::GAP)).child(q))),
            )
            .children(bar.map(|b| div().flex_none().when(dim, |d| d.opacity(dim_a)).child(b)))
            .children(tabs.map(|t| div().flex_none().when(dim, |d| d.opacity(dim_a)).child(t)))
            .children((compact && compact::floating_back(self)).then(|| compact::floating_back_bar(self, window, cx)));
        // Phones: with Now Playing up, the page shows only above its sheet.
        let page = match np_top {
            Some(top) => compact::now_playing::under_sheet(page, top, window),
            None => page.into_any_element(),
        };
        let np_layers = if compact { compact::now_playing::layers(self, np_top, window, cx) } else { Vec::new() };

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
            .on_action(cx.listener(|this, _: &ListUp, window, cx| this.select_row_step(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ListDown, window, cx| this.select_row_step(1, window, cx)))
            .on_action(cx.listener(|this, _: &ListActivate, _, cx| this.activate_selected_row(cx)))
            .on_action(cx.listener(|this, _: &ListRemove, _, cx| this.remove_selected_row(cx)))
            // Taskbar jump list / dock menu: recently played.
            .on_action(cx.listener(|this, _: &PlayRecent0, _, cx| this.play_recent(0, cx)))
            .on_action(cx.listener(|this, _: &Nav1, w, cx| this.nav_slot(1, w, cx)))
            .on_action(cx.listener(|this, _: &Nav2, w, cx| this.nav_slot(2, w, cx)))
            .on_action(cx.listener(|this, _: &Nav3, w, cx| this.nav_slot(3, w, cx)))
            .on_action(cx.listener(|this, _: &Nav4, w, cx| this.nav_slot(4, w, cx)))
            .on_action(cx.listener(|this, _: &Nav5, w, cx| this.nav_slot(5, w, cx)))
            .on_action(cx.listener(|this, _: &Nav6, w, cx| this.nav_slot(6, w, cx)))
            .on_action(cx.listener(|this, _: &Nav7, w, cx| this.nav_slot(7, w, cx)))
            .on_action(cx.listener(|this, _: &Nav8, w, cx| this.nav_slot(8, w, cx)))
            .on_action(cx.listener(|this, _: &Nav9, w, cx| this.nav_slot(9, w, cx)))
            .on_action(cx.listener(|this, _: &Nav0, w, cx| this.nav_slot(0, w, cx)))
            .on_action(cx.listener(|this, _: &LoopStart, _, cx| this.set_loop_point(true, cx)))
            .on_action(cx.listener(|this, _: &LoopEnd, _, cx| this.set_loop_point(false, cx)))
            .on_action(cx.listener(|this, _: &LoopClear, _, cx| this.clear_loop(cx)))
            .on_action(cx.listener(|this, _: &PlayRecent1, _, cx| this.play_recent(1, cx)))
            .on_action(cx.listener(|this, _: &PlayRecent2, _, cx| this.play_recent(2, cx)))
            .on_action(cx.listener(|this, _: &PlayRecent3, _, cx| this.play_recent(3, cx)))
            .on_action(cx.listener(|this, _: &PlayRecent4, _, cx| this.play_recent(4, cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, w, cx| this.focus_search(w, cx)))
            .on_action(cx.listener(|this, _: &ToggleLyrics, w, cx| {
                this.zen = false;
                if this.view == View::NowPlaying {
                    this.show_lyrics = !this.show_lyrics;
                    cx.notify();
                } else {
                    this.show_lyrics = true;
                    this.navigate(View::NowPlaying, w, cx);
                }
                // Remembered for the next start (Settings > Player shows it).
                this.config.ui.np_lyrics = this.show_lyrics;
                this.save_config(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleZen, w, cx| this.toggle_zen(w, cx)))
            .on_action(cx.listener(|this, _: &VolumeUp, _, cx| {
                this.set_volume(this.volume() + 0.05);
                cx.notify()
            }))
            .on_action(cx.listener(|this, _: &VolumeDown, _, cx| {
                this.set_volume(this.volume() - 0.05);
                cx.notify()
            }))
            .on_action(cx.listener(|this, _: &ToggleMute, _, cx| {
                this.toggle_mute();
                cx.notify()
            }))
            .on_action(cx.listener(|this, _: &LikeCurrent, _, cx| {
                if let Some(t) = this.current.clone() {
                    this.toggle_like(&t, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleShuffle, _, cx| this.toggle_shuffle(cx)))
            .on_action(cx.listener(|this, _: &CycleRepeat, _, cx| this.cycle_repeat(cx)))
            // Recording a new key for a shortcut (Settings › About › Shortcuts).
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _, cx| {
                if this.key_capture.is_some() {
                    cx.stop_propagation();
                    this.capture_key(&e.keystroke, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &DownloadCurrent, _, cx| this.download_current(cx)))
            .on_action(cx.listener(|this, _: &CloseOverlay, w, cx| {
                // Phones: Escape is the system back gesture.
                if compact::TOUCH && this.name_dialog.is_none() && this.control_menu.is_none() && this.ctx_menu.is_none() && this.open_select.is_none() {
                    if compact::now_playing::back_pressed(this, cx) {
                        return;
                    }
                    if this.drawer_open {
                        this.drawer_open = false;
                    } else if this.settings_open && this.settings_in_page {
                        this.settings_in_page = false;
                    } else if this.settings_open {
                        this.close_settings(w, cx);
                    } else if !this.history.is_empty() || !matches!(this.view, View::Home | View::Search | View::Library) {
                        compact::back(this, cx);
                    } else {
                        // Nothing to go back to: Android moves the app to the background.
                        cx.propagate();
                    }
                    cx.notify();
                    return;
                }
                if this.name_dialog.is_some() {
                    this.close_name_dialog(cx);
                } else if this.control_menu.is_some() {
                    this.close_control_menu(cx);
                } else if this.ctx_menu.is_some() {
                    this.close_ctx(cx);
                } else if this.open_select.is_some() {
                    this.open_select = None;
                    cx.notify();
                } else if this.settings_open {
                    this.close_settings(w, cx);
                } else if this.zen {
                    this.zen = false;
                    cx.notify();
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
            .child(page)
            .children(np_layers)
            .children(modal)
            .children(drawer)
            .children(menu)
            .children(control_menu)
            .children(dialog)
            .children(approval)
            .children(alpha)
            .children(crop)
            .children(splash)
            .children(toasts)
            .children(widgets::tooltip_layer(window))
    }
}
