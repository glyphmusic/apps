//! Mini player: an always-on-top window that mirrors the main window's
//! `Glyph` state. Two sizes: a slim bar (cover, song, transport) and a card
//! with a big cover; controls fade in on hover there (Spotify-like). The card
//! takes its background from the cover and can be resized freely at its
//! corner (the cover fills what is left above the title).

use crate::app::Glyph;
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::RepeatMode;
use gpui::{
    div, img, point, prelude::*, px, size, App, Bounds, Context, Entity, ObjectFit, Pixels, Window, WindowBackgroundAppearance,
    WindowBounds, WindowDecorations, WindowKind, WindowOptions,
};
use std::cell::Cell;
use std::rc::Rc;

pub const MINI_W: f32 = 300.0;
pub const MINI_H: f32 = 52.0;
/// The big card at its default size.
pub const CARD_W: f32 = 300.0;
pub const CARD_H: f32 = 384.0;
const CARD_MIN_W: f32 = 240.0;
const CARD_MAX_W: f32 = 1200.0;
const CARD_MIN_H: f32 = 220.0;
const CARD_MAX_H: f32 = 1200.0;
const CARD_PAD: f32 = 12.0;
/// Hover bar buttons: as far from the top as from the sides.
const TOP_INSET: f32 = 4.0;
const TOP_BTN: f32 = 24.0;
/// Room above the cover for the hover bar (switch, grip, close).
const CARD_TOP: f32 = TOP_INSET * 2.0 + TOP_BTN;
/// Title and artist under the cover, and the gap above them.
const INFO_H: f32 = 40.0;
const INFO_GAP: f32 = 10.0;
/// Corner radius we draw. On Windows the system rounds the window itself
/// (8 px): the area outside our own rounding would show black there, as
/// the window's transparency is not composited.
const WIN_R: f32 = if cfg!(windows) { 0.0 } else { 8.0 + TOP_INSET };

pub struct MiniPlayer {
    app: Entity<Glyph>,
    /// Volume slider pinned open by a click (card); hovering opens it too.
    vol_open: bool,
    /// Card corner drag: mouse position and card size at the start.
    resize: Option<(gpui::Point<Pixels>, gpui::Size<Pixels>)>,
}

/// Window size of each mode (`card`: the card's width and height).
pub fn mini_size(big: bool, card: (f32, f32)) -> gpui::Size<Pixels> {
    if big { card_size(card.0, card.1) } else { size(px(MINI_W), px(MINI_H)) }
}

/// The card at `w` x `h`, kept within its limits.
fn card_size(w: f32, h: f32) -> gpui::Size<Pixels> {
    size(px(w.clamp(CARD_MIN_W, CARD_MAX_W)), px(h.clamp(CARD_MIN_H, CARD_MAX_H)))
}

impl MiniPlayer {
    /// Opens the mini player bottom-right on the primary screen.
    pub fn open(app: Entity<Glyph>, cx: &mut App) -> Option<gpui::AnyWindowHandle> {
        let sz = {
            let g = app.read(cx);
            mini_size(g.mini_big, (g.mini_card_w, g.mini_card_h))
        };
        let display = cx.primary_display();
        let area = display.as_ref().map(|d| d.visible_bounds());
        let origin = area.map_or(point(px(40.0), px(40.0)), |a| {
            point(a.origin.x + a.size.width - sz.width - px(24.0), a.origin.y + a.size.height - sz.height - px(24.0))
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds { origin, size: sz })),
            titlebar: None,
            kind: WindowKind::PopUp,
            is_movable: true,
            is_resizable: false,
            focus: false,
            window_decorations: Some(WindowDecorations::Client),
            // Transparent so the rounded corners show the desktop, not a square.
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some("glyph".into()),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            #[cfg(windows)]
            if let Some(h) = crate::app::window_hwnd(window) {
                no_frame(h);
            }
            cx.new(|cx| {
                // Song, progress and play state all live in `Glyph`.
                cx.observe(&app, |_, _, cx| cx.notify()).detach();
                MiniPlayer { app, vol_open: false, resize: None }
            })
        })
        .map(|h| h.into())
        .map_err(|e| tracing::warn!("mini player window: {e:#}"))
        .ok()
    }
}

