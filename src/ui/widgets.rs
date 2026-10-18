//! Small building blocks. Everything is flat: no gradients, no glow, no
//! shadows. ASCII only as accents (badges, spinner, empty states).

use crate::app::{track_key, Glyph};
use crate::i18n::t;
use crate::theme;
use crate::visuals::{ascii, Palette};
use glyph_protocol::{ProviderId, Track};
use gpui::{canvas, 
    div, img, prelude::*, px, svg, AnimationExt, AnyElement, Context, Div, ElementId, FontWeight, Hsla, ObjectFit,
    SharedString, Stateful, Svg, Window,
};
use std::rc::Rc;
use std::time::Duration;

pub type Pick = Rc<dyn Fn(f32, &mut Window, &mut gpui::App)>;

/// Flat Lucide icon.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/ui/{name}.svg")))
        .flex_none()
        .size(px(size))
        .text_color(color)
}

// ---- tooltips --------------------------------------------------------------
//
// Tooltips sit next to their button (below it, or above near the bottom edge
// of the window) with a small arrow pointing at it, instead of at the mouse
// where they could cover the button. Only one is open at a time. The first
// waits a moment; once one was open, the next opens at once while the mouse
// moves on within TIP_WARM_S.

const TIP_DELAY: Duration = Duration::from_millis(500);
const TIP_WARM_S: f32 = 0.8;
const TIP_BG: Hsla = Hsla { h: 0.0, s: 0.0, l: 0.12, a: 0.98 };

struct TipNow {
    win: gpui::WindowId,
    label: SharedString,
    /// The button, in window coordinates.
    bounds: gpui::Bounds<gpui::Pixels>,
    since: std::time::Instant,
    wait: Duration,
    /// Clicked: hidden until the mouse leaves the button.
    dismissed: bool,
}

impl TipNow {
    fn shown(&self) -> bool {
        !self.dismissed && self.since.elapsed() >= self.wait
    }
}

thread_local! {
    static TIP_NOW: std::cell::RefCell<Option<TipNow>> = const { std::cell::RefCell::new(None) };
    /// When the last tooltip closed (tooltips stay "warm" for a moment).
    static TIP_LAST: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// A tooltip is open or one closed a moment ago: the next opens at once.
pub fn tips_warm() -> bool {
    TIP_NOW.with(|t| t.borrow().as_ref().is_some_and(TipNow::shown))
        || TIP_LAST.with(|c| c.get().is_some_and(|at| at.elapsed().as_secs_f32() < TIP_WARM_S))
}

/// A tooltip just closed (also the sidebar's own ones).
pub fn tip_closed() {
    TIP_LAST.with(|c| c.set(Some(std::time::Instant::now())));
}

/// Tracks hovering of a button with a tooltip (an invisible layer over it).
fn tip_anchor(label: SharedString) -> gpui::Canvas<gpui::Hitbox> {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            let win = window.window_handle().window_id();
            let view = window.current_view();
            let hovered = hitbox.is_hovered(window) && !MOUSE_OUT.with(|o| o.borrow().contains(&win));
            let start = TIP_NOW.with(|t| {
                let mut t = t.borrow_mut();
                let mine = t.as_ref().is_some_and(|n| n.win == win && n.bounds == bounds);
                if hovered && !mine {
                    let warm = t.as_ref().is_some_and(TipNow::shown)
                        || TIP_LAST.with(|c| c.get().is_some_and(|at| at.elapsed().as_secs_f32() < TIP_WARM_S));
                    let wait = if warm { Duration::ZERO } else { TIP_DELAY };
                    *t = Some(TipNow { win, label: label.clone(), bounds, since: std::time::Instant::now(), wait, dismissed: false });
                    Some(wait)
                } else {
                    if !hovered && mine {
                        if t.as_ref().is_some_and(TipNow::shown) {
                            tip_closed();
                        }
                        *t = None;
                        window.request_animation_frame();
                    }
                    None
                }
            });
            // Show it once the wait is over.
            if let Some(wait) = start {
                if wait.is_zero() {
                    window.request_animation_frame();
                } else {
                    window
                        .spawn(cx, async move |cx| {
                            cx.background_executor().timer(wait).await;
                            cx.update(|_, cx| cx.notify(view)).ok();
                        })
                        .detach();
                }
            }
            window.on_mouse_event(move |_: &gpui::MouseMoveEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Capture {
                    return;
                }
                let back = MOUSE_OUT.with(|o| o.borrow_mut().remove(&win));
                if back || hitbox.is_hovered(window) != hovered {
                    cx.notify(view);
                }
            });
            window.on_mouse_event(move |_: &gpui::MouseExitEvent, phase, _, cx| {
                if phase == gpui::DispatchPhase::Capture && hovered {
                    MOUSE_OUT.with(|o| o.borrow_mut().insert(win));
                    cx.notify(view);
                }
            });
            // A click hides it (until the mouse comes back).
            window.on_mouse_event(move |_: &gpui::MouseDownEvent, phase, _, cx| {
                if phase == gpui::DispatchPhase::Capture && hovered {
                    TIP_NOW.with(|t| {
                        if let Some(n) = t.borrow_mut().as_mut().filter(|n| n.win == win && n.bounds == bounds) {
                            n.dismissed = true;
                        }
                    });
                    cx.notify(view);
                }
            });
        },
    )
    .absolute()
    .inset_0()
}

/// Small arrow (a stepped triangle) pointing up, down or left at the button.
#[derive(Clone, Copy, PartialEq)]
pub enum TipArrow {
    Up,
    Down,
    Left,
}

pub fn tip_arrow(dir: TipArrow) -> Div {
    let mut steps = vec![2.0, 4.0, 6.0, 8.0, 10.0];
    if dir == TipArrow::Down {
        steps.reverse();
    }
    let d = div().flex_none().flex().items_center();
    let d = if dir == TipArrow::Left { d.flex_row() } else { d.flex_col() };
    d.children(steps.into_iter().map(move |w| {
        let b = div().flex_none().bg(TIP_BG);
        if dir == TipArrow::Left { b.w(px(1.0)).h(px(w)) } else { b.h(px(1.0)).w(px(w)) }
    }))
}

/// The tooltip box itself (shared with the sidebar tooltips).
pub fn tip_box(label: SharedString) -> Div {
    div()
        .flex_none()
        .px(px(9.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .bg(TIP_BG)
        .whitespace_nowrap()
        .font_family(theme::FONT)
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.92))
        .child(label)
}

/// The open tooltip of this window, if any: add it on top of the window.
pub fn tooltip_layer(window: &mut Window) -> Option<AnyElement> {
    let win = window.window_handle().window_id();
    let (label, bounds, k) = TIP_NOW.with(|t| {
        let t = t.borrow();
        let n = t.as_ref().filter(|n| n.win == win && n.shown())?;
        let k = ((n.since.elapsed().as_secs_f32() - n.wait.as_secs_f32()) / 0.12).clamp(0.0, 1.0);
        Some((n.label.clone(), n.bounds, k))
    })?;
    if k < 1.0 {
        window.request_animation_frame();
    }
    let e = 1.0 - (1.0 - k) * (1.0 - k);
    // Below the button; above it when there is no room at the bottom.
    let below = f32::from(bounds.bottom()) + 44.0 < f32::from(window.viewport_size().height);
    let mid = bounds.center().x;
    let gap = px(4.0 + 4.0 * (1.0 - e));
    let pos = if below { gpui::point(mid, bounds.bottom() + gap) } else { gpui::point(mid, bounds.top() - gap) };
    let body = div()
        .flex()
        .flex_col()
        .items_center()
        .opacity(e)
        .when(below, |d| d.child(tip_arrow(TipArrow::Up)))
        .child(tip_box(label))
        .when(!below, |d| d.child(tip_arrow(TipArrow::Down)));
    Some(
        gpui::deferred(
            gpui::anchored()
                .anchor(if below { gpui::Anchor::TopCenter } else { gpui::Anchor::BottomCenter })
                .position(pos)
                .snap_to_window_with_margin(px(6.0))
                .child(body),
        )
        .with_priority(4)
        .into_any_element(),
    )
}

/// Tooltip text for an icon button (`tip.<icon>` in the language files).
pub fn tip_for(icon_name: &str) -> Option<SharedString> {
    let key = format!("tip.{icon_name}");
    let v = t(&key);
    (v.as_ref() != key).then_some(v)
}

/// Adds a tooltip with `label` to an element.
pub fn with_tip(el: Stateful<Div>, label: Option<SharedString>) -> Stateful<Div> {
    match label {
        Some(l) => el.relative().child(tip_anchor(l)),
        None => el,
    }
}

pub fn icon_button(id: impl Into<ElementId>, name: &str, size: f32, color: Hsla) -> Stateful<Div> {
    with_tip(icon_button_plain(id, name, size, color), tip_for(name))
}

/// [`icon_button`] without a tooltip (for buttons whose hover opens
/// something of its own, like a volume slider).
pub fn icon_button_plain(id: impl Into<ElementId>, name: &str, size: f32, color: Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size + 14.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(theme::hover()))
        .active(|s| s.bg(theme::active()))
        .child(icon(name, size, color))
}

// ---- toggle buttons ------------------------------------------------------
//
// Design rule: anything toggleable shows its state in the icon itself. On =
// filled icon (where a filled variant exists) in full white or the accent;
// line-only icons (shuffle, repeat) get a small dot underneath instead.
// Off = outline icon, dimmed. Clicking pops the icon briefly.

thread_local! {
    static LAST_TOGGLE: std::cell::RefCell<Option<(SharedString, std::time::Instant)>> =
        const { std::cell::RefCell::new(None) };
}

/// Call from a toggle's click handler so its icon plays the pop animation.
pub fn mark_toggled(key: impl Into<SharedString>) {
    let key = key.into();
    LAST_TOGGLE.with(|l| *l.borrow_mut() = Some((key, std::time::Instant::now())));
}

