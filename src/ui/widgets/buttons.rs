use super::*;

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

/// Call from a toggle's click handler so its icon plays the pop animation.
pub fn mark_toggled(key: impl Into<SharedString>) {
    let key = key.into();
    UI_RT.with(|rt| rt.borrow_mut().last_toggle = Some((key, std::time::Instant::now())));
}

pub(super) fn has_fill(name: &str) -> bool {
    matches!(name, "heart" | "list-music" | "picture-in-picture-2" | "layout-split")
}

/// Toggle icon button. `key` identifies it for the click animation (pass the
/// same key to [`mark_toggled`]); `accent` colours the "on" state.
pub fn toggle_button(id: impl Into<SharedString>, name: &str, size: f32, on: bool, accent: Option<Hsla>) -> Stateful<Div> {
    let id: SharedString = id.into();
    let fill = on && has_fill(name);
    let icon_name = if fill { format!("{name}-fill") } else { name.to_string() };
    let color = if on { accent.unwrap_or_else(theme::text) } else { theme::text_dim() };
    let popping = UI_RT.with(|rt| {
        rt.borrow()
            .last_toggle
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
pub(super) fn heart_burst(id: &SharedString, size: f32, color: Hsla) -> Option<AnyElement> {
    const N: usize = 8;
    const MS: u64 = 600;
    let fresh = UI_RT.with(|rt| {
        rt.borrow()
            .last_toggle
            .as_ref()
            .is_some_and(|(k, at)| k == id && at.elapsed() < Duration::from_millis(MS))
    });
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

pub(super) const HOVER_IN_S: f32 = 0.12;
pub(super) const HOVER_OUT_S: f32 = 0.24;

pub(super) fn hover_level(on: bool, at: std::time::Instant, from: f32) -> f32 {
    let dur = if on { HOVER_IN_S } else { HOVER_OUT_S };
    let t = (at.elapsed().as_secs_f32() / dur).min(1.0);
    let e = 1.0 - (1.0 - t) * (1.0 - t);
    if on { from + (1.0 - from) * e } else { from * (1.0 - e) }
}

/// Notes the hover state of `key` this frame; returns its level and whether
/// it is still fading.
pub(super) fn hover_track(win: gpui::WindowId, key: &SharedString, hovered: bool, instant: bool) -> (f32, bool) {
    UI_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        let m = &mut rt.win_mut(win).hovers;
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
    UI_RT.with(|rt| {
        rt.borrow()
            .cur_win()
            .hovers
            .get(key)
            .map_or(0.0, |&(on, at, from)| hover_level(on, at, from))
    })
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
            let hovered = hitbox.is_hovered(window) && !UI_RT.with(|rt| rt.borrow().win(win).is_some_and(|w| w.mouse_out));
            let (k, moving) = hover_track(win, &key, hovered, cx.reduce_motion());
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
                let back = UI_RT.with(|rt| {
                    let mut rt = rt.borrow_mut();
                    let w = rt.win_mut(win);
                    std::mem::replace(&mut w.mouse_out, false)
                });
                if back || hitbox.is_hovered(window) != hovered {
                    cx.notify(view);
                }
            });
            // Leaving the window: let the box fade out.
            window.on_mouse_event(move |_: &gpui::MouseExitEvent, phase, _, cx| {
                if phase == gpui::DispatchPhase::Capture && hovered {
                    UI_RT.with(|rt| rt.borrow_mut().win_mut(win).mouse_out = true);
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
    toggle_state(id.into(), on, label.into(), true, None)
}

/// Toggle that cannot be used right now: switch and label are dimmed, the
/// pointer cursor is gone, and `tip` explains why on hover.
pub fn toggle_unavailable(id: impl Into<ElementId>, on: bool, label: impl Into<SharedString>, tip: SharedString) -> Stateful<Div> {
    toggle_state(id.into(), on, label.into(), false, Some(tip))
}

pub(super) fn toggle_state(id: ElementId, on: bool, label: SharedString, enabled: bool, tip: Option<SharedString>) -> Stateful<Div> {
    let el = div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.0))
        .when(enabled, |d| d.cursor_pointer())
        .when(!enabled, |d| d.opacity(0.4).text_color(theme::text_faint()))
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
        .child(label);
    match tip {
        Some(t) => with_tip(el, Some(t)),
        None => el,
    }
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