/// Windows 11 draws a thin light frame around popup windows: turn it off,
/// and let the system round the corners (it clips cleanly).
#[cfg(windows)]
fn no_frame(hwnd: usize) {
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(hwnd: isize, attr: u32, value: *const std::ffi::c_void, size: u32) -> i32;
    }
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWA_BORDER_COLOR: u32 = 34;
    const DWMWCP_ROUND: u32 = 2;
    const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;
    unsafe {
        DwmSetWindowAttribute(hwnd as isize, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE as *const u32 as *const _, 4);
        DwmSetWindowAttribute(hwnd as isize, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUND as *const u32 as *const _, 4);
    }
}

impl Render for MiniPlayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (big, shown) = {
            let g = self.app.read(cx);
            (g.mini_big, g.mini_shown)
        };
        // Hidden (kept for a quick reopen): draw nothing, run nothing.
        if !shown {
            return div().size_full();
        }
        let body = if big { self.card(window, cx) } else { self.bar(window, cx) };
        div().size_full().child(body).children(tooltip_layer(window))
    }
}

/// Moves the window when dragged (native caption drag on Windows).
fn grab(d: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
    d.window_control_area(gpui::WindowControlArea::Drag).on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
        if !cfg!(windows) {
            window.start_window_move()
        }
    })
}

/// Switches between the bar and the card in place (the window is resized,
/// keeping its bottom-right corner).
fn switch_size(app: &Entity<Glyph>, window: &mut Window, cx: &mut App) {
    let sz = app.update(cx, |g, cx| {
        g.set_mini_big_flag(!g.mini_big, cx);
        mini_size(g.mini_big, (g.mini_card_w, g.mini_card_h))
    });
    fit_window(window, sz, cx);
}

/// Hides the mini player from its own close button (closes it where it
/// cannot be hidden). The app cannot reach this window while it handles
/// one of its own clicks, so the window does it itself.
fn close(app: &Entity<Glyph>, window: &mut Window, cx: &mut App) {
    let hidden = set_visible(window, false, cx);
    if !hidden {
        window.remove_window();
    }
    app.update(cx, |g, cx| {
        g.mini_shown = false;
        if !hidden {
            g.mini_window = None;
        }
        cx.notify();
    });
}

