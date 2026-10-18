//! Window chrome: titlebar (logo, search, update notice, window pill) and the
//! narrow icon sidebar.

use crate::app::{BackendState, Glyph, View};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::widgets::{cover, hover_fill, hover_k, icon, mono};
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
        .map(|(i, item)| {
            let group = SharedString::from(format!("sugg-{i}"));
            let (art, title, sub): (gpui::AnyElement, String, String) = match &item {
                glyph_core::SearchItem::Song { track } => (
                    cover(app, track, 44.0, 4.0, cx).into_any_element(),
                    track.title.clone(),
                    format!("{} • {}", t("search.kind.song"), track.artist),
                ),
                glyph_core::SearchItem::Artist { name, track } => (
                    match track {
                        Some(tr) => cover(app, tr, 44.0, 22.0, cx).into_any_element(),
                        None => div()
                            .size(px(44.0))
                            .rounded_full()
                            .bg(gpui::hsla(0.0, 0.0, 1.0, 0.08))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("user-round", 18.0, gpui::hsla(0.0, 0.0, 1.0, 0.6)))
                            .into_any_element(),
                    },
                    name.clone(),
                    t("search.kind.artist").to_string(),
                ),
            };
            let is_song = matches!(item, glyph_core::SearchItem::Song { .. });
            let pick = item.clone();
            let hk = hover_k(&group);
            div()
                .id(("sugg", i))
                .flex()
                .items_center()
                .gap(px(12.0))
                .px(px(8.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .relative()
                .child(hover_fill(group.clone(), gpui::hsla(0.0, 0.0, 1.0, 0.08), 6.0))
                // Stop here: the search bar's own click would refocus the input.
                .on_click(cx.listener(move |this, _, w, cx| {
                    cx.stop_propagation();
                    match &pick {
                        glyph_core::SearchItem::Song { track } => this.play_suggestion(track.clone(), w, cx),
                        glyph_core::SearchItem::Artist { name, track } => this.pick_search_artist(name, track.clone(), w, cx),
                    }
                }))
                .child(
                    div()
                        .relative()
                        .flex_none()
                        .child(art)
                        .when(is_song, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .rounded(px(4.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(gpui::hsla(0.0, 0.0, 0.0, 0.45))
                                    .opacity(hk)
                                    .child(icon("play", 16.0, gpui::white())),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.0))
                        .child(div().truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(title))
                        .child(div().truncate().text_size(theme::text_sm()).text_color(gpui::hsla(0.0, 0.0, 1.0, 0.6)).child(sub)),
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
                            // Fades with the row; not clickable while hidden.
                            .opacity(hk)
                            .when(hk < 0.05, |d| d.invisible())
                            .relative()
                            .child(hover_fill(format!("sugg-x-{i}"), gpui::hsla(0.0, 0.0, 1.0, 0.10), 6.0))
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
                .track_scroll(&crate::ui::widgets::handle_for("search-dropdown"))
                .child(crate::ui::widgets::smooth_wheel(&crate::ui::widgets::handle_for("search-dropdown"), false, cx))
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

/// Seconds an icon takes to ease back after the mouse leaves.
pub const NAV_EASE_BACK: f32 = 0.3;

/// (rotation rad, lift px, scale) of a sidebar icon `t` seconds into hover.
fn nav_motion(view: View, t: f32) -> (f32, f32, f32) {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    match view {
        View::NowPlaying => (t * TAU / 1.4, 0.0, 1.0),
        View::Settings => {
            let k = (t / 0.5).min(1.0);
            ((1.0 - (1.0 - k).powi(3)) * FRAC_PI_2, 0.0, 1.0)
        }
        View::Home => {
            let k = (t / 0.35).min(1.0);
            (0.0, -3.0 * (k * PI).sin(), 1.0)
        }
        View::Search => (0.25 * (t * 18.0).sin() * (-t * 5.0).exp(), 0.0, 1.0),
        _ => {
            let k = (t / 0.3).min(1.0);
            (0.0, 0.0, 1.0 + 0.16 * (k * PI).sin())
        }
    }
}

/// Label that floats to the right of a sidebar item after a short hover.
thread_local! {
    /// Start of the sidebar hover whose tooltip shows without the wait.
    static SIDE_INSTANT: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// Wait before a sidebar tooltip shows (none when tooltips are warm).
const SIDE_TIP_WAIT: f32 = 0.35;

fn side_tip_wait(at: std::time::Instant) -> f32 {
    if SIDE_INSTANT.with(|c| c.get()) == Some(at) { 0.0 } else { SIDE_TIP_WAIT }
}

/// The tooltip of the sidebar hover that started at `at` is showing.
fn side_tip_visible(at: std::time::Instant) -> bool {
    at.elapsed().as_secs_f32() >= side_tip_wait(at)
}

/// A sidebar item is entered: its hover start. Right after another tooltip
/// (this sidebar's `prev`, or any other) it shows at once.
fn side_tip_enter(prev: Option<std::time::Instant>) -> std::time::Instant {
    let now = std::time::Instant::now();
    if crate::ui::widgets::tips_warm() || prev.is_some_and(side_tip_visible) {
        SIDE_INSTANT.with(|c| c.set(Some(now)));
    }
    now
}

/// A sidebar item is left: if its tooltip was showing, tooltips are warm.
fn side_tip_leave(at: std::time::Instant) {
    if side_tip_visible(at) {
        crate::ui::widgets::tip_closed();
    }
}

fn side_tooltip(label: SharedString, since: Option<std::time::Instant>) -> Option<gpui::AnyElement> {
    let since = since?;
    let t = since.elapsed().as_secs_f32();
    let a = ((t - side_tip_wait(since)) / 0.15).clamp(0.0, 1.0);
    if a <= 0.0 {
        return None;
    }
    Some(
        gpui::deferred(
            // Same look as the other tooltips, arrow pointing at the icon.
            div()
                .absolute()
                .left(px(47.0 - 4.0 * (1.0 - a)))
                .top_0()
                .h(px(44.0))
                .opacity(a)
                .flex()
                .items_center()
                .child(crate::ui::widgets::tip_arrow(crate::ui::widgets::TipArrow::Left))
                .child(crate::ui::widgets::tip_box(label)),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

/// Recently opened lists as small covers under the navigation (newest first).
fn recent_lists(app: &mut Glyph, cx: &mut Context<Glyph>) -> Option<gpui::AnyElement> {
    let lists = app.recent_lists(5);
    if lists.is_empty() {
        return None;
    }
    let open = if app.view == View::List { app.open_list.clone() } else { None };
    let items: Vec<_> = lists
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            let active = open.as_ref().is_some_and(|o| o.same(&l));
            let art = crate::ui::lists::list_cover(app, &l, 36.0, cx);
            let tip = side_tooltip(app.list_title(&l).into(), app.side_hover.filter(|(h, _)| *h == i).map(|(_, at)| at));
            let lm = l.clone();
            crate::ui::lists::with_list_menu(div().id(("side-list", i)), lm, cx)
                .relative()
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        let prev = this.side_hover.map(|(_, at)| at).or(this.nav_hover.map(|(_, at)| at));
                        this.side_hover = Some((i, side_tip_enter(prev)));
                    } else if let Some((_, at)) = this.side_hover.filter(|(h, _)| *h == i) {
                        side_tip_leave(at);
                        this.side_hover = None;
                    }
                    cx.notify();
                }))
                .children(tip)
                .size(px(44.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.0))
                .cursor_pointer()
                .when(active, |d| d.bg(theme::active()))
                .hover(|s| s.bg(theme::hover()))
                .on_click(cx.listener(move |this, _, _, cx| this.open_list(l.clone(), cx)))
                .child(art)
                .into_any_element()
        })
        .collect();
    Some(
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.0))
            .child(div().w(px(24.0)).h(px(1.0)).my(px(4.0)).bg(gpui::hsla(0.0, 0.0, 1.0, 0.08)))
            .children(items)
            .into_any_element(),
    )
}

pub fn sidebar(app: &mut Glyph, _window: &mut Window, cx: &mut Context<Glyph>) -> impl IntoElement {
    let item = |id: &'static str, name: &'static str, view: View, cx: &mut Context<Glyph>| {
        let active = if view == View::Settings { app.settings_open } else { app.view == view && !app.settings_open };
        // Hover animation per icon: the disc spins, the gear turns, the house
        // hops, the magnifier wiggles, the library pops. On leave it eases back.
        let (angle, dy, scale) = match (app.nav_hover, app.nav_leave) {
            (Some((v, at)), _) if v == view => nav_motion(view, at.elapsed().as_secs_f32()),
            (_, Some((v, at, t_left))) if v == view => {
                let k = (at.elapsed().as_secs_f32() / NAV_EASE_BACK).min(1.0);
                let back = 1.0 - (1.0 - k).powi(3);
                let (a, dy, s) = nav_motion(view, t_left);
                // The disc finishes its turn instead of spinning backwards.
                let a = a.rem_euclid(std::f32::consts::TAU);
                let target = if view == View::NowPlaying && a > std::f32::consts::PI { std::f32::consts::TAU } else { 0.0 };
                (a + (target - a) * back, dy * (1.0 - back), 1.0 + (s - 1.0) * (1.0 - back))
            }
            _ => (0.0, 0.0, 1.0),
        };
        let label = match view {
            View::Home => t("nav.home"),
            View::Search => t("nav.search"),
            View::Library => t("nav.library"),
            View::NowPlaying => t("nav.now_playing"),
            _ => t("nav.settings"),
        };
        let tip = side_tooltip(label, app.nav_hover.filter(|(v, _)| *v == view).map(|(_, at)| at));
        let tf = gpui::Transformation::rotate(gpui::radians(angle))
            .with_translation(gpui::point(px(0.0), px(dy)))
            .with_scaling(gpui::size(scale, scale));
        div()
            .id(id)
            .relative()
            .children(tip)
            .w(px(44.0))
            .h(px(44.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.0))
            .cursor_pointer()
            .when(active, |d| d.bg(theme::active()))
            .hover(|s| s.bg(theme::hover()))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    let prev = this.nav_hover.map(|(_, at)| at).or(this.side_hover.map(|(_, at)| at));
                    this.nav_hover = Some((view, side_tip_enter(prev)));
                } else if let Some((v, at)) = this.nav_hover.filter(|(v, _)| *v == view) {
                    side_tip_leave(at);
                    this.nav_hover = None;
                    this.nav_leave = Some((v, std::time::Instant::now(), at.elapsed().as_secs_f32()));
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, w, cx| this.navigate(view, w, cx)))
            .child(icon(name, 20.0, if active { theme::text() } else { theme::text_dim() }).with_transformation(tf))
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
        .pt(px(12.0))
        .pb(px(12.0))
        .gap(px(6.0))
        .child(home)
        .child(search)
        .child(lib)
        .child(np)
        .children(recent_lists(app, cx))
        .child(div().flex_1())
        .child(settings)
}
