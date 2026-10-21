//! Phone Now Playing, as Spotify's: a sheet over the page ([`sheet`]) that
//! slides up from the mini player and follows the finger down to close, the
//! page showing above it ([`under_sheet`]). Swiping the cover skips, the
//! neighbouring covers sliding in. The lyrics open in their own full screen
//! ([`lyrics_view`]); the queue and Connect are bottom sheets
//! ([`bottom_sheet`]), which the mini player's strip opens too.
//!
//! The page underneath stays the current view, so Now Playing never enters
//! the back history; `View::NowPlaying` from shared code (shortcuts, the
//! player bar) is turned into the sheet ([`adopt_view`]).

use super::*;
use crate::app::Slide;
use gpui::prelude::*;
use std::cell::RefCell;
use std::time::Duration;

/// Page margin; also the gap between the carousel's covers, so the
/// neighbours wait just off screen.
const SIDE: f32 = 24.0;

#[derive(Clone, Copy, PartialEq)]
enum Sheet {
    Queue,
    Devices,
}

struct State {
    open: bool,
    slide: Slide,
    scroll: gpui::ScrollHandle,
    /// Where the app was when the sheet opened: navigating away (a menu's
    /// "Go to album") closes it.
    route: Option<(View, usize)>,
    lyrics: bool,
    lyrics_slide: Slide,
    /// The bottom sheet shown last (kept while it slides out).
    sheet: Sheet,
    sheet_slide: Slide,
}

thread_local! {
    static NP: RefCell<State> = RefCell::new(State {
        open: false,
        slide: Slide::default(),
        scroll: gpui::ScrollHandle::new(),
        route: None,
        lyrics: false,
        lyrics_slide: Slide::default(),
        sheet: Sheet::Queue,
        sheet_slide: Slide::default(),
    });
}

// ------------------------------------------------------------------ state

/// Opens Now Playing over the current page.
pub fn open(app: &Glyph, cx: &mut Context<Glyph>) {
    NP.with(|n| {
        let mut n = n.borrow_mut();
        n.open = true;
        n.route = Some((app.view, app.history.len()));
        n.scroll.set_offset(gpui::point(px(0.0), px(0.0)));
    });
    swipe_reset("np-pull");
    cx.notify();
}

fn close(cx: &mut Context<Glyph>) {
    NP.with(|n| {
        let mut n = n.borrow_mut();
        n.open = false;
        n.lyrics = false;
    });
    cx.notify();
}

fn set_lyrics(on: bool, cx: &mut Context<Glyph>) {
    NP.with(|n| n.borrow_mut().lyrics = on);
    swipe_reset("lyr-pull");
    cx.notify();
}

fn close_sheet(app: &mut Glyph, cx: &mut Context<Glyph>) {
    app.show_queue = false;
    app.show_devices = false;
    cx.notify();
}

/// Now Playing is open (or opening).
pub fn is_open() -> bool {
    NP.with(|n| n.borrow().open)
}

/// `View::NowPlaying` becomes the sheet over the page it was opened from.
pub fn adopt_view(app: &mut Glyph, cx: &mut Context<Glyph>) {
    if app.view != View::NowPlaying {
        return;
    }
    back(app, cx);
    if app.view == View::NowPlaying {
        app.view = View::Home;
    }
    open(app, cx);
}

/// The system back gesture: closes the top-most of the bottom sheet, the
/// lyrics and Now Playing. `false` when none was open.
pub fn back_pressed(app: &mut Glyph, cx: &mut Context<Glyph>) -> bool {
    if app.show_queue || app.show_devices {
        close_sheet(app, cx);
        return true;
    }
    let (open, lyrics) = NP.with(|n| {
        let n = n.borrow();
        (n.open, n.lyrics)
    });
    if open && lyrics {
        set_lyrics(false, cx);
    } else if open {
        close(cx);
    }
    open
}

/// Top edge of the sheet (0 while it covers the screen), `None` while it is
/// closed.
pub fn sheet_top(app: &Glyph, window: &mut Window) -> Option<f32> {
    let vh = f32::from(window.viewport_size().height);
    let route = (app.view, app.history.len());
    let (k, running) = NP.with(|n| {
        let mut n = n.borrow_mut();
        if n.open && (app.current.is_none() || n.route.as_ref().is_some_and(|r| *r != route)) {
            n.open = false;
            n.lyrics = false;
        }
        let open = n.open;
        (n.slide.update(open), n.slide.running())
    });
    if running {
        window.request_animation_frame();
    }
    if k <= 0.001 {
        swipe_reset("np-pull");
        return None;
    }
    let (_, py) = swipe_offset("np-pull", window);
    Some((1.0 - k) * vh + py.max(0.0))
}

