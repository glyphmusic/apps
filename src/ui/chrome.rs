//! Window chrome: titlebar (logo, search, update notice, window pill) and the
//! narrow icon sidebar.

use crate::app::{BackendState, Glyph, View};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::widgets::{cover, icon, mono};
use gpui::{div, prelude::*, px, Context, FontWeight, MouseButton, SharedString, Window, WindowControlArea};

const IS_MAC: bool = cfg!(target_os = "macos");

pub fn titlebar(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let search_focused = app.search_input.read(cx).focus.is_focused(window);
    let shortcut = if IS_MAC { "⌘K" } else { "Ctrl K" };
    let update = app.available_update().map(|u| u.version.clone());

    let status: Option<(String, gpui::Hsla)> = match &app.backend_state {
        BackendState::Connecting => Some((format!("{} {}", app.spinner(), t("status.connecting")), theme::text_faint())),
        BackendState::Offline(_) => Some((t("status.backend_offline").to_string(), theme::warn())),
        BackendState::Ready(glyph_core::BackendMode::Incompatible { protocol_version, .. }) => {
            Some((tf("status.incompatible", &[("v", protocol_version)]).to_string(), theme::warn()))
        }
        _ => None,
    };

    let bar = div()
        .id("titlebar")
        .flex_none()
        .h(px(theme::TITLEBAR_H))
        .w_full()
        .flex()
        .items_center()
        .pl(px(if IS_MAC { 84.0 } else { 18.0 }))
        .pr(px(8.0))
        .gap(px(12.0));

    // Windows moves/maximizes natively through the Drag strip in `Shell`.
    #[cfg(not(windows))]
    let bar = bar.on_mouse_down(MouseButton::Left, |ev, window, _| {
        if ev.click_count == 2 {
            if IS_MAC {
                window.titlebar_double_click();
            } else {
                window.zoom_window();
            }
        } else {
            window.start_window_move();
        }
    });

    bar.child(
        mono()
            .flex_none()
            .text_size(px(15.0))
            .font_weight(FontWeight::BOLD)
            .text_color(theme::pink())
            .child("glyph"),
    )
    .child(div().flex_1())
    .child(theme::on_panel(|| {
        let dropdown = (search_focused && app.search.query.trim().is_empty())
            .then(|| search_dropdown(app, cx))
            .flatten();
        // Flat Liner-style field: solid dark surface, no border until focused,
        // light text, shortcut as a small key chip.
        div()
            .id("search-bar")
            .occlude()
            .relative()
            .flex()
            .items_center()
            .gap(px(10.0))
            .w(px(420.0))
            .h(px(32.0))
            .pl(px(12.0))
            .pr(px(6.0))
            .rounded(px(8.0))
            .bg(gpui::hsla(0.0, 0.0, 0.08, 0.92))
            .border_1()
            .border_color(if search_focused { gpui::hsla(0.0, 0.0, 1.0, 0.22) } else { gpui::transparent_black() })
            .text_size(theme::text_sm())
            .cursor_text()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.focus_search(window, cx)))
            .child(icon("search", 15.0, gpui::hsla(0.0, 0.0, 1.0, 0.55)))
            .child(div().flex_1().min_w_0().child(app.search_input.clone()))
            .child(
                mono()
                    .flex_none()
                    .px(px(6.0))
                    .h(px(20.0))
                    .flex()
                    .items_center()
                    .rounded(px(4.0))
                    .bg(gpui::hsla(0.0, 0.0, 1.0, 0.08))
                    .text_size(px(11.0))
                    .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.55))
                    .child(shortcut),
            )
            .children(dropdown)
    }))
    .child(div().flex_1())
    .when_some(status, |d, (s, c)| {
        d.child(
            mono()
                .id("backend-status")
                .occlude()
                .flex_none()
                .cursor_pointer()
                .text_size(theme::text_xs())
                .text_color(c)
                .child(s)
                .on_click(cx.listener(|this, _, w, cx| this.navigate(View::Settings, w, cx))),
        )
    })
    .when_some(update, |d, v| {
        d.child(
            mono()
                .id("update-badge")
                .occlude()
                .flex_none()
                .cursor_pointer()
                .text_size(theme::text_xs())
                .text_color(theme::ok())
                .hover(|s| s.text_color(theme::text()))
                .child(tf("update.badge", &[("v", &v.trim_start_matches('v'))]))
                .on_click(cx.listener(|this, _, w, cx| this.navigate(View::Settings, w, cx))),
        )
    })
    // The window pill itself is drawn by `Shell` (outside the cached view);
    // keep its space free here.
    .when(!IS_MAC, |d| d.child(div().flex_none().w(px(WINDOW_PILL_W))))
}