/// Shows or hides the window without closing it (Windows), as opening a new
/// window takes a noticeable moment. Returns false where this is not
/// available: the caller closes and reopens the window instead.
pub fn set_visible(window: &Window, show: bool, cx: &mut App) -> bool {
    #[cfg(windows)]
    if let Some(h) = crate::app::window_hwnd(window) {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        }
        const SW_HIDE: i32 = 0;
        const SW_SHOWNOACTIVATE: i32 = 4;
        // Later, not inside this window's own event handling (the call
        // sends it messages right away).
        cx.foreground_executor()
            .spawn(async move {
                unsafe { ShowWindow(h as isize, if show { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
            })
            .detach();
        return true;
    }
    let _ = (window, show, cx);
    false
}

/// Resizes the window to `sz`, keeping its bottom-right corner in place.
pub fn fit_window(window: &mut Window, sz: gpui::Size<Pixels>, cx: &mut App) {
    #[cfg(windows)]
    if let Some(h) = crate::app::window_hwnd(window) {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn SetWindowPos(hwnd: isize, after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
        }
        const SWP_NOZORDER: u32 = 0x4;
        const SWP_NOACTIVATE: u32 = 0x10;
        let b = window.bounds();
        let s = window.scale_factor();
        let dev = |v: Pixels| (f32::from(v) * s).round() as i32;
        let (x, y) = (dev(b.origin.x + b.size.width - sz.width), dev(b.origin.y + b.size.height - sz.height));
        let (w, hh) = (dev(sz.width), dev(sz.height));
        cx.foreground_executor()
            .spawn(async move {
                unsafe { SetWindowPos(h as isize, 0, x, y, w, hh, SWP_NOZORDER | SWP_NOACTIVATE) };
            })
            .detach();
        return;
    }
    let _ = cx;
    window.resize(sz);
}

/// A clickable / draggable bar: `frac` filled, `on` gets the fraction under
/// the mouse (0 at the left, or at the bottom when `vertical`).
pub(crate) fn frac_bar(id: &'static str, vertical: bool, frac: f32, track: gpui::Hsla, fill: gpui::Hsla, on: Rc<dyn Fn(f32, &mut App)>) -> gpui::AnyElement {
    let bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
    let b1 = bounds.clone();
    let at = move |p: gpui::Point<Pixels>| -> Option<f32> {
        let b = bounds.get()?;
        Some(if vertical {
            1.0 - (f32::from(p.y - b.origin.y) / f32::from(b.size.height).max(1.0))
        } else {
            f32::from(p.x - b.origin.x) / f32::from(b.size.width).max(1.0)
        }
        .clamp(0.0, 1.0))
    };
    let at2 = at.clone();
    let on2 = on.clone();
    let frac = frac.clamp(0.0, 1.0);
    let filled = if vertical {
        div().absolute().left_0().right_0().bottom_0().h(gpui::relative(frac)).rounded_full().bg(fill)
    } else {
        div().absolute().top_0().bottom_0().left_0().w(gpui::relative(frac)).rounded_full().bg(fill)
    };
    let knob = if vertical {
        div().absolute().left(px(-4.0)).size(px(12.0)).rounded_full().bg(gpui::white()).bottom(gpui::relative(frac)).mb(px(-6.0))
    } else {
        div().absolute().top(px(-4.0)).size(px(12.0)).rounded_full().bg(gpui::white()).left(gpui::relative(frac)).ml(px(-6.0))
    };
    // A taller invisible hit area around the thin line.
    let hit = div().id(id).relative().cursor_pointer().occlude();
    let hit = if vertical { hit.w(px(20.0)).h_full().flex().justify_center() } else { hit.h(px(14.0)).w_full().flex().items_center() };
    hit.on_mouse_down(gpui::MouseButton::Left, move |e, _, cx| {
        cx.stop_propagation();
        if let Some(f) = at(e.position) {
            on(f, cx);
        }
    })
    .on_mouse_move(move |e, _, cx| {
        if e.pressed_button == Some(gpui::MouseButton::Left) {
            if let Some(f) = at2(e.position) {
                on2(f, cx);
            }
        }
    })
    .child(
        div()
            .relative()
            .when(vertical, |d| d.w(px(4.0)).h_full())
            .when(!vertical, |d| d.h(px(4.0)).w_full())
            .rounded_full()
            .bg(track)
            .child(filled)
            .child(knob)
            .child(gpui::canvas(move |b, _, _| b1.set(Some(b)), |_, _, _, _| {}).absolute().inset_0()),
    )
    .into_any_element()
}

impl MiniPlayer {
    /// The big card: cover, song, like. Hover shows the top bar; hovering
    /// the cover shows transport, progress and volume over it.
    fn card(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let app = self.app.clone();
        let (track, playing, loading, pos, dur, shuffle, repeat, volume) = {
            let g = app.read(cx);
            (g.current.clone(), g.is_playing(), g.is_loading(), g.position(), g.duration(), g.shuffle(), g.repeat(), g.volume())
        };
        let frac = match dur {
            Some(d) if d.as_secs_f32() > 0.0 => (pos.as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let liked = track.as_ref().is_some_and(|t| app.read(cx).is_liked(t));
        let vp = window.viewport_size();
        // Background: the cover's main colour, dark enough for white text.
        let bg = app.read(cx).palette.map_or(gpui::hsla(0.0, 0.0, 0.07, 0.99), |p| {
            let c = p.0[1];
            c.scale((0.14 / c.luma().max(0.01)).min(1.0)).hsla(0.99)
        });
        let art = track.as_ref().and_then(|tr| app.update(cx, |g, gcx| g.cover_path(tr, gcx)));
        let art = match art {
            // GPUI does not clip pictures to rounded parents: round it itself.
            Some(p) => img(p).size_full().rounded(px(8.0)).object_fit(ObjectFit::Cover).into_any_element(),
            None => div().size_full().bg(theme::card()).flex().items_center().justify_center().child(icon("music-2", 48.0, theme::text_faint())).into_any_element(),
        };
        let (title, artist) = match &track {
            Some(t) => (t.title.clone(), t.artist.clone()),
            None => (crate::i18n::t("mini.idle").to_string(), String::new()),
        };
        let stop = |d: gpui::Stateful<gpui::Div>| d.on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
        let ctl = |id: &'static str, name: &'static str, sz: f32, c| stop(icon_button(id, name, sz, c));
        let me = cx.entity().downgrade();

        // Hover bar: size switch, grip (drag), close. The buttons line up
        // with the cover's edges below them.
        let a_close = app.clone();
        let a_sw = app.clone();
        let top_btn = |id: &'static str, name: &'static str| ctl(id, name, 14.0, theme::text_dim()).size(px(TOP_BTN));
        let top = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(CARD_TOP))
            .p(px(TOP_INSET))
            .flex()
            .items_start()
            .opacity(hover_k("mini-card"))
            .child(top_btn("card-small", "minimize-2").on_click(move |_, window, cx| switch_size(&a_sw, window, cx)))
            .child(grab(div().id("card-grip")).flex_1().h_full().flex().items_center().justify_center().child(icon("grip-horizontal", 16.0, theme::text_dim())))
            .child(top_btn("card-close", "x").on_click(move |_, window, cx| close(&a_close, window, cx)));

        // Over the cover: transport row in the middle, progress at the bottom.
        let (a1, a2, a3, a4, a5, a6, a7) = (app.clone(), app.clone(), app.clone(), app.clone(), app.clone(), app.clone(), app.clone());
        // Hovering the button (or the bar itself) shows the volume bar above
        // it; a click pins it open.
        let vol_k = if self.vol_open { 1.0 } else { hover_k("card-vol").max(hover_k("card-vol-pop")) };
        let vol_open = vol_k > 0.0;
        let pinned = self.vol_open;
        let me2 = me.clone();
        let vol_btn = div()
            .relative()
            .child(
                stop(icon_button_plain("card-vol", if volume <= 0.001 { "volume-x" } else { "volume-2" }, 15.0, gpui::white())).on_click(move |_, _, cx| {
                    me2.update(cx, |m, cx| {
                        m.vol_open = !m.vol_open;
                        cx.notify();
                    })
                    .ok();
                }),
            )
            .when(vol_open, |d| {
                let a = a6.clone();
                // Deferred: drawn above the hover bar and not cut off by the cover.
                d.child(gpui::deferred(
                    // Reaches down to the button, so the mouse can move up onto it.
                    div()
                        .id("card-vol-pop")
                        .absolute()
                        .bottom(px(29.0))
                        .left(px(1.0))
                        .w(px(28.0))
                        .h(px(108.0))
                        .pt(px(10.0))
                        .pb(px(14.0))
                        .opacity(vol_k)
                        .occlude()
                        .flex()
                        .justify_center()
                        .child(div().absolute().top_0().left_0().right_0().bottom(px(4.0)).rounded(px(8.0)).bg(gpui::hsla(0.0, 0.0, 0.1, 0.98)))
                        .child(frac_bar(
                            "card-vol-bar",
                            true,
                            volume,
                            gpui::hsla(0.0, 0.0, 1.0, 0.2),
                            theme::pink(),
                            Rc::new(move |f, cx| a.update(cx, |g, cx| {
                                g.set_volume(f);
                                cx.notify();
                            })),
                        ))
                        .child(hover_fill("card-vol-pop", gpui::transparent_black(), 0.0)),
                ))
            })
            .child(hover_fill("card-vol", gpui::transparent_black(), 0.0));
        let play = stop(div().id("card-play"))
            .flex_none()
            .size(px(38.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                press("card-play");
                a2.update(cx, |g, cx| g.toggle_play(cx))
            })
            .child({
                let d = 38.0 - 5.0 * press_k("card-play");
                div().size(px(d)).rounded_full().bg(gpui::white()).flex().items_center().justify_center().child(play_glyph(loading, playing, 14.0, d))
            });
        let controls = div()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(2.0))
            .child(vol_btn)
            .child(ctl("card-shuffle", "shuffle", 15.0, if shuffle { theme::pink() } else { gpui::white() }).on_click(move |_, _, cx| a4.update(cx, |g, cx| g.toggle_shuffle(cx))))
            .child(ctl("card-prev", "skip-back", 16.0, gpui::white()).on_click(move |_, _, cx| {
                press("card-prev");
                a1.update(cx, |g, cx| g.previous(cx))
            }))
            .child(play)
            .child(ctl("card-next", "skip-forward", 16.0, gpui::white()).on_click(move |_, _, cx| {
                press("card-next");
                a3.update(cx, |g, cx| g.next(false, cx))
            }))
            .child(
                ctl("card-repeat", if repeat == RepeatMode::One { "repeat-1" } else { "repeat" }, 15.0, if repeat != RepeatMode::Off { theme::pink() } else { gpui::white() })
                    .on_click(move |_, _, cx| a5.update(cx, |g, cx| g.cycle_repeat(cx))),
            )
            .child({
                let (a_share, t_share) = (app.clone(), track.clone());
                ctl("card-share", "link-2", 15.0, gpui::white()).on_click(move |_, _, cx| {
                    if let Some(t) = &t_share {
                        a_share.update(cx, |g, cx| g.share(t, cx));
                    }
                })
            });
        let progress = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .px(px(12.0))
            .pb(px(12.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(11.0))
                    .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.8))
                    .child(fmt_time(pos))
                    .child(dur.map(fmt_time).unwrap_or_else(|| "-:--".into())),
            )
            .child(frac_bar(
                "card-seek",
                false,
                frac,
                gpui::hsla(0.0, 0.0, 1.0, 0.25),
                gpui::white(),
                Rc::new(move |f, cx| {
                    a7.update(cx, |g, cx| {
                        if let Some(d) = g.duration() {
                            g.seek_to(d.mul_f32(f));
                        }
                        cx.notify();
                    })
                }),
            ));
        let overlay = div()
            .absolute()
            .inset_0()
            .rounded(px(8.0))
            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.55))
            .flex()
            .flex_col()
            .opacity(if pinned { 1.0 } else { hover_k("mini-cover") })
            .child(div().flex_1())
            .child(controls)
            .child(div().flex_1())
            .child(progress);
        // A square cover, as big as the window allows; cover and title sit
        // centred in whatever room is left.
        let side = (f32::from(vp.width) - CARD_PAD * 2.0).min(f32::from(vp.height) - CARD_TOP - CARD_PAD - INFO_GAP - INFO_H).max(60.0);
        let cover = div()
            .id("card-cover")
            .relative()
            .flex_none()
            .size(px(side))
            .rounded(px(8.0))
            .overflow_hidden()
            .on_hover(move |hovered, _, cx| {
                if !*hovered {
                    me.update(cx, |m, cx| {
                        if m.vol_open {
                            m.vol_open = false;
                            cx.notify();
                        }
                    })
                    .ok();
                }
            })
            .child(art)
            .child(overlay)
            // Hover trackers go last: above the sliders (which block the
            // mouse for everything below them).
            .child(hover_fill("mini-cover", gpui::transparent_black(), 0.0));

        let a_art = app.clone();
        let artist_name = artist.clone();
        let t_like = track.clone();
        let a_like = app.clone();
        let info = div()
            .flex_none()
            .w(px(side))
            .h(px(INFO_H))
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(grab(div().id("card-title")).truncate().text_size(px(17.0)).line_height(px(22.0)).font_weight(gpui::FontWeight::BOLD).child(title))
                    .child(
                        div().flex().min_w_0().child(
                            stop(div().id("card-artist"))
                                .truncate()
                                .text_size(px(12.5))
                                .text_color(theme::text_dim())
                                .cursor_pointer()
                                .hover(|s| s.underline().text_color(theme::text()))
                                .on_click(move |_, _, cx| {
                                    let a = artist_name.clone();
                                    if !a.is_empty() {
                                        a_art.update(cx, |g, cx| g.open_artist(&a, cx));
                                    }
                                })
                                .child(artist),
                        ),
                    ),
            )
            .child(
                ctl("card-like", if liked { "heart-fill" } else { "heart" }, 18.0, if liked { theme::pink() } else { theme::text_dim() }).on_click(move |_, _, cx| {
                    if let Some(t) = &t_like {
                        a_like.update(cx, |g, cx| g.toggle_like(t, cx));
                    }
                }),
            );

        // Corner grip: drag to resize, width and height each as you like.
        let me3 = cx.entity().downgrade();
        let grip = div()
            .id("card-resize")
            .absolute()
            // Inside the rounded corner.
            .right(px(4.0))
            .bottom(px(4.0))
            .size(px(18.0))
            .flex()
            .items_end()
            .justify_end()
            .p(px(4.0))
            // No mark: the cursor shows it.
            .cursor(gpui::CursorStyle::ResizeUpLeftDownRight)
            .on_mouse_down(gpui::MouseButton::Left, move |e, _, cx| {
                cx.stop_propagation();
                me3.update(cx, |m, _| m.resize = Some((e.position, vp))).ok();
            });
        let resizing = self.resize.is_some();
        div()
            .id("mini-card")
            .size_full()
            .rounded(px(WIN_R))
            .bg(bg)
            .font_family(theme::FONT)
            .text_color(theme::text())
            .relative()
            .overflow_hidden()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(INFO_GAP))
            .px(px(CARD_PAD))
            .pt(px(CARD_TOP))
            .pb(px(CARD_PAD))
            .when(resizing, |d| {
                // While resizing the window follows the mouse anywhere (the
                // window has it captured, also outside); the size is kept on
                // release.
                let me4 = cx.entity().downgrade();
                d.child(
                    gpui::canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            let m1 = me4.clone();
                            window.on_mouse_event(move |e: &gpui::MouseMoveEvent, phase, window, cx| {
                                if phase != gpui::DispatchPhase::Bubble {
                                    return;
                                }
                                m1.update(cx, |this, _| {
                                    if let Some((start, s0)) = this.resize {
                                        let d = e.position - start;
                                        window.resize(card_size(f32::from(s0.width + d.x), f32::from(s0.height + d.y)));
                                    }
                                })
                                .ok();
                            });
                            let m2 = me4.clone();
                            window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, window, cx| {
                                if phase != gpui::DispatchPhase::Bubble {
                                    return;
                                }
                                m2.update(cx, |this, cx| {
                                    if this.resize.take().is_some() {
                                        let s = window.viewport_size();
                                        this.app.update(cx, |g, cx| g.set_mini_card_size(f32::from(s.width), f32::from(s.height), cx));
                                        cx.notify();
                                    }
                                })
                                .ok();
                            });
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
            })
            .child(cover)
            .child(info)
            .child(top)
            .child(grip)
            .child(hover_fill("mini-card", gpui::transparent_black(), 0.0))
            .into_any_element()
    }

    /// The slim bar.
    fn bar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let app = self.app.clone();
        let (track, playing, loading, frac) = {
            let g = app.read(cx);
            let frac = match g.duration() {
                Some(d) if d.as_secs_f32() > 0.0 => (g.position().as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0),
                _ => 0.0,
            };
            (g.current.clone(), g.is_playing(), g.is_loading(), frac)
        };
        let liked = track.as_ref().is_some_and(|t| app.read(cx).is_liked(t));
        let art = track.as_ref().and_then(|tr| app.update(cx, |g, gcx| g.cover_path(tr, gcx)));
        let art = match art {
            Some(p) => img(p).flex_none().size(px(34.0)).rounded(px(6.0)).object_fit(ObjectFit::Cover).into_any_element(),
            None => div()
                .flex_none()
                .size(px(34.0))
                .rounded(px(6.0))
                .bg(theme::card())
                .flex()
                .items_center()
                .justify_center()
                .child(icon("music-2", 16.0, theme::text_faint()))
                .into_any_element(),
        };
        let (title, artist) = match &track {
            Some(t) => (t.title.clone(), t.artist.clone()),
            None => (crate::i18n::t("mini.idle").to_string(), String::new()),
        };
        let (a1, a2, a3, a5, a6) = (app.clone(), app.clone(), app.clone(), app.clone(), app.clone());
        let artist_name = artist.clone();
        let t_like = track.clone();
        // Buttons must not start a window move.
        let stop = |d: gpui::Stateful<gpui::Div>| d.on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
        let ctl = |id: &'static str, name: &'static str, size: f32, c| stop(icon_button(id, name, size, c));

        div()
            .size_full()
            .rounded(px(WIN_R))
            .bg(gpui::hsla(0.0, 0.0, 0.06, 0.98))
            .font_family(theme::FONT)
            .text_color(theme::text())
            .relative()
            .overflow_hidden()
            .flex()
            .items_center()
            .pl(px(9.0))
            .pr(px(8.0))
            .gap(px(2.0))
            // Cover and title are the grab area: drag them to move the
            // window. The artist is a link to their page instead.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    // The cover is a grab handle; hovering it offers the big card.
                    .child(grab(div().id("mini-drag-art").group("mini-art").relative().child(art)).child({
                        let a_big = a6.clone();
                        div()
                            .id("mini-expand")
                            .occlude()
                            .absolute()
                            .inset_0()
                            .rounded(px(6.0))
                            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.6))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .opacity(0.0)
                            .group_hover("mini-art", |s| s.opacity(1.0))
                            .on_click(move |_, window, cx| switch_size(&a_big, window, cx))
                            .child(icon("maximize-2", 14.0, gpui::white()))
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(grab(
                                div()
                                    .id("mini-drag-title")
                                    .truncate()
                                    .text_size(px(13.0))
                                    .line_height(px(16.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(title),
                            ))
                            .child(
                                div().flex().min_w_0().child(
                                    stop(div().id("mini-artist"))
                                        .truncate()
                                        .text_size(px(11.0))
                                        .line_height(px(13.0))
                                        .text_color(theme::text_dim())
                                        .cursor_pointer()
                                        .hover(|s| s.underline().text_color(theme::text()))
                                        .on_click(move |_, _, cx| {
                                            let a = artist_name.clone();
                                            if !a.is_empty() {
                                                a6.update(cx, |g, cx| g.open_artist(&a, cx));
                                            }
                                        })
                                        .child(artist),
                                ),
                            ),
                    ),
            )
            .child(
                ctl("mini-like", if liked { "heart-fill" } else { "heart" }, 15.0, if liked { theme::pink() } else { theme::text_dim() })
                    .on_click(move |_, _, cx| {
                        if let Some(t) = &t_like {
                            a5.update(cx, |g, cx| g.toggle_like(t, cx));
                        }
                    }),
            )
            .child(ctl("mini-prev", "skip-back", 15.0 - 3.0 * press_k("mini-prev"), theme::text()).size(px(29.0)).on_click(move |_, _, cx| {
                press("mini-prev");
                a1.update(cx, |g, cx| g.previous(cx))
            }))
            .child(
                stop(div().id("mini-play"))
                    .flex_none()
                    .size(px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        press("mini-play");
                        a2.update(cx, |g, cx| g.toggle_play(cx))
                    })
                    .child({
                        let d = 28.0 - 4.0 * press_k("mini-play");
                        div()
                            .size(px(d))
                            .rounded_full()
                            .bg(gpui::white())
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.opacity(0.88))
                            .child(play_glyph(loading, playing, 11.0, d))
                    }),
            )
            .child(ctl("mini-next", "skip-forward", 15.0 - 3.0 * press_k("mini-next"), theme::text()).size(px(29.0)).on_click(move |_, _, cx| {
                press("mini-next");
                a3.update(cx, |g, cx| g.next(false, cx))
            }))
            // Progress along the bottom edge.
            .child(div().absolute().left_0().bottom_0().h(px(2.0)).w(px(MINI_W * frac)).bg(theme::pink()))
            .into_any_element()
    }
}