/// The page, shown only above the sheet's top edge and dimmed the further
/// the sheet is up (nothing while it covers the screen).
pub fn under_sheet(page: impl IntoElement, top: f32, window: &Window) -> AnyElement {
    if top < 0.5 {
        return div().into_any_element();
    }
    let size = window.viewport_size();
    let dim = 0.5 * (1.0 - top / f32::from(size.height)).clamp(0.0, 1.0);
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(top))
        .overflow_hidden()
        .child(div().absolute().top_0().left_0().w(size.width).h(size.height).child(page))
        .child(div().absolute().inset_0().bg(hsla(0.0, 0.0, 0.0, dim)))
        .into_any_element()
}

/// Everything over the page: the sheet, the lyrics and the bottom sheet.
pub fn layers(app: &mut Glyph, top: Option<f32>, window: &mut Window, cx: &mut Context<Glyph>) -> Vec<AnyElement> {
    let mut v = Vec::new();
    if let Some(top) = top {
        v.extend(sheet(app, top, window, cx));
    }
    v.extend(lyrics_view(app, window, cx));
    v.extend(bottom_sheet(app, window, cx));
    v
}

// ------------------------------------------------------------------ gestures

/// Pull down to close: claims a downward pan (with `scroll`, only while that
/// frame is at its top); past the threshold `on_close` runs and the element
/// stays where the finger left it while it slides away.
fn pull<'a>(key: &'static str, scroll: Option<gpui::ScrollHandle>, on_close: fn(&mut Glyph, &mut Context<Glyph>), cx: &'a mut Context<Glyph>) -> impl Fn(&ScrollWheelEvent, &mut Window, &mut gpui::App) + 'static + use<'a> {
    cx.listener(move |this, e: &ScrollWheelEvent, _, cx| {
        let top = scroll.as_ref().is_none_or(|s| s.offset().y >= px(-1.0));
        match swipe_step(key, e, |dx, dy| top && dy > 0.0 && !horizontal(dx, dy)) {
            Step::Pass => {}
            Step::Take => {
                cx.stop_propagation();
                cx.notify();
            }
            Step::Release(_, dy) => {
                cx.stop_propagation();
                if dy > SWIPE_ACT * 1.5 {
                    swipe_hold(key);
                    on_close(this, cx);
                }
                cx.notify();
            }
        }
    })
}

// ------------------------------------------------------------------ pieces

/// Where music plays: icon, name, and whether that is another device.
fn device(app: &Glyph) -> (&'static str, SharedString, bool) {
    if let Some((icon_name, name)) = crate::ui::connect::playing_on(app) {
        return (icon_name, name.into(), true);
    }
    let name = app.config.audio.output_device.clone().map(SharedString::from).unwrap_or_else(crate::ui::connect::this_device);
    ("smartphone", name, false)
}

/// Seek target for a bar: a fraction of the song.
fn seek_pick(cx: &mut Context<Glyph>) -> Pick {
    let weak = cx.entity().downgrade();
    std::rc::Rc::new(move |f, _, cx| {
        weak.update(cx, |this, cx| {
            if let Some(d) = this.duration() {
                this.seek_to(Duration::from_secs_f32(d.as_secs_f32() * f));
                cx.notify();
            }
        })
        .ok();
    })
}

/// Seek bar with the times under it.
fn seek_row(app: &mut Glyph, id: &'static str, width: f32, cx: &mut Context<Glyph>) -> gpui::Div {
    let pos = app.position();
    let dur = app.duration();
    let frac = dur.filter(|d| !d.is_zero()).map(|d| (pos.as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0)).unwrap_or(0.0);
    let seek = seek_pick(cx);
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(thin_bar(id, width, frac, seek, None, cx))
        .child(
            div()
                .flex()
                .justify_between()
                .text_size(px(11.5))
                .text_color(theme::text_faint())
                .child(fmt_time(pos))
                .child(dur.map(fmt_time).unwrap_or_default()),
        )
}

/// Round white play button.
fn play_button(app: &Glyph, id: &'static str, size: f32, cx: &mut Context<Glyph>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(size))
        .rounded_full()
        .bg(gpui::white())
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            // Not also the bar it sits in (that scrolls to the top).
            cx.stop_propagation();
            this.toggle_play(cx)
        }))
        .child(play_glyph(app.is_loading(), app.is_playing(), size * 0.38, size))
}

