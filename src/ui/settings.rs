//! Settings as a modal (Liner layout): left navigation with search and
//! grouped pages, right the page with rows (label + description left,
//! control right). Pages: Playback, Audio, Appearance, Server, Cache &
//! Storage, About.

use crate::app::{BackendState, ConnTest, Glyph, SettingsPage};
use crate::i18n::{t, tf, Lang};
use crate::theme;
use crate::ui::widgets::*;
use glyph_core::config::AudioQuality;
use glyph_core::storage::{format_bytes, StorageKind};
use glyph_core::{BackendMode, UpdateStatus};
use glyph_protocol::ProviderId;
use gpui::{anchored, 
    canvas, deferred, div, hsla, point, prelude::*, px, AnyElement, Context, Div, FontWeight, Hsla,
    MouseButton, Pixels, SharedString, Window,
};
use std::rc::Rc;

pub const REPO_URL: &str = "https://github.com/glyphmusic/glyph";
const DOCS_URL: &str = "https://github.com/glyphmusic/glyph#readme";
const ISSUES_URL: &str = "https://github.com/glyphmusic/glyph/issues/new";
const RELEASES_URL: &str = "https://github.com/glyphmusic/glyph/releases";

const PANEL_W: f32 = 920.0;
const PANEL_H: f32 = 600.0;
const NAV_W: f32 = 232.0;

pub(crate) fn white(a: f32) -> Hsla {
    hsla(0.0, 0.0, 1.0, a)
}

// ------------------------------------------------------------------ pieces

fn switch(id: impl Into<gpui::ElementId>, on: bool) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .cursor_pointer()
        .w(px(36.0))
        .h(px(20.0))
        .rounded_full()
        .p(px(2.0))
        .flex()
        .when(on, |d| d.justify_end())
        .bg(if on { gpui::white() } else { white(0.14) })
        .child(div().size(px(16.0)).rounded_full().bg(if on { gpui::black() } else { white(0.75) }))
}

/// Settings row: label + description left, control right.
pub(crate) fn row(label: impl Into<SharedString>, desc: Option<SharedString>, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(24.0))
        .py(px(14.0))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .gap(px(3.0))
                .child(div().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(label.into()))
                .when_some(desc, |d, s| d.child(div().text_size(px(12.5)).text_color(white(0.55)).child(s))),
        )
        .child(div().flex_none().flex().items_center().gap(px(8.0)).child(control))
        .into_any_element()
}

