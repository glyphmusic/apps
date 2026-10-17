//! Song lists (Liked, Recent, playlists, albums), the artist profile, the
//! library grid and the right-click song menu.
//!
//! Layout follows the usual desktop-player pattern (cover column + numbered
//! list; artist hero + popular songs) with Glyph's own touches: the artist
//! hero is the photo rendered as coloured ASCII, and counts / durations are
//! set in mono like terminal output.

use crate::app::{Glyph, ListRef};
use crate::i18n::{t, tf};
use crate::theme;
use crate::ui::widgets::*;
use glyph_protocol::Track;
use gpui::{
    anchored, deferred, div, img, linear_color_stop, linear_gradient, prelude::*, px, AnyElement, Context, ElementId,
    FontWeight, Hsla, MouseButton, MouseDownEvent, ObjectFit, SharedString, Stateful, Window,
};
use std::rc::Rc;

fn white(a: f32) -> Hsla {
    gpui::hsla(0.0, 0.0, 1.0, a)
}

fn black(a: f32) -> Hsla {
    gpui::hsla(0.0, 0.0, 0.0, a)
}

/// "11:50" or "1:02:03" for a list's total length.
fn total_time(tracks: &[Track]) -> String {
    let s: u64 = tracks.iter().filter_map(|t| t.duration_ms).sum::<u64>() / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// `[ 07 SONGS · 11:50 ]`-style meta line (mono, the ASCII accent).
fn meta_line(tracks: &[Track], color: Hsla) -> gpui::Div {
    mono()
        .text_size(px(11.5))
        .text_color(color)
        .child(format!("[ {:02} {} · {} ]", tracks.len(), t("list.songs_caps"), total_time(tracks)))
}

/// Floating "Back" pill (only when there is history).
fn back_button(app: &Glyph, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    (!app.history.is_empty()).then(|| {
        div()
            .id("nav-back")
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(30.0))
            .px(px(12.0))
            .rounded(px(8.0))
            .bg(black(0.55))
            .border_1()
            .border_color(white(0.1))
            .cursor_pointer()
            .text_size(theme::text_sm())
            .text_color(white(0.9))
            .hover(|s| s.bg(black(0.75)))
            .on_click(cx.listener(|this, _, _, cx| this.go_back(cx)))
            .child(icon("arrow-left", 14.0, white(0.9)))
            .child(t("common.back"))
            .into_any_element()
    })
}

/// Clickable artist name: opens the artist profile.
pub fn artist_link(id: impl Into<ElementId>, name: &str, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    let n = name.to_string();
    div()
        .id(id)
        .min_w_0()
        .truncate()
        .cursor_pointer()
        .hover(|s| s.underline())
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.open_artist(&n, cx)
        }))
        .child(name.to_string())
}

/// Right-click on a song opens the song menu.
pub fn with_menu(el: Stateful<gpui::Div>, tr: Track, from: Option<(ListRef, usize)>, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    el.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.open_ctx(tr.clone(), e.position, from.clone(), cx)
        }),
    )
}

// ------------------------------------------------------------------ covers

