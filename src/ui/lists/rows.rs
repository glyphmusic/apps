use super::*;

// ------------------------------------------------------------------ song rows

thread_local! {
    /// Width of the song rows per list (measured on the first row).
    pub(super) static ROW_W: std::cell::RefCell<std::collections::HashMap<&'static str, f32>> = std::cell::RefCell::new(Default::default());
}

/// Rows narrower than this drop the album column, so title and artist keep
/// their room (the album stays in the song menu).
pub(super) const ALBUM_COL_MIN_ROW: f32 = 700.0;

/// Numbered song row: # (play on hover) · cover · title / artist · album ·
/// heart · time · ⋮. Click plays the list from here; right-click or ⋮ opens
/// the song menu.
pub fn song_row(
    app: &mut Glyph,
    list: Rc<Vec<Track>>,
    i: usize,
    prefix: &'static str,
    from: Option<ListRef>,
    show_album: bool,
    cx: &mut Context<Glyph>,
) -> Stateful<gpui::Div> {
    if crate::ui::compact::TOUCH {
        let from_ctx = from.map(|l| (l, i));
        let tr = list[i].clone();
        let row = crate::ui::compact::song_row(app, list, i, prefix, from_ctx.clone(), cx);
        return with_menu(row, tr, from_ctx, cx);
    }
    let tr = list[i].clone();
    let playing = app.current.as_ref().map(crate::app::track_key) == Some(crate::app::track_key(&tr));
    let liked = app.is_liked(&tr);
    // Hidden explicit song: shown dimmed, does not play.
    let blocked = !app.allowed(&tr);
    let dl_badge = crate::ui::widgets::download_badge(app.download_view(&tr), 14.0, "arrow-down");
    let dl_tip = app.download_tip_one(&tr);
    let group: SharedString = format!("{prefix}-g-{i}").into();
    let art = cover(app, &tr, 40.0, 6.0, cx);
    let l2 = list.clone();
    let from_ctx = from.clone().map(|l| (l, i));
    let (t_like, t_menu) = (tr.clone(), tr.clone());
    let from_menu = from_ctx.clone();
    let t_hover = tr.clone();
    let hk = hover_k(&group);
    let selected = matches!(prefix, "list-row" | "artist-row") && app.selected_row == Some(i);
    let show_album = show_album && ROW_W.with(|m| m.borrow().get(prefix).copied()).is_none_or(|w| w >= ALBUM_COL_MIN_ROW);
    let row = div()
        .id((prefix, i))
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| this.prefetch_on_hover(&t_hover, *hovered, cx)))
        .flex()
        .items_center()
        .gap(px(14.0))
        .h(px(56.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .when(blocked, |d| d.opacity(0.4))
        .when(selected, |d| d.bg(white(0.08)).border_1().border_color(white(0.16)))
        .relative()
        .child(hover_fill(group.clone(), white(0.05), 8.0))
        .when(i == 0, |d| {
            d.child(
                gpui::canvas(
                    move |b, window, _| {
                        let w = f32::from(b.size.width);
                        let old = ROW_W.with(|m| m.borrow_mut().insert(prefix, w));
                        // Crossing the limit shows / hides the album column.
                        if old.is_none_or(|o| (o >= ALBUM_COL_MIN_ROW) != (w >= ALBUM_COL_MIN_ROW)) {
                            window.request_animation_frame();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.selected_row = Some(i);
            this.play_list((*l2).clone(), i, cx);
        }))
        .child(
            div()
                .flex_none()
                .w(px(26.0))
                .h(px(20.0))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                // The playing song shows moving bars instead of its number.
                .child(
                    div().absolute().inset_0().flex().items_center().justify_center().opacity(1.0 - hk).child(if playing {
                        playing_bars(app, 14.0, theme::pink()).into_any_element()
                    } else {
                        mono().text_size(theme::text_sm()).text_color(theme::text_faint()).child(format!("{}", i + 1)).into_any_element()
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(hk)
                        .child(icon("play", 13.0, theme::text())),
                ),
        )
        .child(art)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                // Title and artist keep this much before anything else gives.
                .min_w(px(160.0))
                .gap(px(2.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w_0()
                        .when(tr.explicit, |d| d.child(explicit_badge()))
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_md())
                                .font_weight(if playing { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                .text_color(if playing { theme::pink() } else { theme::text() })
                                .child(tr.title.clone()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w_0()
                        .text_size(theme::text_sm())
                        .text_color(theme::text_dim())
                        .children(dl_badge.map(|b| {
                            crate::ui::widgets::with_tip(
                                div()
                                    .id(format!("{prefix}-dl-{i}"))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(16.0))
                                    .child(b),
                                dl_tip,
                            )
                            .into_any_element()
                        }))
                        .child(artist_link((prefix, i * 8 + 1), &tr.artist, cx)),
                ),
        )
        .when(show_album, |d| {
            d.child(
                div()
                    .flex_shrink(1.0)
                    .min_w_0()
                    .w(px(200.0))
                    .truncate()
                    .text_size(theme::text_sm())
                    .text_color(theme::text_faint())
                    .flex()
                    .children(album_link((prefix, i * 8 + 3), &tr, cx)),
            )
        })
        .child({
            let key: SharedString = format!("{prefix}-like-{}", crate::app::track_key(&t_like)).into();
            let k = key.clone();
            div()
                .flex_none()
                .when(!liked, |d| d.opacity(hk))
                .child(toggle_button(key, "heart", 15.0, liked, Some(theme::pink())).on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    mark_toggled(k.clone());
                    this.toggle_like(&t_like, cx)
                })))
        })
        .child(
            mono()
                .flex_none()
                .w(px(40.0))
                .text_size(theme::text_sm())
                .text_color(theme::text_faint())
                .child(fmt_ms(tr.duration_ms)),
        )
        .child(
            div()
                .id((prefix, i * 8 + 2))
                .flex_none()
                .size(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(|s| s.bg(white(0.08)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_ctx(t_menu.clone(), e.position, from_menu.clone(), cx)
                    }),
                )
                .child(icon("ellipsis-vertical", 16.0, theme::text_dim())),
        );
    with_menu(row, tr, from_ctx, cx)
}

/// One "recently played" row that is an artist: round picture, name and a
/// "Artist" label; click opens the artist page.
pub(super) fn recent_artist_row(app: &mut Glyph, i: usize, name: &str, track: Option<&Track>, cx: &mut Context<Glyph>) -> AnyElement {
    let art: AnyElement = match track {
        Some(tr) => cover(app, tr, 40.0, 20.0, cx).into_any_element(),
        None => div()
            .flex_none()
            .size(px(40.0))
            .rounded_full()
            .bg(theme::card())
            .flex()
            .items_center()
            .justify_center()
            .child(icon("user-round", 18.0, white(0.5)))
            .into_any_element(),
    };
    let n = name.to_string();
    with_artist_menu(div().id(("list-artist-row", i)), name.to_string(), cx)
        .flex()
        .items_center()
        .gap(px(14.0))
        .h(px(56.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .relative()
        .child(hover_fill(format!("list-artist-row-{i}"), white(0.05), 8.0))
        .on_click(cx.listener(move |this, _, _, cx| this.open_artist(&n, cx)))
        .child(
            div()
                .flex_none()
                .w(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .child(icon("user-round", 14.0, theme::text_faint())),
        )
        .child(art)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.0))
                .child(artist_name_badged(app, name, name.to_string(), 14.0, cx).text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()))
                .child(div().text_size(theme::text_sm()).text_color(theme::text_dim()).child(t("search.kind.artist"))),
        )
        .into_any_element()
}

/// "Follow" / "Following" outline button for an artist.
pub fn follow_button(id: impl Into<ElementId>, name: &str, track: Option<Track>, app: &Glyph, h: f32, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    let following = app.is_following(name);
    let n = name.to_string();
    let id: ElementId = id.into();
    let key = format!("follow-{id:?}");
    let swap = format!("follow-swap:{n}");
    let k = reveal_peek(&swap, 0.45);
    let e = 1.0 - (1.0 - k).powi(3);
    let label = if following { t("artist.following") } else { t("artist.follow") };
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .h(px(h))
        .px(px(16.0))
        .rounded(px(8.0))
        .border_1()
        // Following: quiet like the icon buttons next to it; not yet: a
        // brighter outline that asks for the click.
        .border_color(if following { white(0.14) } else { white(0.45) })
        .bg(black(0.35))
        .cursor_pointer()
        .text_size(theme::text_sm())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white(0.95))
        .relative()
        .child(hover_fill(key, white(0.1), 8.0))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            reveal_restart(&format!("follow-swap:{n}"));
            this.toggle_follow(&n, track.clone(), cx)
        }))
        // The check grows in (or shrinks away) while the label decodes.
        .when(following || k < 1.0, |d| {
            let w = if following { e } else { 1.0 - e };
            d.child(
                div()
                    .flex_none()
                    .w(px(20.0 * w))
                    .overflow_hidden()
                    .opacity(w)
                    .child(icon("check", 14.0 * (0.6 + 0.4 * w), white(0.95))),
            )
        })
        .child(decode(&label, k))
}