/// Rows separated by hairlines.
/// Equalizer presets: name key and gains (dB) for 32 Hz .. 16 kHz.
pub const EQ_PRESETS: [(&str, [f32; 10]); 14] = [
    ("flat", [0.0; 10]),
    ("bass_boost", [6.0, 5.0, 4.0, 2.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("bass_reducer", [-6.0, -5.0, -4.0, -2.5, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("treble_boost", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.5, 4.0, 5.0, 6.0]),
    ("treble_reducer", [0.0, 0.0, 0.0, 0.0, 0.0, -1.0, -2.5, -4.0, -5.0, -6.0]),
    ("vocal", [-2.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.5, 2.0, 0.0, -1.0]),
    ("rock", [4.0, 3.0, 2.0, 0.5, -1.0, -1.0, 0.5, 2.0, 3.0, 4.0]),
    ("pop", [-1.0, 1.0, 2.5, 3.5, 2.0, 0.0, -0.5, -0.5, 1.0, 1.5]),
    ("hiphop", [5.0, 4.5, 2.0, 3.0, -1.0, -1.0, 1.0, -0.5, 1.5, 2.5]),
    ("electronic", [4.5, 4.0, 1.5, 0.0, -2.0, 2.0, 1.0, 1.5, 4.0, 5.0]),
    ("acoustic", [4.0, 4.0, 3.0, 1.0, 2.0, 1.5, 3.0, 3.5, 3.0, 2.0]),
    ("classical", [4.5, 3.5, 3.0, 2.5, -1.5, -1.5, 0.0, 2.0, 3.0, 3.5]),
    ("jazz", [3.5, 2.5, 1.5, 2.0, -1.5, -1.5, 0.0, 1.5, 2.5, 3.0]),
    ("small_speakers", [5.0, 4.0, 3.5, 2.5, 1.0, 0.0, -1.0, -2.0, -2.5, -3.0]),
];

fn eq_preset_label(key: &str) -> SharedString {
    t(&format!("eq.preset.{key}"))
}

/// Equalizer page: on/off, presets, one slider per band (-12..+12 dB).
fn equalizer(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let on = app.config.audio.eq_enabled;
    let gains = crate::app::eq_gains(&app.config.audio.eq_gains);
    let preset = app.config.audio.eq_preset.clone();
    let mut options: Vec<(SharedString, bool, Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>)> = EQ_PRESETS
        .iter()
        .map(|(k, g)| {
            let (k, g) = (*k, *g);
            let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_eq(true, k, g, true, cx));
            (eq_preset_label(k), preset == k, f)
        })
        .collect();
    if preset == "custom" {
        options.push((eq_preset_label("custom"), true, Rc::new(|_: &mut Glyph, _: &mut Context<Glyph>| {})));
    }
    let top = rows(vec![
        row(
            t("sp.eq.enable"),
            Some(t("sp.eq.enable.desc")),
            switch("eq-on", on).on_click(cx.listener(move |this, _, _, cx| {
                let g = crate::app::eq_gains(&this.config.audio.eq_gains);
                let p = this.config.audio.eq_preset.clone();
                this.set_eq(!on, &p, g, true, cx)
            })),
        ),
        row(t("sp.eq.preset"), None, select(app, "sel-eq", eq_preset_label(&preset), options, cx)),
    ]);
    div()
        .flex()
        .flex_col()
        .gap(px(18.0))
        .child(top)
        .child(eq_curve(gains, on, cx))
        .child(
            div()
                .flex()
                .justify_end()
                .child(button("eq-reset", t("sp.eq.reset")).on_click(cx.listener(|this, _, _, cx| {
                    let on = this.config.audio.eq_enabled;
                    this.set_eq(on, "flat", [0.0; 10], true, cx)
                }))),
        )
}

thread_local! {
    /// Band being dragged on the equalizer curve.
    static EQ_DRAG: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Spotify-style equalizer: a smooth curve through the ten bands with a
/// filled area below, points to drag up and down (-12..+12 dB).
fn eq_curve(gains: [f32; 10], on: bool, cx: &mut Context<Glyph>) -> Div {
    const NAMES: [&str; 10] = ["32Hz", "64Hz", "125Hz", "250Hz", "500Hz", "1kHz", "2kHz", "4kHz", "8kHz", "16kHz"];
    const LEFT: f32 = 52.0;
    const H: f32 = 210.0;
    let view = cx.entity().downgrade();
    let accent = theme::pink();
    let plot = gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _app_cx| {
            let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let cell = w / 10.0;
            let xs: Vec<f32> = (0..10).map(|i| x0 + cell * (i as f32 + 0.5)).collect();
            let y_of = move |g: f32| y0 + (12.0 - g.clamp(-12.0, 12.0)) / 24.0 * h;
            let line = |x: f32, y: f32, ww: f32, hh: f32, c: gpui::Hsla, window: &mut Window| {
                window.paint_quad(gpui::fill(gpui::Bounds { origin: gpui::point(px(x), px(y)), size: gpui::size(px(ww), px(hh)) }, c));
            };
            // Grid: a line per band, the 0 dB line.
            for &x in &xs {
                line(x - 0.5, y0, 1.0, h, white(0.06), window);
            }
            line(x0, y_of(0.0) - 0.5, w, 1.0, white(0.14), window);
            // Curve (Catmull-Rom through the points), sampled.
            let pts: Vec<(f32, f32)> = (0..10).map(|i| (xs[i], y_of(gains[i]))).collect();
            let mut curve: Vec<(f32, f32)> = Vec::new();
            for i in 0..9usize {
                let p0 = pts[i.saturating_sub(1)];
                let (p1, p2) = (pts[i], pts[i + 1]);
                let p3 = pts[(i + 2).min(9)];
                for k in 0..16 {
                    let t = k as f32 / 16.0;
                    let (t2, t3) = (t * t, t * t * t);
                    let f = |a: f32, b: f32, c: f32, d: f32| 0.5 * (2.0 * b + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3);
                    curve.push((f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1).clamp(y0, y0 + h)));
                }
            }
            curve.push(pts[9]);
            let alpha = if on { 1.0 } else { 0.4 };
            // Filled area down to the bottom, fading out.
            let mut fill = gpui::PathBuilder::fill();
            fill.move_to(gpui::point(px(curve[0].0), px(y0 + h)));
            for &(x, y) in &curve {
                fill.line_to(gpui::point(px(x), px(y)));
            }
            fill.line_to(gpui::point(px(curve[curve.len() - 1].0), px(y0 + h)));
            fill.close();
            if let Ok(path) = fill.build() {
                window.paint_path(
                    path,
                    gpui::linear_gradient(
                        180.0,
                        gpui::linear_color_stop(accent.opacity(0.45 * alpha), 0.0),
                        gpui::linear_color_stop(accent.opacity(0.02), 1.0),
                    ),
                );
            }
            let mut stroke = gpui::PathBuilder::stroke(px(2.5));
            stroke.move_to(gpui::point(px(curve[0].0), px(curve[0].1)));
            for &(x, y) in &curve[1..] {
                stroke.line_to(gpui::point(px(x), px(y)));
            }
            if let Ok(path) = stroke.build() {
                window.paint_path(path, accent.opacity(alpha));
            }
            // Points.
            let dragging = EQ_DRAG.with(|d| d.get());
            for (i, &(x, y)) in pts.iter().enumerate() {
                let r = if dragging == Some(i) { 7.0 } else { 5.0 };
                window.paint_quad(gpui::quad(
                    gpui::Bounds { origin: gpui::point(px(x - r), px(y - r)), size: gpui::size(px(r * 2.0), px(r * 2.0)) },
                    gpui::Corners::all(px(r)),
                    gpui::white().opacity(if on { 1.0 } else { 0.6 }),
                    gpui::Edges::default(),
                    gpui::transparent_black(),
                    gpui::BorderStyle::default(),
                ));
            }
            // Dragging a point (anywhere in its column picks it).
            let gain_at = move |y: f32| (((12.0 - (y - y0) / h * 24.0) * 2.0).round() / 2.0).clamp(-12.0, 12.0);
            let band_at = move |x: f32| (((x - x0) / cell).floor() as i32).clamp(0, 9) as usize;
            let hit = bounds.dilate(px(8.0));
            let (v1, v2, v3) = (view.clone(), view.clone(), view.clone());
            let apply = move |v: &gpui::WeakEntity<Glyph>, band: usize, g: f32, save: bool, cx: &mut gpui::App| {
                v.update(cx, |this, cx| {
                    let mut gs = crate::app::eq_gains(&this.config.audio.eq_gains);
                    gs[band] = g;
                    this.set_eq(true, "custom", gs, save, cx);
                })
                .ok();
            };
            window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, _, cx| {
                if phase != gpui::DispatchPhase::Bubble || e.button != gpui::MouseButton::Left || !hit.contains(&e.position) {
                    return;
                }
                let band = band_at(f32::from(e.position.x));
                EQ_DRAG.with(|d| d.set(Some(band)));
                apply(&v1, band, gain_at(f32::from(e.position.y)), false, cx);
                cx.stop_propagation();
            });
            window.on_mouse_event(move |e: &gpui::MouseMoveEvent, _, _, cx| {
                let Some(band) = EQ_DRAG.with(|d| d.get()) else { return };
                if e.pressed_button != Some(gpui::MouseButton::Left) {
                    EQ_DRAG.with(|d| d.set(None));
                    return;
                }
                apply(&v2, band, gain_at(f32::from(e.position.y)), false, cx);
            });
            window.on_mouse_event(move |e: &gpui::MouseUpEvent, _, _, cx| {
                if let Some(band) = EQ_DRAG.with(|d| d.get()) {
                    EQ_DRAG.with(|d| d.set(None));
                    apply(&v3, band, gain_at(f32::from(e.position.y)), true, cx);
                }
            });
        },
    )
    .size_full();
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            div()
                .relative()
                .h(px(H))
                .child(div().absolute().left_0().top(px(-7.0)).text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(white(0.7)).child("+12dB"))
                .child(div().absolute().left_0().top(px(H / 2.0 - 8.0)).text_size(px(12.0)).text_color(white(0.4)).child("0dB"))
                .child(div().absolute().left_0().bottom(px(-7.0)).text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(white(0.7)).child("-12dB"))
                .child(div().absolute().left(px(LEFT)).right_0().top_0().bottom_0().cursor_pointer().child(plot)),
        )
        .child(
            div()
                .flex()
                .ml(px(LEFT))
                .children(NAMES.iter().map(|n| div().flex_1().text_center().text_size(px(12.0)).font_weight(FontWeight::SEMIBOLD).text_color(white(0.65)).child(*n))),
        )
}

pub(crate) fn rows(items: Vec<AnyElement>) -> Div {
    let n = items.len();
    div().flex().flex_col().children(items.into_iter().enumerate().map(move |(i, r)| {
        div().when(i + 1 < n, |d| d.border_b_1().border_color(white(0.06))).child(r)
    }))
}

pub(crate) fn section_label(text: impl Into<SharedString>) -> Div {
    div()
        .pt(px(18.0))
        .pb(px(2.0))
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white(0.45))
        .child(text.into().to_uppercase())
}

/// A flat surface for grouped content (storage breakdown, slider).
fn surface() -> Div {
    div().rounded(px(10.0)).bg(white(0.035)).p(px(18.0))
}