/// Card under the transport (lyrics, up next): title, optional action.
fn card(id: &'static str, title: SharedString, action: Option<AnyElement>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(px(10.0))
        .p(px(16.0))
        .rounded(px(12.0))
        .bg(white(0.07))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_size(px(16.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child(title))
                .children(action),
        )
}

/// Left/right on the cover skips; the cover follows the finger and the
/// previous / next covers slide in beside it.
fn carousel(app: &mut Glyph, tr: &glyph_protocol::Track, art: f32, window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let step = art + SIDE;
    let prev = app.previous_track();
    let next = app.upcoming().into_iter().next();
    let (ax, _) = swipe_offset("np-art", window);
    // Nothing on that side: the cover only gives a little.
    let ax = if (ax > 0.0 && prev.is_none()) || (ax < 0.0 && next.is_none()) { ax * 0.35 } else { ax };
    let at = |x: f32| div().absolute().top_0().left(px(x));
    let prev_art = prev.map(|p| at(ax - step).child(cover(app, &p, art, 8.0, cx)));
    let next_art = next.map(|n| at(ax + step).child(cover(app, &n, art, 8.0, cx)));
    div()
        .relative()
        .flex_none()
        .size(px(art))
        .on_scroll_wheel(cx.listener(move |this, e: &ScrollWheelEvent, _, cx| match swipe_step("np-art", e, horizontal) {
            Step::Pass => {}
            Step::Take => {
                cx.stop_propagation();
                cx.notify();
            }
            Step::Release(dx, _) => {
                cx.stop_propagation();
                let can = if dx < 0.0 { !this.upcoming().is_empty() } else { this.previous_track().is_some() };
                if dx.abs() >= SWIPE_ACT && can {
                    if dx < 0.0 {
                        this.next(false, cx);
                    } else {
                        this.previous_song(cx);
                    }
                    // The neighbour that came in becomes the cover in place.
                    swipe_settle_from("np-art", (dx - dx.signum() * step, 0.0));
                }
                cx.notify();
            }
        }))
        .children(prev_art)
        .child(at(ax).child(cover(app, tr, art, 8.0, cx)))
        .children(next_art)
        .into_any_element()
}

/// Top of the sheet: close, "Playing from …", the song's menu. Scrolled past
/// the transport, a bar like the mini player slides in over it (tap: back
/// to the top).
fn header(app: &mut Glyph, tr: &glyph_protocol::Track, sk: f32, scroll: &gpui::ScrollHandle, safe_top: gpui::Pixels, cx: &mut Context<Glyph>) -> AnyElement {
    let title = match app.queue_from.clone() {
        Some(from) => div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .items_center()
            .child(div().text_size(px(11.5)).text_color(theme::text_dim()).child(t("np.playing_from")))
            .child(div().max_w_full().truncate().text_size(px(13.5)).font_weight(FontWeight::BOLD).text_color(theme::text()).child(from)),
        None => div().flex_1().min_w_0().flex().justify_center().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(t("nav.now_playing")),
    };
    let t_menu = tr.clone();
    // One header at a time: the normal one fades out over the first half,
    // the mini-player bar in over the second, so their texts never overlap.
    let normal = (sk < 0.5).then(|| {
        div()
            .absolute()
            .inset_0()
            .px(px(4.0))
            .flex()
            .items_center()
            .gap(px(4.0))
            .opacity(1.0 - sk * 2.0)
            .child(icon_action("np-close", "chevron-down").on_click(cx.listener(|_, _, _, cx| close(cx))))
            .child(title)
            .child(icon_action("np-more", "ellipsis-vertical").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_ctx(t_menu.clone(), e.position, None, cx)
                }),
            ))
    });

    let bar = (sk >= 0.5).then(|| {
        let frac = app.duration().filter(|d| !d.is_zero()).map(|d| (app.position().as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0)).unwrap_or(0.0);
        let (dev_icon, dev_name, remote) = device(app);
        let liked = app.is_liked(tr);
        let t_like = tr.clone();
        let to_top = scroll.clone();
        let sub = if remote {
            div().flex().items_center().gap(px(5.0)).min_w_0().child(icon(dev_icon, 12.0, theme::pink())).child(div().truncate().text_size(px(12.0)).text_color(theme::pink()).child(dev_name))
        } else {
            div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(tr.artist.clone())
        };
        div()
            .id("np-bar")
            .absolute()
            .left_0()
            .right_0()
            .top(px(-12.0 * (2.0 - sk * 2.0)))
            .h_full()
            .opacity(sk * 2.0 - 1.0)
            .pl(px(EDGE))
            .pr(px(8.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .cursor_pointer()
            .on_click(move |_, _, _| to_top.set_offset(gpui::point(px(0.0), px(0.0))))
            .child(cover(app, tr, 40.0, 4.0, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(song_title(SharedString::from(tr.title.clone()), tr.explicit).text_size(theme::text_md()).font_weight(FontWeight::MEDIUM))
                    .child(sub),
            )
            .child(
                toggle_button("np-bar-like", "heart", 20.0, liked, Some(theme::pink()))
                    .size(px(40.0))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_like(&t_like, cx)
                    })),
            )
            .child(play_button(app, "np-bar-play", 34.0, cx))
            .child(
                div()
                    .absolute()
                    .left(px(EDGE))
                    .right(px(EDGE))
                    .bottom_0()
                    .h(px(2.0))
                    .rounded_full()
                    .bg(white(0.12))
                    .child(div().h_full().rounded_full().w(gpui::relative(frac)).bg(gpui::white())),
            )
    });

    div()
        .flex_none()
        .pt(safe_top)
        .bg(hsla(0.0, 0.0, 0.06, 0.94 * sk))
        .on_scroll_wheel(pull("np-pull", None, |_, cx| close(cx), cx))
        .child(div().relative().h(px(HEADER_H - 8.0)).children(normal).children(bar))
        .into_any_element()
}

