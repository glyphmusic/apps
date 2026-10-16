//! Window chrome: titlebar (logo, search, update notice, window pill) and the
//! narrow icon sidebar.

use crate::app::{BackendState, Glyph, View};
use crate::theme;
use crate::ui::widgets::pill;
use gpui::{div, prelude::*, px, Context, FontWeight, MouseButton, Window, WindowControlArea};

const IS_MAC: bool = cfg!(target_os = "macos");

pub fn titlebar(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let search_focused = app.search_input.read(cx).focus.is_focused(window);
    let shortcut = if IS_MAC { "⌘K" } else { "Ctrl K" };
    let update = app.available_update().map(|u| u.version.clone());

    let status: Option<(String, gpui::Hsla)> = match &app.backend_state {
        BackendState::Connecting => Some((format!("{} verbinde", app.spinner()), theme::text_faint())),
        BackendState::Offline(_) => Some(("[ BACKEND OFFLINE ]".into(), theme::warn())),
        BackendState::Ready(glyph_core::BackendMode::Incompatible { protocol_version, .. }) => {
            Some((format!("[ SERVER v{protocol_version} · INKOMPATIBEL ]"), theme::warn()))
        }
        _ => None,
    };

    let bar = div()
        .id("titlebar")
        .window_control_area(WindowControlArea::Drag)
        .flex_none()
        .h(px(theme::TITLEBAR_H))
        .w_full()
        .flex()
        .items_center()
        .pl(px(if IS_MAC { 84.0 } else { 14.0 }))
        .pr(px(8.0))
        .gap(px(12.0));

    // Non-Windows: we move/maximize ourselves (Windows does it natively
    // through the Drag control area / HTCAPTION).
    #[cfg(not(windows))]
    let bar = bar
        .on_mouse_down(MouseButton::Left, |ev, window, _| {
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

    bar
        // Logo
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .flex_none()
                .child(
                    div()
                        .text_color(theme::text())
                        .font_weight(FontWeight::BOLD)
                        .child("glyph"),
                )
                .child(div().text_size(theme::text_xs()).text_color(theme::text_faint()).child("·:")),
        )
        .child(div().flex_1())
        // Search bar
        .child(
            div()
                .id("search-bar")
                .occlude()
                .flex()
                .items_center()
                .gap(px(10.0))
                .w(px(420.0))
                .h(px(26.0))
                .px(px(12.0))
                .rounded_full()
                .bg(theme::panel_strong())
                .border_1()
                .border_color(if search_focused { theme::border_strong() } else { theme::border() })
                .text_size(theme::text_sm())
                .cursor_text()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| this.focus_search(window, cx)))
                .child(div().flex_none().text_color(theme::text_faint()).child("/"))
                .child(div().flex_1().min_w_0().child(app.search_input.clone()))
                .child(
                    div()
                        .flex_none()
                        .text_size(theme::text_xs())
                        .text_color(theme::text_ghost())
                        .child(shortcut),
                ),
        )
        .child(div().flex_1())
        .when_some(status, |d, (s, c)| {
            d.child(
                div()
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
                div()
                    .id("update-badge")
                    .occlude()
                    .flex_none()
                    .cursor_pointer()
                    .text_size(theme::text_xs())
                    .text_color(theme::ok())
                    .hover(|s| s.text_color(theme::text()))
                    .child(format!("[ UPDATE v{} VERFÜGBAR ]", v.trim_start_matches('v')))
                    .on_click(cx.listener(|this, _, w, cx| this.navigate(View::Settings, w, cx))),
            )
        })
        .when(!IS_MAC, |d| d.child(window_pill(window)))
}

fn window_pill(window: &Window) -> impl IntoElement {
    let maximized = window.is_maximized();
    let btn = |id: &'static str, label: &'static str, area: WindowControlArea, close: bool| {
        let b = div()
            .id(id)
            .occlude()
            .window_control_area(area)
            .w(px(30.0))
            .h(px(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .text_color(theme::text_dim())
            .cursor_pointer()
            .when(close, |d| d.hover(|s| s.text_color(theme::danger()).bg(theme::danger().opacity(0.12))))
            .when(!close, |d| d.hover(|s| s.text_color(theme::text()).bg(theme::hover())))
            .child(label);
        #[cfg(not(windows))]
        let b = b.on_click(move |_, window, cx| match area {
            WindowControlArea::Min => window.minimize_window(),
            WindowControlArea::Max => window.zoom_window(),
            WindowControlArea::Close => cx.quit(),
            WindowControlArea::Drag => {}
        });
        b
    };
    pill()
        .flex_none()
        .h(px(26.0))
        .px(px(2.0))
        .gap(px(1.0))
        .text_size(theme::text_md())
        .child(btn("win-min", "–", WindowControlArea::Min, false))
        .child(btn("win-max", if maximized { "◫" } else { "□" }, WindowControlArea::Max, false))
        .child(btn("win-close", "×", WindowControlArea::Close, true))
}

pub fn sidebar(app: &mut Glyph, _window: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let accent = app
        .palette
        .map(|p| p.accent())
        .unwrap_or(crate::visuals::Rgb::hex(0xff5fd2));
    let item = |id: &'static str, glyph: &'static str, label: &'static str, view: View, cx: &mut Context<Glyph>| {
        let active = app.view == view;
        div()
            .id(id)
            .relative()
            .w(px(40.0))
            .h(px(40.0))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .rounded(px(8.0))
            .cursor_pointer()
            .text_color(if active { theme::text() } else { theme::text_faint() })
            .when(active, |d| d.bg(theme::active()))
            .hover(|s| s.bg(theme::hover()).text_color(theme::text()))
            .on_click(cx.listener(move |this, _, w, cx| this.navigate(view, w, cx)))
            .child(div().text_size(px(15.0)).child(glyph))
            .child(div().text_size(px(7.5)).text_color(theme::text_ghost()).child(label))
            .when(active, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(-8.0))
                        .top(px(12.0))
                        .w(px(2.0))
                        .h(px(16.0))
                        .rounded_full()
                        .bg(accent.lighten(0.2).hsla(1.0)),
                )
            })
    };
    let home = item("nav-home", "⌂", "home", View::Home, cx);
    let search = item("nav-search", "/?", "suche", View::Search, cx);
    let lib = item("nav-lib", "|||", "lib", View::Library, cx);
    let np = item("nav-np", "♪", "jetzt", View::NowPlaying, cx);
    let settings = item("nav-settings", "*", "setup", View::Settings, cx);
    crate::ui::widgets::panel()
        .flex_none()
        .w(px(theme::SIDEBAR_W))
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .py(px(10.0))
        .gap(px(6.0))
        .child(home)
        .child(search)
        .child(lib)
        .child(np)
        .child(div().flex_1())
        .child(settings)
}
