use super::*;

/// Tracks hovering of a button with a tooltip (an invisible layer over it).
pub(super) fn tip_anchor(label: SharedString) -> gpui::Canvas<gpui::Hitbox> {
    tip_anchor_with(label, false, false)
}

/// `instant`: opens without the usual delay; `wrap`: long text in a box of
/// fixed width.
pub(super) fn tip_anchor_with(label: SharedString, instant: bool, wrap: bool) -> gpui::Canvas<gpui::Hitbox> {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            let win = window.window_handle().window_id();
            let view = window.current_view();
            let (hovered, start) = UI_RT.with(|rt| {
                let mut rt = rt.borrow_mut();
                let w = rt.win_mut(win);
                let hovered = hitbox.is_hovered(window) && !w.mouse_out;
                let mine = w.tip_now.as_ref().is_some_and(|n| n.bounds == bounds);
                let start = if hovered && !mine {
                    let warm = w.tip_now.as_ref().is_some_and(TipNow::shown)
                        || w.tip_last.is_some_and(|at| at.elapsed().as_secs_f32() < TIP_WARM_S);
                    let wait = if warm || instant { Duration::ZERO } else { TIP_DELAY };
                    let since = std::time::Instant::now();
                    w.tip_now = Some(TipNow { label: label.clone(), bounds, since, wait, dismissed: false, wrap });
                    Some((wait, since))
                } else {
                    if hovered {
                        // Keep the open tooltip's text fresh: download progress
                        // changes underneath it while hovering.
                        if let Some(n) = w.tip_now.as_mut().filter(|n| n.bounds == bounds) {
                            n.label = label.clone();
                        }
                    } else if mine {
                        if w.tip_now.as_ref().is_some_and(TipNow::shown) {
                            w.tip_last = Some(std::time::Instant::now());
                        }
                        w.tip_now = None;
                        window.request_animation_frame();
                    }
                    None
                };
                if hovered {
                    w.tip_painted = true;
                }
                (hovered, start)
            });
            // Show it once the wait is over, then keep checking while it is
            // open: a redraw re-tests the hover, and a button that is no
            // longer drawn takes its tooltip along (`begin_window_frame`).
            if let Some((wait, since)) = start {
                if wait.is_zero() {
                    window.request_animation_frame();
                }
                window
                    .spawn(cx, async move |cx| {
                        if !wait.is_zero() {
                            cx.background_executor().timer(wait).await;
                            if cx.update(|_, cx| cx.notify(view)).is_err() {
                                return;
                            }
                        }
                        loop {
                            cx.background_executor().timer(TIP_RECHECK).await;
                            let open = UI_RT.with(|rt| {
                                rt.borrow().win(win).and_then(|w| w.tip_now.as_ref()).is_some_and(|n| n.since == since)
                            });
                            if !open || cx.update(|_, cx| cx.notify(view)).is_err() {
                                return;
                            }
                        }
                    })
                    .detach();
            }
            window.on_mouse_event(move |_: &gpui::MouseMoveEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Capture {
                    return;
                }
                let back = UI_RT.with(|rt| std::mem::replace(&mut rt.borrow_mut().win_mut(win).mouse_out, false));
                if back || hitbox.is_hovered(window) != hovered {
                    cx.notify(view);
                }
            });
            window.on_mouse_event(move |_: &gpui::MouseExitEvent, phase, _, cx| {
                if phase == gpui::DispatchPhase::Capture && hovered {
                    UI_RT.with(|rt| rt.borrow_mut().win_mut(win).mouse_out = true);
                    cx.notify(view);
                }
            });
            // A click hides it (until the mouse comes back).
            window.on_mouse_event(move |_: &gpui::MouseDownEvent, phase, _, cx| {
                if phase == gpui::DispatchPhase::Capture && hovered {
                    UI_RT.with(|rt| {
                        if let Some(n) = rt.borrow_mut().win_mut(win).tip_now.as_mut().filter(|n| n.bounds == bounds) {
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

/// Tooltip box for longer explanations: wraps at a fixed width.
pub fn tip_box_wrapped(label: SharedString) -> Div {
    div()
        .flex_none()
        .w(px(280.0))
        .px(px(11.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .bg(TIP_BG)
        .font_family(theme::FONT)
        .text_size(px(12.5))
        .line_height(px(18.0))
        .text_color(gpui::hsla(0.0, 0.0, 1.0, 0.86))
        .child(label)
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
    let (label, bounds, k, wrap) = UI_RT.with(|rt| {
        let rt = rt.borrow();
        let n = rt.win(win)?.tip_now.as_ref().filter(|n| n.shown())?;
        let k = ((n.since.elapsed().as_secs_f32() - n.wait.as_secs_f32()) / 0.12).clamp(0.0, 1.0);
        Some((n.label.clone(), n.bounds, k, n.wrap))
    })?;
    if k < 1.0 {
        window.request_animation_frame();
    }
    let e = 1.0 - (1.0 - k) * (1.0 - k);
    // Below the button; above it when there is no room at the bottom.
    let room = if wrap { 140.0 } else { 44.0 };
    let below = f32::from(bounds.bottom()) + room < f32::from(window.viewport_size().height);
    let mid = bounds.center().x;
    let gap = px(4.0 + 4.0 * (1.0 - e));
    let pos = if below { gpui::point(mid, bounds.bottom() + gap) } else { gpui::point(mid, bounds.top() - gap) };
    let body = div()
        .flex()
        .flex_col()
        .items_center()
        .opacity(e)
        .when(below, |d| d.child(tip_arrow(TipArrow::Up)))
        .child(if wrap { tip_box_wrapped(label) } else { tip_box(label) })
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

/// Small (i) next to a heading: hovering shows `text` at once. Takes the
/// place of explanation lines under settings rows.
pub fn info_tip(id: impl Into<ElementId>, text: SharedString) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex_none()
        .size(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(9.0))
        .hover(|s| s.bg(gpui::hsla(0.0, 0.0, 1.0, 0.08)))
        .child(icon("info", 13.0, gpui::hsla(0.0, 0.0, 1.0, 0.42)))
        .child(tip_anchor_with(text, true, true))
}

/// Adds a tooltip with `label` to an element.
pub fn with_tip(el: Stateful<Div>, label: Option<SharedString>) -> Stateful<Div> {
    match label {
        Some(l) => el.relative().child(tip_anchor(l)),
        None => el,
    }
}
