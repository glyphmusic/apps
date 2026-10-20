use super::*;

// ---- scrollbar ------------------------------------------------------------

pub(super) fn step_window_smooth(w: &mut WindowUiState, now: std::time::Instant) -> bool {
    let Some((h, tx, ty)) = w.smooth.as_ref() else {
        w.smooth_at = None;
        return false;
    };
    let (tx, ty) = (*tx, *ty);
    // Exponential ease by real frame time (vsync-paced, no fixed step).
    let dt = w
        .smooth_at
        .replace(now)
        .map(|t| now.duration_since(t).as_secs_f32())
        .unwrap_or(1.0 / 60.0)
        .clamp(0.001, 0.05);
    let k = 1.0 - (-dt / 0.07).exp();
    let (cx_, cy_) = (f32::from(h.offset().x), f32::from(h.offset().y));
    let (nx, ny) = (cx_ + (tx - cx_) * k, cy_ + (ty - cy_) * k);
    if (tx - nx).abs() < 0.5 && (ty - ny).abs() < 0.5 {
        h.set_offset(gpui::point(px(tx), px(ty)));
        w.smooth = None;
        w.smooth_at = None;
    } else {
        h.set_offset(gpui::point(px(nx), px(ny)));
    }
    true
}

/// One animation step of smooth wheel scrolling; true while it still moves.
/// Called from the app's 16 ms poll.
pub fn step_smooth_scroll() -> bool {
    let now = std::time::Instant::now();
    UI_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        let mut any = step_window_smooth(&mut rt.fallback_win, now);
        for w in rt.windows.values_mut() {
            any |= step_window_smooth(w, now);
        }
        any
    })
}

/// Starts a new frame's list of wheel targets (called at the top of render).
pub fn begin_wheel_frames() {
    UI_RT.with(|rt| rt.borrow_mut().cur_win_mut().wheel_frames.clear());
}

pub(super) fn register_wheel(win: gpui::WindowId, bounds: gpui::Bounds<gpui::Pixels>, horizontal: bool) -> usize {
    UI_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        let f = &mut rt.win_mut(win).wheel_frames;
        f.push((bounds, horizontal));
        f.len() - 1
    })
}

/// The innermost (last painted) frame under the mouse for that axis.
pub(super) fn topmost_wheel(win: gpui::WindowId, pos: gpui::Point<gpui::Pixels>, horizontal: bool) -> Option<usize> {
    UI_RT.with(|rt| {
        rt.borrow()
            .win(win)
            .and_then(|w| w.wheel_frames.iter().rposition(|(b, h)| *h == horizontal && b.contains(&pos)))
    })
}

/// The wheel moved the frame of `handle` within `within`.
pub fn wheeled_recently(handle: &gpui::ScrollHandle, within: std::time::Duration) -> bool {
    let hb = handle.bounds();
    UI_RT.with(|rt| {
        let rt = rt.borrow();
        rt.windows
            .values()
            .chain(std::iter::once(&rt.fallback_win))
            .any(|w| w.last_wheel.is_some_and(|(b, at)| b == hb && at.elapsed() < within))
    })
}

/// A shared scroll handle for a frame that has none of its own.
pub fn handle_for(key: &str) -> gpui::ScrollHandle {
    UI_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        if rt.handles.len() > 512 && !rt.handles.contains_key(key) {
            rt.handles.clear();
        }
        rt.handles.entry(key.to_string()).or_default().clone()
    })
}

