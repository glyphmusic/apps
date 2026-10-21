//! Phone layout: the same app and views as the desktop, arranged for a narrow
//! touch screen (modelled on Spotify's phone app):
//!
//! * a header with the profile picture on the left; tapping it slides the
//!   profile menu in from the left ([`drawer`]);
//! * three tabs at the bottom (Home, Search, Library) with the mini player
//!   above them; Now Playing slides up over the page from the mini player
//!   ([`now_playing`]), Settings opens from the menu;
//! * Settings as a full-screen list with quick actions on top ([`settings`]);
//! * the Library as a filterable list ([`library`]);
//! * swipes ([`swipe_step`]): left/right on the mini player or the Now
//!   Playing cover skips, up on the mini player opens Now Playing, down at
//!   the top of Now Playing closes it, right on a song row queues the song,
//!   right on the header opens the profile menu and left closes it.
//!
//! The pieces copy the desktop components (cover, play button, rows, chips,
//! switches) with their sizes and colours. Nothing here runs on the desktop.

use crate::app::{Glyph, ListRef, SettingsPage, View};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::lists::list_cover;
use crate::ui::widgets::*;
use gpui::{div, hsla, prelude::*, px, AnyElement, Context, FontWeight, MouseButton, ScrollWheelEvent, SharedString, TouchPhase, Window};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

pub mod now_playing;

/// Phones (Android, iOS).
pub const TOUCH: bool = cfg!(any(target_os = "android", target_os = "ios"));

/// Phones always get the compact layout; desktop windows never do, so the
/// desktop app looks exactly as before.
pub fn is_compact(_window: &Window) -> bool {
    TOUCH
}

/// Mini player height (cover 48 + padding), as the desktop bar's cover row.
pub const MINI_H: f32 = 64.0;
/// Tab bar height (touch targets + label).
pub const TABS_H: f32 = 50.0;
/// Header row with the profile picture.
pub const HEADER_H: f32 = 60.0;
/// Horizontal page padding on phones (layout GAP + PAGE_X).
const EDGE: f32 = theme::PAGE_X;

fn surface() -> gpui::Hsla {
    hsla(0.0, 0.0, 0.07, 1.0)
}

fn white(a: f32) -> gpui::Hsla {
    hsla(0.0, 0.0, 1.0, a)
}

// ------------------------------------------------------------------ swipes

/// Distance a swipe must travel to act (skip, queue, open/close).
const SWIPE_ACT: f32 = 72.0;
/// Seconds a released swipe takes to ease back to rest.
const SWIPE_SETTLE: f32 = 0.24;

/// One swipe on an element. GPUI turns touch pans into scroll events, locked
/// to one axis and hit-tested where the finger went down: `Started`,
/// `Moved`..., `Ended` at release, then maybe a fling's momentum, which a
/// swipe swallows.
#[derive(Clone, Copy, Default)]
struct Swipe {
    /// Finger travel (x, y) of the pan in progress, or at release.
    d: (f32, f32),
    /// The finger is down and this element took the pan.
    live: bool,
    /// Released; the fling's momentum events still belong to this swipe.
    fling: bool,
    /// Easing back to rest from this offset since then.
    settle: Option<(Instant, (f32, f32))>,
    /// Keep the offset after release (the element is leaving anyway).
    hold: bool,
}

thread_local! {
    static SWIPES: RefCell<HashMap<String, Swipe>> = RefCell::new(HashMap::new());
}

/// What a scroll event means for a swipe.
enum Step {
    /// Not this element's pan: let it scroll whatever is underneath.
    Pass,
    /// Taken (finger moved, or fling swallowed): stop it here, redraw.
    Take,
    /// The finger lifted after travelling (x, y).
    Release(f32, f32),
}

/// Feeds one scroll event to the swipe on `key`. `claim` sees the pan's
/// first movement (x, y) and says whether this element takes the pan.
fn swipe_step(key: &str, e: &ScrollWheelEvent, claim: impl FnOnce(f32, f32) -> bool) -> Step {
    let p = e.delta.pixel_delta(px(16.0));
    let (dx, dy) = (f32::from(p.x), f32::from(p.y));
    SWIPES.with(|m| {
        let mut m = m.borrow_mut();
        match e.touch_phase {
            TouchPhase::Started => {
                if !claim(dx, dy) {
                    if let Some(s) = m.get_mut(key) {
                        s.live = false;
                        s.fling = false;
                    }
                    return Step::Pass;
                }
                m.insert(key.to_string(), Swipe { d: (dx, dy), live: true, ..Swipe::default() });
                Step::Take
            }
            TouchPhase::Moved => match m.get_mut(key) {
                Some(s) if s.live => {
                    s.d.0 += dx;
                    s.d.1 += dy;
                    Step::Take
                }
                Some(s) if s.fling => Step::Take,
                _ => Step::Pass,
            },
            TouchPhase::Ended | TouchPhase::Cancelled => match m.get_mut(key) {
                Some(s) if s.live => {
                    s.d.0 += dx;
                    s.d.1 += dy;
                    s.live = false;
                    s.fling = true;
                    s.settle = Some((Instant::now(), s.d));
                    if e.touch_phase == TouchPhase::Ended { Step::Release(s.d.0, s.d.1) } else { Step::Take }
                }
                Some(s) if s.fling => {
                    s.fling = false;
                    Step::Take
                }
                _ => Step::Pass,
            },
        }
    })
}

/// After an acting release: ease in from `from` instead (the next song's
/// cover comes in from the other side).
fn swipe_settle_from(key: &str, from: (f32, f32)) {
    SWIPES.with(|m| {
        if let Some(s) = m.borrow_mut().get_mut(key) {
            s.settle = Some((Instant::now(), from));
        }
    });
}

/// After an acting release: stay where the finger left it.
fn swipe_hold(key: &str) {
    SWIPES.with(|m| {
        if let Some(s) = m.borrow_mut().get_mut(key) {
            s.hold = true;
        }
    });
}