/// The lyrics card: a few lines around the sung one; a tap opens them all.
fn lyrics_card(app: &mut Glyph, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let pos = app.position();
    let body: AnyElement = match app.lyrics.lyrics.clone() {
        _ if app.lyrics.loading => skeleton_lyrics(4).into_any_element(),
        Some(l) if !l.synced.is_empty() => {
            let active = l.active_line(pos.as_millis() as u64, app.lyrics_offset());
            let from = active.unwrap_or(0).saturating_sub(1);
            let to = (from + 4).min(l.synced.len());
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .children(l.synced[from..to].iter().enumerate().map(|(i, line)| {
                    let on = active == Some(from + i);
                    let text: SharedString = if line.text.trim().is_empty() { "♪".into() } else { line.text.clone().into() };
                    div().text_size(px(19.0)).font_weight(FontWeight::BOLD).text_color(if on { gpui::white() } else { white(0.45) }).child(text)
                }))
                .into_any_element()
        }
        Some(l) if l.plain.is_some() => {
            let text = l.plain.unwrap_or_default().lines().take(5).collect::<Vec<_>>().join("\n");
            div().text_size(px(17.0)).font_weight(FontWeight::MEDIUM).text_color(white(0.8)).child(text).into_any_element()
        }
        _ => return None,
    };
    let expand = div().flex_none().size(px(32.0)).rounded_full().bg(hsla(0.0, 0.0, 0.0, 0.25)).flex().items_center().justify_center().child(icon("maximize-2", 15.0, gpui::white()));
    Some(
        card("np-lyrics", t("np.mode.lyrics"), Some(expand.into_any_element()))
            .bg(theme::accent_tile(0.24, 0.5))
            .cursor_pointer()
            .on_click(cx.listener(|_, _, _, cx| set_lyrics(true, cx)))
            .child(body)
            .into_any_element(),
    )
}

/// The next songs; a tap plays one, the action opens the whole queue.
fn queue_card(app: &mut Glyph, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let upcoming: Vec<_> = app.upcoming().into_iter().take(4).collect();
    if upcoming.is_empty() {
        return None;
    }
    let rows: Vec<AnyElement> = upcoming
        .iter()
        .enumerate()
        .map(|(i, u)| {
            div()
                .id(("np-up", i))
                .flex()
                .items_center()
                .gap(px(12.0))
                .mx(px(-8.0))
                .p(px(8.0))
                .rounded(px(8.0))
                .cursor_pointer()
                .active(|s| s.bg(white(0.06)))
                .on_click(cx.listener(move |this, _, _, cx| this.queue_jump(i, cx)))
                .child(cover(app, u, 44.0, 4.0, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().truncate().text_size(px(14.0)).text_color(gpui::white()).child(u.title.clone()))
                        .child(div().truncate().text_size(px(12.5)).text_color(white(0.6)).child(u.artist.clone())),
                )
                .into_any_element()
        })
        .collect();
    let open_queue = div()
        .id("np-open-queue")
        .flex()
        .items_center()
        .gap(px(2.0))
        .h(px(32.0))
        .pl(px(10.0))
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            this.show_queue = true;
            cx.notify();
        }))
        .child(div().text_size(px(12.5)).text_color(white(0.6)).child(t("np.open_queue")))
        .child(icon("chevron-right", 14.0, white(0.6)));
    Some(card("np-queue-card", t("queue.next"), Some(open_queue.into_any_element())).gap(px(4.0)).children(rows).into_any_element())
}