pub(super) fn round_icon_button(id: &'static str, name: &str, size: f32) -> Stateful<gpui::Div> {
    let tip = tip_for(name);
    with_tip(div().id(id), tip)
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .bg(black(0.35))
        .border_1()
        .border_color(white(0.14))
        .cursor_pointer()
        .relative()
        .child(hover_fill(id, white(0.1), 8.0))
        .child(icon(name, 16.0, white(0.9)))
}

/// Round white play button (pause while `playing`); used in the artist hero
/// and on the left of a "Show all" heading.
pub(crate) fn round_play(id: &'static str, playing: bool, loading: bool, size: f32) -> Stateful<gpui::Div> {
    with_tip(div().id(id), tip_for(if playing { "pause" } else { "play" }))
        .flex_none()
        .size(px(size))
        .rounded_full()
        .bg(gpui::white())
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .relative()
        .child(hover_fill(id, black(0.1), size / 2.0))
        // The triangle's weight sits left; nudge it to look centred.
        .child(div().when(!playing && !loading, |d| d.pl(px(2.0))).child(play_glyph(loading, playing, size * 0.42, size)))
}

/// Artist hero: borderless icon action next to the play button; its hover
/// is a rounded box like the follow button's.
pub(super) fn hero_icon(id: &'static str, name: &str) -> Stateful<gpui::Div> {
    with_tip(div().id(id), tip_for(name))
        .flex_none()
        .size(px(36.0))
        .rounded(px(8.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .relative()
        .child(hover_fill(id, white(0.1), 8.0))
        .child(icon(name, 18.0, white(0.85)))
}

/// White pill; `glyph` is the play / pause / loading mark.
pub(super) fn play_pill(id: impl Into<ElementId>, label: SharedString, glyph: AnyElement) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .h(px(36.0))
        .px(px(18.0))
        .rounded(px(8.0))
        .bg(gpui::white())
        .cursor_pointer()
        .text_size(theme::text_sm())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(gpui::black())
        .hover(|s| s.opacity(0.88))
        .child(glyph)
        .child(label)
}
