//! Notifications at the top centre, stacked like a deck: the newest in front,
//! older ones peeking out below it, a little narrower and dimmer. Hovering
//! the stack fans it out into a list; a toast can be swiped up to dismiss.

use crate::app::{Glyph, Toast, ToastIcon};
use crate::theme;
use crate::ui::widgets::icon;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Context, FontWeight, MouseButton, MouseMoveEvent, MouseUpEvent, SharedString, Window};
use std::time::{Duration, Instant};

const TOP: f32 = 40.0;
const H: f32 = 40.0;
const W_DESKTOP: f32 = 460.0;

thread_local! {
    /// Deck width: fixed on the desktop, the screen minus its edges on phones.
    static DECK_W: std::cell::Cell<f32> = const { std::cell::Cell::new(W_DESKTOP) };
    /// Phones: the toast a finger is swiping and when it last moved (pans
    /// arrive as scroll events, without a release when they fling).
    static TOUCH_DRAG: std::cell::Cell<Option<(u64, Instant)>> = const { std::cell::Cell::new(None) };
}

/// The toast under a finger right now (phones).
fn touch_dragging() -> Option<u64> {
    TOUCH_DRAG.with(|d| d.get().filter(|(_, at)| at.elapsed() < Duration::from_millis(150)).map(|(id, _)| id))
}

#[allow(non_snake_case)]
fn W() -> f32 {
    DECK_W.with(|w| w.get())
}
const GAP: f32 = 8.0;
/// How far each older toast peeks out below the one in front.
const PEEK: f32 = 9.0;
/// Narrower per step back in the deck.
const SHRINK: f32 = 30.0;
/// Toasts drawn in the collapsed deck (older ones wait invisibly).
const DECK: usize = 3;
/// Dismiss animation length.
pub const GONE_S: f32 = 0.18;
/// Swipe this far up to dismiss.
const SWIPE: f32 = 22.0;
/// Side padding of the "Undo" button.
const UNDO_PAD: f32 = 10.0;

fn white(a: f32) -> gpui::Hsla {
    gpui::hsla(0.0, 0.0, 1.0, a)
}

/// Upward drag with a little rubber band when pulled down.
fn rubber(dy: f32) -> f32 {
    if dy < 0.0 {
        dy
    } else {
        (dy * 0.25).min(10.0)
    }
}

fn expanded(app: &Glyph) -> bool {
    app.toast_hover || app.toast_drag.is_some()
}

/// Eases positions, depth and drag offsets; the app keeps rendering while
/// anything moves.
fn step(app: &mut Glyph) {
    let open = expanded(app);
    let dragging = app.toast_drag.map(|(id, _)| id).or_else(touch_dragging);
    let mut slot = 0usize;
    for t in app.toasts.iter_mut() {
        if t.gone.is_none() {
            t.target = if open { slot as f32 * (H + GAP) } else { slot.min(DECK) as f32 * PEEK };
            t.depth_target = if open { 0.0 } else { slot as f32 };
            slot += 1;
        }
        if t.y.is_nan() {
            t.y = t.target - 14.0;
            t.depth = t.depth_target;
        }
        t.y += (t.target - t.y) * 0.3;
        if (t.target - t.y).abs() < 0.3 {
            t.y = t.target;
        }
        t.depth += (t.depth_target - t.depth) * 0.3;
        if (t.depth_target - t.depth).abs() < 0.01 {
            t.depth = t.depth_target;
        }
        if dragging != Some(t.id) && t.gone.is_none() {
            t.drag *= 0.6;
            if t.drag.abs() < 0.3 {
                t.drag = 0.0;
            }
        }
        // Stack hovered or a toast held: keep them on screen.
        if open {
            t.at = Instant::now() - Duration::from_millis(300);
        }
    }
}

/// Moving toasts: positions settling, drags returning, fades.
pub fn animating(app: &Glyph) -> bool {
    // Hovered: long texts may be scrolling.
    (app.toast_hover && !app.toasts.is_empty())
        || app.toast_drag.is_some()
        || touch_dragging().is_some()
        || app.toasts.iter().any(|t| {
            let e = t.at.elapsed();
            t.y.is_nan()
                || (t.y - t.target).abs() > 0.3
                || (t.depth - t.depth_target).abs() > 0.01
                || t.drag != 0.0
                || t.gone.is_some()
                || e < Duration::from_millis(250)
                || e > t.life() - Duration::from_millis(400)
        })
}

const BG: gpui::Hsla = gpui::Hsla { h: 0.0, s: 0.0, l: 0.085, a: 0.98 };
/// Marquee speed (px/s), pause before it starts, and the gap between loops.
const MARQUEE_PX_S: f32 = 45.0;
const MARQUEE_DELAY: f32 = 0.5;
const MARQUEE_GAP: f32 = 48.0;

/// Width of `text` in the toast font.
fn text_w(window: &mut Window, text: &str, size: f32, weight: FontWeight) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let run = gpui::TextRun {
        len: text.len(),
        font: gpui::Font { weight, ..gpui::font(theme::FONT) },
        color: gpui::white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(window.text_system().shape_line(SharedString::from(text.to_string()), px(size), &[run], None).width)
}