fn swipe_reset(key: &str) {
    SWIPES.with(|m| m.borrow_mut().remove(key));
}

/// Where the swipe on `key` has the element now: the finger's travel while
/// it is down, easing back to rest after release (true while easing).
fn swipe_value(key: &str) -> ((f32, f32), bool) {
    SWIPES.with(|m| {
        let mut m = m.borrow_mut();
        let Some(s) = m.get_mut(key) else { return ((0.0, 0.0), false) };
        if s.live || s.hold {
            return (s.d, false);
        }
        let Some((at, from)) = s.settle else { return ((0.0, 0.0), false) };
        let t = at.elapsed().as_secs_f32() / SWIPE_SETTLE;
        if t >= 1.0 {
            s.settle = None;
            return ((0.0, 0.0), false);
        }
        // ease-out cubic, as `Slide`
        let k = (1.0 - t).powi(3);
        ((from.0 * k, from.1 * k), true)
    })
}

/// [`swipe_value`], asking for frames while it eases back.
fn swipe_offset(key: &str, window: &mut Window) -> (f32, f32) {
    let (d, easing) = swipe_value(key);
    if easing {
        window.request_animation_frame();
    }
    d
}

/// Redraws while a released swipe eases back, for elements built without a
/// window (song rows).
fn settle_frames(cx: &mut Context<Glyph>) {
    cx.spawn(async move |this, cx| {
        let end = Instant::now() + std::time::Duration::from_secs_f32(SWIPE_SETTLE + 0.05);
        while Instant::now() < end {
            cx.background_executor().timer(std::time::Duration::from_millis(16)).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Song row swipes (as Spotify): right adds the song to the queue, left
/// plays it next. The row follows the finger over the action's icon, which
/// lights up once a release would act.
fn queue_swipe(row: gpui::Stateful<gpui::Div>, key: String, tr: glyph_protocol::Track, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    let ((x, _), _) = swipe_value(&key);
    let x = x.clamp(-160.0, 160.0);
    let ink = if x.abs() >= SWIPE_ACT { theme::pink() } else { theme::text_dim() };
    let slot = |d: gpui::Div| d.absolute().top_0().bottom_0().w(px(x.abs())).flex().items_center().justify_center();
    row.relative()
        .left(px(x))
        .on_scroll_wheel(cx.listener(move |this, e: &ScrollWheelEvent, _, cx| match swipe_step(&key, e, horizontal) {
            Step::Pass => {}
            Step::Take => {
                cx.stop_propagation();
                cx.notify();
            }
            Step::Release(dx, _) => {
                cx.stop_propagation();
                if dx.abs() >= SWIPE_ACT {
                    this.queue_add(tr.clone(), dx < 0.0, cx);
                }
                settle_frames(cx);
                cx.notify();
            }
        }))
        .when(x > 0.5, |d| d.child(slot(div()).left(px(-x)).child(icon("list-plus", 20.0, ink))))
        .when(x < -0.5, |d| d.child(slot(div()).right(px(x)).child(icon("list-start", 20.0, ink))))
}

fn horizontal(dx: f32, dy: f32) -> bool {
    dx.abs() > dy.abs()
}

/// Skips on a left/right swipe: left plays the next song, right the
/// previous one; the swiped element then eases in from the other side.
fn swipe_skip(this: &mut Glyph, key: &str, dx: f32, cx: &mut Context<Glyph>) {
    if dx.abs() < SWIPE_ACT {
        return;
    }
    if dx < 0.0 {
        this.next(false, cx);
    } else {
        this.previous_song(cx);
    }
    swipe_settle_from(key, (-dx.signum() * 120.0, 0.0));
}

/// Opens Now Playing over the current page ([`now_playing`]).
fn open_now_playing(this: &mut Glyph, cx: &mut Context<Glyph>) {
    now_playing::open(this, cx);
}

// ------------------------------------------------------------------ header

/// Profile picture + page title (+ actions on the right).
pub fn header(app: &mut Glyph, title: Option<SharedString>, actions: Vec<AnyElement>, cx: &mut Context<Glyph>) -> impl IntoElement {
    let pfp = crate::ui::profile::own_avatar(app, 40.0, cx);
    div()
        // Swipe right: the profile menu (as the tap on the picture).
        .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, _, cx| match swipe_step("header", e, horizontal) {
            Step::Pass => {}
            Step::Take => cx.stop_propagation(),
            Step::Release(dx, _) => {
                cx.stop_propagation();
                if dx > SWIPE_ACT {
                    this.drawer_open = true;
                    cx.notify();
                }
            }
        }))
        .flex_none()
        .h(px(HEADER_H))
        .px(px(EDGE))
        .flex()
        .items_center()
        .gap(px(12.0))
        .child(
            div()
                .id("pfp")
                .flex_none()
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.drawer_open = true;
                    cx.notify();
                }))
                .child(pfp),
        )
        // Only one flexible child: a spacer beside the title would take half
        // of its width and cut it short ("Welcome ba...").
        .when(title.is_none(), |d| d.child(div().flex_1()))
        .when_some(title, |d, title| {
            d.child(div().flex_1().min_w_0().truncate().text_size(px(22.0)).font_weight(FontWeight::BOLD).text_color(theme::text()).child(title))
        })
        .children(actions)
}

fn icon_action(id: &'static str, name: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(40.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .active(|s| s.bg(white(0.08)))
        .child(icon(name, 22.0, theme::text()))
}

/// Dims the glow behind Now Playing so its text stays readable.
const NP_SCRIM: gpui::Hsla = gpui::Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.2 };

/// Back from a sub page; with nothing to go back to, the page's tab root.
pub fn back(app: &mut Glyph, cx: &mut Context<Glyph>) {
    if app.history.is_empty() {
        app.view = View::Home;
        app.main_scroll.set_offset(gpui::point(px(0.0), px(0.0)));
        cx.notify();
    } else {
        app.go_back(cx);
    }
}

