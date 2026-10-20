use super::*;

/// Quiet track row: cover 48 px, title, artist, badges, duration, actions.
pub fn track_row(
    app: &mut Glyph,
    list: Rc<Vec<Track>>,
    i: usize,
    id_prefix: &'static str,
    cx: &mut Context<Glyph>,
) -> Stateful<Div> {
    let t = list[i].clone();
    let playing = app.current.as_ref().map(track_key) == Some(track_key(&t));
    let liked = app.is_liked(&t);
    // Hidden explicit song: shown dimmed, does not play.
    let blocked = !app.allowed(&t);
    let group: SharedString = format!("{id_prefix}-row-{i}").into();
    let l2 = list.clone();
    let (t_next, t_q, t_like, t_share) = (t.clone(), t.clone(), t.clone(), t.clone());
    let art = cover(app, &t, 44.0, 6.0, cx);
    // Search results decode in (ASCII), everywhere else they just appear.
    let reveal = if id_prefix == "search-row" { reveal_k(&format!("search:{}", track_key(&t)), 0.9) } else { 1.0 };


    let t_menu = t.clone();
    let t_hover = t.clone();
    let selected = matches!(id_prefix, "search-row" | "section-row") && app.selected_row == Some(i);
    div()
        .id((id_prefix, i))
        // Resting on a YouTube song starts yt-dlp early (see ui::ytdlp).
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| this.prefetch_on_hover(&t_hover, *hovered, cx)))
        .on_mouse_down(
            gpui::MouseButton::Right,
            cx.listener(move |this, e: &gpui::MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.open_ctx(t_menu.clone(), e.position, None, cx)
            }),
        )
        .flex()
        .items_center()
        .gap(px(14.0))
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .when(blocked, |d| d.opacity(0.4))
        .when(selected, |d| {
            d.bg(gpui::hsla(0.0, 0.0, 1.0, 0.08))
                .border_1()
                .border_color(gpui::hsla(0.0, 0.0, 1.0, 0.16))
        })
        .relative()
        .child(hover_fill(group.clone(), theme::card(), 8.0))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.selected_row = Some(i);
            this.play_list((*l2).clone(), i, cx);
        }))
        .child(art)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w_0()
                        .when(playing, |d| d.child(playing_bars(app, 13.0, theme::pink())))
                        .when(t.explicit, |d| d.child(explicit_badge()))
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_md())
                                .font_weight(if playing { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                .text_color(if playing { theme::pink() } else { theme::text() })
                                .child(decode(&t.title, reveal)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .text_size(theme::text_sm())
                        .text_color(theme::text_dim())
                        .child(if reveal < 1.0 {
                            div().truncate().child(decode(&t.artist, reveal)).into_any_element()
                        } else {
                            crate::ui::lists::artist_link((id_prefix, i * 10 + 5), &t.artist, cx).into_any_element()
                        })
                        .when_some(crate::ui::lists::album_link((id_prefix, i * 10 + 6), &t, cx), |d, link| d.child(div().flex_none().child(" · ")).child(link)),
                ),
        )
        .when(id_prefix == "search-row", |d| d.child(kind_chip(tr("search.kind.song"))))
        .child(badges(&t))
        .child(
            div()
                .flex()
                .flex_none()
                .gap(px(2.0))
                .opacity(hover_k(&group))
                .child(
                    icon_button((id_prefix, i * 10 + 1), "list-start", 15.0, theme::text_dim())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.queue_add(t_next.clone(), true, cx)
                        })),
                )
                .child(
                    icon_button((id_prefix, i * 10 + 2), "list-plus", 15.0, theme::text_dim())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.queue_add(t_q.clone(), false, cx)
                        })),
                )
                .child(
                    icon_button((id_prefix, i * 10 + 3), "link-2", 15.0, theme::text_dim())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.share(&t_share, cx)
                        })),
                ),
        )
        .child({
            let key: SharedString = format!("{id_prefix}-like-{}", track_key(&t_like)).into();
            let k = key.clone();
            toggle_button(key, "heart", 15.0, liked, Some(theme::pink())).on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                mark_toggled(k.clone());
                this.toggle_like(&t_like, cx)
            }))
        })
        .child(
            mono()
                .flex_none()
                .w(px(44.0))
                .text_size(theme::text_sm())
                .text_color(theme::text_faint())
                .child(fmt_ms(t.duration_ms)),
        )
}

pub fn status_line(text: impl Into<SharedString>) -> Div {
    mono()
        .text_size(theme::text_xs())
        .text_color(theme::text_faint())
        .child(text.into())
}

/// Translate helper kept here so view code reads short.
pub fn tr(key: &str) -> SharedString {
    t(key)
}

/// Soft fade at the top and bottom edge of a scroll frame: `el` plus two
/// gradient overlays in `color` (the surface behind it). With a scroll handle
/// an edge only shows while there is more content beyond it (none at the very
/// top / bottom). The caller sizes the returned wrapper; `el` should fill it.
pub fn fade_edges(el: impl IntoElement, color: Hsla, h: f32, scroll: Option<&gpui::ScrollHandle>) -> Div {
    fade_edges_split(el, (color, h), (color, h), scroll)
}

/// [`fade_edges`] with its own colour and height for the top and the bottom edge.
pub fn fade_edges_split(el: impl IntoElement, top: (Hsla, f32), bottom: (Hsla, f32), scroll: Option<&gpui::ScrollHandle>) -> Div {
    // Edges fade in with the distance scrolled (no pop), and a page that only
    // overflows by a few pixels counts as not scrollable at all.
    let (top_k, bottom_k) = match scroll {
        Some(s) => {
            let y = -f32::from(s.offset().y);
            let max = f32::from(s.max_offset().y);
            if max < 32.0 {
                (0.0, 0.0)
            } else {
                (((y - 2.0) / 40.0).clamp(0.0, 1.0), ((max - y - 2.0) / 40.0).clamp(0.0, 1.0))
            }
        }
        None => (1.0, 1.0),
    };
    let (show_top, show_bottom) = (top_k > 0.0, bottom_k > 0.0);
    let (top_spec, bottom_spec) = (top, bottom);
    // The band also fades out towards its left and right ends (strips with
    // falling strength), so it has no hard corners against the background.
    const RAMP: [f32; 8] = [0.06, 0.16, 0.3, 0.45, 0.6, 0.74, 0.86, 0.95];
    let edge = |top: bool| {
        let (color, h) = if top { top_spec } else { bottom_spec };
        let k = if top { top_k } else { bottom_k };
        let strip = move |f: f32| {
            let c = Hsla { a: color.a * k * f, ..color };
            gpui::linear_gradient(
                if top { 180.0 } else { 0.0 },
                gpui::linear_color_stop(c, 0.0),
                gpui::linear_color_stop(c.opacity(0.0), 1.0),
            )
        };
        div()
            .absolute()
            .left_0()
            .right_0()
            .h(px(h))
            .when(top, |d| d.top_0())
            .when(!top, |d| d.bottom_0())
            .flex()
            .children(RAMP.iter().map(move |f| div().flex_none().w(px(6.0)).h_full().bg(strip(*f))))
            .child(div().flex_1().h_full().bg(strip(1.0)))
            .children(RAMP.iter().rev().map(move |f| div().flex_none().w(px(6.0)).h_full().bg(strip(*f))))
    };
    div().relative().child(el).when(show_top, |d| d.child(edge(true))).when(show_bottom, |d| d.child(edge(false)))
}