fn link_button(id: &'static str, icon_name: &'static str, label: impl Into<SharedString>, url: &'static str) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.0))
        .h(px(32.0))
        .px(px(12.0))
        .rounded(px(8.0))
        .bg(white(0.07))
        .cursor_pointer()
        .hover(|s| s.bg(white(0.12)))
        .text_size(px(13.0))
        .text_color(gpui::white())
        .child(icon(icon_name, 14.0, white(0.8)))
        .child(label.into())
        .on_click(move |_, _, cx| cx.open_url(url))
}

/// Segmented control: one pill, the selected option filled.
pub(crate) fn segmented(items: Vec<(SharedString, bool, Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>)>, cx: &mut Context<Glyph>, id: &'static str) -> Div {
    div()
        .flex()
        .p(px(3.0))
        .gap(px(2.0))
        .rounded(px(9.0))
        .bg(white(0.05))
        .children(items.into_iter().enumerate().map(|(i, (label, on, f))| {
            div()
                .id((id, i))
                .px(px(12.0))
                .h(px(28.0))
                .flex()
                .items_center()
                .rounded(px(7.0))
                .cursor_pointer()
                .text_size(px(13.0))
                .when(on, |d| d.bg(white(0.12)).text_color(gpui::white()).font_weight(FontWeight::MEDIUM))
                .when(!on, |d| d.text_color(white(0.6)).hover(|s| s.text_color(gpui::white())))
                .on_click(cx.listener(move |this, _, _, cx| f(this, cx)))
                .child(label)
        }))
}

/// Dropdown select: a button with the current value; the open list floats
/// below it (right-aligned) with a check on the selected entry.
/// Dropdown open / close animation (same feel as the context menu).
const SEL_IN: f32 = 0.14;
const SEL_OUT: f32 = 0.11;

thread_local! {
    /// Open dropdown and since when; one closing and since when.
    static SEL_ANIM: std::cell::RefCell<(Option<&'static str>, std::time::Instant, Option<(&'static str, std::time::Instant)>)> =
        std::cell::RefCell::new((None, std::time::Instant::now(), None));
}

/// Notes which dropdown is open this frame (starts the open / close fades).
fn select_anim_tick(open: Option<&'static str>) {
    SEL_ANIM.with(|a| {
        let mut a = a.borrow_mut();
        if a.0 != open {
            if let Some(prev) = a.0 {
                a.2 = Some((prev, std::time::Instant::now()));
            }
            a.0 = open;
            a.1 = std::time::Instant::now();
        }
        if a.2.is_some_and(|(_, at)| at.elapsed().as_secs_f32() >= SEL_OUT) {
            a.2 = None;
        }
    });
}

/// A dropdown is still fading in or out (the app keeps rendering).
pub fn select_animating() -> bool {
    SEL_ANIM.with(|a| {
        let a = a.borrow();
        (a.0.is_some() && a.1.elapsed().as_secs_f32() < SEL_IN + 0.02) || a.2.is_some()
    })
}

thread_local! {
    /// Window y of each dropdown button's bottom edge (measured while painting).
    static SELECT_BOTTOM: std::cell::RefCell<std::collections::HashMap<&'static str, f32>> = std::cell::RefCell::new(Default::default());
}

fn select(
    app: &Glyph,
    id: &'static str,
    value: SharedString,
    options: Vec<(SharedString, bool, Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>)>,
    cx: &mut Context<Glyph>,
) -> Div {
    let open = app.open_select == Some(id);
    // Fade + slide in when opening; a closing list fades out without
    // taking clicks.
    let (alpha, live) = SEL_ANIM.with(|a| {
        let a = a.borrow();
        if open {
            let k = (a.1.elapsed().as_secs_f32() / SEL_IN).min(1.0);
            (1.0 - (1.0 - k).powi(3), true)
        } else if let Some((cid, at)) = a.2.filter(|(c, _)| *c == id) {
            let _ = cid;
            ((1.0 - at.elapsed().as_secs_f32() / SEL_OUT).clamp(0.0, 1.0), false)
        } else {
            (0.0, false)
        }
    });
    let show = open || alpha > 0.0;
    // The list always opens below the button at full size; what does not
    // fit above the window's bottom edge is cut off and scrolls.
    let room = SELECT_BOTTOM.with(|m| m.borrow().get(id).copied()).map(|b| crate::ui::widgets::viewport_h() - b - 14.0).unwrap_or(600.0);
    let list_scroll = crate::ui::widgets::handle_for(&format!("select:{id}"));
    let list = show.then(|| {
        let panel = div()
                .id((id, 999usize))
                .track_scroll(&list_scroll)
                .when(live, |d| d.occlude())
                .opacity(alpha)
                .mt(px(-6.0 * (1.0 - alpha)))
                .min_w(px(240.0))
                .max_h(px(room.max(120.0)))
                .overflow_y_scroll()
                .p(px(4.0))
                .rounded(px(10.0))
                .bg(hsla(0.0, 0.0, 0.09, 0.99))
                .border_1()
                .border_color(white(0.08))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .children(options.into_iter().enumerate().map(|(i, (label, on, f))| {
                    div()
                        .id((id, i))
                        // Full height always: a long list scrolls instead of squeezing.
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .h(px(36.0))
                        .px(px(10.0))
                        .rounded(px(7.0))
                        .cursor_pointer()
                        .text_size(px(13.5))
                        .when(on, |d| d.bg(white(0.08)).text_color(gpui::white()))
                        .when(!on, |d| d.text_color(white(0.7)).hover(|s| s.bg(white(0.05)).text_color(gpui::white())))
                        .when(live, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                this.open_select = None;
                                f(this, cx);
                                cx.notify();
                            }))
                        })
                        .child(label)
                        .when(on, |d| d.child(icon("check", 14.0, gpui::white())))
                }));
        let wheel = crate::ui::widgets::smooth_wheel(&list_scroll, false, cx);
        div()
            .absolute()
            .top(px(40.0))
            .right_0()
            .child(deferred(anchored().anchor(gpui::Anchor::TopRight).child(div().relative().child(panel).child(wheel))).with_priority(2))
    });
    div()
        .relative()
        .child(
            gpui::canvas(
                move |b, _, _| SELECT_BOTTOM.with(|m| m.borrow_mut().insert(id, f32::from(b.origin.y + b.size.height))),
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .child(
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(8.0))
                .h(px(34.0))
                .pl(px(12.0))
                .pr(px(10.0))
                .rounded(px(8.0))
                .bg(white(if open { 0.10 } else { 0.06 }))
                .border_1()
                .border_color(white(0.06))
                .cursor_pointer()
                .hover(|s| s.bg(white(0.10)))
                .text_size(px(13.5))
                .text_color(gpui::white())
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_select(id, cx)))
                .child(div().max_w(px(220.0)).truncate().child(value))
                .child(icon("chevron-down", 14.0, white(0.6))),
        )
        .children(list)
}