// ------------------------------------------------------------------ sheet

/// Now Playing: cover carousel, title, seek bar and transport, the device /
/// share / queue row, then the lyrics and up-next cards.
fn sheet(app: &mut Glyph, top: f32, window: &mut Window, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let tr = app.current.clone()?;
    let size = window.viewport_size();
    let vw = f32::from(size.width);
    let safe = window.insets().safe_area;
    let art = (vw - SIDE * 2.0).max(120.0);
    let scroll = NP.with(|n| n.borrow().scroll.clone());
    // The mini-player bar replaces the header once the transport is gone.
    let scrolled = -f32::from(scroll.offset().y);
    let sk = ((scrolled - art - 180.0) / 40.0).clamp(0.0, 1.0);

    // The sung line under the cover (synced lyrics only); a tap opens them.
    let line = app.lyrics.lyrics.as_ref().filter(|l| !l.synced.is_empty()).map(|l| {
        let text = l.active_line(app.position().as_millis() as u64, app.lyrics_offset()).map(|i| l.synced[i].text.clone()).unwrap_or_default();
        div()
            .id("np-line")
            .h(px(22.0))
            .truncate()
            .text_size(px(15.0))
            .font_weight(FontWeight::BOLD)
            .text_color(gpui::white())
            .cursor_pointer()
            .on_click(cx.listener(|_, _, _, cx| set_lyrics(true, cx)))
            .child(text)
    });

    let t_like = tr.clone();
    let liked = app.is_liked(&tr);
    let title_row = div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(song_title(SharedString::from(tr.title.clone()), tr.explicit).text_size(px(21.0)).font_weight(FontWeight::BOLD))
                .child(div().truncate().text_size(px(15.0)).text_color(theme::text_dim()).child(tr.artist.clone())),
        )
        .child(
            toggle_button("np-like", "heart", 24.0, liked, Some(theme::pink()))
                .size(px(44.0))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_like(&t_like, cx))),
        );

    let shuffle = app.shuffle();
    let repeat = app.repeat();
    let side_btn = |id: &'static str, name: &'static str, on: bool| {
        div()
            .id(id)
            .flex_none()
            .size(px(48.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(icon(name, 22.0, if on { theme::pink() } else { theme::text_dim() }))
    };
    let transport = div()
        .flex()
        .items_center()
        .justify_between()
        .child(side_btn("np-shuffle", "shuffle", shuffle).on_click(cx.listener(|this, _, _, cx| this.toggle_shuffle(cx))))
        .child(icon_button("np-prev", "skip-back", 28.0, theme::text()).size(px(56.0)).on_click(cx.listener(|this, _, _, cx| this.previous(cx))))
        .child(play_button(app, "np-play", 68.0, cx))
        .child(icon_button("np-next", "skip-forward", 28.0, theme::text()).size(px(56.0)).on_click(cx.listener(|this, _, _, cx| this.next(false, cx))))
        .child(
            side_btn("np-repeat", if repeat == glyph_core::RepeatMode::One { "repeat-1" } else { "repeat" }, repeat != glyph_core::RepeatMode::Off)
                .on_click(cx.listener(|this, _, _, cx| this.cycle_repeat(cx))),
        );

    // Where it plays (tap: Connect), share, the queue.
    let (dev_icon, dev_name, remote) = device(app);
    let dev_ink = if remote { theme::pink() } else { theme::text_dim() };
    let t_share = tr.clone();
    let tools = div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .child(
            div()
                .id("np-device")
                .flex_1()
                .min_w_0()
                .h(px(40.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.show_devices {
                        this.toggle_devices_panel(cx);
                    }
                }))
                .child(icon(dev_icon, 16.0, dev_ink))
                .child(div().truncate().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(dev_ink).child(dev_name)),
        )
        .child(icon_button("np-share", "share-2", 20.0, theme::text_dim()).size(px(40.0)).on_click(cx.listener(move |this, _, _, cx| this.share(&t_share, cx))))
        .child(
            icon_button("np-queue", "list-music", 20.0, theme::text_dim()).size(px(40.0)).mr(px(-8.0)).on_click(cx.listener(|this, _, _, cx| {
                this.show_queue = true;
                cx.notify();
            })),
        );

    let lyrics = lyrics_card(app, cx);
    let queue = queue_card(app, cx);
    let seek = seek_row(app, "np-seek", art, cx);
    let carousel = carousel(app, &tr, art, window, cx);
    let header = header(app, &tr, sk, &scroll, safe.top, cx);

    // Grouped, not evenly spaced: title, seek bar and transport belong
    // together; the cards below are their own block.
    let has_line = line.is_some();
    let content = div()
        .on_scroll_wheel(pull("np-pull", Some(scroll.clone()), |_, cx| close(cx), cx))
        .px(px(SIDE))
        .pt(px(12.0))
        .pb(safe.bottom + px(32.0))
        .flex()
        .flex_col()
        .child(carousel)
        .children(line.map(|l| l.mt(px(18.0))))
        .child(title_row.mt(px(if has_line { 10.0 } else { 24.0 })))
        .child(seek.mt(px(16.0)))
        .child(transport.mt(px(12.0)))
        .child(tools.mt(px(6.0)))
        .children(lyrics.map(|l| div().mt(px(22.0)).child(l)))
        .children(queue.map(|q| div().mt(px(12.0)).child(q)));

    Some(
        div()
            .id("np-sheet")
            .occlude()
            .absolute()
            .left_0()
            .right_0()
            .top(px(top))
            .h(size.height)
            .when(top > 0.5, |d| d.rounded_t(px(16.0)))
            .overflow_hidden()
            .bg(NP_SCRIM)
            .flex()
            .flex_col()
            .child(header)
            .child(div().id("np-scroll").track_scroll(&scroll).flex_1().min_h_0().overflow_y_scroll().child(content))
            .into_any_element(),
    )
}