/// Dropdown under the focused, empty search bar: recent searches (or recently
/// played), each row cover + title + "Song • Artist", play overlay and a
/// remove button on hover. Painted deferred so it floats above the content.
fn search_dropdown(app: &mut Glyph, cx: &mut Context<Glyph>) -> Option<gpui::AnyElement> {
    let (is_history, items) = app.search_suggestions();
    if items.is_empty() {
        return None;
    }
    let rows: Vec<_> = items
        .into_iter()
        .enumerate()
        .map(|(i, tr)| {
            let group = SharedString::from(format!("sugg-{i}"));
            let art = cover(app, &tr, 44.0, 4.0, cx);
            let play = tr.clone();
            div()
                .id(("sugg", i))
                .group(group.clone())
                .flex()
                .items_center()
                .gap(px(12.0))
                .px(px(8.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(gpui::hsla(0.0, 0.0, 1.0, 0.08)))
                // Stop here: the search bar's own click would refocus the input.
                .on_click(cx.listener(move |this, _, w, cx| {
                    cx.stop_propagation();
                    this.play_suggestion(play.clone(), w, cx)
                }))
                .child(
                    div()
                        .relative()
                        .flex_none()
                        .child(art)
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded(px(4.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(gpui::hsla(0.0, 0.0, 0.0, 0.45))
                                .invisible()
                                .group_hover(group.clone(), |s| s.visible())
                                .child(icon("play", 16.0, gpui::white())),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.0))
                        .child(div().truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(tr.title.clone()))
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_sm())
                                .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.6))
                                .child(format!("{} • {}", t("search.kind.song"), tr.artist)),
                        ),
                )
                .when(is_history, |d| {
                    d.child(
                        div()
                            .id(("sugg-x", i))
                            .flex_none()
                            .size(px(28.0))
                            .rounded(px(6.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .invisible()
                            .group_hover(group.clone(), |s| s.visible())
                            .hover(|s| s.bg(gpui::hsla(0.0, 0.0, 1.0, 0.10)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.remove_search_history(i, cx);
                            }))
                            .child(icon("x", 14.0, gpui::hsla(0.0, 0.0, 1.0, 0.7))),
                    )
                })
        })
        .collect();

    Some(
        gpui::deferred(
            div()
                .id("search-dropdown")
                .occlude()
                .absolute()
                .top(px(38.0))
                .left_0()
                .right_0()
                .max_h(px(440.0))
                .overflow_y_scroll()
                .p(px(8.0))
                .rounded(px(8.0))
                .bg(gpui::hsla(0.0, 0.0, 0.11, 0.98))
                .border_1()
                .border_color(gpui::hsla(0.0, 0.0, 1.0, 0.06))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .px(px(8.0))
                        .pt(px(4.0))
                        .pb(px(8.0))
                        .text_size(theme::text_md())
                        .font_weight(FontWeight::BOLD)
                        .text_color(gpui::white())
                        .child(if is_history { t("search.history") } else { t("search.recently_played") }),
                )
                .children(rows),
        )
        .with_priority(1)
        .into_any_element(),
    )
}

/// Pill: 3 × 26 px buttons, 2 px gaps, 3 px padding (Liner's proportions).
pub const WINDOW_PILL_W: f32 = 3.0 * 26.0 + 2.0 * 2.0 + 2.0 * 3.0;
pub const WINDOW_PILL_H: f32 = 32.0;

/// Minimize / maximize / close as a flat dark pill. Plain click handlers on
/// every platform (no native caption hit areas): no OS tooltip, and the
/// actions are async (`ShowWindowAsync`) so they never re-enter GPUI.
pub fn window_pill() -> impl IntoElement {
    let btn = |id: &'static str, name: &'static str, area: WindowControlArea| {
        let close = area == WindowControlArea::Close;
        div()
            .id(id)
            .occlude()
            .size(px(26.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .group(id)
            .when(close, |d| d.hover(|s| s.bg(gpui::rgb(0xE81123))))
            .when(!close, |d| d.hover(|s| s.bg(gpui::hsla(0.0, 0.0, 1.0, 0.10))))
            // An svg is tinted only by its own text color (it does not inherit
            // the parent's), so hover colors go through the group.
            .child(
                gpui::svg()
                    .path(format!("icons/ui/{name}.svg"))
                    .size(px(if name == "square" { 11.0 } else { 13.0 }))
                    .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.70))
                    .group_hover(id, |s| s.text_color(gpui::white())),
            )
            .on_click(move |_, window, cx| match area {
                WindowControlArea::Min => window.minimize_window(),
                WindowControlArea::Max => window.zoom_window(),
                WindowControlArea::Close => cx.quit(),
                WindowControlArea::Drag => {}
            })
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(WINDOW_PILL_H))
        .px(px(3.0))
        .gap(px(2.0))
        .rounded(px(8.0))
        .bg(gpui::hsla(0.0, 0.0, 0.0, 0.50))
        .child(btn("win-min", "minus", WindowControlArea::Min))
        .child(btn("win-max", "square", WindowControlArea::Max))
        .child(btn("win-close", "x", WindowControlArea::Close))
}

pub fn sidebar(app: &mut Glyph, _window: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let item = |id: &'static str, name: &'static str, view: View, cx: &mut Context<Glyph>| {
        let active = if view == View::Settings { app.settings_open } else { app.view == view && !app.settings_open };
        div()
            .id(id)
            .w(px(44.0))
            .h(px(44.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.0))
            .cursor_pointer()
            .when(active, |d| d.bg(theme::active()))
            .hover(|s| s.bg(theme::hover()))
            .on_click(cx.listener(move |this, _, w, cx| this.navigate(view, w, cx)))
            .child(icon(name, 20.0, if active { theme::text() } else { theme::text_dim() }))
    };
    let home = item("nav-home", "house", View::Home, cx);
    let search = item("nav-search", "search", View::Search, cx);
    let lib = item("nav-lib", "library", View::Library, cx);
    let np = item("nav-np", "disc-3", View::NowPlaying, cx);
    let settings = item("nav-settings", "settings", View::Settings, cx);
    div()
        .flex_none()
        .w(px(theme::SIDEBAR_W))
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .py(px(6.0))
        .gap(px(6.0))
        .child(home)
        .child(search)
        .child(lib)
        .child(np)
        .child(div().flex_1())
        .child(settings)
}