fn has_fill(name: &str) -> bool {
    matches!(name, "heart" | "list-music" | "picture-in-picture-2" | "layout-split")
}

/// Toggle icon button. `key` identifies it for the click animation (pass the
/// same key to [`mark_toggled`]); `accent` colours the "on" state.
pub fn toggle_button(id: impl Into<SharedString>, name: &str, size: f32, on: bool, accent: Option<Hsla>) -> Stateful<Div> {
    let id: SharedString = id.into();
    let fill = on && has_fill(name);
    let icon_name = if fill { format!("{name}-fill") } else { name.to_string() };
    let color = if on { accent.unwrap_or_else(theme::text) } else { theme::text_dim() };
    let popping = LAST_TOGGLE.with(|l| {
        l.borrow()
            .as_ref()
            .is_some_and(|(k, at)| *k == id && at.elapsed() < Duration::from_millis(400))
    });
    let glyph = icon(&icon_name, size, color);
    let glyph: AnyElement = if popping {
        // A new element id per state, so the animation runs once per click.
        glyph
            .with_animation(
                ElementId::Name(format!("{id}-pop-{on}").into()),
                gpui::Animation::new(Duration::from_millis(260)).with_easing(gpui::ease_out_quint()),
                move |svg, d| {
                    // 0.7 → 1.18 → 1.0
                    let s = if d < 0.45 { 0.7 + (d / 0.45) * 0.48 } else { 1.18 - ((d - 0.45) / 0.55) * 0.18 };
                    svg.size(px(size * s))
                },
            )
            .into_any_element()
    } else {
        glyph.into_any_element()
    };
    with_tip(div()
        .id(ElementId::Name(id.clone())), tip_for(name))
        .flex_none()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size + 14.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(theme::hover()))
        .active(|s| s.bg(theme::active()))
        .child(glyph)
        .children((name == "heart" && on).then(|| heart_burst(&id, size, color)).flatten())
        .when(on && !has_fill(name), |d| {
            d.child(
                div()
                    .absolute()
                    .bottom(px(1.0))
                    .left(px((size + 14.0 - 4.0) / 2.0))
                    .size(px(4.0))
                    .rounded_full()
                    .bg(color),
            )
        })
}

/// Little dots flying out of a heart that was just liked (after
/// [`mark_toggled`]); `None` once the burst is over.
fn heart_burst(id: &SharedString, size: f32, color: Hsla) -> Option<AnyElement> {
    const N: usize = 8;
    const MS: u64 = 600;
    let fresh = LAST_TOGGLE.with(|l| l.borrow().as_ref().is_some_and(|(k, at)| k == id && at.elapsed() < Duration::from_millis(MS)));
    if !fresh {
        return None;
    }
    let box_ = size + 14.0;
    let c = box_ / 2.0;
    let mut layer = div().absolute().inset_0();
    for i in 0..N {
        // Evenly around the heart, every other one a bit farther and smaller.
        let a = i as f32 / N as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        let (far, dot) = if i % 2 == 0 { (size * 1.05, 3.5) } else { (size * 0.8, 2.5) };
        let (dx, dy) = (a.cos(), a.sin());
        layer = layer.child(
            div()
                .absolute()
                .size(px(dot))
                .rounded_full()
                .bg(color)
                .left(px(c - dot / 2.0))
                .top(px(c - dot / 2.0))
                .with_animation(
                    ElementId::Name(format!("{id}-burst-{i}").into()),
                    gpui::Animation::new(Duration::from_millis(MS)).with_easing(gpui::ease_out_quint()),
                    move |d, k| {
                        let r = size * 0.35 + far * k;
                        d.left(px(c + dx * r - dot / 2.0)).top(px(c + dy * r - dot / 2.0)).opacity((1.0 - k).powf(1.5))
                    },
                ),
        );
    }
    Some(layer.into_any_element())
}

// ---- hover fades ---------------------------------------------------------
//
// GPUI's `.hover()` switches styles instantly. Cards and rows use
// [`hover_fill`] instead: a fill behind the box's content that fades in when
// the mouse arrives and out when it leaves. [`hover_k`] gives the same level
// to things that appear on hover (action buttons, the cover's Edit overlay).

thread_local! {
    /// Windows the mouse has left (their hit test can still name the last box).
    static MOUSE_OUT: std::cell::RefCell<std::collections::HashSet<gpui::WindowId>> = std::cell::RefCell::new(Default::default());
    /// Per key: hovered, when that last changed, level at that moment.
    static HOVERS: std::cell::RefCell<std::collections::HashMap<SharedString, (bool, std::time::Instant, f32)>> =
        std::cell::RefCell::new(Default::default());
}

const HOVER_IN_S: f32 = 0.12;
const HOVER_OUT_S: f32 = 0.24;

fn hover_level(on: bool, at: std::time::Instant, from: f32) -> f32 {
    let dur = if on { HOVER_IN_S } else { HOVER_OUT_S };
    let t = (at.elapsed().as_secs_f32() / dur).min(1.0);
    let e = 1.0 - (1.0 - t) * (1.0 - t);
    if on { from + (1.0 - from) * e } else { from * (1.0 - e) }
}

/// Notes the hover state of `key` this frame; returns its level and whether
/// it is still fading.
fn hover_track(key: &SharedString, hovered: bool, instant: bool) -> (f32, bool) {
    HOVERS.with(|h| {
        let mut m = h.borrow_mut();
        if m.len() > 2000 {
            m.retain(|_, &mut (on, at, from)| on || hover_level(on, at, from) > 0.0);
        }
        let e = m.entry(key.clone()).or_insert((false, std::time::Instant::now(), 0.0));
        if e.0 != hovered {
            *e = (hovered, std::time::Instant::now(), hover_level(e.0, e.1, e.2));
        }
        if instant {
            // Reduced motion: jump to the end.
            *e = (hovered, std::time::Instant::now(), if hovered { 1.0 } else { 0.0 });
        }
        let k = hover_level(e.0, e.1, e.2);
        (k, if e.0 { k < 1.0 } else { k > 0.0 })
    })
}

/// Hover level 0..1 of the box whose [`hover_fill`] has `key`.
pub fn hover_k(key: &str) -> f32 {
    HOVERS.with(|h| h.borrow().get(key).map_or(0.0, |&(on, at, from)| hover_level(on, at, from)))
}

/// Straight blend from `a` to `b` (`k` 0..1), for colours that fade with hover.
pub fn mix(a: Hsla, b: Hsla, k: f32) -> Hsla {
    let k = k.clamp(0.0, 1.0);
    let (a, b) = (gpui::Rgba::from(a), gpui::Rgba::from(b));
    gpui::Rgba { r: a.r + (b.r - a.r) * k, g: a.g + (b.g - a.g) * k, b: a.b + (b.b - a.b) * k, a: a.a + (b.a - a.a) * k }.into()
}

/// Fill in `color` that fades in while the mouse is over the parent box and
/// out when it leaves. Add it as the first child of a `.relative()` box (it
/// fills the box, behind the rest); `radius` matches the box's corners.
pub fn hover_fill(key: impl Into<SharedString>, color: Hsla, radius: f32) -> gpui::Canvas<gpui::Hitbox> {
    let key = key.into();
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            let win = window.window_handle().window_id();
            let hovered = hitbox.is_hovered(window) && !MOUSE_OUT.with(|o| o.borrow().contains(&win));
            let (k, moving) = hover_track(&key, hovered, cx.reduce_motion());
            if k > 0.0 && color.a > 0.0 {
                window.paint_quad(gpui::quad(
                    bounds,
                    gpui::Corners::all(px(radius)),
                    color.opacity(k),
                    gpui::Edges::default(),
                    gpui::transparent_black(),
                    gpui::BorderStyle::default(),
                ));
            }
            if moving {
                window.request_animation_frame();
            }
            // Re-render when the mouse crosses the edge (as `.hover()` does).
            let view = window.current_view();
            window.on_mouse_event(move |_: &gpui::MouseMoveEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Capture {
                    return;
                }
                let back = MOUSE_OUT.with(|o| o.borrow_mut().remove(&win));
                if back || hitbox.is_hovered(window) != hovered {
                    cx.notify(view);
                }
            });
            // Leaving the window: let the box fade out.
            window.on_mouse_event(move |_: &gpui::MouseExitEvent, phase, _, cx| {
                if phase == gpui::DispatchPhase::Capture && hovered {
                    MOUSE_OUT.with(|o| o.borrow_mut().insert(win));
                    cx.notify(view);
                }
            });
        },
    )
    .absolute()
    .inset_0()
}

/// Floating flat panel: black 68 %, 1 px white-12 % border.
pub fn panel() -> Div {
    div()
        .bg(theme::panel())
        .border_1()
        .border_color(theme::border())
        .rounded(px(theme::RADIUS))
}

/// Flat dark card (#141414-like), no border.
pub fn card() -> Div {
    // Clip children (covers, icon tiles) to the card's rounded corners.
    div().bg(theme::card()).rounded(px(8.0)).overflow_hidden()
}

pub fn pill() -> Div {
    div()
        .flex()
        .items_center()
        .bg(theme::panel_strong())
        .border_1()
        .border_color(theme::border())
        .rounded(px(8.0))
}

pub fn mono() -> Div {
    div().font_family(theme::MONO)
}

/// Page heading; sits on the background, so its ink adapts to it.
pub fn heading(text: impl Into<SharedString>) -> AnyElement {
    let text: SharedString = text.into();
    theme::adaptive(("heading", text.clone()), || {
        div()
            .self_start()
            .text_size(px(24.0))
            .font_weight(FontWeight::BOLD)
            .text_color(theme::text())
            .child(text)
            .into_any_element()
    })
}