/// Soft edge over the text area (`left`: fades in from the left side).
fn edge(left: bool) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(18.0))
        .when(left, |d| d.left_0())
        .when(!left, |d| d.right_0())
        .bg(gpui::linear_gradient(
            if left { 90.0 } else { 270.0 },
            gpui::linear_color_stop(BG, 0.0),
            gpui::linear_color_stop(BG.opacity(0.0), 1.0),
        ))
}

fn pill(t: &Toast, text_width: f32, undo_w: f32, marquee_t: Option<f32>, cx: &mut Context<Glyph>) -> AnyElement {
    let e = t.at.elapsed().as_secs_f32();
    let left = t.life().as_secs_f32() - e;
    let fade_in = (e / 0.2).min(1.0);
    let fade_out = (left / 0.35).clamp(0.0, 1.0);
    let gone = t.gone.map(|g| (g.elapsed().as_secs_f32() / GONE_S).min(1.0)).unwrap_or(0.0);
    // Deeper in the deck: dimmer, and gone past the last visible step.
    let depth_a = (1.0 - t.depth * 0.22).clamp(0.0, 1.0) * (DECK as f32 + 0.5 - t.depth).clamp(0.0, 1.0);
    let alpha = (fade_in * fade_out * (1.0 - gone) * depth_a).clamp(0.0, 1.0);
    // Only the front toast shows its text; the ones behind are just edges.
    let content_a = (1.0 - t.depth).clamp(0.0, 1.0);
    let lift = if crate::ui::compact::TOUCH {
        // Phones: the deck sits at the bottom; toasts leave downwards.
        (1.0 - fade_out) * 8.0 + gone * 36.0
    } else {
        (1.0 - fade_out) * -8.0 - gone * 36.0
    };
    let y = t.y + t.drag + lift;
    let w = W() - t.depth * SHRINK;
    let id = t.id;
    let (icon_name, icon_color) = match t.icon {
        ToastIcon::Check => ("check", gpui::hsla(145.0 / 360.0, 0.55, 0.52, 1.0)),
        ToastIcon::Info => ("info", white(0.6)),
    };
    let body = div()
        .id(("toast", id as usize))
        .absolute()
        .top(px(y))
        .left(px((W() - w) / 2.0))
        .w(px(w))
        .h(px(H))
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(16.0))
        .rounded(px(10.0))
        .bg(gpui::hsla(0.0, 0.0, 0.085, 0.98))
        .border_1()
        .border_color(white(0.07))
        .shadow_lg()
        // Above the titlebar's drag strip: without this the native caption
        // wins and the grab cursor never shows.
        .occlude()
        .cursor_grab()
        .opacity(alpha)
        .font_family(theme::FONT)
        // Phones: swipe down (or up) to dismiss.
        .when(crate::ui::compact::TOUCH, |d| {
            d.on_scroll_wheel(cx.listener(move |this, e: &gpui::ScrollWheelEvent, _, cx| {
                cx.stop_propagation();
                let Some(t) = this.toasts.iter_mut().find(|t| t.id == id && t.gone.is_none()) else { return };
                match e.touch_phase {
                    gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled => {
                        TOUCH_DRAG.with(|d| d.set(None));
                        if t.drag.abs() > SWIPE {
                            t.gone = Some(Instant::now());
                        }
                    }
                    _ => {
                        TOUCH_DRAG.with(|d| d.set(Some((id, Instant::now()))));
                        t.drag += f32::from(e.delta.pixel_delta(px(16.0)).y);
                        if t.drag.abs() > SWIPE * 3.0 {
                            t.gone = Some(Instant::now());
                        }
                    }
                }
                cx.notify();
            }))
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                cx.stop_propagation();
                // Phones: a tap dismisses (no hover deck, no swipe).
                if crate::ui::compact::TOUCH {
                    if let Some(t) = this.toasts.iter_mut().find(|t| t.id == id && t.gone.is_none()) {
                        t.gone = Some(Instant::now());
                    }
                    cx.notify();
                } else if this.toasts.iter().any(|t| t.id == id && t.gone.is_none()) {
                    this.toast_drag = Some((id, f32::from(e.position.y)));
                    cx.notify();
                }
            }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .flex_1()
                .min_w_0()
                .h_full()
                .opacity(content_a)
                .child(div().flex_none().child(icon(icon_name, 15.0, icon_color)))
                .child({
                    let avail = w - 32.0 - 15.0 - 10.0 - if t.undo.is_some() { undo_w + 10.0 } else { 0.0 };
                    let overflow = text_width > avail + 0.5;
                    let row = |key: &'static str| {
                        div()
                            .id(key)
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .whitespace_nowrap()
                            .child(div().text_size(px(14.0)).font_weight(FontWeight::SEMIBOLD).text_color(white(0.95)).child(t.title.clone()))
                            .when_some(t.sub.clone(), |d, sub| {
                                d.child(div().text_size(px(13.0)).text_color(white(0.3)).child("·")).child(
                                    div().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(white(0.45)).child(sub.to_uppercase()),
                                )
                            })
                    };
                    let loop_w = text_width + MARQUEE_GAP;
                    let off = match (overflow, marquee_t) {
                        (true, Some(mt)) => ((mt - MARQUEE_DELAY).max(0.0) * MARQUEE_PX_S) % loop_w,
                        _ => 0.0,
                    };
                    let moving = overflow && off > 0.0;
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_hidden()
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(px(-off))
                                .flex()
                                .items_center()
                                .gap(px(MARQUEE_GAP))
                                .child(row("toast-text-a"))
                                .when(moving, |d| d.child(row("toast-text-b"))),
                        )
                        .when(moving, |d| d.child(edge(true)))
                        .when(overflow, |d| d.child(edge(false)))
                })
                .when(t.undo.is_some(), |d| {
                    d.child(
                        div()
                            .id(("toast-undo", id as usize))
                            .flex_none()
                            .flex()
                            .items_center()
                            .h(px(26.0))
                            .px(px(UNDO_PAD))
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(white(0.9))
                            .hover(|s| s.bg(white(0.08)))
                            .active(|s| s.bg(white(0.12)))
                            // Not a grab of the toast: the button takes the press.
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.run_undo(id, cx);
                            }))
                            .child(crate::i18n::t("toast.undo")),
                    )
                }),
        );
    body.into_any_element()
}