/// Pinned bar above every sub page (lists, artists, Now Playing, ...): the
/// back button stays reachable however far the page is scrolled.
pub fn back_bar(app: &mut Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    let np = app.view == View::NowPlaying;
    let title: Option<SharedString> = match app.view {
        View::NowPlaying => Some(t("nav.now_playing")),
        _ => None,
    };
    div()
        .flex_none()
        .h(px(HEADER_H - 8.0))
        .when(np, |d| d.bg(NP_SCRIM))
        .px(px(4.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .child(icon_action("back-bar", if np { "chevron-down" } else { "arrow-left" }).on_click(cx.listener(|this, _, _, cx| back(this, cx))))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .when(np, |d| d.flex().justify_center())
                .text_size(px(if np { 13.0 } else { 16.0 }))
                .font_weight(if np { FontWeight::MEDIUM } else { FontWeight::BOLD })
                .text_color(theme::text())
                .children(title),
        )
        .child(div().flex_none().size(px(40.0)))
        .into_any_element()
}

/// Pages whose picture runs up under the status bar (artist): no back bar,
/// the button floats over the page instead.
pub fn floating_back(app: &Glyph) -> bool {
    app.view == View::Artist && !app.settings_open
}

/// Round back button over the photo plus a dark band under the status bar;
/// fixed while the page scrolls under it.
pub fn floating_back_bar(app: &Glyph, window: &Window, cx: &mut Context<Glyph>) -> AnyElement {
    let top = window.insets().safe_area.top;
    let fade = |a: f32| gpui::hsla(0.0, 0.0, 0.0, a);
    // Once the photo has scrolled away the band turns solid with the name,
    // so the page no longer runs visibly under the status bar and button.
    let photo_h = (f32::from(window.viewport_size().width) * 0.9).round();
    let scrolled = -f32::from(app.main_scroll.offset().y);
    let k = ((scrolled - (photo_h - HEADER_H - 40.0)) / 60.0).clamp(0.0, 1.0);
    let name = app.open_artist.clone().unwrap_or_default();
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .child(div().h(top + px(HEADER_H)).bg(gpui::linear_gradient(180.0, gpui::linear_color_stop(fade(0.55), 0.0), gpui::linear_color_stop(fade(0.0), 1.0))))
        .when(k > 0.0, |d| {
            d.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(top + px(HEADER_H - 8.0))
                    .pt(top)
                    .pl(px(56.0))
                    .pr(px(EDGE))
                    .flex()
                    .items_center()
                    .opacity(k)
                    .bg(hsla(0.0, 0.0, 0.06, 1.0))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(16.0)).font_weight(FontWeight::BOLD).text_color(theme::text()).child(name)),
            )
        })
        .child(
            div().absolute().top(top + px(6.0)).left(px(8.0)).child(
                icon_action("back-float", "arrow-left")
                    .bg(fade(0.45 * (1.0 - k)))
                    .on_click(cx.listener(|this, _, _, cx| back(this, cx))),
            ),
        )
        .into_any_element()
}

/// Marker in the data dir: the alpha notice was confirmed once.
const ALPHA_SEEN: &str = "alpha-notice-seen";

thread_local! {
    /// The alpha notice is up (checked against the marker on first render).
    static ALPHA_OPEN: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// First start on a phone: says this is an alpha, once.
pub fn alpha_notice(window: &Window, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    if !TOUCH {
        return None;
    }
    let open = ALPHA_OPEN.with(|o| match o.get() {
        Some(v) => v,
        None => {
            let v = !glyph_core::paths::data_dir().join(ALPHA_SEEN).exists();
            o.set(Some(v));
            v
        }
    });
    if !open {
        return None;
    }
    let w = (f32::from(window.viewport_size().width) - 32.0).min(420.0);
    let panel = div()
        .id("alpha-notice")
        .occlude()
        .w(px(w))
        .p(px(24.0))
        .rounded(px(16.0))
        .bg(gpui::hsla(0.0, 0.0, 0.09, 0.99))
        .flex()
        .flex_col()
        .gap(px(14.0))
        .font_family(theme::FONT)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(icon("info", 22.0, theme::pink()))
                .child(div().flex_1().text_size(px(20.0)).font_weight(FontWeight::BOLD).text_color(white(0.95)).child(t("mobile.alpha.title"))),
        )
        .child(div().text_size(px(15.0)).line_height(px(22.0)).text_color(white(0.72)).child(t("mobile.alpha.body")))
        .child(
            primary_button("alpha-ok", t("mobile.alpha.ok"))
                .h(px(48.0))
                .rounded(px(24.0))
                .on_click(cx.listener(|_, _, _, cx| {
                    ALPHA_OPEN.with(|o| o.set(Some(false)));
                    let _ = glyph_core::paths::write_atomic(&glyph_core::paths::data_dir().join(ALPHA_SEEN), b"1");
                    cx.notify();
                })),
        );
    Some(gpui::deferred(modal_layer(0.6, modal_backdrop("alpha-backdrop", 0.6).child(panel))).with_priority(6).into_any_element())
}

// ------------------------------------------------------------------ tabs

/// Bottom navigation: Home, Search, Library.
pub fn tab_bar(app: &mut Glyph, cx: &mut Context<Glyph>) -> impl IntoElement {
    let item = |id: &'static str, name: &'static str, view: View, label: SharedString, cx: &mut Context<Glyph>| {
        let active = app.view == view && !app.settings_open;
        let ink = if active { theme::text() } else { theme::text_faint() };
        div()
            .id(id)
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(2.0))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, w, cx| {
                if this.settings_open {
                    this.close_settings(w, cx);
                }
                this.navigate(view, w, cx)
            }))
            .child(icon(name, 20.0, ink))
            .child(div().text_size(px(10.0)).font_weight(if active { FontWeight::MEDIUM } else { FontWeight::NORMAL }).text_color(ink).child(label))
    };
    let home = item("tab-home", "house", View::Home, t("nav.home"), cx);
    let search = item("tab-search", "search", View::Search, t("nav.search"), cx);
    let lib = item("tab-lib", "library", View::Library, t("nav.library"), cx);
    theme::on_panel(|| div().flex().items_center().h(px(TABS_H)).child(home).child(search).child(lib))

}