/// Section title with a small icon (Liner style), adaptive ink.
pub fn section_title(icon_name: &str, title: impl Into<SharedString>) -> AnyElement {
    let title: SharedString = title.into();
    let icon_name = icon_name.to_string();
    theme::adaptive(("section", title.clone()), || {
        div()
            .flex()
            .self_start()
            .items_center()
            .gap(px(10.0))
            .text_color(theme::text())
            .child(icon(&icon_name, 18.0, theme::text()))
            .child(
                div()
                    .text_size(px(16.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .into_any_element()
    })
}

/// Text button (flat, rounded).
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(14.0))
        .h(px(32.0))
        .rounded(px(8.0))
        .bg(theme::card())
        .border_1()
        .border_color(theme::border())
        .cursor_pointer()
        .text_size(theme::text_sm())
        .text_color(theme::text())
        .hover(|s| s.bg(theme::card_hover()))
        .active(|s| s.bg(theme::active()))
        .child(label.into())
}

/// Primary button: flat white, black text.
pub fn primary_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(16.0))
        .h(px(32.0))
        .rounded(px(8.0))
        .bg(theme::text())
        .cursor_pointer()
        .text_size(theme::text_sm())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(gpui::black())
        .hover(|s| s.opacity(0.88))
        .child(label.into())
}

/// Small text link-style button.
pub fn text_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(8.0))
        .py(px(3.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .text_color(theme::text_dim())
        .hover(|s| s.bg(theme::hover()).text_color(theme::text()))
        .child(label.into())
}

pub fn toggle(id: impl Into<ElementId>, on: bool, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.0))
        .cursor_pointer()
        .text_size(theme::text_sm())
        .child(
            div()
                .flex_none()
                .w(px(30.0))
                .h(px(18.0))
                .rounded_full()
                .p(px(2.0))
                .flex()
                .when(on, |d| d.justify_end())
                .bg(if on { theme::text() } else { theme::active() })
                .child(
                    div()
                        .size(px(14.0))
                        .rounded_full()
                        .bg(if on { gpui::black() } else { theme::text_dim() }),
                ),
        )
        .child(label.into())
}

/// ASCII source badge `[AU]` (mono accent).
pub fn badge(p: ProviderId) -> Div {
    // Hover names the source the abbreviation stands for.
    let tip: SharedString = crate::i18n::tf("tip.source", &[("name", &p.display_name())]).into();
    div().flex_none().child(
        with_tip(mono().id(SharedString::from(format!("src-badge-{}", p.badge()))), Some(tip))
            .text_size(theme::text_xs())
            .text_color(theme::text_faint())
            .child(format!("[{}]", p.badge())),
    )
}

pub fn badges(t: &Track) -> Div {
    div()
        .flex()
        .flex_none()
        .gap(px(2.0))
        .children(t.sources().into_iter().map(badge))
}

/// Flat type chip (`Track`, `Album`, `Artist`) in a solid song colour.
pub fn chip(label: impl Into<SharedString>, on: bool) -> Div {
    div()
        .flex_none()
        .px(px(12.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .text_size(px(12.0))
        // Fixed line height so the label sits in the vertical centre.
        .line_height(px(14.0))
        // Own surface: fixed colours, never the adaptive background ink.
        .bg(if on { gpui::white() } else { gpui::hsla(0.0, 0.0, 0.08, 0.92) })
        .text_color(if on { gpui::black() } else { gpui::hsla(0.0, 0.0, 1.0, 0.85) })
        .child(label.into())
}

pub fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn fmt_ms(ms: Option<u64>) -> String {
    ms.map(|m| fmt_time(Duration::from_millis(m)))
        .unwrap_or_else(|| "-:--".into())
}

/// Thin flat bar (white fill on grey track, 4 px), clickable in segments.
thread_local! {
    /// Bar being dragged: (bar id, current fraction).
    static BAR_DRAG: std::cell::Cell<Option<(&'static str, f32)>> = const { std::cell::Cell::new(None) };
    /// Bar under the mouse (shows the knob) and the mouse fraction on it.
    static BAR_HOVER: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
    static BAR_HOVER_AT: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// Fraction under the mouse on bar `id`, while the mouse is over it.
pub fn bar_hover(id: &str) -> Option<f32> {
    (BAR_HOVER.with(|h| h.get()) == Some(id)).then(|| BAR_HOVER_AT.with(|h| h.get()).clamp(0.0, 1.0))
}

/// Formats a bar fraction for the little label above it ("1:23", "64%").
pub type BarLabel = Rc<dyn Fn(f32) -> String>;

/// Thin progress / volume bar: click or drag anywhere on it. Volume-like bars
/// apply while dragging; seek bars (`id` contains "seek") show the dragged
/// position and seek once on release.
pub fn thin_bar(id: &'static str, width: f32, frac: f32, on_pick: Pick, label: Option<BarLabel>, cx: &mut Context<crate::app::Glyph>) -> Div {
    let view = cx.entity().downgrade();
    let live = !id.contains("seek");
    div().flex_none().w(px(width)).h(px(14.0)).child(
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, app_cx| {
                let drag = BAR_DRAG.with(|d| d.get()).filter(|(d, _)| *d == id);
                let shown = drag.map(|(_, f)| f).unwrap_or(frac).clamp(0.0, 1.0);
                let hot = drag.is_some() || BAR_HOVER.with(|h| h.get()) == Some(id);
                let x0 = f32::from(bounds.origin.x);
                let w = f32::from(bounds.size.width);
                let cy = f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0;
                let th = if hot { 5.0 } else { 4.0 };
                let track = gpui::Bounds { origin: gpui::point(px(x0), px(cy - th / 2.0)), size: gpui::size(px(w), px(th)) };
                let quad = |b: gpui::Bounds<gpui::Pixels>, r: f32, c: Hsla| {
                    gpui::quad(b, gpui::Corners::all(px(r)), c, gpui::Edges::default(), gpui::transparent_black(), gpui::BorderStyle::default())
                };
                window.paint_quad(quad(track, th / 2.0, gpui::hsla(0.0, 0.0, 1.0, 0.18)));
                let fill_w = w * shown;
                if fill_w > 0.5 {
                    let filled = gpui::Bounds { origin: track.origin, size: gpui::size(px(fill_w), px(th)) };
                    window.paint_quad(quad(filled, th / 2.0, if hot { theme::pink() } else { theme::text() }));
                }
                if hot {
                    let k = 11.0;
                    let knob = gpui::Bounds { origin: gpui::point(px(x0 + fill_w - k / 2.0), px(cy - k / 2.0)), size: gpui::size(px(k), px(k)) };
                    window.paint_quad(quad(knob, k / 2.0, gpui::white()));
                }
                // Value above the bar: where you drag to, or where the mouse points.
                if let Some(fmt) = label.as_ref().filter(|_| hot) {
                    let at = drag.map(|(_, f)| f).unwrap_or_else(|| BAR_HOVER_AT.with(|h| h.get())).clamp(0.0, 1.0);
                    let text: SharedString = fmt(at).into();
                    let run = gpui::TextRun {
                        len: text.len(),
                        font: gpui::font(theme::MONO),
                        color: gpui::hsla(0.0, 0.0, 1.0, 0.95),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let line = window.text_system().shape_line(text, px(11.0), &[run], None);
                    let tw = f32::from(line.width) + 12.0;
                    let lx = (x0 + w * at - tw / 2.0).clamp(x0 - 20.0, x0 + w + 20.0 - tw);
                    let ly = cy - 30.0;
                    let pill = gpui::Bounds { origin: gpui::point(px(lx), px(ly)), size: gpui::size(px(tw), px(19.0)) };
                    window.paint_quad(quad(pill, 5.0, gpui::hsla(0.0, 0.0, 0.12, 0.98)));
                    let _ = line.paint(gpui::point(px(lx + 6.0), px(ly + 2.0)), px(15.0), gpui::TextAlign::Left, None, window, app_cx);
                }
                let frac_at = move |x: f32| ((x - x0) / w.max(1.0)).clamp(0.0, 1.0);
                let (p1, p2, p3) = (on_pick.clone(), on_pick.clone(), on_pick.clone());
                let (v1, v2, v3) = (view.clone(), view.clone(), view.clone());
                // A little taller hit area than the drawn bar.
                let hit = bounds.dilate(px(4.0));
                window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Bubble || e.button != gpui::MouseButton::Left || !hit.contains(&e.position) {
                        return;
                    }
                    let f = frac_at(f32::from(e.position.x));
                    BAR_DRAG.with(|d| d.set(Some((id, f))));
                    if live {
                        p1(f, window, cx);
                    }
                    cx.stop_propagation();
                    v1.update(cx, |_, cx| cx.notify()).ok();
                });
                window.on_mouse_event(move |e: &gpui::MouseMoveEvent, _, window, cx| {
                    let over = hit.contains(&e.position);
                    let was = BAR_HOVER.with(|h| h.get()) == Some(id);
                    if over {
                        BAR_HOVER_AT.with(|h| h.set(frac_at(f32::from(e.position.x))));
                        v2.update(cx, |_, cx| cx.notify()).ok();
                    }
                    if over != was {
                        BAR_HOVER.with(|h| h.set(if over { Some(id) } else { None }));
                        v2.update(cx, |_, cx| cx.notify()).ok();
                    }
                    if BAR_DRAG.with(|d| d.get()).is_some_and(|(d, _)| d == id) {
                        if e.pressed_button != Some(gpui::MouseButton::Left) {
                            BAR_DRAG.with(|d| d.set(None));
                            return;
                        }
                        let f = frac_at(f32::from(e.position.x));
                        BAR_DRAG.with(|d| d.set(Some((id, f))));
                        if live {
                            p2(f, window, cx);
                        }
                        v2.update(cx, |_, cx| cx.notify()).ok();
                    }
                });
                window.on_mouse_event(move |e: &gpui::MouseUpEvent, _, window, cx| {
                    if let Some((d, _)) = BAR_DRAG.with(|d| d.get()).filter(|(d, _)| *d == id) {
                        let _ = d;
                        BAR_DRAG.with(|d| d.set(None));
                        p3(frac_at(f32::from(e.position.x)), window, cx);
                        v3.update(cx, |_, cx| cx.notify()).ok();
                    }
                });
            },
        )
        .size_full(),
    )
}

/// Empty / error state, Liner style: a small icon in a soft circle, title and
/// one hint line, left-aligned on the background (no card, no ASCII art).
/// `art` picks the icon.
pub fn ascii_state(
    art: &'static str,
    title: impl Into<SharedString>,
    sub: impl Into<SharedString>,
) -> AnyElement {
    let (title, sub) = (title.into(), sub.into());
    let icon_name = match art {
        ART_SEARCH => "search",
        ART_OFFLINE => "refresh-cw",
        _ => "music-2",
    };
    theme::adaptive(("empty", title.clone()), move || {
        div()
            .self_start()
            .flex()
            .items_center()
            .gap(px(14.0))
            .py(px(8.0))
            .child(
                div()
                    .flex_none()
                    .size(px(44.0))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::text().opacity(0.08))
                    .child(icon(icon_name, 18.0, theme::text_dim())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(theme::text_md())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(title),
                    )
                    .when(!sub.is_empty(), |d| {
                        d.child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(sub))
                    }),
            )
            .into_any_element()
    })
}

