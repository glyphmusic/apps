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
/// the picked one outlined.
pub(super) fn theme_swatches(app: &Glyph, cx: &mut Context<Glyph>) -> Div {
    let cur = crate::theme::theme_by_id(&app.config.visuals.theme).id;
    div().flex().items_center().gap(px(6.0)).children(crate::theme::THEMES.iter().enumerate().map(|(i, th)| {
        let on = th.id == cur;
        let id = th.id;
        let accent: Hsla = gpui::rgb(th.accent).into();
        let ring: Hsla = th.glow.map_or(gpui::rgb(0x1d5cff).into(), |g| gpui::rgb(g[1]).into());
        with_tip(div().id(("theme", i)), Some(t(th.name)))
            .flex_none()
            .size(px(28.0))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .border_2()
            .border_color(if on { gpui::white() } else { gpui::transparent_black() })
            .on_click(cx.listener(move |this, _, _, cx| this.set_theme(id, cx)))
            .child(div().size(px(20.0)).rounded_full().bg(ring).flex().items_center().justify_center().child(div().size(px(12.0)).rounded_full().bg(accent)))
    }))
}