/// "Advanced" disclosure row; the section below only shows when expanded.
fn advanced_toggle(app: &Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    let open = app.settings_advanced;
    div()
        .id("sp-advanced")
        .flex()
        .items_center()
        .gap(px(6.0))
        .pt(px(18.0))
        .pb(px(4.0))
        .cursor_pointer()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(white(0.55))
        .hover(|s| s.text_color(gpui::white()))
        .on_click(cx.listener(|this, _, _, cx| {
            this.settings_advanced = !this.settings_advanced;
            cx.notify();
        }))
        .child(icon(if open { "chevron-down" } else { "chevron-right" }, 14.0, white(0.55)))
        .child(t("sp.advanced"))
        .into_any_element()
}

pub(crate) fn input_box(input: impl IntoElement) -> Div {
    div()
        .w(px(300.0))
        .h(px(34.0))
        .px(px(10.0))
        .flex()
        .items_center()
        .rounded(px(8.0))
        .bg(hsla(0.0, 0.0, 0.0, 0.45))
        .border_1()
        .border_color(white(0.08))
        .text_size(px(13.0))
        .font_family(theme::MONO)
        .child(input)
}

// ------------------------------------------------------------------ modal

pub fn modal(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    theme::on_panel(|| modal_inner(app, window, cx))
}

fn modal_inner(app: &mut Glyph, _window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    select_anim_tick(app.open_select);
    let page = app.settings_page;
    let (title, sub, body) = match page {
        SettingsPage::Playback => (t("sp.playback"), t("sp.playback.sub"), playback(app, cx)),
        SettingsPage::Audio => (t("sp.eq"), t("sp.eq.sub"), equalizer(app, cx)),
        SettingsPage::Appearance => (t("sp.appearance"), t("sp.appearance.sub"), appearance(app, cx)),
        SettingsPage::Server => (t("sp.server"), t("sp.server.sub"), server(app, cx)),
        SettingsPage::Account => (t("sp.account"), t("sp.account.sub"), crate::ui::account::page(app, cx)),
        SettingsPage::Storage => (t("sp.storage"), t("sp.storage.sub"), storage(app, cx)),
        SettingsPage::About => (t("sp.about"), t("sp.about.sub"), about(app, cx)),
        SettingsPage::Support => (t("sp.support"), t("sp.support.sub"), support(app, cx)),
    };

    // Outer shell holds the navigation; the page sits in its own inset,
    // bordered card next to it.
    let page_bg = hsla(0.0, 0.0, 0.045, 1.0);
    let content = div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .rounded(px(12.0))
        .overflow_hidden()
        .bg(page_bg)
        .border_1()
        .border_color(white(0.08))
        .child(
            fade_edges(
                div()
                    .id("settings-page")
                    .track_scroll(&app.settings_scroll)
                    .size_full()
                    .overflow_y_scroll()
                    .px(px(28.0))
                    .py(px(22.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .pr(px(36.0))
                            .child(div().text_size(px(17.0)).font_weight(FontWeight::SEMIBOLD).text_color(gpui::white()).child(title))
                            .child(div().text_size(px(13.0)).text_color(white(0.55)).child(sub)),
                    )
                    .child(div().pt(px(10.0)).child(body)),
                page_bg,
                22.0,
                Some(&app.settings_scroll),
            )
            .size_full()
            .child(scrollbar(&app.settings_scroll, true, cx)),
        )
        .child(
            div()
                .id("settings-close")
                .absolute()
                .top(px(14.0))
                .right(px(14.0))
                .size(px(30.0))
                .rounded(px(8.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(white(0.08)))
                .on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx)))
                .child(icon("x", 16.0, white(0.7))),
        );

    let lift = (1.0 - app.settings_k()) * 14.0;
    let panel = div()
        .id("settings-panel")
        .occlude()
        .relative()
        .mt(px(lift))
        .w(px(PANEL_W))
        .max_w(gpui::relative(0.94))
        .h(px(PANEL_H))
        .max_h(gpui::relative(0.9))
        .flex()
        .p(px(8.0))
        .gap(px(4.0))
        .rounded(px(16.0))
        .overflow_hidden()
        .bg(hsla(0.0, 0.0, 0.08, 0.985))
        .border_1()
        .border_color(white(0.07))
        .font_family(theme::FONT)
        // Clicks inside never reach the backdrop (which closes the modal).
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(nav(app, cx))
        .child(content);

    div()
        .id("settings-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(0.0, 0.0, 0.0, 0.30))
        .on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx)))
        .child(panel)
        .into_any_element()
}