pub fn spinner_line(spin: char, label: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(theme::text_sm())
        .text_color(theme::text_dim())
        .child(mono().text_color(theme::text()).child(spin.to_string()))
        .child(label.into())
}

pub const ART_EMPTY: &str = r" .-------------.
 |  .       .  |
 |      _      |
 |   .-' '-.   |
 '-------------'";

pub const ART_OFFLINE: &str = r"  ____   ____
 |    |-|    |
 |  x | | x  |
 |____|-|____|";

pub const ART_SEARCH: &str = r"    .--.
   / .. \
   \ '' /__
    '--'\  \
         \__\";

/// Cover image (real provider cover via `Backend::cover`), with a flat
/// placeholder or – if `visuals.ascii_covers` – an ASCII pattern.
pub fn cover(app: &mut Glyph, t: &Track, size: f32, radius: f32, cx: &mut Context<Glyph>) -> Div {
    let el = cover_inner(app, t, size, radius, cx);
    // Once the picture is there it "ASCIIs in" (everywhere: rows, home,
    // search), timed from the moment it first shows.
    if app.cover_path(t, cx).is_some() && (!app.config.visuals.ascii_covers || app.ascii_ready(t, size)) {
        let k = reveal_k(&format!("cover:{}:{size}", track_key(t)), 0.9);
        if k < 1.0 {
            return ascii_veil(el, format!("cv-{}-{size}", track_key(t)), size, size, radius, k);
        }
    }
    el
}

fn cover_inner(app: &mut Glyph, t: &Track, size: f32, radius: f32, cx: &mut Context<Glyph>) -> Div {
    let path = app.cover_path(t, cx);
    let base = div()
        .flex_none()
        .size(px(size))
        .rounded(px(radius))
        .overflow_hidden()
        .bg(theme::card());
    // Mono cell is ~0.6 em wide and 1.05 em tall: size the grid to fill the square.
    let fs = (size / 26.0).clamp(5.0, 11.0);
    let cols = ((size / (fs * 0.6)) as u32).max(6);
    let rows = ((size / (fs * 1.05)) as u32).max(3);
    let grid = |bg: Hsla, fg: Hsla| {
        div()
            .flex_none()
            .size(px(size))
            .rounded(px(radius))
            .overflow_hidden()
            .bg(bg)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .font_family(theme::MONO)
            .text_size(px(fs))
            .line_height(px(fs * 1.05))
            .text_color(fg)
    };
    if let Some(p) = path {
        if app.config.visuals.ascii_covers {
            // The real cover rendered through the ASCII ramp (computed off the
            // UI thread, cached per track and size).
            // Rendered at 2x so it stays sharp on high-DPI screens.
            match app.ascii_cover(t, &p, (size * 2.0) as u32, cx) {
                Some(im) => return base.child(img(im).size(px(size)).rounded(px(radius))),
                None => return skeleton(format!("cover-sk-{}-{size}", track_key(t)), size, size, radius),
            }
        }
        return base.child(
            img(p)
                .size(px(size))
                .rounded(px(radius))
                .object_fit(ObjectFit::Cover),
        );
    }
    // ASCII mode only ever shows the real cover; without one it is the same
    // flat placeholder as the normal mode.
    if app.cover_loading(t) || app.backend.is_none() {
        return skeleton(format!("cover-sk-{}-{size}", track_key(t)), size, size, radius);
    }
    base.flex()
        .items_center()
        .justify_center()
        .child(icon("music-2", (size * 0.36).clamp(14.0, 64.0), theme::text_faint()))
}

// ---- loading skeletons ----------------------------------------------------

// The shimmer is ASCII: a dim field of ramp characters with a brighter band
// sweeping across it. It is drawn from the clock at render time, so the app
// keeps re-rendering while any skeleton is on screen (see `take_skeleton_seen`).

thread_local! {
    /// Settings -> Appearance -> ASCII effects (set every frame).
    static ASCII_FX: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

pub fn set_ascii_effects(on: bool) {
    ASCII_FX.with(|c| c.set(on));
}

/// ASCII reveal and shimmer are on.
pub fn ascii_effects() -> bool {
    ASCII_FX.with(|c| c.get())
}

thread_local! {
    static SKELETON_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SKELETON_T0: std::time::Instant = std::time::Instant::now();
}

/// True if a skeleton was rendered since the last call (the app then keeps
/// animating).
pub fn take_skeleton_seen() -> bool {
    SKELETON_SEEN.with(|c| c.replace(false))
}

const SHIMMER_RAMP: &[u8] = b" .:-=+*#";


thread_local! {
    /// Shaped shimmer glyphs per (character, font size): shaped once, then
    /// only painted. Rebuilding the shimmer as text every frame re-shaped
    /// thousands of characters per frame (typing and scrolling lagged).
    static SHIMMER_GLYPHS: std::cell::RefCell<std::collections::HashMap<(u8, u32), gpui::ShapedLine>> = std::cell::RefCell::new(Default::default());
}

fn shimmer_block(cols: usize, rows: usize, fs: f32, seed: u32, radius: f32) -> Div {
    let _ = cols;
    if !ascii_effects() {
        // ASCII effects off: a plain, still box.
        return div().size_full().rounded(px(radius)).bg(gpui::hsla(0.0, 0.0, 1.0, 0.06));
    }
    div()
        .size_full()
        .overflow_hidden()
        .rounded(px(radius))
        .bg(gpui::hsla(0.0, 0.0, 1.0, 0.03))
        .child(canvas(|_, _, _| (), move |bounds, _, window, cx| paint_shimmer(bounds, rows, fs, seed, window, cx)).size_full())
}

/// Paints the shimmer field into `bounds`, vertically centred.
fn paint_shimmer(bounds: gpui::Bounds<gpui::Pixels>, rows: usize, fs: f32, seed: u32, window: &mut Window, cx: &mut gpui::App) {
    SKELETON_SEEN.with(|c| c.set(true));
    let key_fs = (fs * 10.0).round() as u32;
    let glyphs: Vec<Option<gpui::ShapedLine>> = SHIMMER_GLYPHS.with(|g| {
        let mut g = g.borrow_mut();
        SHIMMER_RAMP
            .iter()
            .map(|&b| {
                (b != b' ').then(|| {
                    g.entry((b, key_fs))
                        .or_insert_with(|| {
                            window.text_system().shape_line(
                                SharedString::from((b as char).to_string()),
                                px(fs),
                                &[gpui::TextRun {
                                    len: 1,
                                    font: gpui::font(theme::MONO),
                                    color: gpui::hsla(0.0, 0.0, 1.0, 0.32),
                                    background_color: None,
                                    underline: None,
                                    strikethrough: None,
                                }],
                                None,
                            )
                        })
                        .clone()
                })
            })
            .collect()
    });
    let cw = glyphs.iter().flatten().next().map(|l| f32::from(l.width)).filter(|w| *w > 0.5).unwrap_or(fs * 0.6);
    let lh = fs * 1.05;
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    let cols = (w / cw).ceil().max(1.0) as usize;
    let rows = rows.min(((h / lh).floor() as usize).max(1));
    let ox = f32::from(bounds.origin.x);
    let oy = f32::from(bounds.origin.y) + ((h - rows as f32 * lh) / 2.0).max(0.0);
    let t = SKELETON_T0.with(|t0| t0.elapsed().as_secs_f32());
    let period = (cols as f32 + 40.0) / 60.0;
    let x0 = ((t % period) / period) * (cols as f32 + 40.0) - 20.0;
    // Only the visible part of the window is worth painting.
    let vis = window.content_mask().bounds;
    let (vy0, vy1) = (f32::from(vis.origin.y), f32::from(vis.origin.y + vis.size.height));
    let (vx0, vx1) = (f32::from(vis.origin.x), f32::from(vis.origin.x + vis.size.width));
    for y in 0..rows {
        let py = oy + y as f32 * lh;
        if py + lh < vy0 || py > vy1 {
            continue;
        }
        for x in 0..cols {
            let px_ = ox + x as f32 * cw;
            if px_ + cw < vx0 || px_ > vx1 {
                continue;
            }
            let d = (x as f32 + y as f32 * 1.6) - x0;
            let band = (-(d * d) / 18.0).exp();
            let hh = (x as u32).wrapping_mul(2654435761) ^ (y as u32 + seed).wrapping_mul(40503);
            let grain = (hh % 7) as f32 / 7.0 * 0.22;
            let v = (0.18 + grain + band * 0.8).min(1.0);
            let i = ((v * (SHIMMER_RAMP.len() - 1) as f32).round() as usize).min(SHIMMER_RAMP.len() - 1);
            if let Some(line) = &glyphs[i] {
                let _ = line.paint(gpui::point(px(px_), px(py)), px(lh), gpui::TextAlign::Left, None, window, cx);
            }
        }
    }
}

fn seed_of(id: &str) -> u32 {
    id.bytes().fold(2166136261u32, |a, b| (a ^ b as u32).wrapping_mul(16777619))
}

/// A placeholder block with the ASCII shimmer while content loads.
pub fn skeleton(id: impl Into<SharedString>, w: f32, h: f32, radius: f32) -> Div {
    let id: SharedString = id.into();
    let fs = if h < 18.0 { (h * 0.95).max(6.0) } else { 9.0f32.max((w * h / (0.6 * 1.05 * 1500.0)).sqrt()) };
    let cols = ((w / (fs * 0.6)).ceil() as usize).max(1);
    let rows = ((h / (fs * 1.05)).floor() as usize).max(1);
    div().flex_none().w(px(w)).h(px(h)).child(shimmer_block(cols, rows, fs, seed_of(&id), radius))
}

/// One shimmering text line that takes its parent's width (clipped).
fn skeleton_fill(id: impl Into<SharedString>, radius: f32, h: f32) -> AnyElement {
    let id: SharedString = id.into();
    let fs = (h * 0.95).max(6.0);
    shimmer_block(140, 1, fs, seed_of(&id), radius).into_any_element()
}

/// `n` skeleton track rows (cover, title, artist, duration).
pub fn skeleton_rows(prefix: &str, n: usize) -> Div {
    div().flex().flex_col().gap(px(2.0)).children((0..n).map(|i| {
        // Vary the bar widths a little so it does not look like a grid.
        let title_w = [0.42, 0.55, 0.36, 0.48, 0.6, 0.4][i % 6];
        let artist_w = [0.24, 0.3, 0.2, 0.27, 0.33, 0.22][i % 6];
        div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .px(px(8.0))
            .py(px(6.0))
            .child(skeleton(format!("{prefix}-sk-c{i}"), 44.0, 44.0, 6.0))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(div().h(px(12.0)).w(gpui::relative(title_w)).child(skeleton_fill(format!("{prefix}-sk-t{i}"), 4.0, 12.0)))
                    .child(div().h(px(10.0)).w(gpui::relative(artist_w)).child(skeleton_fill(format!("{prefix}-sk-a{i}"), 4.0, 10.0))),
            )
            .child(skeleton(format!("{prefix}-sk-d{i}"), 32.0, 10.0, 4.0))
    }))
}

