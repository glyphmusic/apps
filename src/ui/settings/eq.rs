use super::*;

pub(super) type EqPick = Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>;

/// Equalizer page: on/off, presets (built-in and own), the curve, saving
/// and sharing.
pub(super) fn equalizer(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    let on = app.config.audio.eq_enabled;
    let gains = crate::app::eq_gains(&app.config.audio.eq_gains);
    let preset = app.config.audio.eq_preset.clone();
    let mut options: Vec<(SharedString, bool, EqPick)> = EQ_PRESETS
        .iter()
        .map(|(k, g)| {
            let (k, g) = (*k, *g);
            let f: EqPick = Rc::new(move |this, cx| this.set_eq(true, k, g, true, cx));
            (eq_preset_label(k), preset == k, f)
        })
        .collect();
    for p in &app.config.audio.eq_user_presets {
        let key = format!("user:{}", p.name);
        let g = p.gains();
        let selected = preset == key;
        let f: EqPick = Rc::new(move |this, cx| this.set_eq(true, &key, g, true, cx));
        options.push((SharedString::from(p.name.clone()), selected, f));
    }
    if preset == "custom" {
        options.push((eq_preset_label("custom"), true, Rc::new(|_: &mut Glyph, _: &mut Context<Glyph>| {})));
    }
    let own = preset.strip_prefix("user:").map(str::to_string);
    let action = |key: &str, f: fn(&mut Glyph, &mut Context<Glyph>)| -> (SharedString, bool, EqPick) { (t(key), false, Rc::new(f)) };
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
        row(t("sp.eq.bass"), Some(t("sp.eq.bass.desc")), {
            let cur = app.config.audio.eq_bass_db;
            let weak = cx.entity().downgrade();
            let pick: crate::ui::widgets::Pick = Rc::new(move |f, _, cx| {
                weak.update(cx, |this, cx| {
                    let h = this.config.audio.eq_headroom_db;
                    this.set_eq_extras(f * 24.0 - 12.0, h, true, cx)
                })
                .ok();
            });
            let label: crate::ui::widgets::BarLabel = Rc::new(|f: f32| db_label(((f * 24.0 - 12.0) * 2.0).round() / 2.0));
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .when(!on, |d| d.opacity(0.45))
                .child(thin_bar("eq-bass", 180.0, (cur + 12.0) / 24.0, pick, Some(label), cx))
                .child(mono().w(px(56.0)).flex().justify_end().text_size(px(12.0)).text_color(white(0.6)).child(db_label(cur)))
        }),
        row(t("sp.eq.headroom"), Some(t("sp.eq.headroom.desc")), {
            let cur = app.config.audio.eq_headroom_db;
            let weak = cx.entity().downgrade();
            let pick: crate::ui::widgets::Pick = Rc::new(move |f, _, cx| {
                weak.update(cx, |this, cx| {
                    let b = this.config.audio.eq_bass_db;
                    this.set_eq_extras(b, f * 12.0, true, cx)
                })
                .ok();
            });
            let label: crate::ui::widgets::BarLabel = Rc::new(|f: f32| db_label(-((f * 12.0) * 2.0).round() / 2.0));
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .when(!on, |d| d.opacity(0.45))
                .child(thin_bar("eq-headroom", 180.0, cur / 12.0, pick, Some(label), cx))
                .child(mono().w(px(56.0)).flex().justify_end().text_size(px(12.0)).text_color(white(0.6)).child(db_label(-cur)))
        }),
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
                .when(crate::ui::compact::TOUCH, |d| d.flex_wrap())
                .child(div().text_size(px(12.0)).text_color(white(0.45)).child(t("sp.eq.hint")))
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(button("eq-reset", t("sp.eq.reset")).on_click(cx.listener(|this, _, _, cx| {
                            let on = this.config.audio.eq_enabled;
                            this.set_eq(on, "flat", [0.0; 10], true, cx);
                            this.set_eq_extras(0.0, 0.0, true, cx)
                        })))
                        .when_some(own, |d, name| {
                            d.child(button("eq-delete", t("sp.eq.delete")).on_click(cx.listener(move |this, _, _, cx| this.delete_eq_preset(&name, cx))))
                        })
                        .child(primary_button("eq-save", t("sp.eq.save")).on_click(cx.listener(|this, _, w, cx| this.open_eq_save_dialog(w, cx)))),
                ),
        )
}

/// "+4.5 dB" / "0 dB" / "-3 dB".
fn db_label(db: f32) -> String {
    if db.abs() < 0.05 { "0 dB".into() } else { format!("{db:+.1} dB") }
}

thread_local! {
    /// Band being dragged on the equalizer curve.
    pub(super) static EQ_DRAG: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Spotify-style equalizer: a smooth curve through the ten bands with a
/// filled area below, points to drag up and down (-12..+12 dB).
pub(super) fn eq_curve(gains: [f32; 10], on: bool, cx: &mut Context<Glyph>) -> Div {
    const NAMES: [&str; 10] = if crate::ui::compact::TOUCH {
        // Phones: ten labels in ~330 px, so without "Hz".
        ["32", "64", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"]
    } else {
        ["32Hz", "64Hz", "125Hz", "250Hz", "500Hz", "1kHz", "2kHz", "4kHz", "8kHz", "16kHz"]
    };
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