// ------------------------------------------------------------------ mini player

/// Mini player: cover, title/artist and play/next. Tapping it (or swiping
/// up) opens Now Playing; swiping left/right skips, the song following the
/// finger.
pub fn mini_bar(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> Option<impl IntoElement> {
    let tr = app.current.clone()?;
    let playing = app.is_playing();
    let frac = app
        .duration()
        .filter(|d| !d.is_zero())
        .map(|d| (app.position().as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let art = cover(app, &tr, crate::ui::player_bar::COVER, 6.0, cx);
    let title = song_title(SharedString::from(tr.title.clone()), tr.explicit).text_size(theme::text_md()).font_weight(FontWeight::MEDIUM);
    let artist = div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tr.artist.clone());

    let pk = press_k("mini-play");
    let play_d = 34.0 - 5.0 * pk;
    let play = div()
        .id("mini-play")
        .flex_none()
        .size(px(44.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            press("mini-play");
            this.toggle_play(cx)
        }))
        .child(
            div()
                .size(px(play_d))
                .rounded_full()
                .bg(gpui::white())
                .flex()
                .items_center()
                .justify_center()
                .child(play_glyph(app.is_loading(), playing, 15.0 - 2.0 * pk, play_d)),
        );
    let next = icon_button("mini-next", "skip-forward", 17.0, theme::text())
        .size(px(44.0))
        .on_click(cx.listener(|this, _, _, cx| this.next(false, cx)));

    let (sx, _) = swipe_offset("mini", window);
    let on_swipe = cx.listener(|this, e: &ScrollWheelEvent, _, cx| match swipe_step("mini", e, |_, _| true) {
        Step::Pass => {}
        Step::Take => {
            cx.stop_propagation();
            cx.notify();
        }
        Step::Release(dx, dy) => {
            cx.stop_propagation();
            if horizontal(dx, dy) {
                swipe_skip(this, "mini", dx, cx);
            } else if dy < -SWIPE_ACT * 0.5 {
                open_now_playing(this, cx);
            }
            cx.notify();
        }
    });

    Some(theme::on_panel(|| {
        div()
            .on_scroll_wheel(on_swipe)
            .relative()
            .h(px(MINI_H))
            .bg(hsla(0.0, 0.0, 0.06, 0.94))
            .rounded(px(12.0))
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(8.0))
            .pl(px(8.0))
            .pr(px(4.0))
            .child(
                div()
                    .id("mini-open")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    // The swiped song slides under the buttons, not over them.
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| open_now_playing(this, cx)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .relative()
                            .left(px(sx))
                            .opacity((1.0 - sx.abs() / 220.0).max(0.15))
                            .child(art)
                            .child(div().flex().flex_col().min_w_0().gap(px(2.0)).child(title).child(artist)),
                    ),
            )
            .child(play)
            .child(next)
            // Thin progress line along the bottom edge.
            .child(
                div()
                    .absolute()
                    .left(px(8.0))
                    .right(px(8.0))
                    .bottom_0()
                    .h(px(2.0))
                    .rounded_full()
                    .bg(white(0.12))
                    .child(div().h_full().rounded_full().w(gpui::relative(frac)).bg(gpui::white())),
            )
    }))
}

// ------------------------------------------------------------------ profile menu

/// One row of a full-width touch list: icon, title, optional subtitle.
fn menu_row(id: impl Into<gpui::ElementId>, icon_name: &str, title: SharedString, sub: Option<SharedString>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(16.0))
        .min_h(px(52.0))
        .px(px(16.0))
        .py(px(8.0))
        .cursor_pointer()
        .active(|s| s.bg(white(0.06)))
        .child(icon(icon_name, 22.0, white(0.85)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().truncate().text_size(px(15.0)).text_color(gpui::white()).child(title))
                .when_some(sub, |d, sub| d.child(div().text_size(px(12.5)).text_color(white(0.55)).child(sub))),
        )
}