thread_local! {
    static VIEWPORT_H: std::cell::Cell<f32> = const { std::cell::Cell::new(900.0) };
}

/// Window height (logical px) of the frame being drawn.
pub fn viewport_h() -> f32 {
    VIEWPORT_H.with(|v| v.get())
}

pub fn set_viewport_h(h: f32) {
    VIEWPORT_H.with(|v| v.set(h));
}

/// Small "E" box in front of explicit song titles.
pub fn explicit_badge() -> Div {
    // Hover says what the "E" means.
    div().flex_none().child(with_tip(div().id("explicit-badge"), Some(t("tip.explicit")))
        .flex_none()
        .size(px(15.0))
        .rounded(px(3.0))
        .bg(gpui::hsla(0.0, 0.0, 1.0, 0.22))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(gpui::hsla(0.0, 0.0, 0.0, 0.85))
        .child("E"))
}

/// A song title with the "E" box in front when it is explicit (truncates).
pub fn song_title(title: impl Into<SharedString>, explicit: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .min_w_0()
        .when(explicit, |d| d.child(explicit_badge()))
        .child(div().min_w_0().truncate().child(title.into()))
}

/// Opacity for something in progress (downloads): a soft 1.5 s pulse.
pub fn pulse() -> f32 {
    let t = SKELETON_T0.with(|t0| t0.elapsed().as_secs_f32());
    0.35 + 0.65 * (0.5 + 0.5 * (t * std::f32::consts::TAU / 1.5).sin())
}

// ---- download badge ------------------------------------------------------
//
// While a song downloads a ring fills around the arrow (accent, clockwise
// from 12 o'clock; a short spinning arc while the size is still unknown).
// When it is done the full ring fades and the arrow shrinks away while the
// check pops in.

/// Seconds the "done" transition takes.
pub const DL_DONE_ANIM_S: f32 = 0.65;

/// What a download badge shows.
#[derive(Clone, Copy, PartialEq)]
pub enum DlView {
    None,
    /// Fraction fetched (`None` = size not known yet).
    Busy(Option<f32>),
    /// Downloaded; seconds since it finished while that is still animating.
    Done(Option<f32>),
}

/// Ring of `size` px: a faint track plus the done part. `alpha` fades both.
pub fn progress_ring(size: f32, stroke: f32, fraction: Option<f32>, alpha: f32) -> gpui::Canvas<()> {
    use std::f32::consts::{FRAC_PI_2, TAU};
    let spin = SKELETON_T0.with(|t0| t0.elapsed().as_secs_f32());
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let c = bounds.center();
        let r = size / 2.0 - stroke / 2.0;
        let mut arc = |from: f32, sweep: f32, color: Hsla| {
            let n = ((sweep.abs() / TAU) * 64.0).ceil().max(2.0) as usize;
            let mut b = gpui::PathBuilder::stroke(px(stroke));
            for i in 0..=n {
                let a = from + sweep * (i as f32 / n as f32);
                let p = gpui::point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
                if i == 0 {
                    b.move_to(p)
                } else {
                    b.line_to(p)
                }
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, color);
            }
        };
        arc(0.0, TAU, gpui::hsla(0.0, 0.0, 1.0, 0.16 * alpha));
        match fraction {
            Some(f) if f > 0.0 => arc(-FRAC_PI_2, TAU * f.min(1.0), theme::pink().opacity(alpha)),
            Some(_) => {}
            None => arc(spin * TAU * 1.1 - FRAC_PI_2, TAU * 0.28, theme::pink().opacity(alpha)),
        }
    })
    .size(px(size))
}

/// Icon of a round play button (`button` px): pause / play, or a spinning
/// arc just inside the button's edge while a song loads.
pub fn play_glyph(loading: bool, playing: bool, size: f32, button: f32) -> AnyElement {
    if !loading {
        return icon(if playing { "pause" } else { "play" }, size, gpui::black()).into_any_element();
    }
    use std::f32::consts::{FRAC_PI_2, TAU};
    let spin = SKELETON_T0.with(|t0| t0.elapsed().as_secs_f32());
    let ring = button * 0.56;
    let stroke = (button / 16.0).max(2.0);
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let c = bounds.center();
        let r = ring / 2.0 - stroke / 2.0;
        // A ~100° arc that turns once every 0.8 s.
        let (from, sweep) = (spin * TAU / 0.8 - FRAC_PI_2, TAU * 0.28);
        let mut b = gpui::PathBuilder::stroke(px(stroke));
        for i in 0..=24 {
            let a = from + sweep * (i as f32 / 24.0);
            let p = gpui::point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
            if i == 0 {
                b.move_to(p)
            } else {
                b.line_to(p)
            }
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, gpui::black());
        }
    })
    .size(px(ring))
    .into_any_element()
}

/// The download state as a `size` px badge: ring + `arrow` icon while busy,
/// the pink check when done. `None` for songs that are not downloaded.
pub fn download_badge(view: DlView, size: f32, arrow: &str) -> Option<Div> {
    let stroke = (size / 9.0).max(1.5);
    let inner = size * 0.58;
    let frame = || div().flex_none().relative().size(px(size)).flex().items_center().justify_center();
    let ring = |f: Option<f32>, alpha: f32| div().absolute().inset_0().child(progress_ring(size, stroke, f, alpha));
    match view {
        DlView::None => None,
        DlView::Busy(f) => Some(frame().child(ring(f, 1.0)).child(icon(arrow, inner, theme::text()))),
        DlView::Done(None) => Some(frame().child(icon("circle-check", size, theme::pink()))),
        DlView::Done(Some(age)) => {
            // Ring + arrow out over the first 0.25 s ...
            let out = 1.0 - (age / 0.25).min(1.0);
            // ... check in from 0.15 s: 0.4 → 1.15 → 1.0.
            let p = ((age - 0.15) / (DL_DONE_ANIM_S - 0.15)).clamp(0.0, 1.0);
            let scale = if p < 0.6 { 0.4 + p / 0.6 * 0.75 } else { 1.15 - (p - 0.6) / 0.4 * 0.15 };
            let mut d = frame();
            if out > 0.0 {
                d = d.child(ring(Some(1.0), out)).child(icon(arrow, (inner * out).max(0.1), theme::text().opacity(out)));
            }
            if p > 0.0 {
                d = d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity((p * 3.0).min(1.0))
                        .child(icon("circle-check", size * scale, theme::pink())),
                );
            }
            Some(d)
        }
    }
}

/// Skeleton rows for artists in search results: round picture, name, chip.
pub fn skeleton_artist_rows(prefix: &str, n: usize) -> Div {
    div().flex().flex_col().gap(px(2.0)).children((0..n).map(|i| {
        let name_w = [0.3, 0.22, 0.26][i % 3];
        div()
            .flex()
            .items_center()
            .gap(px(14.0))
            .px(px(10.0))
            .py(px(6.0))
            .child(skeleton(format!("{prefix}-ska-c{i}"), 48.0, 48.0, 24.0))
            .child(div().flex_1().child(div().h(px(13.0)).w(gpui::relative(name_w)).child(skeleton_fill(format!("{prefix}-ska-n{i}"), 4.0, 13.0))))
            .child(skeleton(format!("{prefix}-ska-k{i}"), 46.0, 18.0, 4.0))
    }))
}

// ---- ASCII reveal --------------------------------------------------------
//
// New content "decodes" in: text starts as random ramp characters that settle
// into the real letters from left to right, and pictures start under an ASCII
// shimmer that fades out. Keys remember when something was first shown.

thread_local! {
    static REVEALS: std::cell::RefCell<std::collections::HashMap<String, std::time::Instant>> = std::cell::RefCell::new(Default::default());
}

const DECODE_RAMP: &[char] = &['#', '%', '&', '*', '+', '=', '-', ':', '.', '@', '$', '0', 'X'];

