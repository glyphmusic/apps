//! Settings as a modal (Liner layout): left navigation with search and
//! grouped pages (some with sub-tabs), right the page with rows (label +
//! description left, control right). Pages: Playback, Audio, Appearance,
//! Account, Server, Cache & Storage, Backup & share, About. The search finds
//! pages, sub-tabs and single options (see [`OPTIONS`]).

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

thread_local! {
    /// Option picked in the settings search: (label, since when, scrolled to).
    /// Its row lights up briefly and scrolls into view.
    static FLASH: std::cell::RefCell<Option<(String, std::time::Instant, bool)>> = const { std::cell::RefCell::new(None) };
    /// Scroll state of the settings page (for scrolling to a found option).
    static PAGE_SCROLL: std::cell::RefCell<Option<gpui::ScrollHandle>> = const { std::cell::RefCell::new(None) };
}

const FLASH_S: f32 = 1.8;

/// Highlights the row labelled `label` on the next settings page shown.
pub fn flash_option(label: impl Into<String>) {
    FLASH.with(|f| *f.borrow_mut() = Some((label.into(), std::time::Instant::now(), false)));
}

/// A found option is still lit (the app keeps rendering).
pub fn flash_animating() -> bool {
    FLASH.with(|f| f.borrow().as_ref().is_some_and(|(_, at, _)| at.elapsed().as_secs_f32() < FLASH_S))
}

/// Highlight strength (1 -> 0) for the row labelled `label`, if it is the found one.
fn flash_k(label: &str) -> Option<f32> {
    FLASH.with(|f| {
        let f = f.borrow();
        let (l, at, _) = f.as_ref()?;
        let e = at.elapsed().as_secs_f32();
        (l == label && e < FLASH_S).then(|| if e < 0.7 { 1.0 } else { 1.0 - (e - 0.7) / (FLASH_S - 0.7) })
    })
}

/// Scrolls the settings page so the found row (window bounds `b`) is in view; once.
fn scroll_to_found(b: gpui::Bounds<Pixels>) {
    let first = FLASH.with(|f| f.borrow_mut().as_mut().is_some_and(|fl| !std::mem::replace(&mut fl.2, true)));
    if !first {
        return;
    }
    PAGE_SCROLL.with(|h| {
        let Some(h) = h.borrow().clone() else { return };
        let vp = h.bounds();
        let vh = f32::from(vp.size.height);
        let top = f32::from(b.origin.y) - f32::from(vp.origin.y);
        if top >= 0.0 && top + f32::from(b.size.height) <= vh {
            return;
        }
        let max = f32::from(h.max_offset().y).max(0.0);
        let y = (f32::from(h.offset().y) - (top - vh * 0.3)).clamp(-max, 0.0);
        h.set_offset(point(px(0.0), px(y)));
    });
}

