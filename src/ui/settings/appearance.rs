use super::*;

pub(super) fn appearance(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
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
    let weak = cx.entity().downgrade();
    let set_reactivity: crate::ui::widgets::Pick = Rc::new(move |f, _, cx| {
        weak.update(cx, |this, cx| this.set_music_reactivity(f, cx)).ok();
    });
    let big = app.mini_big;
    let mini = segmented(
        vec![
            (t("sp.appearance.mini.bar"), !big, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_mini_big(false, cx)) as Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>),
            (t("sp.appearance.mini.card"), big, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_mini_big(true, cx))),
        ],
        cx,
        "mini-style",
    );
    let reactivity = app.config.visuals.music_reactivity;
    let intensity = app.config.visuals.intensity;
    // Background shape: a new one with every song, or always one.
    let shape = app.config.visuals.glow_shape.clone();
    let shape_label = |name: &str| -> SharedString { if name.is_empty() { t("sp.appearance.shape.auto") } else { t(&format!("sp.appearance.shape.{name}")) } };
    let shapes = std::iter::once("")
        .chain(crate::visuals::Preset::ALL.iter().map(|p| p.name()))
        .map(|name| {
            let f: Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)> = Rc::new(move |this, cx| this.set_glow_shape(name, cx));
            (shape_label(name), shape == name, f)
        })
        .collect();
    let shape_select = select(app, "sel-glow-shape", shape_label(&shape), shapes, cx);
    let scale = app.config.ui.text_scale();
    let text_size = segmented(
        [(0.9f32, "sp.appearance.text.small"), (1.0, "sp.appearance.text.normal"), (1.15, "sp.appearance.text.large"), (1.3, "sp.appearance.text.huge")]
            .into_iter()
            .map(|(k, key)| (t(key), (scale - k).abs() < 0.02, Rc::new(move |this: &mut Glyph, cx: &mut Context<Glyph>| this.set_text_scale(k, cx)) as Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>))
            .collect(),
        cx,
        "text-size",
    );
    rows(vec![
        row(t("settings.language"), Some(t("sp.appearance.lang.desc")), language),
        row(t("sp.appearance.text"), Some(t("sp.appearance.text.desc")), text_size),
        row(t("sp.appearance.shape"), Some(t("sp.appearance.shape.desc")), shape_select),
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
            t("settings.visuals.music_reactivity"),
            Some(t("sp.appearance.music_reactivity.desc")),
            div().flex().items_center().gap(px(10.0))
                .child(thin_bar("music-reactivity", 160.0, reactivity, set_reactivity, None, cx))
                .child(mono().w(px(36.0)).flex().justify_end().text_size(px(12.0)).text_color(white(0.6)).child(format!("{:.0}%", reactivity * 100.0))),
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
        row(t("sp.appearance.theme"), Some(t("sp.appearance.theme.desc")), theme_swatches(app, cx)),
        row(
            t("sp.appearance.ascii_fx"),
            Some(t("sp.appearance.ascii_fx.desc")),
            switch("ascii-fx", app.config.visuals.ascii_effects).on_click(cx.listener(|this, _, _, cx| {
                this.config.visuals.ascii_effects = !this.config.visuals.ascii_effects;
                this.save_config(cx);
                cx.notify();
            })),
        ),
        row(t("sp.appearance.mini"), Some(t("sp.appearance.mini.desc")), mini),
        row(
            t("sp.appearance.throttle"),
            Some(t("sp.appearance.throttle.desc")),
            switch("throttle-unfocused", app.config.visuals.throttle_unfocused).on_click(cx.listener(|this, _, _, cx| {
                this.set_throttle_unfocused(!this.config.visuals.throttle_unfocused, cx);
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

/// One round swatch per colour theme (accent inside, glow colour as ring),
/// the picked one outlined. Themes from plugins follow the built-in ones.
pub(super) fn theme_swatches(app: &Glyph, cx: &mut Context<Glyph>) -> Div {
    let picked = &app.config.visuals.theme;
    let cur: String = if crate::theme::known_theme(picked) { picked.clone() } else { crate::theme::THEMES[0].id.to_string() };
    let builtin = crate::theme::THEMES.iter().map(|th| (th.id.to_string(), t(th.name), crate::theme::colors(th.id)));
    let plugins = crate::theme::plugin_themes().into_iter().map(|th| (th.id, SharedString::from(th.name), th.colors));
    let all: Vec<_> = builtin.chain(plugins).collect();
    // Wraps once plugins add more swatches than fit beside the label.
    div().flex().flex_wrap().justify_end().max_w(px(360.0)).items_center().gap(px(6.0)).children(all.into_iter().enumerate().map(|(i, (id, name, c))| {
        let on = id == cur;
        let accent: Hsla = gpui::rgb(c.accent).into();
        let ring: Hsla = c.glow.map_or(gpui::rgb(0x1d5cff).into(), |g| gpui::rgb(g[1]).into());
        with_tip(div().id(("theme", i)), Some(name))
            .flex_none()
            .size(px(28.0))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .border_2()
            .border_color(if on { gpui::white() } else { gpui::transparent_black() })
            .on_click(cx.listener(move |this, _, _, cx| this.set_theme(&id, cx)))
            .child(div().size(px(20.0)).rounded_full().bg(ring).flex().items_center().justify_center().child(div().size(px(12.0)).rounded_full().bg(accent)))
    }))
}