/// The profile menu, sliding in from the left over a dimmed page.
pub fn drawer(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let k = app.drawer_slide.update(app.drawer_open);
    if app.drawer_slide.running() {
        window.request_animation_frame();
    }
    if k <= 0.001 {
        swipe_reset("drawer");
        return None;
    }
    let safe = window.insets().safe_area;
    let w = (f32::from(window.viewport_size().width) * 0.86).min(360.0);
    // Swiped left: the menu follows the finger and closes past the threshold.
    let sx = swipe_offset("drawer", window).0.min(0.0);
    let close = |this: &mut Glyph, cx: &mut Context<Glyph>| {
        this.drawer_open = false;
        cx.notify();
    };

    let account = app.signed_in();
    let (name, sub) = match &account {
        Some(a) => (format!("@{}", a.account.handle), t("profile.view")),
        None => (t("mobile.signed_out").to_string(), t("account.signin.button")),
    };
    let handle = account.as_ref().map(|a| a.account.handle.clone());
    let pfp = crate::ui::profile::own_avatar(app, 52.0, cx);
    let profile = div()
        .id("drawer-profile")
        .flex()
        .items_center()
        .gap(px(14.0))
        .px(px(16.0))
        .py(px(14.0))
        .cursor_pointer()
        .active(|s| s.bg(white(0.06)))
        .on_click(cx.listener(move |this, _, _, cx| {
            close(this, cx);
            match &handle {
                Some(h) => this.open_profile(h, cx),
                None => this.open_settings(Some(SettingsPage::Account), cx),
            }
        }))
        .child(pfp)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(px(2.0))
                .child(div().truncate().text_size(px(19.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child(name))
                .child(div().text_size(px(13.0)).text_color(white(0.6)).child(sub)),
        );

    let has_downloads = app.backend.as_ref().is_some_and(|b| !b.downloaded_tracks().is_empty());
    let mut rows: Vec<AnyElement> = Vec::new();
    rows.push(menu_row("dr-stats", "chart-column", t("stats.title"), None).on_click(cx.listener(move |this, _, _, cx| {
        close(this, cx);
        this.open_stats(cx)
    })).into_any_element());
    rows.push(menu_row("dr-recent", "history", t("home.recent"), None).on_click(cx.listener(move |this, _, _, cx| {
        close(this, cx);
        this.open_list(ListRef::Recent, cx)
    })).into_any_element());
    if has_downloads {
        rows.push(menu_row("dr-dl", "arrow-down-to-line", t("library.downloads"), None).on_click(cx.listener(move |this, _, _, cx| {
            close(this, cx);
            this.open_list(ListRef::Downloads, cx)
        })).into_any_element());
    }
    rows.push(menu_row("dr-migrate", "arrow-right-left", t("library.migrate"), None).on_click(cx.listener(move |this, _, _, cx| {
        close(this, cx);
        this.open_migrate(cx)
    })).into_any_element());
    rows.push(menu_row("dr-settings", "settings", t("nav.settings"), None).on_click(cx.listener(move |this, _, _, cx| {
        close(this, cx);
        this.open_settings(None, cx)
    })).into_any_element());

    let panel = div()
        .id("drawer")
        .occlude()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(-(1.0 - k) * w + sx))
        .w(px(w))
        .bg(surface())
        .pt(safe.top + px(8.0))
        .pb(safe.bottom)
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(profile)
        .child(div().h(px(1.0)).mx(px(16.0)).mb(px(6.0)).bg(white(0.1)))
        .children(rows);

    Some(
        div()
            .id("drawer-backdrop")
            .occlude()
            .absolute()
            .inset_0()
            .bg(hsla(0.0, 0.0, 0.0, 0.55 * k * (1.0 + sx / w).max(0.0)))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| close(this, cx)))
            .on_scroll_wheel(cx.listener(move |this, e: &ScrollWheelEvent, _, cx| {
                cx.stop_propagation();
                match swipe_step("drawer", e, horizontal) {
                    Step::Pass => {}
                    Step::Take => cx.notify(),
                    Step::Release(dx, _) => {
                        if dx < -SWIPE_ACT {
                            swipe_hold("drawer");
                            close(this, cx);
                        }
                        cx.notify();
                    }
                }
            }))
            .child(panel)
            .into_any_element(),
    )
}

// ------------------------------------------------------------------ settings

/// Categories in the phone settings list, in the desktop nav's order.
fn settings_pages(signed_in: bool) -> Vec<SettingsPage> {
    let mut v = vec![SettingsPage::Account];
    if signed_in {
        v.push(SettingsPage::Profile);
    }
    v.extend([
        SettingsPage::Playback,
        SettingsPage::Audio,
        SettingsPage::Appearance,
        SettingsPage::Storage,
        SettingsPage::Migrate,
        SettingsPage::Transfer,
        SettingsPage::Server,
        // Integrations (Discord, MPD, MCP) talk to apps on a computer.
        SettingsPage::About,
    ]);
    v
}

/// One quick action tile: icon, label, state line; tap toggles.
fn quick_tile(id: &'static str, icon_name: &str, label: SharedString, on: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .h(px(84.0))
        .rounded(px(10.0))
        .bg(if on { white(0.14) } else { white(0.07) })
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(4.0))
        .px(px(6.0))
        .cursor_pointer()
        .active(|s| s.bg(white(0.18)))
        .child(icon(icon_name, 22.0, if on { theme::pink() } else { white(0.8) }))
        .child(div().text_size(px(12.0)).text_color(gpui::white()).truncate().child(label))
        .child(div().text_size(px(11.0)).text_color(white(0.55)).child(if on { t("common.on") } else { t("common.off") }))
}