fn nav(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let q = app.settings_search.read(cx).value.trim().to_lowercase();
    let current = app.settings_page;
    // (page, icon, label, extra search words)
    let groups: [(SharedString, Vec<(SettingsPage, &'static str, SharedString, &'static str)>); 2] = [
        (
            t("sp.group.prefs"),
            vec![
                (SettingsPage::Playback, "play", t("sp.playback"), "sources source quelle youtube soundcloud priority radio quality qualität audio"),
                (SettingsPage::Audio, "volume-2", t("sp.eq"), "equalizer eq bass treble höhen tiefen klang sound preset"),
                (SettingsPage::Appearance, "palette", t("sp.appearance"), "language sprache theme ascii cover glow"),
            ],
        ),
        (
            t("sp.group.app"),
            vec![
                (SettingsPage::Account, "user-round", t("sp.account"), "account konto login sign in anmelden sync devices geräte handle passkey"),
                (SettingsPage::Server, "server", t("sp.server"), "url api key keys schlüssel"),
                (SettingsPage::Storage, "database", t("sp.storage"), "cache download downloads speicher"),
                (SettingsPage::About, "info", t("sp.about"), "update version"),
            ],
        ),
    ];
    let matches = |label: &SharedString, words: &str| q.is_empty() || label.to_lowercase().contains(&q) || words.contains(&q);
    let item = |page: SettingsPage, icon_name: &'static str, label: SharedString, cx: &mut Context<Glyph>| {
        let on = page == current;
        div()
            .id(("sp-nav", page as usize))
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(36.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(px(14.0))
            .when(on, |d| d.bg(white(0.08)).text_color(gpui::white()))
            .when(!on, |d| d.text_color(white(0.75)).hover(|s| s.bg(white(0.04)).text_color(gpui::white())))
            .on_click(cx.listener(move |this, _, _, cx| this.set_settings_page(page, cx)))
            .child(icon(icon_name, 16.0, if on { gpui::white() } else { white(0.7) }))
            .child(label)
    };

    let mut col = div()
        .flex_none()
        .w(px(NAV_W))
        .h_full()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .p(px(6.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .h(px(34.0))
                .px(px(10.0))
                .mb(px(6.0))
                .rounded(px(8.0))
                .bg(white(0.06))
                .text_size(px(13.0))
                .child(icon("search", 14.0, white(0.5)))
                .child(div().flex_1().min_w_0().child(app.settings_search.clone())),
        );
    for (group, items) in groups {
        let visible: Vec<_> = items.into_iter().filter(|(_, _, l, w)| matches(l, w)).collect();
        if visible.is_empty() {
            continue;
        }
        col = col.child(div().px(px(10.0)).pt(px(10.0)).pb(px(4.0)).text_size(px(12.0)).text_color(white(0.45)).child(group));
        for (page, ic, label, _) in visible {
            col = col.child(item(page, ic, label, cx));
        }
    }
    col
}

// ------------------------------------------------------------------ pages

fn playback(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    // Everyday options first; sources and their order live under "Advanced".
    let cur = app.config.audio.quality;
    let options = [AudioQuality::Low, AudioQuality::Standard, AudioQuality::High]
        .into_iter()
        .map(|q| {
            let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_audio_quality(q, cx));
            (quality_label(q), q == cur, f)
        })
        .collect();
    let quality = select(app, "sel-quality", quality_label(cur), options, cx);
    let cur_dl = app.config.audio.download_quality;
    let dl_options = [AudioQuality::Low, AudioQuality::Standard, AudioQuality::High]
        .into_iter()
        .map(|q| {
            let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_download_quality(q, cx));
            (quality_label(q), q == cur_dl, f)
        })
        .collect();
    let dl_quality = select(app, "sel-dl-quality", quality_label(cur_dl), dl_options, cx);
    let basic = rows(vec![
        row(t("sp.audio.quality"), Some(t("sp.audio.quality.desc")), quality),
        row(t("sp.audio.dl_quality"), Some(t("sp.audio.dl_quality.desc")), dl_quality),
        row(t("sp.crossfade"), Some(t("sp.crossfade.desc")), {
            let cur = app.config.audio.crossfade_s;
            let label = |s: u32| -> String { if s == 0 { t("sp.crossfade.off").to_string() } else { format!("{s} s") } };
            let weak = cx.entity().downgrade();
            let pick: crate::ui::widgets::Pick = Rc::new(move |f, _, cx| {
                weak.update(cx, |this, cx| this.set_crossfade((f * 20.0).round() as u32, cx)).ok();
            });
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(crate::ui::widgets::thin_bar("crossfade", 180.0, cur as f32 / 20.0, pick, Some(Rc::new(move |f: f32| label((f * 20.0).round() as u32)) as crate::ui::widgets::BarLabel), cx))
                .child(div().w(px(36.0)).text_size(px(13.0)).text_color(white(0.75)).child(label(cur)))
        }),
        row(t("sp.output"), Some(t("sp.output.desc")), {
            let cur = app.config.audio.output_device.clone();
            let mut options: Vec<(SharedString, bool, Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>)> =
                vec![(t("sp.output.default"), cur.is_none(), Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_output_device(None, cx)))];
            let mut names = app.output_devices.clone();
            if let Some(c) = &cur {
                if !names.contains(c) {
                    names.push(c.clone());
                }
            }
            for n in names {
                let sel = cur.as_deref() == Some(n.as_str());
                let n2 = n.clone();
                options.push((n.into(), sel, Rc::new(move |this: &mut Glyph, cx: &mut Context<Glyph>| this.set_output_device(Some(n2.clone()), cx))));
            }
            let label: SharedString = cur.map(Into::into).unwrap_or_else(|| t("sp.output.default"));
            select(app, "sel-output", label, options, cx)
        }),
        row(t("sp.level"), Some(t("sp.level.desc")), {
            use glyph_core::config::VolumeLevel as V;
            let cur = app.config.audio.level;
            let label = |l: V| -> SharedString {
                match l {
                    V::Loud => t("sp.level.loud"),
                    V::Normal => t("sp.level.normal"),
                    V::Quiet => t("sp.level.quiet"),
                }
            };
            let options = [V::Loud, V::Normal, V::Quiet]
                .into_iter()
                .map(|l| {
                    let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_volume_level(l, cx));
                    (label(l), l == cur, f)
                })
                .collect();
            select(app, "sel-level", label(cur), options, cx)
        }),
        row(
            t("sp.normalize"),
            Some(t("sp.normalize.desc")),
            switch("normalize", app.config.audio.normalize).on_click(cx.listener(|this, _, _, cx| this.toggle_normalize(cx))),
        ),
        row(
            t("sp.explicit"),
            Some(t("sp.explicit.desc")),
            switch("hide-explicit", app.config.audio.hide_explicit).on_click(cx.listener(|this, _, _, cx| {
                this.config.audio.hide_explicit = !this.config.audio.hide_explicit;
                this.save_config(cx);
                cx.notify();
            })),
        ),
        row(
            t("settings.sources.fallback"),
            Some(t("settings.sources.fallback.hint")),
            switch("embed-fallback", app.embed_fallback()).on_click(cx.listener(|this, _, _, cx| this.toggle_embed_fallback(cx))),
        ),
        // Local yt-dlp plays YouTube directly (only matters with YouTube on).
        row(t("ytdlp.title"), Some(crate::ui::ytdlp::description(app)), crate::ui::ytdlp::control(app, cx)),
        row(
            t("sp.radio"),
            Some(t("sp.radio.desc")),
            switch("radio", app.radio).on_click(cx.listener(|this, _, _, cx| {
                this.radio = !this.radio;
                cx.notify();
            })),
        ),
    ]);

    // Artists hidden with "Don't show this artist", each with "Show again".
    let hidden = app.library.hidden_artists.clone();
    let hidden_row = row(
        t("sp.hidden"),
        Some(if hidden.is_empty() { t("sp.hidden.none") } else { t("sp.hidden.desc") }),
        div().flex().flex_wrap().justify_end().gap(px(6.0)).max_w(px(320.0)).children(hidden.into_iter().enumerate().map(|(i, name)| {
            let n = name.clone();
            div()
                .id(("hidden-artist", i))
                .flex()
                .items_center()
                .gap(px(6.0))
                .h(px(28.0))
                .pl(px(10.0))
                .pr(px(6.0))
                .rounded(px(8.0))
                .bg(white(0.06))
                .border_1()
                .border_color(white(0.08))
                .text_size(px(12.5))
                .text_color(white(0.85))
                .child(name)
                .child(
                    div()
                        .id(("hidden-show", i))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(18.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(white(0.12)))
                        .tooltip(|_, cx| cx.new(|_| Tip(t("ctx.show_artist"))).into())
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_hidden_artist(&n, cx)))
                        .child(icon("eye", 12.0, white(0.7))),
                )
        })),
    );
    let mut col = div().flex().flex_col().child(basic).child(rows(vec![hidden_row])).child(advanced_toggle(app, cx));
    if app.settings_advanced {
        let enabled = app.config.providers.enabled.clone();
        let sources: Vec<AnyElement> = [ProviderId::Audius, ProviderId::Jamendo, ProviderId::Archive]
            .into_iter()
            .map(|p| {
                let cooling = app
                    .backend
                    .as_ref()
                    .and_then(|b| crate::guard::guard(|| b.provider_cooling_down(p)))
                    .unwrap_or(false);
                row(
                    p.display_name(),
                    Some(if cooling { t("settings.sources.paused") } else { t(&format!("sp.src.{}", p.badge().to_lowercase())) }),
                    switch(("prov", p as usize), enabled.contains(&p)).on_click(cx.listener(move |this, _, _, cx| this.toggle_provider(p, cx))),
                )
            })
            .collect();
        col = col
            .child(rows(sources))
            .child(section_label(t("settings.priority")))
            .child(div().pb(px(8.0)).text_size(px(12.5)).text_color(white(0.55)).child(t("settings.priority.hint")))
            .child(priority_list(app, cx));
    }
    col
}