/// One wheel notch: glide the frame there. `false` if it cannot move that way.
pub(super) fn wheel_step(win: gpui::WindowId, hs: &gpui::ScrollHandle, e: &gpui::ScrollWheelEvent, horizontal: bool) -> bool {
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
    UI_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        let w = rt.win_mut(win);
        let (bx, by) = w
            .smooth
            .as_ref()
            .filter(|(h, _, _)| h.bounds() == hs.bounds())
            .map(|(_, x, y)| (*x, *y))
            .unwrap_or_else(|| (f32::from(hs.offset().x), f32::from(hs.offset().y)));
        let (tx, ty) = if horizontal { ((bx + d * 56.0).clamp(-mx, 0.0), by) } else { (bx, (by + d * 56.0).clamp(-my, 0.0)) };
        w.smooth = Some((hs.clone(), tx, ty));
        w.last_wheel = Some((hs.bounds(), std::time::Instant::now()));
    });
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
            let win = window.window_handle().window_id();
            let idx = register_wheel(win, handle.bounds(), horizontal);
            let (hs, vs) = (handle.clone(), view.clone());
            window.on_mouse_event(move |e: &gpui::ScrollWheelEvent, phase, _, cx| {
                if phase != gpui::DispatchPhase::Capture || !hs.bounds().contains(&e.position) {
                    return;
                }
                if topmost_wheel(win, e.position, horizontal) != Some(idx) {
                    return;
                }
                if wheel_step(win, &hs, e, horizontal) {
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
pub(super) fn overlay_open(view: &gpui::WeakEntity<crate::app::Glyph>, cx: &gpui::App) -> bool {
    view.upgrade().is_some_and(|v| {
        let a = v.read(cx);
        a.open_select.is_some() || a.ctx_menu.is_some() || a.control_menu.is_some() || a.name_dialog.is_some()
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
                    let win = window.window_handle().window_id();
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
                        let idx = register_wheel(win, handle.bounds(), false);
                        window.on_mouse_event(move |e: &gpui::ScrollWheelEvent, phase, _, cx| {
                            if phase != gpui::DispatchPhase::Capture || !hs.bounds().contains(&e.position) {
                                return;
                            }
                            // An inner frame or a menu on top takes the wheel.
                            if topmost_wheel(win, e.position, false) != Some(idx) || overlay_open(&vs, cx) {
                                return;
                            }
                            if wheel_step(win, &hs, e, false) {
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
                    // Shown while scrolling, dragging or hovered; fades out
                    // after a second of stillness.
                    let key = (f32::from(bounds.origin.x) as i32, f32::from(bounds.origin.y) as i32);
                    let hot = bounds.dilate(px(4.0)).contains(&window.mouse_position());
                    let hb = handle.bounds();
                    let (dragging, idle) = UI_RT.with(|rt| {
                        let mut rt = rt.borrow_mut();
                        let w = rt.win_mut(win);
                        let dragging = w.scroll_drag.as_ref().is_some_and(|(h, _, _)| h.bounds() == hb);
                        if w.scroll_seen.len() > 256 && !w.scroll_seen.contains_key(&key) {
                            w.scroll_seen.clear();
                        }
                        let e = w.scroll_seen.entry(key).or_insert((y, None, false));
                        if (e.0 - y).abs() > 0.5 || dragging || hot {
                            e.1 = Some(std::time::Instant::now());
                        }
                        e.0 = y;
                        e.2 = hot;
                        let idle = e.1.map_or(f32::MAX, |t| t.elapsed().as_secs_f32());
                        (dragging, idle)
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
                        UI_RT.with(|rt| rt.borrow_mut().win_mut(win).scroll_drag = Some((h1.clone(), my, start)));
                        cx.stop_propagation();
                        v1.update(cx, |_, cx| cx.notify()).ok();
                    });
                    window.on_mouse_event(move |e: &gpui::MouseMoveEvent, _, _, cx| {
                        let Some((h, grab_y, grab_scroll)) = UI_RT.with(|rt| rt.borrow().win(win).and_then(|w| w.scroll_drag.clone())) else { return };
                        if h.bounds() != h2.bounds() {
                            return;
                        }
                        if e.pressed_button != Some(gpui::MouseButton::Left) {
                            UI_RT.with(|rt| rt.borrow_mut().win_mut(win).scroll_drag = None);
                            v2.update(cx, |_, cx| cx.notify()).ok();
                            return;
                        }
                        let y = (grab_scroll + (f32::from(e.position.y) - grab_y) * per_px).clamp(0.0, max);
                        h.set_offset(gpui::point(px(0.0), px(-y)));
                        v2.update(cx, |_, cx| cx.notify()).ok();
                    });
                    window.on_mouse_event(move |_: &gpui::MouseUpEvent, _, _, _| {
                        UI_RT.with(|rt| rt.borrow_mut().win_mut(win).scroll_drag = None);
                    });
                    let v3 = view.clone();
                    window.on_mouse_event(move |e: &gpui::MouseMoveEvent, _, _, cx| {
                        let over = bounds.dilate(px(4.0)).contains(&e.position);
                        let was = UI_RT.with(|rt| rt.borrow().win(win).and_then(|w| w.scroll_seen.get(&key)).is_some_and(|s| s.2));
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

/// Scroll to a measured list row, including headers and the actual row pitch.
pub fn scroll_row_into_view(window: &Window, key: &'static str, row: usize, pitch: f32, scroll: &gpui::ScrollHandle) {
    let win = window.window_handle().window_id();
    let top = UI_RT.with(|rt| rt.borrow().win(win).and_then(|w| w.vlist_top.get(key).copied()));
    let Some(top) = top else { return };
    let viewport = scroll.bounds();
    let row_top = top + row as f32 * pitch;
    let row_bottom = row_top + pitch;
    let delta = if row_top < f32::from(viewport.top()) {
        f32::from(viewport.top()) - row_top
    } else if row_bottom > f32::from(viewport.bottom()) {
        f32::from(viewport.bottom()) - row_bottom
    } else { return };
    let mut offset = scroll.offset();
    offset.y = px((f32::from(offset.y) + delta).clamp(-f32::from(scroll.max_offset().y).max(0.0), 0.0));
    scroll.set_offset(offset);
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
    let top = UI_RT.with(|rt| rt.borrow().cur_win().vlist_top.get(key).copied());
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
                move |bounds, window, _| {
                    let win = window.window_handle().window_id();
                    UI_RT.with(|rt| {
                        rt.borrow_mut().win_mut(win).vlist_top.insert(key, f32::from(bounds.origin.y));
                    });
                },
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

/// Vertical shift of row `i` of list `key` while row `from` is dragged to
/// slot `to` (`drag`): the rows in between glide one `pitch` aside to make
/// room, easing there instead of jumping. `None` (no drag) snaps back.
pub fn drag_shift(key: &'static str, i: usize, drag: Option<(usize, usize)>, pitch: f32) -> f32 {
    UI_RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        let m = &mut rt.drag_shift;
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
    UI_RT.with(|rt| rt.borrow().drag_shift.values().any(|(_, t)| t.is_some_and(|t| t.elapsed().as_secs_f32() < 0.2)))
}