/// Full-screen settings: a category list with quick actions on top, or the
/// open category (the desktop page content) with its sub-tabs as chips.
pub fn settings(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let safe = window.insets().safe_area;
    let in_page = app.settings_in_page;
    let page = app.settings_page;
    let title = if in_page { crate::ui::settings::page_info(page).0 } else { t("nav.settings") };

    let back = div()
        .id("settings-back")
        .flex_none()
        .size(px(44.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .active(|s| s.bg(white(0.08)))
        .on_click(cx.listener(|this, _, w, cx| {
            if this.settings_in_page {
                this.settings_in_page = false;
                this.open_select = None;
                cx.notify();
            } else {
                this.close_settings(w, cx);
            }
        }))
        .child(icon("arrow-left", 22.0, gpui::white()));
    let head = div()
        .flex_none()
        .h(px(HEADER_H))
        .px(px(6.0))
        .flex()
        .items_center()
        .child(back)
        .child(div().flex_1().flex().justify_center().text_size(px(17.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child(title))
        .child(div().flex_none().size(px(44.0)));

    let body: AnyElement = if in_page {
        let tabs = crate::ui::settings::subtabs(app, page);
        let chips = (!tabs.is_empty()).then(|| {
            let sel = app.settings_sub as usize;
            div().flex().flex_wrap().gap(px(8.0)).pb(px(12.0)).children(tabs.iter().enumerate().map(|(i, key)| {
                div()
                    .id(("set-sub", i))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_sub = i as u8;
                        this.open_select = None;
                        cx.notify();
                    }))
                    // The off chip's own fill hardly shows on the settings
                    // surface; a lighter one keeps the tabs visible.
                    .child(chip(t(key), i == sel).when(i != sel, |c| c.bg(white(0.1))))
            }))
        });
        let content = crate::ui::settings::page_body(app, cx);
        div()
            .id("settings-page")
            .track_scroll(&app.settings_scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll().touch_axis()
            .px(px(16.0))
            .pb(safe.bottom + px(24.0))
            .children(chips)
            .child(content)
            .into_any_element()
    } else {
        let eq_on = app.config.audio.eq_enabled;
        let saver = app.config.storage.data_saver;
        let explicit = app.config.audio.hide_explicit;
        let quick = div()
            .flex()
            .gap(px(8.0))
            .px(px(16.0))
            .pb(px(10.0))
            .child(quick_tile("q-saver", "chart-column", t("sp.storage.saver"), saver).on_click(cx.listener(move |this, _, _, cx| this.set_data_saver(!saver, cx))))
            .child(quick_tile("q-eq", "sliders-horizontal", t("sp.eq"), eq_on).on_click(cx.listener(move |this, _, _, cx| {
                let g = crate::app::eq_gains(&this.config.audio.eq_gains);
                let p = this.config.audio.eq_preset.clone();
                this.set_eq(!eq_on, &p, g, true, cx)
            })))
            .child(quick_tile("q-explicit", "eye-off", t("sp.explicit"), explicit).on_click(cx.listener(move |this, _, _, cx| {
                this.config.audio.hide_explicit = !explicit;
                this.save_config(cx);
                cx.notify();
            })));
        let rows: Vec<AnyElement> = settings_pages(app.signed_in().is_some())
            .into_iter()
            .map(|p| {
                let (name, sub, icon_name) = crate::ui::settings::page_info(p);
                menu_row(("set-page", p as usize), icon_name, name, Some(sub))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if p != this.settings_page {
                            this.settings_sub = 0;
                        }
                        this.settings_page = p;
                        this.settings_in_page = true;
                        this.open_select = None;
                        this.settings_scroll.set_offset(gpui::point(px(0.0), px(0.0)));
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .id("settings-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll().touch_axis()
            .pb(safe.bottom + px(24.0))
            .child(quick)
            .children(rows)
            .into_any_element()
    };

    theme::on_panel(|| {
        div()
            .id("settings-screen")
            .occlude()
            .absolute()
            .inset_0()
            .bg(surface())
            .pt(safe.top)
            .flex()
            .flex_col()
            .font_family(theme::FONT)
            .child(head)
            .child(body)
            .into_any_element()
    })
}

// ------------------------------------------------------------------ library

/// The library as a list: Liked Songs, playlists, saved albums and followed
/// artists, filtered by chips.
pub fn library(app: &mut Glyph, _window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let filter = app.lib_filter;
    let chip_btn = |id: &'static str, label: SharedString, f: u8, cx: &mut Context<Glyph>| {
        div()
            .id(id)
            .flex_none()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.lib_filter = if this.lib_filter == f { 0 } else { f };
                cx.notify();
            }))
            .child(chip(label, filter == f))
    };
    let chips = div()
        .flex()
        .gap(px(8.0))
        .px(px(EDGE))
        .pb(px(10.0))
        .child(chip_btn("lf-pl", t("library.filter.playlists"), 1, cx))
        .child(chip_btn("lf-al", t("search.filter.albums"), 2, cx))
        .child(chip_btn("lf-ar", t("search.filter.artists"), 3, cx));

    // (list, subtitle)
    let mut lists: Vec<(ListRef, String)> = Vec::new();
    if filter <= 1 {
        let pl = t("list.kind.playlist").to_string();
        lists.push((ListRef::Liked, pl.clone()));
        for p in &app.library.playlists {
            lists.push((ListRef::User(p.id.clone()), pl.clone()));
        }
    }
    if filter == 0 || filter == 2 {
        for k in &app.library.saved_albums {
            let l = ListRef::from_key(k);
            let by = match &l {
                ListRef::Album { artist, .. } => format!("{} · {artist}", t("list.kind.album")),
                _ => t("list.kind.album").to_string(),
            };
            lists.push((l, by));
        }
    }
    let pin_rank = |l: &ListRef| app.library.pinned.iter().position(|p| *p == l.key()).unwrap_or(usize::MAX);
    lists.sort_by_key(|(l, _)| pin_rank(l));

    let mut rows: Vec<AnyElement> = Vec::new();
    for (i, (l, kind)) in lists.into_iter().enumerate() {
        let art = list_cover(app, &l, 56.0, cx);
        let title = app.list_title(&l);
        let n = app.list_tracks(&l).len();
        let pinned = app.is_pinned(&l);
        let sub = if matches!(l, ListRef::Album { .. }) { kind } else { format!("{kind} · {}", tf("common.n_songs", &[("n", &n)])) };
        rows.push(
            lib_row(("lib-row", i), art, title, sub, pinned)
                .on_click(cx.listener(move |this, _, _, cx| this.open_list(l.clone(), cx)))
                .into_any_element(),
        );
    }
    if filter == 0 || filter == 3 {
        for (i, f) in app.library.followed.clone().into_iter().enumerate() {
            let art: AnyElement = match &f.track {
                Some(tr) => cover(app, tr, 56.0, 28.0, cx).into_any_element(),
                None => div().size(px(56.0)).rounded_full().bg(theme::card()).flex().items_center().justify_center().child(icon("user-round", 22.0, white(0.5))).into_any_element(),
            };
            let name = f.name.clone();
            rows.push(
                lib_row(("lib-artist", i), art, f.name.clone(), t("search.kind.artist").to_string(), false)
                    .on_click(cx.listener(move |this, _, _, cx| this.open_artist(&name, cx)))
                    .into_any_element(),
            );
        }
    }

    div()
        .id("lib-scroll")
        .track_scroll(&app.main_scroll)
        .size_full()
        .overflow_y_scroll().touch_axis()
        .flex()
        .flex_col()
        .child(chips)
        .children(rows)
        .child(div().h(px(24.0)))
        .into_any_element()
}

/// Library row: 56 px art, title, "Playlist · 12 songs".
fn lib_row(id: impl Into<gpui::ElementId>, art: AnyElement, title: String, sub: String, pinned: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(12.0))
        .px(px(EDGE))
        .py(px(6.0))
        .cursor_pointer()
        .active(|s| s.bg(white(0.05)))
        .child(art)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(div().truncate().text_size(px(15.0)).text_color(theme::text()).child(title))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .when(pinned, |d| d.child(icon("pin-fill", 12.0, theme::pink())))
                        .child(div().truncate().text_size(px(12.5)).text_color(theme::text_dim()).child(sub)),
                ),
        )
}