/// Settings row: label + description left, control right.
pub(crate) fn row(label: impl Into<SharedString>, desc: Option<SharedString>, control: impl IntoElement) -> AnyElement {
    let label: SharedString = label.into();
    let lit = flash_k(&label);
    div()
        .when_some(lit, |d, k| {
            d.relative()
                .mx(px(-10.0))
                .px(px(10.0))
                .rounded(px(8.0))
                .bg(theme::pink().opacity(0.14 * k))
                .child(canvas(|b, _, _| scroll_to_found(b), |_, _, _, _| {}).absolute().inset_0())
        })
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
                .child(div().text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child(label))
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

/// Display name of a preset key: built-in (translated), own (`user:<name>`)
/// or "custom".
pub(crate) fn eq_preset_label(key: &str) -> SharedString {
    match key.strip_prefix("user:") {
        Some(name) => SharedString::from(name.to_string()),
        None => t(&format!("eq.preset.{key}")),
    }
}

/// Preset key for a curve: the current preset while it still matches, else
/// the first built-in or own preset with exactly these gains, else "custom".
pub(crate) fn eq_match(current: &str, gains: &[f32; 10], user: &[glyph_core::eq::EqPreset]) -> String {
    use glyph_core::eq::same_gains;
    let builtin = EQ_PRESETS.iter().map(|(k, g)| (k.to_string(), *g));
    let own = user.iter().map(|p| (format!("user:{}", p.name), p.gains()));
    let all: Vec<(String, [f32; 10])> = builtin.chain(own).collect();
    if all.iter().any(|(k, g)| k == current && same_gains(g, gains)) {
        return current.to_string();
    }
    all.into_iter().find(|(_, g)| same_gains(g, gains)).map(|(k, _)| k).unwrap_or_else(|| "custom".into())
}

type Pick = Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>;

/// Equalizer page: on/off, presets (built-in and own), the curve, saving
/// and sharing.
fn equalizer(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let on = app.config.audio.eq_enabled;
    let gains = crate::app::eq_gains(&app.config.audio.eq_gains);
    let preset = app.config.audio.eq_preset.clone();
    let mut options: Vec<(SharedString, bool, Pick)> = EQ_PRESETS
        .iter()
        .map(|(k, g)| {
            let (k, g) = (*k, *g);
            let f: Pick = Rc::new(move |this, cx| this.set_eq(true, k, g, true, cx));
            (eq_preset_label(k), preset == k, f)
        })
        .collect();
    for p in &app.config.audio.eq_user_presets {
        let key = format!("user:{}", p.name);
        let g = p.gains();
        let selected = preset == key;
        let f: Pick = Rc::new(move |this, cx| this.set_eq(true, &key, g, true, cx));
        options.push((SharedString::from(p.name.clone()), selected, f));
    }
    if preset == "custom" {
        options.push((eq_preset_label("custom"), true, Rc::new(|_: &mut Glyph, _: &mut Context<Glyph>| {})));
    }
    let own = preset.strip_prefix("user:").map(str::to_string);
    let action = |key: &str, f: fn(&mut Glyph, &mut Context<Glyph>)| -> (SharedString, bool, Pick) { (t(key), false, Rc::new(f)) };
    let share = vec![
        action("sp.eq.copy_code", |this, cx| this.copy_eq_code(cx)),
        action("sp.eq.paste_code", |this, cx| this.paste_eq_code(cx)),
        action("sp.eq.export_file", |this, cx| this.export_eq_file(cx)),
        action("sp.eq.import_file", |this, cx| this.import_eq_file(cx)),
    ];
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
        row(t("sp.eq.preset"), Some(t("sp.eq.preset.desc")), select(app, "sel-eq", eq_preset_label(&preset), options, cx)),
        row(t("sp.eq.share"), Some(t("sp.eq.share.desc")), select(app, "sel-eq-share", t("sp.eq.share.button"), share, cx)),
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
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(div().text_size(px(12.0)).text_color(white(0.45)).child(t("sp.eq.hint")))
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(button("eq-reset", t("sp.eq.reset")).on_click(cx.listener(|this, _, _, cx| {
                            let on = this.config.audio.eq_enabled;
                            this.set_eq(on, "flat", [0.0; 10], true, cx)
                        })))
                        .when_some(own, |d, name| {
                            d.child(button("eq-delete", t("sp.eq.delete")).on_click(cx.listener(move |this, _, _, cx| this.delete_eq_preset(&name, cx))))
                        })
                        .child(primary_button("eq-save", t("sp.eq.save")).on_click(cx.listener(|this, _, w, cx| this.open_eq_save_dialog(w, cx)))),
                ),
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
        // Own hitbox, so a dropdown floating over the curve takes the click
        // instead of the curve.
        |bounds, window, _| window.insert_hitbox(bounds.dilate(px(8.0)), gpui::HitboxBehavior::Normal),
        move |bounds, hitbox, window, _app_cx| {
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
                if dragging == Some(i) {
                    // Soft ring around the dragged point.
                    window.paint_quad(gpui::quad(
                        gpui::Bounds { origin: gpui::point(px(x - 12.0), px(y - 12.0)), size: gpui::size(px(24.0), px(24.0)) },
                        gpui::Corners::all(px(12.0)),
                        accent.opacity(0.25),
                        gpui::Edges::default(),
                        gpui::transparent_black(),
                        gpui::BorderStyle::default(),
                    ));
                }
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
            let (v1, v2, v3) = (view.clone(), view.clone(), view.clone());
            let apply = move |v: &gpui::WeakEntity<Glyph>, band: usize, g: f32, save: bool, cx: &mut gpui::App| {
                v.update(cx, |this, cx| {
                    let mut gs = crate::app::eq_gains(&this.config.audio.eq_gains);
                    gs[band] = g;
                    // A curve that equals a preset shows that preset's name.
                    let preset = eq_match(&this.config.audio.eq_preset, &gs, &this.config.audio.eq_user_presets);
                    this.set_eq(true, &preset, gs, save, cx);
                })
                .ok();
            };
            window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Bubble || e.button != gpui::MouseButton::Left || !hitbox.is_hovered_at(e.position, window) {
                    return;
                }
                // Something else (an open dropdown) is in the way: let it close first.
                if v1.upgrade().is_some_and(|v| v.read(cx).open_select.is_some()) {
                    return;
                }
                let band = band_at(f32::from(e.position.x));
                if e.click_count >= 2 {
                    // Double-click puts a band back to 0 dB.
                    EQ_DRAG.with(|d| d.set(None));
                    apply(&v1, band, 0.0, true, cx);
                } else {
                    EQ_DRAG.with(|d| d.set(Some(band)));
                    apply(&v1, band, gain_at(f32::from(e.position.y)), false, cx);
                }
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
        .child(div().flex().ml(px(LEFT)).children(gains.iter().map(|g| {
            let label = if g.abs() < 0.05 { "0".to_string() } else { format!("{g:+.1}") };
            div()
                .flex_1()
                .text_center()
                .text_size(px(11.5))
                .font_family(theme::MONO)
                .text_color(if g.abs() < 0.05 { white(0.35) } else { accent.opacity(0.9) })
                .child(label)
        })))
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

/// Title, subtitle and nav icon of a settings page.
fn page_info(page: SettingsPage) -> (SharedString, SharedString, &'static str) {
    match page {
        SettingsPage::Playback => (t("sp.playback"), t("sp.playback.sub"), "play"),
        SettingsPage::Audio => (t("sp.eq"), t("sp.eq.sub"), "volume-2"),
        SettingsPage::Appearance => (t("sp.appearance"), t("sp.appearance.sub"), "palette"),
        SettingsPage::Account => (t("sp.account"), t("sp.account.sub"), "user-round"),
        SettingsPage::Server => (t("sp.server"), t("sp.server.sub"), "server"),
        SettingsPage::Storage => (t("sp.storage"), t("sp.storage.sub"), "database"),
        SettingsPage::Transfer => (t("sp.transfer"), t("sp.transfer.sub"), "copy"),
        SettingsPage::About => (t("sp.about"), t("sp.about.sub"), "info"),
    }
}

/// Sub-tabs of a settings page (label keys); empty: the page has none.
/// `app.settings_sub` indexes this list.
pub fn subtabs(app: &Glyph, page: SettingsPage) -> &'static [&'static str] {
    match page {
        SettingsPage::Playback => &["sp.sub.general", "sp.sub.content", "settings.sources"],
        SettingsPage::Account if app.signed_in().is_some() => &["sp.sub.overview", "sp.sub.devices", "account.security"],
        SettingsPage::Server => &["sp.sub.connection", "settings.keys"],
        SettingsPage::About => &["sp.sub.version", "sp.support"],
        _ => &[],
    }
}

/// Every option the settings search finds on its own: page, sub-tab, label
/// key (the row's label, so the row can light up), extra search words.
const OPTIONS: &[(SettingsPage, u8, &str, &str)] = &[
    (SettingsPage::Playback, 0, "sp.audio.quality", "streaming quality qualität bitrate"),
    (SettingsPage::Playback, 0, "sp.audio.dl_quality", "download quality qualität flac"),
    (SettingsPage::Playback, 0, "sp.crossfade", "crossfade überblenden fade"),
    (SettingsPage::Playback, 0, "sp.output", "output ausgabe device gerät speaker lautsprecher headphones kopfhörer"),
    (SettingsPage::Playback, 0, "sp.level", "volume lautstärke level"),
    (SettingsPage::Playback, 0, "sp.normalize", "normalize normalisieren loudness lautheit"),
    (SettingsPage::Playback, 1, "sp.explicit", "explicit explizit"),
    (SettingsPage::Playback, 1, "sp.radio", "radio autoplay"),
    (SettingsPage::Playback, 1, "sp.hidden", "hidden ausgeblendet blocked artists künstler"),
    (SettingsPage::Playback, 2, "settings.sources.fallback", "youtube soundcloud embed fallback"),
    (SettingsPage::Playback, 2, "ytdlp.title", "yt-dlp ytdlp youtube"),
    (SettingsPage::Playback, 2, "settings.priority", "priority priorität order reihenfolge audius jamendo archive"),
    (SettingsPage::Audio, 0, "sp.eq.enable", "equalizer eq"),
    (SettingsPage::Audio, 0, "sp.eq.preset", "preset bass treble höhen tiefen"),
    (SettingsPage::Audio, 0, "sp.eq.share", "share teilen export import code datei file"),
    (SettingsPage::Appearance, 0, "settings.language", "language sprache"),
    (SettingsPage::Appearance, 0, "settings.visuals.intensity", "background hintergrund glow intensity intensität"),
    (SettingsPage::Appearance, 0, "settings.visuals.ascii_covers", "ascii cover"),
    (SettingsPage::Appearance, 0, "sp.appearance.startup", "startup start animation splash"),
    (SettingsPage::Account, 0, "account.handle", "handle name rename umbenennen"),
    (SettingsPage::Account, 0, "account.keys", "sync api keys schlüssel"),
    (SettingsPage::Account, 0, "account.limit", "device limit gerätelimit"),
    (SettingsPage::Account, 1, "account.pair", "pair koppeln code device gerät add hinzufügen"),
    (SettingsPage::Account, 2, "account.password.change", "password passwort"),
    (SettingsPage::Account, 2, "account.passkey.add", "passkey"),
    (SettingsPage::Account, 2, "account.recovery", "recovery wiederherstellung code"),
    (SettingsPage::Account, 2, "account.delete", "delete löschen account"),
    (SettingsPage::Server, 0, "settings.server.url", "server url adresse"),
    (SettingsPage::Server, 1, "settings.keys.lastfm", "api key lastfm last.fm"),
    (SettingsPage::Server, 1, "settings.keys.youtube", "api key youtube"),
    (SettingsPage::Server, 1, "settings.keys.spotify_client_id", "api key spotify"),
    (SettingsPage::Server, 1, "settings.keys.jamendo_client_id", "api key jamendo"),
    (SettingsPage::Storage, 0, "sp.storage.limit", "cache limit size größe"),
    (SettingsPage::Storage, 0, "sp.storage.usage", "clear leeren cache downloads covers lyrics speicher"),
    (SettingsPage::Transfer, 0, "sp.transfer.code", "share teilen export code copy kopieren"),
    (SettingsPage::Transfer, 0, "sp.transfer.file", "export file datei save speichern backup"),
    (SettingsPage::Transfer, 0, "sp.transfer.import", "import apply übernehmen paste einfügen"),
    (SettingsPage::About, 0, "update.auto", "auto update automatisch"),
    (SettingsPage::About, 0, "update.channel", "prerelease beta channel kanal"),
    (SettingsPage::About, 1, "sp.support.issue", "bug issue report melden"),
    (SettingsPage::About, 1, "sp.support.diag", "diagnostics diagnose"),
    (SettingsPage::About, 1, "sp.support.data", "data daten folder ordner"),
];

fn modal_inner(app: &mut Glyph, _window: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    select_anim_tick(app.open_select);
    PAGE_SCROLL.with(|h| *h.borrow_mut() = Some(app.settings_scroll.clone()));
    let page = app.settings_page;
    let (title, sub, _) = page_info(page);
    let tab = subtabs(app, page).get(app.settings_sub as usize).map(|k| t(k));
    let body = match page {
        SettingsPage::Playback => playback(app, cx),
        SettingsPage::Audio => equalizer(app, cx),
        SettingsPage::Appearance => appearance(app, cx),
        SettingsPage::Server => server(app, cx),
        SettingsPage::Account => crate::ui::account::page(app, cx),
        SettingsPage::Storage => storage(app, cx),
        SettingsPage::Transfer => crate::ui::transfer::page(app, cx),
        SettingsPage::About if app.settings_sub == 1 => support(app, cx),
        SettingsPage::About => about(app, cx),
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
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .text_size(px(17.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(div().text_color(if tab.is_some() { white(0.5) } else { gpui::white() }).child(title))
                                    .when_some(tab, |d, tab| d.child(icon("chevron-right", 16.0, white(0.4))).child(div().text_color(gpui::white()).child(tab))),
                            )
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

fn nav(app: &mut Glyph, cx: &mut Context<Glyph>) -> gpui::Stateful<Div> {
    let q = app.settings_search.read(cx).value.trim().to_lowercase();
    let (current, current_sub) = (app.settings_page, app.settings_sub);
    let signed_in = app.signed_in().is_some();
    // (page, extra search words)
    let groups: [(SharedString, Vec<(SettingsPage, &'static str)>); 2] = [
        (
            t("sp.group.prefs"),
            vec![
                (SettingsPage::Playback, "sources source quelle youtube soundcloud priority radio quality qualität audio"),
                (SettingsPage::Audio, "equalizer eq bass treble höhen tiefen klang sound preset teilen share import export code datei file"),
                (SettingsPage::Appearance, "language sprache theme ascii cover glow"),
            ],
        ),
        (
            t("sp.group.app"),
            vec![
                (SettingsPage::Account, "account konto login sign in anmelden sync devices geräte handle passkey"),
                (SettingsPage::Server, "url api key keys schlüssel"),
                (SettingsPage::Storage, "cache download downloads speicher"),
                (SettingsPage::Transfer, "backup export import share teilen code datei file"),
                (SettingsPage::About, "update version support hilfe help"),
            ],
        ),
    ];
    let hit = |text: &str| q.is_empty() || text.to_lowercase().contains(&q);

    // Single options matching the search (with their page / sub-tab).
    let found: Vec<(SettingsPage, u8, SharedString)> = if q.is_empty() {
        Vec::new()
    } else {
        OPTIONS
            .iter()
            .filter(|(p, _, _, _)| *p != SettingsPage::Account || signed_in)
            .filter(|(_, _, key, words)| hit(&t(key)) || words.contains(&q))
            .map(|(p, s, key, _)| (*p, *s, t(key)))
            .take(14)
            .collect()
    };

    let mut col = div()
        .id("sp-nav")
        .flex_none()
        .w(px(NAV_W))
        .h_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .p(px(6.0))
        .child(
            div()
                .flex()
                .flex_none()
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
        let mut visible = Vec::new();
        for (page, words) in items {
            let (label, _, icon_name) = page_info(page);
            let subs = subtabs(app, page);
            let sub_hit: Vec<bool> = subs.iter().map(|k| !q.is_empty() && hit(&t(k))).collect();
            let page_hit = hit(&label) || words.contains(&q) || found.iter().any(|f| f.0 == page);
            if page_hit || sub_hit.contains(&true) {
                visible.push((page, label, icon_name, subs, sub_hit));
            }
        }
        if visible.is_empty() {
            continue;
        }
        col = col.child(div().flex_none().px(px(10.0)).pt(px(10.0)).pb(px(4.0)).text_size(px(12.0)).text_color(white(0.45)).child(group));
        for (page, label, icon_name, subs, sub_hit) in visible {
            let on = page == current;
            let open = !subs.is_empty() && (on || sub_hit.contains(&true));
            col = col.child(
                div()
                    .id(("sp-nav", page as usize))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.0))
                    .h(px(36.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .text_size(px(14.0))
                    .when(on, |d| d.bg(white(0.08)).text_color(gpui::white()))
                    .when(!on, |d| d.text_color(white(0.75)).hover(|s| s.bg(white(0.04)).text_color(gpui::white())))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.settings_page != page {
                            this.set_settings_page(page, cx);
                        }
                    }))
                    .child(icon(icon_name, 16.0, if on { gpui::white() } else { white(0.7) }))
                    .child(div().flex_1().child(label))
                    .when(!subs.is_empty(), |d| d.child(icon(if open { "chevron-down" } else { "chevron-right" }, 14.0, white(0.45)))),
            );
            if open {
                // Sub-tabs hang off a thin line under the page's icon.
                col = col.child(
                    div()
                        .flex_none()
                        .ml(px(18.0))
                        .my(px(2.0))
                        .pl(px(8.0))
                        .border_l_1()
                        .border_color(white(0.10))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .children(subs.iter().enumerate().map(|(i, key)| {
                            let sel = on && current_sub as usize == i;
                            let sub = i as u8;
                            div()
                                .id(("sp-sub", page as usize * 8 + i))
                                .flex()
                                .items_center()
                                .h(px(32.0))
                                .px(px(10.0))
                                .rounded(px(7.0))
                                .cursor_pointer()
                                .text_size(px(13.5))
                                .when(sel, |d| d.bg(white(0.08)).text_color(gpui::white()).font_weight(FontWeight::MEDIUM))
                                .when(!sel, |d| d.text_color(white(0.6)).hover(|s| s.bg(white(0.04)).text_color(gpui::white())))
                                .on_click(cx.listener(move |this, _, _, cx| this.set_settings_tab(page, sub, cx)))
                                .child(t(key))
                        })),
                );
            }
        }
    }
    if !found.is_empty() {
        col = col.child(div().flex_none().px(px(10.0)).pt(px(14.0)).pb(px(4.0)).text_size(px(12.0)).text_color(white(0.45)).child(t("sp.group.options")));
        for (i, (page, sub, label)) in found.into_iter().enumerate() {
            let (page_label, _, _) = page_info(page);
            let crumb: SharedString = match subtabs(app, page).get(sub as usize) {
                Some(k) => format!("{page_label} › {}", t(k)).into(),
                None => page_label,
            };
            let l = label.clone();
            col = col.child(
                div()
                    .id(("sp-found", i))
                    .flex()
                    .flex_none()
                    .flex_col()
                    .justify_center()
                    .gap(px(1.0))
                    .py(px(6.0))
                    .px(px(10.0))
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(white(0.05)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        flash_option(l.to_string());
                        this.set_settings_tab(page, sub, cx);
                    }))
                    .child(div().text_size(px(13.5)).text_color(white(0.9)).truncate().child(label))
                    .child(div().text_size(px(11.5)).text_color(white(0.45)).truncate().child(crumb)),
            );
        }
    }
    col
}

// ------------------------------------------------------------------ pages

fn playback(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    // Sub-tabs: general (sound and output), content (what plays), sources.
    match app.settings_sub {
        1 => return content(app, cx),
        2 => return sources(app, cx),
        _ => {}
    }
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
    rows(vec![
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
    ])
}

/// Playback > Content: what shows up and what plays next.
fn content(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let content = rows(vec![
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
    div().flex().flex_col().child(content).child(div().border_t_1().border_color(white(0.06)).child(hidden_row))
}

/// Playback > Sources: YouTube / SoundCloud fallback, yt-dlp, the free
/// sources and their order.
fn sources(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let top = rows(vec![
        row(
            t("settings.sources.fallback"),
            Some(t("settings.sources.fallback.hint")),
            switch("embed-fallback", app.embed_fallback()).on_click(cx.listener(|this, _, _, cx| this.toggle_embed_fallback(cx))),
        ),
        // Local yt-dlp plays YouTube directly (only matters with YouTube on).
        row(t("ytdlp.title"), Some(crate::ui::ytdlp::description(app)), crate::ui::ytdlp::control(app, cx)),
    ]);
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
    div()
        .flex()
        .flex_col()
        .child(top)
        .child(section_label(t("settings.sources")))
        .child(rows(sources))
        .child(section_label(t("settings.priority")))
        .child(div().pb(px(8.0)).text_size(px(12.5)).text_color(white(0.55)).child(t("settings.priority.hint")))
        .child(priority_list(app, cx))
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
    let set_int: crate::ui::widgets::Pick = Rc::new(move |f, _, cx| {
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
    let items = vec![
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
    if app.settings_sub == 1 {
        return div()
            .flex()
            .flex_col()
            .child(div().pt(px(6.0)).pb(px(4.0)).text_size(px(12.5)).text_color(white(0.55)).child(t("settings.keys.hint")))
            .child(rows(keys));
    }
    rows(items)
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