/// Progress 0..1 of the reveal for `key`, `dur` seconds long (starts the
/// first time it is asked for).
pub fn reveal_k(key: &str, dur: f32) -> f32 {
    if !ascii_effects() {
        return 1.0;
    }
    REVEALS.with(|r| {
        let mut m = r.borrow_mut();
        if m.len() > 4000 {
            m.retain(|_, at| at.elapsed().as_secs() < 120);
        }
        let at = *m.entry(key.to_string()).or_insert_with(std::time::Instant::now);
        (at.elapsed().as_secs_f32() / dur.max(0.01)).min(1.0)
    })
}

/// Something is still revealing (the app keeps re-rendering).
pub fn reveal_active() -> bool {
    REVEALS.with(|r| r.borrow().values().any(|at| at.elapsed().as_secs_f32() < 1.0))
}

/// Starts the reveal under `key` over (a label that just changed).
pub fn reveal_restart(key: &str) {
    REVEALS.with(|r| r.borrow_mut().insert(key.to_string(), std::time::Instant::now()));
}

/// Like [`reveal_k`], but only a reveal started with [`reveal_restart`]
/// runs; anything else is shown finished.
pub fn reveal_peek(key: &str, dur: f32) -> f32 {
    if !ascii_effects() {
        return 1.0;
    }
    REVEALS.with(|r| r.borrow().get(key).map_or(1.0, |at| (at.elapsed().as_secs_f32() / dur.max(0.01)).min(1.0)))
}

/// Forget reveals under `prefix` (e.g. a new search starts over).
pub fn reset_reveals(prefix: &str) {
    REVEALS.with(|r| r.borrow_mut().retain(|k, _| !k.starts_with(prefix)));
}

/// `text` part-way decoded: the first `k` share is real, the rest flickers
/// through ASCII characters (spaces stay spaces).
pub fn decode(text: &str, k: f32) -> String {
    if k >= 1.0 {
        return text.to_string();
    }
    let n = text.chars().count();
    let done = (k.max(0.0) * n as f32) as usize;
    let tick = SKELETON_T0.with(|t0| (t0.elapsed().as_secs_f32() * 12.0) as u32);
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            if i < done || c.is_whitespace() {
                c
            } else {
                let h = (i as u32).wrapping_mul(2654435761) ^ tick.wrapping_mul(40503);
                DECODE_RAMP[(h >> 7) as usize % DECODE_RAMP.len()]
            }
        })
        .collect()
}

/// `el` (a picture) under an ASCII shimmer that fades out as `k` goes to 1.
pub fn ascii_veil(el: impl IntoElement, id: impl Into<SharedString>, w: f32, h: f32, r: f32, k: f32) -> Div {
    let e = 1.0 - (1.0 - k).powi(2);
    div()
        .relative()
        .flex_none()
        .child(div().opacity(0.15 + 0.85 * e).child(el))
        .when(k < 1.0, |d| d.child(div().absolute().inset_0().opacity(1.0 - e).child(skeleton(id, w, h, r))))
}

/// Skeleton lyric lines.
pub fn skeleton_lyrics(n: usize) -> Div {
    div().flex().flex_col().gap(px(26.0)).children((0..n).map(|i| {
        let w = [0.62, 0.48, 0.7, 0.4, 0.56, 0.66, 0.44][i % 7];
        div().h(px(24.0)).w(gpui::relative(w)).child(skeleton_fill(format!("lyr-sk-{i}"), 6.0, 24.0))
    }))
}

/// Small type label ("Song", "Artist") in mixed search results.
pub fn kind_chip(label: SharedString) -> Div {
    div()
        .flex_none()
        .px(px(6.0))
        .h(px(18.0))
        .flex()
        .items_center()
        .rounded(px(4.0))
        .bg(gpui::hsla(0.0, 0.0, 1.0, 0.08))
        .text_size(px(11.0))
        .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.6))
        .child(label)
}

/// Quiet track row: cover 48 px, title, artist, badges, duration, actions.
pub fn track_row(
    app: &mut Glyph,
    list: Rc<Vec<Track>>,
    i: usize,
    id_prefix: &'static str,
    cx: &mut Context<Glyph>,
) -> Stateful<Div> {
    let t = list[i].clone();
    let playing = app.current.as_ref().map(track_key) == Some(track_key(&t));
    let liked = app.is_liked(&t);
    // Hidden explicit song: shown dimmed, does not play.
    let blocked = !app.allowed(&t);
    let group: SharedString = format!("{id_prefix}-row-{i}").into();
    let l2 = list.clone();
    let (t_next, t_q, t_like, t_share) = (t.clone(), t.clone(), t.clone(), t.clone());
    let art = cover(app, &t, 44.0, 6.0, cx);
    // Search results decode in (ASCII), everywhere else they just appear.
    let reveal = if id_prefix == "search-row" { reveal_k(&format!("search:{}", track_key(&t)), 0.9) } else { 1.0 };


    let t_menu = t.clone();
    let t_hover = t.clone();
    div()
        .id((id_prefix, i))
        // Resting on a YouTube song starts yt-dlp early (see ui::ytdlp).
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| this.prefetch_on_hover(&t_hover, *hovered, cx)))
        .on_mouse_down(
            gpui::MouseButton::Right,
            cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.open_ctx(t_menu.clone(), e.position, None, cx)
            }),
        )
        .flex()
        .items_center()
        .gap(px(14.0))
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .when(blocked, |d| d.opacity(0.4))
        .relative()
        .child(hover_fill(group.clone(), theme::card(), 8.0))
        .on_click(cx.listener(move |this, _, _, cx| this.play_list((*l2).clone(), i, cx)))
        .child(art)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w_0()
                        .when(t.explicit, |d| d.child(explicit_badge()))
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_md())
                                .font_weight(if playing { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                .text_color(theme::text())
                                .child(decode(&t.title, reveal)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .text_size(theme::text_sm())
                        .text_color(theme::text_dim())
                        .child(if reveal < 1.0 {
                            div().truncate().child(decode(&t.artist, reveal)).into_any_element()
                        } else {
                            crate::ui::lists::artist_link((id_prefix, i * 10 + 5), &t.artist, cx).into_any_element()
                        })
                        .when_some(crate::ui::lists::album_link((id_prefix, i * 10 + 6), &t, cx), |d, link| d.child(div().flex_none().child(" · ")).child(link)),
                ),
        )
        .when(id_prefix == "search-row", |d| d.child(kind_chip(tr("search.kind.song"))))
        .child(badges(&t))
        .child(
            div()
                .flex()
                .flex_none()
                .gap(px(2.0))
                .opacity(hover_k(&group))
                .child(
                    icon_button((id_prefix, i * 10 + 1), "list-start", 15.0, theme::text_dim())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.queue_add(t_next.clone(), true, cx)
                        })),
                )
                .child(
                    icon_button((id_prefix, i * 10 + 2), "list-plus", 15.0, theme::text_dim())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.queue_add(t_q.clone(), false, cx)
                        })),
                )
                .child(
                    icon_button((id_prefix, i * 10 + 3), "link-2", 15.0, theme::text_dim())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.share(&t_share, cx)
                        })),
                ),
        )
        .child({
            let key: SharedString = format!("{id_prefix}-like-{}", track_key(&t_like)).into();
            let k = key.clone();
            toggle_button(key, "heart", 15.0, liked, Some(theme::pink())).on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                mark_toggled(k.clone());
                this.toggle_like(&t_like, cx)
            }))
        })
        .child(
            mono()
                .flex_none()
                .w(px(44.0))
                .text_size(theme::text_sm())
                .text_color(theme::text_faint())
                .child(fmt_ms(t.duration_ms)),
        )
}

pub fn status_line(text: impl Into<SharedString>) -> Div {
    mono()
        .text_size(theme::text_xs())
        .text_color(theme::text_faint())
        .child(text.into())
}

/// Translate helper kept here so view code reads short.
pub fn tr(key: &str) -> SharedString {
    t(key)
}

/// Soft fade at the top and bottom edge of a scroll frame: `el` plus two
/// gradient overlays in `color` (the surface behind it). With a scroll handle
/// an edge only shows while there is more content beyond it (none at the very
/// top / bottom). The caller sizes the returned wrapper; `el` should fill it.
pub fn fade_edges(el: impl IntoElement, color: Hsla, h: f32, scroll: Option<&gpui::ScrollHandle>) -> Div {
    fade_edges_split(el, (color, h), (color, h), scroll)
}

/// [`fade_edges`] with its own colour and height for the top and the bottom edge.
pub fn fade_edges_split(el: impl IntoElement, top: (Hsla, f32), bottom: (Hsla, f32), scroll: Option<&gpui::ScrollHandle>) -> Div {
    // Edges fade in with the distance scrolled (no pop), and a page that only
    // overflows by a few pixels counts as not scrollable at all.
    let (top_k, bottom_k) = match scroll {
        Some(s) => {
            let y = -f32::from(s.offset().y);
            let max = f32::from(s.max_offset().y);
            if max < 32.0 {
                (0.0, 0.0)
            } else {
                (((y - 2.0) / 40.0).clamp(0.0, 1.0), ((max - y - 2.0) / 40.0).clamp(0.0, 1.0))
            }
        }
        None => (1.0, 1.0),
    };
    let (show_top, show_bottom) = (top_k > 0.0, bottom_k > 0.0);
    let (top_spec, bottom_spec) = (top, bottom);
    // The band also fades out towards its left and right ends (strips with
    // falling strength), so it has no hard corners against the background.
    const RAMP: [f32; 8] = [0.06, 0.16, 0.3, 0.45, 0.6, 0.74, 0.86, 0.95];
    let edge = |top: bool| {
        let (color, h) = if top { top_spec } else { bottom_spec };
        let k = if top { top_k } else { bottom_k };
        let strip = move |f: f32| {
            let c = Hsla { a: color.a * k * f, ..color };
            gpui::linear_gradient(
                if top { 180.0 } else { 0.0 },
                gpui::linear_color_stop(c, 0.0),
                gpui::linear_color_stop(c.opacity(0.0), 1.0),
            )
        };
        div()
            .absolute()
            .left_0()
            .right_0()
            .h(px(h))
            .when(top, |d| d.top_0())
            .when(!top, |d| d.bottom_0())
            .flex()
            .children(RAMP.iter().map(move |f| div().flex_none().w(px(6.0)).h_full().bg(strip(*f))))
            .child(div().flex_1().h_full().bg(strip(1.0)))
            .children(RAMP.iter().rev().map(move |f| div().flex_none().w(px(6.0)).h_full().bg(strip(*f))))
    };
    div().relative().child(el).when(show_top, |d| d.child(edge(true))).when(show_bottom, |d| d.child(edge(false)))
}