/// Header actions of the Library tab: search and new playlist.
pub fn library_actions(cx: &mut Context<Glyph>) -> Vec<AnyElement> {
    vec![
        icon_action("lib-search", "search").on_click(cx.listener(|this, _, w, cx| this.navigate(View::Search, w, cx))).into_any_element(),
        icon_action("lib-new", "plus").on_click(cx.listener(|this, _, w, cx| this.open_name_dialog(Vec::new(), None, w, cx))).into_any_element(),
    ]
}

// ------------------------------------------------------------------ song list

/// A song list (playlist, album, Liked Songs, Recent) like Spotify's: back,
/// title and song count, download on the left, shuffle and a big play button
/// on the right, then one row per song.
pub fn list_view(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let Some(l) = app.open_list.clone() else { return div().into_any_element() };
    let tracks = app.shown_tracks(&l);
    let title = app.list_title(&l);
    let n = tracks.len();
    let vw = f32::from(window.viewport_size().width);
    let art_size = (vw * 0.62).clamp(140.0, 280.0);
    let art = list_cover(app, &l, art_size, cx);

    let here = app.current.as_ref().is_some_and(|c| {
        let k = crate::app::track_key(c);
        tracks.iter().any(|t| crate::app::track_key(t) == k)
    });
    let (playing, loading) = (here && app.is_playing(), here && app.is_loading());
    let has_dl = app.backend.is_some() && n > 0;
    let all_dl = has_dl && tracks.iter().all(|tr| app.is_downloaded(tr));

    let (t_play, t_shuf, t_dl) = (tracks.clone(), tracks.clone(), tracks.clone());
    let play = div()
        .id("cl-play")
        .flex_none()
        .size(px(52.0))
        .rounded_full()
        .bg(theme::pink())
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            if here {
                this.toggle_play(cx)
            } else if !t_play.is_empty() {
                this.play_list(t_play.clone(), 0, cx)
            }
        }))
        .child(play_glyph(loading, playing, 20.0, 52.0));
    let actions = div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .when(has_dl, |d| {
            d.child(
                icon_action("cl-dl", "arrow-down-to-line")
                    .when(all_dl, |d| d.bg(white(0.1)))
                    .on_click(cx.listener(move |this, _, _, cx| this.download_tracks(t_dl.clone(), cx))),
            )
        })
        .child(div().flex_1())
        // An empty list has nothing to play.
        .when(n > 0, |d| {
            d.child(icon_action("cl-shuffle", "shuffle").on_click(cx.listener(move |this, _, _, cx| this.play_shuffled(t_shuf.clone(), cx))))
                .child(play)
        });

    let head = div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .px(px(EDGE))
        // Cover, then title + count as one block, then the actions.
        .child(div().flex().justify_center().pt(px(8.0)).pb(px(16.0)).child(art))
        .child(div().text_size(px(24.0)).font_weight(FontWeight::BOLD).text_color(theme::text()).child(title))
        .child(div().text_size(px(13.0)).text_color(theme::text_dim()).child(tf("common.n_songs", &[("n", &n)])))
        .child(actions.mt(px(8.0)));

    let list = std::rc::Rc::new(tracks);
    let scroll = app.main_scroll.clone();
    let rows = virtual_rows("cl-rows", list.len(), 60.0, &scroll, |i| {
        let tr = list[i].clone();
        let now = app.current.as_ref().map(crate::app::track_key) == Some(crate::app::track_key(&tr));
        let all = list.clone();
        let t_queue = tr.clone();
        let row = div()
            .id(("cl-row", i))
            .h(px(60.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(EDGE))
            .cursor_pointer()
            .active(|s| s.bg(white(0.05)))
            .on_click(cx.listener(move |this, _, _, cx| this.play_list((*all).clone(), i, cx)))
            .child(cover(app, &tr, 48.0, 4.0, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(song_title(SharedString::from(tr.title.clone()), tr.explicit).text_size(px(15.0)).text_color(if now { theme::pink() } else { theme::text() }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .when(app.is_downloaded(&tr), |d| d.child(icon("arrow-down-to-line", 12.0, theme::pink())))
                            .child(div().truncate().text_size(px(12.5)).text_color(theme::text_dim()).child(tr.artist.clone())),
                    ),
            );
        queue_swipe(row, format!("q-cl-{i}"), t_queue, cx).into_any_element()
    });

    div()
        .id("cl-scroll")
        .track_scroll(&app.main_scroll)
        .size_full()
        .overflow_y_scroll().touch_axis()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(head)
        .when(n == 0, |d| d.child(div().px(px(EDGE)).pt(px(8.0)).text_size(px(14.0)).text_color(theme::text_dim()).child(t("mobile.list.empty"))))
        .child(rows)
        .child(div().h(px(24.0)))
        .into_any_element()
}

// ------------------------------------------------------------------ search

/// Top of the Search tab: page title + field, or (while typing) a back arrow
/// beside the field like Spotify.
pub fn search_bar(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let active = app.search_input.read(cx).focus.is_focused(window) || !app.search.query.trim().is_empty();
    let field = crate::ui::chrome::search_field(app, window, cx, None).into_any_element();
    if !active {
        let head = header(app, Some(t("nav.search")), Vec::new(), cx).into_any_element();
        return div().flex().flex_col().child(head).child(div().px(px(EDGE)).pb(px(8.0)).child(field)).into_any_element();
    }
    div()
        .flex_none()
        .h(px(HEADER_H))
        .pl(px(4.0))
        .pr(px(EDGE))
        .flex()
        .items_center()
        .gap(px(4.0))
        .child(icon_action("search-back", "arrow-left").on_click(cx.listener(|this, _, w, cx| {
            this.search_input.update(cx, |i, cx| i.set_value(String::new(), cx));
            this.search.query.clear();
            let f = this.focus.clone();
            w.focus(&f, cx);
            cx.notify();
        })))
        .child(div().flex_1().min_w_0().child(field))
        .into_any_element()
}

/// Empty query: recent searches (or recently played) as a full-width list,
/// each row with an always-visible remove button.
pub fn search_recent(app: &mut Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    let (is_history, items) = app.search_suggestions();
    if items.is_empty() {
        return div().pt(px(24.0)).text_size(px(14.0)).text_color(theme::text_dim()).child(t("search.empty.sub")).into_any_element();
    }
    let rows: Vec<AnyElement> = items
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            let (art, title, sub): (AnyElement, String, String) = match &item {
                glyph_core::SearchItem::Song { track } => (
                    cover(app, track, 52.0, 4.0, cx).into_any_element(),
                    track.title.clone(),
                    format!("{} • {}", t("search.kind.song"), track.artist),
                ),
                glyph_core::SearchItem::Artist { name, track } => (
                    match track {
                        Some(tr) => cover(app, tr, 52.0, 26.0, cx).into_any_element(),
                        None => div()
                            .size(px(52.0))
                            .rounded_full()
                            .bg(white(0.08))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("user-round", 22.0, white(0.6)))
                            .into_any_element(),
                    },
                    name.clone(),
                    t("search.kind.artist").to_string(),
                ),
            };
            let pick = item.clone();
            div()
                .id(("recent-search", i))
                .h(px(64.0))
                .flex()
                .items_center()
                .gap(px(12.0))
                .cursor_pointer()
                .active(|s| s.bg(white(0.05)))
                .on_click(cx.listener(move |this, _, w, cx| match &pick {
                    glyph_core::SearchItem::Song { track } => this.play_suggestion(track.clone(), w, cx),
                    glyph_core::SearchItem::Artist { name, track } => this.pick_search_artist(name, track.clone(), w, cx),
                }))
                .child(div().flex_none().child(art))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(div().truncate().text_size(px(15.0)).text_color(theme::text()).child(title))
                        .child(div().truncate().text_size(px(12.5)).text_color(theme::text_dim()).child(sub)),
                )
                .when(is_history, |d| {
                    d.child(div().id(("recent-x", i)).flex_none().size(px(40.0)).flex().items_center().justify_center().rounded_full().active(|s| s.bg(white(0.08))).child(icon("x", 20.0, theme::text_dim())).on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.remove_search_history(i, cx);
                    })))
                })
                .into_any_element()
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .pb(px(6.0))
                .text_size(px(18.0))
                .font_weight(FontWeight::BOLD)
                .text_color(theme::text())
                .child(if is_history { t("search.history") } else { t("search.recently_played") }),
        )
        .children(rows)
        .into_any_element()
}