/// The toast stack plus, while one is held, a full-window layer that tracks
/// the swipe.
pub fn layer(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    if app.toasts.is_empty() {
        app.toast_drag = None;
        app.toast_hover = false;
        return None;
    }
    step(app);
    // Phones: above the mini player and tabs, as wide as the screen allows,
    // so the pinned back button at the top stays free.
    let bottom = crate::ui::compact::TOUCH.then(|| {
        use crate::ui::compact::{MINI_H, TABS_H};
        let vw = f32::from(window.viewport_size().width);
        DECK_W.with(|w| w.set((vw - 24.0).min(W_DESKTOP)));
        // Settings and Now Playing cover the tabs and the mini player.
        let bare = app.view == crate::app::View::NowPlaying || app.settings_open || crate::ui::compact::now_playing::is_open();
        let tabs = if bare { theme::GAP } else { TABS_H + theme::GAP * 2.0 };
        let mini = if !bare && app.current.is_some() { MINI_H + theme::GAP } else { 0.0 };
        f32::from(window.insets().safe_area.bottom) + tabs + mini + 8.0
    });
    let live = app.toasts.iter().filter(|t| t.gone.is_none()).count();
    let stack_h = if expanded(app) {
        live as f32 * (H + GAP)
    } else {
        H + live.saturating_sub(1).min(DECK) as f32 * PEEK
    };
    // Oldest drawn first so the newer ones lie on top.
    let marquee_t = app.toast_hover.then(|| app.toast_hover_at.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0));
    let toasts: Vec<Toast> = app.toasts.iter().rev().cloned().collect();
    let undo_w = if toasts.iter().any(|t| t.undo.is_some()) {
        text_w(window, &crate::i18n::t("toast.undo"), 13.0, FontWeight::SEMIBOLD) + UNDO_PAD * 2.0
    } else {
        0.0
    };
    let pills: Vec<AnyElement> = toasts
        .iter()
        .map(|t| {
            let mut tw = text_w(window, &t.title, 14.0, FontWeight::SEMIBOLD);
            if let Some(sub) = &t.sub {
                tw += 10.0 + text_w(window, "·", 13.0, FontWeight::NORMAL) + 10.0 + text_w(window, &sub.to_uppercase(), 13.0, FontWeight::MEDIUM);
            }
            pill(t, tw, undo_w, marquee_t, cx)
        })
        .collect();
    let stack = div()
        .absolute()
        .map(|d| match bottom {
            Some(b) => d.bottom(px(b)),
            None => d.top(px(TOP)),
        })
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .id("toast-deck")
                .relative()
                .w(px(W()))
                .h(px(stack_h.max(H)))
                .when(!crate::ui::compact::TOUCH, |d| d.on_hover(cx.listener(|this, h: &bool, _, cx| {
                    this.toast_hover = *h;
                    this.toast_hover_at = h.then(std::time::Instant::now);
                    cx.notify();
                })))
                .children(pills),
        );
    let drag_layer = app.toast_drag.map(|(id, start)| {
        div()
            .id("toast-drag")
            .absolute()
            .inset_0()
            .cursor_grabbing()
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                if e.pressed_button != Some(MouseButton::Left) {
                    release(this, id);
                } else if let Some(t) = this.toasts.iter_mut().find(|t| t.id == id) {
                    t.drag = rubber(f32::from(e.position.y) - start);
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                    release(this, id);
                    cx.notify();
                }),
            )
    });
    Some(div().absolute().inset_0().child(stack).children(drag_layer).into_any_element())
}

/// Ends a swipe: far enough up dismisses, otherwise it springs back.
fn release(app: &mut Glyph, id: u64) {
    app.toast_drag = None;
    if let Some(t) = app.toasts.iter_mut().find(|t| t.id == id) {
        if t.drag < -SWIPE {
            t.gone = Some(Instant::now());
        }
    }
}