// ------------------------------------------------------------------ lyrics

/// The lyrics full screen in the song's colour (the desktop lyrics view:
/// karaoke line, tap a line to jump), with a seek bar and play button.
fn lyrics_view(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let (k, running) = NP.with(|n| {
        let mut n = n.borrow_mut();
        let want = n.open && n.lyrics;
        (n.lyrics_slide.update(want), n.lyrics_slide.running())
    });
    if running {
        window.request_animation_frame();
    }
    if k <= 0.001 {
        swipe_reset("lyr-pull");
        return None;
    }
    let tr = app.current.clone()?;
    let size = window.viewport_size();
    let vw = f32::from(size.width);
    let safe = window.insets().safe_area;
    let (_, py) = swipe_offset("lyr-pull", window);
    let top = (1.0 - k) * f32::from(size.height) + py.max(0.0);

    let header = div()
        .flex_none()
        .h(px(HEADER_H - 8.0))
        .px(px(4.0))
        .flex()
        .items_center()
        .gap(px(4.0))
        .on_scroll_wheel(pull("lyr-pull", None, |_, cx| set_lyrics(false, cx), cx))
        .child(icon_action("lyr-close", "chevron-down").on_click(cx.listener(|_, _, _, cx| set_lyrics(false, cx))))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_center()
                .child(div().max_w_full().truncate().text_size(px(14.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child(tr.title.clone()))
                .child(div().max_w_full().truncate().text_size(px(12.5)).text_color(white(0.7)).child(tr.artist.clone())),
        )
        .child(div().flex_none().size(px(40.0)));

    let seek = seek_row(app, "lyr-seek", vw - SIDE * 2.0, cx);
    let controls = div()
        .flex_none()
        .px(px(SIDE))
        .pt(px(8.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(seek)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(28.0))
                .child(icon_button("lyr-prev", "skip-back", 24.0, theme::text()).size(px(48.0)).on_click(cx.listener(|this, _, _, cx| this.previous(cx))))
                .child(play_button(app, "lyr-play", 56.0, cx))
                .child(icon_button("lyr-next", "skip-forward", 24.0, theme::text()).size(px(48.0)).on_click(cx.listener(|this, _, _, cx| this.next(false, cx)))),
        );

    let body = crate::ui::now_playing::lyrics(app, cx, true);
    Some(
        div()
            .id("np-lyrics-view")
            .occlude()
            .absolute()
            .left_0()
            .right_0()
            .top(px(top))
            .h(size.height)
            .when(top > 0.5, |d| d.rounded_t(px(16.0)))
            .overflow_hidden()
            .bg(theme::accent_tile(0.24, 0.5))
            .pt(safe.top)
            .pb(safe.bottom + px(16.0))
            .flex()
            .flex_col()
            .child(header)
            .child(div().flex_1().min_h_0().px(px(SIDE - 4.0)).flex().child(body))
            .child(controls)
            .into_any_element(),
    )
}

// ------------------------------------------------------------------ bottom sheets

/// The queue or Connect from the bottom over a dimmed screen; a tap on the
/// dim or pulling the handle down closes it.
fn bottom_sheet(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let want = app.show_queue || app.show_devices;
    let (kind, k, running) = NP.with(|n| {
        let mut n = n.borrow_mut();
        if app.show_devices {
            n.sheet = Sheet::Devices;
        } else if app.show_queue {
            n.sheet = Sheet::Queue;
        }
        (n.sheet, n.sheet_slide.update(want), n.sheet_slide.running())
    });
    if running {
        window.request_animation_frame();
    }
    if k <= 0.001 {
        swipe_reset("sheet-pull");
        return None;
    }
    let safe = window.insets().safe_area;
    let vh = f32::from(window.viewport_size().height);
    let h = (vh * 0.86).min(vh - f32::from(safe.top) - 12.0);
    let (_, py) = swipe_offset("sheet-pull", window);
    let top = vh - h * k + py.max(0.0);

    let (head, body) = match kind {
        Sheet::Queue => queue_list(app, cx),
        Sheet::Devices => devices_list(app, cx),
    };
    // Handle + title: pulled down, the sheet follows and closes.
    let grab = div()
        .flex_none()
        .on_scroll_wheel(pull("sheet-pull", None, close_sheet, cx))
        .child(div().h(px(24.0)).flex().items_center().justify_center().child(div().w(px(36.0)).h(px(4.0)).rounded_full().bg(white(0.25))))
        .child(head);
    let panel = div()
        .id("np-bottom-sheet")
        .occlude()
        .absolute()
        .left_0()
        .right_0()
        .top(px(top))
        .h(px(h))
        .rounded_t(px(16.0))
        .bg(surface())
        .pb(safe.bottom)
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(grab)
        .child(body);
    Some(
        div()
            .id("np-bottom-dim")
            .occlude()
            .absolute()
            .inset_0()
            .bg(hsla(0.0, 0.0, 0.0, 0.55 * k))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| close_sheet(this, cx)))
            .child(panel)
            .into_any_element(),
    )
}