/// Cover for a list: heart tile (Liked), clock tile (Recent), 2×2 mosaic or
/// single cover (playlists), first cover (albums).
pub fn list_cover(app: &mut Glyph, l: &ListRef, size: f32, cx: &mut Context<Glyph>) -> AnyElement {
    let r = (size * 0.06).clamp(6.0, 12.0);
    let tile = |bg: Hsla, name: &str, fg: Hsla| {
        div()
            .flex_none()
            .size(px(size))
            .rounded(px(r))
            .bg(bg)
            .flex()
            .items_center()
            .justify_center()
            .child(icon(name, (size * 0.3).clamp(16.0, 72.0), fg))
            .into_any_element()
    };
    let tracks = app.list_tracks(l);
    match l {
        ListRef::Liked | ListRef::ArtistLiked(_) => tile(gpui::hsla(330.0 / 360.0, 0.45, 0.22, 1.0), "heart-fill", theme::pink()),
        ListRef::Recent => tile(theme::card(), "history", white(0.75)),
        ListRef::User(_) => {
            // Distinct covers (by path) for the mosaic.
            let mut paths: Vec<std::path::PathBuf> = Vec::new();
            for tr in tracks.iter().take(12) {
                if let Some(p) = app.cover_path(tr, cx) {
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }
                if paths.len() == 4 {
                    break;
                }
            }
            if paths.len() == 4 {
                let h = size / 2.0;
                let rr = px(r);
                let q = |p: std::path::PathBuf| img(p).size(px(h)).object_fit(ObjectFit::Cover);
                let mut it = paths.into_iter();
                let (a, b, c, d) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
                div()
                    .flex_none()
                    .size(px(size))
                    .flex()
                    .flex_col()
                    .child(div().flex().child(q(a).rounded_tl(rr)).child(q(b).rounded_tr(rr)))
                    .child(div().flex().child(q(c).rounded_bl(rr)).child(q(d).rounded_br(rr)))
                    .into_any_element()
            } else if let Some(first) = tracks.first() {
                cover(app, first, size, r, cx).into_any_element()
            } else {
                tile(theme::card(), "list-music", white(0.5))
            }
        }
        ListRef::Album { .. } => match tracks.first() {
            Some(first) => cover(app, first, size, r, cx).into_any_element(),
            None => tile(theme::card(), "disc-3", white(0.5)),
        },
    }
}