// ------------------------------------------------------------------ song row

/// Phone song row (search, artist pages, sections, lists): cover, title and
/// artist get the width; a heart when liked and the ⋮ menu on the right.
pub fn song_row(app: &mut Glyph, list: std::rc::Rc<Vec<glyph_protocol::Track>>, i: usize, prefix: &'static str, from: Option<(ListRef, usize)>, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    let tr = list[i].clone();
    let now = app.current.as_ref().map(crate::app::track_key) == Some(crate::app::track_key(&tr));
    let liked = app.is_liked(&tr);
    let blocked = !app.allowed(&tr);
    let (t_like, t_menu, t_queue, from_menu) = (tr.clone(), tr.clone(), tr.clone(), from.clone());
    let all = list.clone();
    let row = div()
        .id((prefix, i))
        .h(px(56.0))
        .flex()
        .items_center()
        .gap(px(12.0))
        .cursor_pointer()
        .when(blocked, |d| d.opacity(0.4))
        .active(|s| s.bg(white(0.05)))
        .on_click(cx.listener(move |this, _, _, cx| this.play_list((*all).clone(), i, cx)))
        .child(cover(app, &tr, 48.0, 4.0, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(song_title(SharedString::from(tr.title.clone()), tr.explicit).text_size(px(15.0)).text_color(if now { theme::pink() } else { theme::text() }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .min_w_0()
                        .when(app.is_downloaded(&tr), |d| d.child(icon("arrow-down-to-line", 12.0, theme::pink())))
                        .child(div().truncate().text_size(px(12.5)).text_color(theme::text_dim()).child(tr.artist.clone())),
                ),
        )
        .when(liked, |d| {
            d.child(
                div()
                    .id((prefix, i * 4 + 1))
                    .flex_none()
                    .size(px(36.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_like(&t_like, cx)
                    }))
                    .child(icon("heart", 18.0, theme::pink())),
            )
        })
        .child(
            div()
                .id((prefix, i * 4 + 2))
                .flex_none()
                .size(px(36.0))
                .mr(px(-8.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .active(|s| s.bg(white(0.08)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_ctx(t_menu.clone(), e.position, from_menu.clone(), cx)
                    }),
                )
                .child(icon("ellipsis-vertical", 18.0, theme::text_dim())),
        );
    queue_swipe(row, format!("q-{prefix}-{i}"), t_queue, cx)
}