/// Sheet title with a line under it.
fn sheet_head(title: SharedString, sub: Option<AnyElement>) -> gpui::Div {
    div()
        .px(px(EDGE))
        .pb(px(12.0))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(div().text_size(px(22.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child(title))
        .children(sub)
}

/// Section heading in a sheet.
fn sheet_section(text: SharedString) -> AnyElement {
    div().px(px(EDGE)).pt(px(14.0)).pb(px(6.0)).text_size(px(15.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child(text).into_any_element()
}

/// The queue: now playing, the songs queued by hand, what plays on from the
/// list; a tap plays a song, x removes it. Shuffle, repeat and the sleep
/// timer at the bottom.
fn queue_list(app: &mut Glyph, cx: &mut Context<Glyph>) -> (AnyElement, AnyElement) {
    let up = app.upcoming();
    let n = up.len();
    let manual = app.manual_queue_len().min(n);
    let (dev_icon, dev_name, remote) = device(app);
    let dev_ink = if remote { theme::pink() } else { theme::text_dim() };
    let sub = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .children(app.queue_from.clone().map(|f| div().truncate().text_size(px(13.0)).text_color(theme::text_dim()).child(format!("{} {}", t("np.playing_from"), f))))
        .child(div().flex().items_center().gap(px(6.0)).child(icon(dev_icon, 13.0, dev_ink)).child(div().truncate().text_size(px(12.5)).text_color(dev_ink).child(dev_name)));
    let head = sheet_head(t("queue.title"), Some(sub.into_any_element()));

    let row = |app: &mut Glyph, id: (&'static str, usize), tr: &glyph_protocol::Track, now: bool, cx: &mut Context<Glyph>| {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(12.0))
            .h(px(60.0))
            .pl(px(EDGE))
            .pr(px(4.0))
            .child(cover(app, tr, 48.0, 4.0, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(song_title(SharedString::from(tr.title.clone()), tr.explicit).text_size(px(15.0)).text_color(if now { theme::pink() } else { theme::text() }))
                    .child(div().truncate().text_size(px(12.5)).text_color(theme::text_dim()).child(tr.artist.clone())),
            )
    };

    let mut rows: Vec<AnyElement> = Vec::new();
    if let Some(cur) = app.current.clone() {
        rows.push(sheet_section(t("queue.now")));
        rows.push(row(app, ("sq-now", 0), &cur, true, cx).child(div().flex_none().size(px(40.0)).flex().items_center().justify_center().child(icon("audio-lines", 16.0, theme::pink()))).into_any_element());
    }
    if manual > 0 {
        rows.push(sheet_section(tf("queue.next_n", &[("n", &manual)])));
    }
    for (i, tr) in up.iter().enumerate() {
        if i == manual && n > manual {
            let rest = n - manual;
            let head = match app.queue_from.as_deref() {
                Some(from) => tf("queue.from_n", &[("name", &from), ("n", &rest)]),
                None if manual > 0 => tf("queue.later_n", &[("n", &rest)]),
                None => tf("queue.next_n", &[("n", &rest)]),
            };
            rows.push(sheet_section(head));
        }
        rows.push(
            row(app, ("sq", i), tr, false, cx)
                .cursor_pointer()
                .active(|s| s.bg(white(0.06)))
                .on_click(cx.listener(move |this, _, _, cx| this.queue_jump(i, cx)))
                .child(icon_button(("sq-rm", i), "x", 16.0, theme::text_dim()).size(px(40.0)).on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.queue_remove(i, cx)
                })))
                .into_any_element(),
        );
    }
    if n == 0 {
        rows.push(div().px(px(EDGE)).pt(px(16.0)).text_size(px(13.5)).text_color(theme::text_dim()).child(t("queue.empty")).into_any_element());
    }

    let repeat = app.repeat();
    let tiles = div()
        .flex_none()
        .px(px(EDGE))
        .pt(px(10.0))
        .flex()
        .gap(px(8.0))
        .child(quick_tile("sq-shuffle", "shuffle", t("tip.shuffle"), app.shuffle()).on_click(cx.listener(|this, _, _, cx| this.toggle_shuffle(cx))))
        .child(
            quick_tile("sq-repeat", if repeat == glyph_core::RepeatMode::One { "repeat-1" } else { "repeat" }, t("tip.repeat"), repeat != glyph_core::RepeatMode::Off)
                .on_click(cx.listener(|this, _, _, cx| this.cycle_repeat(cx))),
        )
        .child(quick_tile("sq-timer", "timer", t("tip.timer"), app.sleep.is_some()).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, e: &gpui::MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.open_control_menu(crate::app::ControlMenu::Sleep, e.position, cx)
            }),
        ));

    let body = div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(div().id("sq-scroll").track_scroll(&app.queue_scroll).flex_1().min_h_0().overflow_y_scroll().pb(px(12.0)).children(rows))
        .child(tiles)
        .into_any_element();
    (head.into_any_element(), body)
}