/// The dragged priority entry: the whole row follows the cursor.
#[derive(Clone)]
struct PrioDrag {
    from: usize,
    provider: ProviderId,
    width: Pixels,
}

fn prio_row_content(i: usize, p: ProviderId) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(10.0))
        .child(icon("grip-vertical", 15.0, white(0.4)))
        .child(mono().w(px(18.0)).text_size(px(13.0)).text_color(white(0.45)).child(format!("{}", i + 1)))
        .child(div().flex_1().text_size(px(14.0)).text_color(gpui::white()).child(p.display_name()))
        .child(badge(p))
}

impl Render for PrioDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        theme::on_panel(|| {
            prio_row_content(self.from, self.provider)
                .w(self.width)
                .rounded(px(8.0))
                .bg(hsla(0.0, 0.0, 0.17, 0.98))
                .border_1()
                .border_color(white(0.12))
                .font_family(theme::FONT)
                .cursor_grabbing()
        })
    }
}

fn priority_list(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let width = px(PANEL_W - NAV_W - 64.0);
    div()
        .flex()
        .flex_col()
        .rounded(px(10.0))
        .bg(white(0.035))
        .children(app.config.providers.priority.clone().into_iter().enumerate().map(|(i, p)| {
            div()
                .id(("prio", i))
                .rounded(px(8.0))
                .cursor_grab()
                .hover(|s| s.bg(white(0.03)))
                .on_drag(PrioDrag { from: i, provider: p, width }, |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<PrioDrag>(|s, _, _, _| s.bg(white(0.08)))
                .on_drop(cx.listener(move |this, d: &PrioDrag, _, cx| this.reorder_priority(d.from, i, cx)))
                .child(prio_row_content(i, p))
        }))
}

fn quality_label(q: AudioQuality) -> SharedString {
    match q {
        AudioQuality::Low => t("sp.audio.q.low"),
        AudioQuality::Standard => t("sp.audio.q.standard"),
        AudioQuality::High => t("sp.audio.q.high"),
    }
}

fn appearance(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let cur = app.lang;
    let options = Lang::ALL
        .into_iter()
        .map(|l| {
            let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_language(l, cx));
            (SharedString::from(l.native_name()), l == cur, f)
        })
        .collect();
    let language = select(app, "sel-lang", cur.native_name().into(), options, cx);

    let weak = cx.entity().downgrade();
    let set_int: Pick = Rc::new(move |f, _, cx| {
        weak.update(cx, |this, cx| this.set_intensity(f, cx)).ok();
    });
    let intensity = app.config.visuals.intensity;
    rows(vec![
        row(t("settings.language"), Some(t("sp.appearance.lang.desc")), language),
        row(
            t("settings.visuals.intensity"),
            Some(t("sp.appearance.intensity.desc")),
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(thin_bar("intensity", 160.0, intensity, set_int, None, cx))
                .child(mono().w(px(36.0)).flex().justify_end().text_size(px(12.0)).text_color(white(0.6)).child(format!("{:.0}%", intensity * 100.0))),
        ),
        row(
            t("settings.visuals.ascii_covers"),
            Some(t("sp.appearance.ascii.desc")),
            switch("ascii-covers", app.config.visuals.ascii_covers).on_click(cx.listener(|this, _, _, cx| {
                this.config.visuals.ascii_covers = !this.config.visuals.ascii_covers;
                this.save_config(cx);
                cx.notify();
            })),
        ),
        row(
            t("sp.appearance.startup"),
            Some(t("sp.appearance.startup.desc")),
            switch("startup-anim", app.config.visuals.startup_animation).on_click(cx.listener(|this, _, _, cx| {
                this.config.visuals.startup_animation = !this.config.visuals.startup_animation;
                this.save_config(cx);
                cx.notify();
            })),
        ),
    ])
}

fn server(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let mode: String = match &app.backend_state {
        BackendState::Connecting => format!("{} {}", app.spinner(), t("status.connecting")),
        BackendState::Ready(BackendMode::Server { url, server_version }) => {
            // The public server is just "Glyph server"; your own shows its host.
            let name = if url.trim_end_matches('/') == glyph_core::config::DEFAULT_SERVER_URL {
                t("settings.server.public").to_string()
            } else {
                url.split("://").nth(1).unwrap_or(url).trim_end_matches('/').to_string()
            };
            tf("settings.server.connected", &[("name", &name), ("v", server_version)]).to_string()
        }
        BackendState::Ready(BackendMode::Direct) => t("status.direct").to_string(),
        BackendState::Ready(BackendMode::Incompatible { protocol_version, .. }) => tf("status.incompatible", &[("v", protocol_version)]).to_string(),
        BackendState::Offline(e) => format!("{} {e}", t("status.backend_offline")),
    };
    let conn = match &app.conn_test {
        None => None,
        Some(ConnTest::Running) => Some((format!("{} {}", app.spinner(), t("settings.server.testing")), white(0.6))),
        Some(ConnTest::Ok(s)) => Some((s.clone(), theme::ok())),
        Some(ConnTest::Failed(s)) => Some((s.clone(), theme::warn())),
    };
    let mut items = vec![
        row(t("settings.server.url"), Some(t("sp.server.url.desc")), input_box(app.server_input.clone())),
        row(
            mode,
            conn.as_ref().map(|(s, _)| SharedString::from(s.clone())),
            div()
                .flex()
                .gap(px(8.0))
                .child(button("conn-test", t("settings.server.test")).on_click(cx.listener(|this, _, _, cx| this.test_connection(cx))))
                .child(button("conn-apply", t("settings.server.apply")).on_click(cx.listener(|this, _, _, cx| this.apply_server(cx)))),
        ),
    ];
    let keys: Vec<AnyElement> = app
        .key_inputs
        .iter()
        .map(|(name, input)| row(t(&format!("settings.keys.{name}")), None, input_box(input.clone())))
        .collect();
    let mut col = div().flex().flex_col().child(rows(items)).child(advanced_toggle(app, cx));
    if app.settings_advanced {
        col = col
            .child(section_label(t("settings.keys")))
            .child(div().pb(px(4.0)).text_size(px(12.5)).text_color(white(0.55)).child(t("settings.keys.hint")))
            .child(rows(keys));
    }
    col
}

const STORAGE_COLORS: [u32; 4] = [0x22c3e6, 0xa970ff, 0xff5fa2, 0xf2b84b];
const LIMITS_MB: [u64; 7] = [250, 1024, 2048, 3072, 5120, 10240, 0];

fn limit_label(mb: u64) -> SharedString {
    match mb {
        0 => t("sp.storage.nolimit"),
        m if m < 1024 => format!("{m} MB").into(),
        m => format!("{} GB", m / 1024).into(),
    }
}