// ---- scrollbar ------------------------------------------------------------

thread_local! {
    /// Smooth wheel scrolling in progress: (handle, target offset y).
    /// Smooth scroll in progress: (handle, target offset x, target offset y).
    static SMOOTH: std::cell::RefCell<Option<(gpui::ScrollHandle, f32, f32)>> = const { std::cell::RefCell::new(None) };
    /// Scroll frames that take the wheel, in paint order (inner ones later).
    static WHEEL_FRAMES: std::cell::RefCell<Vec<(gpui::Bounds<gpui::Pixels>, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The frame the wheel moved last and when.
    static LAST_WHEEL: std::cell::Cell<Option<(gpui::Bounds<gpui::Pixels>, std::time::Instant)>> = const { std::cell::Cell::new(None) };
    /// Scroll handles for frames that have none of their own (dropdowns, rows).
    static HANDLES: std::cell::RefCell<std::collections::HashMap<String, gpui::ScrollHandle>> = std::cell::RefCell::new(Default::default());
    static SMOOTH_AT: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// One animation step of smooth wheel scrolling; true while it still moves.
/// Called from the app's 16 ms poll.
pub fn step_smooth_scroll() -> bool {
    SMOOTH.with(|s| {
        let mut s = s.borrow_mut();
        let Some((h, tx, ty)) = s.as_ref() else {
            SMOOTH_AT.with(|a| a.set(None));
            return false;
        };
        let (tx, ty) = (*tx, *ty);
        // Exponential ease by real frame time (vsync-paced, no fixed step).
        let now = std::time::Instant::now();
        let dt = SMOOTH_AT.with(|a| a.replace(Some(now))).map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(1.0 / 60.0).clamp(0.001, 0.05);
        let k = 1.0 - (-dt / 0.07).exp();
        let (cx_, cy_) = (f32::from(h.offset().x), f32::from(h.offset().y));
        let (nx, ny) = (cx_ + (tx - cx_) * k, cy_ + (ty - cy_) * k);
        if (tx - nx).abs() < 0.5 && (ty - ny).abs() < 0.5 {
            h.set_offset(gpui::point(px(tx), px(ty)));
            *s = None;
            SMOOTH_AT.with(|a| a.set(None));
        } else {
            h.set_offset(gpui::point(px(nx), px(ny)));
        }
        true
    })
}

thread_local! {
    /// Thumb being dragged: (scroll handle, mouse y at grab, scroll y at grab).
    static SCROLL_DRAG: std::cell::RefCell<Option<(gpui::ScrollHandle, f32, f32)>> = const { std::cell::RefCell::new(None) };
    /// Per scrollbar (keyed by its position): last offset seen, when it last
    /// moved or was hovered, and whether the mouse is over it.
    static SCROLL_SEEN: std::cell::RefCell<std::collections::HashMap<(i32, i32), (f32, Option<std::time::Instant>, bool)>> = Default::default();
}

/// Starts a new frame's list of wheel targets (called at the top of render).
pub fn begin_wheel_frames() {
    WHEEL_FRAMES.with(|f| f.borrow_mut().clear());
}

fn register_wheel(bounds: gpui::Bounds<gpui::Pixels>, horizontal: bool) -> usize {
    WHEEL_FRAMES.with(|f| {
        let mut f = f.borrow_mut();
        f.push((bounds, horizontal));
        f.len() - 1
    })
}

/// The innermost (last painted) frame under the mouse for that axis.
fn topmost_wheel(pos: gpui::Point<gpui::Pixels>, horizontal: bool) -> Option<usize> {
    WHEEL_FRAMES.with(|f| f.borrow().iter().rposition(|(b, h)| *h == horizontal && b.contains(&pos)))
}

/// The wheel moved the frame of `handle` within `within`.
pub fn wheeled_recently(handle: &gpui::ScrollHandle, within: std::time::Duration) -> bool {
    LAST_WHEEL.with(|l| l.get()).is_some_and(|(b, at)| b == handle.bounds() && at.elapsed() < within)
}

/// A shared scroll handle for a frame that has none of its own.
pub fn handle_for(key: &str) -> gpui::ScrollHandle {
    HANDLES.with(|h| h.borrow_mut().entry(key.to_string()).or_default().clone())
}

/// One wheel notch: glide the frame there. `false` if it cannot move that way.
fn wheel_step(hs: &gpui::ScrollHandle, e: &gpui::ScrollWheelEvent, horizontal: bool) -> bool {
    let gpui::ScrollDelta::Lines(lines) = e.delta else { return false };
    // Shift turns the wheel sideways for horizontal rows.
    let d = if horizontal { if lines.x != 0.0 { lines.x } else if e.modifiers.shift { lines.y } else { 0.0 } } else { lines.y };
    if d == 0.0 {
        return false;
    }
    let max = hs.max_offset();
    let (mx, my) = (f32::from(max.x), f32::from(max.y));
    if (horizontal && mx <= 0.0) || (!horizontal && my <= 0.0) {
        return false;
    }
    let (bx, by) = SMOOTH
        .with(|s| s.borrow().as_ref().filter(|(h, _, _)| h.bounds() == hs.bounds()).map(|(_, x, y)| (*x, *y)))
        .unwrap_or_else(|| (f32::from(hs.offset().x), f32::from(hs.offset().y)));
    let (tx, ty) = if horizontal { ((bx + d * 56.0).clamp(-mx, 0.0), by) } else { (bx, (by + d * 56.0).clamp(-my, 0.0)) };
    SMOOTH.with(|s| *s.borrow_mut() = Some((hs.clone(), tx, ty)));
    LAST_WHEEL.with(|l| l.set(Some((hs.bounds(), std::time::Instant::now()))));
    true
}

/// Smooth mouse-wheel scrolling for any frame tracked by `handle` (put it
/// next to the frame, inside a `relative()` wrapper).
pub fn smooth_wheel(handle: &gpui::ScrollHandle, horizontal: bool, cx: &mut Context<crate::app::Glyph>) -> AnyElement {
    let handle = handle.clone();
    let view = cx.entity().downgrade();
    gpui::canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            let idx = register_wheel(handle.bounds(), horizontal);
            let (hs, vs) = (handle.clone(), view.clone());
            window.on_mouse_event(move |e: &gpui::ScrollWheelEvent, phase, _, cx| {
                if phase != gpui::DispatchPhase::Capture || !hs.bounds().contains(&e.position) {
                    return;
                }
                if topmost_wheel(e.position, horizontal) != Some(idx) {
                    return;
                }
                if wheel_step(&hs, e, horizontal) {
                    cx.stop_propagation();
                    vs.update(cx, |_, cx| cx.notify()).ok();
                }
            });
        },
    )
    .absolute()
    .size_0()
    .into_any_element()
}

/// A dropdown, context menu or dialog is open over the page: page scroll
/// frames leave the mouse wheel alone then.
fn overlay_open(view: &gpui::WeakEntity<crate::app::Glyph>, cx: &gpui::App) -> bool {
    view.upgrade().is_some_and(|v| {
        let a = v.read(cx);
        a.open_select.is_some() || a.ctx_menu.is_some() || a.name_dialog.is_some()
    })
}