// ------------------------------------------------------------------ song rows

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
    let tr = list[i].clone();
    let playing = app.current.as_ref().map(crate::app::track_key) == Some(crate::app::track_key(&tr));
    let liked = app.is_liked(&tr);
    let group: SharedString = format!("{prefix}-g-{i}").into();
    let art = cover(app, &tr, 40.0, 6.0, cx);
    let l2 = list.clone();
    let from_ctx = from.clone().map(|l| (l, i));
    let (t_like, t_menu) = (tr.clone(), tr.clone());
    let from_menu = from_ctx.clone();
    let row = div()
        .id((prefix, i))
        .group(group.clone())
        .flex()
        .items_center()
        .gap(px(14.0))
        .h(px(56.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(white(0.05)))
        .on_click(cx.listener(move |this, _, _, cx| this.play_list((*l2).clone(), i, cx)))
        .child(
            div()
                .flex_none()
                .w(px(26.0))
                .h(px(20.0))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    mono()
                        .text_size(theme::text_sm())
                        .text_color(if playing { theme::pink() } else { theme::text_faint() })
                        .group_hover(group.clone(), |s| s.opacity(0.0))
                        .child(format!("{}", i + 1)),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(0.0)
                        .group_hover(group.clone(), |s| s.opacity(1.0))
                        .child(icon("play", 13.0, theme::text())),
                ),
        )
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
                        .truncate()
                        .text_size(theme::text_md())
                        .font_weight(if playing { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                        .text_color(if playing { theme::pink() } else { theme::text() })
                        .child(tr.title.clone()),
                )
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .text_size(theme::text_sm())
                        .text_color(theme::text_dim())
                        .child(artist_link((prefix, i * 8 + 1), &tr.artist, cx)),
                ),
        )
        .when(show_album, |d| {
            d.child(
                div()
                    .flex_none()
                    .w(px(200.0))
                    .truncate()
                    .text_size(theme::text_sm())
                    .text_color(theme::text_faint())
                    .child(tr.album.clone().unwrap_or_default()),
            )
        })
        .child({
            let key: SharedString = format!("{prefix}-like-{}", crate::app::track_key(&t_like)).into();
            let k = key.clone();
            div()
                .flex_none()
                .when(!liked, |d| d.opacity(0.0).group_hover(group.clone(), |s| s.opacity(1.0)))
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

fn round_icon_button(id: impl Into<ElementId>, name: &str, size: f32) -> Stateful<gpui::Div> {
    div()
        .id(id)
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
        .hover(|s| s.bg(white(0.1)))
        .child(icon(name, 16.0, white(0.9)))
}

fn play_pill(id: impl Into<ElementId>, label: SharedString) -> Stateful<gpui::Div> {
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
        .child(icon("play", 14.0, gpui::black()))
        .child(label)
}

// ------------------------------------------------------------------ list view

pub fn list_view(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let Some(l) = app.open_list.clone() else { return div().into_any_element() };
    let tracks = app.list_tracks(&l);
    let title = app.list_title(&l);
    let art = list_cover(app, &l, 248.0, cx);
    let kind = match &l {
        ListRef::Album { .. } => t("list.kind.album"),
        _ => t("list.kind.playlist"),
    };
    // Only albums and "liked by artist" have a subtitle (the artist).
    let sub: Option<AnyElement> = match &l {
        ListRef::Album { artist, .. } | ListRef::ArtistLiked(artist) => Some(
            div()
                .flex()
                .text_size(theme::text_sm())
                .text_color(theme::text_dim())
                .child(artist_link("list-artist", artist, cx))
                .into_any_element(),
        ),
        _ => None,
    };
    let (t_play, t_shuf, t_queue) = (tracks.clone(), tracks.clone(), tracks.clone());
    let has = !tracks.is_empty();
    let user_id = match &l {
        ListRef::User(id) => Some(id.clone()),
        _ => None,
    };

    let left = div()
        .flex_none()
        .w(px(248.0))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(art)
        .child(
            div()
                .pt(px(4.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(mono().text_size(px(11.0)).text_color(theme::text_faint()).child(format!("[{}]", kind.to_uppercase())))
                .child(div().text_size(px(22.0)).font_weight(FontWeight::BOLD).text_color(theme::text()).child(title.clone()))
                .children(sub)
                .child(meta_line(&tracks, theme::text_faint())),
        )
        .when(has, |d| {
            d.child(
                play_pill("list-play", t("list.play_all"))
                    .w_full()
                    .on_click(cx.listener(move |this, _, _, cx| this.play_list(t_play.clone(), 0, cx))),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        button("list-queue", t("list.add_queue"))
                            .flex_1()
                            .justify_center()
                            .h(px(34.0))
                            .on_click(cx.listener(move |this, _, _, cx| this.queue_all(t_queue.clone(), cx))),
                    )
                    .child(round_icon_button("list-shuffle", "shuffle", 34.0).on_click(cx.listener(move |this, _, _, cx| this.play_shuffled(t_shuf.clone(), cx)))),
            )
        })
        .when_some(user_id, |d, id| {
            let id2 = id.clone();
            d.child(
                div()
                    .flex()
                    .gap(px(4.0))
                    .child(
                        text_button("list-rename", t("list.rename"))
                            .text_size(theme::text_sm())
                            .on_click(cx.listener(move |this, _, w, cx| this.open_name_dialog(None, Some(id2.clone()), w, cx))),
                    )
                    .child(
                        text_button("list-delete", t("list.delete"))
                            .text_size(theme::text_sm())
                            .on_click(cx.listener(move |this, _, _, cx| this.delete_playlist(&id, cx))),
                    ),
            )
        });

    let right: AnyElement = if tracks.is_empty() {
        let (a, b) = match &l {
            ListRef::Liked | ListRef::ArtistLiked(_) => (t("library.liked.empty"), t("library.liked.empty.sub")),
            ListRef::Recent => (t("home.empty.title"), t("library.recent.empty.sub")),
            _ => (t("library.playlist.empty"), t("list.empty.sub")),
        };
        ascii_state(ART_EMPTY, a, b).into_any_element()
    } else {
        let list = Rc::new(tracks);
        let show_album = !matches!(l, ListRef::Album { .. });
        let rows: Vec<_> = (0..list.len()).map(|i| song_row(app, list.clone(), i, "list-row", Some(l.clone()), show_album, cx)).collect();
        div().flex().flex_col().children(rows).into_any_element()
    };

    div()
        .id("list-scroll")
        .track_scroll(&app.main_scroll)
        .size_full()
        .overflow_y_scroll()
        .px(px(32.0))
        .py(px(20.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(16.0))
                .children(back_button(app, cx).map(|b| div().flex().child(b)))
                .child(div().flex().items_start().gap(px(36.0)).child(left).child(div().flex_1().min_w_0().child(right))),
        )
        .into_any_element()
}

// ------------------------------------------------------------------ artist

pub fn artist_view(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let Some(name) = app.open_artist.clone() else { return div().into_any_element() };
    let (tracks, loading, info, info_loading, show_all, about_open) = match app.artists.get(&name) {
        Some(p) => (p.tracks.clone(), p.loading, p.info.clone(), p.info_loading, p.show_all, p.about_open),
        None => (Vec::new(), true, None, true, false, false),
    };
    // Hero: the artist photo (Wikipedia), else the top song's cover, as ASCII.
    let src = info
        .as_ref()
        .and_then(|i| i.image.clone())
        .or_else(|| tracks.first().cloned().and_then(|tr| app.cover_path(&tr, cx)));
    let hero_img = src.as_ref().and_then(|p| app.hero_image(p, cx));
    let still_loading = hero_img.is_none() && (src.is_some() || info_loading || loading);

    let (t_play, t_shuf, t_queue) = (tracks.clone(), tracks.clone(), tracks.clone());
    let has = !tracks.is_empty();
    let n_songs = tracks.len();

    let about_card = if let Some(i) = info.clone() {
        let url = i.url.clone();
        let nm = name.clone();
        Some(
            div()
                .id("artist-about")
                .w(px(340.0))
                .p(px(14.0))
                .rounded(px(12.0))
                .bg(black(0.6))
                .border_1()
                .border_color(white(0.1))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(div().text_size(px(14.0)).font_weight(FontWeight::SEMIBOLD).text_color(white(0.95)).child(t("artist.about")))
                .child(
                    div()
                        .id("about-text")
                        .text_size(px(12.5))
                        .line_height(px(18.0))
                        .text_color(white(0.72))
                        .overflow_hidden()
                        .when(!about_open, |d| d.max_h(px(18.0 * 4.0)))
                        .when(about_open, |d| d.max_h(px(18.0 * 11.0)).overflow_y_scroll())
                        .child(i.about.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .id("about-more")
                                .cursor_pointer()
                                .text_size(px(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(white(0.6))
                                .hover(|s| s.text_color(white(0.95)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(p) = this.artists.get_mut(&nm) {
                                        p.about_open = !p.about_open;
                                    }
                                    cx.notify();
                                }))
                                .child(if about_open { t("artist.less") } else { t("artist.more") }),
                        )
                        .when(!url.is_empty(), |d| {
                            d.child(
                                div()
                                    .id("about-wiki")
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .cursor_pointer()
                                    .text_size(px(11.5))
                                    .text_color(white(0.45))
                                    .hover(|s| s.text_color(white(0.85)))
                                    .on_click(move |_, _, cx| cx.open_url(&url))
                                    .child("Wikipedia")
                                    .child(icon("external-link", 11.0, white(0.45))),
                            )
                        }),
                )
                .into_any_element(),
        )
    } else if info_loading {
        Some(
            div()
                .w(px(340.0))
                .p(px(14.0))
                .rounded(px(12.0))
                .bg(black(0.6))
                .border_1()
                .border_color(white(0.1))
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(div().text_size(px(14.0)).font_weight(FontWeight::SEMIBOLD).text_color(white(0.95)).child(t("artist.about")))
                .child(skeleton("about-sk", 310.0, 64.0, 6.0))
                .into_any_element(),
        )
    } else {
        None
    };

    // Full-bleed hero: the ASCII photo is the page background here and fades
    // out at the bottom (baked into the raster), so it melts into the glow.
    let hero = div()
        .relative()
        .mx(px(-32.0))
        .mt(px(-20.0))
        .h(px(420.0))
        .overflow_hidden()
        .when_some(hero_img, |d, im| d.child(div().absolute().inset_0().child(img(im).size_full().object_fit(ObjectFit::Cover))))
        .when(still_loading, |d| d.child(div().absolute().inset_0().child(skeleton("hero-sk", 1680.0, 420.0, 0.0))))
        .children(back_button(app, cx).map(|b| div().absolute().top(px(20.0)).left(px(32.0)).child(b)))
        .child(
            div()
                .absolute()
                .left(px(32.0))
                .bottom(px(40.0))
                .right(px(if about_card.is_some() { 400.0 } else { 32.0 }))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(mono().text_size(px(11.0)).text_color(white(0.55)).child(format!("[{}]", t("artist.badge"))))
                .child(div().text_size(px(46.0)).line_height(px(50.0)).font_weight(FontWeight::BOLD).text_color(gpui::white()).truncate().child(name.clone()))
                .when(has, |d| d.child(mono().text_size(px(11.5)).text_color(white(0.6)).child(format!("[ {:02} {} ]", n_songs, t("list.songs_caps")))))
                .when(has, |d| {
                    d.child(
                        div()
                            .pt(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(play_pill("artist-play", t("common.play")).on_click(cx.listener(move |this, _, _, cx| this.play_list(t_play.clone(), 0, cx))))
                            .child(round_icon_button("artist-shuffle", "shuffle", 36.0).on_click(cx.listener(move |this, _, _, cx| this.play_shuffled(t_shuf.clone(), cx))))
                            .child(round_icon_button("artist-queue", "list-plus", 36.0).on_click(cx.listener(move |this, _, _, cx| this.queue_all(t_queue.clone(), cx)))),
                    )
                }),
        )
        .children(about_card.map(|c| div().absolute().right(px(32.0)).bottom(px(40.0)).child(c)));

    // Popular songs: 5, then "Show more".
    let n_show = if show_all { tracks.len() } else { tracks.len().min(5) };
    let songs: AnyElement = if tracks.is_empty() && loading {
        skeleton_rows("artist", 5).into_any_element()
    } else if tracks.is_empty() {
        ascii_state(ART_EMPTY, t("artist.empty"), t("artist.empty.sub")).into_any_element()
    } else {
        let list = Rc::new(tracks.clone());
        let rows: Vec<_> = (0..n_show).map(|i| song_row(app, list.clone(), i, "artist-row", None, true, cx)).collect();
        div().flex().flex_col().children(rows).into_any_element()
    };
    let nm = name.clone();
    let more = (tracks.len() > 5).then(|| {
        div().flex().pt(px(4.0)).child(
            button("artist-all", if show_all { t("artist.show_less") } else { t("artist.show_more_n") }).on_click(cx.listener(move |this, _, _, cx| {
                if let Some(p) = this.artists.get_mut(&nm) {
                    p.show_all = !p.show_all;
                }
                cx.notify();
            })),
        )
    });

    // Your liked songs by this artist.
    let liked = app.liked_by(&name);
    let liked_card = (!liked.is_empty()).then(|| {
        let l = ListRef::ArtistLiked(name.clone());
        let n = liked.len();
        div()
            .flex_none()
            .w(px(300.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(section_title("heart", t("artist.liked")))
            .child(
                card()
                    .id("artist-liked")
                    .flex()
                    .items_center()
                    .gap(px(14.0))
                    .p(px(10.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::card_hover()))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_list(l.clone(), cx)))
                    .child(
                        div()
                            .flex_none()
                            .size(px(56.0))
                            .rounded(px(8.0))
                            .bg(gpui::hsla(330.0 / 360.0, 0.45, 0.22, 1.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("heart-fill", 20.0, theme::pink())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(theme::text_md())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(white(0.95))
                                    .child(tf("artist.liked_n", &[("n", &n)])),
                            )
                            .child(div().truncate().text_size(theme::text_sm()).text_color(white(0.55)).child(tf("artist.by", &[("name", &name)]))),
                    ),
            )
    });

    // Albums, grouped from the songs found.
    let mut albums: Vec<(String, Vec<Track>)> = Vec::new();
    for tr in &tracks {
        if let Some(a) = tr.album.clone().filter(|a| !a.trim().is_empty()) {
            match albums.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&a)) {
                Some((_, v)) => v.push(tr.clone()),
                None => albums.push((a, vec![tr.clone()])),
            }
        }
    }
    let album_cards: Vec<AnyElement> = albums
        .into_iter()
        .enumerate()
        .map(|(i, (title, list))| {
            let art = cover(app, &list[0], 160.0, 8.0, cx);
            let n = list.len();
            let l = ListRef::Album { title: title.clone(), artist: name.clone(), tracks: list };
            div()
                .id(("album", i))
                .flex_none()
                .w(px(160.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.open_list(l.clone(), cx)))
                .child(art)
                .child(div().pt(px(4.0)).truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(title))
                .child(mono().text_size(px(11.0)).text_color(theme::text_faint()).child(format!("{:02} {}", n, t("list.songs_caps"))))
                .into_any_element()
        })
        .collect();

    let body = div()
        .flex()
        .flex_col()
        .gap(px(22.0))
        .child(hero)
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(32.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(section_title("music-2", t("artist.popular")))
                        .child(songs)
                        .children(more),
                )
                .children(liked_card),
        )
        .when(!album_cards.is_empty(), |d| {
            d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(section_title("disc-3", t("artist.albums")))
                    .child(div().id("artist-albums").flex().gap(px(20.0)).overflow_x_scroll().children(album_cards)),
            )
        });
    div().id("artist-scroll").track_scroll(&app.main_scroll).size_full().overflow_y_scroll().px(px(32.0)).py(px(20.0)).child(body).into_any_element()
}

// ------------------------------------------------------------------ library

pub fn library(app: &mut Glyph, _w: &mut Window, cx: &mut Context<Glyph>) -> AnyElement {
    let head = div()
        .flex()
        .items_center()
        .justify_between()
        .child(heading(t("nav.library")))
        .child(
            div()
                .id("lib-new")
                .flex()
                .items_center()
                .gap(px(6.0))
                .h(px(32.0))
                .px(px(14.0))
                .rounded(px(8.0))
                .bg(theme::card())
                .border_1()
                .border_color(theme::border())
                .cursor_pointer()
                .text_size(theme::text_sm())
                .text_color(white(0.9))
                .hover(|s| s.bg(theme::card_hover()))
                .on_click(cx.listener(|this, _, w, cx| this.open_name_dialog(None, None, w, cx)))
                .child(icon("plus", 14.0, white(0.9)))
                .child(t("library.playlist.new")),
        );
    if !app.library_ok {
        return div()
            .id("lib-scroll")
        .track_scroll(&app.main_scroll)
            .size_full()
            .px(px(32.0))
            .py(px(20.0))
            .child(div().flex().flex_col().gap(px(18.0)).child(head).child(ascii_state(ART_OFFLINE, t("status.library_offline"), t("library.offline.sub"))))
            .into_any_element();
    }
    let mut lists: Vec<(ListRef, String)> = vec![(ListRef::Liked, t("list.kind.playlist").to_string())];
    if !app.library.recent.is_empty() {
        lists.push((ListRef::Recent, t("list.kind.playlist").to_string()));
    }
    for p in &app.library.playlists {
        lists.push((ListRef::User(p.id.clone()), t("list.kind.playlist").to_string()));
    }
    let tiles: Vec<AnyElement> = lists
        .into_iter()
        .enumerate()
        .map(|(i, (l, kind))| {
            let art = list_cover(app, &l, 168.0, cx);
            let title = app.list_title(&l);
            let n = app.list_tracks(&l).len();
            div()
                .id(("lib-tile", i))
                .flex_none()
                .w(px(184.0))
                .p(px(8.0))
                .rounded(px(10.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .cursor_pointer()
                .hover(|s| s.bg(white(0.05)))
                .on_click(cx.listener(move |this, _, _, cx| this.open_list(l.clone(), cx)))
                .child(art)
                .child(div().pt(px(6.0)).truncate().text_size(theme::text_md()).font_weight(FontWeight::MEDIUM).text_color(theme::text()).child(title))
                .child(div().truncate().text_size(theme::text_sm()).text_color(theme::text_dim()).child(format!("{kind} · {}", tf("common.n_songs", &[("n", &n)]))))
                .into_any_element()
        })
        .collect();
    div()
        .id("lib-scroll")
        .track_scroll(&app.main_scroll)
        .size_full()
        .overflow_y_scroll()
        .px(px(32.0))
        .py(px(20.0))
        .child(div().flex().flex_col().gap(px(18.0)).child(head).child(div().flex().flex_wrap().gap(px(8.0)).ml(px(-8.0)).children(tiles)))
        .into_any_element()
}

// ------------------------------------------------------------------ song menu

/// One menu row. `closes_sub`: hovering it closes the playlist flyout.
fn ctx_item(id: ElementId, name: &str, label: SharedString, color: Hsla, closes_sub: bool, cx: &mut Context<Glyph>) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(10.0))
        .h(px(32.0))
        .px(px(10.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .text_size(px(13.0))
        .text_color(white(0.88))
        .hover(|s| s.bg(white(0.07)))
        .when(closes_sub, |d| {
            d.on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if let Some(m) = this.ctx_menu.as_mut().filter(|m| *hovered && m.playlists) {
                    m.playlists = false;
                    cx.notify();
                }
            }))
        })
        .child(icon(name, 15.0, color))
        .child(div().flex_1().truncate().child(label))
}

pub fn ctx_menu(app: &mut Glyph, window: &mut Window, cx: &mut Context<Glyph>) -> Option<AnyElement> {
    let m = app.ctx_menu.clone()?;
    let tr = m.track.clone();
    let sep = || div().h(px(1.0)).mx(px(6.0)).my(px(4.0)).bg(white(0.07));
    let mut body: Vec<AnyElement> = Vec::new();
    // "Add to playlist" flyout (opens on hover, to the right or left).
    let mut sub: Vec<AnyElement> = Vec::new();
    if m.playlists {
        let t_new = tr.clone();
        sub.push(
            ctx_item("ctx-new-pl".into(), "plus", t("library.playlist.new"), white(0.7), false, cx)
                .on_click(cx.listener(move |this, _, w, cx| this.open_name_dialog(Some(t_new.clone()), None, w, cx)))
                .into_any_element(),
        );
        if !app.library.playlists.is_empty() {
            sub.push(sep().into_any_element());
        }
        let pls: Vec<(String, String)> = app.library.playlists.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
        for (i, (id, name)) in pls.into_iter().enumerate() {
            let t_add = tr.clone();
            sub.push(
                ctx_item(("ctx-pl", i).into(), "list-music", name.into(), white(0.7), false, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ctx_menu = None;
                        this.add_to_playlist(&id, t_add.clone(), cx)
                    }))
                    .into_any_element(),
            );
        }
    }
    let flip = f32::from(m.pos.x) + 244.0 + 236.0 > f32::from(window.viewport_size().width);
    {
        let art = cover(app, &tr, 36.0, 5.0, cx);
        body.push(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .px(px(6.0))
                .py(px(4.0))
                .child(art)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().truncate().text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(white(0.95)).child(tr.title.clone()))
                        .child(div().truncate().text_size(px(12.0)).text_color(white(0.55)).child(tr.artist.clone())),
                )
                .into_any_element(),
        );
        body.push(sep().into_any_element());
        let t1 = tr.clone();
        body.push(
            ctx_item("ctx-play".into(), "play", t("ctx.play"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ctx_menu = None;
                    this.play_list(vec![t1.clone()], 0, cx)
                }))
                .into_any_element(),
        );
        let t2 = tr.clone();
        body.push(
            ctx_item("ctx-next".into(), "list-start", t("ctx.play_next"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ctx_menu = None;
                    this.queue_add(t2.clone(), true, cx)
                }))
                .into_any_element(),
        );
        let t3 = tr.clone();
        body.push(
            ctx_item("ctx-queue".into(), "list-plus", t("ctx.queue"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ctx_menu = None;
                    this.queue_add(t3.clone(), false, cx)
                }))
                .into_any_element(),
        );
        body.push(sep().into_any_element());
        let liked = app.is_liked(&tr);
        let t4 = tr.clone();
        body.push(
            ctx_item(
                "ctx-like".into(),
                if liked { "heart-fill" } else { "heart" },
                if liked { t("ctx.unlike") } else { t("ctx.like") },
                if liked { theme::pink() } else { white(0.7) },
                true,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ctx_menu = None;
                this.toggle_like(&t4.clone(), cx)
            }))
            .into_any_element(),
        );
        body.push(
            ctx_item("ctx-add-pl".into(), "list-music", t("ctx.add_playlist"), white(0.7), false, cx)
                .relative()
                .when(m.playlists, |d| d.bg(white(0.07)))
                .child(icon("chevron-right", 14.0, white(0.4)))
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if let Some(m) = this.ctx_menu.as_mut().filter(|m| *hovered && !m.playlists) {
                        m.playlists = true;
                        cx.notify();
                    }
                }))
                .when(m.playlists, |d| {
                    d.child(
                        div()
                            .id("ctx-sub")
                            .absolute()
                            .top(px(-6.0))
                            .when(!flip, |d| d.left(px(236.0)))
                            .when(flip, |d| d.right(px(236.0)))
                            .w(px(228.0))
                            .max_h(px(320.0))
                            .overflow_y_scroll()
                            .p(px(6.0))
                            .rounded(px(10.0))
                            .bg(gpui::hsla(0.0, 0.0, 0.075, 0.98))
                            .border_1()
                            .border_color(white(0.1))
                            .flex()
                            .flex_col()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .children(sub),
                    )
                })
                .into_any_element(),
        );
        if let Some((l, i)) = m.from.clone().filter(|(l, _)| matches!(l, ListRef::User(_) | ListRef::Recent)) {
            let label = if l == ListRef::Recent { t("ctx.remove_recent") } else { t("ctx.remove") };
            body.push(
                ctx_item("ctx-remove".into(), "circle-minus", label, white(0.7), true, cx)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ctx_menu = None;
                        this.remove_from_list(&l, i, cx)
                    }))
                    .into_any_element(),
            );
        }
        body.push(sep().into_any_element());
        let artist = tr.artist.clone();
        body.push(
            ctx_item("ctx-artist".into(), "user-round", t("ctx.artist"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ctx_menu = None;
                    this.open_artist(&artist, cx)
                }))
                .into_any_element(),
        );
        let t5 = tr.clone();
        body.push(
            ctx_item("ctx-link".into(), "link-2", t("ctx.copy_link"), white(0.7), true, cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ctx_menu = None;
                    this.share(&t5, cx);
                    cx.notify();
                }))
                .into_any_element(),
        );
    }

    let menu = div()
        .id("ctx-menu")
        .w(px(244.0))
        .p(px(6.0))
        .rounded(px(10.0))
        .bg(gpui::hsla(0.0, 0.0, 0.075, 0.98))
        .border_1()
        .border_color(white(0.1))
        .flex()
        .flex_col()
        .font_family(theme::FONT)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .children(body);
    Some(
        deferred(
            div()
                .id("ctx-backdrop")
                .absolute()
                .inset_0()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_ctx(cx)))
                .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| this.close_ctx(cx)))
                .on_scroll_wheel(cx.listener(|this, _, _, cx| this.close_ctx(cx)))
                .child(anchored().position(m.pos).snap_to_window_with_margin(px(8.0)).child(menu)),
        )
        .with_priority(3)
        .into_any_element(),
    )
}