fn storage_label(k: StorageKind) -> SharedString {
    match k {
        StorageKind::Covers => t("sp.storage.covers"),
        StorageKind::Downloads => t("sp.storage.downloads"),
        StorageKind::Lyrics => t("sp.storage.lyrics"),
        StorageKind::Catalog => t("sp.storage.catalog"),
    }
}

/// Ring chart of the storage split, drawn as filled ring segments.
fn donut(parts: Vec<(f32, Hsla)>, size: f32, thickness: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let c = bounds.center();
            let r_out = size / 2.0;
            let r_in = r_out - thickness;
            let total: f32 = parts.iter().map(|p| p.0).sum();
            let ring = |a0: f32, a1: f32, color: Hsla, window: &mut Window| {
                let steps = (((a1 - a0) / std::f32::consts::TAU) * 96.0).ceil().max(2.0) as usize;
                let at = |r: f32, a: f32| point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
                let mut b = gpui::PathBuilder::fill();
                b.move_to(at(r_out, a0));
                for i in 1..=steps {
                    b.line_to(at(r_out, a0 + (a1 - a0) * i as f32 / steps as f32));
                }
                for i in (0..=steps).rev() {
                    b.line_to(at(r_in, a0 + (a1 - a0) * i as f32 / steps as f32));
                }
                b.close();
                if let Ok(path) = b.build() {
                    window.paint_path(path, color);
                }
            };
            let start = -std::f32::consts::FRAC_PI_2;
            if total <= 0.0 {
                ring(start, start + std::f32::consts::TAU - 0.0001, white(0.08), window);
                return;
            }
            let mut a = start;
            for (v, color) in &parts {
                if *v <= 0.0 {
                    continue;
                }
                let sweep = v / total * std::f32::consts::TAU;
                ring(a, a + sweep - 0.0001, *color, window);
                a += sweep;
            }
        },
    )
    .size(px(size))
}

/// Filled checkbox: white square with a black check when on.
fn checkbox(on: bool) -> Div {
    div()
        .flex_none()
        .size(px(16.0))
        .rounded(px(4.0))
        .flex()
        .items_center()
        .justify_center()
        .when(on, |d| d.bg(gpui::white()).child(icon("check", 12.0, gpui::black())))
        .when(!on, |d| d.border_1().border_color(white(0.35)))
}