/// Connect: where music plays and the devices to play on (the desktop
/// panel's list), or the jam.
fn devices_list(app: &mut Glyph, cx: &mut Context<Glyph>) -> (AnyElement, AnyElement) {
    let jam_view = app.devices_jam;
    let jam_on = app.jam_active();
    let search_on = app.discovery.is_some();
    let title = div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .when(jam_view, |d| {
            d.ml(px(-10.0)).child(icon_action("sd-back", "arrow-left").on_click(cx.listener(|this, _, _, cx| {
                this.devices_jam = false;
                cx.notify();
            })))
        })
        .child(
            div()
                .flex_1()
                .text_size(px(22.0))
                .font_weight(FontWeight::BOLD)
                .text_color(if jam_view && jam_on { theme::pink() } else { gpui::white() })
                .child(if jam_view { t("jam.title") } else { t("connect.title") }),
        )
        .when(!jam_view && search_on, |d| {
            d.child(icon_button("sd-rescan", "refresh-cw", 18.0, theme::text_dim()).size(px(40.0)).on_click(cx.listener(|this, _, _, cx| {
                this.rescan_devices_now();
                cx.notify()
            })))
        })
        .when(!jam_view, |d| {
            d.child(icon_button("sd-jam", "users", 18.0, if jam_on { theme::pink() } else { theme::text_dim() }).size(px(40.0)).mr(px(-8.0)).on_click(cx.listener(|this, _, _, cx| {
                this.devices_jam = true;
                cx.notify();
            })))
        });
    let head = div().px(px(EDGE)).pb(px(8.0)).child(title);
    let content = if jam_view { crate::ui::jam::section_el(app, cx) } else { crate::ui::connect::devices(app, cx) };
    let body = div().id("sd-scroll").track_scroll(&app.devices_scroll).flex_1().min_h_0().overflow_y_scroll().px(px(EDGE - 10.0)).pb(px(12.0)).child(content).into_any_element();
    (head.into_any_element(), body)
}