// ------------------------------------------------------------------ name dialog

/// "New playlist" / "Rename playlist": a small centred dialog with one field.
pub fn name_dialog(app: &mut Glyph, cx: &mut Context<Glyph>) -> AnyElement {
    let renaming = app.name_dialog.as_ref().is_some_and(|d| d.rename.is_some());
    let input = app.playlist_name.clone();
    let panel = div()
        .id("name-dialog")
        .occlude()
        .w(px(380.0))
        .p(px(20.0))
        .rounded(px(12.0))
        .bg(gpui::hsla(0.0, 0.0, 0.07, 0.99))
        .border_1()
        .border_color(white(0.08))
        .flex()
        .flex_col()
        .gap(px(14.0))
        .font_family(theme::FONT)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .text_size(px(16.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(white(0.95))
                .child(if renaming { t("dialog.playlist.rename") } else { t("dialog.playlist.title") }),
        )
        .child(
            div()
                .h(px(38.0))
                .px(px(12.0))
                .flex()
                .items_center()
                .rounded(px(8.0))
                .bg(black(0.45))
                .border_1()
                .border_color(white(0.12))
                .text_size(px(13.5))
                .child(input),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.0))
                .child(button("dialog-cancel", t("dialog.cancel")).on_click(cx.listener(|this, _, _, cx| this.close_name_dialog(cx))))
                .child(
                    primary_button("dialog-ok", if renaming { t("dialog.save") } else { t("dialog.create") })
                        .on_click(cx.listener(|this, _, _, cx| this.submit_name_dialog(cx))),
                ),
        );
    deferred(
        div()
            .id("dialog-backdrop")
            .absolute()
            .inset_0()
            .bg(black(0.45))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_name_dialog(cx)))
            .child(panel),
    )
    .with_priority(4)
    .into_any_element()
}
