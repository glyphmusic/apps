use super::*;

pub fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn fmt_ms(ms: Option<u64>) -> String {
    ms.map(|m| fmt_time(Duration::from_millis(m)))
        .unwrap_or_else(|| "-:--".into())
}

/// Fraction under the mouse on bar `id`, while the mouse is over it.
pub fn bar_hover(id: &str) -> Option<f32> {
    UI_RT.with(|rt| {
        let w = rt.borrow();
        let w = w.cur_win();
        (w.bar_hover == Some(id)).then(|| w.bar_hover_at.clamp(0.0, 1.0))
    })
}

thread_local! {
    /// Touchpad scrolling over a bar: pixels not yet turned into a step.
    static WHEEL_REST: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// Steps of one scroll event over a bar: +1 per wheel notch up, -1 down;
/// touchpads make a step every 40 px.
pub fn wheel_steps(e: &gpui::ScrollWheelEvent) -> i32 {
    match e.delta {
        gpui::ScrollDelta::Lines(p) => {
            if p.y.abs() < 0.01 { 0 } else { p.y.signum() as i32 }
        }
        gpui::ScrollDelta::Pixels(p) => WHEEL_REST.with(|r| {
            let total = r.get() + f32::from(p.y);
            let steps = (total / 40.0).trunc();
            r.set(total - steps * 40.0);
            steps as i32
        }),
    }
}

/// Mouse wheel over a seek bar: jumps by the seconds set in Settings >
/// Player per notch (nothing when that is off).
pub fn seek_wheel(cx: &mut Context<crate::app::Glyph>) -> impl Fn(&gpui::ScrollWheelEvent, &mut Window, &mut gpui::App) + 'static {
    cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| {
        let step = this.config.ui.wheel_seek_s;
        if step == 0 || this.duration().is_none() {
            return;
        }
        let n = wheel_steps(e);
        if n != 0 {
            this.seek_relative(n as f32 * step as f32);
            cx.notify();
        }
        cx.stop_propagation();
    })
}

/// Mouse wheel over a volume bar: louder / quieter by the percent set in
/// Settings > Player per notch (nothing when that is off).
pub fn volume_wheel(cx: &mut Context<crate::app::Glyph>) -> impl Fn(&gpui::ScrollWheelEvent, &mut Window, &mut gpui::App) + 'static {
    cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| {
        let step = this.config.ui.wheel_volume_pct;
        if step == 0 {
            return;
        }
        let n = wheel_steps(e);
        if n != 0 {
            let v = this.volume() + n as f32 * step as f32 / 100.0;
            // Lands on the steps (5 %, 10 %, ...), not in between.
            let k = step as f32 / 100.0;
            this.set_volume(((v / k).round() * k).clamp(0.0, 1.0));
            cx.notify();
        }
        cx.stop_propagation();
    })
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
                let win = window.window_handle().window_id();
                let (drag, bar_hover, bar_hover_at) = UI_RT.with(|rt| {
                    let rt = rt.borrow();
                    let w = rt.win(win);
                    (
                        w.and_then(|w| w.bar_drag).filter(|(d, _)| *d == id),
                        w.and_then(|w| w.bar_hover),
                        w.map_or(0.0, |w| w.bar_hover_at),
                    )
                });
                let shown = drag.map(|(_, f)| f).unwrap_or(frac).clamp(0.0, 1.0);
                let hot = drag.is_some() || bar_hover == Some(id);
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
                    let at = drag.map(|(_, f)| f).unwrap_or(bar_hover_at).clamp(0.0, 1.0);
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
                    UI_RT.with(|rt| rt.borrow_mut().win_mut(win).bar_drag = Some((id, f)));
                    if live {
                        p1(f, window, cx);
                    }
                    cx.stop_propagation();
                    v1.update(cx, |_, cx| cx.notify()).ok();
                });
                window.on_mouse_event(move |e: &gpui::MouseMoveEvent, _, window, cx| {
                    let over = hit.contains(&e.position);
                    let (was, dragging) = UI_RT.with(|rt| {
                        let mut rt = rt.borrow_mut();
                        let w = rt.win_mut(win);
                        let was = w.bar_hover == Some(id);
                        if over {
                            w.bar_hover_at = frac_at(f32::from(e.position.x));
                        }
                        if over != was {
                            w.bar_hover = if over { Some(id) } else { None };
                        }
                        let dragging = w.bar_drag.is_some_and(|(d, _)| d == id);
                        (was, dragging)
                    });
                    if over || over != was {
                        v2.update(cx, |_, cx| cx.notify()).ok();
                    }
                    if dragging {
                        if e.pressed_button != Some(gpui::MouseButton::Left) {
                            UI_RT.with(|rt| rt.borrow_mut().win_mut(win).bar_drag = None);
                            return;
                        }
                        let f = frac_at(f32::from(e.position.x));
                        UI_RT.with(|rt| rt.borrow_mut().win_mut(win).bar_drag = Some((id, f)));
                        if live {
                            p2(f, window, cx);
                        }
                        v2.update(cx, |_, cx| cx.notify()).ok();
                    }
                });
                window.on_mouse_event(move |e: &gpui::MouseUpEvent, _, window, cx| {
                    let released = UI_RT.with(|rt| {
                        let mut rt = rt.borrow_mut();
                        let w = rt.win_mut(win);
                        if w.bar_drag.is_some_and(|(d, _)| d == id) {
                            w.bar_drag = None;
                            true
                        } else {
                            false
                        }
                    });
                    if released {
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
            // Wraps the text within the page instead of running past it.
            .max_w_full()
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
                    .min_w_0()
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
