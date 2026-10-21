use super::*;

/// Playback > Content: what shows up and what plays next.
pub(super) fn content(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    use glyph_core::config::ShuffleStyle;
    let style = app.config.playback.shuffle;
    let shuffle = segmented(
        vec![
            (t("sp.shuffle.smart"), style == ShuffleStyle::Smart, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_shuffle_style(ShuffleStyle::Smart, cx)) as Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>),
            (t("sp.shuffle.random"), style == ShuffleStyle::Random, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_shuffle_style(ShuffleStyle::Random, cx))),
        ],
        cx,
        "shuffle-style",
    );
    let content = rows(vec![
        row(t("sp.shuffle"), Some(if style == ShuffleStyle::Smart { t("sp.shuffle.smart.desc") } else { t("sp.shuffle.random.desc") }), shuffle),
        row(
            t("sp.stats"),
            Some(t("sp.stats.desc")),
            switch("stats-on", app.config.stats.enabled).on_click(cx.listener(|this, _, _, cx| {
                this.set_stats_enabled(!this.config.stats.enabled, cx);
            })),
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
            t("sp.radio"),
            // Without a working Last.fm key on the server the switch says so
            // instead of silently doing nothing.
            Some(if app.radio_ok == Some(false) { t("radio.off") } else { t("sp.radio.desc") }),
            if app.radio_ok == Some(false) {
                with_tip(switch("radio", app.radio).opacity(0.4), Some(t("radio.off")))
            } else {
                switch("radio", app.radio).on_click(cx.listener(|this, _, _, cx| {
                    this.radio = !this.radio;
                    cx.notify();
                }))
            },
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
                    with_tip(div().id(("hidden-show", i)), Some(t("ctx.show_artist")))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(18.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(white(0.12)))
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_hidden_artist(&n, cx)))
                        .child(icon("eye", 12.0, white(0.7))),
                )
        })),
    );
    div().flex().flex_col().child(content).child(div().border_t_1().border_color(white(0.06)).child(hidden_row))
}

/// Playback > Sources: YouTube / SoundCloud fallback, yt-dlp, the free
/// sources and their order.
pub(super) fn sources(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
    use glyph_core::config::YouTubeMode;
    let local = app.config.providers.youtube_mode == YouTubeMode::Local;
    let youtube_mode = segmented(vec![
        (t("ytdlp.mode.local"), local, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_youtube_mode(YouTubeMode::Local, cx)) as Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>),
        (t("ytdlp.mode.server"), !local, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_youtube_mode(YouTubeMode::Server, cx))),
    ], cx, "youtube-mode");
    let top = rows(vec![
        row(t("sp.search.mode"), Some(t("sp.search.mode.desc")), segmented(vec![
            (t("ytdlp.mode.local"), app.config.providers.local_search, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_local_search(true, cx)) as Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>),
            (t("ytdlp.mode.server"), !app.config.providers.local_search, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_local_search(false, cx))),
        ], cx, "search-mode")),
        row(t("ytdlp.mode"), Some(t("ytdlp.mode.hint")), youtube_mode),
        row(t("ytdlp.only"), Some(t("ytdlp.only.hint")),
            switch("youtube-only", app.config.providers.youtube_only).on_click(cx.listener(|this, _, _, cx| {
                this.set_youtube_only(!this.config.providers.youtube_only, cx);
            }))),
        row(
            t("settings.sources.fallback"),
            Some(t("settings.sources.fallback.hint")),
            switch("embed-fallback", app.embed_fallback()).on_click(cx.listener(|this, _, _, cx| this.toggle_embed_fallback(cx))),
        ),
        // Which program finds YouTube audio on this computer.
        row(t("sp.extractor"), Some(t("sp.extractor.desc")), {
            use glyph_core::config::Extractor;
            let cur = app.config.providers.extractor;
            segmented(
                vec![
                    ("yt-dlp".into(), cur == Extractor::YtDlp, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_extractor(Extractor::YtDlp, cx)) as Rc<dyn Fn(&mut Glyph, &mut Context<Glyph>)>),
                    ("r-dlp".into(), cur == Extractor::RDlp, Rc::new(|this: &mut Glyph, cx: &mut Context<Glyph>| this.set_extractor(Extractor::RDlp, cx))),
                ],
                cx,
                "extractor",
            )
        }),
        // Local yt-dlp plays YouTube directly (only matters with YouTube on).
        row_status(t("ytdlp.title"), Some(crate::ui::ytdlp::description(app)), crate::ui::ytdlp::control(app, cx)),
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
pub(super) struct PrioDrag {
    from: usize,
    provider: ProviderId,
    width: Pixels,
}

pub(super) fn prio_row_content(i: usize, p: ProviderId) -> Div {
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

pub(super) fn priority_list(app: &mut Glyph, cx: &mut Context<Glyph>) -> Div {
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
                .relative()
                .child(hover_fill(format!("prio-{i}"), white(0.03), 8.0))
                .on_drag(PrioDrag { from: i, provider: p, width }, |d, _, _, cx| cx.new(|_| d.clone()))
                .drag_over::<PrioDrag>(|s, _, _, _| s.bg(white(0.08)))
                .on_drop(cx.listener(move |this, d: &PrioDrag, _, cx| this.reorder_priority(d.from, i, cx)))
                .child(prio_row_content(i, p))
        }))
}

pub(super) fn quality_label(q: AudioQuality) -> SharedString {
    match q {
        AudioQuality::Auto => t("sp.audio.q.auto"),
        AudioQuality::Low => t("sp.audio.q.low"),
        AudioQuality::Standard => t("sp.audio.q.standard"),
        AudioQuality::High => t("sp.audio.q.high"),
    }
}