/// Thin overlay scrollbar for a frame tracked by `handle` (shown only when
/// there is something to scroll). The thumb can be dragged; clicking the
/// track jumps there.
pub fn scrollbar(handle: &gpui::ScrollHandle, smooth: bool, cx: &mut Context<crate::app::Glyph>) -> AnyElement {
    let handle = handle.clone();
    let view = cx.entity().downgrade();
    // What the edge fades / this bar were rendered with. The handle only learns
    // the new page's size during layout, so after a page change the first frame
    // can show stale fades; repaint once when they disagree.
    let rendered = (f32::from(handle.offset().y), f32::from(handle.max_offset().y));
    div()
        .absolute()
        .top(px(4.0))
        .bottom(px(4.0))
        .right(px(3.0))
        .w(px(8.0))
        .child(
            gpui::canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    let max = f32::from(handle.max_offset().y);
                    // A page shorter than the one before can be left scrolled
                    // past its end (nothing to scroll back with): clamp it.
                    if -f32::from(handle.offset().y) > max.max(0.0) + 0.5 {
                        handle.set_offset(gpui::point(px(0.0), px(-max.max(0.0))));
                    }
                    let now = (f32::from(handle.offset().y), max);
                    if (now.0 - rendered.0).abs() > 0.5 || (now.1 - rendered.1).abs() > 0.5 {
                        let v = view.clone();
                        cx.defer(move |cx| {
                            v.update(cx, |_, cx| cx.notify()).ok();
                        });
                    }
                    if max < 32.0 {
                        // Overflows by a few pixels only: keep the page still
                        // instead of letting it wobble up and down.
                        if max > 0.0 {
                            let hs = handle.clone();
                            let vs = view.clone();
                            window.on_mouse_event(move |e: &gpui::ScrollWheelEvent, phase, _, cx| {
                                if overlay_open(&vs, cx) {
                                    return;
                                }
                                if phase == gpui::DispatchPhase::Capture && hs.bounds().contains(&e.position) && e.delta.pixel_delta(px(20.0)).y != px(0.0) {
                                    cx.stop_propagation();
                                }
                            });
                        }
                        return;
                    }
                    // Mouse wheel: take the step over (before the scroll view
                    // jumps) and glide there over a few frames.
                    if smooth {
                        let hs = handle.clone();
                        let vs = view.clone();
                        let idx = register_wheel(handle.bounds(), false);
                        window.on_mouse_event(move |e: &gpui::ScrollWheelEvent, phase, _, cx| {
                            if phase != gpui::DispatchPhase::Capture || !hs.bounds().contains(&e.position) {
                                return;
                            }
                            // An inner frame or a menu on top takes the wheel.
                            if topmost_wheel(e.position, false) != Some(idx) || overlay_open(&vs, cx) {
                                return;
                            }
                            if wheel_step(&hs, e, false) {
                                cx.stop_propagation();
                                vs.update(cx, |_, cx| cx.notify()).ok();
                            }
                        });
                    }
                    let track_h = f32::from(bounds.size.height);
                    let view_h = f32::from(handle.bounds().size.height).max(1.0);
                    let thumb_h = (track_h * view_h / (view_h + max)).clamp(28.0, track_h);
                    let y = (-f32::from(handle.offset().y)).clamp(0.0, max);
                    let top = f32::from(bounds.origin.y) + (track_h - thumb_h) * (y / max);
                    let dragging = SCROLL_DRAG.with(|d| d.borrow().as_ref().is_some_and(|(h, _, _)| h.bounds() == handle.bounds()));
                    // Shown while scrolling, dragging or hovered; fades out
                    // after a second of stillness.
                    let key = (f32::from(bounds.origin.x) as i32, f32::from(bounds.origin.y) as i32);
                    let hot = bounds.dilate(px(4.0)).contains(&window.mouse_position());
                    let idle = SCROLL_SEEN.with(|m| {
                        let mut m = m.borrow_mut();
                        let e = m.entry(key).or_insert((y, None, false));
                        if (e.0 - y).abs() > 0.5 || dragging || hot {
                            e.1 = Some(std::time::Instant::now());
                        }
                        e.0 = y;
                        e.2 = hot;
                        e.1.map_or(f32::MAX, |t| t.elapsed().as_secs_f32())
                    });
                    const HOLD: f32 = 0.9;
                    const FADE: f32 = 0.35;
                    let vis = (1.0 - (idle - HOLD) / FADE).clamp(0.0, 1.0);
                    if idle < HOLD + FADE {
                        window.request_animation_frame();
                    }
                    let thumb = gpui::Bounds {
                        origin: gpui::point(bounds.origin.x + px(2.0), px(top)),
                        size: gpui::size(px(3.0), px(thumb_h)),
                    };
                    window.paint_quad(gpui::quad(
                        thumb,
                        gpui::Corners::all(px(1.5)),
                        gpui::hsla(0.0, 0.0, 1.0, vis * if dragging { 0.28 } else if hot { 0.16 } else { 0.09 }),
                        gpui::Edges::default(),
                        gpui::transparent_black(),
                        gpui::BorderStyle::default(),
                    ));
                    let per_px = max / (track_h - thumb_h).max(1.0);
                    let (h1, h2, v1, v2) = (handle.clone(), handle.clone(), view.clone(), view.clone());
                    window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _, cx| {
                        if phase != gpui::DispatchPhase::Bubble || e.button != gpui::MouseButton::Left || !bounds.contains(&e.position) {
                            return;
                        }
                        let my = f32::from(e.position.y);
                        let mut start = (-f32::from(h1.offset().y)).clamp(0.0, max);
                        if my < top || my > top + thumb_h {
                            // Track click: centre the thumb on the mouse.
                            start = ((my - f32::from(bounds.origin.y) - thumb_h / 2.0) * per_px).clamp(0.0, max);
                            h1.set_offset(gpui::point(px(0.0), px(-start)));
                        }
                        SCROLL_DRAG.with(|d| *d.borrow_mut() = Some((h1.clone(), my, start)));
                        cx.stop_propagation();
                        v1.update(cx, |_, cx| cx.notify()).ok();
                    });
                    window.on_mouse_event(move |e: &gpui::MouseMoveEvent, _, _, cx| {
                        let Some((h, grab_y, grab_scroll)) = SCROLL_DRAG.with(|d| d.borrow().clone()) else { return };
                        if h.bounds() != h2.bounds() {
                            return;
                        }
                        if e.pressed_button != Some(gpui::MouseButton::Left) {
                            SCROLL_DRAG.with(|d| *d.borrow_mut() = None);
                            v2.update(cx, |_, cx| cx.notify()).ok();
                            return;
                        }
                        let y = (grab_scroll + (f32::from(e.position.y) - grab_y) * per_px).clamp(0.0, max);
                        h.set_offset(gpui::point(px(0.0), px(-y)));
                        v2.update(cx, |_, cx| cx.notify()).ok();
                    });
                    window.on_mouse_event(move |_: &gpui::MouseUpEvent, _, _, _| {
                        SCROLL_DRAG.with(|d| *d.borrow_mut() = None);
                    });
                    let v3 = view.clone();
                    window.on_mouse_event(move |e: &gpui::MouseMoveEvent, _, _, cx| {
                        let over = bounds.dilate(px(4.0)).contains(&e.position);
                        let was = SCROLL_SEEN.with(|m| m.borrow().get(&key).is_some_and(|s| s.2));
                        if over != was {
                            v3.update(cx, |_, cx| cx.notify()).ok();
                        }
                    });
                },
            )
            .size_full(),
        )
        .into_any_element()
}

thread_local! {
    /// Screen y of each virtual list's top edge, from the last frame.
    static VLIST_TOP: std::cell::RefCell<std::collections::HashMap<&'static str, f32>> = Default::default();
}

/// A long list of fixed-height rows inside the page scroll frame `scroll`:
/// only the rows near the visible part are built, the rest is empty space of
/// the same height. Keeps scrolling smooth with hundreds of rows.
pub fn virtual_rows(
    key: &'static str,
    n: usize,
    pitch: f32,
    scroll: &gpui::ScrollHandle,
    mut row: impl FnMut(usize) -> AnyElement,
) -> Div {
    const OVERSCAN: usize = 8;
    let vp = scroll.bounds();
    let top = VLIST_TOP.with(|m| m.borrow().get(key).copied());
    let (first, last) = match top {
        Some(top) if f32::from(vp.size.height) > 0.0 => {
            let a = ((f32::from(vp.top()) - top) / pitch).floor().max(0.0) as usize;
            let b = ((f32::from(vp.bottom()) - top) / pitch).ceil().max(0.0) as usize;
            (a.saturating_sub(OVERSCAN).min(n), (b + OVERSCAN).min(n))
        }
        // First frame: position unknown yet, build a screenful.
        _ => (0, n.min(24)),
    };
    let first = first.min(last);
    div()
        .relative()
        .flex()
        .flex_col()
        .child(
            gpui::canvas(
                move |bounds, _, _| VLIST_TOP.with(|m| m.borrow_mut().insert(key, f32::from(bounds.origin.y))),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_0(),
        )
        .child(div().flex_none().h(px(first as f32 * pitch)))
        .children((first..last).map(|i| div().flex_none().h(px(pitch)).child(row(i)).into_any_element()))
        .child(div().flex_none().h(px((n - last) as f32 * pitch)))
}

thread_local! {
    /// Drag-to-reorder lists: eased shift of each row (by list key), and when
    /// a row last moved.
    static DRAG_SHIFT: std::cell::RefCell<std::collections::HashMap<&'static str, (std::collections::HashMap<usize, (f32, std::time::Instant)>, Option<std::time::Instant>)>> = Default::default();
}

/// Vertical shift of row `i` of list `key` while row `from` is dragged to
/// slot `to` (`drag`): the rows in between glide one `pitch` aside to make
/// room, easing there instead of jumping. `None` (no drag) snaps back.
pub fn drag_shift(key: &'static str, i: usize, drag: Option<(usize, usize)>, pitch: f32) -> f32 {
    DRAG_SHIFT.with(|m| {
        let mut m = m.borrow_mut();
        let Some((from, to)) = drag else {
            m.remove(key);
            return 0.0;
        };
        let target = if from < i && i <= to {
            -pitch
        } else if to <= i && i < from {
            pitch
        } else {
            0.0
        };
        let (rows, moving) = m.entry(key).or_default();
        let now = std::time::Instant::now();
        let (cur, at) = rows.entry(i).or_insert((0.0, now));
        let dt = now.duration_since(*at).as_secs_f32().clamp(0.0, 0.05);
        *cur += (target - *cur) * (1.0 - (-dt / 0.06).exp());
        *at = now;
        if (target - *cur).abs() > 0.5 {
            *moving = Some(now);
        } else {
            *cur = target;
        }
        *cur
    })
}

/// A reorder animation still has distance to go (keeps frames coming).
pub fn drag_shift_running() -> bool {
    DRAG_SHIFT.with(|m| m.borrow().values().any(|(_, t)| t.is_some_and(|t| t.elapsed().as_secs_f32() < 0.2)))
}

thread_local! {
    /// Last press of the transport buttons, for the little "pop" when clicked.
    static PRESSED: std::cell::RefCell<std::collections::HashMap<&'static str, std::time::Instant>> = Default::default();
}

const PRESS_SECS: f32 = 0.22;

/// Marks a button as just pressed.
pub fn press(id: &'static str) {
    PRESSED.with(|m| m.borrow_mut().insert(id, std::time::Instant::now()));
}

/// 1 right after a press, easing back to 0.
pub fn press_k(id: &'static str) -> f32 {
    PRESSED.with(|m| m.borrow().get(id).map_or(0.0, |t| (1.0 - t.elapsed().as_secs_f32() / PRESS_SECS).clamp(0.0, 1.0).powi(2)))
}

/// A press animation is still running (keeps frames coming).
pub fn press_running() -> bool {
    PRESSED.with(|m| m.borrow().values().any(|t| t.elapsed().as_secs_f32() < PRESS_SECS))
}