fn storage(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let Some(stats) = app.storage else {
        return div().child(skeleton("storage-sk", 520.0, 160.0, 10.0));
    };
    let total = stats.total();
    let selected = app.storage_selected;
    let legend = StorageKind::ALL.into_iter().enumerate().map(|(i, k)| {
        let v = stats.get(k);
        let pct = if total > 0 { v as f32 / total as f32 * 100.0 } else { 0.0 };
        let pct = if v > 0 && pct < 1.0 { "<1%".to_string() } else { format!("{pct:.0}%") };
        let on = selected[i];
        div()
            .id(("storage-kind", i))
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(36.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .cursor_pointer()
            .text_size(px(13.5))
            .hover(|s| s.bg(white(0.05)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.storage_selected[i] = !this.storage_selected[i];
                cx.notify();
            }))
            .child(checkbox(on))
            .child(div().ml(px(4.0)).size(px(8.0)).rounded_full().bg(gpui::rgb(STORAGE_COLORS[i])))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { gpui::white() } else { white(0.55) })
                    .child(storage_label(k)),
            )
            .child(
                mono()
                    .px(px(6.0))
                    .rounded(px(4.0))
                    .bg(white(0.08))
                    .text_size(px(11.0))
                    .text_color(white(0.6))
                    .child(pct),
            )
            .child(div().flex_1())
            .child(
                mono()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { white(0.85) } else { white(0.45) })
                    .child(format_bytes(v)),
            )
    });
    let parts: Vec<(f32, Hsla)> = StorageKind::ALL
        .into_iter()
        .enumerate()
        .map(|(i, k)| (stats.get(k) as f32, gpui::rgb(STORAGE_COLORS[i]).into()))
        .collect();
    let (num, unit) = {
        let s = format_bytes(total);
        let mut it = s.splitn(2, ' ');
        (it.next().unwrap_or("0").to_string(), it.next().unwrap_or("B").to_string())
    };
    let breakdown = surface()
        .p(px(14.0))
        .flex()
        .items_center()
        .gap(px(28.0))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.0)).children(legend))
        .child(
            div()
                .flex_none()
                .pr(px(16.0))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .relative()
                        .size(px(116.0))
                        .child(donut(parts, 116.0, 14.0))
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .child(div().text_size(px(22.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child(num))
                                .child(div().text_size(px(10.5)).text_color(white(0.6)).child(unit.to_uppercase())),
                        ),
                )
                .child(div().text_size(px(12.0)).text_color(white(0.6)).child(t("sp.storage.usage"))),
        );

    // Cache limit: a slider track with tick stops, filled up to the current one.
    let cur = app.config.storage.cache_limit_mb;
    let cur_i = LIMITS_MB.iter().position(|m| *m == cur);
    let n = LIMITS_MB.len();
    let stops = LIMITS_MB.into_iter().enumerate().map(|(i, mb)| {
        let on = Some(i) == cur_i;
        let passed = cur_i.map(|c| i <= c).unwrap_or(false);
        let out_filled = cur_i.map(|c| i < c).unwrap_or(false);
        // Each stop draws the half of the track into it and the half out of it.
        let seg = |filled: bool, visible: bool| {
            div().flex_1().h(px(2.0)).when(visible, |d| d.bg(if filled { gpui::white() } else { white(0.18) }))
        };
        let mark = if on {
            div().size(px(14.0)).rounded_full().bg(gpui::white())
        } else {
            div().w(px(1.5)).h(px(8.0)).bg(if passed { gpui::white() } else { white(0.3) })
        };
        div()
            .id(("limit", i))
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.0))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.set_cache_limit(mb, cx)))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(16.0))
                    .flex()
                    .items_center()
                    .child(seg(passed, i > 0))
                    .child(seg(out_filled, i + 1 < n))
                    .child(div().absolute().inset_0().flex().items_center().justify_center().child(mark)),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .when(on, |d| d.font_weight(FontWeight::SEMIBOLD).text_color(gpui::white()))
                    .when(!on, |d| d.text_color(white(0.5)).hover(|s| s.text_color(white(0.8))))
                    .child(limit_label(mb)),
            )
    });
    let limit = surface()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(div().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(t("sp.storage.limit")))
        .child(div().text_size(px(12.5)).text_color(white(0.55)).child(t("sp.storage.limit.desc")))
        .child(div().pt(px(20.0)).flex().children(stops));

    // Bottom actions: clear the checked kinds, open the cache folder.
    let sel_bytes: u64 = StorageKind::ALL
        .into_iter()
        .enumerate()
        .filter(|(i, _)| selected[*i])
        .map(|(_, k)| stats.get(k))
        .sum();
    let can_clear = sel_bytes > 0;
    let clear = div()
        .id("storage-clear")
        .flex_1()
        .h(px(40.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.0))
        .bg(gpui::white())
        .text_size(px(13.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(gpui::black())
        .when(can_clear, |d| {
            d.cursor_pointer().hover(|s| s.opacity(0.88)).on_click(cx.listener(move |this, _, _, cx| {
                for (i, k) in StorageKind::ALL.into_iter().enumerate() {
                    if this.storage_selected[i] {
                        this.clear_storage(k, cx);
                    }
                }
            }))
        })
        .when(!can_clear, |d| d.opacity(0.4))
        .child(tf("sp.storage.clearsel", &[("size", &format_bytes(sel_bytes))]));
    let data_dir = glyph_core::paths::data_dir();
    let open = div()
        .id("storage-open")
        .flex_none()
        .h(px(40.0))
        .px(px(18.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(10.0))
        .bg(white(0.07))
        .cursor_pointer()
        .hover(|s| s.bg(white(0.11)))
        .text_size(px(13.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(gpui::white())
        .on_click(move |_, _, cx| cx.reveal_path(&data_dir))
        .child(icon("folder-open", 15.0, white(0.85)))
        .child(t("sp.storage.openfolder"));

    div()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .pt(px(6.0))
        .child(breakdown)
        .child(limit)
        .child(div().pt(px(6.0)).flex().gap(px(10.0)).child(clear).child(open))
}

fn about(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let u = &app.update;
    let status: (String, Hsla) = if u.checking {
        (format!("{} {}", app.spinner(), t("update.checking")), white(0.6))
    } else if let Some((done, total)) = u.installing {
        let pct = if total > 0 { done * 100 / total } else { 0 };
        (tf("update.downloading", &[("p", &pct)]).to_string(), gpui::white())
    } else if u.installed {
        (t("update.installed").to_string(), theme::ok())
    } else if let Some(e) = &u.error {
        (tf("update.error", &[("e", e)]).to_string(), theme::warn())
    } else {
        match &u.status {
            None => (t("update.not_checked").to_string(), white(0.55)),
            Some(UpdateStatus::UpToDate { current }) => (tf("update.up_to_date", &[("v", current)]).to_string(), theme::ok()),
            Some(UpdateStatus::Skipped) => (t("update.skipped").to_string(), white(0.55)),
            Some(UpdateStatus::Available(i)) => (tf("update.badge", &[("v", &i.version.trim_start_matches('v'))]).to_string(), theme::ok()),
        }
    };
    let avail = app.available_update().cloned();
    let installable = avail
        .clone()
        .filter(|i| i.can_self_install && !app.update.installed && app.update.installing.is_none());
    let prerelease = app.config.updates.channel == "prerelease";

    let card = surface()
        .mt(px(10.0))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(10.0))
        .py(px(28.0))
        .child(gpui::img("icons/app/icon-256.png").size(px(64.0)).rounded(px(14.0)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(div().text_size(px(20.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).child("Glyph"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .px(px(7.0))
                        .h(px(20.0))
                        .rounded(px(6.0))
                        .bg(white(0.08))
                        .child(div().size(px(6.0)).rounded_full().bg(theme::ok()))
                        .child(mono().text_size(px(11.5)).text_color(white(0.85)).child(format!("v{}", env!("CARGO_PKG_VERSION")))),
                ),
        )
        .child(
            mono()
                .text_size(px(11.5))
                .text_color(white(0.5))
                .child(tf("sp.about.client", &[("v", &env!("CARGO_PKG_VERSION")), ("p", &glyph_protocol::PROTOCOL_VERSION), ("target", &glyph_core::updater::current_target())])),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .pt(px(6.0))
                .child(
                    div()
                        .id("upd-check")
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .h(px(34.0))
                        .px(px(14.0))
                        .rounded(px(8.0))
                        .bg(white(0.08))
                        .cursor_pointer()
                        .hover(|s| s.bg(white(0.13)))
                        .text_size(px(13.5))
                        .text_color(gpui::white())
                        .on_click(cx.listener(|this, _, _, cx| this.check_updates(true, cx)))
                        .child(icon("refresh-cw", 14.0, white(0.85)))
                        .child(t("update.check")),
                )
                .when_some(installable, |d, info| {
                    d.child(primary_button("upd-install", t("update.install")).on_click(cx.listener(move |this, _, _, cx| this.install_update(info.clone(), cx))))
                })
                .when(app.update.installed, |d| {
                    d.child(primary_button("upd-restart", t("update.restart")).on_click(cx.listener(|this, _, _, cx| this.restart(cx))))
                }),
        )
        .child(mono().text_size(px(11.5)).text_color(status.1).child(status.0))
        .child(
            div()
                .flex()
                .gap(px(8.0))
                .pt(px(14.0))
                .child(link_button("lnk-gh", "github", "GitHub", REPO_URL))
                .child(link_button("lnk-rel", "external-link", t("sp.link.releases"), RELEASES_URL))
                .child(link_button("lnk-sup", "life-buoy", t("sp.support"), ISSUES_URL)),
        );

    let channel = segmented(
        vec![
            (t("update.channel.stable"), !prerelease, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_update_channel(false, cx))),
            (t("update.channel.prerelease"), prerelease, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_update_channel(true, cx))),
        ],
        cx,
        "seg-channel",
    );
    let mut items = vec![
        row(
            t("update.auto"),
            Some(t("sp.about.auto.desc")),
            switch("upd-auto", app.config.updates.auto_install).on_click(cx.listener(|this, _, _, cx| this.toggle_auto_update(cx))),
        ),
        row(t("update.channel"), Some(t("sp.about.channel.desc")), channel),
    ];
    if let Some(i) = avail.filter(|i| !i.can_self_install) {
        items.push(div().py(px(12.0)).text_size(px(12.5)).text_color(white(0.55)).child(tf("update.package_manager", &[("url", &i.html_url)])).into_any_element());
    }
    div().flex().flex_col().gap(px(10.0)).child(card).child(rows(items))
}

fn support(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let diag = app.diagnostics();
    let data_dir = glyph_core::paths::data_dir();
    rows(vec![
        row(
            t("sp.support.issue"),
            Some(t("sp.support.issue.desc")),
            link_button("sup-issue", "external-link", t("sp.support.open"), ISSUES_URL),
        ),
        row(
            t("sp.support.diag"),
            Some(t("sp.support.diag.desc")),
            button("sup-diag", t("sp.support.copy")).on_click(cx.listener(move |this, _, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(diag.clone()));
                this.toast(t("sp.support.copied").to_string());
                cx.notify();
            })),
        ),
        row(
            t("sp.docs"),
            Some(t("sp.support.docs.desc")),
            link_button("sup-docs", "book-open", t("sp.support.open"), DOCS_URL),
        ),
        row(
            t("sp.support.data"),
            Some(t("sp.support.data.desc")),
            button("sup-data", t("sp.storage.open")).on_click(move |_, _, cx| cx.reveal_path(&data_dir)),
        ),
    ])
}
